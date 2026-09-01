//! Bounded thread-safe queue for buffering decoded video frames between networking and rendering.

use crate::decoder::DecodedFrame;
use std::collections::VecDeque;
use std::sync::Mutex;

use std::sync::atomic::{AtomicU64, Ordering};

/// Bounded thread-safe queue for decoded frames with automatic stale-frame dropping for latency control.
#[derive(Debug)]
pub struct FrameQueue {
    capacity: usize,
    buffer: Mutex<VecDeque<DecodedFrame>>,
    total_pushed: AtomicU64,
    total_popped: AtomicU64,
    stale_dropped: AtomicU64,
    overflow_dropped: AtomicU64,
}

impl FrameQueue {
    /// Creates a new [`FrameQueue`] with specified maximum capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            total_pushed: AtomicU64::new(0),
            total_popped: AtomicU64::new(0),
            stale_dropped: AtomicU64::new(0),
            overflow_dropped: AtomicU64::new(0),
        }
    }

    /// Pushes a new [`DecodedFrame`] into the queue.
    /// Returns `true` if an old frame was dropped due to queue overflow.
    pub fn push(&self, frame: DecodedFrame) -> bool {
        self.total_pushed.fetch_add(1, Ordering::Relaxed);
        let mut queue = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dropped = if queue.len() >= self.capacity {
            queue.pop_front();
            self.overflow_dropped.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        };
        queue.push_back(frame);
        dropped
    }

    /// Pops the next frame ready for rendering.
    #[must_use]
    pub fn pop(&self) -> Option<DecodedFrame> {
        let frame = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        if frame.is_some() {
            self.total_popped.fetch_add(1, Ordering::Relaxed);
        }
        frame
    }

    /// Pops the freshest (latest) frame ready for rendering and discards any older stale frames.
    ///
    /// For interactive remote-desktop streaming, this collapses any accumulated queue backlog
    /// to 0 frames of latency, returning the newest frame and the count of discarded stale frames.
    #[must_use]
    pub fn pop_latest(&self) -> (Option<DecodedFrame>, usize) {
        let mut queue = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let len = queue.len();
        if len == 0 {
            drop(queue);
            (None, 0)
        } else {
            let stale = len - 1;
            let latest = queue.pop_back();
            queue.clear();
            drop(queue);
            if stale > 0 {
                self.stale_dropped
                    .fetch_add(stale as u64, Ordering::Relaxed);
            }
            if latest.is_some() {
                self.total_popped.fetch_add(1, Ordering::Relaxed);
            }
            (latest, stale)
        }
    }

    /// Drops all frames older than `min_pts_ns`. Returns the number of stale frames dropped.
    pub fn drop_stale(&self, min_pts_ns: u64) -> usize {
        let mut queue = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let initial_len = queue.len();
        queue.retain(|f| f.pts_ns >= min_pts_ns);
        let dropped = initial_len - queue.len();
        drop(queue);
        if dropped > 0 {
            self.stale_dropped
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
        dropped
    }

    /// Returns current number of buffered frames.
    #[must_use]
    pub fn len(&self) -> usize {
        let queue = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.len()
    }

    /// Checks if the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all buffered frames from the queue.
    pub fn clear(&self) {
        let mut queue = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let len = queue.len();
        if len > 0 {
            self.stale_dropped.fetch_add(len as u64, Ordering::Relaxed);
        }
        queue.clear();
    }

    /// Returns the total number of frames pushed.
    #[must_use]
    pub fn total_pushed(&self) -> u64 {
        self.total_pushed.load(Ordering::Relaxed)
    }

    /// Returns the total number of frames popped for presentation.
    #[must_use]
    pub fn total_popped(&self) -> u64 {
        self.total_popped.load(Ordering::Relaxed)
    }

    /// Returns the total count of stale frames dropped to preserve low latency.
    #[must_use]
    pub fn stale_dropped(&self) -> u64 {
        self.stale_dropped.load(Ordering::Relaxed)
    }

    /// Returns the total count of overflow frames dropped upon push.
    #[must_use]
    pub fn overflow_dropped(&self) -> u64 {
        self.overflow_dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::PixelFormat;
    use std::time::Duration;

    fn make_test_frame(id: u64, pts: u64) -> DecodedFrame {
        DecodedFrame {
            frame_id: id,
            pts_ns: pts,
            width: 1920,
            height: 1080,
            format: PixelFormat::Bgra8,
            buffer: vec![0; 64],
            decode_duration: Duration::from_millis(1),
        }
    }

    #[test]
    fn test_frame_queue_push_pop_overflow() {
        let queue = FrameQueue::new(2);
        assert!(queue.is_empty());

        assert!(!queue.push(make_test_frame(1, 100)));
        assert!(!queue.push(make_test_frame(2, 200)));
        assert_eq!(queue.len(), 2);

        // Third push overflows capacity = 2, dropping frame 1
        assert!(queue.push(make_test_frame(3, 300)));
        assert_eq!(queue.len(), 2);

        let f2 = queue.pop().unwrap();
        assert_eq!(f2.frame_id, 2);

        let f3 = queue.pop().unwrap();
        assert_eq!(f3.frame_id, 3);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_frame_queue_drop_stale() {
        let queue = FrameQueue::new(5);
        queue.push(make_test_frame(1, 100));
        queue.push(make_test_frame(2, 200));
        queue.push(make_test_frame(3, 300));

        let dropped = queue.drop_stale(250);
        assert_eq!(dropped, 2);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop().unwrap().frame_id, 3);
    }

    #[test]
    fn test_frame_queue_pop_latest() {
        let queue = FrameQueue::new(5);
        assert_eq!(queue.pop_latest(), (None, 0));

        queue.push(make_test_frame(1, 100));
        queue.push(make_test_frame(2, 200));
        queue.push(make_test_frame(3, 300));
        assert_eq!(queue.len(), 3);

        // pop_latest returns frame 3 and drops 2 stale frames (1 and 2)
        let (latest, stale) = queue.pop_latest();
        assert_eq!(latest.unwrap().frame_id, 3);
        assert_eq!(stale, 2);
        assert_eq!(queue.stale_dropped(), 2);
        assert!(queue.is_empty());
    }
}
