//! Original, output-only voice cues. One bounded worker owns all device I/O;
//! neither the foreground nor the voice mixer waits for it.

mod render;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait as _, StreamTrait as _};
use cpal::{FromSample, SizedSample, StreamConfig};
use tokio::sync::mpsc;

use crate::platform::audio::in_device_format;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Join,
    Leave,
    PeerJoin,
}

#[derive(Clone)]
pub struct CueSettings(Arc<Control>);

struct Control {
    disabled: AtomicBool,
    deafened: AtomicBool,
    stopped: AtomicBool,
    epoch: AtomicU64,
}

impl CueSettings {
    fn new(enabled: bool) -> Self {
        Self(Arc::new(Control {
            disabled: AtomicBool::new(!enabled),
            deafened: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            epoch: AtomicU64::new(1),
        }))
    }

    pub fn enabled(&self) -> bool {
        !self.0.disabled.load(Ordering::Acquire)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.0.disabled.store(!enabled, Ordering::Release);
        if !enabled {
            self.discard_pending();
        }
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.0.deafened.store(deafened, Ordering::Release);
        if deafened {
            self.discard_pending();
        }
    }

    pub fn discard_pending(&self) {
        self.0.epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn audible(&self, epoch: u64) -> bool {
        self.enabled()
            && !self.0.deafened.load(Ordering::Acquire)
            && !self.0.stopped.load(Ordering::Acquire)
            && self.0.epoch.load(Ordering::Acquire) == epoch
    }
}

pub struct CuePlayer {
    requests: mpsc::Sender<Option<Request>>,
    settings: CueSettings,
    own: Arc<OwnCues>,
}

struct Request {
    cue: Cue,
    epoch: u64,
}

/// Separate atomic slots keep a full peer queue from dropping a local join/leave.
/// Each own transition has its own epoch; only the newest can remain audible.
#[derive(Default)]
struct OwnCues {
    join: AtomicU64,
    leave: AtomicU64,
}

impl OwnCues {
    fn take(&self, queued: Option<Request>, settings: &CueSettings) -> Option<Request> {
        let join = self.join.swap(0, Ordering::AcqRel);
        let leave = self.leave.swap(0, Ordering::AcqRel);
        let epoch = join.max(leave);
        let own = (epoch != 0).then_some(Request {
            cue: if join > leave { Cue::Join } else { Cue::Leave },
            epoch,
        });
        own.filter(|request| settings.audible(request.epoch))
            .or_else(|| queued.filter(|request| settings.audible(request.epoch)))
    }
}

impl CuePlayer {
    /// The selector runs only on the worker, also after the call engine closes.
    /// It can share the voice output selection without owning another preference.
    pub fn start(
        enabled: bool,
        select_output: impl Fn() -> Option<cpal::Device> + Send + 'static,
    ) -> Self {
        let settings = CueSettings::new(enabled);
        let (requests, mut rx) = mpsc::channel::<Option<Request>>(4);
        let control = settings.clone();
        let own = Arc::new(OwnCues::default());
        let pending = own.clone();
        thread::Builder::new()
            .name("voice-cues".into())
            .spawn(move || {
                while let Some(queued) = rx.blocking_recv() {
                    if let Some(request) = pending.take(queued, &control) {
                        play(request, &control, &select_output);
                    }
                }
            })
            .ok();
        Self {
            requests,
            settings,
            own,
        }
    }

    pub fn settings(&self) -> CueSettings {
        self.settings.clone()
    }

    pub fn play(&self, cue: Cue) {
        let epoch = self.settings.0.epoch.load(Ordering::Acquire);
        if self.settings.audible(epoch) {
            match cue {
                Cue::Join => self.own.join.store(epoch, Ordering::Release),
                Cue::Leave => self.own.leave.store(epoch, Ordering::Release),
                Cue::PeerJoin => {
                    self.play_peer(cue);
                    return;
                }
            }
            // When full, an already-queued peer event also wakes the worker.
            let _ = self.requests.try_send(None);
        }
    }

    pub fn play_peer(&self, cue: Cue) {
        let epoch = self.settings.0.epoch.load(Ordering::Acquire);
        if self.settings.audible(epoch) {
            let _ = self.requests.try_send(Some(Request { cue, epoch }));
        }
    }
}

impl Drop for CuePlayer {
    fn drop(&mut self) {
        self.settings.0.stopped.store(true, Ordering::Release);
    }
}

fn play(
    request: Request,
    settings: &CueSettings,
    select_output: &impl Fn() -> Option<cpal::Device>,
) {
    let Some(device) = select_output() else {
        return;
    };
    let Ok(config) = device.default_output_config() else {
        return;
    };
    if config.channels() == 0 || config.sample_rate() == 0 || !settings.audible(request.epoch) {
        return;
    }
    let completion = Arc::new(AtomicBool::new(false));
    let stream = in_device_format!(
        config.sample_format(),
        build,
        &device,
        config.into(),
        request,
        settings.clone(),
        completion.clone()
    );
    let Some(stream) = stream else {
        return;
    };
    // A broken device must not strand the worker. No polling while idle or playing:
    // callbacks wake this thread, with a finite watchdog only for driver failure.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !completion.load(Ordering::Acquire) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        thread::park_timeout(deadline - now);
    }
    drop(stream);
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    request: Request,
    settings: CueSettings,
    completion: Arc<AtomicBool>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut renderer = render::Renderer::new(request.cue, config.sample_rate, config.channels);
    let worker = thread::current();
    let error_worker = worker.clone();
    let error_completion = completion.clone();
    let mut finished_at = None;
    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                if !settings.audible(request.epoch) {
                    out.fill(T::from_sample(0.));
                    completion.store(true, Ordering::Release);
                    worker.unpark();
                    return;
                }
                renderer.fill(out);
                if renderer.finished() {
                    // Keep the final buffer alive until its scheduled playback.
                    let timestamp = info.timestamp();
                    let drain = timestamp.playback.duration_since(timestamp.callback)
                        + Duration::from_secs_f64(out.len() as f64 / renderer.samples_per_second());
                    let deadline = *finished_at.get_or_insert_with(|| Instant::now() + drain);
                    if Instant::now() >= deadline {
                        completion.store(true, Ordering::Release);
                        worker.unpark();
                    }
                }
            },
            move |_| {
                error_completion.store(true, Ordering::Release);
                error_worker.unpark();
            },
            Some(Duration::from_secs(1)),
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deafen_and_disable_discard_queued_cues_even_after_reenabling() {
        // Construct control only: tests never start a worker or open devices.
        let settings = CueSettings::new(true);
        let initial = settings.0.epoch.load(Ordering::Acquire);
        assert!(settings.audible(initial));
        settings.set_deafened(true);
        assert!(!settings.audible(initial + 1));
        settings.set_deafened(false);
        assert!(!settings.audible(initial));
        assert!(settings.audible(initial + 1));
        settings.set_enabled(false);
        assert!(!settings.audible(initial + 2));
        settings.set_enabled(true);
        assert!(!settings.audible(initial + 1));
        assert!(settings.audible(initial + 2));
    }

    #[test]
    fn burst_queue_is_bounded_and_call_changes_invalidate_old_requests() {
        let settings = CueSettings::new(true);
        let (requests, mut rx) = mpsc::channel(4);
        let player = CuePlayer {
            requests,
            settings,
            own: Arc::default(),
        };
        for _ in 0..100 {
            player.play_peer(Cue::PeerJoin);
        }
        assert_eq!(rx.len(), 4);
        player.play(Cue::Join);
        let join = player
            .own
            .take(rx.try_recv().unwrap(), &player.settings)
            .unwrap();
        assert_eq!(join.cue, Cue::Join);
        assert!(player.settings.audible(join.epoch));
        player.settings.discard_pending();
        let stale = rx.try_recv().unwrap().unwrap();
        assert!(!player.settings.audible(stale.epoch));
        for _ in 0..100 {
            player.play_peer(Cue::PeerJoin);
        }
        assert_eq!(rx.len(), 4);
        player.play(Cue::Leave);
        let leave = player
            .own
            .take(rx.try_recv().unwrap(), &player.settings)
            .unwrap();
        assert_eq!(leave.cue, Cue::Leave);
        assert!(player.settings.audible(leave.epoch));
        player.play(Cue::Join);
        player.settings.discard_pending();
        player.play(Cue::Leave);
        let newest = player.own.take(None, &player.settings).unwrap();
        assert_eq!(newest.cue, Cue::Leave);
        assert!(player.own.take(None, &player.settings).is_none());
        let settings = player.settings();
        drop(player);
        assert!(!settings.audible(newest.epoch));
    }
}
