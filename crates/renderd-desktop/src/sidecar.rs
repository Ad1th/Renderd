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
