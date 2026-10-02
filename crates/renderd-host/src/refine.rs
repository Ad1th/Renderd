//! Re-encoding the screen once it goes still.
//!
//! `ScreenCaptureKit` only delivers a frame when something on screen changes,
//! and the encoder only ever sees what it is given. On a slow link the frames
//! of a scroll are coded coarsely to fit the bitrate, and when the scroll
//! stops the last of them stays on the viewer's screen exactly as coarse as it
//! was coded — blurry text, indefinitely, on a desktop that is now perfectly
//! still. Worse, if capture skipped the final frames of the scroll because the
//! send queue was deep, the viewer is left showing a frame that is not even
//! the current content.
//!
//! The second case is common, and not only because of capture skipping. On a
//! slow link the low-latency rate controller codes a big change — a scroll, a
//! window switch — as one large frame and then *drops* the frames after it
//! until its budget recovers: on a 2 Mbps link a full-screen change of 1080p
//! text came out as one 66-148 KB frame followed by six of eight frames
//! dropped. If the screen stops moving inside that window, the frame that
//! shows where it stopped is one of the dropped ones.
//!
//! [`StaticRefiner`] keeps the newest captured surface and, once the screen
//! has been still for a moment, hands it to the encoder again until a couple
//! of those passes actually come out of it. The first guarantees the viewer
//! ends on the current screen; the next spends the now-idle budget on the
//! detail the motion frames could not afford. A still screen costs nothing
//! once the passes are done.

use std::time::{Duration, Instant};

/// Stillness before the first re-encode pass.
pub const FIRST_PASS_AFTER: Duration = Duration::from_millis(60);

/// Time between re-encode passes.
///
/// After a large frame the low-latency rate controller drops every frame
/// until its budget has recovered — after a full-screen change on a 2 Mbps
/// link, most of a second. Passes are spaced so that a run of them spans that.
pub const PASS_SPACING: Duration = Duration::from_millis(100);

/// Passes that must actually come out of the encoder after the screen goes
/// still: the first makes sure the viewer ends on the current screen, the
/// second spends the idle budget on detail.
pub const PASSES: u32 = 2;

/// Passes tried before giving up, whether or not they came out.
pub const MAX_ATTEMPTS: u32 = 12;

/// Send-queue depth above which a pass waits: the link is still busy with
/// real frames, and a pass would only queue behind them.
pub const MAX_QUEUE: Duration = Duration::from_millis(10);

/// When the next re-encode pass is due.
#[derive(Debug, Default)]
pub struct RefineSchedule {
    passes_left: u32,
    attempts_left: u32,
    next_at: Option<Instant>,
}

impl RefineSchedule {
    /// The screen changed at `now`: start over.
    pub fn on_capture(&mut self, now: Instant) {
        self.passes_left = PASSES;
        self.attempts_left = MAX_ATTEMPTS;
        self.next_at = Some(now + FIRST_PASS_AFTER);
    }

    /// When the next pass is due, or `None` once done.
    #[must_use]
    pub const fn next_at(&self) -> Option<Instant> {
        self.next_at
    }

    /// Whether a pass should run at `now`, consuming an attempt if so. A due
    /// pass waits another [`PASS_SPACING`] while `link_busy`.
    pub fn take_pass(&mut self, now: Instant, link_busy: bool) -> bool {
        match self.next_at {
            Some(at) if now >= at => {}
            _ => return false,
        }
        if link_busy {
            self.next_at = Some(now + PASS_SPACING);
            return false;
        }
        self.attempts_left = self.attempts_left.saturating_sub(1);
        self.next_at = (self.attempts_left > 0).then(|| now + PASS_SPACING);
        true
    }

    /// The last pass came out of the encoder rather than being dropped.
    pub fn on_landed(&mut self) {
        self.passes_left = self.passes_left.saturating_sub(1);
        if self.passes_left == 0 {
            self.next_at = None;
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::StaticRefiner;

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::{Arc, Condvar, Mutex, PoisonError};
    use std::thread::JoinHandle;
    use std::time::Instant;

    use renderd_vt_sys::{HeldSurface, IoSurface};

    use super::{RefineSchedule, MAX_QUEUE};
    use crate::encode::{EncodePipeline, Submitted};

    /// The newest captured surface, held so the capture pool leaves it alone.
    struct Latest {
        surface: HeldSurface,
        pts_ns: i64,
        captured: Instant,
    }

    #[derive(Default)]
    struct State {
        latest: Option<Latest>,
        schedule: RefineSchedule,
        /// The encoder's output count when the last pass was submitted.
        pending: Option<u64>,
        stop: bool,
    }

    #[derive(Default)]
    struct Shared {
        state: Mutex<State>,
        wake: Condvar,
    }

    /// Re-encodes the newest captured surface a few times once the screen goes
    /// still (see the module documentation).
    pub struct StaticRefiner {
        shared: Arc<Shared>,
        thread: Mutex<Option<JoinHandle<()>>>,
    }

    impl std::fmt::Debug for StaticRefiner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("StaticRefiner").finish_non_exhaustive()
        }
    }

    impl StaticRefiner {
        /// Starts the refiner thread feeding `encode`.
        #[must_use]
        pub fn start(encode: Arc<EncodePipeline>) -> Self {
            let shared = Arc::new(Shared::default());
            let worker = Arc::clone(&shared);
            let thread = std::thread::Builder::new()
                .name("renderd-refine".into())
                .spawn(move || run(&worker, &encode))
                .ok();
            Self {
                shared,
                thread: Mutex::new(thread),
            }
        }

        /// Records a freshly captured surface, whether or not it was encoded.
        pub fn on_capture(&self, surface: &IoSurface, pts_ns: i64) {
            let now = Instant::now();
            let latest = Latest {
                surface: HeldSurface::new(surface.clone()),
                pts_ns,
                captured: now,
            };
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.latest = Some(latest);
            state.schedule.on_capture(now);
            state.pending = None;
            drop(state);
            self.shared.wake.notify_one();
        }

        /// Stops the thread and releases the held surface.
        pub fn stop(&self) {
            {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                state.stop = true;
                state.latest = None;
            }
            self.shared.wake.notify_one();
            let handle = self
                .thread
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(handle) = handle {
                let _ = handle.join();
            }
        }
    }

    impl Drop for StaticRefiner {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn run(shared: &Shared, encode: &EncodePipeline) {
        let mut passes: u64 = 0;
        let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if state.stop {
                break;
            }
            if let Some(before) = state.pending {
                if encode.frames_output() > before {
                    state.pending = None;
                    state.schedule.on_landed();
                }
            }
            let now = Instant::now();
            match state.schedule.next_at() {
                None => {
                    // Every pass ran: hand the surface back to the capture pool.
                    state.latest = None;
                    state = shared
                        .wake
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                    continue;
                }
                Some(at) if now < at => {
                    state = shared
                        .wake
                        .wait_timeout(state, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                    continue;
                }
                Some(_) => {}
            }

            let busy = encode.link().queue_delay(encode.current_bitrate()) > MAX_QUEUE;
            if !state.schedule.take_pass(now, busy) {
                continue;
            }
            let Some(latest) = state.latest.as_ref() else {
                continue;
            };
            let surface = latest.surface.surface().clone();
            let elapsed = now.saturating_duration_since(latest.captured);
            let pts_ns = latest
                .pts_ns
                .saturating_add(i64::try_from(elapsed.as_nanos()).unwrap_or(i64::MAX));
            let before = encode.frames_output();
            state.pending = Some(before);
            drop(state);

            match encode.submit(&surface, pts_ns) {
                Ok(Submitted::Encoded) => {
                    passes += 1;
                    if passes <= 3 || passes % 500 == 0 {
                        tracing::debug!(passes, "Re-encoded the still screen");
                    }
                }
                Ok(Submitted::Skipped) => {}
                Err(e) => tracing::debug!("re-encode pass failed: {e}"),
            }
            state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passes_run_until_enough_of_them_land() {
        let mut schedule = RefineSchedule::default();
        let t0 = Instant::now();
        assert!(!schedule.take_pass(t0, false), "nothing captured yet");
        schedule.on_capture(t0);
        assert!(!schedule.take_pass(t0 + Duration::from_millis(30), false));

        let mut at = t0 + FIRST_PASS_AFTER;
        // The encoder drops the first two passes while it repays a big frame.
        for _ in 0..2 {
            assert!(schedule.take_pass(at, false));
            at += PASS_SPACING;
        }
        for _ in 0..PASSES {
            assert!(schedule.take_pass(at, false));
            schedule.on_landed();
            at += PASS_SPACING;
        }
        assert!(
            schedule.next_at().is_none(),
            "done until the screen changes"
        );
        assert!(!schedule.take_pass(at + Duration::from_secs(1), false));
    }

    #[test]
    fn test_attempts_are_bounded() {
        let mut schedule = RefineSchedule::default();
        let t0 = Instant::now();
        schedule.on_capture(t0);
        let mut at = t0 + FIRST_PASS_AFTER;
        for _ in 0..MAX_ATTEMPTS {
            assert!(schedule.take_pass(at, false));
            at += PASS_SPACING;
        }
        assert!(schedule.next_at().is_none());
    }

    #[test]
    fn test_a_new_capture_restarts_the_passes() {
        let mut schedule = RefineSchedule::default();
        let t0 = Instant::now();
        schedule.on_capture(t0);
        assert!(schedule.take_pass(t0 + FIRST_PASS_AFTER, false));
        let t1 = t0 + Duration::from_millis(70);
        schedule.on_capture(t1);
        assert_eq!(schedule.next_at(), Some(t1 + FIRST_PASS_AFTER));
    }

    #[test]
    fn test_busy_link_postpones_without_using_an_attempt() {
        let mut schedule = RefineSchedule::default();
        let t0 = Instant::now();
        schedule.on_capture(t0);
        let due = t0 + FIRST_PASS_AFTER;
        assert!(!schedule.take_pass(due, true));
        assert_eq!(schedule.next_at(), Some(due + PASS_SPACING));
        let mut at = due + PASS_SPACING;
        for _ in 0..MAX_ATTEMPTS {
            assert!(schedule.take_pass(at, false));
            at += PASS_SPACING;
        }
    }
}
