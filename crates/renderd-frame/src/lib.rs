//! Fragment header codec and sliding-window frame reassembly state machine.
//!
//! This crate defines the wire format for video frame fragments sent over QUIC datagrams
//! and implements the [`ReassemblyBuffer`] that reconstructs complete frames from
//! out-of-order fragment arrivals.
//!
//! # Architecture
//!
//! The crate is organized into five modules:
//!
//! - [`header`] — [`FragmentHeader`] (16-byte packed struct, little-endian) with
//!   [`FragmentHeader::encode`] / [`FragmentHeader::decode`].
//! - [`flags`] — [`FragmentFlags`] bitfield constants for `is_keyframe`,
//!   `is_last_fragment`, and `phase_sync_valid`.
//! - [`reassembly`] — [`ReassemblyBuffer`] sliding-window state machine
//!   (window depth `W = 4` frames by default).
//! - [`validate`] — [`ValidateHeader`] trait with field range checks.
//! - [`error`] — [`FrameError`] enum.
//!
//! # Usage
//!
//! ```rust,no_run
//! use renderd_frame::{FragmentHeader, ReassemblyBuffer, HEADER_SIZE};
//!
//! // Decode a raw QUIC datagram received from the host
//! fn on_datagram(buf: &[u8]) {
//!     let header = FragmentHeader::decode(buf).expect("header decode");
//!     let payload = bytes::Bytes::copy_from_slice(&buf[HEADER_SIZE..]);
//!
//!     // Window capacity of 4 in-flight frames per RFC-0002 §12.2
//!     let mut window = ReassemblyBuffer::new(4);
//!     if let Ok(Some(complete_frame)) = window.insert(header, payload) {
//!         // Pass complete_frame.payload to the D3D12 video decoder
//!         let _ = complete_frame;
//!     }
//! }
//! ```
//!
//! # Panics
//!
//! This crate does not panic. [`FragmentHeader::decode`] and [`ReassemblyBuffer::insert`]
//! return errors or `None` on malformed input rather than panicking.

pub mod error;
pub mod flags;
pub mod header;
pub mod reassembly;
pub mod validate;

pub use error::*;
pub use flags::*;
pub use header::*;
pub use reassembly::*;
pub use validate::*;
