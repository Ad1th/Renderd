//! Send-queue backpressure between the network sender and the capture path.
//!
//! BBR paces datagrams onto the wire at its estimate of the path rate. Whenever
//! the encoder produces faster than that — a keyframe, a scroll, a bitrate the
//! ABR loop has not pulled down yet — the excess waits in `quinn`'s datagram
//! queue, and every frame behind it is shown that much later. On a 6 Mbps link a
//! single 100 KB keyframe is ~130 ms of queue; a few of them is the "frames come
//! in late" lag.
//!
//! Dropping an *encoded* frame to shed that queue breaks the reference chain and
//! costs a keyframe, which is the most expensive thing to put on a slow link. So
//! the queue is shed one step earlier instead: while it holds more than
//! [`SKIP_ABOVE`] of video, captured frames are simply not handed to the
//! encoder. The encoder sees a lower frame rate, the stream stays decodable, and
//! the next frame that does go out is fresh rather than stale.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use renderd_net::FragmentBurst;

/// Queue depth, in time at the current bitrate, above which capture starts skipping.
pub const SKIP_ABOVE: Duration = Duration::from_millis(50);

/// Queue depth below which capture resumes after skipping.
///
/// Lower than [`SKIP_ABOVE`] so a queue hovering at the threshold does not make
/// capture alternate frame by frame.
pub const RESUME_BELOW: Duration = Duration::from_millis(25);

/// Time the link needs to drain `queued_bytes` at `bitrate_kbps`.
#[must_use]
pub fn drain_time(queued_bytes: usize, bitrate_kbps: u32) -> Duration {
    let kbps = u64::from(bitrate_kbps.max(1));
    // bytes * 8 / kbps = milliseconds; scaled to microseconds for resolution.
    let micros = (queued_bytes as u64).saturating_mul(8_000) / kbps;
    Duration::from_micros(micros)
}

/// Hysteresis for capture skipping: whether to skip given the current state
/// and how long the send queue will take to drain.
#[must_use]
pub fn next_skip_state(skipping: bool, queue: Duration) -> bool {
    if skipping {
        queue >= RESUME_BELOW
    } else {
        queue > SKIP_ABOVE
    }
}

/// Live view of the session's send queue, shared by the sender and capture path.
#[derive(Debug, Default)]
pub struct LinkPressure {
    connection: Mutex<Option<quinn::Connection>>,
    skipping: AtomicBool,
    skipped: AtomicU64,
}

impl LinkPressure {
    /// Creates a detached `LinkPressure` that never asks capture to skip.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts tracking `connection`'s send queue.
    pub fn attach(&self, connection: quinn::Connection) {
        if let Ok(mut guard) = self.connection.lock() {
            *guard = Some(connection);
        }
        self.skipping.store(false, Ordering::Relaxed);
    }

    /// Stops tracking, so capture for the next session starts unthrottled.
    pub fn detach(&self) {
        if let Ok(mut guard) = self.connection.lock() {
            *guard = None;
        }
        self.skipping.store(false, Ordering::Relaxed);
    }

    /// Bytes waiting in the send queue right now, or 0 with no session attached.
    #[must_use]
    pub fn queued_bytes(&self) -> usize {
        self.connection
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(FragmentBurst::queued_bytes))
            .unwrap_or(0)
    }

    /// How long the current send queue takes to drain at `bitrate_kbps`.
    #[must_use]
    pub fn queue_delay(&self, bitrate_kbps: u32) -> Duration {
        drain_time(self.queued_bytes(), bitrate_kbps)
    }

    /// Returns `true` if the frame just captured should not be encoded, and
    /// counts it if so.
    ///
    /// Reads the queue live on every call rather than a value the sender last
    /// published: the sender only wakes when there is a frame to send, so a
    /// published value would go stale — and stay stale — exactly while capture is
    /// skipping, which would stall the stream.
    pub fn should_skip_frame(&self, bitrate_kbps: u32) -> bool {
        let was_skipping = self.skipping.load(Ordering::Relaxed);
        let skip = next_skip_state(was_skipping, self.queue_delay(bitrate_kbps));
        if skip != was_skipping {
            self.skipping.store(skip, Ordering::Relaxed);
        }
        if skip {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
        skip
    }

    /// Total captured frames skipped because the send queue was too deep.
    #[must_use]
    pub fn skipped_frames(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drain_time_at_link_rate() {
        // 100 KB at 6 Mbps is 133 ms of link time.
        assert_eq!(drain_time(100_000, 6_000).as_millis(), 133);
        assert_eq!(drain_time(0, 6_000), Duration::ZERO);
        // A zero bitrate is treated as 1 kbps rather than dividing by zero.
        assert_eq!(drain_time(1, 0), Duration::from_millis(8));
    }

    #[test]
    fn test_skip_hysteresis() {
        assert!(!next_skip_state(false, Duration::from_millis(40)));
        assert!(next_skip_state(false, Duration::from_millis(60)));
        // Once skipping, keep skipping until the queue is well below the trigger.
        assert!(next_skip_state(true, Duration::from_millis(40)));
        assert!(next_skip_state(true, Duration::from_millis(25)));
        assert!(!next_skip_state(true, Duration::from_millis(24)));
    }

    #[test]
    fn test_detached_link_never_skips() {
        let link = LinkPressure::new();
        for _ in 0..10 {
            assert!(!link.should_skip_frame(1_000));
        }
        assert_eq!(link.skipped_frames(), 0);
        assert_eq!(link.queued_bytes(), 0);
    }
}
