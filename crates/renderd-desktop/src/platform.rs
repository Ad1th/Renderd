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

/// Asks macOS to show its screen recording prompt.
///
/// macOS only shows the prompt once per app. After that this just reports the
/// current answer, and the user has to change it in System Settings.
#[cfg(target_os = "macos")]
pub fn request_screen_access() -> ScreenAccess {
    if renderd_sc_sys::permission::ScreenRecordingPermission::request() {
        ScreenAccess::Granted
    } else {
        ScreenAccess::Denied
    }
}

/// Asks for screen recording access. Nothing to ask for on this platform.
#[cfg(not(target_os = "macos"))]
pub const fn request_screen_access() -> ScreenAccess {
    ScreenAccess::Unneeded
}

/// Opens the Screen Recording page of System Settings.
///
/// # Errors
/// Returns a sentence for the window if the page could not be opened.
#[cfg(target_os = "macos")]
pub fn open_screen_settings() -> Result<(), String> {
    const PANE: &str =
        "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
    std::process::Command::new("/usr/bin/open")
        .arg(PANE)
        .status()
        .map_err(|error| format!("Could not open System Settings: {error}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err("Could not open System Settings.".to_string())
            }
        })
}

/// Opens the screen recording settings. There are none on this platform.
///
/// # Errors
/// Never fails here; the signature matches the macOS version.
#[cfg(not(target_os = "macos"))]
pub fn open_screen_settings() -> Result<(), String> {
    Ok(())
}

/// Short name of the operating system, for the window to adapt its wording.
pub const fn name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}
