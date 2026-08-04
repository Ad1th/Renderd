#![allow(unsafe_code)]

//! macOS TCC screen recording permission checker.
//!
//! Before starting a `ScreenCaptureKit` stream, the application must hold screen capture
//! privacy authorization (TCC). This module provides non-blocking status preflighting
//! and user authorization request prompts.

extern "C" {
    /// Returns `true` if screen capture authorization has been granted to the process.
    pub fn CGPreflightScreenCaptureAccess() -> bool;

    /// Triggers the macOS system prompt requesting screen capture authorization if not yet decided.
    /// Returns `true` if authorization is granted.
    pub fn CGRequestScreenCaptureAccess() -> bool;
}

/// Authorization status for macOS screen recording privacy permission (TCC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PermissionStatus {
    /// Authorization has been explicitly granted by the user or system profile.
    Granted,
    /// Authorization is denied, restricted, or not yet granted.
    #[default]
    Denied,
}

impl PermissionStatus {
    /// Returns `true` if screen recording permission is granted.
    #[must_use]
    pub const fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }
}

/// macOS screen recording TCC permission manager.
pub struct ScreenRecordingPermission;

impl ScreenRecordingPermission {
    /// Checks the current screen recording authorization status without triggering a prompt.
    #[must_use]
    pub fn check() -> PermissionStatus {
        // SAFETY: CGPreflightScreenCaptureAccess is a thread-safe CoreGraphics query function.
        let granted = unsafe { CGPreflightScreenCaptureAccess() };
        if granted {
            PermissionStatus::Granted
        } else {
            PermissionStatus::Denied
        }
    }

    /// Prompts the user for screen recording authorization if not yet granted.
    ///
    /// Returns `true` if authorization is granted after request.
    #[must_use]
    pub fn request() -> bool {
        // SAFETY: CGRequestScreenCaptureAccess is a thread-safe CoreGraphics authorization prompt function.
        unsafe { CGRequestScreenCaptureAccess() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_check_returns_status_without_crashing() {
        let status = ScreenRecordingPermission::check();
        // Just verify that preflight query executes cleanly without panic.
        assert!(matches!(status, PermissionStatus::Granted | PermissionStatus::Denied));
        assert_eq!(status.is_granted(), status == PermissionStatus::Granted);
    }
}
