//! Direct3D 12 video decoder integration (`renderd-viewer/src/decode/d3d12_decode.rs`).
//!
//! Hardware decodes incoming H.265 (HEVC) bitstream packets into NV12 CPU buffers using
//! `ID3D12VideoDecoder` (from `windows::Win32::Media::MediaFoundation`) (RFC-0002 §6.3).
//!
//! Threading model: The decoder owns all D3D12/media COM objects. COM video interfaces defined
//! in `windows 0.58` implement `Send + Sync`. `HANDLE` is a raw-pointer wrapper without those
//! impls, so it is never stored as a persistent struct field. Event handles are created, used,
//! and closed within individual calls on the current thread.

#[cfg(target_os = "windows")]
use crate::decoder::PixelFormat;
use crate::decoder::{DecodedFrame, Decoder};
use crate::error::ViewerError;
use std::collections::VecDeque;
use std::time::Instant;

// ── Windows-only imports ──────────────────────────────────────────────────────
#[cfg(target_os = "windows")]
use windows::{
    Win32::Foundation::{CloseHandle, FALSE},
    Win32::Graphics::Direct3D12::{
        D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandQueue, ID3D12Device, ID3D12Fence,
        ID3D12GraphicsCommandList, ID3D12Resource, D3D12_COMMAND_LIST_TYPE_VIDEO_DECODE,
        D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
        D3D12_COMMAND_QUEUE_PRIORITY_NORMAL, D3D12_FENCE_FLAG_NONE, D3D12_HEAP_FLAG_NONE,
        D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_HEAP_TYPE_READBACK,
        D3D12_HEAP_TYPE_UPLOAD, D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_RANGE,
        D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_FLAG_NONE,
        D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
        D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS, D3D12_RESOURCE_FLAG_NONE,
        D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST,
        D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_GENERIC_READ,
        D3D12_RESOURCE_STATE_VIDEO_DECODE_WRITE, D3D12_RESOURCE_TRANSITION_BARRIER,
        D3D12_TEXTURE_COPY_LOCATION, D3D12_TEXTURE_COPY_LOCATION_0,
        D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT, D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
    },
    Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC},
    Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, DXGI_ERROR_NOT_FOUND,
    },
    Win32::Media::MediaFoundation::{
        ID3D12VideoDecodeCommandList, ID3D12VideoDecoder, ID3D12VideoDecoderHeap,
        ID3D12VideoDevice, D3D12_BITSTREAM_ENCRYPTION_TYPE_NONE,
        D3D12_FEATURE_VIDEO_DECODE_SUPPORT, D3D12_VIDEO_DECODER_DESC,
        D3D12_VIDEO_DECODER_HEAP_DESC, D3D12_VIDEO_DECODE_COMPRESSED_BITSTREAM,
        D3D12_VIDEO_DECODE_CONFIGURATION, D3D12_VIDEO_DECODE_CONVERSION_ARGUMENTS,
        D3D12_VIDEO_DECODE_INPUT_STREAM_ARGUMENTS, D3D12_VIDEO_DECODE_OUTPUT_STREAM_ARGUMENTS,
        D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN, D3D12_VIDEO_DECODE_REFERENCE_FRAMES,
        D3D12_VIDEO_DECODE_SUPPORT_DATA, D3D12_VIDEO_DECODE_SUPPORT_FLAG_SUPPORTED,
        D3D12_VIDEO_FRAME_CODED_INTERLACE_TYPE_NONE,
    },
    Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE},
};

// ── D3D12Decoder ─────────────────────────────────────────────────────────────

/// Direct3D 12 hardware HEVC video decoder.
///
/// All D3D12/media COM objects implement `Send + Sync` in `windows 0.58`.
/// Persistent `HANDLE` fields are intentionally avoided so `D3D12Decoder` is `Send + Sync`
/// automatically without requiring unsafe raw pointer overrides.
#[derive(Debug)]
pub struct D3D12Decoder {
    initialized: bool,
    codec: String,
    width: u32,
    height: u32,
    output_queue: VecDeque<DecodedFrame>,
    decoded_count: u64,

    // ── Windows-only D3D12 objects ────────────────────────────────────────
    #[cfg(target_os = "windows")]
    device: Option<ID3D12Device>,
    #[cfg(target_os = "windows")]
    video_device: Option<ID3D12VideoDevice>,
    #[cfg(target_os = "windows")]
    command_queue: Option<ID3D12CommandQueue>,
    #[cfg(target_os = "windows")]
    command_allocator: Option<ID3D12CommandAllocator>,
    #[cfg(target_os = "windows")]
    video_decoder: Option<ID3D12VideoDecoder>,
    #[cfg(target_os = "windows")]
    decoder_heap: Option<ID3D12VideoDecoderHeap>,
    #[cfg(target_os = "windows")]
    output_texture: Option<ID3D12Resource>,
    #[cfg(target_os = "windows")]
    fence: Option<ID3D12Fence>,
    #[cfg(target_os = "windows")]
    fence_value: u64,

    // Read-back layout: populated during init, used every decode call
    #[cfg(target_os = "windows")]
    y_footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    #[cfg(target_os = "windows")]
    uv_footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    #[cfg(target_os = "windows")]
    y_num_rows: u32,
    #[cfg(target_os = "windows")]
    uv_num_rows: u32,
    #[cfg(target_os = "windows")]
    readback_total_bytes: u64,
    #[cfg(target_os = "windows")]
    readback_buffer: Option<ID3D12Resource>,
}

impl Default for D3D12Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl D3D12Decoder {
    /// Creates a new `D3D12Decoder` with all handles in the uninitialised state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            codec: String::new(),
            width: 0,
            height: 0,
            output_queue: VecDeque::new(),
            decoded_count: 0,
            #[cfg(target_os = "windows")]
            device: None,
            #[cfg(target_os = "windows")]
            video_device: None,
            #[cfg(target_os = "windows")]
            command_queue: None,
            #[cfg(target_os = "windows")]
            command_allocator: None,
            #[cfg(target_os = "windows")]
            video_decoder: None,
            #[cfg(target_os = "windows")]
            decoder_heap: None,
            #[cfg(target_os = "windows")]
            output_texture: None,
            #[cfg(target_os = "windows")]
            fence: None,
            #[cfg(target_os = "windows")]
            fence_value: 0,
            #[cfg(target_os = "windows")]
            y_footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                Offset: 0,
                Footprint: windows::Win32::Graphics::Direct3D12::D3D12_SUBRESOURCE_FOOTPRINT {
                    Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
                    Width: 0,
                    Height: 0,
                    Depth: 0,
                    RowPitch: 0,
                },
            },
            #[cfg(target_os = "windows")]
            uv_footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                Offset: 0,
                Footprint: windows::Win32::Graphics::Direct3D12::D3D12_SUBRESOURCE_FOOTPRINT {
                    Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
                    Width: 0,
                    Height: 0,
                    Depth: 0,
                    RowPitch: 0,
                },
            },
            #[cfg(target_os = "windows")]
            y_num_rows: 0,
            #[cfg(target_os = "windows")]
            uv_num_rows: 0,
            #[cfg(target_os = "windows")]
            readback_total_bytes: 0,
            #[cfg(target_os = "windows")]
            readback_buffer: None,
        }
    }

    /// Target video codec string (e.g. `"hevc"`).
    #[must_use]
    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Total count of decoded frames so far.
    #[must_use]
    pub const fn decoded_count(&self) -> u64 {
        self.decoded_count
    }
}

// ── Decoder trait ─────────────────────────────────────────────────────────────

impl Decoder for D3D12Decoder {
    fn initialize(&mut self, codec: &str, width: u32, height: u32) -> Result<(), ViewerError> {
        self.codec = codec.to_lowercase();
        self.width = width;
        self.height = height;
        self.initialized = true;

        #[cfg(target_os = "windows")]
        self.init_d3d12_video_decoder()?;

        tracing::info!(
            codec = %self.codec,
            width,
            height,
            "D3D12Decoder initialized"
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
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }

        let start_time = Instant::now();

        tracing::trace!(
            frame_id,
            pts_ns,
            packet_len = packet.len(),
            "D3D12Decoder: decode_packet"
        );

        #[cfg(target_os = "windows")]
        {
            let frame = self.decode_packet_d3d12(packet, frame_id, pts_ns, start_time)?;
            self.output_queue.push_back(frame);
            self.decoded_count += 1;
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (packet, frame_id, pts_ns, start_time);
            Err(ViewerError::Decoder(
                "D3D12Decoder is only available on Windows".to_string(),
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
        Ok(())
    }
}

// ── Windows implementation ────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
impl D3D12Decoder {
    /// Initialise D3D12 device, video device, command infrastructure, decoder, and readback resources.
    fn init_d3d12_video_decoder(&mut self) -> Result<(), ViewerError> {
        unsafe { self.init_d3d12_inner() }
    }

    unsafe fn init_d3d12_inner(&mut self) -> Result<(), ViewerError> {
        use windows::core::Interface;

        // 1. DXGI factory ─────────────────────────────────────────────────────
        let factory: IDXGIFactory1 = CreateDXGIFactory1()
            .map_err(|e| ViewerError::Decoder(format!("CreateDXGIFactory1: {e}")))?;

        // 2. Adapter enumeration – pick the first adapter that can create a D3D12 device ──
        let mut adapter_index = 0u32;
        let device: ID3D12Device = loop {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(adapter_index) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => {
                    return Err(ViewerError::Decoder(
                        "No D3D12-capable adapter found".to_string(),
                    ))
                }
                Err(e) => return Err(ViewerError::Decoder(format!("EnumAdapters1: {e}"))),
            };
            let mut dev: Option<ID3D12Device> = None;
            if D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut dev).is_ok() {
                if let Some(d) = dev {
                    tracing::info!(adapter_index, "D3D12: selected adapter");
                    break d;
                }
            }
            adapter_index += 1;
        };

        // 3. Video device (QueryInterface) ────────────────────────────────────
        let video_device: ID3D12VideoDevice = device
            .cast()
            .map_err(|e| ViewerError::Decoder(format!("QI ID3D12VideoDevice: {e}")))?;

        // 4. Verify HEVC decode support ───────────────────────────────────────
        let mut support = D3D12_VIDEO_DECODE_SUPPORT_DATA {
            Configuration: D3D12_VIDEO_DECODE_CONFIGURATION {
                DecodeProfile: D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN,
                BitstreamEncryption: D3D12_BITSTREAM_ENCRYPTION_TYPE_NONE,
                InterlaceType: D3D12_VIDEO_FRAME_CODED_INTERLACE_TYPE_NONE,
            },
            Width: self.width,
            Height: self.height,
            DecodeFormat: DXGI_FORMAT_NV12,
            FrameRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            BitRate: 0,
            ..Default::default()
        };
        video_device
            .CheckFeatureSupport(
                D3D12_FEATURE_VIDEO_DECODE_SUPPORT,
                &raw mut support as *mut _,
                std::mem::size_of::<D3D12_VIDEO_DECODE_SUPPORT_DATA>() as u32,
            )
            .map_err(|e| ViewerError::Decoder(format!("CheckFeatureSupport HEVC: {e}")))?;
        if support.SupportFlags != D3D12_VIDEO_DECODE_SUPPORT_FLAG_SUPPORTED {
            return Err(ViewerError::Decoder(
                "Hardware HEVC Main NV12 decode not supported on this adapter".to_string(),
            ));
        }
        tracing::info!("D3D12: HEVC Main NV12 decode supported");

        // 5. VIDEO_DECODE command queue ────────────────────────────────────────
        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_VIDEO_DECODE,
            Priority: D3D12_COMMAND_QUEUE_PRIORITY_NORMAL.0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        };
        let command_queue: ID3D12CommandQueue = device
            .CreateCommandQueue(&queue_desc)
            .map_err(|e| ViewerError::Decoder(format!("CreateCommandQueue: {e}")))?;

        // 6. Command allocator ─────────────────────────────────────────────────
        let command_allocator: ID3D12CommandAllocator = device
            .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_VIDEO_DECODE)
            .map_err(|e| ViewerError::Decoder(format!("CreateCommandAllocator: {e}")))?;

        // 7. VideoDecoder ─────────────────────────────────────────────────────
        let decoder_desc = D3D12_VIDEO_DECODER_DESC {
            NodeMask: 0,
            Configuration: D3D12_VIDEO_DECODE_CONFIGURATION {
                DecodeProfile: D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN,
                BitstreamEncryption: D3D12_BITSTREAM_ENCRYPTION_TYPE_NONE,
                InterlaceType: D3D12_VIDEO_FRAME_CODED_INTERLACE_TYPE_NONE,
            },
        };
        let video_decoder: ID3D12VideoDecoder = video_device
            .CreateVideoDecoder(&decoder_desc)
            .map_err(|e| ViewerError::Decoder(format!("CreateVideoDecoder: {e}")))?;

        // 8. VideoDecoderHeap ─────────────────────────────────────────────────
        let heap_desc = D3D12_VIDEO_DECODER_HEAP_DESC {
            NodeMask: 0,
            Configuration: D3D12_VIDEO_DECODE_CONFIGURATION {
                DecodeProfile: D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN,
                BitstreamEncryption: D3D12_BITSTREAM_ENCRYPTION_TYPE_NONE,
                InterlaceType: D3D12_VIDEO_FRAME_CODED_INTERLACE_TYPE_NONE,
            },
            DecodeWidth: self.width,
            DecodeHeight: self.height,
            Format: DXGI_FORMAT_NV12,
            FrameRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            BitRate: 0,
            MaxDecodePictureBufferCount: 1,
        };
        let decoder_heap: ID3D12VideoDecoderHeap = video_device
            .CreateVideoDecoderHeap(&heap_desc)
            .map_err(|e| ViewerError::Decoder(format!("CreateVideoDecoderHeap: {e}")))?;

        // 9. Output NV12 texture (GPU default heap) ───────────────────────────
        let output_heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        let output_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: u64::from(self.width),
            Height: self.height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
        };
        let mut output_texture: Option<ID3D12Resource> = None;
        device
            .CreateCommittedResource(
                &output_heap,
                D3D12_HEAP_FLAG_NONE,
                &output_desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut output_texture,
            )
            .map_err(|e| ViewerError::Decoder(format!("CreateCommittedResource (output): {e}")))?;
        let output_texture = output_texture.ok_or_else(|| {
            ViewerError::Decoder("CreateCommittedResource returned None (output)".to_string())
        })?;

        // 10. GetCopyableFootprints for the NV12 texture (2 subresources: Y + UV) ──
        let mut footprints = [
            D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(),
            D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(),
        ];
        let mut num_rows = [0u32; 2];
        let mut row_size_bytes = [0u64; 2];
        let mut total_bytes = 0u64;
        device.GetCopyableFootprints(
            &output_desc,
            0,
            2,
            0,
            Some(footprints.as_mut_ptr()),
            Some(num_rows.as_mut_ptr()),
            Some(row_size_bytes.as_mut_ptr()),
            Some(&mut total_bytes),
        );

        tracing::info!(
            y_offset = footprints[0].Offset,
            y_row_pitch = footprints[0].Footprint.RowPitch,
            y_num_rows = num_rows[0],
            uv_offset = footprints[1].Offset,
            uv_row_pitch = footprints[1].Footprint.RowPitch,
            uv_num_rows = num_rows[1],
            total_bytes,
            "D3D12: NV12 readback footprint"
        );

        // 11. Read-back buffer (CPU read, GPU copy destination) ───────────────
        let rb_heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_READBACK,
            ..Default::default()
        };
        let rb_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: total_bytes,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: windows::Win32::Graphics::Direct3D12::D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };
        let mut readback_buffer: Option<ID3D12Resource> = None;
        device
            .CreateCommittedResource(
                &rb_heap,
                D3D12_HEAP_FLAG_NONE,
                &rb_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
                &mut readback_buffer,
            )
            .map_err(|e| {
                ViewerError::Decoder(format!("CreateCommittedResource (readback): {e}"))
            })?;
        let readback_buffer = readback_buffer.ok_or_else(|| {
            ViewerError::Decoder("CreateCommittedResource returned None (readback)".to_string())
        })?;

        // 12. Fence ───────────────────────────────────────────────────────────
        let fence: ID3D12Fence = device
            .CreateFence(0, D3D12_FENCE_FLAG_NONE)
            .map_err(|e| ViewerError::Decoder(format!("CreateFence: {e}")))?;

        // Store everything
        self.device = Some(device);
        self.video_device = Some(video_device);
        self.command_queue = Some(command_queue);
        self.command_allocator = Some(command_allocator);
        self.video_decoder = Some(video_decoder);
        self.decoder_heap = Some(decoder_heap);
        self.output_texture = Some(output_texture);
        self.readback_buffer = Some(readback_buffer);
        self.fence = Some(fence);
        self.fence_value = 0;
        self.y_footprint = footprints[0];
        self.uv_footprint = footprints[1];
        self.y_num_rows = num_rows[0];
        self.uv_num_rows = num_rows[1];
        self.readback_total_bytes = total_bytes;

        Ok(())
    }

    /// Submit one HEVC packet for hardware decode, read back the NV12 result, and
    /// return a [`DecodedFrame`] containing a tightly-packed NV12 buffer.
    fn decode_packet_d3d12(
        &mut self,
        packet: &[u8],
        frame_id: u64,
        pts_ns: u64,
        start_time: Instant,
    ) -> Result<DecodedFrame, ViewerError> {
        unsafe { self.decode_inner(packet, frame_id, pts_ns, start_time) }
    }

    #[allow(clippy::too_many_lines)]
    unsafe fn decode_inner(
        &mut self,
        packet: &[u8],
        frame_id: u64,
        pts_ns: u64,
        start_time: Instant,
    ) -> Result<DecodedFrame, ViewerError> {
        use windows::core::Interface;

        let device = self
            .device
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("device not init".to_string()))?;
        let video_device = self
            .video_device
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("video_device not init".to_string()))?;
        let command_queue = self
            .command_queue
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("command_queue not init".to_string()))?;
        let command_allocator = self
            .command_allocator
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("command_allocator not init".to_string()))?;
        let video_decoder = self
            .video_decoder
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("video_decoder not init".to_string()))?;
        let decoder_heap = self
            .decoder_heap
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("decoder_heap not init".to_string()))?;
        let output_texture = self
            .output_texture
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("output_texture not init".to_string()))?;
        let readback_buffer = self
            .readback_buffer
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("readback_buffer not init".to_string()))?;
        let fence = self
            .fence
            .as_ref()
            .ok_or_else(|| ViewerError::Decoder("fence not init".to_string()))?;

        // 1. Upload bitstream to a temporary UPLOAD buffer ────────────────────
        let upload_heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_UPLOAD,
            ..Default::default()
        };
        let upload_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: packet.len() as u64,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: windows::Win32::Graphics::Direct3D12::D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };
        let mut upload_resource: Option<ID3D12Resource> = None;
        device
            .CreateCommittedResource(
                &upload_heap,
                D3D12_HEAP_FLAG_NONE,
                &upload_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                &mut upload_resource,
            )
            .map_err(|e| ViewerError::Decoder(format!("CreateCommittedResource (upload): {e}")))?;
        let upload_resource = upload_resource
            .ok_or_else(|| ViewerError::Decoder("upload resource None".to_string()))?;

        // Map, copy packet, unmap
        let mut mapped_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        upload_resource
            .Map(0, None, Some(&mut mapped_ptr))
            .map_err(|e| ViewerError::Decoder(format!("Map upload: {e}")))?;
        std::ptr::copy_nonoverlapping(packet.as_ptr(), mapped_ptr.cast::<u8>(), packet.len());
        upload_resource.Unmap(0, None);

        // 2. Reset command allocator and create a new video decode command list ──
        command_allocator
            .Reset()
            .map_err(|e| ViewerError::Decoder(format!("CommandAllocator::Reset: {e}")))?;

        let cmd_list: ID3D12VideoDecodeCommandList = device
            .CreateCommandList(
                0,
                D3D12_COMMAND_LIST_TYPE_VIDEO_DECODE,
                command_allocator,
                None::<&windows::Win32::Graphics::Direct3D12::ID3D12PipelineState>,
            )
            .map_err(|e| ViewerError::Decoder(format!("CreateCommandList: {e}")))?;

        // 3. Transition output texture: COMMON → VIDEO_DECODE_WRITE ──────────
        let barrier_to_decode = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: std::mem::ManuallyDrop::new(Some(output_texture.clone())),
                    Subresource: 0xffff_ffff,
                    StateBefore: D3D12_RESOURCE_STATE_COMMON,
                    StateAfter: D3D12_RESOURCE_STATE_VIDEO_DECODE_WRITE,
                }),
            },
        };
        cmd_list.ResourceBarrier(&[barrier_to_decode]);

        // 4. Build decode arguments ────────────────────────────────────────────
        let compressed = D3D12_VIDEO_DECODE_COMPRESSED_BITSTREAM {
            pBuffer: std::mem::ManuallyDrop::new(Some(upload_resource.clone())),
            Offset: 0,
            Size: packet.len() as u64,
        };
        let input_args = D3D12_VIDEO_DECODE_INPUT_STREAM_ARGUMENTS {
            NumFrameArguments: 0,
            FrameArguments: Default::default(),
            ReferenceFrames: D3D12_VIDEO_DECODE_REFERENCE_FRAMES {
                NumTexture2Ds: 0,
                ppTexture2Ds: std::ptr::null_mut(),
                pSubresources: std::ptr::null(),
                ppHeaps: std::ptr::null_mut(),
            },
            CompressedBitstream: compressed,
            pHeap: std::mem::ManuallyDrop::new(Some(decoder_heap.clone())),
        };
        let conversion = D3D12_VIDEO_DECODE_CONVERSION_ARGUMENTS {
            Enable: FALSE,
            pReferenceTexture2D: std::mem::ManuallyDrop::new(None),
            ReferenceSubresource: 0,
            OutputColorSpace: Default::default(),
            DecodeColorSpace: Default::default(),
        };
        let output_args = D3D12_VIDEO_DECODE_OUTPUT_STREAM_ARGUMENTS {
            pOutputTexture2D: std::mem::ManuallyDrop::new(Some(output_texture.clone())),
            OutputSubresource: 0,
            ConversionArguments: conversion,
        };

        cmd_list.DecodeFrame(video_decoder, &output_args, &input_args);

        // 5. Transition output texture: VIDEO_DECODE_WRITE → COPY_SOURCE ────
        let barrier_to_copy_src = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: std::mem::ManuallyDrop::new(Some(output_texture.clone())),
                    Subresource: 0xffff_ffff,
                    StateBefore: D3D12_RESOURCE_STATE_VIDEO_DECODE_WRITE,
                    StateAfter: D3D12_RESOURCE_STATE_COPY_SOURCE,
                }),
            },
        };
        cmd_list.ResourceBarrier(&[barrier_to_copy_src]);

        // 6. Copy both NV12 subresources to readback buffer ──────────────────
        // Y plane (subresource 0)
        let dst_y = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::ManuallyDrop::new(Some(readback_buffer.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: self.y_footprint,
            },
        };
        let src_y = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::ManuallyDrop::new(Some(output_texture.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                SubresourceIndex: 0,
            },
        };
        cmd_list.CopyTextureRegion(&dst_y, 0, 0, 0, &src_y, None);

        // UV plane (subresource 1)
        let dst_uv = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::ManuallyDrop::new(Some(readback_buffer.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: self.uv_footprint,
            },
        };
        let src_uv = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::ManuallyDrop::new(Some(output_texture.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                SubresourceIndex: 1,
            },
        };
        cmd_list.CopyTextureRegion(&dst_uv, 0, 0, 0, &src_uv, None);

        // 7. Close and execute ─────────────────────────────────────────────────
        cmd_list
            .Close()
            .map_err(|e| ViewerError::Decoder(format!("CommandList::Close: {e}")))?;

        let cmd_list_base: windows::Win32::Graphics::Direct3D12::ID3D12CommandList = cmd_list
            .cast()
            .map_err(|e| ViewerError::Decoder(format!("cast to ID3D12CommandList: {e}")))?;
        command_queue.ExecuteCommandLists(&[Some(cmd_list_base)]);

        // 8. Fence signal + wait (event handle created and closed here) ────────
        self.fence_value += 1;
        let signal_value = self.fence_value;
        command_queue
            .Signal(fence, signal_value)
            .map_err(|e| ViewerError::Decoder(format!("Signal: {e}")))?;

        if fence.GetCompletedValue() < signal_value {
            let event = CreateEventW(None, false, false, None)
                .map_err(|e| ViewerError::Decoder(format!("CreateEventW: {e}")))?;
            fence
                .SetEventOnCompletion(signal_value, event)
                .map_err(|e| {
                    let _ = CloseHandle(event);
                    ViewerError::Decoder(format!("SetEventOnCompletion: {e}"))
                })?;
            WaitForSingleObject(event, INFINITE);
            let _ = CloseHandle(event);
        }

        // 9. Map readback buffer, extract NV12 planes row-by-row ─────────────
        let mut mapped: *mut core::ffi::c_void = std::ptr::null_mut();
        readback_buffer
            .Map(0, None, Some(&mut mapped))
            .map_err(|e| ViewerError::Decoder(format!("Map readback: {e}")))?;

        let mapped_slice =
            std::slice::from_raw_parts(mapped.cast::<u8>(), self.readback_total_bytes as usize);

        let width = self.width as usize;
        let height = self.height as usize;
        let y_row_pitch = self.y_footprint.Footprint.RowPitch as usize;
        let uv_row_pitch = self.uv_footprint.Footprint.RowPitch as usize;
        let y_offset = self.y_footprint.Offset as usize;
        let uv_offset = self.uv_footprint.Offset as usize;

        // Tightly-packed NV12: width*height Y bytes + width*(height/2) UV bytes
        let mut nv12 = Vec::<u8>::with_capacity(width * height * 3 / 2);

        // Y plane – copy each visible row, strip GPU padding
        for row in 0..self.y_num_rows as usize {
            let src_start = y_offset + row * y_row_pitch;
            let src_end = src_start + width;
            nv12.extend_from_slice(&mapped_slice[src_start..src_end]);
        }

        // UV plane – each UV row contains width interleaved U+V bytes
        for row in 0..self.uv_num_rows as usize {
            let src_start = uv_offset + row * uv_row_pitch;
            let src_end = src_start + width;
            nv12.extend_from_slice(&mapped_slice[src_start..src_end]);
        }

        let read_range = D3D12_RANGE {
            Begin: 0,
            End: self.readback_total_bytes as usize,
        };
        readback_buffer.Unmap(0, Some(&read_range));

        // 10. Log diagnostics for first several frames ─────────────────────────
        if self.decoded_count < 8 {
            log_nv12_stats(&nv12, self.width, self.height, frame_id, packet.len());
        }

        let decode_duration = start_time.elapsed();
        Ok(DecodedFrame {
            frame_id,
            pts_ns,
            width: self.width,
            height: self.height,
            format: PixelFormat::Nv12,
            buffer: nv12,
            decode_duration,
        })
    }
}

// ── Diagnostics ───────────────────────────────────────────────────────────────

/// Log per-plane statistics for an NV12 buffer.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn log_nv12_stats(buf: &[u8], width: u32, height: u32, frame_id: u64, packet_len: usize) {
    let y_len = (width as usize) * (height as usize);
    let uv_len = y_len / 2;

    if buf.len() < y_len + uv_len {
        tracing::warn!(
            buf_len = buf.len(),
            expected = y_len + uv_len,
            "NV12 buffer short"
        );
        return;
    }

    let y = &buf[..y_len];
    let uv = &buf[y_len..y_len + uv_len];

    let y_min = *y.iter().min().unwrap_or(&0);
    let y_max = *y.iter().max().unwrap_or(&0);
    let y_avg = y.iter().map(|&b| u64::from(b)).sum::<u64>() / y_len as u64;

    let uv_min = *uv.iter().min().unwrap_or(&0);
    let uv_max = *uv.iter().max().unwrap_or(&0);
    let uv_avg = uv.iter().map(|&b| u64::from(b)).sum::<u64>() / uv_len as u64;

    let y_first16: &[u8] = &y[..16.min(y_len)];
    let uv_first16: &[u8] = &uv[..16.min(uv_len)];

    tracing::info!(
        frame_id,
        packet_len,
        width,
        height,
        y_min,
        y_max,
        y_avg,
        uv_min,
        uv_max,
        uv_avg,
        y_first16 = ?y_first16,
        uv_first16 = ?uv_first16,
        "D3D12Decoder: decoded frame NV12 stats"
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_d3d12_decoder_new() {
        let decoder = D3D12Decoder::new();
        assert!(decoder.codec().is_empty());
        assert_eq!(decoder.decoded_count(), 0);
    }

    #[test]
    fn test_d3d12_decoder_reset() {
        let mut decoder = D3D12Decoder::new();
        assert!(decoder.reset().is_ok());
    }

    /// On non-Windows platforms, `decode_packet` must return an error (not panic).
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_decode_packet_non_windows_returns_error() {
        let mut decoder = D3D12Decoder::new();
        decoder.codec = "hevc".to_string();
        decoder.width = 64;
        decoder.height = 64;
        decoder.initialized = true;
        let result = decoder.decode_packet(&[0u8; 16], 1, 0);
        assert!(result.is_err(), "expected error on non-Windows");
    }
}
