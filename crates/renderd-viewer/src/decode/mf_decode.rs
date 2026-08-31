//! Media Foundation video decoder (`renderd-viewer/src/decode/mf_decode.rs`).
//!
//! Decodes H.264 and HEVC Annex-B packets into NV12 buffers using a Media Foundation
//! decoder MFT.
//!
//! # Why this exists alongside the D3D12 decoder
//!
//! `ID3D12VideoDecoder` is a slice-level API: it needs DXVA picture parameters, slice
//! control and an explicit reference-frame list supplied per frame, which means parsing
//! the HEVC bitstream first. Media Foundation's decoder MFTs do that parsing themselves
//! and accept the Annex-B stream directly, so this path trades some copy overhead for
//! actually being able to decode what the host sends.
//!
//! # Choices that favour working over fast
//!
//! - **Software MFTs only** (`MFT_ENUM_FLAG_SYNCMFT`). Hardware MFTs are asynchronous
//!   and require the full event-driven model; the synchronous software decoders are
//!   present on every install and are comfortably fast enough for 1080p.
//! - **H.264 is preferred by the viewer.** The Microsoft H.264 decoder ships with every
//!   Windows 10 and later install. HEVC needs the HEVC Video Extensions from the Store,
//!   so it is offered second and simply fails to initialize when absent.

#[cfg(target_os = "windows")]
use crate::decoder::PixelFormat;
use crate::decoder::{DecodedFrame, Decoder};
use crate::error::ViewerError;
use std::collections::VecDeque;
use std::time::Instant;

#[cfg(target_os = "windows")]
use windows::{
    core::Interface,
    Win32::Media::MediaFoundation::{
        IMFMediaType, IMFSample, IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer,
        MFCreateSample, MFMediaType_Video, MFStartup, MFTEnumEx, MFVideoFormat_H264,
        MFVideoFormat_HEVC, MFVideoFormat_NV12, MFSTARTUP_LITE, MFT_CATEGORY_VIDEO_DECODER,
        MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
        MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
        MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_E_TRANSFORM_NEED_MORE_INPUT,
        MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
        MF_MT_SUBTYPE, MF_VERSION,
    },
};

/// Number of leading frames whose pipeline diagnostics are logged at INFO.
#[cfg(target_os = "windows")]
const DIAGNOSTIC_FRAMES: u64 = 8;

/// Media Foundation ticks per second (100 ns units).
#[cfg(all(target_os = "windows", test))]
const MF_TICKS_PER_SECOND: i64 = 10_000_000;

/// Media Foundation software video decoder producing NV12 frames.
#[derive(Debug)]
pub struct MediaFoundationDecoder {
    initialized: bool,
    codec: String,
    width: u32,
    height: u32,
    /// Frame dimensions reported by the negotiated output type, which may be padded
    /// up to the codec's macroblock alignment relative to the requested size.
    out_width: u32,
    out_height: u32,
    output_queue: VecDeque<DecodedFrame>,
    decoded_count: u64,

    #[cfg(target_os = "windows")]
    transform: Option<IMFTransform>,
    #[cfg(target_os = "windows")]
    output_provides_samples: bool,
    #[cfg(target_os = "windows")]
    output_buffer_size: u32,
}

// SAFETY: The `Decoder` trait requires `Send + Sync`, but windows-rs does not mark the
// Media Foundation interfaces as either. Two things make this sound here:
//
//  * Ownership is exclusive. The decoder lives in one task, every trait method takes
//    `&mut self`, and no handle to the transform escapes this module — so the `Sync`
//    half is never exercised by concurrent calls.
//  * `init_inner` puts the thread into the multithreaded apartment before creating the
//    transform, and the stock Microsoft decoder MFTs register as ThreadingModel="Both",
//    so they aggregate the free-threaded marshaler and may be called from any MTA
//    thread. That is what makes the `Send` half safe when tokio migrates the owning
//    task between worker threads.
#[cfg(target_os = "windows")]
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for MediaFoundationDecoder {}
#[cfg(target_os = "windows")]
unsafe impl Sync for MediaFoundationDecoder {}

impl Default for MediaFoundationDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaFoundationDecoder {
    /// Creates a new decoder with no Media Foundation objects allocated yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            codec: String::new(),
            width: 0,
            height: 0,
            out_width: 0,
            out_height: 0,
            output_queue: VecDeque::new(),
            decoded_count: 0,
            #[cfg(target_os = "windows")]
            transform: None,
            #[cfg(target_os = "windows")]
            output_provides_samples: false,
            #[cfg(target_os = "windows")]
            output_buffer_size: 0,
        }
    }

    /// Target video codec string (`"h264"` or `"hevc"`).
    #[must_use]
    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Total number of frames decoded so far.
    #[must_use]
    pub const fn decoded_count(&self) -> u64 {
        self.decoded_count
    }

    /// Frame dimensions the negotiated output type actually produces.
    #[must_use]
    pub const fn output_dimensions(&self) -> (u32, u32) {
        (self.out_width, self.out_height)
    }
}

impl Decoder for MediaFoundationDecoder {
    fn initialize(&mut self, codec: &str, width: u32, height: u32) -> Result<(), ViewerError> {
        self.codec = codec.to_lowercase();
        self.width = width;
        self.height = height;
        self.out_width = width;
        self.out_height = height;
        self.output_queue.clear();
        self.decoded_count = 0;

        if width == 0 || height == 0 {
            return Err(ViewerError::Decoder(format!(
                "invalid decode dimensions {width}x{height}"
            )));
        }

        #[cfg(target_os = "windows")]
        {
            self.init_media_foundation()?;
            tracing::info!(
                codec = %self.codec,
                width,
                height,
                out_width = self.out_width,
                out_height = self.out_height,
                "MediaFoundationDecoder initialized"
            );
            self.initialized = true;
            Ok(())
        }

        #[cfg(not(target_os = "windows"))]
        {
            Err(ViewerError::Decoder(
                "MediaFoundationDecoder is only available on Windows".to_string(),
            ))
        }
    }

    fn decode_packet(
        &mut self,
        packet: &[u8],
        frame_id: u64,
        pts_ns: u64,
    ) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }
        if packet.is_empty() {
            return Ok(());
        }

        let start_time = Instant::now();

        #[cfg(target_os = "windows")]
        {
            self.submit_and_drain(packet, frame_id, pts_ns, start_time)
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (packet, frame_id, pts_ns, start_time);
            Err(ViewerError::Decoder(
                "MediaFoundationDecoder is only available on Windows".to_string(),
            ))
        }
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }
        Ok(self.output_queue.pop_front())
    }

    fn reset(&mut self) -> Result<(), ViewerError> {
        self.output_queue.clear();
        self.decoded_count = 0;
        #[cfg(target_os = "windows")]
        {
            // Drop the transform so the next initialize builds one from the new
            // stream's parameter sets rather than reusing the old stream's.
            self.transform = None;
        }
        self.initialized = false;
        Ok(())
    }
}

// ── Windows implementation ────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
impl MediaFoundationDecoder {
    /// Media Foundation subtype GUID for the configured codec.
    fn input_subtype(&self) -> Result<windows::core::GUID, ViewerError> {
        match self.codec.as_str() {
            "h264" | "avc" | "avc1" => Ok(MFVideoFormat_H264),
            "hevc" | "h265" | "hvc1" => Ok(MFVideoFormat_HEVC),
            other => Err(ViewerError::Decoder(format!(
                "unsupported codec '{other}'; expected h264 or hevc"
            ))),
        }
    }

    fn init_media_foundation(&mut self) -> Result<(), ViewerError> {
        unsafe { self.init_inner() }
    }

    unsafe fn init_inner(&mut self) -> Result<(), ViewerError> {
        let subtype = self.input_subtype()?;

        // Join the multithreaded apartment before creating any MF object, so the
        // transform can legally be called from whichever tokio worker thread the owning
        // task happens to be running on. RPC_E_CHANGED_MODE means this thread is already
        // in an apartment, which is fine; anything else is worth surfacing.
        let com = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
        if com.is_err() && com != windows::Win32::Foundation::RPC_E_CHANGED_MODE {
            tracing::warn!(hr = ?com, "CoInitializeEx(MTA) returned an unexpected status");
        }

        // MFSTARTUP_LITE skips the full platform (no sockets/network sources), which is
        // all a decoder MFT needs. Calling it more than once per process is harmless.
        MFStartup(MF_VERSION, MFSTARTUP_LITE)
            .map_err(|e| ViewerError::Decoder(format!("MFStartup failed: {e}")))?;

        let transform = self.enumerate_decoder(subtype)?;

        // Ask for low latency where the MFT honours it; failure is not fatal.
        if let Ok(attributes) = transform.GetAttributes() {
            let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
        }

        self.configure_input(&transform, subtype)?;
        let (out_width, out_height) = self.configure_output(&transform)?;

        let stream_info = transform
            .GetOutputStreamInfo(0)
            .map_err(|e| ViewerError::Decoder(format!("GetOutputStreamInfo failed: {e}")))?;
        self.output_provides_samples =
            (stream_info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
        // Some MFTs report 0 before the first stream change; fall back to a full NV12 frame.
        let nv12_size = out_width.saturating_mul(out_height) * 3 / 2;
        self.output_buffer_size = stream_info.cbSize.max(nv12_size);

        transform
            .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
            .map_err(|e| ViewerError::Decoder(format!("NOTIFY_BEGIN_STREAMING failed: {e}")))?;
        transform
            .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
            .map_err(|e| ViewerError::Decoder(format!("NOTIFY_START_OF_STREAM failed: {e}")))?;

        tracing::info!(
            provides_samples = self.output_provides_samples,
            output_buffer_size = self.output_buffer_size,
            "Media Foundation decoder MFT ready"
        );

        self.out_width = out_width;
        self.out_height = out_height;
        self.transform = Some(transform);
        Ok(())
    }

    /// Finds a synchronous software decoder MFT for `subtype` producing NV12.
    unsafe fn enumerate_decoder(
        &self,
        subtype: windows::core::GUID,
    ) -> Result<IMFTransform, ViewerError> {
        let input_info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: subtype,
        };
        let output_info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };

        let mut activates = std::ptr::null_mut();
        let mut count: u32 = 0;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input_info),
            Some(&output_info),
            &mut activates,
            &mut count,
        )
        .map_err(|e| ViewerError::Decoder(format!("MFTEnumEx failed: {e}")))?;

        if count == 0 || activates.is_null() {
            return Err(ViewerError::Decoder(format!(
                "no Media Foundation decoder registered for {}. \
                 HEVC needs the 'HEVC Video Extensions' from the Microsoft Store; \
                 H.264 is present on every Windows 10 and later install.",
                self.codec
            )));
        }

        // Take the first (highest-ranked) activate, then release every entry and the
        // array itself — MFTEnumEx hands ownership of both to the caller.
        let slice = std::slice::from_raw_parts(activates, count as usize);
        let chosen = slice[0].clone();
        for entry in slice {
            drop(entry.clone());
        }
        windows::Win32::System::Com::CoTaskMemFree(Some(activates.cast()));

        let activate = chosen.ok_or_else(|| {
            ViewerError::Decoder("MFTEnumEx returned a null activation object".to_string())
        })?;

        activate
            .ActivateObject::<IMFTransform>()
            .map_err(|e| ViewerError::Decoder(format!("ActivateObject(IMFTransform) failed: {e}")))
    }

    /// Declares the compressed input format on stream 0.
    unsafe fn configure_input(
        &self,
        transform: &IMFTransform,
        subtype: windows::core::GUID,
    ) -> Result<(), ViewerError> {
        let media_type: IMFMediaType = MFCreateMediaType()
            .map_err(|e| ViewerError::Decoder(format!("MFCreateMediaType (input) failed: {e}")))?;

        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| ViewerError::Decoder(format!("set input major type failed: {e}")))?;
        media_type
            .SetGUID(&MF_MT_SUBTYPE, &subtype)
            .map_err(|e| ViewerError::Decoder(format!("set input subtype failed: {e}")))?;
        media_type
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_size(self.width, self.height))
            .map_err(|e| ViewerError::Decoder(format!("set input frame size failed: {e}")))?;

        transform
            .SetInputType(0, &media_type, 0)
            .map_err(|e| ViewerError::Decoder(format!("SetInputType failed: {e}")))
    }

    /// Picks the first NV12 output type the MFT offers and returns its frame size.
    unsafe fn configure_output(&self, transform: &IMFTransform) -> Result<(u32, u32), ViewerError> {
        for index in 0..32u32 {
            let Ok(candidate) = transform.GetOutputAvailableType(0, index) else {
                break;
            };
            let Ok(sub) = candidate.GetGUID(&MF_MT_SUBTYPE) else {
                continue;
            };
            if sub != MFVideoFormat_NV12 {
                continue;
            }

            // Restate the frame size; some decoders leave it unset until configured.
            let _ = candidate.SetUINT64(&MF_MT_FRAME_SIZE, pack_size(self.width, self.height));
            transform
                .SetOutputType(0, &candidate, 0)
                .map_err(|e| ViewerError::Decoder(format!("SetOutputType(NV12) failed: {e}")))?;

            let (width, height) = candidate
                .GetUINT64(&MF_MT_FRAME_SIZE)
                .map_or((self.width, self.height), unpack_size);
            return Ok((width, height));
        }

        Err(ViewerError::Decoder(
            "decoder MFT offered no NV12 output type".to_string(),
        ))
    }

    /// Re-reads the output type after the MFT reports a format change mid-stream.
    unsafe fn renegotiate_output(&mut self) -> Result<(), ViewerError> {
        let transform = self
            .transform
            .clone()
            .ok_or_else(|| ViewerError::Decoder("transform not initialized".to_string()))?;
        let (width, height) = self.configure_output(&transform)?;
        let nv12_size = width.saturating_mul(height) * 3 / 2;
        if let Ok(info) = transform.GetOutputStreamInfo(0) {
            self.output_provides_samples =
                (info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
            self.output_buffer_size = info.cbSize.max(nv12_size);
        } else {
            self.output_buffer_size = nv12_size;
        }
        tracing::info!(width, height, "Decoder output format changed");
        self.out_width = width;
        self.out_height = height;
        Ok(())
    }

    fn submit_and_drain(
        &mut self,
        packet: &[u8],
        frame_id: u64,
        pts_ns: u64,
        start_time: Instant,
    ) -> Result<(), ViewerError> {
        unsafe {
            self.submit_input(packet, pts_ns)?;
            self.drain_output(frame_id, pts_ns, start_time)
        }
    }

    /// Wraps `packet` in an `IMFSample` and feeds it to the transform.
    unsafe fn submit_input(&mut self, packet: &[u8], pts_ns: u64) -> Result<(), ViewerError> {
        let transform = self
            .transform
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("transform not initialized".to_string()))?;

        let len = u32::try_from(packet.len()).map_err(|_| {
            ViewerError::Decoder(format!("packet of {} bytes exceeds u32", packet.len()))
        })?;

        let buffer = MFCreateMemoryBuffer(len)
            .map_err(|e| ViewerError::Decoder(format!("MFCreateMemoryBuffer failed: {e}")))?;

        let mut dest: *mut u8 = std::ptr::null_mut();
        let mut max_len: u32 = 0;
        buffer
            .Lock(&mut dest, Some(&mut max_len), None)
            .map_err(|e| ViewerError::Decoder(format!("Lock input buffer failed: {e}")))?;
        if dest.is_null() || max_len < len {
            let _ = buffer.Unlock();
            return Err(ViewerError::Decoder(
                "input buffer lock returned an unusable pointer".to_string(),
            ));
        }
        std::ptr::copy_nonoverlapping(packet.as_ptr(), dest, packet.len());
        buffer
            .Unlock()
            .map_err(|e| ViewerError::Decoder(format!("Unlock input buffer failed: {e}")))?;
        buffer
            .SetCurrentLength(len)
            .map_err(|e| ViewerError::Decoder(format!("SetCurrentLength failed: {e}")))?;

        let sample =
            MFCreateSample().map_err(|e| ViewerError::Decoder(format!("MFCreateSample: {e}")))?;
        sample
            .AddBuffer(&buffer)
            .map_err(|e| ViewerError::Decoder(format!("AddBuffer failed: {e}")))?;
        let _ = sample.SetSampleTime(ns_to_mf_ticks(pts_ns));

        transform.ProcessInput(0, &sample, 0).map_err(|e| {
            ViewerError::Decoder(format!("ProcessInput failed: {e} (hr {:#x})", e.code().0))
        })
    }

    /// Pulls every frame the transform can currently produce into the output queue.
    unsafe fn drain_output(
        &mut self,
        frame_id: u64,
        pts_ns: u64,
        start_time: Instant,
    ) -> Result<(), ViewerError> {
        loop {
            let transform = self
                .transform
                .clone()
                .ok_or_else(|| ViewerError::Decoder("transform not initialized".to_string()))?;

            // When the MFT does not allocate, we must supply a sample large enough for
            // the negotiated output type.
            let supplied = if self.output_provides_samples {
                None
            } else {
                let buffer = MFCreateMemoryBuffer(self.output_buffer_size).map_err(|e| {
                    ViewerError::Decoder(format!("MFCreateMemoryBuffer (output): {e}"))
                })?;
                let sample = MFCreateSample()
                    .map_err(|e| ViewerError::Decoder(format!("MFCreateSample (output): {e}")))?;
                sample
                    .AddBuffer(&buffer)
                    .map_err(|e| ViewerError::Decoder(format!("AddBuffer (output): {e}")))?;
                Some(sample)
            };

            let mut data = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(supplied),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            }];
            let mut status: u32 = 0;

            let result = transform.ProcessOutput(0, &mut data, &mut status);

            // Reclaim whatever the call left in the struct: our own supplied sample, or
            // one the MFT allocated. Either way the reference is ours to release.
            let produced = std::mem::ManuallyDrop::take(&mut data[0].pSample);
            let _ = std::mem::ManuallyDrop::take(&mut data[0].pEvents);

            match result {
                Ok(()) => {
                    if let Some(sample) = produced {
                        let frame = self.sample_to_frame(&sample, frame_id, pts_ns, start_time)?;
                        // Bound the queue: the render loop drains it, and holding more
                        // than a few frames only adds latency.
                        while self.output_queue.len() >= 4 {
                            self.output_queue.pop_front();
                        }
                        self.output_queue.push_back(frame);
                        self.decoded_count += 1;
                    }
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.renegotiate_output()?;
                }
                Err(e) => {
                    return Err(ViewerError::Decoder(format!(
                        "ProcessOutput failed: {e} (hr {:#x})",
                        e.code().0
                    )));
                }
            }
        }
    }

    /// Copies an NV12 `IMFSample` into a tightly packed [`DecodedFrame`].
    unsafe fn sample_to_frame(
        &self,
        sample: &IMFSample,
        frame_id: u64,
        pts_ns: u64,
        start_time: Instant,
    ) -> Result<DecodedFrame, ViewerError> {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| ViewerError::Decoder(format!("ConvertToContiguousBuffer: {e}")))?;

        // Decoders align the coded size up to a macroblock multiple — 1080 becomes 1088
        // for H.264 — so emit the negotiated size and leave the padding rows in the
        // source buffer. Reading fewer rows than the decoder wrote is always safe.
        let width = (self.width.min(self.out_width)) as usize;
        let height = (self.height.min(self.out_height)) as usize;
        let mut nv12 = Vec::<u8>::with_capacity(width * height * 3 / 2);

        // IMF2DBuffer exposes the real stride; without it the rows are packed at width.
        if let Ok(two_d) = buffer.cast::<windows::Win32::Media::MediaFoundation::IMF2DBuffer>() {
            let mut scanline0: *mut u8 = std::ptr::null_mut();
            let mut pitch: i32 = 0;
            two_d
                .Lock2D(&mut scanline0, &mut pitch)
                .map_err(|e| ViewerError::Decoder(format!("Lock2D failed: {e}")))?;

            let Ok(pitch) = usize::try_from(pitch) else {
                let _ = two_d.Unlock2D();
                return Err(ViewerError::Decoder(format!(
                    "NV12 sample reported a negative stride ({pitch})"
                )));
            };
            let copied = copy_nv12_planes(
                scanline0,
                pitch,
                width,
                height,
                self.out_height as usize,
                &mut nv12,
            );
            let _ = two_d.Unlock2D();

            if !copied {
                return Err(ViewerError::Decoder(
                    "NV12 sample had an unusable stride or null scanline".to_string(),
                ));
            }
        } else {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut current_len: u32 = 0;
            buffer
                .Lock(&mut data, None, Some(&mut current_len))
                .map_err(|e| ViewerError::Decoder(format!("Lock output buffer failed: {e}")))?;

            // Without IMF2DBuffer the rows are packed at the coded width, so that is
            // the stride to walk even though we only keep `width` bytes of each row.
            let coded_width = self.out_width as usize;
            let coded_height = self.out_height as usize;
            let needed = coded_width * coded_height * 3 / 2;
            if data.is_null() || (current_len as usize) < needed {
                let _ = buffer.Unlock();
                return Err(ViewerError::Decoder(format!(
                    "NV12 sample is {current_len} bytes, need {needed} for coded \
                     {coded_width}x{coded_height}"
                )));
            }
            let copied =
                copy_nv12_planes(data, coded_width, width, height, coded_height, &mut nv12);
            let _ = buffer.Unlock();
            if !copied {
                return Err(ViewerError::Decoder(
                    "NV12 contiguous buffer had an unusable layout".to_string(),
                ));
            }
        }

        if self.decoded_count < DIAGNOSTIC_FRAMES {
            tracing::info!(
                frame_id,
                out_width = self.out_width,
                out_height = self.out_height,
                nv12_bytes = nv12.len(),
                "MediaFoundationDecoder: produced NV12 frame"
            );
        }

        Ok(DecodedFrame {
            frame_id,
            pts_ns,
            width: self.out_width,
            height: self.out_height,
            format: PixelFormat::Nv12,
            buffer: nv12,
            decode_duration: start_time.elapsed(),
        })
    }
}

/// Packs a width and height into the `MF_MT_FRAME_SIZE` attribute layout.
#[cfg(target_os = "windows")]
const fn pack_size(width: u32, height: u32) -> u64 {
    ((width as u64) << 32) | (height as u64)
}

/// Unpacks the `MF_MT_FRAME_SIZE` attribute layout into width and height.
#[cfg(target_os = "windows")]
const fn unpack_size(packed: u64) -> (u32, u32) {
    #[allow(clippy::cast_possible_truncation)]
    ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32)
}

/// Converts nanoseconds to Media Foundation's 100 ns tick units, saturating.
#[cfg(target_os = "windows")]
#[allow(clippy::cast_possible_wrap)]
const fn ns_to_mf_ticks(pts_ns: u64) -> i64 {
    let ticks = pts_ns / 100;
    if ticks > i64::MAX as u64 {
        i64::MAX
    } else {
        ticks as i64
    }
}

/// Copies an NV12 image out of a strided decoder buffer into tightly packed rows.
///
/// `pitch` is the source stride in bytes, `width`/`height` the visible region to keep,
/// and `coded_height` the full luma height the decoder wrote — the chroma plane starts
/// after that many luma rows, not after the visible ones.
///
/// Returns `false` without producing a usable image if the pointer is null or the
/// stride is too small to hold a row.
///
/// # Safety
/// `scanline0` must point to at least `pitch * coded_height * 3 / 2` readable bytes.
#[cfg(target_os = "windows")]
unsafe fn copy_nv12_planes(
    scanline0: *const u8,
    pitch: usize,
    width: usize,
    height: usize,
    coded_height: usize,
    out: &mut Vec<u8>,
) -> bool {
    if scanline0.is_null() || pitch < width || coded_height < height {
        return false;
    }

    for row in 0..height {
        let src = scanline0.add(row * pitch);
        out.extend_from_slice(std::slice::from_raw_parts(src, width));
    }
    // The chroma plane follows the full coded luma plane, at half its height.
    let chroma_base = scanline0.add(coded_height * pitch);
    for row in 0..height / 2 {
        let src = chroma_base.add(row * pitch);
        out.extend_from_slice(std::slice::from_raw_parts(src, width));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_decoder_is_empty() {
        let decoder = MediaFoundationDecoder::new();
        assert!(decoder.codec().is_empty());
        assert_eq!(decoder.decoded_count(), 0);
        assert_eq!(decoder.output_dimensions(), (0, 0));
    }

    #[test]
    fn test_zero_dimensions_rejected() {
        let mut decoder = MediaFoundationDecoder::new();
        let err = decoder.initialize("h264", 0, 0).unwrap_err();
        assert!(format!("{err}").contains("0x0"), "got: {err}");
    }

    #[test]
    fn test_decode_before_initialize_is_error() {
        let mut decoder = MediaFoundationDecoder::new();
        assert!(decoder.decode_packet(&[0u8; 8], 1, 0).is_err());
    }

    #[test]
    fn test_reset_clears_state() {
        let mut decoder = MediaFoundationDecoder::new();
        assert!(decoder.reset().is_ok());
        assert_eq!(decoder.decoded_count(), 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_frame_size_packing_roundtrips() {
        for (w, h) in [(1920, 1080), (3840, 2160), (1, 1), (u32::MAX, u32::MAX)] {
            assert_eq!(unpack_size(pack_size(w, h)), (w, h));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_ns_to_mf_ticks() {
        assert_eq!(ns_to_mf_ticks(0), 0);
        assert_eq!(ns_to_mf_ticks(1_000_000_000), MF_TICKS_PER_SECOND);
        assert_eq!(ns_to_mf_ticks(u64::MAX), i64::MAX);
    }
}
