#include <stdio.h>
#include <stdlib.h>
#include "videotoolbox_shim.h"

/// Per-frame shim tracing, off unless RENDERD_VT_TRACE is set to a non-empty,
/// non-"0" value.
///
/// These traces run in the encode and decode hot paths and write to unbuffered
/// stderr, which costs more per frame than the work being traced. They are kept
/// because they are the only visibility into the CoreMedia calls when a stream
/// fails, but they must not be on by default.
static int renderd_vt_trace_enabled(void) {
    static int cached = -1;
    if (cached < 0) {
        const char *value = getenv("RENDERD_VT_TRACE");
        cached = (value != NULL && value[0] != '\0' && value[0] != '0') ? 1 : 0;
    }
    return cached;
}

#define VT_TRACE(...)                                   \
    do {                                                \
        if (renderd_vt_trace_enabled()) {               \
            fprintf(stderr, __VA_ARGS__);               \
        }                                               \
    } while (0)

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

    // 3. Set Profile Level for maximum compression efficiency
    if (codec_type == kCMVideoCodecType_H264) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
    } else if (codec_type == kCMVideoCodecType_HEVC) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_HEVC_Main_AutoLevel);
    }

    // 4. Set MaxKeyFrameIntervalDuration to 0.5 seconds (RFC-0002 §6.1)
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

    // 5. Set initial target bitrate
    renderd_VTCompressionSessionSetBitrate(session, initial_bitrate_kbps);

    // 6. Prepare encoder for low-latency session execution
    VTCompressionSessionPrepareToEncodeFrames(session);

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

    // Convert kbps to bits per second (kVTCompressionPropertyKey_AverageBitRate expects bps)
    int64_t bits_per_sec = (int64_t)bitrate_kbps * 1000;
    CFNumberRef bps_num = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt64Type,
        &bits_per_sec
    );

    OSStatus status = kVTParameterErr;
    if (bps_num != NULL) {
        status = VTSessionSetProperty(
            session,
            kVTCompressionPropertyKey_AverageBitRate,
            bps_num
        );
        CFRelease(bps_num);
    }

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

    CMTime presentation_timestamp = CMTimeMake(pts_ns, 1000000000);
    CFDictionaryRef frame_properties = NULL;

    if (force_keyframe) {
        const void *keys[] = { kVTEncodeFrameOptionKey_ForceKeyFrame };
        const void *values[] = { kCFBooleanTrue };
        frame_properties = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys,
            values,
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks
        );
    }

    VTEncodeInfoFlags info_flags_out = 0;
    OSStatus status = VTCompressionSessionEncodeFrame(
        session,
        (CVPixelBufferRef)image_buffer,
        presentation_timestamp,
        kCMTimeInvalid,
        frame_properties,
        frame_ctx,
        &info_flags_out
    );

    if (frame_properties != NULL) {
        CFRelease(frame_properties);
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

static void internal_decompression_wrapper(
    void *decompressionOutputRefCon,
    void *sourceFrameRefCon,
    OSStatus status,
    VTDecodeInfoFlags infoFlags,
    CVImageBufferRef imageBuffer,
    CMTime presentationTimeStamp,
    CMTime presentationDuration
) {
    (void)infoFlags;
    (void)presentationDuration;
    VT_TRACE("[VT_SHIM TRACE 5]: internal_decompression_wrapper callback fired! refCon=%p, sourceFrame=%p, status=%d (0x%x), infoFlags=0x%x, imageBuffer=%p, pts_sec=%.3f\n",
            decompressionOutputRefCon, sourceFrameRefCon, (int)status, (unsigned int)status, (unsigned int)infoFlags, (void*)imageBuffer,
            presentationTimeStamp.timescale > 0 ? (double)presentationTimeStamp.value / presentationTimeStamp.timescale : 0.0);

    if (decompressionOutputRefCon == NULL) return;
    RenderD_VTDecompressionContext *ctx = (RenderD_VTDecompressionContext *)decompressionOutputRefCon;
    int64_t pts_ns = presentationTimeStamp.timescale > 0 ? (presentationTimeStamp.value * 1000000000) / presentationTimeStamp.timescale : 0;
    if (ctx->callback) {
        ctx->callback(ctx->user_ctx, sourceFrameRefCon, status, infoFlags, imageBuffer, pts_ns);
    }
}

static CMVideoFormatDescriptionRef renderd_CreateFormatDescriptionFromNAL(
    CMVideoCodecType codec_type,
    int32_t width,
    int32_t height,
    const uint8_t *data,
    size_t data_len
) {
    CMVideoFormatDescriptionRef format_desc = NULL;
    VT_TRACE("[VT_SHIM TRACE 2a]: renderd_CreateFormatDescriptionFromNAL: codec_type=0x%x, width=%d, height=%d, data_len=%zu\n",
            (unsigned int)codec_type, width, height, data_len);

    if (data != NULL && data_len > 8) {
        const uint8_t *vps_ptr = NULL; size_t vps_len = 0;
        const uint8_t *sps_ptr = NULL; size_t sps_len = 0;
        const uint8_t *pps_ptr = NULL; size_t pps_len = 0;

        size_t offset = 0;
        while (offset + 4 < data_len) {
            size_t start_code_len = 0;
            if (data[offset] == 0 && data[offset+1] == 0 && data[offset+2] == 0 && data[offset+3] == 1) {
                start_code_len = 4;
            } else if (data[offset] == 0 && data[offset+1] == 0 && data[offset+2] == 1) {
                start_code_len = 3;
            }

            if (start_code_len > 0) {
                size_t nal_start = offset + start_code_len;
                size_t next_start = data_len;

                for (size_t i = nal_start; i + 3 < data_len; i++) {
                    if ((data[i] == 0 && data[i+1] == 0 && data[i+2] == 0 && data[i+3] == 1) ||
                        (data[i] == 0 && data[i+1] == 0 && data[i+2] == 1)) {
                        next_start = i;
                        break;
                    }
                }

                size_t nal_size = next_start - nal_start;
                if (codec_type == kCMVideoCodecType_HEVC && nal_size > 0) {
                    uint8_t nal_type = (data[nal_start] >> 1) & 0x3F;
                    if (nal_type == 32) { vps_ptr = &data[nal_start]; vps_len = nal_size; }
                    else if (nal_type == 33) { sps_ptr = &data[nal_start]; sps_len = nal_size; }
                    else if (nal_type == 34) { pps_ptr = &data[nal_start]; pps_len = nal_size; }
                } else if (codec_type == kCMVideoCodecType_H264 && nal_size > 0) {
                    uint8_t nal_type = data[nal_start] & 0x1F;
                    if (nal_type == 7) { sps_ptr = &data[nal_start]; sps_len = nal_size; }
                    else if (nal_type == 8) { pps_ptr = &data[nal_start]; pps_len = nal_size; }
                }

                offset = next_start;
            } else {
                offset++;
            }
        }

        if (codec_type == kCMVideoCodecType_HEVC && vps_ptr != NULL && sps_ptr != NULL && pps_ptr != NULL) {
            const uint8_t *param_ptrs[3] = {vps_ptr, sps_ptr, pps_ptr};
            size_t param_sizes[3] = {vps_len, sps_len, pps_len};
            OSStatus status = CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                kCFAllocatorDefault,
                3,
                param_ptrs,
                param_sizes,
                4,
                NULL,
                &format_desc
            );
            VT_TRACE("[VT_SHIM TRACE 2a-HEVC]: CMVideoFormatDescriptionCreateFromHEVCParameterSets status=%d (0x%x), format_desc=%p\n",
                    (int)status, (unsigned int)status, (void*)format_desc);
        } else if (codec_type == kCMVideoCodecType_H264 && sps_ptr != NULL && pps_ptr != NULL) {
            const uint8_t *param_ptrs[2] = {sps_ptr, pps_ptr};
            size_t param_sizes[2] = {sps_len, pps_len};
            OSStatus status = CMVideoFormatDescriptionCreateFromH264ParameterSets(
                kCFAllocatorDefault,
                2,
                param_ptrs,
                param_sizes,
                4,
                &format_desc
            );
            VT_TRACE("[VT_SHIM TRACE 2a-H264]: CMVideoFormatDescriptionCreateFromH264ParameterSets status=%d (0x%x), format_desc=%p\n",
                    (int)status, (unsigned int)status, (void*)format_desc);
        }
    }

    if (format_desc == NULL && (data == NULL || data_len == 0)) {
        CMVideoFormatDescriptionCreate(
            kCFAllocatorDefault,
            codec_type,
            width > 0 ? width : 1920,
            height > 0 ? height : 1080,
            NULL,
            &format_desc
        );
    }

    VT_TRACE("[VT_SHIM TRACE 2b]: renderd_CreateFormatDescriptionFromNAL result: format_desc=%p\n", (void*)format_desc);
    return format_desc;
}

OSStatus renderd_VTDecompressionSessionCreateFromNAL(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    const uint8_t *data,
    size_t data_len,
    RenderD_VTDecompressionOutputCallback callback,
    void *callback_ctx,
    VTDecompressionSessionRef *session_out
) {
    if (session_out == NULL || callback == NULL || width <= 0 || height <= 0) {
        VT_TRACE("[VT_SHIM TRACE 2c-ERR]: renderd_VTDecompressionSessionCreateFromNAL parameter error!\n");
        return kVTParameterErr;
    }

    CMVideoFormatDescriptionRef format_desc = renderd_CreateFormatDescriptionFromNAL(codec_type, width, height, data, data_len);
    if (format_desc == NULL) {
        VT_TRACE("[VT_SHIM TRACE 2c-ERR]: renderd_CreateFormatDescriptionFromNAL returned NULL!\n");
        return kVTAllocationFailedErr;
    }

    CFMutableDictionaryRef destination_attrs = CFDictionaryCreateMutable(
        kCFAllocatorDefault,
        2,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );

    int32_t pixel_format = kCVPixelFormatType_32BGRA;
    CFNumberRef pixel_format_num = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt32Type,
        &pixel_format
    );
    if (pixel_format_num != NULL) {
        CFDictionarySetValue(destination_attrs, kCVPixelBufferPixelFormatTypeKey, pixel_format_num);
        CFRelease(pixel_format_num);
    }

    RenderD_VTDecompressionContext *ctx = (RenderD_VTDecompressionContext *)malloc(sizeof(RenderD_VTDecompressionContext));
    if (ctx == NULL) {
        CFRelease(destination_attrs);
        CFRelease(format_desc);
        VT_TRACE("[VT_SHIM TRACE 2c-ERR]: malloc(RenderD_VTDecompressionContext) failed!\n");
        return kVTAllocationFailedErr;
    }
    ctx->callback = callback;
    ctx->user_ctx = callback_ctx;

    VTDecompressionOutputCallbackRecord cb_record;
    cb_record.decompressionOutputCallback = internal_decompression_wrapper;
    cb_record.decompressionOutputRefCon = ctx;

    VTDecompressionSessionRef session = NULL;
    OSStatus status = VTDecompressionSessionCreate(
        kCFAllocatorDefault,
        format_desc,
        NULL,
        destination_attrs,
        &cb_record,
        &session
    );

    CFRelease(destination_attrs);
    CFRelease(format_desc);

    VT_TRACE("[VT_SHIM TRACE 2c]: VTDecompressionSessionCreate result: status=%d (0x%x), session=%p\n",
            (int)status, (unsigned int)status, (void*)session);

    if (status != noErr || session == NULL) {
        free(ctx);
        return status;
    }

    *session_out = session;
    return noErr;
}

OSStatus renderd_VTDecompressionSessionCreate(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    RenderD_VTDecompressionOutputCallback callback,
    void *callback_ctx,
    VTDecompressionSessionRef *session_out
) {
    return renderd_VTDecompressionSessionCreateFromNAL(
        width,
        height,
        codec_type,
        NULL,
        0,
        callback,
        callback_ctx,
        session_out
    );
}

OSStatus renderd_VTDecompressionSessionDecodeFrame(
    VTDecompressionSessionRef session,
    CMVideoCodecType codec_type,
    int32_t width,
    int32_t height,
    CMVideoFormatDescriptionRef *inout_format_desc,
    const uint8_t *data,
    size_t data_len,
    int64_t pts_ns,
    void *frame_ctx
) {
    VT_TRACE("[VT_SHIM TRACE 4a]: renderd_VTDecompressionSessionDecodeFrame: session=%p, data_len=%zu, pts_ns=%lld, frame_ctx=%p\n",
            (void*)session, data_len, (long long)pts_ns, frame_ctx);

    if (session == NULL || data == NULL || data_len == 0) {
        VT_TRACE("[VT_SHIM TRACE 4a-ERR]: Parameter error in renderd_VTDecompressionSessionDecodeFrame!\n");
        return kVTParameterErr;
    }

    uint8_t *temp_buf = (uint8_t *)malloc(data_len + 64);
    if (!temp_buf) return kVTAllocationFailedErr;

    const uint8_t *data_ptr = temp_buf;
    size_t block_len = 0;

    // Scan Annex-B bitstream and convert all 0x00000001 / 0x000001 startcodes to 4-byte big-endian NAL length prefixes
    size_t i = 0;
    while (i < data_len) {
        size_t startcode_len = 0;
        if (i + 4 <= data_len && data[i] == 0 && data[i+1] == 0 && data[i+2] == 0 && data[i+3] == 1) {
            startcode_len = 4;
        } else if (i + 3 <= data_len && data[i] == 0 && data[i+1] == 0 && data[i+2] == 1) {
            startcode_len = 3;
        }

        if (startcode_len > 0) {
            size_t nal_start = i + startcode_len;
            size_t next_start = nal_start;
            while (next_start < data_len) {
                if (next_start + 4 <= data_len && data[next_start] == 0 && data[next_start+1] == 0 && data[next_start+2] == 0 && data[next_start+3] == 1) {
                    break;
                }
                if (next_start + 3 <= data_len && data[next_start] == 0 && data[next_start+1] == 0 && data[next_start+2] == 1) {
                    break;
                }
                next_start++;
            }
            uint32_t nal_size = (uint32_t)(next_start - nal_start);
            temp_buf[block_len++] = (uint8_t)((nal_size >> 24) & 0xFF);
            temp_buf[block_len++] = (uint8_t)((nal_size >> 16) & 0xFF);
            temp_buf[block_len++] = (uint8_t)((nal_size >> 8) & 0xFF);
            temp_buf[block_len++] = (uint8_t)(nal_size & 0xFF);
            memcpy(temp_buf + block_len, data + nal_start, nal_size);
            block_len += nal_size;
            i = next_start;
        } else {
            i++;
        }
    }

    CMBlockBufferRef block_buffer = NULL;
    OSStatus status = CMBlockBufferCreateWithMemoryBlock(
        kCFAllocatorDefault,
        (void *)data_ptr,
        block_len,
        kCFAllocatorNull,
        NULL,
        0,
        block_len,
        0,
        &block_buffer
    );

    VT_TRACE("[VT_SHIM TRACE 4b]: CMBlockBufferCreateWithMemoryBlock: status=%d (0x%x), block_buffer=%p\n",
            (int)status, (unsigned int)status, (void*)block_buffer);

    if (status != noErr || block_buffer == NULL) {
        if (temp_buf) free(temp_buf);
        return status;
    }

    CMSampleTimingInfo timing_info;
    timing_info.duration = kCMTimeInvalid;
    timing_info.presentationTimeStamp = CMTimeMake(pts_ns, 1000000000);
    timing_info.decodeTimeStamp = kCMTimeInvalid;

    // Only keyframes carry parameter sets, so most packets produce no format
    // description; those reuse the one cached for this session by the caller.
    //
    // The codec and dimensions must come from the session rather than being assumed:
    // building this with a hardcoded HEVC type meant an H.264 stream never produced a
    // format description at all, and every frame failed with kVTVideoDecoderBadDataErr.
    CMVideoFormatDescriptionRef format_desc =
        renderd_CreateFormatDescriptionFromNAL(codec_type, width, height, data, data_len);

    if (format_desc != NULL) {
        if (inout_format_desc != NULL) {
            if (*inout_format_desc != NULL) {
                CFRelease(*inout_format_desc);
            }
            *inout_format_desc = (CMVideoFormatDescriptionRef)CFRetain(format_desc);
        }
    } else if (inout_format_desc != NULL && *inout_format_desc != NULL) {
        format_desc = (CMVideoFormatDescriptionRef)CFRetain(*inout_format_desc);
    }

    if (format_desc == NULL) {
        // Nothing to describe the sample with yet: the stream has not delivered a
        // keyframe since this session was created. Drop the packet rather than handing
        // CMSampleBufferCreate a NULL description it will reject.
        if (block_buffer) CFRelease(block_buffer);
        if (temp_buf) free(temp_buf);
        return kVTVideoDecoderBadDataErr;
    }

    CMSampleBufferRef sample_buffer = NULL;
    size_t sample_size = block_len;
    status = CMSampleBufferCreate(
        kCFAllocatorDefault,
        block_buffer,
        true,
        NULL,
        NULL,
        format_desc,
        1,
        1,
        &timing_info,
        1,
        &sample_size,
        &sample_buffer
    );

    if (format_desc != NULL) {
        CFRelease(format_desc);
    }

    VT_TRACE("[VT_SHIM TRACE 4c]: CMSampleBufferCreate: status=%d (0x%x), sample_buffer=%p\n",
            (int)status, (unsigned int)status, (void*)sample_buffer);

    if (status == noErr && sample_buffer != NULL) {
        VTDecodeInfoFlags info_flags_out = 0;
        VTDecodeFrameFlags decode_flags = kVTDecodeFrame_EnableAsynchronousDecompression;
        status = VTDecompressionSessionDecodeFrame(
            session,
            sample_buffer,
            decode_flags,
            frame_ctx,
            &info_flags_out
        );
        VT_TRACE("[VT_SHIM TRACE 4d]: VTDecompressionSessionDecodeFrame: status=%d (0x%x), info_flags_out=0x%x\n",
                (int)status, (unsigned int)status, (unsigned int)info_flags_out);
        CFRelease(sample_buffer);
    }

    CFRelease(block_buffer);
    if (temp_buf) free(temp_buf);

    return status;
}

OSStatus renderd_VTDecompressionSessionWaitForAsynchronousFrames(
    VTDecompressionSessionRef session
) {
    if (session == NULL) return kVTInvalidSessionErr;
    return VTDecompressionSessionWaitForAsynchronousFrames(session);
}

void renderd_VTDecompressionSessionInvalidate(
    VTDecompressionSessionRef session
) {
    if (session != NULL) {
        VTDecompressionSessionInvalidate(session);
        CFRelease(session);
    }
}

void renderd_CVPixelBufferGetDimensions(
    CVImageBufferRef image_buffer,
    int32_t *out_width,
    int32_t *out_height
) {
    if (image_buffer && out_width && out_height) {
        CVPixelBufferRef pix = (CVPixelBufferRef)image_buffer;
        *out_width = (int32_t)CVPixelBufferGetWidth(pix);
        *out_height = (int32_t)CVPixelBufferGetHeight(pix);
    }
}

OSStatus renderd_CVPixelBufferCopyBGRA(
    CVImageBufferRef image_buffer,
    uint8_t *out_dest,
    size_t dest_capacity,
    int32_t *out_width,
    int32_t *out_height
) {
    VT_TRACE("[VT_SHIM TRACE 6a]: renderd_CVPixelBufferCopyBGRA: image_buffer=%p, dest_capacity=%zu\n",
            (void*)image_buffer, dest_capacity);

    if (image_buffer == NULL || out_dest == NULL || out_width == NULL || out_height == NULL) {
        VT_TRACE("[VT_SHIM TRACE 6a-ERR]: Parameter error in renderd_CVPixelBufferCopyBGRA!\n");
        return kVTParameterErr;
    }

    CVPixelBufferRef pixel_buffer = (CVPixelBufferRef)image_buffer;
    OSStatus status = CVPixelBufferLockBaseAddress(pixel_buffer, kCVPixelBufferLock_ReadOnly);
    if (status != kCVReturnSuccess) {
        VT_TRACE("[VT_SHIM TRACE 6b-ERR]: CVPixelBufferLockBaseAddress status=%d (0x%x)\n",
                (int)status, (unsigned int)status);
        return status;
    }

    int32_t width = (int32_t)CVPixelBufferGetWidth(pixel_buffer);
    int32_t height = (int32_t)CVPixelBufferGetHeight(pixel_buffer);
    size_t bytes_per_row = CVPixelBufferGetBytesPerRow(pixel_buffer);
    uint8_t *base_addr = (uint8_t *)CVPixelBufferGetBaseAddress(pixel_buffer);

    VT_TRACE("[VT_SHIM TRACE 6b]: CVPixelBuffer locked base address: width=%d, height=%d, bytes_per_row=%zu, base_addr=%p\n",
            width, height, bytes_per_row, (void*)base_addr);

    *out_width = width;
    *out_height = height;

    size_t expected_size = (size_t)width * (size_t)height * 4;
    if (dest_capacity < expected_size || base_addr == NULL) {
        CVPixelBufferUnlockBaseAddress(pixel_buffer, kCVPixelBufferLock_ReadOnly);
        VT_TRACE("[VT_SHIM TRACE 6c-ERR]: Allocation error or NULL base address!\n");
        return kVTAllocationFailedErr;
    }

    if (bytes_per_row == (size_t)width * 4) {
        memcpy(out_dest, base_addr, expected_size);
    } else {
        size_t row_bytes = (size_t)width * 4;
        for (int32_t r = 0; r < height; r++) {
            memcpy(out_dest + (size_t)r * row_bytes, base_addr + (size_t)r * bytes_per_row, row_bytes);
        }
    }

    CVPixelBufferUnlockBaseAddress(pixel_buffer, kCVPixelBufferLock_ReadOnly);
    return noErr;
}

OSStatus renderd_CMSampleBufferExtractNALs(
    CMSampleBufferRef sample_buffer,
    uint8_t *out_buf,
    size_t max_capacity,
    size_t *out_size,
    bool *out_is_keyframe
) {
    if (sample_buffer == NULL || out_buf == NULL || out_size == NULL || out_is_keyframe == NULL) {
        return kVTParameterErr;
    }

    *out_size = 0;
    *out_is_keyframe = false;

    // 1. Determine keyframe status from sample attachments
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample_buffer, false);
    if (attachments != NULL && CFArrayGetCount(attachments) > 0) {
        CFDictionaryRef dict = (CFDictionaryRef)CFArrayGetValueAtIndex(attachments, 0);
        if (dict != NULL) {
            CFBooleanRef not_sync = (CFBooleanRef)CFDictionaryGetValue(dict, kCMSampleAttachmentKey_NotSync);
            if (not_sync == NULL || !CFBooleanGetValue(not_sync)) {
                *out_is_keyframe = true;
            }
        }
    } else {
        *out_is_keyframe = true;
    }

    size_t total_written = 0;

    // 2. On keyframes, extract VPS, SPS, PPS parameter sets from CMVideoFormatDescription
    if (*out_is_keyframe) {
        CMVideoFormatDescriptionRef format_desc = CMSampleBufferGetFormatDescription(sample_buffer);
        if (format_desc != NULL) {
            CMVideoCodecType codec_type = CMFormatDescriptionGetMediaSubType(format_desc);

            if (codec_type == kCMVideoCodecType_HEVC) {
                // HEVC parameter sets: VPS (0), SPS (1), PPS (2)
                for (size_t i = 0; i < 3; i++) {
                    const uint8_t *param_ptr = NULL;
                    size_t param_size = 0;
                    OSStatus status = CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(
                        format_desc,
                        i,
                        &param_ptr,
                        &param_size,
                        NULL,
                        NULL
                    );
                    if (status == noErr && param_ptr != NULL && param_size > 0) {
                        if (total_written + 4 + param_size <= max_capacity) {
                            out_buf[total_written++] = 0;
                            out_buf[total_written++] = 0;
                            out_buf[total_written++] = 0;
                            out_buf[total_written++] = 1;
                            memcpy(out_buf + total_written, param_ptr, param_size);
                            total_written += param_size;
                        }
                    }
                }
            } else if (codec_type == kCMVideoCodecType_H264) {
                // H.264 parameter sets: SPS (0), PPS (1)
                for (size_t i = 0; i < 2; i++) {
                    const uint8_t *param_ptr = NULL;
                    size_t param_size = 0;
                    OSStatus status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                        format_desc,
                        i,
                        &param_ptr,
                        &param_size,
                        NULL,
                        NULL
                    );
                    if (status == noErr && param_ptr != NULL && param_size > 0) {
                        if (total_written + 4 + param_size <= max_capacity) {
                            out_buf[total_written++] = 0;
                            out_buf[total_written++] = 0;
                            out_buf[total_written++] = 0;
                            out_buf[total_written++] = 1;
                            memcpy(out_buf + total_written, param_ptr, param_size);
                            total_written += param_size;
                        }
                    }
                }
            }
        }
    }

    // 3. Extract sample data block buffer NAL units
    CMBlockBufferRef block_buffer = CMSampleBufferGetDataBuffer(sample_buffer);
    if (block_buffer == NULL) {
        *out_size = total_written;
        return noErr;
    }

    size_t block_len = CMBlockBufferGetDataLength(block_buffer);
    if (block_len == 0) {
        *out_size = total_written;
        return noErr;
    }

    char *data_ptr = NULL;
    OSStatus status = CMBlockBufferGetDataPointer(
        block_buffer,
        0,
        NULL,
        NULL,
        &data_ptr
    );

    char *allocated_buf = NULL;
    if (status != noErr || data_ptr == NULL) {
        allocated_buf = (char *)malloc(block_len);
        if (allocated_buf == NULL) return kVTAllocationFailedErr;
        status = CMBlockBufferCopyDataBytes(block_buffer, 0, block_len, allocated_buf);
        if (status != noErr) {
            free(allocated_buf);
            return status;
        }
        data_ptr = allocated_buf;
    }

    // Convert AVCC / HVCC 4-byte big-endian length prefixes to Annex B 0x00000001 start codes
    size_t offset = 0;
    while (offset + 4 <= block_len) {
        uint32_t nal_len = (uint32_t)(((uint8_t)data_ptr[offset] << 24) |
                                     ((uint8_t)data_ptr[offset + 1] << 16) |
                                     ((uint8_t)data_ptr[offset + 2] << 8) |
                                     ((uint8_t)data_ptr[offset + 3]));
        offset += 4;

        if (offset + nal_len > block_len) {
            break;
        }

        if (total_written + 4 + nal_len <= max_capacity) {
            out_buf[total_written++] = 0;
            out_buf[total_written++] = 0;
            out_buf[total_written++] = 0;
            out_buf[total_written++] = 1;
            memcpy(out_buf + total_written, data_ptr + offset, nal_len);
            total_written += nal_len;
        }

        offset += nal_len;
    }

    if (allocated_buf != NULL) {
        free(allocated_buf);
    }

    *out_size = total_written;
    return noErr;
}

void renderd_CFRelease(void *obj) {
    if (obj != NULL) {
        CFRelease((CFTypeRef)obj);
    }
}

OSStatus renderd_CMSampleBufferGetPresentationTimeNanos(
    CMSampleBufferRef sample_buffer,
    int64_t *out_pts_ns
) {
    if (sample_buffer == NULL || out_pts_ns == NULL) {
        return kVTParameterErr;
    }

    *out_pts_ns = 0;

    CMTime pts = CMSampleBufferGetPresentationTimeStamp(sample_buffer);
    if (!CMTIME_IS_VALID(pts) || CMTIME_IS_INDEFINITE(pts)) {
        return kVTParameterErr;
    }

    // Rescale to a 1 ns timebase; CMTimeConvertScale saturates rather than wrapping.
    CMTime nanos = CMTimeConvertScale(pts, 1000000000, kCMTimeRoundingMethod_Default);
    if (!CMTIME_IS_VALID(nanos)) {
        return kVTParameterErr;
    }

    *out_pts_ns = (int64_t)nanos.value;
    return noErr;
}
