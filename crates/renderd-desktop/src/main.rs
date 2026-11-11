//! The Renderd desktop app.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod platform;
mod state;

use renderd_desktop::{Settings, Supervisor, Update};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::state::AppState;

/// Command-line flag that starts a session as soon as the app opens.
const START_FLAG: &str = "--start";

/// Name of the file the settings are kept in, inside the app's config folder.
const SETTINGS_FILE: &str = "settings.json";

/// The window's assets and configuration, embedded at build time.
// The generated code ends a failed asset lookup with `std::process::exit`,
// which the workspace bans in its own code.
#[allow(clippy::disallowed_methods)]
fn context() -> tauri::Context {
    tauri::generate_context!()
}

/// Passes a supervisor update on to the window.
fn forward(app: &AppHandle, update: Update) {
    // Emitting only fails while the app is shutting down, when nobody listens.
    let _ = match update {
        Update::Status(status) => app.emit("status", status),
        Update::Log(line) => app.emit("log", line),
    };
}

/// Loads the settings and puts the shared state in place.
fn setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let settings_path = app.path().app_config_dir()?.join(SETTINGS_FILE);
    let settings = Settings::load(&settings_path);

    let handle = app.handle().clone();
    let supervisor = Supervisor::new(settings.role, move |update| forward(&handle, update));
    app.manage(AppState::new(supervisor, settings, settings_path));

    // `renderd --start` goes straight to sharing (or viewing), for login
    // items and shortcuts. A failure shows in the window like any other.
    if std::env::args().any(|arg| arg == START_FLAG) {
        let _ = commands::start_session(&app.state::<AppState>(), None);
    }
    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| setup(app))
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_settings,
            commands::save_settings,
            commands::get_status,
            commands::get_logs,
            commands::start,
            commands::stop,
            commands::screen_access,
            commands::request_screen_access,
            commands::open_screen_settings,
        ])
        .build(context())
        .expect("the Renderd window could not be created");

    app.run(|app, event| {
        // Quitting must not leave a host or viewer running with no window.
        if matches!(event, RunEvent::Exit) {
            app.state::<AppState>().supervisor.shutdown();
        }
    });
}
