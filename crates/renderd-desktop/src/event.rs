//! What a log line means for the session.
//!
//! The daemons already log every step of a session. This module picks out the
//! handful of lines that change what the window should show.

use serde::Serialize;

/// The picture the two sides agreed on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamInfo {
    /// Codec name, `hevc` or `h264`.
    pub codec: String,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Frames per second the session asked for.
    pub fps: f64,
}

/// Live numbers from a running session.
///
/// Each log line carries only some of these, so every value is optional and a
/// later sample fills in what an earlier one left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Metrics {
    /// Frames sent (host) or shown (viewer) per second.
    pub fps: Option<f64>,
    /// Video bitrate on the wire, in kbit/s.
    pub bitrate_kbps: Option<f64>,
    /// Round trip time to the other side, in milliseconds.
    pub rtt_ms: Option<f64>,
    /// Video waiting to be sent on the host, in kilobytes.
    pub send_queue_kb: Option<f64>,
    /// Time to decode one frame on the viewer, in milliseconds.
    pub decode_ms: Option<f64>,
    /// Frames the viewer gave up on since it connected.
    pub lost_frames: Option<f64>,
}

impl Metrics {
    /// Overlays `newer` on this sample, keeping old values it does not carry.
    #[must_use]
    pub fn merged(self, newer: Self) -> Self {
        Self {
            fps: newer.fps.or(self.fps),
            bitrate_kbps: newer.bitrate_kbps.or(self.bitrate_kbps),
            rtt_ms: newer.rtt_ms.or(self.rtt_ms),
            send_queue_kb: newer.send_queue_kb.or(self.send_queue_kb),
            decode_ms: newer.decode_ms.or(self.decode_ms),
            lost_frames: newer.lost_frames.or(self.lost_frames),
        }
    }
}
