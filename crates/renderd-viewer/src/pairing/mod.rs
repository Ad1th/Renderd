//! Viewer SPAKE2+ pairing subsystem module (`renderd-viewer/src/pairing/`).

pub mod prover;
pub mod ui;

pub use prover::{ViewerPairingClient, ViewerPairingError};
pub use ui::{PairingUi, PairingUiState};
