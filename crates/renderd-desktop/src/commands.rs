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
