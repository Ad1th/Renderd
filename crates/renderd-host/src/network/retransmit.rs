//! Recently sent fragments, kept so the viewer can ask for lost ones again.
//!
//! A lost fragment used to cost a keyframe: the viewer could only ask for a
//! fresh IDR, the biggest frame the encoder makes, and on a slow link that is
//! hundreds of milliseconds of frozen picture. Every datagram the sender puts
//! on the wire is already a cheap reference-counted [`Bytes`], so holding the
//! last second of them costs nothing but memory, and answering a `Nack` is a
//! lookup and a resend.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytes::Bytes;

/// How long a sent frame stays available for retransmission.
///
/// The viewer gives a missing frame up after two round trips plus 120 ms; a
/// second covers that on any path the stream is usable over.
pub const RETAIN_FOR: Duration = Duration::from_secs(1);

/// Upper bound on the bytes held, whatever [`RETAIN_FOR`] allows.
///
/// A few first-of-session 1080p keyframes plus a second of 60 Mbps video.
pub const RETAIN_BYTES: usize = 8 * 1024 * 1024;

/// One sent frame's datagrams.
#[derive(Debug)]
struct SentFrame {
    frame_id: u64,
    sent_at: Instant,
    datagrams: Vec<Bytes>,
    bytes: usize,
    /// When each datagram was last resent, to ignore a repeated request that
    /// crossed the previous answer on the wire.
    resent_at: Vec<Option<Instant>>,
}

/// Sent datagrams of the last [`RETAIN_FOR`], keyed by frame.
#[derive(Debug, Default)]
pub struct RetransmitCache {
    frames: Mutex<VecDeque<SentFrame>>,
    resent: AtomicU64,
    unavailable: AtomicU64,
}

impl RetransmitCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the datagrams `frame_id` went out as.
    pub fn insert(&self, frame_id: u64, datagrams: Vec<Bytes>, now: Instant) {
        let bytes = datagrams.iter().map(Bytes::len).sum();
        let resent_at = vec![None; datagrams.len()];
        let Ok(mut frames) = self.frames.lock() else {
            return;
        };
        frames.push_back(SentFrame {
            frame_id,
            sent_at: now,
            datagrams,
            bytes,
            resent_at,
        });
        let mut total: usize = frames.iter().map(|f| f.bytes).sum();
        while let Some(oldest) = frames.front() {
            let expired = now.saturating_duration_since(oldest.sent_at) > RETAIN_FOR;
            if !expired && total <= RETAIN_BYTES || frames.len() == 1 {
                break;
            }
            total -= oldest.bytes;
            frames.pop_front();
        }
    }

    /// The datagrams to resend for a request naming `frag_ids` of `frame_id`
    /// (all of them when `frag_ids` is empty).
    ///
    /// Datagrams resent less than `min_gap` ago are left out: the request was
    /// most likely sent before the previous answer arrived. A frame no longer
    /// held — too old, or never sent because the sender skipped it — yields
    /// nothing; the viewer then gives it up and asks for a keyframe.
    pub fn take_for_resend(
        &self,
        frame_id: u64,
        frag_ids: &[u32],
        now: Instant,
        min_gap: Duration,
    ) -> Vec<Bytes> {
        let Ok(mut frames) = self.frames.lock() else {
            return Vec::new();
        };
        let Some(frame) = frames.iter_mut().rev().find(|f| f.frame_id == frame_id) else {
            self.unavailable.fetch_add(1, Ordering::Relaxed);
            return Vec::new();
        };
        if now.saturating_duration_since(frame.sent_at) > RETAIN_FOR {
            self.unavailable.fetch_add(1, Ordering::Relaxed);
            return Vec::new();
        }
        let wanted: Vec<usize> = if frag_ids.is_empty() {
            (0..frame.datagrams.len()).collect()
        } else {
            frag_ids
                .iter()
                .filter_map(|&i| usize::try_from(i).ok())
                .collect()
        };
        let mut out = Vec::new();
        for idx in wanted {
            let (Some(datagram), Some(resent)) =
                (frame.datagrams.get(idx), frame.resent_at.get_mut(idx))
            else {
                continue;
            };
            if resent.is_some_and(|at| now.saturating_duration_since(at) < min_gap) {
                continue;
            }
            *resent = Some(now);
            out.push(datagram.clone());
        }
        self.resent.fetch_add(out.len() as u64, Ordering::Relaxed);
        out
    }

    /// The last datagram of the most recently sent frame.
    ///
    /// Sent again when the stream goes quiet, it lets the viewer notice a lost
    /// tail: without anything arriving after it, a frame whose last fragments
    /// were lost looks exactly like one still on its way.
    #[must_use]
    pub fn tail_probe(&self) -> Option<Bytes> {
        self.frames
            .lock()
            .ok()?
            .back()
            .and_then(|f| f.datagrams.last().cloned())
    }

    /// Datagrams resent in answer to requests so far.
    #[must_use]
    pub fn resent(&self) -> u64 {
        self.resent.load(Ordering::Relaxed)
    }

    /// Requests for frames no longer held.
    #[must_use]
    pub fn unavailable(&self) -> u64 {
        self.unavailable.load(Ordering::Relaxed)
    }

    /// Forgets everything, for a new session.
    pub fn clear(&self) {
        if let Ok(mut frames) = self.frames.lock() {
            frames.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datagrams(n: u8) -> Vec<Bytes> {
        (0..n).map(|i| Bytes::from(vec![i; 4])).collect()
    }

    #[test]
    fn test_resends_named_fragments() {
        let cache = RetransmitCache::new();
        let t0 = Instant::now();
        cache.insert(7, datagrams(4), t0);
        let out = cache.take_for_resend(7, &[1, 3], t0, Duration::from_millis(10));
        assert_eq!(out, vec![Bytes::from(vec![1; 4]), Bytes::from(vec![3; 4])]);
        assert_eq!(cache.resent(), 2);
    }

    #[test]
    fn test_empty_request_resends_whole_frame() {
        let cache = RetransmitCache::new();
        let t0 = Instant::now();
        cache.insert(7, datagrams(3), t0);
        assert_eq!(
            cache
                .take_for_resend(7, &[], t0, Duration::from_millis(10))
                .len(),
            3
        );
    }

    #[test]
    fn test_repeat_inside_min_gap_is_ignored() {
        let cache = RetransmitCache::new();
        let t0 = Instant::now();
        cache.insert(7, datagrams(2), t0);
        let gap = Duration::from_millis(10);
        assert_eq!(cache.take_for_resend(7, &[0], t0, gap).len(), 1);
        assert!(cache
            .take_for_resend(7, &[0], t0 + Duration::from_millis(5), gap)
            .is_empty());
        assert_eq!(
            cache
                .take_for_resend(7, &[0], t0 + Duration::from_millis(15), gap)
                .len(),
            1
        );
    }

    #[test]
    fn test_unknown_and_out_of_range_requests_yield_nothing() {
        let cache = RetransmitCache::new();
        let t0 = Instant::now();
        cache.insert(7, datagrams(2), t0);
        let gap = Duration::from_millis(10);
        assert!(cache.take_for_resend(8, &[0], t0, gap).is_empty());
        assert!(cache.take_for_resend(7, &[9], t0, gap).is_empty());
        assert_eq!(cache.unavailable(), 1);
    }

    #[test]
    fn test_old_frames_expire() {
        let cache = RetransmitCache::new();
        let t0 = Instant::now();
        cache.insert(1, datagrams(2), t0);
        let later = t0 + RETAIN_FOR + Duration::from_millis(1);
        cache.insert(2, datagrams(2), later);
        assert!(cache
            .take_for_resend(1, &[0], later, Duration::ZERO)
            .is_empty());
        assert_eq!(
            cache.take_for_resend(2, &[0], later, Duration::ZERO).len(),
            1
        );
    }

    #[test]
    fn test_bytes_are_bounded() {
        let cache = RetransmitCache::new();
        let t0 = Instant::now();
        let big = vec![Bytes::from(vec![0u8; RETAIN_BYTES / 2 + 1])];
        cache.insert(1, big.clone(), t0);
        cache.insert(2, big.clone(), t0);
        cache.insert(3, big, t0);
        assert!(cache.take_for_resend(1, &[], t0, Duration::ZERO).is_empty());
        assert_eq!(cache.take_for_resend(3, &[], t0, Duration::ZERO).len(), 1);
    }

    #[test]
    fn test_tail_probe_is_last_datagram_of_newest_frame() {
        let cache = RetransmitCache::new();
        assert!(cache.tail_probe().is_none());
        let t0 = Instant::now();
        cache.insert(1, datagrams(2), t0);
        cache.insert(2, datagrams(3), t0);
        assert_eq!(cache.tail_probe(), Some(Bytes::from(vec![2; 4])));
    }
}
