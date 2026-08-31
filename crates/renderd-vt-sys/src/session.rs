#![allow(unsafe_code)]

//! Safe RAII Rust wrapper for `VTCompressionSessionRef`.
//!
//! `CompressionSession` manages the lifecycle of an Apple `VideoToolbox` hardware
//! video encoder instance configured for ultra-low-latency real-time P2P streaming
//! (RFC-0002 §6.1).

use std::sync::Arc;

use crate::bindings::{
    renderd_CVPixelBufferCopyBGRA, renderd_CVPixelBufferGetDimensions,
    renderd_VTCompressionSessionCreate, renderd_VTCompressionSessionEncodeFrame,
    renderd_VTCompressionSessionInvalidate, renderd_VTCompressionSessionSetBitrate,
    renderd_VTDecompressionSessionCreate, renderd_VTDecompressionSessionCreateFromNAL,
    renderd_VTDecompressionSessionDecodeFrame, renderd_VTDecompressionSessionInvalidate,
    renderd_VTDecompressionSessionWaitForAsynchronousFrames, CMSampleBufferRef, CMVideoCodecType,
    CVImageBufferRef, OSStatus, RenderD_VTDecompressionContext, VTCompressionSessionRef,
    VTDecodeInfoFlags, VTDecompressionSessionRef, VTEncodeInfoFlags, CODEC_TYPE_H264,
    CODEC_TYPE_HEVC,
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

/// Type signature for decoded frame output callbacks.
///
/// Parameters:
/// - `status`: `VtError` status code (`0` / `noErr` on success)
/// - `info_flags`: `VideoToolbox` decode info flags
/// - `image_buffer`: `CVImageBufferRef` (`CVPixelBuffer`) containing uncompressed pixels
/// - `frame_id`: Frame sequence identifier
/// - `pts_ns`: presentation timestamp in nanoseconds
pub type DecompressionOutputCallback =
    Arc<dyn Fn(VtError, VTDecodeInfoFlags, CVImageBufferRef, u64, i64) + Send + Sync + 'static>;

struct DecompressionCallbackBox {
    context: RenderD_VTDecompressionContext,
    callback: DecompressionOutputCallback,
}

unsafe extern "C" fn vt_decompression_output_callback(
    output_callback_ref_con: *mut std::ffi::c_void,
    source_frame_ref_con: *mut std::ffi::c_void,
    status: OSStatus,
    info_flags: VTDecodeInfoFlags,
    image_buffer: CVImageBufferRef,
    pts_ns: i64,
) {
    if output_callback_ref_con.is_null() {
        return;
    }
    let frame_id = source_frame_ref_con as usize as u64;
    let box_ptr = output_callback_ref_con.cast::<DecompressionCallbackBox>();
    let cb_box = unsafe { &*box_ptr };
    (cb_box.callback)(VtError(status), info_flags, image_buffer, frame_id, pts_ns);
}

/// Safe RAII wrapper around `VTDecompressionSessionRef`.
///
/// Encapsulates hardware-accelerated H.265/H.264 video decoding using Apple's `VideoToolbox` framework.
pub struct DecompressionSession {
    session: VTDecompressionSessionRef,
    codec: VideoCodec,
    width: i32,
    height: i32,
    /// Most recent `CMVideoFormatDescriptionRef` seen on this session.
    ///
    /// Parameter sets ride along with keyframes only, so every other packet has to be
    /// described by whatever the last keyframe produced. Held per session — a shared
    /// static would leak one stream's parameter sets into another's.
    format_desc: std::sync::atomic::AtomicPtr<std::ffi::c_void>,
    _callback_box: Box<DecompressionCallbackBox>,
}

impl DecompressionSession {
    /// Creates and initializes a new hardware decompression session.
    ///
    /// # Errors
    /// Returns [`VtError`] if session creation or format description fails.
    pub fn new<F>(width: i32, height: i32, codec: VideoCodec, callback: F) -> Result<Self, VtError>
    where
        F: Fn(VtError, VTDecodeInfoFlags, CVImageBufferRef, u64, i64) + Send + Sync + 'static,
    {
        let mut cb_box = Box::new(DecompressionCallbackBox {
            context: RenderD_VTDecompressionContext {
                callback: vt_decompression_output_callback,
                user_ctx: std::ptr::null_mut(),
            },
            callback: Arc::new(callback),
        });

        let raw_box_ptr =
            std::ptr::from_ref::<DecompressionCallbackBox>(cb_box.as_ref()).cast_mut();
        cb_box.context.user_ctx = raw_box_ptr.cast::<std::ffi::c_void>();
        let raw_ctx_ptr = std::ptr::addr_of_mut!(cb_box.context).cast::<std::ffi::c_void>();

        let mut session: VTDecompressionSessionRef = std::ptr::null_mut();

        // SAFETY: renderd_VTDecompressionSessionCreate initializes session handle.
        let status = unsafe {
            renderd_VTDecompressionSessionCreate(
                width,
                height,
                codec.to_fourcc(),
                vt_decompression_output_callback,
                raw_ctx_ptr,
                &mut session,
            )
        };

        if status != 0 || session.is_null() {
            return Err(VtError(status));
        }

        Ok(Self {
            session,
            codec,
            width,
            height,
            format_desc: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
            _callback_box: cb_box,
        })
    }

    /// Creates and initializes a new hardware decompression session using parameter sets from a keyframe NAL packet.
    ///
    /// # Errors
    /// Returns [`VtError`] if session creation or format description fails.
    pub fn from_nal<F>(
        width: i32,
        height: i32,
        codec: VideoCodec,
        nal_data: &[u8],
        callback: F,
    ) -> Result<Self, VtError>
    where
        F: Fn(VtError, VTDecodeInfoFlags, CVImageBufferRef, u64, i64) + Send + Sync + 'static,
    {
        let mut cb_box = Box::new(DecompressionCallbackBox {
            context: RenderD_VTDecompressionContext {
                callback: vt_decompression_output_callback,
                user_ctx: std::ptr::null_mut(),
            },
            callback: Arc::new(callback),
        });

        let raw_box_ptr =
            std::ptr::from_ref::<DecompressionCallbackBox>(cb_box.as_ref()).cast_mut();
        cb_box.context.user_ctx = raw_box_ptr.cast::<std::ffi::c_void>();
        let raw_ctx_ptr = std::ptr::addr_of_mut!(cb_box.context).cast::<std::ffi::c_void>();

        let mut session: VTDecompressionSessionRef = std::ptr::null_mut();

        // SAFETY: renderd_VTDecompressionSessionCreateFromNAL initializes session handle.
        let status = unsafe {
            renderd_VTDecompressionSessionCreateFromNAL(
                width,
                height,
                codec.to_fourcc(),
                nal_data.as_ptr(),
                nal_data.len(),
                vt_decompression_output_callback,
                raw_ctx_ptr,
                &mut session,
            )
        };

        if status != 0 || session.is_null() {
            return Err(VtError(status));
        }

        Ok(Self {
            session,
            codec,
            width,
            height,
            format_desc: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
            _callback_box: cb_box,
        })
    }

    /// Submits a compressed NAL unit bitstream packet for decoding.
    ///
    /// # Errors
    /// Returns [`VtError`] if frame submission fails.
    pub fn decode_frame(&self, data: &[u8], pts_ns: i64) -> Result<(), VtError> {
        self.decode_frame_with_ctx(data, pts_ns, std::ptr::null_mut())
    }

    /// Submits a compressed NAL unit bitstream packet with a custom frame context pointer.
    ///
    /// # Errors
    /// Returns [`VtError`] if frame submission fails.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn decode_frame_with_ctx(
        &self,
        data: &[u8],
        pts_ns: i64,
        frame_ctx: *mut std::ffi::c_void,
    ) -> Result<(), VtError> {
        if self.session.is_null() || data.is_empty() {
            return Err(VtError(VtError::PARAMETER));
        }
        // SAFETY: self.session is a valid non-null VTDecompressionSessionRef, and
        // format_desc is a stable per-session slot the shim retains and releases.
        let status = unsafe {
            renderd_VTDecompressionSessionDecodeFrame(
                self.session,
                self.codec.to_fourcc(),
                self.width,
                self.height,
                self.format_desc.as_ptr(),
                data.as_ptr(),
                data.len(),
                pts_ns,
                frame_ctx,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(VtError(status))
        }
    }

    /// Synchronously waits for all in-flight asynchronous decompression frames to finish.
    ///
    /// # Errors
    /// Returns [`VtError`] if wait fails.
    pub fn wait_for_async_frames(&self) -> Result<(), VtError> {
        if self.session.is_null() {
            return Err(VtError(VtError::INVALID_SESSION));
        }
        // SAFETY: self.session is a valid non-null VTDecompressionSessionRef.
        let status =
            unsafe { renderd_VTDecompressionSessionWaitForAsynchronousFrames(self.session) };
        if status == 0 {
            Ok(())
        } else {
            Err(VtError(status))
        }
    }
}

impl Drop for DecompressionSession {
    fn drop(&mut self) {
        if !self.session.is_null() {
            // SAFETY: self.session is a valid non-null VTDecompressionSessionRef handle.
            unsafe {
                renderd_VTDecompressionSessionInvalidate(self.session);
            }
            self.session = std::ptr::null_mut();
        }

        // Release the cached format description the shim retained for this session.
        let cached = self
            .format_desc
            .swap(std::ptr::null_mut(), std::sync::atomic::Ordering::AcqRel);
        if !cached.is_null() {
            // SAFETY: `cached` was produced by CFRetain inside the shim and is owned here.
            unsafe {
                crate::bindings::renderd_CFRelease(cached);
            }
        }
    }
}

// SAFETY: DecompressionSession internal C session handle and callback context are safe to transfer across threads.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for DecompressionSession {}

// SAFETY: Method calls on DecompressionSession use thread-safe VideoToolbox C APIs.
unsafe impl Sync for DecompressionSession {}

/// Helper function to copy BGRA32 pixel data from a `CVPixelBuffer` handle into a byte slice.
///
/// # Errors
/// Returns [`VtError`] if locking or buffer copy fails.
///
/// # Safety
/// `image_buffer` must be a valid live `CVImageBufferRef`.
pub unsafe fn copy_pixel_buffer_bgra(
    image_buffer: CVImageBufferRef,
    out_dest: &mut [u8],
) -> Result<(u32, u32), VtError> {
    if image_buffer.is_null() {
        return Err(VtError(VtError::PARAMETER));
    }
    let mut width: i32 = 0;
    let mut height: i32 = 0;
    // SAFETY: image_buffer is a valid CVImageBufferRef and out_dest is a mutable byte slice.
    let status = unsafe {
        renderd_CVPixelBufferCopyBGRA(
            image_buffer,
            out_dest.as_mut_ptr(),
            out_dest.len(),
            &mut width,
            &mut height,
        )
    };
    if status == 0 {
        let w = u32::try_from(width).unwrap_or_default();
        let h = u32::try_from(height).unwrap_or_default();
        Ok((w, h))
    } else {
        Err(VtError(status))
    }
}

/// Helper function to retrieve pixel width and height of a `CVPixelBuffer` handle.
///
/// # Safety
/// `image_buffer` must be a valid `CVImageBufferRef`.
#[must_use]
pub unsafe fn get_pixel_buffer_dimensions(image_buffer: CVImageBufferRef) -> (u32, u32) {
    let mut width: i32 = 0;
    let mut height: i32 = 0;
    if !image_buffer.is_null() {
        // SAFETY: image_buffer is a valid CVImageBufferRef.
        unsafe {
            renderd_CVPixelBufferGetDimensions(image_buffer, &mut width, &mut height);
        }
    }
    let w = u32::try_from(width).unwrap_or_default();
    let h = u32::try_from(height).unwrap_or_default();
    (w, h)
}

/// Extracts NAL units (including VPS/SPS/PPS parameter sets on keyframes) from a `CMSampleBufferRef`.
///
/// # Errors
/// Returns [`VtError`] if extraction or locking fails.
///
/// # Safety
/// `sample_buffer` must be a valid `CMSampleBufferRef`.
pub unsafe fn sample_buffer_extract_nals(
    sample_buffer: CMSampleBufferRef,
) -> Result<(Vec<u8>, bool), VtError> {
    if sample_buffer.is_null() {
        return Err(VtError(VtError::PARAMETER));
    }

    let mut out_buf = vec![0u8; 4 * 1024 * 1024];
    let mut out_size: usize = 0;
    let mut is_keyframe: bool = false;

    // SAFETY: sample_buffer is a valid CMSampleBufferRef and out_buf has 4MB capacity.
    let status = unsafe {
        crate::bindings::renderd_CMSampleBufferExtractNALs(
            sample_buffer,
            out_buf.as_mut_ptr(),
            out_buf.len(),
            &mut out_size,
            &mut is_keyframe,
        )
    };

    if status == 0 {
        out_buf.truncate(out_size);
        Ok((out_buf, is_keyframe))
    } else {
        Err(VtError(status))
    }
}

/// Reads the presentation timestamp of an encoded `CMSampleBufferRef`, in nanoseconds.
///
/// # Errors
/// Returns [`VtError`] if the sample buffer is null or carries no valid timestamp.
///
/// # Safety
/// `sample_buffer` must be a valid `CMSampleBufferRef`.
pub unsafe fn sample_buffer_presentation_time_ns(
    sample_buffer: CMSampleBufferRef,
) -> Result<i64, VtError> {
    if sample_buffer.is_null() {
        return Err(VtError(VtError::PARAMETER));
    }

    let mut pts_ns: i64 = 0;
    // SAFETY: sample_buffer is a valid CMSampleBufferRef and pts_ns is a valid out-pointer.
    let status = unsafe {
        crate::bindings::renderd_CMSampleBufferGetPresentationTimeNanos(sample_buffer, &mut pts_ns)
    };

    if status == 0 {
        Ok(pts_ns)
    } else {
        Err(VtError(status))
    }
}

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
    #[ignore = "Requires hardware VideoToolbox GPU acceleration (unavailable in virtualized CI)"]
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

        // VideoToolbox hardware may be unavailable in CI (virtual macOS runners
        // without a GPU or with restricted entitlements). Skip rather than fail.
        let session = match session_res {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "test_create_set_bitrate_and_drop_session: \
                     CompressionSession::new returned VtError({}) — \
                     VideoToolbox hardware unavailable in this environment; skipping.",
                    e.code()
                );
                return;
            }
        };

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
        if raw_surface.is_null() {
            eprintln!(
                "test_create_set_bitrate_and_drop_session: \
                 IOSurfaceCreate returned null — \
                 IOSurface unavailable in this environment; skipping encode step."
            );
            return;
        }

        // SAFETY: raw_surface is a valid surface.
        let surface = unsafe { IoSurface::from_raw(raw_surface) }.unwrap();

        // Submit frame forcing keyframe
        let encode_res = session.encode_surface(&surface, 1_000_000_000, true);
        assert_eq!(encode_res, Ok(()));

        // Session drops cleanly when leaving scope
    }

    #[test]
    fn test_create_and_drop_decompression_session() {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let decoded_count = Arc::new(AtomicUsize::new(0));
            let dc = decoded_count.clone();

            let session_res = DecompressionSession::new(
                64,
                64,
                codec,
                move |_err, _flags, _image_buf, _frame_id, _pts_ns| {
                    dc.fetch_add(1, Ordering::SeqCst);
                },
            );

            if codec == VideoCodec::Hevc && session_res.is_err() {
                continue;
            }

            assert!(
                session_res.is_ok(),
                "DecompressionSession creation failed for {:?}: {:?}",
                codec,
                session_res.err()
            );

            let session = session_res.unwrap();
            assert!(session.wait_for_async_frames().is_ok());
        }
    }
}
