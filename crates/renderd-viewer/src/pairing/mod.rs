//! Viewer SPAKE2+ pairing subsystem module (`renderd-viewer/src/pairing/`).

pub mod prover;

pub use prover::{ViewerPairingClient, ViewerPairingError};
