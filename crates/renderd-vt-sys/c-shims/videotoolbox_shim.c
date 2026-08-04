#include "videotoolbox_shim.h"

OSStatus renderd_VTCompressionSessionCreate(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    uint32_t initial_bitrate_kbps,
    RenderD_VTOutputCallback callback,
    void *callback_ctx,
    VTCompressionSessionRef *session_out
) {
    if (session_out == NULL || callback == NULL || width <= 0 || height <= 0) {
        return kVTParameterErr;
    }

    VTCompressionSessionRef session = NULL;
    OSStatus status = VTCompressionSessionCreate(
        kCFAllocatorDefault,
        width,
        height,
        codec_type,
        NULL,
        NULL,
        kCFAllocatorDefault,
        (VTCompressionOutputCallback)callback,
        callback_ctx,
        &session
    );

    if (status != noErr || session == NULL) {
        return status;
    }

    // 1. Enable RealTime mode for ultra-low latency streaming
    VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);

    // 2. Disable frame reordering (B-frames) to ensure 0-frame latency (P-frames / IDR only)
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);

    // 3. Set MaxKeyFrameIntervalDuration to 0.5 seconds (RFC-0002 §6.1)
    double max_keyframe_interval_sec = 0.5;
    CFNumberRef max_keyframe_interval = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberFloat64Type,
        &max_keyframe_interval_sec
    );
    if (max_keyframe_interval != NULL) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, max_keyframe_interval);
        CFRelease(max_keyframe_interval);
    }

    // 4. Set initial target bitrate
    if (initial_bitrate_kbps > 0) {
        renderd_VTCompressionSessionSetBitrate(session, initial_bitrate_kbps);
    }

    // Prepare session for frame processing
    status = VTCompressionSessionPrepareToEncodeFrames(session);
    if (status != noErr) {
        VTCompressionSessionInvalidate(session);
        CFRelease(session);
        return status;
    }

    *session_out = session;
    return noErr;
}

OSStatus renderd_VTCompressionSessionSetBitrate(
    VTCompressionSessionRef session,
    uint32_t bitrate_kbps
) {
    if (session == NULL) {
        return kVTInvalidSessionErr;
    }

    int64_t bitrate_bps = (int64_t)bitrate_kbps * 1000;
    CFNumberRef bitrate_num = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt64Type,
        &bitrate_bps
    );
    if (bitrate_num == NULL) {
        return kVTAllocationFailedErr;
    }

    OSStatus status = VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, bitrate_num);
    CFRelease(bitrate_num);

    return status;
}

OSStatus renderd_VTCompressionSessionEncodeFrame(
    VTCompressionSessionRef session,
    CVImageBufferRef image_buffer,
    int64_t pts_ns,
    bool force_keyframe,
    void *frame_ctx
) {
    if (session == NULL || image_buffer == NULL) {
        return kVTParameterErr;
    }

    // Construct CMTime presentation timestamp (nanoseconds / 1_000_000_000)
    CMTime pts = CMTimeMake(pts_ns, 1000000000);

    CFDictionaryRef frame_props = NULL;
    if (force_keyframe) {
        const void *keys[] = { kVTEncodeFrameOptionKey_ForceKeyFrame };
        const void *values[] = { kCFBooleanTrue };
        frame_props = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys,
            values,
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks
        );
    }

    VTEncodeInfoFlags flags_out = 0;
    OSStatus status = VTCompressionSessionEncodeFrame(
        session,
        image_buffer,
        pts,
        kCMTimeInvalid,
        frame_props,
        frame_ctx,
        &flags_out
    );

    if (frame_props != NULL) {
        CFRelease(frame_props);
    }

    return status;
}

void renderd_VTCompressionSessionInvalidate(
    VTCompressionSessionRef session
) {
    if (session != NULL) {
        VTCompressionSessionInvalidate(session);
        CFRelease(session);
    }
}
