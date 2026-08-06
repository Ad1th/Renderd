//! Viewer network subsystem module (`renderd-viewer/src/network/`).

pub mod control;
pub mod data;

pub use control::ViewerControlClient;
pub use data::DatagramReceiver;
