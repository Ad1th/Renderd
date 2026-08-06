#ifndef RENDERD_VIDEOTOOLBOX_SHIM_H
#define RENDERD_VIDEOTOOLBOX_SHIM_H

#include <CoreFoundation/CoreFoundation.h>
#include <CoreMedia/CoreMedia.h>
#include <VideoToolbox/VideoToolbox.h>
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Callback function invoked when VideoToolbox finishes compressing a frame.
typedef void (*RenderD_VTOutputCallback)(
    void *output_callback_ref_con,
    void *source_frame_ref_con,
    OSStatus status,
    VTEncodeInfoFlags info_flags,
    CMSampleBufferRef sample_buffer
);

/// Creates a hardware-accelerated VTCompressionSession configured for low-latency streaming.
OSStatus renderd_VTCompressionSessionCreate(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    uint32_t initial_bitrate_kbps,
    RenderD_VTOutputCallback callback,
    void *callback_ctx,
    VTCompressionSessionRef *session_out
);

/// Dynamically updates the target average bitrate for an active compression session.
OSStatus renderd_VTCompressionSessionSetBitrate(
    VTCompressionSessionRef session,
    uint32_t bitrate_kbps
);

/// Submits an image buffer to the compression session for encoding.
OSStatus renderd_VTCompressionSessionEncodeFrame(
    VTCompressionSessionRef session,
    CVImageBufferRef image_buffer,
    int64_t pts_ns,
    bool force_keyframe,
    void *frame_ctx
);

/// Invalidates and releases a VTCompressionSessionRef handle.
void renderd_VTCompressionSessionInvalidate(
    VTCompressionSessionRef session
);

/// Callback function invoked when VideoToolbox finishes decompressing a frame.
typedef void (*RenderD_VTDecompressionOutputCallback)(
    void *output_callback_ref_con,
    void *source_frame_ref_con,
    OSStatus status,
    VTDecodeInfoFlags info_flags,
    CVImageBufferRef image_buffer,
    int64_t pts_ns
);

/// Context structure for decompression output callback bridging.
typedef struct {
    RenderD_VTDecompressionOutputCallback callback;
    void *user_ctx;
} RenderD_VTDecompressionContext;

/// Creates a hardware-accelerated VTDecompressionSession configured for low-latency video decoding.
OSStatus renderd_VTDecompressionSessionCreate(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    RenderD_VTDecompressionOutputCallback callback,
    void *callback_ctx,
    VTDecompressionSessionRef *session_out
);

/// Creates a hardware-accelerated VTDecompressionSession using parameter sets from a keyframe NAL packet.
OSStatus renderd_VTDecompressionSessionCreateFromNAL(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    const uint8_t *data,
    size_t data_len,
    RenderD_VTDecompressionOutputCallback callback,
    void *callback_ctx,
    VTDecompressionSessionRef *session_out
);

/// Submits a compressed video NAL bitstream packet to the decompression session for decoding.
OSStatus renderd_VTDecompressionSessionDecodeFrame(
    VTDecompressionSessionRef session,
    const uint8_t *data,
    size_t data_len,
    int64_t pts_ns,
    void *frame_ctx
);

/// Synchronously waits for all in-flight asynchronous decompression frames to complete.
OSStatus renderd_VTDecompressionSessionWaitForAsynchronousFrames(
    VTDecompressionSessionRef session
);

/// Invalidates and releases a VTDecompressionSessionRef handle.
void renderd_VTDecompressionSessionInvalidate(
    VTDecompressionSessionRef session
);

/// Retrieves pixel dimensions of a CVPixelBuffer handle.
void renderd_CVPixelBufferGetDimensions(
    CVImageBufferRef image_buffer,
    int32_t *out_width,
    int32_t *out_height
);

/// Locks a CVPixelBuffer handle and copies BGRA32 pixel data to out_dest.
OSStatus renderd_CVPixelBufferCopyBGRA(
    CVImageBufferRef image_buffer,
    uint8_t *out_dest,
    size_t dest_capacity,
    int32_t *out_width,
    int32_t *out_height
);

/// Extracts NAL units (including VPS/SPS/PPS parameter sets on keyframes) from a CMSampleBufferRef.
OSStatus renderd_CMSampleBufferExtractNALs(
    CMSampleBufferRef sample_buffer,
    uint8_t *out_buf,
    size_t max_capacity,
    size_t *out_size,
    bool *out_is_keyframe
);

#ifdef __cplusplus
}
#endif

#endif // RENDERD_VIDEOTOOLBOX_SHIM_H
