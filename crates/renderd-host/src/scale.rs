//! Choosing the encoded picture size for a session's bitrate ceiling.
//!
//! A hardware encoder at a fixed bitrate cannot make a picture of any size
//! look right. Measured on an M3 with a text-heavy desktop at 60 fps and a
//! 6 Mbps target: at 1920×1080 the low-latency rate controller could not get
//! below ~8.5 Mbps (H.264) / ~7.7 Mbps (HEVC) and dropped a quarter of the
//! frames trying; at 1600×900 it held ~6.4 / ~6.0 Mbps with no drops. The
//! difference is bits per pixel per frame: about 0.069 is what dense desktop
//! content needs, so the host picks the largest standard height whose pixel
//! rate the ceiling can feed at that density, and lets `ScreenCaptureKit`
//! scale the display down to it. The virtual display itself stays at the
//! viewer's native size, so windows and text keep their layout.

/// Bits per pixel per frame the encoder needs for a clean desktop picture.
///
/// HEVC fits at 900p60 in 6 Mbps (0.069); H.264 is ~6% less efficient.
#[must_use]
pub fn bits_per_pixel_floor(codec: &str) -> f64 {
    if codec.eq_ignore_ascii_case("h264") {
        0.069
    } else {
        0.065
    }
}

/// Heights tried, tallest first, when the display is taller than the budget.
const STANDARD_HEIGHTS: [u32; 7] = [2160, 1440, 1200, 1080, 900, 720, 540];

/// Returns the width × height to encode for a `display_w` × `display_h`
/// display at `fps`, given the session's bitrate ceiling.
///
/// `max_height` of 0 means automatic. A non-zero value is a hard cap and wins
/// over the automatic choice. The result never exceeds the display, keeps its
/// aspect ratio, and has even dimensions (4:2:0 chroma needs them).
#[must_use]
pub fn stream_size(
    display_w: u32,
    display_h: u32,
    fps: u32,
    codec: &str,
    ceiling_kbps: u32,
    max_height: u32,
) -> (u32, u32) {
    let display_w = display_w.max(2);
    let display_h = display_h.max(2);
    let cap = if max_height == 0 {
        display_h
    } else {
        max_height.min(display_h)
    };

    let budget = f64::from(ceiling_kbps) * 1_000.0 / f64::from(fps.max(1));
    let floor = bits_per_pixel_floor(codec);
    let fits = |h: u32| {
        let (w, h) = scaled(display_w, display_h, h);
        max_height != 0 || budget / f64::from(w * h) >= floor
    };

    let height = std::iter::once(cap)
        .chain(STANDARD_HEIGHTS.iter().copied().filter(|&h| h < cap))
        .find(|&h| fits(h))
        .unwrap_or_else(|| STANDARD_HEIGHTS[STANDARD_HEIGHTS.len() - 1].min(cap));

    scaled(display_w, display_h, height)
}

/// `display_w` × `display_h` scaled to `height`, rounded to even dimensions.
fn scaled(display_w: u32, display_h: u32, height: u32) -> (u32, u32) {
    let height = height.min(display_h);
    let width = u64::from(display_w) * u64::from(height) / u64::from(display_h);
    let width = u32::try_from(width).unwrap_or(display_w);
    (even(width), even(height))
}

const fn even(v: u32) -> u32 {
    let v = if v < 2 { 2 } else { v };
    v & !1
}

/// Heights the encoder may drop to below the session's size.
///
/// Every rung is a multiple of 16 in both dimensions. A decoder pads a frame
/// up to whole 16-pixel blocks and crops it back using the stream's own
/// parameters, which the viewer cannot always read back from its platform
/// decoder; with no padding there is nothing to crop. The session's own size
/// needs no such care: the viewer knows it from the handshake.
const ADAPTIVE_HEIGHTS: [u32; 4] = [1152, 896, 720, 544];

/// Rounds to the nearest non-zero multiple of 16.
const fn align16(v: u32) -> u32 {
    let rounded = (v + 8) / 16 * 16;
    if rounded < 16 {
        16
    } else {
        rounded
    }
}

/// The sizes the encoder may use in a session of `native` size, largest first.
#[must_use]
pub fn ladder(native: (u32, u32)) -> Vec<(u32, u32)> {
    let (w, h) = (native.0.max(2), native.1.max(2));
    let mut rungs = vec![(w, h)];
    for rung in ADAPTIVE_HEIGHTS {
        if rung < h {
            let width = u64::from(w) * u64::from(rung) / u64::from(h);
            rungs.push((align16(u32::try_from(width).unwrap_or(w)), rung));
        }
    }
    rungs
}

/// Whether `bitrate_kbps` affords `size` at `fps`, given `margin` times the
/// bits per pixel a clean picture needs (see [`bits_per_pixel_floor`]).
#[must_use]
pub fn fits(size: (u32, u32), fps: u32, codec: &str, bitrate_kbps: u32, margin: f64) -> bool {
    let budget = f64::from(bitrate_kbps) * 1_000.0 / f64::from(fps.max(1));
    budget / (f64::from(size.0) * f64::from(size.1)) >= bits_per_pixel_floor(codec) * margin
}

/// Share of a size's bitrate need below which the encoder leaves that size.
///
/// The gap between this and a full fit to step up is the hysteresis that
/// keeps the size from flapping. It also keeps a LAN session at its full
/// size: 1080p60 needs 8.1 Mbps, a little over the default starting 8 Mbps,
/// and a still desktop never gives the bitrate a reason to climb.
pub const STAY_MARGIN: f64 = 0.75;

/// How long the encoder waits before stepping down: the picture too big for
/// the bitrate ([`SizePolicy::Smooth`]), or the link congested
/// ([`SizePolicy::Sharp`]).
pub const DOWN_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// How long the encoder waits before stepping up: the bitrate affording a
/// bigger picture without congestion ([`SizePolicy::Smooth`]), or the link
/// stable since it was last unstable ([`SizePolicy::Sharp`]).
pub const UP_AFTER: std::time::Duration = std::time::Duration::from_secs(8);

/// Least time between two size changes. Each one costs a keyframe.
pub const MIN_CHANGE_GAP: std::time::Duration = std::time::Duration::from_secs(5);

/// A break in congestion this long ends a run of it.
///
/// A link that is falling behind does not report congestion on every tick:
/// capture skips frames while the send queue is deep, the queue drains, and a
/// report or two comes back clean before it fills again.
pub const CALM_GAP: std::time::Duration = std::time::Duration::from_millis(500);

/// How long a return to full size must hold before it counts as having
/// worked. With [`SizePolicy::Sharp`], one the link loses sooner doubles the
/// wait before the next try, so a link too slow for full-size motion is not
/// put through the same lag every few seconds.
pub const RETRY_HOLD: std::time::Duration = std::time::Duration::from_secs(30);

/// Longest a sharp session waits after the link was last unstable before
/// trying full size again.
pub const MAX_UP_AFTER: std::time::Duration = std::time::Duration::from_secs(64);

/// What the encoded size gives way to on a slow link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizePolicy {
    /// Stay at the session's size, motion included, and step down only while
    /// the link is unstable: congested for [`DOWN_AFTER`] with no
    /// [`CALM_GAP`] in between. Back to full size [`UP_AFTER`] after it was
    /// last unstable, or longer if the last return did not hold (see
    /// [`RETRY_HOLD`]). A shorter run of congestion, such as a keyframe
    /// draining, neither lowers the size nor delays its return.
    #[default]
    Sharp,
    /// Follow the bitrate: step down to the size it can feed, even on a steady
    /// link, so motion stays smooth, and leave a still screen to the refiner
    /// to sharpen (see [`crate::refine`]).
    Smooth,
}

/// Which way the encoded size wants to move, and since when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressure {
    Down(std::time::Instant),
    Up(std::time::Instant),
}

/// What [`SizePolicy::Sharp`] tracks about the link.
#[derive(Debug, Clone)]
struct Stability {
    /// First report of the current run of congestion.
    run_start: Option<std::time::Instant>,
    last_congested: Option<std::time::Instant>,
    /// When a run of congestion was last [`DOWN_AFTER`] long or more.
    last_unstable: Option<std::time::Instant>,
    /// When the encoder last went back to full size.
    last_return: Option<std::time::Instant>,
    /// How long after the link was last unstable to go back to full size.
    retry_after: std::time::Duration,
}

impl Stability {
    const fn new() -> Self {
        Self {
            run_start: None,
            last_congested: None,
            last_unstable: None,
            last_return: None,
            retry_after: UP_AFTER,
        }
    }

    /// Records one report; returns whether the link is unstable.
    fn on_report(&mut self, congested: bool, now: std::time::Instant) -> bool {
        if !congested {
            return false;
        }
        if !self.settling(now) {
            self.run_start = Some(now);
        }
        self.last_congested = Some(now);
        let unstable = self
            .run_start
            .is_some_and(|start| now.saturating_duration_since(start) >= DOWN_AFTER);
        if unstable {
            self.last_unstable = Some(now);
        }
        unstable
    }

    /// Whether the link reported congestion within the last [`CALM_GAP`].
    fn settling(&self, now: std::time::Instant) -> bool {
        self.last_congested
            .is_some_and(|at| now.saturating_duration_since(at) < CALM_GAP)
    }

    /// Whether the link has been stable long enough to go back to full size.
    fn settled(&self, now: std::time::Instant) -> bool {
        !self.settling(now)
            && !self
                .last_unstable
                .is_some_and(|at| now.saturating_duration_since(at) < self.retry_after)
    }

    /// The encoder went back to full size at `now`.
    const fn on_return(&mut self, now: std::time::Instant) {
        self.last_return = Some(now);
    }

    /// The encoder left full size at `now`. A return the link lost within
    /// [`RETRY_HOLD`] doubles the wait before the next one.
    fn on_leave(&mut self, now: std::time::Instant) {
        let failed = self
            .last_return
            .is_some_and(|at| now.saturating_duration_since(at) < RETRY_HOLD);
        self.retry_after = if failed {
            (self.retry_after * 2).min(MAX_UP_AFTER)
        } else {
            UP_AFTER
        };
    }
}

/// Moves the encoded picture size with the link.
///
/// The session's size is set once, from the bitrate *ceiling*, before anything
/// is known about the link. When the link turns out to carry far less, every
/// full-screen change is still coded at that size: measured on an M3 at
/// 2 Mbps, 83 KB at 1080p against 25 KB at 544p — a third of a second of link
/// time against a tenth.
///
/// With [`SizePolicy::Smooth`] the governor moves the encoder down the
/// [`ladder`] once the bitrate has been too low for the current size for
/// [`DOWN_AFTER`], and back up one rung at a time once it has afforded the
/// next one for [`UP_AFTER`] without congestion. With [`SizePolicy::Sharp`]
/// it ignores what the bitrate could feed, and moves down only once the link
/// has been congested for [`DOWN_AFTER`], then straight back to the session's
/// size [`UP_AFTER`] after that. Down is quick because every moment at the
/// wrong size is latency; up is slow because every change costs a keyframe.
#[derive(Debug, Clone)]
pub struct ResolutionGovernor {
    ladder: Vec<(u32, u32)>,
    fps: u32,
    codec: String,
    policy: SizePolicy,
    current: usize,
    pressure: Option<Pressure>,
    last_change: Option<std::time::Instant>,
    stability: Stability,
}

impl ResolutionGovernor {
    /// A governor for a session of `native` size at `fps`, encoding `codec`.
    #[must_use]
    pub fn new(native: (u32, u32), fps: u32, codec: &str, policy: SizePolicy) -> Self {
        Self {
            ladder: ladder(native),
            fps,
            codec: codec.to_string(),
            policy,
            current: 0,
            pressure: None,
            last_change: None,
            stability: Stability::new(),
        }
    }

    /// The size to start encoding at, given the session's starting bitrate.
    pub fn start(&mut self, bitrate_kbps: u32) -> (u32, u32) {
        self.current = match self.policy {
            SizePolicy::Sharp => 0,
            SizePolicy::Smooth => self.largest_fitting(bitrate_kbps, STAY_MARGIN),
        };
        self.ladder[self.current]
    }

    /// The size the encoder is at.
    #[must_use]
    pub fn current(&self) -> (u32, u32) {
        self.ladder[self.current]
    }

    /// Index of the largest rung `bitrate_kbps` affords at `margin`.
    fn largest_fitting(&self, bitrate_kbps: u32, margin: f64) -> usize {
        self.ladder
            .iter()
            .position(|&size| self.fits(size, bitrate_kbps, margin))
            .unwrap_or(self.ladder.len() - 1)
    }

    fn fits(&self, size: (u32, u32), bitrate_kbps: u32, margin: f64) -> bool {
        fits(size, self.fps, &self.codec, bitrate_kbps, margin)
    }

    /// Takes one ABR decision; returns the size to switch the encoder to, if
    /// it is time to.
    pub fn update(
        &mut self,
        bitrate_kbps: u32,
        congested: bool,
        now: std::time::Instant,
    ) -> Option<(u32, u32)> {
        let may_change = !self
            .last_change
            .is_some_and(|at| now.saturating_duration_since(at) < MIN_CHANGE_GAP);
        let next = match self.policy {
            SizePolicy::Sharp => self.sharp_next(bitrate_kbps, congested, now, may_change),
            SizePolicy::Smooth => self.smooth_next(bitrate_kbps, congested, now, may_change),
        }?;
        self.current = next;
        self.last_change = Some(now);
        Some(self.ladder[next])
    }

    /// [`SizePolicy::Smooth`]: the rung to move to, if it is time to.
    fn smooth_next(
        &mut self,
        bitrate_kbps: u32,
        congested: bool,
        now: std::time::Instant,
        may_change: bool,
    ) -> Option<usize> {
        self.pressure = self.bitrate_pressure(bitrate_kbps, congested, now);
        if !may_change {
            return None;
        }
        let next = match self.pressure? {
            Pressure::Down(since) if now.saturating_duration_since(since) >= DOWN_AFTER => {
                self.rung_below(bitrate_kbps)
            }
            Pressure::Up(since) if now.saturating_duration_since(since) >= UP_AFTER => {
                self.current - 1
            }
            _ => return None,
        };
        self.pressure = None;
        (next != self.current).then_some(next)
    }

    /// [`SizePolicy::Sharp`]: the rung to move to, if it is time to.
    fn sharp_next(
        &mut self,
        bitrate_kbps: u32,
        congested: bool,
        now: std::time::Instant,
        may_change: bool,
    ) -> Option<usize> {
        let unstable = self.stability.on_report(congested, now);
        if !may_change {
            return None;
        }
        let next = if unstable {
            self.rung_below(bitrate_kbps)
        } else if self.current > 0 && self.stability.settled(now) {
            0
        } else {
            return None;
        };
        if next == self.current {
            return None;
        }
        if next == 0 {
            self.stability.on_return(now);
        } else if self.current == 0 {
            self.stability.on_leave(now);
        }
        Some(next)
    }

    /// Where a step down goes: straight to a size the bitrate fits outright,
    /// with room to spare, and at least one rung down.
    fn rung_below(&self, bitrate_kbps: u32) -> usize {
        self.largest_fitting(bitrate_kbps, 1.0)
            .max(self.current + 1)
            .min(self.ladder.len() - 1)
    }

    /// [`SizePolicy::Smooth`]: down while the bitrate cannot feed the current
    /// size, up while it can feed the next one.
    fn bitrate_pressure(
        &self,
        bitrate_kbps: u32,
        congested: bool,
        now: std::time::Instant,
    ) -> Option<Pressure> {
        let too_big = !self.fits(self.ladder[self.current], bitrate_kbps, STAY_MARGIN);
        let room_above = self.current > 0
            && !congested
            && self.fits(self.ladder[self.current - 1], bitrate_kbps, 1.0);
        if too_big {
            Some(self.down_since(now))
        } else if room_above {
            Some(self.up_since(now))
        } else {
            None
        }
    }

    /// Down pressure, keeping its start if it was already pushing down.
    const fn down_since(&self, now: std::time::Instant) -> Pressure {
        match self.pressure {
            Some(Pressure::Down(since)) => Pressure::Down(since),
            _ => Pressure::Down(now),
        }
    }

    /// Up pressure, keeping its start if it was already pushing up.
    const fn up_since(&self, now: std::time::Instant) -> Pressure {
        match self.pressure {
            Some(Pressure::Up(since)) => Pressure::Up(since),
            _ => Pressure::Up(now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn test_ladder_rungs_are_block_aligned_below_native() {
        let rungs = ladder((1920, 1080));
        assert_eq!(
            rungs,
            vec![(1920, 1080), (1600, 896), (1280, 720), (960, 544)]
        );
        for &(w, h) in &rungs[1..] {
            assert_eq!((w % 16, h % 16), (0, 0));
        }
        assert_eq!(ladder((2560, 1440))[1], (2048, 1152));
        assert_eq!(ladder((1280, 720)), vec![(1280, 720), (960, 544)]);
    }

    #[test]
    fn test_start_picks_the_largest_rung_the_bitrate_affords() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        assert_eq!(gov.start(10_000), (1920, 1080));
        // The default starting bitrate keeps a LAN session at full size.
        assert_eq!(gov.start(8_000), (1920, 1080));
        assert_eq!(gov.start(5_000), (1600, 896));
        assert_eq!(gov.start(2_000), (960, 544));
        assert_eq!(gov.start(500), (960, 544), "the bottom rung is the floor");
    }

    /// A session sitting just under its size's full need must not drift down.
    #[test]
    fn test_holds_size_inside_the_hysteresis_band() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        gov.start(8_000);
        let t0 = Instant::now();
        for s in 0..30 {
            assert_eq!(gov.update(8_000, false, t0 + Duration::from_secs(s)), None);
        }
        assert_eq!(gov.current(), (1920, 1080));
    }

    #[test]
    fn test_steps_down_after_a_sustained_drop() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        gov.start(10_000);
        let t0 = Instant::now();
        assert_eq!(gov.update(3_000, true, t0), None);
        assert_eq!(gov.update(3_000, false, t0 + Duration::from_secs(1)), None);
        assert_eq!(
            gov.update(3_000, false, t0 + DOWN_AFTER),
            Some((960, 544)),
            "straight to the rung that fits"
        );
    }

    #[test]
    fn test_a_brief_dip_does_not_change_size() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        gov.start(10_000);
        let t0 = Instant::now();
        assert_eq!(gov.update(3_000, true, t0), None);
        assert_eq!(gov.update(10_000, false, t0 + Duration::from_secs(1)), None);
        assert_eq!(gov.update(3_000, false, t0 + Duration::from_secs(2)), None);
        assert_eq!(gov.update(3_000, false, t0 + Duration::from_secs(3)), None);
    }

    #[test]
    fn test_steps_up_one_rung_at_a_time_and_only_uncongested() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        gov.start(2_000);
        let t0 = Instant::now();
        assert_eq!(gov.update(10_000, false, t0), None);
        // Congestion resets the clock.
        assert_eq!(gov.update(10_000, true, t0 + Duration::from_secs(5)), None);
        let t1 = t0 + Duration::from_secs(6);
        assert_eq!(gov.update(10_000, false, t1), None);
        assert_eq!(gov.update(10_000, false, t1 + UP_AFTER), Some((1280, 720)));
        let t2 = t1 + UP_AFTER + MIN_CHANGE_GAP;
        assert_eq!(gov.update(10_000, false, t2), None);
        assert_eq!(gov.update(10_000, false, t2 + UP_AFTER), Some((1600, 896)));
    }

    #[test]
    fn test_changes_are_spaced_out() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        gov.start(10_000);
        let t0 = Instant::now();
        gov.update(6_000, false, t0);
        assert_eq!(gov.update(6_000, false, t0 + DOWN_AFTER), Some((1600, 896)));
        let t1 = t0 + DOWN_AFTER;
        gov.update(2_000, false, t1 + Duration::from_millis(100));
        assert_eq!(gov.update(2_000, false, t1 + Duration::from_secs(3)), None);
        assert_eq!(
            gov.update(2_000, false, t1 + MIN_CHANGE_GAP),
            Some((960, 544))
        );
    }

    #[test]
    fn test_bottom_rung_never_steps_further_down() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Smooth);
        gov.start(1_000);
        let t0 = Instant::now();
        for s in 0..20 {
            assert_eq!(gov.update(1_000, false, t0 + Duration::from_secs(s)), None);
        }
    }

    /// Feeds one report every 100 ms from `from` for `ticks` reports, congested
    /// where `congested(tick)` says so; returns the first size change and when.
    fn run_reports(
        gov: &mut ResolutionGovernor,
        bitrate_kbps: u32,
        from: Instant,
        ticks: u32,
        congested: impl Fn(u32) -> bool,
    ) -> Option<(Duration, (u32, u32))> {
        (0..ticks).find_map(|tick| {
            let after = Duration::from_millis(u64::from(tick) * 100);
            gov.update(bitrate_kbps, congested(tick), from + after)
                .map(|size| (after, size))
        })
    }

    /// Reports at 2 Mbps, congested or not, every 100 ms from `from` until the
    /// size changes; returns when, and to what.
    fn until_change(
        gov: &mut ResolutionGovernor,
        from: Instant,
        congested: bool,
    ) -> (Instant, (u32, u32)) {
        let (after, size) =
            run_reports(gov, 2_000, from, 2_000, |_| congested).expect("the size changes");
        (from + after, size)
    }

    /// A sharp 1080p60 HEVC session, started.
    fn sharp_governor() -> ResolutionGovernor {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Sharp);
        gov.start(8_000);
        gov
    }

    #[test]
    fn test_sharp_starts_at_full_size_whatever_the_bitrate() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc", SizePolicy::Sharp);
        assert_eq!(gov.start(2_000), (1920, 1080));
        assert_eq!(gov.start(500), (1920, 1080));
    }

    /// A slow link that keeps up is not a reason to blur the picture.
    #[test]
    fn test_sharp_holds_full_size_on_a_steady_slow_link() {
        let mut gov = sharp_governor();
        assert_eq!(
            run_reports(&mut gov, 2_000, Instant::now(), 600, |_| false),
            None
        );
        assert_eq!(gov.current(), (1920, 1080));
    }

    /// Congestion on and off, never a calm half-second, is an unstable link.
    #[test]
    fn test_sharp_steps_down_once_the_link_stays_unstable() {
        let mut gov = sharp_governor();
        let change = run_reports(&mut gov, 2_000, Instant::now(), 100, |tick| tick % 3 != 1);
        assert_eq!(
            change,
            Some((DOWN_AFTER, (960, 544))),
            "straight to the rung the bitrate fits"
        );
    }

    #[test]
    fn test_sharp_rides_out_bursts_of_congestion() {
        let mut gov = sharp_governor();
        // 1.8 s congested, then a calm 600 ms, over and over: a big frame
        // draining now and then, not a link that cannot keep up.
        let change = run_reports(&mut gov, 2_000, Instant::now(), 600, |tick| tick % 24 < 18);
        assert_eq!(change, None);
        assert_eq!(gov.current(), (1920, 1080));
    }

    #[test]
    fn test_sharp_goes_straight_back_to_full_size_once_stable() {
        let mut gov = sharp_governor();
        let (down_at, _) = until_change(&mut gov, Instant::now(), true);
        // The bitrate never recovers on a still desktop.
        let (up_at, size) = until_change(&mut gov, down_at + CALM_GAP, false);
        assert_eq!((up_at - down_at, size), (UP_AFTER, (1920, 1080)));
    }

    /// A keyframe draining (a sharpened still screen, a periodic IDR) queues
    /// for a moment; that does not make the link unstable again.
    #[test]
    fn test_sharp_a_short_burst_does_not_delay_the_return() {
        let mut gov = sharp_governor();
        let (down_at, _) = until_change(&mut gov, Instant::now(), true);
        let from = down_at + CALM_GAP;
        let change = run_reports(&mut gov, 2_000, from, 200, |tick| (35..42).contains(&tick));
        assert_eq!(
            change.map(|(after, size)| (from + after, size)),
            Some((down_at + UP_AFTER, (1920, 1080)))
        );
    }

    #[test]
    fn test_sharp_stays_down_while_the_link_stays_unstable() {
        let mut gov = sharp_governor();
        let (down_at, _) = until_change(&mut gov, Instant::now(), true);
        let from = down_at + Duration::from_millis(100);
        assert_eq!(
            run_reports(&mut gov, 2_000, from, 200, |tick| tick % 4 == 0),
            None
        );
        assert_eq!(gov.current(), (960, 544));
    }

    #[test]
    fn test_sharp_waits_longer_each_time_full_size_fails_again() {
        let mut gov = sharp_governor();
        let mut t = Instant::now();
        for wait in [
            UP_AFTER,
            UP_AFTER * 2,
            UP_AFTER * 4,
            MAX_UP_AFTER,
            MAX_UP_AFTER,
        ] {
            let (down_at, size) = until_change(&mut gov, t, true);
            assert_eq!(size, (960, 544));
            let (up_at, size) = until_change(&mut gov, down_at + CALM_GAP, false);
            assert_eq!((up_at - down_at, size), (wait, (1920, 1080)));
            // The link falters again as soon as it is back at full size.
            t = up_at + Duration::from_millis(100);
        }
    }

    #[test]
    fn test_sharp_retry_wait_resets_once_full_size_holds() {
        let mut gov = sharp_governor();
        let (down_at, _) = until_change(&mut gov, Instant::now(), true);
        let (up_at, _) = until_change(&mut gov, down_at + CALM_GAP, false);
        let (down_at, _) = until_change(&mut gov, up_at + Duration::from_millis(100), true);
        let (up_at, _) = until_change(&mut gov, down_at + CALM_GAP, false);
        assert_eq!(up_at - down_at, UP_AFTER * 2, "the first return failed");

        // This time full size holds before the link falters.
        let (down_at, _) = until_change(&mut gov, up_at + RETRY_HOLD, true);
        let (up_at, _) = until_change(&mut gov, down_at + CALM_GAP, false);
        assert_eq!(up_at - down_at, UP_AFTER);
    }

    #[test]
    fn test_ten_mbps_keeps_native_1080p() {
        assert_eq!(stream_size(1920, 1080, 60, "hevc", 10_000, 0), (1920, 1080));
        assert_eq!(stream_size(1920, 1080, 60, "h264", 10_000, 0), (1920, 1080));
    }

    /// The measured case: 6 Mbps cannot feed 1080p60 of desktop text.
    #[test]
    fn test_six_mbps_drops_to_900p() {
        assert_eq!(stream_size(1920, 1080, 60, "hevc", 6_000, 0), (1600, 900));
        assert_eq!(stream_size(1920, 1080, 60, "h264", 6_000, 0), (1600, 900));
    }

    #[test]
    fn test_lower_frame_rate_affords_more_pixels() {
        assert_eq!(stream_size(1920, 1080, 30, "hevc", 6_000, 0), (1920, 1080));
    }

    #[test]
    fn test_very_slow_link_bottoms_out_at_540p() {
        assert_eq!(stream_size(1920, 1080, 60, "h264", 1_000, 0), (960, 540));
    }

    #[test]
    fn test_ultrawide_keeps_aspect_ratio() {
        assert_eq!(stream_size(3440, 1440, 60, "hevc", 6_000, 0), (1720, 720));
    }

    #[test]
    fn test_explicit_cap_wins() {
        assert_eq!(
            stream_size(1920, 1080, 60, "hevc", 50_000, 720),
            (1280, 720)
        );
        // A cap above the display does not upscale.
        assert_eq!(
            stream_size(1920, 1080, 60, "hevc", 1_000, 1440),
            (1920, 1080)
        );
    }

    #[test]
    fn test_dimensions_are_even() {
        let (w, h) = stream_size(1366, 768, 60, "hevc", 2_000, 0);
        assert_eq!((w % 2, h % 2), (0, 0));
        let (w, h) = stream_size(1, 1, 60, "hevc", 2_000, 0);
        assert_eq!((w, h), (2, 2));
    }
}
