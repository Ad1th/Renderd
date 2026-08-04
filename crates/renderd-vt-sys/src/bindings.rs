#![allow(unsafe_code)]

//! Raw FFI bindings to `VideoToolbox` C API and `videotoolbox_shim.c`.

use core_foundation::base::CFTypeRef;

/// Opaque pointer to `VideoToolbox` compression session.
pub type VTCompressionSessionRef = *mut std::ffi::c_void;

/// Opaque pointer to `CoreVideo` image buffer.
pub type CVImageBufferRef = *const std::ffi::c_void;

/// Opaque pointer to `CoreMedia` sample buffer.
pub type CMSampleBufferRef = CFTypeRef;

/// macOS `OSStatus` error code integer.
pub type OSStatus = i32;

/// Bitfield flags passed to `VideoToolbox` output callback.
pub type VTEncodeInfoFlags = u32;

/// Four-character code for video codec type (`hvc1` or `avc1`).
pub type CMVideoCodecType = u32;

/// `FourCC` constant for HEVC (H.265): `'hvc1'`.
pub const CODEC_TYPE_HEVC: CMVideoCodecType = u32::from_be_bytes(*b"hvc1");

/// `FourCC` constant for AVC (H.264): `'avc1'`.
pub const CODEC_TYPE_H264: CMVideoCodecType = u32::from_be_bytes(*b"avc1");

/// Raw C callback function signature invoked when `VideoToolbox` finishes encoding a frame.
#[allow(non_camel_case_types)]
pub type RenderD_VTOutputCallback = unsafe extern "C" fn(
    output_callback_ref_con: *mut std::ffi::c_void,
    source_frame_ref_con: *mut std::ffi::c_void,
    status: OSStatus,
    info_flags: VTEncodeInfoFlags,
    sample_buffer: CMSampleBufferRef,
);

extern "C" {
    /// Creates a hardware-accelerated `VTCompressionSession` configured for real-time low-latency streaming.
    pub fn renderd_VTCompressionSessionCreate(
        width: i32,
        height: i32,
        codec_type: CMVideoCodecType,
        initial_bitrate_kbps: u32,
        callback: RenderD_VTOutputCallback,
        callback_ctx: *mut std::ffi::c_void,
        session_out: *mut VTCompressionSessionRef,
    ) -> OSStatus;

    /// Dynamically updates the target average bitrate for an active compression session.
    pub fn renderd_VTCompressionSessionSetBitrate(
        session: VTCompressionSessionRef,
        bitrate_kbps: u32,
    ) -> OSStatus;

    /// Submits an image buffer to the compression session for encoding.
    pub fn renderd_VTCompressionSessionEncodeFrame(
        session: VTCompressionSessionRef,
        image_buffer: CVImageBufferRef,
        pts_ns: i64,
        force_keyframe: bool,
        frame_ctx: *mut std::ffi::c_void,
    ) -> OSStatus;

    /// Invalidates and releases a `VTCompressionSessionRef` handle.
    pub fn renderd_VTCompressionSessionInvalidate(session: VTCompressionSessionRef);
}
