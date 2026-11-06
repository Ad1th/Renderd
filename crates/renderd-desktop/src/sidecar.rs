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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("renderd-sidecar-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_a_binary_beside_the_app() {
        let dir = scratch_dir("beside");
        let binary = dir.join(file_name(Role::Viewer));
        fs::write(&binary, b"").unwrap();
        assert_eq!(locate(Role::Viewer, &dir, None), Some(binary));
    }

    #[test]
    fn a_missing_binary_is_none() {
        let dir = scratch_dir("missing");
        assert_eq!(locate(Role::Host, &dir, None), None);
    }

    #[test]
    fn an_override_wins_when_it_exists() {
        let dir = scratch_dir("override");
        fs::write(dir.join(file_name(Role::Host)), b"").unwrap();
        let custom = dir.join("custom-host");
        fs::write(&custom, b"").unwrap();
        assert_eq!(locate(Role::Host, &dir, Some(&custom)), Some(custom));
    }

    #[test]
    fn a_dangling_override_falls_back() {
        let dir = scratch_dir("dangling");
        let binary = dir.join(file_name(Role::Host));
        fs::write(&binary, b"").unwrap();
        let gone = dir.join("nope");
        assert_eq!(locate(Role::Host, &dir, Some(&gone)), Some(binary));
    }

    #[test]
    fn finds_a_binary_in_the_bin_folder() {
        let dir = scratch_dir("bin");
        fs::create_dir_all(dir.join("bin")).unwrap();
        let binary = dir.join("bin").join(file_name(Role::Viewer));
        fs::write(&binary, b"").unwrap();
        assert_eq!(locate(Role::Viewer, &dir, None), Some(binary));
    }

    #[test]
    fn override_variables_differ_per_role() {
        assert_eq!(override_var(Role::Host), "RENDERD_HOST_BIN");
        assert_eq!(override_var(Role::Viewer), "RENDERD_VIEWER_BIN");
    }
}
