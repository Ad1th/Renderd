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
#[derive(Debug)]
pub struct ReassemblyBuffer {
    capacity: usize,
    pending: BTreeMap<u64, PendingFrame>,
    highest_completed_frame_id: u64,
}

impl ReassemblyBuffer {
    /// Creates a new `ReassemblyBuffer` with the specified maximum in-flight frame capacity.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            pending: BTreeMap::new(),
            highest_completed_frame_id: 0,
        }
    }

    /// Inserts a validated fragment into the reassembly window.
    ///
    /// # Errors
    /// Returns [`FrameError`] if header validation fails, fragment is out of bounds, duplicate, or capacity is exceeded.
    pub fn insert(
        &mut self,
        header: FragmentHeader,
        payload: Bytes,
    ) -> Result<Option<ReassembledFrame>, FrameError> {
        header.validate()?;

        if header.frame_id <= self.highest_completed_frame_id {
            // Drop stale fragment for already completed frame
            return Ok(None);
        }

        if !self.pending.contains_key(&header.frame_id) && self.pending.len() >= self.capacity {
            return Err(FrameError::WindowOverflow(header.frame_id));
        }

        let flags = FragmentFlags::from_bits(header.flags);

        let entry = self
            .pending
            .entry(header.frame_id)
            .or_insert_with(|| PendingFrame {
                frag_total: header.frag_total,
                received_count: 0,
                is_keyframe: flags.is_keyframe(),
                pts_offset_us: header.pts_offset_us,
                fragments: vec![None; usize::from(header.frag_total)],
            });

        let idx = usize::from(header.frag_id);
        if entry.fragments[idx].is_some() {
            return Err(FrameError::DuplicateFragment {
                frame_id: header.frame_id,
                frag_id: header.frag_id,
            });
        }

        entry.fragments[idx] = Some(payload);
        entry.received_count += 1;

        if entry.received_count == entry.frag_total {
            let Some(pending_frame) = self.pending.remove(&header.frame_id) else {
                return Ok(None);
            };
            self.highest_completed_frame_id = self.highest_completed_frame_id.max(header.frame_id);

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
        self.pending.retain(|&frame_id, _| frame_id >= min_frame_id);
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flags::{FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG};

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
}
