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

/// Opaque pointer to `VideoToolbox` decompression session.
pub type VTDecompressionSessionRef = *mut std::ffi::c_void;

/// Bitfield flags passed to `VideoToolbox` decode output callback.
pub type VTDecodeInfoFlags = u32;

/// Raw C callback function signature invoked when `VideoToolbox` finishes decompressing a frame.
#[allow(non_camel_case_types)]
pub type RenderD_VTDecompressionOutputCallback = unsafe extern "C" fn(
    output_callback_ref_con: *mut std::ffi::c_void,
    source_frame_ref_con: *mut std::ffi::c_void,
    status: OSStatus,
    info_flags: VTDecodeInfoFlags,
    image_buffer: CVImageBufferRef,
    pts_ns: i64,
);

/// Context structure for bridging C decompression output callback.
#[repr(C)]
pub struct RenderD_VTDecompressionContext {
    /// Function pointer to raw C callback.
    pub callback: RenderD_VTDecompressionOutputCallback,
    /// User context pointer.
    pub user_ctx: *mut std::ffi::c_void,
}

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

    /// Creates a hardware-accelerated `VTDecompressionSession` configured for low-latency video decoding.
    pub fn renderd_VTDecompressionSessionCreate(
        width: i32,
        height: i32,
        codec_type: CMVideoCodecType,
        callback: RenderD_VTDecompressionOutputCallback,
        callback_ctx: *mut std::ffi::c_void,
        session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;

    /// Creates a hardware-accelerated `VTDecompressionSession` using parameter sets from a keyframe NAL packet.
    pub fn renderd_VTDecompressionSessionCreateFromNAL(
        width: i32,
        height: i32,
        codec_type: CMVideoCodecType,
        data: *const u8,
        data_len: usize,
        callback: RenderD_VTDecompressionOutputCallback,
        callback_ctx: *mut std::ffi::c_void,
        session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;

    /// Submits a compressed video NAL bitstream packet to the decompression session for decoding.
    pub fn renderd_VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        data: *const u8,
        data_len: usize,
        pts_ns: i64,
        frame_ctx: *mut std::ffi::c_void,
    ) -> OSStatus;

    /// Synchronously waits for all in-flight asynchronous decompression frames to complete.
    pub fn renderd_VTDecompressionSessionWaitForAsynchronousFrames(
        session: VTDecompressionSessionRef,
    ) -> OSStatus;

    /// Invalidates and releases a `VTDecompressionSessionRef` handle.
    pub fn renderd_VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);

    /// Retrieves pixel dimensions of a `CVPixelBuffer` handle.
    pub fn renderd_CVPixelBufferGetDimensions(
        image_buffer: CVImageBufferRef,
        out_width: *mut i32,
        out_height: *mut i32,
    );

    /// Locks a `CVPixelBuffer` handle and copies BGRA32 pixel data to `out_dest`.
    pub fn renderd_CVPixelBufferCopyBGRA(
        image_buffer: CVImageBufferRef,
        out_dest: *mut u8,
        dest_capacity: usize,
        out_width: *mut i32,
        out_height: *mut i32,
    ) -> OSStatus;

    /// Extracts NAL units (including VPS/SPS/PPS parameter sets on keyframes) from a `CMSampleBufferRef`.
    pub fn renderd_CMSampleBufferExtractNALs(
        sample_buffer: CMSampleBufferRef,
        out_buf: *mut u8,
        max_capacity: usize,
        out_size: *mut usize,
        out_is_keyframe: *mut bool,
    ) -> OSStatus;

    /// Reads the presentation timestamp of a `CMSampleBufferRef`, rescaled to nanoseconds.
    pub fn renderd_CMSampleBufferGetPresentationTimeNanos(
        sample_buffer: CMSampleBufferRef,
        out_pts_ns: *mut i64,
    ) -> OSStatus;
}
