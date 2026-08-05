//! Bounded thread-safe queue for buffering decoded video frames between networking and rendering.

use std::collections::VecDeque;
use std::sync::Mutex;
use crate::decoder::DecodedFrame;

/// Bounded thread-safe queue for decoded frames with automatic stale-frame dropping for latency control.
#[derive(Debug)]
pub struct FrameQueue {
    capacity: usize,
    buffer: Mutex<VecDeque<DecodedFrame>>,
}

impl FrameQueue {
    /// Creates a new [`FrameQueue`] with specified maximum capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// Pushes a new [`DecodedFrame`] into the queue.
    /// Returns `true` if an old frame was dropped due to queue overflow.
    pub fn push(&self, frame: DecodedFrame) -> bool {
        let mut queue = self.buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dropped = if queue.len() >= self.capacity {
            queue.pop_front();
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
        let mut queue = self.buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.pop_front()
    }

    /// Drops all frames older than `min_pts_ns`. Returns the number of stale frames dropped.
    pub fn drop_stale(&self, min_pts_ns: u64) -> usize {
        let mut queue = self.buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let initial_len = queue.len();
        queue.retain(|f| f.pts_ns >= min_pts_ns);
        initial_len - queue.len()
    }

    /// Returns current number of buffered frames.
    #[must_use]
    pub fn len(&self) -> usize {
        let queue = self.buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.len()
    }

    /// Checks if the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all buffered frames from the queue.
    pub fn clear(&self) {
        let mut queue = self.buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use crate::decoder::PixelFormat;

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
}
