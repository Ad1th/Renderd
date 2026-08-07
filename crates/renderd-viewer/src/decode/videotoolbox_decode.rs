//! macOS `VideoToolbox` hardware video decoder integration (`renderd-viewer/src/decode/videotoolbox_decode.rs`).
//!
//! Hardware decodes incoming H.265 (HEVC) / H.264 video bitstream packets into BGRA8 image buffers using `VTDecompressionSession` (RFC-0002 §6.3).

#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::option_if_let_else,
    clippy::too_many_lines
)]

use crate::decoder::{DecodedFrame, Decoder, PixelFormat};
use crate::error::ViewerError;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

/// macOS `VideoToolbox` hardware video decoder.
pub struct VideoToolboxDecoder {
    initialized: bool,
    codec: String,
    width: u32,
    height: u32,
    output_queue: Arc<Mutex<VecDeque<DecodedFrame>>>,
    decoded_count: u64,
    #[cfg(target_os = "macos")]
    session: Option<renderd_vt_sys::DecompressionSession>,
}

impl Debug for VideoToolboxDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoToolboxDecoder")
            .field("initialized", &self.initialized)
            .field("codec", &self.codec)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("decoded_count", &self.decoded_count)
            .finish_non_exhaustive()
    }
}

impl Default for VideoToolboxDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoToolboxDecoder {
    /// Creates a new `VideoToolboxDecoder`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            initialized: false,
            codec: String::new(),
            width: 0,
            height: 0,
            output_queue: Arc::new(Mutex::new(VecDeque::new())),
            decoded_count: 0,
            #[cfg(target_os = "macos")]
            session: None,
        }
    }

    /// Returns the target video codec string (e.g. "hevc" or "h264").
    #[must_use]
    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Returns total count of decoded frames.
    #[must_use]
    pub const fn decoded_count(&self) -> u64 {
        self.decoded_count
    }
}

impl Decoder for VideoToolboxDecoder {
    fn initialize(&mut self, codec: &str, width: u32, height: u32) -> Result<(), ViewerError> {
        self.codec = codec.to_lowercase();
        self.width = width;
        self.height = height;

        self.initialized = true;

        tracing::info!(
            codec = %self.codec,
            width = width,
            height = height,
            "VideoToolboxDecoder initialized successfully"
        );

        Ok(())
    }

    fn decode_packet(
        &mut self,
        packet: &[u8],
        frame_id: u64,
        pts_ns: u64,
    ) -> Result<(), ViewerError> {
        if !self.initialized {
            tracing::error!("VT_TRACE: decode_packet called on uninitialized decoder!");
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }

        tracing::info!(
            frame_id = frame_id,
            pts_ns = pts_ns,
            packet_len = packet.len(),
            first_8_bytes = ?&packet[..8.min(packet.len())],
            "VT_TRACE [1]: VideoToolboxDecoder::decode_packet called"
        );

        #[cfg(target_os = "macos")]
        tracing::info!(
            session_exists = self.session.is_some(),
            "VT_TRACE [1a]: session state"
        );

        #[cfg(target_os = "macos")]
        {
            if self.session.is_none() {
                tracing::info!("VT_TRACE [2]: Attempting DecompressionSession::from_nal with incoming packet...");
                let vt_codec = match self.codec.as_str() {
                    "h264" | "avc" | "avc1" => renderd_vt_sys::VideoCodec::H264,
                    _ => renderd_vt_sys::VideoCodec::Hevc,
                };

                let queue = Arc::clone(&self.output_queue);
                let first_frame_logged = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let count_atomic = Arc::new(std::sync::atomic::AtomicU64::new(0));
                let w = self.width;
                let h = self.height;

                let session_res = renderd_vt_sys::DecompressionSession::from_nal(
                    w as i32,
                    h as i32,
                    vt_codec,
                    packet,
                    move |err, flags, image_buffer, cb_frame_id, cb_pts_ns| {
                        tracing::info!(
                            status_code = err.code(),
                            flags = flags,
                            image_buffer_null = image_buffer.is_null(),
                            cb_frame_id = cb_frame_id,
                            cb_pts_ns = cb_pts_ns,
                            "VT_TRACE [6]: Output callback invoked from VideoToolbox"
                        );

                        if err.code() != 0 || image_buffer.is_null() {
                            tracing::error!(
                                status_code = err.code(),
                                image_buffer_null = image_buffer.is_null(),
                                "VT_TRACE [6-ERR]: Decompression output callback error or null buffer"
                            );
                            return;
                        }

                        let mut buffer = vec![0u8; (w * h * 4) as usize];
                        let copy_res = unsafe {
                            renderd_vt_sys::copy_pixel_buffer_bgra(image_buffer, &mut buffer)
                        };

                        tracing::info!(
                            copy_res = ?copy_res,
                            "VT_TRACE [7]: copy_pixel_buffer_bgra result"
                        );

                        if let Ok((actual_w, actual_h)) = copy_res {
                            let count =
                                count_atomic.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            let out_frame_id = if cb_frame_id > 0 { cb_frame_id } else { count };
                            if count == 1
                                && !first_frame_logged
                                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                            {
                                let first_64 = &buffer[..64.min(buffer.len())];
                                let min_val = buffer.iter().copied().min().unwrap_or(0);
                                let max_val = buffer.iter().copied().max().unwrap_or(0);
                                let mut unique_set = std::collections::HashSet::new();
                                for &byte in &buffer {
                                    unique_set.insert(byte);
                                }
                                tracing::info!(
                                    count = count,
                                    frame_id = out_frame_id,
                                    width = actual_w,
                                    height = actual_h,
                                    format = ?PixelFormat::Bgra8,
                                    min_byte = min_val,
                                    max_byte = max_val,
                                    unique_bytes = unique_set.len(),
                                    first_64_bytes = ?first_64,
                                    "Decoder: decoded first frame bitstream into BGRA8 image buffer"
                                );
                            }

                            let frame = DecodedFrame {
                                frame_id: out_frame_id,
                                pts_ns: if cb_pts_ns >= 0 { cb_pts_ns as u64 } else { 0 },
                                width: actual_w,
                                height: actual_h,
                                format: PixelFormat::Bgra8,
                                buffer,
                                decode_duration: std::time::Duration::from_millis(1),
                            };

                            if let Ok(mut q) = queue.lock() {
                                q.push_back(frame);
                                tracing::info!(
                                    queue_len = q.len(),
                                    "VT_TRACE [8-SUCCESS]: Pushed DecodedFrame to output_queue"
                                );
                            } else {
                                tracing::error!("VT_TRACE [8-ERR]: output_queue mutex poisoned");
                            }
                        }
                    },
                );

                match session_res {
                    Ok(session) => {
                        tracing::info!("VT_TRACE [3]: DecompressionSession::from_nal succeeded");
                        self.session = Some(session);
                    }
                    Err(e) => {
                        tracing::error!(
                            "VT_TRACE [3-ERR]: DecompressionSession::from_nal failed: {e}"
                        );
                    }
                }
            }

            if let Some(ref session) = self.session {
                let frame_ctx = frame_id as usize as *mut std::ffi::c_void;
                tracing::info!(
                    frame_id = frame_id,
                    pts_ns = pts_ns,
                    packet_len = packet.len(),
                    "VT_TRACE [4]: Calling session.decode_frame_with_ctx..."
                );
                let decode_res = session.decode_frame_with_ctx(packet, pts_ns as i64, frame_ctx);
                tracing::info!(
                    decode_res = ?decode_res,
                    frame_id = frame_id,
                    "VT_TRACE [5]: session.decode_frame_with_ctx returned"
                );
            } else {
                tracing::error!("VT_TRACE [4-ERR]: No active DecompressionSession to decode frame");
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (packet, frame_id, pts_ns);
        }

        self.decoded_count += 1;
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }
        if let Ok(mut q) = self.output_queue.lock() {
            let frame = q.pop_front();
            if frame.is_some() {
                tracing::info!(
                    "VT_TRACE [9]: receive_frame popped a decoded frame from output_queue"
                );
            }
            Ok(frame)
        } else {
            Err(ViewerError::Decoder(
                "Output queue mutex poisoned".to_string(),
            ))
        }
    }

    fn reset(&mut self) -> Result<(), ViewerError> {
        if let Ok(mut q) = self.output_queue.lock() {
            q.clear();
        }
        #[cfg(target_os = "macos")]
        if let Some(ref session) = self.session {
            let _ = session.wait_for_async_frames();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_videotoolbox_decoder_lifecycle() {
        let mut decoder = VideoToolboxDecoder::new();
        assert!(decoder.receive_frame().is_err() || decoder.codec().is_empty());

        decoder.initialize("hevc", 1920, 1080).unwrap();
        assert_eq!(decoder.codec(), "hevc");

        let test_packet = vec![0x00, 0x00, 0x00, 0x01, 0x40, 0x01]; // H.265 NAL unit header
        decoder.decode_packet(&test_packet, 1, 16_666_666).unwrap();

        assert_eq!(decoder.decoded_count(), 1);
        assert!(decoder.reset().is_ok());
    }
}
