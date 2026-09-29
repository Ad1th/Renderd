//! Direct3D 11 presenter for Windows (`renderd-viewer/src/render/d3d11_presenter.rs`).
//!
//! The software renderer converted every NV12 frame to BGRA on the CPU, scaled
//! it with nearest-neighbour sampling, and pushed it to the window through GDI.
//! At 1080p that is millions of pixels of CPU work per frame, on the UI thread,
//! plus a blit the compositor cannot schedule — time that lands directly on
//! glass-to-glass latency.
//!
//! This presenter keeps the frame on the GPU instead. A hardware-decoded frame
//! arrives as the decoder's own texture slice ([`GpuSurface`]); the D3D11 video
//! processor — fixed-function hardware on every GPU with a video engine —
//! converts it from BT.709 limited-range YCbCr to RGB, scales it with proper
//! filtering, and letterboxes it straight into a flip-model swap chain's back
//! buffer. Frames that arrive in system memory (software decode) are uploaded
//! to a dynamic texture and take the same path.
//!
//! Latency choices:
//! - flip-model swap chain (`FLIP_DISCARD`), two buffers, and the device's
//!   maximum frame latency capped at 1 (see [`D3d11Context`]), so at most one
//!   frame is ever queued ahead of the display;
//! - presented with sync interval 0: the compositor shows the newest frame at
//!   the next refresh and a frame that is superseded before then is dropped
//!   rather than queued. With `vsync = false` and driver support, frames are
//!   presented with `ALLOW_TEARING` for the lowest possible latency in
//!   fullscreen.

#![allow(unsafe_code)]

use std::sync::Arc;

use windows::core::Interface;
use windows::Win32::Foundation::{FALSE, HWND, RECT, TRUE};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CPU_ACCESS_WRITE, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_TEX2D_VPIV,
    D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DYNAMIC, D3D11_VIDEO_COLOR,
    D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12,
    DXGI_FORMAT_UNKNOWN, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory2, IDXGISwapChain1, DXGI_ERROR_DEVICE_REMOVED,
    DXGI_ERROR_DEVICE_RESET, DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING,
    DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use crate::decoder::{DecodedFrame, PixelFormat};
use crate::error::ViewerError;
use crate::gpu::{D3d11Context, GpuSurface};
use crate::renderer::{compute_aspect_fit_rect, Renderer, ViewportSize};

/// `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` for the decoded stream: BT.709 matrix
/// (bit 2) and 16-235 nominal range (bits 4-5 = 1), which is what
/// `ScreenCaptureKit`'s `420v` capture and `VideoToolbox` produce.
const INPUT_COLOR_SPACE: u32 = (1 << 2) | (1 << 4);

/// `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` for the back buffer: full-range RGB.
const OUTPUT_COLOR_SPACE: u32 = 0;

/// Input views kept before the cache is flushed. The decoder's pool is a
/// handful of slices of one texture array, so this is only reached when a new
/// session brings new textures.
const MAX_CACHED_INPUT_VIEWS: usize = 32;

/// Video processor sized for one input and output size.
struct VideoPipeline {
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    input: (u32, u32),
    output: (u32, u32),
}

/// A CPU-side frame's staging texture.
struct Upload {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

/// What is on screen, kept so a resize can redraw it.
struct Shown {
    texture: ID3D11Texture2D,
    array_index: u32,
    width: u32,
    height: u32,
    /// Holds the decoder's sample so it does not recycle the surface while shown.
    _surface: Option<GpuSurface>,
}

/// Direct3D 11 video-processor presenter.
pub struct D3d11Presenter {
    d3d: Arc<D3d11Context>,
    window: Option<Arc<winit::window::Window>>,
    hwnd: Option<HWND>,
    swapchain: Option<IDXGISwapChain1>,
    tearing: bool,
    vsync: bool,
    size: ViewportSize,
    processor: Option<VideoPipeline>,
    input_views: Vec<(usize, u32, ID3D11VideoProcessorInputView)>,
    upload: Option<Upload>,
    shown: Option<Shown>,
    dirty: bool,
}

// SAFETY: every D3D11/DXGI object here belongs to the multithread-protected
// device in `D3d11Context`. The presenter itself is only driven from the UI
// thread; `Renderer: Send + Sync` is required so the app can own it.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for D3d11Presenter {}
unsafe impl Sync for D3d11Presenter {}

impl std::fmt::Debug for D3d11Presenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3d11Presenter")
            .field("size", &self.size)
            .field("tearing", &self.tearing)
            .field("vsync", &self.vsync)
            .field("has_swapchain", &self.swapchain.is_some())
            .finish_non_exhaustive()
    }
}

fn renderer_err(what: &str, e: &windows::core::Error) -> ViewerError {
    ViewerError::Renderer(format!("{what} failed: {e}"))
}

impl D3d11Presenter {
    /// Creates a presenter on `d3d`'s device. `vsync = false` presents with
    /// tearing allowed where the driver supports it.
    #[must_use]
    pub fn new(d3d: Arc<D3d11Context>, vsync: bool) -> Self {
        Self {
            d3d,
            window: None,
            hwnd: None,
            swapchain: None,
            tearing: false,
            vsync,
            size: ViewportSize::default(),
            processor: None,
            input_views: Vec::new(),
            upload: None,
            shown: None,
            dirty: false,
        }
    }

    const fn swapchain_flags(&self) -> DXGI_SWAP_CHAIN_FLAG {
        if self.tearing {
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
        } else {
            DXGI_SWAP_CHAIN_FLAG(0)
        }
    }

    fn create_swapchain(&mut self, size: ViewportSize) -> Result<(), ViewerError> {
        let hwnd = self
            .hwnd
            .ok_or_else(|| ViewerError::Renderer("no window attached".into()))?;
        self.tearing = crate::render::check_tearing_support();
        // SAFETY: DXGI factory and swap chain creation with a valid HWND and a
        // device from the shared context.
        unsafe {
            let factory: IDXGIFactory2 =
                CreateDXGIFactory1().map_err(|e| renderer_err("CreateDXGIFactory1", &e))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: size.width.max(1),
                Height: size.height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: FALSE,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                #[allow(clippy::cast_sign_loss)]
                Flags: self.swapchain_flags().0 as u32,
            };
            let swapchain = factory
                .CreateSwapChainForHwnd(&self.d3d.device, hwnd, &desc, None, None)
                .map_err(|e| renderer_err("CreateSwapChainForHwnd", &e))?;
            // Alt+Enter exclusive fullscreen would bypass the window's own
            // borderless fullscreen and reset the swap chain.
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            self.swapchain = Some(swapchain);
        }
        Ok(())
    }

    /// Makes sure a video processor exists for this input and output size.
    fn ensure_processor(
        &mut self,
        input: (u32, u32),
        output: (u32, u32),
    ) -> Result<(), ViewerError> {
        if self
            .processor
            .as_ref()
            .is_some_and(|p| p.input == input && p.output == output)
        {
            return Ok(());
        }
        self.processor = None;
        self.input_views.clear();

        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            InputWidth: input.0,
            InputHeight: input.1,
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            },
            OutputWidth: output.0,
            OutputHeight: output.1,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        // SAFETY: video processor creation on the shared video device.
        unsafe {
            let enumerator = self
                .d3d
                .video_device
                .CreateVideoProcessorEnumerator(&content)
                .map_err(|e| renderer_err("CreateVideoProcessorEnumerator", &e))?;
            let processor = self
                .d3d
                .video_device
                .CreateVideoProcessor(&enumerator, 0)
                .map_err(|e| renderer_err("CreateVideoProcessor", &e))?;

            let video = &self.d3d.video_context;
            video.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            video.VideoProcessorSetStreamColorSpace(
                &processor,
                0,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                    _bitfield: INPUT_COLOR_SPACE,
                },
            );
            video.VideoProcessorSetOutputColorSpace(
                &processor,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                    _bitfield: OUTPUT_COLOR_SPACE,
                },
            );
            // No denoise / edge enhancement / colour tweaks: desktop content
            // should come out exactly as it went in.
            video.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, FALSE);
            video.VideoProcessorSetOutputBackgroundColor(
                &processor,
                FALSE,
                &D3D11_VIDEO_COLOR {
                    Anonymous: D3D11_VIDEO_COLOR_0 {
                        RGBA: D3D11_VIDEO_COLOR_RGBA {
                            R: 0.0,
                            G: 0.0,
                            B: 0.0,
                            A: 1.0,
                        },
                    },
                },
            );

            tracing::info!(
                input = ?input,
                output = ?output,
                "D3D11 video processor ready"
            );
            self.processor = Some(VideoPipeline {
                enumerator,
                processor,
                input,
                output,
            });
        }
        Ok(())
    }

    fn input_view(
        &mut self,
        texture: &ID3D11Texture2D,
        array_index: u32,
    ) -> Result<ID3D11VideoProcessorInputView, ViewerError> {
        let key = texture.as_raw() as usize;
        if let Some((_, _, view)) = self
            .input_views
            .iter()
            .find(|(t, i, _)| *t == key && *i == array_index)
        {
            return Ok(view.clone());
        }
        if self.input_views.len() >= MAX_CACHED_INPUT_VIEWS {
            self.input_views.clear();
        }
        let processor = self
            .processor
            .as_ref()
            .ok_or_else(|| ViewerError::Renderer("video processor missing".into()))?;
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: array_index,
                },
            },
        };
        let mut view = None;
        // SAFETY: the texture and enumerator belong to the shared device.
        unsafe {
            self.d3d
                .video_device
                .CreateVideoProcessorInputView(
                    texture,
                    &processor.enumerator,
                    &desc,
                    Some(&mut view),
                )
                .map_err(|e| renderer_err("CreateVideoProcessorInputView", &e))?;
        }
        let view =
            view.ok_or_else(|| ViewerError::Renderer("no video processor input view".into()))?;
        self.input_views.push((key, array_index, view.clone()));
        Ok(view)
    }

    /// Copies a system-memory frame into a dynamic texture the video processor
    /// can read.
    fn upload(&mut self, frame: &DecodedFrame) -> Result<ID3D11Texture2D, ViewerError> {
        let (format, bytes_needed) = match frame.format {
            PixelFormat::Nv12 => (
                DXGI_FORMAT_NV12,
                (frame.width as usize) * (frame.height as usize) * 3 / 2,
            ),
            PixelFormat::Bgra8 => (
                DXGI_FORMAT_B8G8R8A8_UNORM,
                (frame.width as usize) * (frame.height as usize) * 4,
            ),
            PixelFormat::P010 => {
                return Err(ViewerError::Renderer(
                    "P010 frames are not supported by the D3D11 presenter".into(),
                ))
            }
        };
        if frame.buffer.len() < bytes_needed || frame.width < 2 || frame.height < 2 {
            return Err(ViewerError::Renderer(format!(
                "frame buffer holds {} bytes, need {bytes_needed}",
                frame.buffer.len()
            )));
        }

        let reuse = self.upload.as_ref().is_some_and(|u| {
            u.width == frame.width && u.height == frame.height && u.format == format
        });
        if !reuse {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: frame.width,
                Height: frame.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DYNAMIC,
                #[allow(clippy::cast_sign_loss)]
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                #[allow(clippy::cast_sign_loss)]
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };
            let mut texture = None;
            // SAFETY: texture creation on the shared device with a valid desc.
            unsafe {
                self.d3d
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
                    .map_err(|e| renderer_err("CreateTexture2D (upload)", &e))?;
            }
            let texture =
                texture.ok_or_else(|| ViewerError::Renderer("no upload texture".into()))?;
            self.upload = Some(Upload {
                texture,
                width: frame.width,
                height: frame.height,
                format,
            });
        }
        let texture = self
            .upload
            .as_ref()
            .map(|u| u.texture.clone())
            .ok_or_else(|| ViewerError::Renderer("no upload texture".into()))?;

        let width = frame.width as usize;
        let height = frame.height as usize;
        let (row_bytes, rows) = if format == DXGI_FORMAT_NV12 {
            // Y rows, then the interleaved UV plane: half as many rows, same width.
            (width, height + height / 2)
        } else {
            (width * 4, height)
        };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the texture is DYNAMIC with CPU write access; the mapped region
        // is `RowPitch` bytes per row for `rows` rows (for NV12 the UV plane
        // follows the Y plane at `RowPitch * height`), and every copy below is
        // bounded by both the source length checked above and that pitch.
        unsafe {
            self.d3d
                .context
                .Map(&texture, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .map_err(|e| renderer_err("Map (upload)", &e))?;
            let pitch = mapped.RowPitch as usize;
            if mapped.pData.is_null() || pitch < row_bytes {
                self.d3d.context.Unmap(&texture, 0);
                return Err(ViewerError::Renderer(
                    "upload texture mapped unusably".into(),
                ));
            }
            let dest = mapped.pData.cast::<u8>();
            for row in 0..rows {
                std::ptr::copy_nonoverlapping(
                    frame.buffer.as_ptr().add(row * row_bytes),
                    dest.add(row * pitch),
                    row_bytes,
                );
            }
            self.d3d.context.Unmap(&texture, 0);
        }
        Ok(texture)
    }

    /// Draws whatever is in `self.shown` into the back buffer.
    fn draw(&mut self) -> Result<(), ViewerError> {
        let Some(swapchain) = self.swapchain.clone() else {
            return Ok(());
        };
        let Some((texture, array_index, width, height)) = self
            .shown
            .as_ref()
            .map(|s| (s.texture.clone(), s.array_index, s.width, s.height))
        else {
            return Ok(());
        };

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: GetDesc on a live texture.
        unsafe { texture.GetDesc(&mut desc) };
        let output = (self.size.width.max(1), self.size.height.max(1));
        self.ensure_processor((desc.Width, desc.Height), output)?;
        let input_view = self.input_view(&texture, array_index)?;
        let processor = self
            .processor
            .as_ref()
            .map(|p| (p.processor.clone(), p.enumerator.clone()))
            .ok_or_else(|| ViewerError::Renderer("video processor missing".into()))?;

        // SAFETY: back buffer 0 is always the current one under the D3D11 flip
        // model; every view and rect below refers to live objects on the shared
        // device, and the stream's input view reference is released after the
        // Blt.
        unsafe {
            let back_buffer: ID3D11Texture2D = swapchain
                .GetBuffer(0)
                .map_err(|e| renderer_err("GetBuffer", &e))?;
            let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output_view: Option<ID3D11VideoProcessorOutputView> = None;
            self.d3d
                .video_device
                .CreateVideoProcessorOutputView(
                    &back_buffer,
                    &processor.1,
                    &out_desc,
                    Some(&mut output_view),
                )
                .map_err(|e| renderer_err("CreateVideoProcessorOutputView", &e))?;
            let output_view = output_view
                .ok_or_else(|| ViewerError::Renderer("no video processor output view".into()))?;

            let (dst_x, dst_y, dst_w, dst_h) =
                compute_aspect_fit_rect(width, height, output.0, output.1);
            let to_rect = |x: u32, y: u32, w: u32, h: u32| RECT {
                left: i32::try_from(x).unwrap_or(0),
                top: i32::try_from(y).unwrap_or(0),
                right: i32::try_from(x + w).unwrap_or(i32::MAX),
                bottom: i32::try_from(y + h).unwrap_or(i32::MAX),
            };
            let video = &self.d3d.video_context;
            // The decoder's texture is the coded size (1080 rounds up to 1088);
            // only the visible rows are shown.
            video.VideoProcessorSetStreamSourceRect(
                &processor.0,
                0,
                TRUE,
                Some(&to_rect(0, 0, width, height)),
            );
            video.VideoProcessorSetStreamDestRect(
                &processor.0,
                0,
                TRUE,
                Some(&to_rect(dst_x, dst_y, dst_w, dst_h)),
            );
            video.VideoProcessorSetOutputTargetRect(
                &processor.0,
                TRUE,
                Some(&to_rect(0, 0, output.0, output.1)),
            );

            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: TRUE,
                pInputSurface: std::mem::ManuallyDrop::new(Some(input_view)),
                ..Default::default()
            };
            let result = video.VideoProcessorBlt(
                &processor.0,
                &output_view,
                0,
                std::slice::from_ref(&stream),
            );
            std::mem::ManuallyDrop::drop(&mut stream.pInputSurface);
            result.map_err(|e| renderer_err("VideoProcessorBlt", &e))?;
        }
        self.dirty = true;
        Ok(())
    }

    fn present_now(&mut self) -> Result<(), ViewerError> {
        if !self.dirty {
            return Ok(());
        }
        self.dirty = false;
        let Some(ref swapchain) = self.swapchain else {
            return Ok(());
        };
        let flags = if !self.vsync && self.tearing {
            DXGI_PRESENT_ALLOW_TEARING
        } else {
            DXGI_PRESENT(0)
        };
        // SAFETY: Present on a live swap chain.
        let hr = unsafe { swapchain.Present(0, flags) };
        if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
            return Err(ViewerError::Renderer(format!(
                "GPU device lost during Present ({hr:?})"
            )));
        }
        Ok(())
    }
}

impl Renderer for D3d11Presenter {
    fn attach_window(&mut self, window: Arc<winit::window::Window>) -> Result<(), ViewerError> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let handle = window
            .window_handle()
            .map_err(|e| ViewerError::Renderer(format!("no native window handle: {e}")))?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return Err(ViewerError::Renderer("window is not a Win32 window".into()));
        };
        self.hwnd = Some(HWND(win32.hwnd.get() as *mut core::ffi::c_void));
        self.window = Some(window);
        Ok(())
    }

    fn initialize(&mut self, initial_size: ViewportSize) -> Result<(), ViewerError> {
        self.size = initial_size;
        self.create_swapchain(initial_size)?;
        tracing::info!(
            width = initial_size.width,
            height = initial_size.height,
            tearing = self.tearing,
            vsync = self.vsync,
            "D3D11 presenter initialized (flip model, frame latency 1)"
        );
        Ok(())
    }

    fn resize(&mut self, new_size: ViewportSize) -> Result<(), ViewerError> {
        if new_size.width == 0 || new_size.height == 0 {
            // Minimised: keep the old buffers until the window comes back.
            return Ok(());
        }
        self.size = new_size;
        let flags = self.swapchain_flags();
        if let Some(ref swapchain) = self.swapchain {
            // SAFETY: no back-buffer views outlive a draw, so ResizeBuffers is legal.
            unsafe {
                swapchain
                    .ResizeBuffers(
                        0,
                        new_size.width,
                        new_size.height,
                        DXGI_FORMAT_UNKNOWN,
                        flags,
                    )
                    .map_err(|e| renderer_err("ResizeBuffers", &e))?;
            }
        }
        // Redraw what was on screen at the new size rather than waiting for the
        // next frame, which on a still desktop might be seconds away.
        self.draw()?;
        self.present_now()
    }

    fn render_frame(&mut self, frame: &DecodedFrame) -> Result<(), ViewerError> {
        let shown = if let Some(ref gpu) = frame.gpu {
            Shown {
                texture: gpu.texture.clone(),
                array_index: gpu.array_index,
                width: frame.width,
                height: frame.height,
                _surface: Some(gpu.clone()),
            }
        } else {
            Shown {
                texture: self.upload(frame)?,
                array_index: 0,
                width: frame.width,
                height: frame.height,
                _surface: None,
            }
        };
        self.shown = Some(shown);
        self.draw()
    }

    fn present(&mut self) -> Result<(), ViewerError> {
        self.present_now()
    }

    fn shutdown(&mut self) -> Result<(), ViewerError> {
        self.shown = None;
        self.input_views.clear();
        self.processor = None;
        self.upload = None;
        self.swapchain = None;
        self.window = None;
        self.hwnd = None;
        Ok(())
    }
}
