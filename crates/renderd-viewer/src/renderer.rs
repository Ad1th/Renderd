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
                                let nv12_min_len = num_pixels + num_pixels / 2;
                                if src.len() >= nv12_min_len && dest.len() >= num_pixels {
                                    let y_plane = &src[..num_pixels];
                                    let uv_plane = &src[num_pixels..nv12_min_len];

                                    for row in 0..height {
                                        for col in 0..width {
                                            let y_idx = (row * width + col) as usize;
                                            let y_val = f32::from(y_plane[y_idx]);

                                            let uv_row = row / 2;
                                            let uv_col = col & !1;
                                            let uv_offset = (uv_row * width + uv_col) as usize;

                                            let u_val = f32::from(uv_plane[uv_offset]) - 128.0;
                                            let v_val = f32::from(uv_plane[uv_offset + 1]) - 128.0;

                                            let r =
                                                (y_val + 1.402 * v_val).clamp(0.0, 255.0) as u32;
                                            let g = (y_val - 0.344_136 * u_val - 0.714_136 * v_val)
                                                .clamp(0.0, 255.0)
                                                as u32;
                                            let b =
                                                (y_val + 1.772 * u_val).clamp(0.0, 255.0) as u32;
                                            let a = 255u32;

                                            let pixel = (a << 24) | (r << 16) | (g << 8) | b;
                                            if (pixel & 0x00FF_FFFF) != 0 {
                                                non_zero_pixels += 1;
                                            }
                                            dest[y_idx] = pixel;
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
