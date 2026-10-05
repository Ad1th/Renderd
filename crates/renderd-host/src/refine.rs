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
//!
//! On a slow link the encoder also runs below the display's resolution (see
//! [`crate::scale`]), and no number of passes brings back pixels that were
//! never encoded: text stays soft however long the screen is still. So once
//! the screen has settled, the refiner moves the encoder up to the display's
//! native size for one full-resolution keyframe and a couple of refinement
//! passes ([`plan_sharpen`]), and drops back to the link's size as soon as the
//! screen moves again ([`MotionDetector`]).

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

/// Stillness before the encoder is moved up to the display's native size.
///
/// Long enough that typing and cursor movement do not trigger it, short
/// enough to feel like the picture sharpening as soon as you stop.
pub const SHARPEN_AFTER: Duration = Duration::from_millis(500);

/// Minimum time at the link's size after moving, before sharpening again.
pub const SHARPEN_COOLDOWN: Duration = Duration::from_secs(2);

/// Longest a full-resolution keyframe may take to drain at the current
/// bitrate. A text-heavy desktop keyframe measures about half a bit per pixel.
pub const MAX_SHARPEN_DRAIN: Duration = Duration::from_millis(1200);

/// Captures within [`MOTION_WINDOW`] that count as the screen moving again.
pub const MOTION_FRAMES: usize = 4;

/// See [`MOTION_FRAMES`].
pub const MOTION_WINDOW: Duration = Duration::from_millis(300);

/// Approximate size of a full-resolution desktop keyframe, in bits per pixel.
const KEYFRAME_BITS_PER_PIXEL: f64 = 0.5;

/// Whether a native-size keyframe can drain in a tolerable time at
/// `bitrate_kbps`.
#[must_use]
pub fn sharpen_affordable(native: (u32, u32), bitrate_kbps: u32) -> bool {
    if bitrate_kbps == 0 {
        return false;
    }
    let bits = f64::from(native.0) * f64::from(native.1) * KEYFRAME_BITS_PER_PIXEL;
    bits / (f64::from(bitrate_kbps) * 1_000.0) <= MAX_SHARPEN_DRAIN.as_secs_f64()
}

/// What the refiner should do about sharpening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenPlan {
    /// Sharpen now.
    Now,
    /// Sharpen at this time if nothing changes.
    At(Instant),
    /// Nothing to do until the screen changes.
    Never,
}

/// Decides whether and when to move the encoder up to `native`.
///
/// `encoded` is the size the encoder runs at, `captured` when the screen last
/// changed and `last_restore` when the encoder last went back down.
#[must_use]
pub fn plan_sharpen(
    now: Instant,
    encoded: (u32, u32),
    native: (u32, u32),
    bitrate_kbps: u32,
    captured: Instant,
    last_restore: Option<Instant>,
) -> SharpenPlan {
    if encoded.0 >= native.0 && encoded.1 >= native.1 {
        return SharpenPlan::Never;
    }
    if !sharpen_affordable(native, bitrate_kbps) {
        return SharpenPlan::Never;
    }
    let mut ready = captured + SHARPEN_AFTER;
    if let Some(at) = last_restore {
        ready = ready.max(at + SHARPEN_COOLDOWN);
    }
    if now >= ready {
        SharpenPlan::Now
    } else {
        SharpenPlan::At(ready)
    }
}

/// Tells a screen that is moving from one that merely changed.
///
/// A blinking cursor or a clock produces an isolated capture now and then;
/// scrolling, dragging and video produce a run of them. Only a run is allowed
/// to undo a sharpened picture.
#[derive(Debug, Default)]
pub struct MotionDetector {
    recent: std::collections::VecDeque<Instant>,
}

impl MotionDetector {
    /// Records a capture at `now`; returns whether the screen is moving.
    pub fn on_capture(&mut self, now: Instant) -> bool {
        self.recent.push_back(now);
        while self
            .recent
            .front()
            .is_some_and(|&t| now.saturating_duration_since(t) > MOTION_WINDOW)
        {
            self.recent.pop_front();
        }
        self.recent.len() >= MOTION_FRAMES
    }
}

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

    use super::{plan_sharpen, MotionDetector, RefineSchedule, SharpenPlan, MAX_QUEUE};
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
        motion: MotionDetector,
        /// The display's size: what a sharpened picture is encoded at.
        native: (u32, u32),
        /// Whether the current still screen has already been sharpened.
        sharpen_done: bool,
        /// When the encoder last went back down from a sharpened size.
        last_restore: Option<Instant>,
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
        encode: Arc<EncodePipeline>,
        shared: Arc<Shared>,
        thread: Mutex<Option<JoinHandle<()>>>,
    }

    impl std::fmt::Debug for StaticRefiner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("StaticRefiner").finish_non_exhaustive()
        }
    }

    impl StaticRefiner {
        /// Starts the refiner thread feeding `encode`. `native` is the size of
        /// the captured surfaces, which a sharpened picture is encoded at.
        #[must_use]
        pub fn start(encode: Arc<EncodePipeline>, native: (u32, u32)) -> Self {
            let shared = Arc::new(Shared::default());
            shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .native = native;
            let worker = Arc::clone(&shared);
            let worker_encode = Arc::clone(&encode);
            let thread = std::thread::Builder::new()
                .name("renderd-refine".into())
                .spawn(move || run(&worker, &worker_encode))
                .ok();
            Self {
                encode,
                shared,
                thread: Mutex::new(thread),
            }
        }

        /// Records a freshly captured surface, whether or not it was encoded.
        ///
        /// Call this *before* submitting the surface: if the screen has started
        /// moving again after a sharpened still, the encoder goes back to the
        /// link's size here, so that the frame is encoded at it.
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
            state.sharpen_done = false;
            let moving = state.motion.on_capture(now);
            let restore = moving && self.encode.is_sharpened();
            if restore {
                state.last_restore = Some(now);
            }
            drop(state);
            if restore {
                if let Err(e) = self.encode.restore_size() {
                    tracing::warn!("could not leave the sharpened size: {e}");
                }
            }
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

    /// Whether the settled screen should be re-encoded at the display's size.
    fn sharpen_wanted(state: &State, encode: &EncodePipeline, now: Instant) -> SharpenPlan {
        let (Some(latest), Some(encoded)) = (state.latest.as_ref(), encode.encoded_size()) else {
            return SharpenPlan::Never;
        };
        if state.sharpen_done || encode.is_sharpened() || state.native == (0, 0) {
            return SharpenPlan::Never;
        }
        plan_sharpen(
            now,
            encoded,
            state.native,
            encode.current_bitrate(),
            latest.captured,
            state.last_restore,
        )
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
                    match sharpen_wanted(&state, encode, now) {
                        SharpenPlan::Now => {
                            state.sharpen_done = true;
                            let native = state.native;
                            drop(state);
                            let landed = match encode.sharpen(native) {
                                Ok(changed) => changed,
                                Err(e) => {
                                    tracing::debug!("sharpen failed: {e}");
                                    false
                                }
                            };
                            state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
                            if landed {
                                // The new session starts with a keyframe of the
                                // still screen, then refines it like any other.
                                state.schedule.on_capture(Instant::now());
                                state.pending = None;
                            }
                            continue;
                        }
                        SharpenPlan::At(at) => {
                            state = shared
                                .wake
                                .wait_timeout(state, at.saturating_duration_since(now))
                                .unwrap_or_else(PoisonError::into_inner)
                                .0;
                            continue;
                        }
                        SharpenPlan::Never => {}
                    }
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

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn test_sharpen_waits_for_stillness_then_goes() {
        let t0 = Instant::now();
        let low = (960, 544);
        let native = (1920, 1080);
        assert_eq!(
            plan_sharpen(at(t0, 100), low, native, 3_000, t0, None),
            SharpenPlan::At(t0 + SHARPEN_AFTER)
        );
        assert_eq!(
            plan_sharpen(at(t0, 600), low, native, 3_000, t0, None),
            SharpenPlan::Now
        );
    }

    #[test]
    fn test_sharpen_not_needed_at_native_size() {
        let t0 = Instant::now();
        assert_eq!(
            plan_sharpen(at(t0, 900), (1920, 1080), (1920, 1080), 3_000, t0, None),
            SharpenPlan::Never
        );
    }

    #[test]
    fn test_sharpen_skipped_when_the_keyframe_would_clog_the_link() {
        // ~1 Mbit of keyframe on a 500 kbps link is two seconds of queue.
        assert!(!sharpen_affordable((1920, 1080), 500));
        assert!(sharpen_affordable((1920, 1080), 1_500));
        assert!(sharpen_affordable((1920, 1080), 3_000));
        assert!(!sharpen_affordable((1920, 1080), 0));
    }

    #[test]
    fn test_sharpen_cools_down_after_returning_to_the_link_size() {
        let t0 = Instant::now();
        let restored = at(t0, 1_000);
        assert_eq!(
            plan_sharpen(
                at(t0, 1_700),
                (960, 544),
                (1920, 1080),
                3_000,
                at(t0, 1_000),
                Some(restored)
            ),
            SharpenPlan::At(restored + SHARPEN_COOLDOWN)
        );
    }

    #[test]
    fn test_isolated_captures_are_not_motion() {
        let mut motion = MotionDetector::default();
        let t0 = Instant::now();
        // A blinking cursor: one capture every half second.
        for i in 0..10 {
            assert!(!motion.on_capture(at(t0, i * 500)));
        }
    }

    #[test]
    fn test_a_run_of_captures_is_motion() {
        let mut motion = MotionDetector::default();
        let t0 = Instant::now();
        assert!(!motion.on_capture(at(t0, 0)));
        assert!(!motion.on_capture(at(t0, 16)));
        assert!(!motion.on_capture(at(t0, 33)));
        assert!(motion.on_capture(at(t0, 50)), "four within 300 ms");
    }
}
