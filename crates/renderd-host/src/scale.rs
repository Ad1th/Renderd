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

/// Picture must be too big for the bitrate this long before the encoder steps down.
pub const DOWN_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// Bitrate must afford a bigger picture this long before the encoder steps up.
pub const UP_AFTER: std::time::Duration = std::time::Duration::from_secs(8);

/// Least time between two size changes. Each one costs a keyframe.
pub const MIN_CHANGE_GAP: std::time::Duration = std::time::Duration::from_secs(5);

/// Which way the encoded size wants to move, and since when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressure {
    Down(std::time::Instant),
    Up(std::time::Instant),
}

/// Follows the session's bitrate with the encoded picture size.
///
/// The session's size is set once, from the bitrate *ceiling*, before anything
/// is known about the link. When the link turns out to carry far less, every
/// full-screen change is still coded at that size: measured on an M3 at
/// 2 Mbps, 83 KB at 1080p against 25 KB at 544p — a third of a second of link
/// time against a tenth. The governor moves the encoder down the [`ladder`]
/// once the bitrate has been too low for the current size for
/// [`DOWN_AFTER`], and back up one rung at a time once it has afforded the
/// next one for [`UP_AFTER`] without congestion. Down is quick because every
/// moment at the wrong size is latency; up is slow because every change costs
/// a keyframe.
#[derive(Debug, Clone)]
pub struct ResolutionGovernor {
    ladder: Vec<(u32, u32)>,
    fps: u32,
    codec: String,
    current: usize,
    pressure: Option<Pressure>,
    last_change: Option<std::time::Instant>,
}

impl ResolutionGovernor {
    /// A governor for a session of `native` size at `fps`, encoding `codec`.
    #[must_use]
    pub fn new(native: (u32, u32), fps: u32, codec: &str) -> Self {
        Self {
            ladder: ladder(native),
            fps,
            codec: codec.to_string(),
            current: 0,
            pressure: None,
            last_change: None,
        }
    }

    /// The size to start encoding at, given the session's starting bitrate.
    pub fn start(&mut self, bitrate_kbps: u32) -> (u32, u32) {
        self.current = self.largest_fitting(bitrate_kbps, STAY_MARGIN);
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
        let too_big = !self.fits(self.ladder[self.current], bitrate_kbps, STAY_MARGIN);
        let room_above = self.current > 0
            && !congested
            && self.fits(self.ladder[self.current - 1], bitrate_kbps, 1.0);
        let pressure = if too_big {
            match self.pressure {
                Some(Pressure::Down(since)) => Some(Pressure::Down(since)),
                _ => Some(Pressure::Down(now)),
            }
        } else if room_above {
            match self.pressure {
                Some(Pressure::Up(since)) => Some(Pressure::Up(since)),
                _ => Some(Pressure::Up(now)),
            }
        } else {
            None
        };
        self.pressure = pressure;

        if self
            .last_change
            .is_some_and(|at| now.saturating_duration_since(at) < MIN_CHANGE_GAP)
        {
            return None;
        }
        let next = match pressure? {
            // Straight to a size that fits outright, with room to spare.
            Pressure::Down(since) if now.saturating_duration_since(since) >= DOWN_AFTER => self
                .largest_fitting(bitrate_kbps, 1.0)
                .max(self.current + 1)
                .min(self.ladder.len() - 1),
            Pressure::Up(since) if now.saturating_duration_since(since) >= UP_AFTER => {
                self.current - 1
            }
            _ => return None,
        };
        if next == self.current {
            self.pressure = None;
            return None;
        }
        self.current = next;
        self.pressure = None;
        self.last_change = Some(now);
        Some(self.ladder[next])
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
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
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
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
        gov.start(8_000);
        let t0 = Instant::now();
        for s in 0..30 {
            assert_eq!(gov.update(8_000, false, t0 + Duration::from_secs(s)), None);
        }
        assert_eq!(gov.current(), (1920, 1080));
    }

    #[test]
    fn test_steps_down_after_a_sustained_drop() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
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
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
        gov.start(10_000);
        let t0 = Instant::now();
        assert_eq!(gov.update(3_000, true, t0), None);
        assert_eq!(gov.update(10_000, false, t0 + Duration::from_secs(1)), None);
        assert_eq!(gov.update(3_000, false, t0 + Duration::from_secs(2)), None);
        assert_eq!(gov.update(3_000, false, t0 + Duration::from_secs(3)), None);
    }

    #[test]
    fn test_steps_up_one_rung_at_a_time_and_only_uncongested() {
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
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
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
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
        let mut gov = ResolutionGovernor::new((1920, 1080), 60, "hevc");
        gov.start(1_000);
        let t0 = Instant::now();
        for s in 0..20 {
            assert_eq!(gov.update(1_000, false, t0 + Duration::from_secs(s)), None);
        }
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
