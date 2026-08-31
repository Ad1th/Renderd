//! Graphics renderer abstraction for swapchain management and frame presentation.

use crate::decoder::{DecodedFrame, PixelFormat};
use crate::error::ViewerError;
use std::fmt::Debug;

/// Surface dimensions for rendering viewport in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewportSize {
    /// Viewport width in physical pixels.
    pub width: u32,
    /// Viewport height in physical pixels.
    pub height: u32,
}

/// Trait abstraction for graphics renderers (e.g. `Direct3D12`, Vulkan, or Mock).
pub trait Renderer: Send + Sync {
    /// Attaches a native `winit` window to the renderer for software or hardware presentation.
    ///
    /// # Errors
    /// Returns [`ViewerError::Renderer`] if attaching window surface fails.
    fn attach_window(
        &mut self,
        _window: std::sync::Arc<winit::window::Window>,
    ) -> Result<(), ViewerError> {
        Ok(())
    }

    /// Initializes the graphics rendering context and swapchain for the window surface.
    ///
    /// # Errors
    /// Returns [`ViewerError::Renderer`] if graphics API initialization fails.
    fn initialize(&mut self, initial_size: ViewportSize) -> Result<(), ViewerError>;

    /// Handles window or viewport resize events, re-creating swapchain buffers as necessary.
    ///
    /// # Errors
    /// Returns [`ViewerError::Renderer`] if swapchain resize fails.
    fn resize(&mut self, new_size: ViewportSize) -> Result<(), ViewerError>;

    /// Renders an uncompressed [`DecodedFrame`] to the current back buffer.
    ///
    /// # Errors
    /// Returns [`ViewerError::Renderer`] if rendering fails.
    fn render_frame(&mut self, frame: &DecodedFrame) -> Result<(), ViewerError>;

    /// Presents the rendered back buffer to the display swapchain with vertical synchronization.
    ///
    /// # Errors
    /// Returns [`ViewerError::Renderer`] if presentation fails.
    fn present(&mut self) -> Result<(), ViewerError>;

    /// Shuts down the graphics renderer and releases GPU resources.
    ///
    /// # Errors
    /// Returns [`ViewerError::Renderer`] if shutdown fails.
    fn shutdown(&mut self) -> Result<(), ViewerError>;
}

static SOFT_RENDER_LOG_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Converts one NV12 pixel to a `0RGB` word, writing it into `out`.
///
/// Returns 1 if the resulting pixel has any colour, 0 if it is pure black; the caller
/// uses this to report how much of a frame was non-blank.
///
/// Uses BT.601 full-range coefficients in 16.16 fixed point. This runs once per pixel
/// per frame — over two million times per frame at 1080p — so the scalar float form it
/// replaces dominated the frame budget on the software path.
#[inline]
fn nv12_to_bgra(luma: u8, chroma_b: u8, chroma_r: u8, out: &mut u32) -> u8 {
    /// 1.402 << 16
    const R_CR: i32 = 91_881;
    /// 0.344136 << 16
    const G_CB: i32 = 22_554;
    /// 0.714136 << 16
    const G_CR: i32 = 46_802;
    /// 1.772 << 16
    const B_CB: i32 = 116_130;

    let luma = i32::from(luma) << 16;
    let chroma_b = i32::from(chroma_b) - 128;
    let chroma_r = i32::from(chroma_r) - 128;

    let red = ((luma + R_CR * chroma_r) >> 16).clamp(0, 255);
    let green = ((luma - G_CB * chroma_b - G_CR * chroma_r) >> 16).clamp(0, 255);
    let blue = ((luma + B_CB * chroma_b) >> 16).clamp(0, 255);

    #[allow(clippy::cast_sign_loss)]
    let pixel = 0xFF00_0000 | ((red as u32) << 16) | ((green as u32) << 8) | (blue as u32);
    *out = pixel;
    u8::from(pixel & 0x00FF_FFFF != 0)
}

/// Software surface renderer using `softbuffer` for cross-platform pixel presentation.
pub struct SoftRenderer {
    surface: std::sync::Mutex<
        Option<
            softbuffer::Surface<
                std::sync::Arc<winit::window::Window>,
                std::sync::Arc<winit::window::Window>,
            >,
        >,
    >,
    initialized: bool,
    size: ViewportSize,
}

impl Debug for SoftRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoftRenderer")
            .field("initialized", &self.initialized)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Default for SoftRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftRenderer {
    /// Creates a new [`SoftRenderer`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            surface: std::sync::Mutex::new(None),
            initialized: false,
            size: ViewportSize {
                width: 0,
                height: 0,
            },
        }
    }
}

impl Renderer for SoftRenderer {
    fn attach_window(
        &mut self,
        window: std::sync::Arc<winit::window::Window>,
    ) -> Result<(), ViewerError> {
        let context = softbuffer::Context::new(window.clone()).map_err(|e| {
            ViewerError::Renderer(format!("Failed to create softbuffer context: {e}"))
        })?;
        let surface = softbuffer::Surface::new(&context, window).map_err(|e| {
            ViewerError::Renderer(format!("Failed to create softbuffer surface: {e}"))
        })?;
        if let Ok(mut guard) = self.surface.lock() {
            *guard = Some(surface);
        }
        Ok(())
    }

    fn initialize(&mut self, initial_size: ViewportSize) -> Result<(), ViewerError> {
        self.size = initial_size;
        self.initialized = true;
        tracing::info!(
            width = initial_size.width,
            height = initial_size.height,
            "SoftRenderer initialized successfully"
        );
        Ok(())
    }

    fn resize(&mut self, new_size: ViewportSize) -> Result<(), ViewerError> {
        self.size = new_size;
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::suboptimal_flops
    )]
    fn render_frame(&mut self, frame: &DecodedFrame) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }

        if let Ok(mut guard) = self.surface.lock() {
            if let Some(ref mut surface) = *guard {
                let width = frame.width;
                let height = frame.height;
                if let (Some(w), Some(h)) = (
                    std::num::NonZeroU32::new(width),
                    std::num::NonZeroU32::new(height),
                ) {
                    let _ = surface.resize(w, h);
                    if let Ok(mut buffer) = surface.buffer_mut() {
                        let src = &frame.buffer;
                        let dest = &mut buffer;
                        let num_pixels = (width * height) as usize;
                        let mut non_zero_pixels = 0u64;

                        match frame.format {
                            PixelFormat::Bgra8 => {
                                if src.len() >= num_pixels * 4 && dest.len() >= num_pixels {
                                    for i in 0..num_pixels {
                                        let b = u32::from(src[i * 4]);
                                        let g = u32::from(src[i * 4 + 1]);
                                        let r = u32::from(src[i * 4 + 2]);
                                        let a = u32::from(src[i * 4 + 3]);
                                        let pixel = (a << 24) | (r << 16) | (g << 8) | b;
                                        if (pixel & 0x00FF_FFFF) != 0 {
                                            non_zero_pixels += 1;
                                        }
                                        dest[i] = pixel;
                                    }
                                }
                            }
                            PixelFormat::Nv12 | PixelFormat::P010 => {
                                let uv_width = (width as usize).div_ceil(2) * 2;
                                let uv_rows = (height as usize).div_ceil(2);
                                let y_len = num_pixels;
                                let uv_len = uv_rows * uv_width;

                                if src.len() >= y_len + uv_len && dest.len() >= num_pixels {
                                    let y_plane = &src[..y_len];
                                    let uv_plane = &src[y_len..y_len + uv_len];

                                    for row in 0..height as usize {
                                        let uv_row_base = (row / 2) * uv_width;
                                        for col in 0..width as usize {
                                            let y_idx = row * width as usize + col;
                                            let uv_offset = uv_row_base + (col & !1);

                                            non_zero_pixels += u64::from(nv12_to_bgra(
                                                y_plane[y_idx],
                                                uv_plane[uv_offset],
                                                uv_plane[uv_offset + 1],
                                                &mut dest[y_idx],
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        let count = SOFT_RENDER_LOG_COUNT
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            + 1;
                        if count <= 5 {
                            tracing::info!(
                                count = count,
                                format = ?frame.format,
                                width = width,
                                height = height,
                                src_len = src.len(),
                                dest_pixels = dest.len(),
                                non_zero_pixels = non_zero_pixels,
                                "RENDER: SoftRenderer presented frame to softbuffer swapchain"
                            );
                        }

                        let _ = buffer.present();
                    }
                }
            }
        }

        Ok(())
    }

    fn present(&mut self) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ViewerError> {
        self.initialized = false;
        if let Ok(mut guard) = self.surface.lock() {
            *guard = None;
        }
        Ok(())
    }
}

/// Null / Mock implementation of [`Renderer`] for testing and headless execution.
#[derive(Debug, Default)]
pub struct NullRenderer {
    initialized: bool,
    size: ViewportSize,
}

impl NullRenderer {
    /// Creates a new [`NullRenderer`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            size: ViewportSize {
                width: 0,
                height: 0,
            },
        }
    }

    /// Checks if the renderer is initialized.
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Returns current viewport size.
    #[must_use]
    pub const fn viewport_size(&self) -> ViewportSize {
        self.size
    }
}

impl Renderer for NullRenderer {
    fn initialize(&mut self, initial_size: ViewportSize) -> Result<(), ViewerError> {
        self.size = initial_size;
        self.initialized = true;
        Ok(())
    }

    fn resize(&mut self, new_size: ViewportSize) -> Result<(), ViewerError> {
        self.size = new_size;
        Ok(())
    }

    fn render_frame(&mut self, _frame: &DecodedFrame) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }
        Ok(())
    }

    fn present(&mut self) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ViewerError> {
        self.initialized = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_renderer_lifecycle() {
        let mut renderer = NullRenderer::new();
        assert!(!renderer.is_initialized());
        assert!(renderer.present().is_err());

        let sz = ViewportSize {
            width: 1920,
            height: 1080,
        };
        renderer.initialize(sz).unwrap();
        assert!(renderer.is_initialized());
        assert_eq!(renderer.viewport_size(), sz);

        let new_sz = ViewportSize {
            width: 2560,
            height: 1440,
        };
        renderer.resize(new_sz).unwrap();
        assert_eq!(renderer.viewport_size(), new_sz);

        assert!(renderer.present().is_ok());
        renderer.shutdown().unwrap();
        assert!(!renderer.is_initialized());
    }

    /// The fixed-point BT.601 conversion must agree with the float form it replaced.
    #[test]
    fn test_nv12_to_bgra_matches_float_reference() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::suboptimal_flops
        )]
        fn reference(luma: u8, chroma_b: u8, chroma_r: u8) -> (u32, u32, u32) {
            let luma = f32::from(luma);
            let chroma_b = f32::from(chroma_b) - 128.0;
            let chroma_r = f32::from(chroma_r) - 128.0;
            (
                (luma + 1.402 * chroma_r).clamp(0.0, 255.0) as u32,
                (luma - 0.344_136 * chroma_b - 0.714_136 * chroma_r).clamp(0.0, 255.0) as u32,
                (luma + 1.772 * chroma_b).clamp(0.0, 255.0) as u32,
            )
        }

        for luma in (0..=255u8).step_by(17) {
            for chroma_b in (0..=255u8).step_by(51) {
                for chroma_r in (0..=255u8).step_by(51) {
                    let mut out = 0u32;
                    nv12_to_bgra(luma, chroma_b, chroma_r, &mut out);
                    let got = ((out >> 16) & 0xFF, (out >> 8) & 0xFF, out & 0xFF);
                    let want = reference(luma, chroma_b, chroma_r);
                    assert!(
                        got.0.abs_diff(want.0) <= 1
                            && got.1.abs_diff(want.1) <= 1
                            && got.2.abs_diff(want.2) <= 1,
                        "luma={luma} cb={chroma_b} cr={chroma_r}: got {got:?} want {want:?}"
                    );
                    assert_eq!(out >> 24, 0xFF, "alpha must be opaque");
                }
            }
        }
    }

    /// Neutral chroma with zero luma is black and reports as blank.
    #[test]
    fn test_nv12_to_bgra_black_is_reported_blank() {
        let mut out = 0u32;
        assert_eq!(nv12_to_bgra(0, 128, 128, &mut out), 0);
        assert_eq!(out & 0x00FF_FFFF, 0);

        assert_eq!(nv12_to_bgra(255, 128, 128, &mut out), 1);
        assert_eq!(out & 0x00FF_FFFF, 0x00FF_FFFF);
    }

    /// An odd-sized NV12 frame must render without indexing past the chroma plane.
    /// The UV plane of an odd-width frame is padded to an even width, and an odd-height
    /// frame still carries a final half-height chroma row.
    #[test]
    fn test_odd_dimension_nv12_frame_is_within_bounds() {
        let (width, height) = (5usize, 3usize);
        let uv_width = width.div_ceil(2) * 2;
        let uv_rows = height.div_ceil(2);
        let y_len = width * height;
        let uv_len = uv_rows * uv_width;

        let src = vec![128u8; y_len + uv_len];
        let mut dest = vec![0u32; y_len];

        for row in 0..height {
            let uv_row_base = (row / 2) * uv_width;
            for col in 0..width {
                let y_idx = row * width + col;
                let uv_offset = uv_row_base + (col & !1);
                assert!(
                    y_len + uv_offset + 1 < src.len(),
                    "chroma read at row {row} col {col} is out of bounds"
                );
                nv12_to_bgra(
                    src[y_idx],
                    src[y_len + uv_offset],
                    src[y_len + uv_offset + 1],
                    &mut dest[y_idx],
                );
            }
        }
    }

    #[test]
    fn test_soft_renderer_uninitialized_error() {
        let mut renderer = SoftRenderer::new();
        let frame = DecodedFrame {
            frame_id: 1,
            pts_ns: 0,
            width: 100,
            height: 100,
            format: PixelFormat::Bgra8,
            buffer: vec![255u8; 40000],
            decode_duration: std::time::Duration::from_millis(1),
        };

        assert!(renderer.render_frame(&frame).is_err());
        assert!(renderer.present().is_err());
    }
}
