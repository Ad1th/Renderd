//! The few things that differ between macOS and Windows.

use serde::Serialize;

/// Whether this computer may record its screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenAccess {
    /// Recording is allowed.
    Granted,
    /// macOS has not allowed it yet.
    Denied,
    /// This platform does not gate screen recording, or does not share at all.
    Unneeded,
}

/// Checks screen recording access without prompting.
#[cfg(target_os = "macos")]
pub fn screen_access() -> ScreenAccess {
    if renderd_sc_sys::permission::ScreenRecordingPermission::check().is_granted() {
        ScreenAccess::Granted
    } else {
        ScreenAccess::Denied
    }
}

/// Checks screen recording access without prompting.
#[cfg(not(target_os = "macos"))]
pub const fn screen_access() -> ScreenAccess {
    ScreenAccess::Unneeded
}
