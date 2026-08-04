//! Error types for frame header codec and reassembly.

use thiserror::Error;

/// Error type returned during frame fragment encoding, decoding, or reassembly.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    /// Buffer passed for header encoding or decoding is too short.
    #[error("Buffer too short: expected at least {expected} bytes, got {got}")]
    BufferTooShort {
        /// Expected minimum buffer size in bytes.
        expected: usize,
        /// Actual buffer size provided.
        got: usize,
    },

    /// Presentation timestamp offset exceeds 24-bit representation limit.
    #[error("PTS offset {0} us exceeds 24-bit maximum (16,777,215 us)")]
    PtsOffsetOverflow(u32),

    /// Invalid fragment total (0) or fragment ID out of bounds.
    #[error("Invalid fragment bounds: frag_id {frag_id} >= frag_total {frag_total}")]
    InvalidFragmentBounds {
        /// Fragment ID index.
        frag_id: u16,
        /// Total fragment count.
        frag_total: u16,
    },

    /// Duplicate fragment received for an already buffered position.
    #[error("Duplicate fragment {frag_id} received for frame {frame_id}")]
    DuplicateFragment {
        /// Frame ID.
        frame_id: u64,
        /// Fragment ID index.
        frag_id: u16,
    },

    /// Reassembly buffer capacity exceeded.
    #[error("Reassembly window full: cannot track frame {0}")]
    WindowOverflow(u64),
}
