//! Everything the window can ask the app to do.

use serde::Serialize;
use tauri::State;

use renderd_desktop::{LogLine, Role, Settings, Status};

use crate::platform::{self, ScreenAccess};
use crate::state::AppState;

/// Facts about this build that the window needs once, at start.
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    /// Version of the app.
    version: &'static str,
    /// `macos`, `windows` or `linux`.
    platform: &'static str,
    /// Roles this computer can play.
    roles: Vec<Role>,
}

/// Describes this build.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: platform::name(),
        roles: [Role::Host, Role::Viewer]
            .into_iter()
            .filter(|role| role.supported_here())
            .collect(),
    }
}

/// Returns the saved settings.
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Saves new settings and returns them as stored.
///
/// # Errors
/// Refuses to change role while a session is running.
#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    let settings = settings.sanitized();
    if !state.supervisor.set_role(settings.role) {
        return Err("Stop the current session before switching.".to_string());
    }
    Ok(state.replace_settings(settings))
}

/// Returns the current session status.
#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Status {
    state.supervisor.status()
}

/// Returns the recent log lines, oldest first.
#[tauri::command]
pub fn get_logs(state: State<'_, AppState>) -> Vec<LogLine> {
    state.supervisor.logs()
}
