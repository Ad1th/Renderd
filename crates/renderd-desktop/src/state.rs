//! What the app keeps for as long as it runs.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use renderd_desktop::{Settings, Supervisor};

/// Shared between every command the window can call.
pub struct AppState {
    /// Runs the host or viewer process.
    pub supervisor: Supervisor,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
}

impl AppState {
    /// Loads the saved settings and wraps them with the supervisor.
    pub fn new(supervisor: Supervisor, settings: Settings, settings_path: PathBuf) -> Self {
        Self {
            supervisor,
            settings: Mutex::new(settings),
            settings_path,
        }
    }

    /// The current settings.
    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
