//! Platform-specific windowing and display system initialization helpers.

#[cfg(target_os = "windows")]
pub mod windows;

/// Initializes platform-specific windowing and system hooks (e.g. DPI awareness on Windows).
///
/// # Errors
/// Returns [`crate::error::ViewerError`] if platform initialization fails.
pub const fn init_platform() -> Result<(), crate::error::ViewerError> {
    #[cfg(target_os = "windows")]
    {
        windows::enable_dpi_awareness()?;
    }
    Ok(())
}
