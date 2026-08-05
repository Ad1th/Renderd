#![allow(unsafe_code)]

//! macOS login item auto-start manager using `SMAppService.mainApp`.

use crate::error::HostError;

/// Auto-start registration status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AutoStartStatus {
    /// Service is not registered as a login item.
    #[default]
    NotRegistered,
    /// Service is enabled as a login item.
    Enabled,
    /// Service is registered but requires user approval in System Settings.
    RequiresApproval,
    /// Service or app bundle could not be found.
    NotFound,
    /// Status is unknown or platform does not support `SMAppService`.
    Unknown,
}

/// macOS login item auto-start controller.
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoStart;

impl AutoStart {
    /// Create a new `AutoStart` instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Enable auto-start at user login via `SMAppService.mainApp`.
    ///
    /// # Errors
    ///
    /// Returns a [`HostError::Initialization`] if `SMAppService` registration fails.
    pub fn enable() -> Result<(), HostError> {
        #[cfg(target_os = "macos")]
        {
            macos::enable_main_app().map_err(HostError::Initialization)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(HostError::Initialization(
                "Auto-start is only supported on macOS".to_string(),
            ))
        }
    }

    /// Disable auto-start at user login via `SMAppService.mainApp`.
    ///
    /// # Errors
    ///
    /// Returns a [`HostError::Initialization`] if `SMAppService` unregistration fails.
    pub fn disable() -> Result<(), HostError> {
        #[cfg(target_os = "macos")]
        {
            macos::disable_main_app().map_err(HostError::Initialization)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(HostError::Initialization(
                "Auto-start is only supported on macOS".to_string(),
            ))
        }
    }

    /// Query current auto-start registration status.
    #[must_use]
    pub fn status() -> AutoStartStatus {
        #[cfg(target_os = "macos")]
        {
            macos::status_main_app()
        }
        #[cfg(not(target_os = "macos"))]
        {
            AutoStartStatus::Unknown
        }
    }
}

/// Alias for `AutoStart` manager scaffold compatibility.
pub type AutoStartManager = AutoStart;

#[cfg(target_os = "macos")]
mod macos {
    use super::AutoStartStatus;
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::{msg_send, msg_send_id};
    use objc2_foundation::NSError;

    pub fn enable_main_app() -> Result<(), String> {
        // SAFETY: SMAppService is a thread-safe system API available on macOS 13+.
        unsafe {
            let class = AnyClass::get("SMAppService")
                .ok_or_else(|| "SMAppService class not found in system runtime".to_string())?;
            let service: Option<Retained<objc2::runtime::AnyObject>> = msg_send_id![class, mainApp];
            let service = service.ok_or_else(|| "SMAppService.mainApp returned nil".to_string())?;
            let mut error: *mut NSError = std::ptr::null_mut();
            let success: bool = msg_send![&service, registerAndReturnError: &mut error];
            if success {
                Ok(())
            } else if !error.is_null() {
                let err_obj = &*error;
                let err_msg: Retained<objc2_foundation::NSString> =
                    msg_send_id![err_obj, localizedDescription];
                Err(err_msg.to_string())
            } else {
                Err("Failed to register login item with SMAppService".to_string())
            }
        }
    }

    pub fn disable_main_app() -> Result<(), String> {
        // SAFETY: SMAppService is a thread-safe system API available on macOS 13+.
        unsafe {
            let class = AnyClass::get("SMAppService")
                .ok_or_else(|| "SMAppService class not found in system runtime".to_string())?;
            let service: Option<Retained<objc2::runtime::AnyObject>> = msg_send_id![class, mainApp];
            let service = service.ok_or_else(|| "SMAppService.mainApp returned nil".to_string())?;
            let mut error: *mut NSError = std::ptr::null_mut();
            let success: bool = msg_send![&service, unregisterAndReturnError: &mut error];
            if success {
                Ok(())
            } else if !error.is_null() {
                let err_obj = &*error;
                let err_msg: Retained<objc2_foundation::NSString> =
                    msg_send_id![err_obj, localizedDescription];
                Err(err_msg.to_string())
            } else {
                Err("Failed to unregister login item with SMAppService".to_string())
            }
        }
    }

    pub fn status_main_app() -> AutoStartStatus {
        // SAFETY: SMAppService is a thread-safe system API available on macOS 13+.
        unsafe {
            let Some(class) = AnyClass::get("SMAppService") else {
                return AutoStartStatus::Unknown;
            };
            let service: Option<Retained<objc2::runtime::AnyObject>> = msg_send_id![class, mainApp];
            let Some(service) = service else {
                return AutoStartStatus::Unknown;
            };
            let status: isize = msg_send![&service, status];
            match status {
                0 => AutoStartStatus::NotRegistered,
                1 => AutoStartStatus::Enabled,
                2 => AutoStartStatus::RequiresApproval,
                3 => AutoStartStatus::NotFound,
                _ => AutoStartStatus::Unknown,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autostart_status_query_does_not_panic() {
        let status = AutoStart::status();
        assert!(matches!(
            status,
            AutoStartStatus::NotRegistered
                | AutoStartStatus::Enabled
                | AutoStartStatus::RequiresApproval
                | AutoStartStatus::NotFound
                | AutoStartStatus::Unknown
        ));
    }
}
