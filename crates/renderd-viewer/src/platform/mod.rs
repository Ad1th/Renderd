//! Platform-specific windowing and display system initialization helpers.

#[cfg(target_os = "windows")]
pub mod windows;

/// Initializes platform-specific windowing and system hooks (e.g. DPI awareness on Windows).
///
/// # Errors
/// Returns [`crate::error::ViewerError`] if platform initialization fails.
pub fn init_platform() -> Result<(), crate::error::ViewerError> {
    #[cfg(target_os = "windows")]
    {
        windows::enable_dpi_awareness()?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        tracing::debug!("No platform-specific windowing hooks required for non-Windows OS");
    }
    Ok(())
}
