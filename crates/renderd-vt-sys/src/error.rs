//! `OSStatus` error code translation for `VideoToolbox` operations.
//!
//! `VideoToolbox` API functions return `OSStatus` (an `i32` alias defined by
//! Apple in `MacTypes.h`) to signal success or failure. The raw negative integer
//! values are opaque to users without access to the `VideoToolbox` headers.
//!
//! This module defines [`VtError`], a typed newtype over `OSStatus` that maps
//! the most common `VideoToolbox` error codes to human-readable messages for
//! diagnostics and logging.

use thiserror::Error;

/// Error returned by `VideoToolbox` API operations.
///
/// Wraps Apple's `OSStatus` (`i32`) and maps known `VideoToolbox` error codes
/// to human-readable descriptions. An `OSStatus` of `0` (`noErr`) is never
/// wrapped in a `VtError` — callers should only construct this type on failure.
///
/// # Known Codes
///
/// The following common `VideoToolbox` status codes are mapped:
///
/// | Code | Constant | Meaning |
/// |------|----------|---------|
/// | `-12900` | `kVTPropertyNotSupportedErr` | Property not supported by this encoder |
/// | `-12901` | `kVTPropertyReadOnlyErr` | Property is read-only |
/// | `-12902` | `kVTParameterErr` | Invalid parameter |
/// | `-12903` | `kVTInvalidSessionErr` | Session is no longer valid |
/// | `-12904` | `kVTAllocationFailedErr` | Memory allocation failed |
/// | `-12905` | `kVTPixelTransferNotSupportedErr` | Pixel format transfer not supported |
/// | `-17390` | `kVTHardwareNotAvailableErr` | Hardware encoder not available |
/// | `-17391` | `kVTHardwareAcceleratedVideoEncoderNotAvailableErr` | No HW video encoder |
/// | `-8961`  | `kVTVideoEncoderMalfunctionErr` | Encoder reported a malfunction |
/// | `-8960`  | `kVTVideoEncoderAuthorizationErr` | Encoder authorization denied |
/// | other   | — | Unknown status, raw code included |
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VtError(
    /// The raw `OSStatus` integer returned by the `VideoToolbox` API.
    pub i32,
);

// Raw OSStatus constants from VideoToolbox/VTErrors.h and CoreFoundation/MacTypes.h.
// Defined as associated constants rather than importing the C headers so that the
// error table is self-contained and verifiable without a macOS SDK in scope.
impl VtError {
    /// `kVTPropertyNotSupportedErr` — the specified property is not supported.
    pub const PROPERTY_NOT_SUPPORTED: i32 = -12_900;
    /// `kVTPropertyReadOnlyErr` — the specified property is read-only.
    pub const PROPERTY_READ_ONLY: i32 = -12_901;
    /// `kVTParameterErr` — a parameter value is invalid.
    pub const PARAMETER: i32 = -12_902;
    /// `kVTInvalidSessionErr` — the `VTCompressionSession` is no longer valid.
    pub const INVALID_SESSION: i32 = -12_903;
    /// `kVTAllocationFailedErr` — a memory allocation failed.
    pub const ALLOCATION_FAILED: i32 = -12_904;
    /// `kVTPixelTransferNotSupportedErr` — pixel format transfer not supported.
    pub const PIXEL_TRANSFER_NOT_SUPPORTED: i32 = -12_905;
    /// `kVTHardwareNotAvailableErr` — hardware encoder/decoder not available.
    pub const HARDWARE_NOT_AVAILABLE: i32 = -17_390;
    /// `kVTHardwareAcceleratedVideoEncoderNotAvailableErr` — no HW video encoder.
    pub const HW_VIDEO_ENCODER_NOT_AVAILABLE: i32 = -17_391;
    /// `kVTVideoEncoderMalfunctionErr` — encoder reported an internal malfunction.
    pub const ENCODER_MALFUNCTION: i32 = -8_961;
    /// `kVTVideoEncoderAuthorizationErr` — encoder authorization was denied.
    pub const ENCODER_AUTHORIZATION: i32 = -8_960;

    /// Returns the raw `OSStatus` code.
    #[must_use]
    pub const fn code(self) -> i32 {
        self.0
    }

    /// Returns `true` if this error indicates that no hardware encoder is available.
    ///
    /// This covers both `kVTHardwareNotAvailableErr` and
    /// `kVTHardwareAcceleratedVideoEncoderNotAvailableErr`.
    #[must_use]
    pub const fn is_hw_unavailable(self) -> bool {
        matches!(
            self.0,
            Self::HARDWARE_NOT_AVAILABLE | Self::HW_VIDEO_ENCODER_NOT_AVAILABLE
        )
    }

    /// Returns `true` if this error indicates the session has been invalidated.
    #[must_use]
    pub const fn is_invalid_session(self) -> bool {
        self.0 == Self::INVALID_SESSION
    }
}

impl std::fmt::Display for VtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let description = match self.0 {
            Self::PROPERTY_NOT_SUPPORTED => "VideoToolbox: property not supported",
            Self::PROPERTY_READ_ONLY => "VideoToolbox: property is read-only",
            Self::PARAMETER => "VideoToolbox: invalid parameter",
            Self::INVALID_SESSION => "VideoToolbox: session is invalid or has been invalidated",
            Self::ALLOCATION_FAILED => "VideoToolbox: memory allocation failed",
            Self::PIXEL_TRANSFER_NOT_SUPPORTED => {
                "VideoToolbox: pixel format transfer not supported"
            }
            Self::HARDWARE_NOT_AVAILABLE => "VideoToolbox: hardware encoder/decoder not available",
            Self::HW_VIDEO_ENCODER_NOT_AVAILABLE => {
                "VideoToolbox: hardware-accelerated video encoder not available on this device"
            }
            Self::ENCODER_MALFUNCTION => "VideoToolbox: encoder reported an internal malfunction",
            Self::ENCODER_AUTHORIZATION => {
                "VideoToolbox: authorization to use the encoder was denied"
            }
            _ => return write!(f, "VideoToolbox: unknown OSStatus error (code {})", self.0),
        };
        f.write_str(description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_status_display() {
        assert_eq!(
            VtError(VtError::INVALID_SESSION).to_string(),
            "VideoToolbox: session is invalid or has been invalidated",
        );
        assert_eq!(
            VtError(VtError::HW_VIDEO_ENCODER_NOT_AVAILABLE).to_string(),
            "VideoToolbox: hardware-accelerated video encoder not available on this device",
        );
        assert_eq!(
            VtError(VtError::PARAMETER).to_string(),
            "VideoToolbox: invalid parameter",
        );
        assert_eq!(
            VtError(VtError::HARDWARE_NOT_AVAILABLE).to_string(),
            "VideoToolbox: hardware encoder/decoder not available",
        );
    }

    #[test]
    fn test_unknown_status_display() {
        let unknown = VtError(-99_999);
        assert!(unknown.to_string().contains("-99999"));
        assert!(unknown.to_string().contains("unknown OSStatus error"));
    }

    #[test]
    fn test_hw_unavailable_predicate() {
        assert!(VtError(VtError::HARDWARE_NOT_AVAILABLE).is_hw_unavailable());
        assert!(VtError(VtError::HW_VIDEO_ENCODER_NOT_AVAILABLE).is_hw_unavailable());
        assert!(!VtError(VtError::INVALID_SESSION).is_hw_unavailable());
    }

    #[test]
    fn test_invalid_session_predicate() {
        assert!(VtError(VtError::INVALID_SESSION).is_invalid_session());
        assert!(!VtError(VtError::PARAMETER).is_invalid_session());
    }

    #[test]
    fn test_code_accessor() {
        let err = VtError(-12_903);
        assert_eq!(err.code(), -12_903);
    }

    #[test]
    fn test_error_trait_impl() {
        // Verify VtError satisfies std::error::Error (required for ? operator and anyhow).
        let err: Box<dyn std::error::Error> = Box::new(VtError(VtError::PARAMETER));
        assert!(err.to_string().contains("invalid parameter"));
    }

    #[test]
    fn test_eq_and_copy() {
        let a = VtError(-12_902);
        let b = a; // Copy
        assert_eq!(a, b);
    }
}
