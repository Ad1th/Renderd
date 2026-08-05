//! Platform-specific Win32 windowing and DPI initialization helpers.

#![cfg(target_os = "windows")]

use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// Sets Per-Monitor v2 DPI awareness context on Windows platforms.
///
/// # Errors
/// Returns [`crate::error::ViewerError::Window`] if setting DPI awareness fails.
pub fn enable_dpi_awareness() -> Result<(), crate::error::ViewerError> {
    // SAFETY: SetProcessDpiAwarenessContext calls Win32 API to enable Per-Monitor v2 DPI awareness.
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            tracing::warn!(
                "Failed to set Per-Monitor v2 DPI awareness; falling back to system defaults"
            );
        }
    }
    Ok(())
}
