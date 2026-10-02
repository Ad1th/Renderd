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

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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

/// Samples of packet loss kept, one per [`LinkPressure::packet_loss`] call.
///
/// The ABR loop samples every 100 ms, so this is about the last second.
pub const LOSS_SAMPLES: usize = 10;

/// Fewest packets over the sampling window for a loss rate to mean anything.
/// A still desktop sends a handful of packets a second; one of them lost is
/// not 20% loss.
pub const MIN_LOSS_PACKETS: u64 = 50;

/// Running packet counts behind [`LinkPressure::packet_loss`].
#[derive(Debug, Default)]
struct LossWindow {
    /// QUIC's cumulative `(sent, lost)` packet counters at the last sample.
    last: Option<(u64, u64)>,
    /// Per-sample `(sent, lost)` deltas, oldest first.
    samples: VecDeque<(u64, u64)>,
}

impl LossWindow {
    /// Adds the counters read now and returns the loss rate over the window.
    fn sample(&mut self, sent: u64, lost: u64) -> f64 {
        if let Some((last_sent, last_lost)) = self.last {
            self.samples.push_back((
                sent.saturating_sub(last_sent),
                lost.saturating_sub(last_lost),
            ));
            while self.samples.len() > LOSS_SAMPLES {
                self.samples.pop_front();
            }
        }
        self.last = Some((sent, lost));
        let (sent, lost) = self
            .samples
            .iter()
            .fold((0, 0), |(s, l), &(ds, dl)| (s + ds, l + dl));
        if sent < MIN_LOSS_PACKETS {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let rate = lost as f64 / sent as f64;
        rate.clamp(0.0, 1.0)
    }
}

/// Live view of the session's send queue, shared by the sender and capture path.
#[derive(Debug, Default)]
pub struct LinkPressure {
    connection: Mutex<Option<quinn::Connection>>,
    skipping: AtomicBool,
    skipped: AtomicU64,
    sent_kbps: AtomicU32,
    loss: Mutex<LossWindow>,
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
        if let Ok(mut loss) = self.loss.lock() {
            *loss = LossWindow::default();
        }
    }

    /// Stops tracking, so capture for the next session starts unthrottled.
    pub fn detach(&self) {
        if let Ok(mut guard) = self.connection.lock() {
            *guard = None;
        }
        self.skipping.store(false, Ordering::Relaxed);
        self.sent_kbps.store(0, Ordering::Relaxed);
        if let Ok(mut loss) = self.loss.lock() {
            *loss = LossWindow::default();
        }
    }

    /// Packet loss over about the last second, as QUIC's own loss detection
    /// sees it, or `None` with no session attached.
    ///
    /// Each call is one sample; call it once per ABR tick. This replaces the
    /// viewer's frame-loss rate, which over a 100 ms report made one lost
    /// frame among six read as 14% loss, counted frames the sender skipped on
    /// purpose as lost, and counted a lost frame twice when its gap and its
    /// eviction were both reported.
    #[must_use]
    pub fn packet_loss(&self) -> Option<f64> {
        let stats = self
            .connection
            .lock()
            .ok()?
            .as_ref()
            .map(|c| c.stats().path)?;
        let mut loss = self.loss.lock().ok()?;
        Some(loss.sample(stats.sent_packets, stats.lost_packets))
    }

    /// Records the video bitrate the sender actually put on the wire over its
    /// last measurement interval.
    pub fn set_sent_kbps(&self, kbps: u32) {
        self.sent_kbps.store(kbps, Ordering::Relaxed);
    }

    /// The video bitrate actually sent over the last measurement interval.
    #[must_use]
    pub fn sent_kbps(&self) -> u32 {
        self.sent_kbps.load(Ordering::Relaxed)
    }

    /// Returns `true` if the encoder is producing well under `target_kbps` — a
    /// still desktop — so a clean link says nothing about spare capacity.
    #[must_use]
    pub fn is_app_limited(&self, target_kbps: u32) -> bool {
        u64::from(self.sent_kbps()) * 10 < u64::from(target_kbps) * 6
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

    #[test]
    fn test_loss_window_rates_the_last_second() {
        let mut window = LossWindow::default();
        assert!(window.sample(0, 0).abs() < f64::EPSILON);
        // 100 packets, 3 lost.
        assert!((window.sample(100, 3) - 0.03).abs() < 1e-9);
        // The window keeps LOSS_SAMPLES deltas; old loss ages out.
        let mut sent = 100;
        for _ in 0..LOSS_SAMPLES {
            sent += 100;
            window.sample(sent, 3);
        }
        assert!(window.sample(sent + 100, 3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_loss_needs_enough_packets_to_mean_anything() {
        let mut window = LossWindow::default();
        window.sample(0, 0);
        assert!(window.sample(5, 1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_detached_link_reports_no_loss_rate() {
        assert!(LinkPressure::new().packet_loss().is_none());
    }

    #[test]
    fn test_app_limited_below_sixty_percent_of_target() {
        let link = LinkPressure::new();
        link.set_sent_kbps(5_000);
        assert!(!link.is_app_limited(8_000));
        link.set_sent_kbps(4_000);
        assert!(link.is_app_limited(8_000));
        link.detach();
        assert!(
            link.is_app_limited(8_000),
            "no measurement yet counts as idle"
        );
    }
}
