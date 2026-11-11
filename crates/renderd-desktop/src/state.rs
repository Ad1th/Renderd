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

impl AppState {
    /// Replaces the settings and writes them to disk.
    ///
    /// The settings are repaired first, and the repaired copy is returned so
    /// the window shows what was really saved. A failed write is not fatal:
    /// the choice still applies until the app closes.
    pub fn replace_settings(&self, settings: Settings) -> Settings {
        let settings = settings.sanitized();
        if let Err(error) = settings.save(&self.settings_path) {
            eprintln!(
                "could not save settings to {}: {error}",
                self.settings_path.display()
            );
        }
        *self.settings.lock().unwrap_or_else(PoisonError::into_inner) = settings.clone();
        settings
    }
}
