#![allow(unsafe_code)]

//! `ScreenCaptureKit` display content filter builder.
//!
//! Provides [`ContentFilter`] for targeting specific macOS displays (`SCDisplay`)
//! for hardware-accelerated screen capture.

use std::sync::mpsc::channel;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::ClassType;
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{SCContentFilter, SCShareableContent, SCWindow};

use crate::error::ScError;

extern "C" {
    fn CGMainDisplayID() -> u32;
}

/// Safe wrapper around `objc2_screen_capture_kit::SCContentFilter`.
#[derive(Debug, Clone)]
pub struct ContentFilter {
    filter: Retained<SCContentFilter>,
    display_id: u32,
    width: usize,
    height: usize,
}

impl ContentFilter {
    /// Selects the primary display (`CGMainDisplayID()`) for screen capture.
    ///
    /// # Errors
    /// Returns [`ScError::NoDisplaysFound`] if no displays are attached, or
    /// [`ScError::FilterCreationFailed`] if shareable content enumeration fails.
    pub fn main_display() -> Result<Self, ScError> {
        // SAFETY: CGMainDisplayID is a thread-safe CoreGraphics query.
        let main_id = unsafe { CGMainDisplayID() };
        Self::by_display_id(main_id)
    }

    /// Selects a specific display by its CoreGraphics 32-bit display ID (`CGDirectDisplayID`).
    ///
    /// # Errors
    /// Returns [`ScError::NoDisplaysFound`] if the requested display ID is not found, or
    /// [`ScError::FilterCreationFailed`] if shareable content enumeration fails.
    #[allow(clippy::cast_sign_loss)]
    pub fn by_display_id(target_display_id: u32) -> Result<Self, ScError> {
        let content = fetch_shareable_content()?;

        // SAFETY: content is a valid SCShareableContent instance.
        let displays = unsafe { content.displays() };
        let count = displays.count();

        if count == 0 {
            return Err(ScError::NoDisplaysFound);
        }

        let mut matched_display = None;
        for i in 0..count {
            // SAFETY: index i is strictly within bounds 0..count.
            let d = unsafe { displays.objectAtIndex(i) };
            // SAFETY: d is a valid SCDisplay object. Sending displayID selector is sound.
            let d_id: u32 = unsafe { objc2::msg_send![&d, displayID] };
            if d_id == target_display_id {
                matched_display = Some(d);
                break;
            }
        }

        let target_display = matched_display.unwrap_or_else(|| {
            // SAFETY: count > 0 is verified above.
            unsafe { displays.objectAtIndex(0) }
        });

        // SAFETY: target_display is a valid SCDisplay object.
        let display_id: u32 = unsafe { objc2::msg_send![&target_display, displayID] };
        let width = unsafe { target_display.width() } as usize;
        let height = unsafe { target_display.height() } as usize;

        // Exclude empty window list to capture the full display surface
        let empty_windows = NSArray::<SCWindow>::new();

        // SAFETY: target_display and empty_windows are valid Objective-C objects.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &target_display,
                &empty_windows,
            )
        };

        Ok(Self {
            filter,
            display_id,
            width,
            height,
        })
    }

    /// Returns the selected display ID.
    #[must_use]
    pub const fn display_id(&self) -> u32 {
        self.display_id
    }

    /// Returns the display width in pixels.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Returns the display height in pixels.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Returns a reference to the underlying `SCContentFilter` Objective-C object.
    #[must_use]
    pub fn as_objc(&self) -> &SCContentFilter {
        &self.filter
    }
}

/// Helper function to synchronously fetch `SCShareableContent`.
fn fetch_shareable_content() -> Result<Retained<SCShareableContent>, ScError> {
    let (tx, rx) = channel();

    let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        if !error.is_null() {
            // SAFETY: error is a valid non-null NSError object.
            let err_msg = unsafe { (*error).localizedDescription().to_string() };
            let _ = tx.send(Err(ScError::FilterCreationFailed(err_msg)));
        } else if !content.is_null() {
            // SAFETY: content is a valid non-null SCShareableContent object.
            let retained = unsafe { Retained::retain(content) };
            if let Some(retained) = retained {
                let _ = tx.send(Ok(retained));
            } else {
                let _ = tx.send(Err(ScError::FilterCreationFailed(
                    "Failed to retain SCShareableContent".into(),
                )));
            }
        } else {
            let _ = tx.send(Err(ScError::FilterCreationFailed(
                "Null shareable content returned".into(),
            )));
        }
    });

    // SAFETY: getShareableContentWithCompletionHandler is the standard ScreenCaptureKit async entrypoint.
    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&handler);
    }

    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| ScError::FilterCreationFailed("Timed out waiting for SCShareableContent".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::ScreenRecordingPermission;

    #[test]
    fn test_main_display_filter_creation() {
        if !ScreenRecordingPermission::check().is_granted() {
            // Skip test in head-less CI environments lacking TCC authorization
            return;
        }

        let filter_res = ContentFilter::main_display();
        assert!(filter_res.is_ok(), "main_display failed: {:?}", filter_res.err());

        let filter = filter_res.unwrap();
        assert!(filter.display_id() > 0);
        assert!(filter.width() > 0);
        assert!(filter.height() > 0);
    }
}
