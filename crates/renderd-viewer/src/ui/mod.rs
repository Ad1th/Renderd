//! Viewer UI subsystem module (`renderd-viewer/src/ui/`).

pub mod overlay;
pub mod settings;

pub use overlay::StatusOverlay;
pub use settings::{SystemTrayManager, TrayMenuAction, ViewerSettingsState};
