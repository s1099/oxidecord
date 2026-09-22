//! Playing a video attachment: starting the decoder, pumping its frames onto
//! the foreground, and tearing it down again.
//!
//! Frames arrive from [`crate::platform::video`] on the decoder's own thread
//! and cross to gpui over a channel, the same way gateway events do. Each one
//! becomes a [`RenderImage`] and replaces the last, and the last is handed
//! back to gpui as it goes — see [`HomeScreen::show_frame`] for why that part
//! isn't optional.

use std::sync::Arc;
use std::time::Duration;

use gpui::*;
use gpui_component::slider::{SliderEvent, SliderState};
use image::{Frame, ImageBuffer};

use crate::platform::video::{VideoEvent, VideoFrame, VideoPlayer};
use crate::screens::home::HomeScreen;
use crate::screens::home::state::{MediaKey, PlaybackState, VideoPlayback};

impl HomeScreen {
    /// Starts playing one video attachment, stopping whatever was playing.
    ///
    /// `target` is the size frames are decoded at, in physical pixels. Asking
    /// the decoder for the size the card actually draws — rather than the
    /// source's own — is what keeps a playing video down to a few hundred
    /// kilobytes of frames instead of tens of megabytes.
    pub(in crate::screens::home) fn play_video(
        &mut self,
        key: MediaKey,
        url: String,
        target: (u32, u32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_video(window, cx);

        let (tx, rx) = futures::channel::mpsc::unbounded::<VideoEvent>();
        let player = VideoPlayer::open(url, target, tx);

        // The scrubber runs 0..1 over the clip. `set_value` doesn't emit, so
        // following the play position here can't be mistaken for a drag.
        let scrubber = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        cx.subscribe_in(
            &scrubber,
            window,
            |this, _, SliderEvent::Change(value), _window, cx| {
                this.seek_video(value.start(), cx);
            },
        )
        .detach();

        self.video = Some(VideoPlayback {
            key,
            player,
            frame: None,
            duration: Duration::ZERO,
            position: Duration::ZERO,
            state: PlaybackState::Loading,
            scrubber,
        });

        cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                let handled = this.update_in(cx, |this, window, cx| {
                    this.handle_video_event(key, event, window, cx)
                });
                // The screen is gone, or this playback was replaced by another;
                // either way nothing is listening, so let the sender close and
                // the decoder stop.
                if !matches!(handled, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        cx.notify();
    }

    /// Ends the current playback and releases the frame it left on screen.
    pub(in crate::screens::home) fn stop_video(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(playback) = self.video.take() else {
            return;
        };
        // Dropping the player stops the decoder; the frame it last handed over
        // is ours to give back.
        if let Some(frame) = playback.frame {
            let _ = window.drop_image(frame);
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn toggle_video_paused(&mut self, cx: &mut Context<Self>) {
        let Some(playback) = &self.video else {
            return;
        };
        playback.player.set_paused(!playback.player.is_paused());
        cx.notify();
    }

    pub(in crate::screens::home) fn toggle_video_muted(&mut self, cx: &mut Context<Self>) {
        let Some(playback) = &self.video else {
            return;
        };
        playback.player.set_muted(!playback.player.is_muted());
        cx.notify();
    }

    /// Jumps to `fraction` of the way through the clip, from a click on the
    /// progress bar. A source that never reported a duration can't be seeked
    /// meaningfully, so the click is ignored rather than guessed at.
    pub(in crate::screens::home) fn seek_video(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let Some(playback) = &mut self.video else {
            return;
        };
        if playback.duration.is_zero() {
            return;
        }

        let position = playback.duration.mul_f32(fraction.clamp(0., 1.));
        playback.player.seek(position);
        // Moved now rather than waiting for the next frame, so the handle
        // follows the click instead of lagging a decode behind it.
        playback.position = position;
        if playback.state == PlaybackState::Ended {
            playback.state = PlaybackState::Playing;
        }
        cx.notify();
    }

    /// Returns whether the playback this event belongs to is still the current
    /// one; once it isn't, the pump feeding it stops.
    fn handle_video_event(
        &mut self,
        key: MediaKey,
        event: VideoEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // A late event from a playback the user has already moved on from.
        // Dropping it matters: its frames would otherwise be drawn over
        // whatever is playing now.
        if self
            .video
            .as_ref()
            .is_none_or(|playback| playback.key != key)
        {
            return false;
        }

        match event {
            VideoEvent::Opened { duration } => {
                if let Some(playback) = &mut self.video {
                    playback.duration = duration;
                }
            }
            VideoEvent::Frame(frame) => self.show_frame(frame, window, cx),
            VideoEvent::Ended => {
                if let Some(playback) = &mut self.video {
                    playback.state = PlaybackState::Ended;
                }
            }
            VideoEvent::Failed(reason) => {
                if let Some(playback) = &mut self.video {
                    playback.state = PlaybackState::Failed(reason);
                }
            }
        }

        cx.notify();
        true
    }

    /// Puts a decoded frame on screen and gives the previous one back to gpui.
    ///
    /// The eviction is the important half. `RenderImage` is uploaded into the
    /// window's sprite atlas on first paint and kept there against its id, and
    /// a video mints a new id thirty times a second — so without `drop_image`
    /// the atlas grows for as long as the clip plays.
    fn show_frame(&mut self, frame: VideoFrame, window: &mut Window, cx: &mut Context<Self>) {
        let Some(playback) = &mut self.video else {
            return;
        };

        // The decoder counts frames it has handed over and stops once too many
        // are outstanding, so this has to be reported however the frame is
        // handled below — including when it can't be turned into an image.
        playback.player.frame_consumed();

        let position = frame.position;
        let Some(image) = to_image(frame) else {
            return;
        };

        playback.position = position;
        playback.state = PlaybackState::Playing;
        if let Some(previous) = playback.frame.replace(image) {
            let _ = window.drop_image(previous);
        }

        let fraction = fraction_of(position, playback.duration);
        let scrubber = playback.scrubber.clone();
        scrubber.update(cx, |scrubber, cx| scrubber.set_value(fraction, window, cx));
    }
}

fn fraction_of(position: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 0.;
    }
    (position.as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.)
}

/// Wraps decoded pixels as something `img()` can draw.
///
/// No conversion happens here: the decoder is asked for BGRA, which is the
/// layout `RenderImage` stores, so this is a move rather than a pass over the
/// pixels. `None` only when the buffer doesn't match the reported size, which
/// would mean the decoder contradicted itself.
fn to_image(frame: VideoFrame) -> Option<Arc<RenderImage>> {
    let buffer = ImageBuffer::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}
