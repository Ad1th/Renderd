//! Sliding-window fragment reassembly buffer state machine per RFC-0002 §12.2.

use bytes::{Bytes, BytesMut};
use std::collections::BTreeMap;

use crate::error::FrameError;
use crate::flags::FragmentFlags;
use crate::header::FragmentHeader;
use crate::validate::ValidateHeader;

/// Completed reassembled video frame ready for decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReassembledFrame {
    /// Monotonic frame sequence identifier.
    pub frame_id: u64,
    /// Indicates whether the frame is an IDR keyframe.
    pub is_keyframe: bool,
    /// Presentation timestamp offset in microseconds.
    pub pts_offset_us: u32,
    /// Contiguous payload buffer containing the reassembled frame data.
    pub payload: Bytes,
}

/// State tracking an in-flight frame undergoing fragment reassembly.
#[derive(Debug)]
struct PendingFrame {
    frag_total: u16,
    received_count: u16,
    is_keyframe: bool,
    pts_offset_us: u32,
    fragments: Vec<Option<Bytes>>,
}

/// Sliding-window reassembly buffer state machine tracking out-of-order datagrams.
///
/// The window holds at most `capacity` partially received frames. When a fragment for
/// a new frame arrives and the window is full, the oldest incomplete frame is evicted
/// so that packet loss cannot permanently wedge the stream — an evicted frame is
/// recorded in the retirement watermark so its late fragments are discarded rather
/// than re-occupying a slot.
#[derive(Debug)]
pub struct ReassemblyBuffer {
    capacity: usize,
    pending: BTreeMap<u64, PendingFrame>,
    /// Highest frame ID that has been completed **or** abandoned. Fragments at or
    /// below this watermark are stale and dropped. `None` until the first retirement,
    /// so that `frame_id == 0` is a deliverable frame.
    retired_watermark: Option<u64>,
    dropped_frames: u64,
}

impl ReassemblyBuffer {
    /// Creates a new `ReassemblyBuffer` with the specified maximum in-flight frame capacity.
    ///
    /// A `capacity` of 0 is clamped to 1 so at least one frame can always be assembled.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity: if capacity == 0 { 1 } else { capacity },
            pending: BTreeMap::new(),
            retired_watermark: None,
            dropped_frames: 0,
        }
    }

    /// Returns `true` if `frame_id` is at or below the retirement watermark.
    const fn is_stale(&self, frame_id: u64) -> bool {
        match self.retired_watermark {
            Some(watermark) => frame_id <= watermark,
            None => false,
        }
    }

    /// Raises the retirement watermark to at least `frame_id`.
    fn retire(&mut self, frame_id: u64) {
        self.retired_watermark = Some(match self.retired_watermark {
            Some(watermark) if watermark >= frame_id => watermark,
            _ => frame_id,
        });
    }

    /// Inserts a validated fragment into the reassembly window.
    ///
    /// # Errors
    /// Returns [`FrameError`] if header validation fails, the fragment is out of bounds,
    /// duplicate, or its `frag_total` disagrees with the frame already being assembled.
    pub fn insert(
        &mut self,
        header: FragmentHeader,
        payload: Bytes,
    ) -> Result<Option<ReassembledFrame>, FrameError> {
        header.validate()?;

        if self.is_stale(header.frame_id) {
            // Drop stale fragment for an already completed or abandoned frame.
            return Ok(None);
        }

        if !self.pending.contains_key(&header.frame_id) && self.pending.len() >= self.capacity {
            // Sliding window: make room by abandoning the oldest incomplete frame.
            // If the arriving frame is itself the oldest, drop it instead.
            let Some(&oldest) = self.pending.keys().next() else {
                return Err(FrameError::WindowOverflow(header.frame_id));
            };
            if header.frame_id < oldest {
                self.dropped_frames += 1;
                return Ok(None);
            }
            self.pending.remove(&oldest);
            self.retire(oldest);
            self.dropped_frames += 1;
        }

        let flags = FragmentFlags::from_bits(header.flags);

        let entry = self
            .pending
            .entry(header.frame_id)
            .or_insert_with(|| PendingFrame {
                frag_total: header.frag_total,
                received_count: 0,
                is_keyframe: false,
                pts_offset_us: header.pts_offset_us,
                fragments: vec![None; usize::from(header.frag_total)],
            });

        // A peer must describe the same frame identically in every fragment. Without this
        // check a mismatched `frag_total` would index past the allocated fragment vector.
        if entry.frag_total != header.frag_total {
            return Err(FrameError::FragmentTotalMismatch {
                frame_id: header.frame_id,
                expected: entry.frag_total,
                got: header.frag_total,
            });
        }

        let idx = usize::from(header.frag_id);
        if entry.fragments[idx].is_some() {
            return Err(FrameError::DuplicateFragment {
                frame_id: header.frame_id,
                frag_id: header.frag_id,
            });
        }

        // The keyframe flag is set on every fragment of a keyframe, but tolerate peers
        // that only mark the first one: any fragment claiming keyframe makes the frame one.
        entry.is_keyframe |= flags.is_keyframe();
        // The first fragment carries the authoritative presentation timestamp.
        if header.frag_id == 0 {
            entry.pts_offset_us = header.pts_offset_us;
        }

        entry.fragments[idx] = Some(payload);
        entry.received_count += 1;

        if entry.received_count == entry.frag_total {
            let Some(pending_frame) = self.pending.remove(&header.frame_id) else {
                return Ok(None);
            };
            self.retire(header.frame_id);

            let total_bytes: usize = pending_frame
                .fragments
                .iter()
                .map(|f| f.as_ref().map_or(0, Bytes::len))
                .sum();

            let mut assembled = BytesMut::with_capacity(total_bytes);
            for data in pending_frame.fragments.into_iter().flatten() {
                assembled.extend_from_slice(&data);
            }

            Ok(Some(ReassembledFrame {
                frame_id: header.frame_id,
                is_keyframe: pending_frame.is_keyframe,
                pts_offset_us: pending_frame.pts_offset_us,
                payload: assembled.freeze(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Evicts incomplete frames older than `min_frame_id` to free window slots under packet loss.
    pub fn drop_older_than(&mut self, min_frame_id: u64) {
        let evicted: Vec<u64> = self
            .pending
            .range(..min_frame_id)
            .map(|(&frame_id, _)| frame_id)
            .collect();
        for frame_id in evicted {
            self.pending.remove(&frame_id);
            self.retire(frame_id);
            self.dropped_frames += 1;
        }
    }

    /// Returns the number of frames currently mid-reassembly.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Returns the number of frames abandoned incomplete since construction.
    #[must_use]
    pub const fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flags::{FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG};

    fn header(frame_id: u64, frag_id: u16, frag_total: u16) -> FragmentHeader {
        let mut flags = FragmentFlags::new();
        flags.set_first(frag_id == 0);
        flags.set_last(frag_id == frag_total - 1);
        FragmentHeader {
            frame_id,
            frag_id,
            frag_total,
            flags: flags.bits(),
            pts_offset_us: 100,
        }
    }

    #[test]
    fn test_single_fragment_reassembly() {
        let mut buffer = ReassemblyBuffer::new(16);
        let header = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 1,
            flags: FLAG_FIRST_FRAG | FLAG_LAST_FRAG | FLAG_KEYFRAME,
            pts_offset_us: 500,
        };
        let payload = Bytes::from_static(b"frame_payload_data");

        let result = buffer.insert(header, payload.clone()).unwrap();
        assert!(result.is_some());
        let frame = result.unwrap();
        assert_eq!(frame.frame_id, 1);
        assert!(frame.is_keyframe);
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn test_out_of_order_reassembly() {
        let mut buffer = ReassemblyBuffer::new(16);

        let h1 = FragmentHeader {
            frame_id: 1,
            frag_id: 1,
            frag_total: 2,
            flags: FLAG_LAST_FRAG,
            pts_offset_us: 100,
        };
        let h0 = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 2,
            flags: FLAG_FIRST_FRAG,
            pts_offset_us: 100,
        };

        assert!(buffer
            .insert(h1, Bytes::from_static(b"world"))
            .unwrap()
            .is_none());

        let result = buffer.insert(h0, Bytes::from_static(b"hello ")).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().payload, Bytes::from_static(b"hello world"));
    }

    /// Frame 0 is a legitimate frame ID and must be deliverable.
    #[test]
    fn test_frame_id_zero_is_delivered() {
        let mut buffer = ReassemblyBuffer::new(4);
        let frame = buffer
            .insert(header(0, 0, 1), Bytes::from_static(b"first"))
            .unwrap();
        assert!(frame.is_some(), "frame_id 0 must not be treated as stale");
        assert_eq!(frame.unwrap().frame_id, 0);
    }

    /// A fragment whose `frag_total` disagrees with the frame under assembly must be
    /// rejected, not indexed past the end of the fragment vector.
    #[test]
    fn test_mismatched_frag_total_is_rejected_not_panic() {
        let mut buffer = ReassemblyBuffer::new(4);
        buffer
            .insert(header(7, 0, 2), Bytes::from_static(b"a"))
            .unwrap();

        // Same frame, but now claiming 5 fragments with index 4 — would be out of bounds.
        let err = buffer
            .insert(header(7, 4, 5), Bytes::from_static(b"b"))
            .unwrap_err();
        assert_eq!(
            err,
            FrameError::FragmentTotalMismatch {
                frame_id: 7,
                expected: 2,
                got: 5
            }
        );
    }

    /// Losing one fragment of several frames must not permanently wedge the window.
    #[test]
    fn test_window_slides_past_lost_fragments() {
        let mut buffer = ReassemblyBuffer::new(4);

        // Frames 1..=4 each lose their second fragment.
        for frame_id in 1..=4u64 {
            assert!(buffer
                .insert(header(frame_id, 0, 2), Bytes::from_static(b"x"))
                .unwrap()
                .is_none());
        }
        assert_eq!(buffer.pending_len(), 4);

        // Frame 5 arrives complete: the window must evict the oldest stuck frame
        // and still deliver frame 5 rather than returning WindowOverflow forever.
        assert!(buffer
            .insert(header(5, 0, 2), Bytes::from_static(b"he"))
            .unwrap()
            .is_none());
        let done = buffer
            .insert(header(5, 1, 2), Bytes::from_static(b"llo"))
            .unwrap();
        assert_eq!(done.unwrap().payload, Bytes::from_static(b"hello"));
        assert!(buffer.dropped_frames() >= 1);
    }

    /// Late fragments of an evicted frame must not re-occupy a window slot.
    #[test]
    fn test_late_fragment_of_evicted_frame_is_dropped() {
        let mut buffer = ReassemblyBuffer::new(1);
        buffer
            .insert(header(1, 0, 2), Bytes::from_static(b"x"))
            .unwrap();
        // Frame 2 evicts frame 1.
        buffer
            .insert(header(2, 0, 2), Bytes::from_static(b"y"))
            .unwrap();
        assert_eq!(buffer.pending_len(), 1);

        // The straggler for frame 1 is dropped silently.
        assert!(buffer
            .insert(header(1, 1, 2), Bytes::from_static(b"z"))
            .unwrap()
            .is_none());
        assert_eq!(buffer.pending_len(), 1);
    }

    /// The keyframe flag is honoured no matter which fragment carries it.
    #[test]
    fn test_keyframe_flag_survives_out_of_order_arrival() {
        let mut buffer = ReassemblyBuffer::new(4);
        let mut h0 = header(3, 0, 2);
        h0.flags |= FLAG_KEYFRAME;

        // Non-keyframe-flagged last fragment arrives first.
        buffer
            .insert(header(3, 1, 2), Bytes::from_static(b"b"))
            .unwrap();
        let frame = buffer
            .insert(h0, Bytes::from_static(b"a"))
            .unwrap()
            .unwrap();
        assert!(frame.is_keyframe);
    }

    #[test]
    fn test_drop_older_than_retires_frames() {
        let mut buffer = ReassemblyBuffer::new(8);
        buffer
            .insert(header(1, 0, 2), Bytes::from_static(b"a"))
            .unwrap();
        buffer
            .insert(header(2, 0, 2), Bytes::from_static(b"b"))
            .unwrap();
        buffer.drop_older_than(2);
        assert_eq!(buffer.pending_len(), 1);
        // Frame 1 is retired: its straggler must not be re-admitted.
        assert!(buffer
            .insert(header(1, 1, 2), Bytes::from_static(b"c"))
            .unwrap()
            .is_none());
        assert_eq!(buffer.pending_len(), 1);
    }
}
