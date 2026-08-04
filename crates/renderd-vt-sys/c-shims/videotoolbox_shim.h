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
///
/// Parameters:
/// - width: frame width in pixels
/// - height: frame height in pixels
/// - codec_type: kCMVideoCodecType_HEVC ('hvc1') or kCMVideoCodecType_H264 ('avc1')
/// - initial_bitrate_kbps: target bitrate in kilobits per second
/// - callback: function pointer to output callback
/// - callback_ctx: user context pointer passed to output callback
/// - session_out: receives the created VTCompressionSessionRef on success
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
///
/// Parameters:
/// - session: valid VTCompressionSessionRef handle
/// - image_buffer: CVImageBufferRef (or IOSurface-backed pixel buffer)
/// - pts_ns: presentation timestamp in nanoseconds
/// - force_keyframe: if true, forces an IDR keyframe output
/// - frame_ctx: optional per-frame context pointer passed to callback
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

#ifdef __cplusplus
}
#endif

#endif // RENDERD_VIDEOTOOLBOX_SHIM_H
