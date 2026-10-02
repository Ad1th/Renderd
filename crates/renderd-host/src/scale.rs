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

#[cfg(test)]
mod tests {
    use super::*;

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
