#![allow(unsafe_code)]

//! Safe RAII Rust wrapper for `VTCompressionSessionRef`.
//!
//! `CompressionSession` manages the lifecycle of an Apple `VideoToolbox` hardware
//! video encoder instance configured for ultra-low-latency real-time P2P streaming
//! (RFC-0002 §6.1).

use std::sync::Arc;

use crate::bindings::{
    renderd_VTCompressionSessionCreate, renderd_VTCompressionSessionEncodeFrame,
    renderd_VTCompressionSessionInvalidate, renderd_VTCompressionSessionSetBitrate,
    CMSampleBufferRef, CMVideoCodecType, CVImageBufferRef, OSStatus, VTCompressionSessionRef,
    VTEncodeInfoFlags, CODEC_TYPE_H264, CODEC_TYPE_HEVC,
};
use crate::error::VtError;
use crate::surface::IoSurface;

/// Supported video codecs for hardware compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoCodec {
    /// H.265 / HEVC (Primary codec, RFC-0002 §6.1)
    #[default]
    Hevc,
    /// H.264 / AVC (Fallback codec)
    H264,
}

impl VideoCodec {
    /// Returns the `CoreMedia` `FourCC` integer code for this video codec.
    #[must_use]
    pub const fn to_fourcc(self) -> CMVideoCodecType {
        match self {
            Self::Hevc => CODEC_TYPE_HEVC,
            Self::H264 => CODEC_TYPE_H264,
        }
    }
}

/// Type signature for encoded sample buffer callbacks.
///
/// Parameters:
/// - `status`: `VtError` status code (`0` / `noErr` on success)
/// - `info_flags`: `VideoToolbox` encode info flags
/// - `sample_buffer`: `CoreMedia` sample buffer containing encoded NAL units
pub type OutputCallback =
    Arc<dyn Fn(VtError, VTEncodeInfoFlags, CMSampleBufferRef) + Send + Sync + 'static>;

struct CallbackBox {
    callback: OutputCallback,
}

/// Raw C callback function registered with `VideoToolbox`.
unsafe extern "C" fn vt_output_callback(
    output_callback_ref_con: *mut std::ffi::c_void,
    _source_frame_ref_con: *mut std::ffi::c_void,
    status: OSStatus,
    info_flags: VTEncodeInfoFlags,
    sample_buffer: CMSampleBufferRef,
) {
    if output_callback_ref_con.is_null() {
        return;
    }

    // SAFETY: output_callback_ref_con is a valid non-null pointer to a CallbackBox
    // allocated by Box::into_raw in CompressionSession::new, which lives until Drop.
    let box_ptr = output_callback_ref_con.cast::<CallbackBox>();
    let cb_box = unsafe { &*box_ptr };
    (cb_box.callback)(VtError(status), info_flags, sample_buffer);
}

extern "C" {
    fn CVPixelBufferCreateWithIOSurface(
        allocator: core_foundation::base::CFTypeRef,
        surface: crate::surface::IOSurfaceRef,
        pixelBufferAttributes: core_foundation::base::CFTypeRef,
        pixelBufferOut: *mut CVImageBufferRef,
    ) -> i32;
}

/// Safe RAII wrapper around `VTCompressionSessionRef`.
///
/// Encapsulates hardware-accelerated H.265/H.264 encoding configured for:
/// - `RealTime` encoding mode enabled
/// - Frame reordering disabled (B-frames disabled, 0-frame latency)
/// - `MaxKeyFrameIntervalDuration` set to 0.5 seconds
///
/// Implements [`Drop`] to invalidate and release the underlying session handle.
pub struct CompressionSession {
    session: VTCompressionSessionRef,
    _callback_box: Box<CallbackBox>,
}

impl CompressionSession {
    /// Creates and initializes a new hardware compression session.
    ///
    /// # Errors
    /// Returns [`VtError`] if session creation or property configuration fails.
    pub fn new<F>(
        width: i32,
        height: i32,
        codec: VideoCodec,
        initial_bitrate_kbps: u32,
        callback: F,
    ) -> Result<Self, VtError>
    where
        F: Fn(VtError, VTEncodeInfoFlags, CMSampleBufferRef) + Send + Sync + 'static,
    {
        let cb_box = Box::new(CallbackBox {
            callback: Arc::new(callback),
        });

        let raw_cb_ctx = std::ptr::from_ref::<CallbackBox>(cb_box.as_ref())
            .cast_mut()
            .cast::<std::ffi::c_void>();

        let mut session: VTCompressionSessionRef = std::ptr::null_mut();

        // SAFETY: renderd_VTCompressionSessionCreate initializes the session pointer,
        // registers vt_output_callback with raw_cb_ctx, and returns OSStatus.
        let status = unsafe {
            renderd_VTCompressionSessionCreate(
                width,
                height,
                codec.to_fourcc(),
                initial_bitrate_kbps,
                vt_output_callback,
                raw_cb_ctx,
                &mut session,
            )
        };

        if status != 0 || session.is_null() {
            return Err(VtError(status));
        }

        Ok(Self {
            session,
            _callback_box: cb_box,
        })
    }

    /// Dynamically adjusts target average bitrate in kilobits per second.
    ///
    /// # Errors
    /// Returns [`VtError`] if property update fails.
    pub fn set_bitrate(&self, bitrate_kbps: u32) -> Result<(), VtError> {
        // SAFETY: self.session is a valid non-null VTCompressionSessionRef.
        let status = unsafe { renderd_VTCompressionSessionSetBitrate(self.session, bitrate_kbps) };
        if status == 0 {
            Ok(())
        } else {
            Err(VtError(status))
        }
    }

    /// Submits an `IoSurface` GPU memory buffer to the encoder.
    ///
    /// Parameters:
    /// - `surface`: GPU surface containing input frame pixels
    /// - `pts_ns`: Presentation timestamp in nanoseconds
    /// - `force_keyframe`: If true, requests an immediate IDR keyframe
    ///
    /// # Errors
    /// Returns [`VtError`] if frame submission fails.
    pub fn encode_surface(
        &self,
        surface: &IoSurface,
        pts_ns: i64,
        force_keyframe: bool,
    ) -> Result<(), VtError> {
        let mut pixel_buffer: CVImageBufferRef = std::ptr::null();
        // SAFETY: CVPixelBufferCreateWithIOSurface wraps the IOSurface GPU memory handle in a CVPixelBufferRef.
        let status = unsafe {
            CVPixelBufferCreateWithIOSurface(
                std::ptr::null(),
                surface.as_raw(),
                std::ptr::null(),
                &mut pixel_buffer,
            )
        };

        if status != 0 || pixel_buffer.is_null() {
            return Err(VtError(status));
        }

        // SAFETY: pixel_buffer is valid non-null CVImageBufferRef created from surface.
        let res = unsafe { self.encode_buffer(pixel_buffer, pts_ns, force_keyframe) };

        // SAFETY: Release the transient CVPixelBufferRef wrapper (+1 count created by CVPixelBufferCreateWithIOSurface).
        unsafe {
            core_foundation::base::CFRelease(pixel_buffer);
        }

        res
    }

    /// Submits a raw `CVImageBufferRef` (or `IOSurfaceRef`) to the encoder.
    ///
    /// # Safety
    /// `image_buffer` must be a valid, live `CVImageBufferRef`.
    ///
    /// # Errors
    /// Returns [`VtError`] if frame submission fails.
    pub unsafe fn encode_buffer(
        &self,
        image_buffer: CVImageBufferRef,
        pts_ns: i64,
        force_keyframe: bool,
    ) -> Result<(), VtError> {
        if image_buffer.is_null() {
            return Err(VtError(VtError::PARAMETER));
        }

        // SAFETY: self.session is valid and image_buffer is non-null.
        let status = unsafe {
            renderd_VTCompressionSessionEncodeFrame(
                self.session,
                image_buffer,
                pts_ns,
                force_keyframe,
                std::ptr::null_mut(),
            )
        };

        if status == 0 {
            Ok(())
        } else {
            Err(VtError(status))
        }
    }
}

impl Drop for CompressionSession {
    fn drop(&mut self) {
        if !self.session.is_null() {
            // SAFETY: self.session is a valid non-null VTCompressionSessionRef handle.
            unsafe {
                renderd_VTCompressionSessionInvalidate(self.session);
            }
            self.session = std::ptr::null_mut();
        }
    }
}

// SAFETY: CompressionSession internal C session handle and callback context are safe to transfer across threads.
unsafe impl Send for CompressionSession {}

// SAFETY: Method calls on CompressionSession (set_bitrate, encode_surface) use thread-safe VideoToolbox C APIs.
unsafe impl Sync for CompressionSession {}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation::base::TCFType;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    extern "C" {
        fn IOSurfaceCreate(
            properties: core_foundation::base::CFTypeRef,
        ) -> crate::surface::IOSurfaceRef;
    }

    #[test]
    fn test_codec_fourcc() {
        assert_eq!(VideoCodec::Hevc.to_fourcc(), CODEC_TYPE_HEVC);
        assert_eq!(VideoCodec::H264.to_fourcc(), CODEC_TYPE_H264);
    }

    #[test]
    fn test_create_set_bitrate_and_drop_session() {
        let frame_count = Arc::new(AtomicUsize::new(0));
        let keyframe_received = Arc::new(AtomicBool::new(false));

        let fc = frame_count;
        let kr = keyframe_received;

        let session_res = CompressionSession::new(
            64,
            64,
            VideoCodec::Hevc,
            5000,
            move |err, _flags, sample_buf| {
                if err.code() == 0 && !sample_buf.is_null() {
                    fc.fetch_add(1, Ordering::SeqCst);
                    kr.store(true, Ordering::SeqCst);
                }
            },
        );

        assert!(
            session_res.is_ok(),
            "CompressionSession creation failed: {:?}",
            session_res.err()
        );

        let session = session_res.unwrap();

        // Issue #039 verification: set_bitrate returns ok (OSStatus 0)
        let bitrate_res = session.set_bitrate(10_000);
        assert_eq!(bitrate_res, Ok(()));

        // Create test surface and encode
        let width_key = core_foundation::string::CFString::new("IOSurfaceWidth");
        let height_key = core_foundation::string::CFString::new("IOSurfaceHeight");
        let bytes_per_elem_key = core_foundation::string::CFString::new("IOSurfaceBytesPerElement");
        let pixel_format_key = core_foundation::string::CFString::new("IOSurfacePixelFormat");

        let width_val = core_foundation::number::CFNumber::from(64);
        let height_val = core_foundation::number::CFNumber::from(64);
        let bytes_per_elem_val = core_foundation::number::CFNumber::from(4);
        // '420v' bi-planar YCbCr 4:2:0 video range NV12 pixel format
        let pixel_format_val = core_foundation::number::CFNumber::from(0x3432_3076_i32);

        let dict = core_foundation::dictionary::CFDictionary::from_CFType_pairs(&[
            (width_key.as_CFType(), width_val.as_CFType()),
            (height_key.as_CFType(), height_val.as_CFType()),
            (
                bytes_per_elem_key.as_CFType(),
                bytes_per_elem_val.as_CFType(),
            ),
            (pixel_format_key.as_CFType(), pixel_format_val.as_CFType()),
        ]);

        // SAFETY: dict is a valid CFDictionary.
        let raw_surface = unsafe { IOSurfaceCreate(dict.as_concrete_TypeRef().cast()) };
        assert!(!raw_surface.is_null());

        // SAFETY: raw_surface is a valid surface.
        let surface = unsafe { IoSurface::from_raw(raw_surface) }.unwrap();

        // Submit frame forcing keyframe
        let encode_res = session.encode_surface(&surface, 1_000_000_000, true);
        assert_eq!(encode_res, Ok(()));

        // Session drops cleanly when leaving scope
    }
}
