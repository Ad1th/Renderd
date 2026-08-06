#include <stdio.h>
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
    renderd_VTCompressionSessionSetBitrate(session, initial_bitrate_kbps);

    // 5. Prepare encoder for low-latency session execution
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

    // Convert kbps to bytes per second (RFC-0002 §6.1)
    int64_t bytes_per_sec = (int64_t)bitrate_kbps * 1000 / 8;
    CFNumberRef bps_num = CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt64Type,
        &bytes_per_sec
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
    fprintf(stderr, "[VT_SHIM TRACE 5]: internal_decompression_wrapper callback fired! refCon=%p, sourceFrame=%p, status=%d (0x%x), infoFlags=0x%x, imageBuffer=%p, pts_sec=%.3f\n",
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
    fprintf(stderr, "[VT_SHIM TRACE 2a]: renderd_CreateFormatDescriptionFromNAL: codec_type=0x%x, width=%d, height=%d, data_len=%zu\n",
            (unsigned int)codec_type, width, height, data_len);

    if (data != NULL && data_len > 8) {
        const uint8_t *ptrs[3] = {NULL, NULL, NULL};
        size_t sizes[3] = {0, 0, 0};
        size_t count = 0;

        size_t offset = 0;
        while (offset + 4 < data_len && count < 3) {
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
                    if (nal_type == 32 || nal_type == 33 || nal_type == 34) {
                        ptrs[count] = &data[nal_start];
                        sizes[count] = nal_size;
                        count++;
                    }
                } else if (codec_type == kCMVideoCodecType_H264 && nal_size > 0) {
                    uint8_t nal_type = data[nal_start] & 0x1F;
                    if (nal_type == 7 || nal_type == 8) {
                        ptrs[count] = &data[nal_start];
                        sizes[count] = nal_size;
                        count++;
                    }
                }

                offset = next_start;
            } else {
                offset++;
            }
        }

        if (count > 0) {
            if (codec_type == kCMVideoCodecType_HEVC) {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    kCFAllocatorDefault,
                    count,
                    ptrs,
                    sizes,
                    4,
                    NULL,
                    &format_desc
                );
            } else if (codec_type == kCMVideoCodecType_H264) {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    kCFAllocatorDefault,
                    count,
                    ptrs,
                    sizes,
                    4,
                    &format_desc
                );
            }
        }
    }

    if (format_desc == NULL && codec_type == kCMVideoCodecType_HEVC) {
        static const uint8_t dummy_hvcC[] = {
            0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x90, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x90, 0xf0, 0x00, 0xfc,
            0xfd, 0xf8, 0xf8, 0x00, 0x00, 0x0f, 0x00
        };
        CFDataRef hvcc_data = CFDataCreate(kCFAllocatorDefault, dummy_hvcC, sizeof(dummy_hvcC));
        if (hvcc_data != NULL) {
            CFMutableDictionaryRef atoms = CFDictionaryCreateMutable(
                kCFAllocatorDefault, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks
            );
            CFDictionarySetValue(atoms, CFSTR("hvcC"), hvcc_data);
            CFMutableDictionaryRef extensions = CFDictionaryCreateMutable(
                kCFAllocatorDefault, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks
            );
            CFDictionarySetValue(extensions, CFSTR("SampleDescriptionExtensionAtoms"), atoms);

            CMVideoFormatDescriptionCreate(
                kCFAllocatorDefault,
                codec_type,
                width > 0 ? width : 1920,
                height > 0 ? height : 1080,
                extensions,
                &format_desc
            );
            CFRelease(extensions);
            CFRelease(atoms);
            CFRelease(hvcc_data);
        }
    }

    if (format_desc == NULL) {
        CMVideoFormatDescriptionCreate(
            kCFAllocatorDefault,
            codec_type,
            width > 0 ? width : 1920,
            height > 0 ? height : 1080,
            NULL,
            &format_desc
        );
    }

    fprintf(stderr, "[VT_SHIM TRACE 2b]: renderd_CreateFormatDescriptionFromNAL result: format_desc=%p\n", (void*)format_desc);
    return format_desc;
}

OSStatus renderd_VTDecompressionSessionCreate(
    int32_t width,
    int32_t height,
    CMVideoCodecType codec_type,
    RenderD_VTDecompressionOutputCallback callback,
    void *callback_ctx,
    VTDecompressionSessionRef *session_out
) {
    if (session_out == NULL || callback == NULL || width <= 0 || height <= 0) {
        fprintf(stderr, "[VT_SHIM TRACE 2c-ERR]: renderd_VTDecompressionSessionCreate parameter error!\n");
        return kVTParameterErr;
    }

    CMVideoFormatDescriptionRef format_desc = renderd_CreateFormatDescriptionFromNAL(codec_type, width, height, NULL, 0);
    if (format_desc == NULL) {
        fprintf(stderr, "[VT_SHIM TRACE 2c-ERR]: renderd_CreateFormatDescriptionFromNAL returned NULL!\n");
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
        fprintf(stderr, "[VT_SHIM TRACE 2c-ERR]: malloc(RenderD_VTDecompressionContext) failed!\n");
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

    fprintf(stderr, "[VT_SHIM TRACE 2c]: VTDecompressionSessionCreate result: status=%d (0x%x), session=%p\n",
            (int)status, (unsigned int)status, (void*)session);

    if (status != noErr || session == NULL) {
        free(ctx);
        return status;
    }

    *session_out = session;
    return noErr;
}

OSStatus renderd_VTDecompressionSessionDecodeFrame(
    VTDecompressionSessionRef session,
    const uint8_t *data,
    size_t data_len,
    int64_t pts_ns,
    void *frame_ctx
) {
    fprintf(stderr, "[VT_SHIM TRACE 4a]: renderd_VTDecompressionSessionDecodeFrame: session=%p, data_len=%zu, pts_ns=%lld, frame_ctx=%p\n",
            (void*)session, data_len, (long long)pts_ns, frame_ctx);

    if (session == NULL || data == NULL || data_len == 0) {
        fprintf(stderr, "[VT_SHIM TRACE 4a-ERR]: Parameter error in renderd_VTDecompressionSessionDecodeFrame!\n");
        return kVTParameterErr;
    }

    uint8_t *temp_buf = NULL;
    const uint8_t *data_ptr = data;
    size_t block_len = data_len;

    if (data_len >= 4 && data[0] == 0 && data[1] == 0 && data[2] == 0 && data[3] == 1) {
        temp_buf = (uint8_t *)malloc(data_len);
        if (!temp_buf) return kVTAllocationFailedErr;
        memcpy(temp_buf, data, data_len);
        uint32_t nal_len = (uint32_t)(data_len - 4);
        temp_buf[0] = (uint8_t)((nal_len >> 24) & 0xFF);
        temp_buf[1] = (uint8_t)((nal_len >> 16) & 0xFF);
        temp_buf[2] = (uint8_t)((nal_len >> 8) & 0xFF);
        temp_buf[3] = (uint8_t)(nal_len & 0xFF);
        data_ptr = temp_buf;
    } else if (data_len >= 3 && data[0] == 0 && data[1] == 0 && data[2] == 1) {
        temp_buf = (uint8_t *)malloc(data_len + 1);
        if (!temp_buf) return kVTAllocationFailedErr;
        memcpy(temp_buf + 4, data + 3, data_len - 3);
        uint32_t nal_len = (uint32_t)(data_len - 3);
        temp_buf[0] = (uint8_t)((nal_len >> 24) & 0xFF);
        temp_buf[1] = (uint8_t)((nal_len >> 16) & 0xFF);
        temp_buf[2] = (uint8_t)((nal_len >> 8) & 0xFF);
        temp_buf[3] = (uint8_t)(nal_len & 0xFF);
        data_ptr = temp_buf;
        block_len = data_len + 1;
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

    fprintf(stderr, "[VT_SHIM TRACE 4b]: CMBlockBufferCreateWithMemoryBlock: status=%d (0x%x), block_buffer=%p\n",
            (int)status, (unsigned int)status, (void*)block_buffer);

    if (status != noErr || block_buffer == NULL) {
        if (temp_buf) free(temp_buf);
        return status;
    }

    CMSampleTimingInfo timing_info;
    timing_info.duration = kCMTimeInvalid;
    timing_info.presentationTimeStamp = CMTimeMake(pts_ns, 1000000000);
    timing_info.decodeTimeStamp = kCMTimeInvalid;

    CMSampleBufferRef sample_buffer = NULL;
    size_t sample_size = block_len;
    status = CMSampleBufferCreate(
        kCFAllocatorDefault,
        block_buffer,
        true,
        NULL,
        NULL,
        NULL,
        1,
        1,
        &timing_info,
        1,
        &sample_size,
        &sample_buffer
    );

    fprintf(stderr, "[VT_SHIM TRACE 4c]: CMSampleBufferCreate: status=%d (0x%x), sample_buffer=%p\n",
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
        fprintf(stderr, "[VT_SHIM TRACE 4d]: VTDecompressionSessionDecodeFrame: status=%d (0x%x), info_flags_out=0x%x\n",
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
    fprintf(stderr, "[VT_SHIM TRACE 6a]: renderd_CVPixelBufferCopyBGRA: image_buffer=%p, dest_capacity=%zu\n",
            (void*)image_buffer, dest_capacity);

    if (image_buffer == NULL || out_dest == NULL || out_width == NULL || out_height == NULL) {
        fprintf(stderr, "[VT_SHIM TRACE 6a-ERR]: Parameter error in renderd_CVPixelBufferCopyBGRA!\n");
        return kVTParameterErr;
    }

    CVPixelBufferRef pixel_buffer = (CVPixelBufferRef)image_buffer;
    OSStatus status = CVPixelBufferLockBaseAddress(pixel_buffer, kCVPixelBufferLock_ReadOnly);
    if (status != kCVReturnSuccess) {
        fprintf(stderr, "[VT_SHIM TRACE 6b-ERR]: CVPixelBufferLockBaseAddress status=%d (0x%x)\n",
                (int)status, (unsigned int)status);
        return status;
    }

    int32_t width = (int32_t)CVPixelBufferGetWidth(pixel_buffer);
    int32_t height = (int32_t)CVPixelBufferGetHeight(pixel_buffer);
    size_t bytes_per_row = CVPixelBufferGetBytesPerRow(pixel_buffer);
    uint8_t *base_addr = (uint8_t *)CVPixelBufferGetBaseAddress(pixel_buffer);

    fprintf(stderr, "[VT_SHIM TRACE 6b]: CVPixelBuffer locked base address: width=%d, height=%d, bytes_per_row=%zu, base_addr=%p\n",
            width, height, bytes_per_row, (void*)base_addr);

    *out_width = width;
    *out_height = height;

    size_t expected_size = (size_t)width * (size_t)height * 4;
    if (dest_capacity < expected_size || base_addr == NULL) {
        CVPixelBufferUnlockBaseAddress(pixel_buffer, kCVPixelBufferLock_ReadOnly);
        fprintf(stderr, "[VT_SHIM TRACE 6c-ERR]: Allocation error or NULL base address!\n");
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
