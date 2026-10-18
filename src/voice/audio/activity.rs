use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MIN_MEAN_SQUARE: f64 = 0.0001;
const HOLD: Duration = Duration::from_millis(200);

pub(super) struct CaptureActivity {
    started: Instant,
    last_active_ms: AtomicU64,
}

impl Default for CaptureActivity {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            last_active_ms: AtomicU64::new(0),
        }
    }
}

impl CaptureActivity {
    pub(super) fn observe(&self, power: f64, samples: usize, now: Instant) {
        // ponytail: energy also detects loud noise; use the shared VAD when voice processing exists.
        if samples > 0 && power / samples as f64 >= MIN_MEAN_SQUARE {
            self.last_active_ms
                .store(self.timestamp(now), Ordering::Relaxed);
        }
    }

    pub(super) fn active(&self, now: Instant) -> bool {
        let last = self.last_active_ms.load(Ordering::Relaxed);
        last != 0 && self.timestamp(now).saturating_sub(last) < HOLD.as_millis() as u64
    }

    pub(super) fn clear(&self) {
        self.last_active_ms.store(0, Ordering::Relaxed);
    }

    fn timestamp(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started).as_millis() as u64 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_noise_and_invalid_samples_do_not_activate_or_extend_the_hold() {
        let activity = CaptureActivity::default();
        let start = activity.started;
        assert!(!activity.active(start));
        for (power, samples) in [(0., 960), (0.000001 * 960., 960), (f64::NAN, 960), (1., 0)] {
            activity.observe(power, samples, start);
            assert!(!activity.active(start));
        }
        activity.observe(0.01 * 960., 960, start);
        assert!(activity.active(start));
        let quiet = start + Duration::from_millis(199);
        activity.observe(0., 960, quiet);
        assert!(activity.active(quiet));
        assert!(!activity.active(start + HOLD));
        activity.observe(0.01 * 960., 960, start + HOLD);
        assert!(activity.active(start + HOLD));
        activity.clear();
        assert!(!activity.active(start + HOLD));
    }
}
