//! Everything the window can ask the app to do.

// Tauri hands commands their state by value; they cannot take a reference.
#![allow(clippy::needless_pass_by_value)]

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use renderd_desktop::{launch, sidecar, LogLine, Role, Settings, Status};

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

/// Starts sharing or viewing, depending on the saved role.
///
/// `host` is the address typed in the window when viewing. Leave it out to
/// let the viewer find a host on the network.
///
/// # Errors
/// Returns a sentence for the window when the typed address is not usable.
#[tauri::command]
pub fn start(state: State<'_, AppState>, host: Option<String>) -> Result<Status, String> {
    let mut settings = state.settings();
    let launch = launch::plan(&settings, host.as_deref())?;

    if let Some(addr) = launch.host.as_deref() {
        settings.viewer.remember_host(addr);
        state.replace_settings(settings);
    }

    // A packaged app ships the binary beside itself. Failing that, a bare
    // name lets the system find one installed on the PATH.
    let program = sidecar::locate_installed(launch.role)
        .unwrap_or_else(|| PathBuf::from(sidecar::file_name(launch.role)));
    Ok(state.supervisor.start(&program, &launch.args))
}

/// Stops the running session.
#[tauri::command]
pub fn stop(state: State<'_, AppState>) {
    state.supervisor.stop();
}

/// Reports whether the screen may be recorded, without prompting.
#[tauri::command]
pub fn screen_access() -> ScreenAccess {
    platform::screen_access()
}

/// Shows the macOS screen recording prompt if it has not been shown yet.
#[tauri::command]
pub fn request_screen_access() -> ScreenAccess {
    platform::request_screen_access()
}

/// Opens the Screen Recording page of System Settings.
///
/// # Errors
/// Returns a sentence for the window if the page could not be opened.
#[tauri::command]
pub fn open_screen_settings() -> Result<(), String> {
    platform::open_screen_settings()
}
