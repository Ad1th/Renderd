//! Finding the host and viewer binaries the app starts.

use std::path::{Path, PathBuf};

use crate::role::Role;

/// File name of the binary for `role` on this platform.
#[must_use]
pub fn file_name(role: Role) -> String {
    format!("{}{}", role.binary_name(), std::env::consts::EXE_SUFFIX)
}

/// Name of the environment variable that overrides where a binary is found.
#[must_use]
pub const fn override_var(role: Role) -> &'static str {
    match role {
        Role::Host => "RENDERD_HOST_BIN",
        Role::Viewer => "RENDERD_VIEWER_BIN",
    }
}

/// Places a binary for `role` may live, most specific first.
///
/// `app_dir` is the folder holding the app's own executable. A packaged app
/// ships the binaries beside it; on macOS a bundle may also keep them under
/// `Contents/Resources`. A `cargo run` build finds them in the same target
/// folder, which is also `app_dir`.
#[must_use]
pub fn candidates(role: Role, app_dir: &Path) -> Vec<PathBuf> {
    let name = file_name(role);
    vec![
        app_dir.join(&name),
        app_dir.join("../Resources").join(&name),
        app_dir.join("bin").join(&name),
    ]
}

/// Finds the binary for `role`.
///
/// An explicit `override_path` (from the environment) wins when it points at a
/// file. Otherwise the first existing candidate under `app_dir` is used.
#[must_use]
pub fn locate(role: Role, app_dir: &Path, override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    candidates(role, app_dir)
        .into_iter()
        .find(|path| path.is_file())
}

/// Finds the binary for `role` next to the running app.
///
/// Reads the override from the environment and the app folder from the
/// current executable.
#[must_use]
pub fn locate_installed(role: Role) -> Option<PathBuf> {
    let override_path = std::env::var_os(override_var(role)).map(PathBuf::from);
    let exe = std::env::current_exe().ok()?;
    let app_dir = exe.parent()?;
    locate(role, app_dir, override_path.as_deref())
}
