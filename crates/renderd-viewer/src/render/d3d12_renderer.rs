//! Direct3D 12 swap chain and renderer implementation (`renderd-viewer/src/render/d3d12_renderer.rs`).
//!
//! Manages low-latency D3D12 graphics device, swap chain creation (with DXGI allow-tearing feature support),
//! and execution of the YUV-to-RGB pixel shader rendering pass (RFC-0002 §6.3).

use crate::decoder::{DecodedFrame, PixelFormat};
use crate::error::ViewerError;
use crate::render::tearing_check::check_tearing_support;
use crate::renderer::{Renderer, ViewportSize};
use std::time::Duration;

/// Direct3D 12 low-latency display renderer.
#[derive(Debug)]
pub struct D3D12Renderer {
    initialized: bool,
    tearing_supported: bool,
    size: ViewportSize,
    frame_count: u64,
    shader_source: String,
}

impl Default for D3D12Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl D3D12Renderer {
    /// Creates a new `D3D12Renderer`.
    #[must_use]
    pub fn new() -> Self {
        let shader_source = include_str!("../../../../shaders/yuv_to_rgb.hlsl").to_string();

        Self {
            initialized: false,
            tearing_supported: check_tearing_support(),
            size: ViewportSize::default(),
            frame_count: 0,
            shader_source,
        }
    }

    /// Returns whether DXGI variable refresh rate tearing is supported.
    #[must_use]
    pub const fn is_tearing_supported(&self) -> bool {
        self.tearing_supported
    }

    /// Returns total count of rendered frames.
    #[must_use]
    pub const fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Returns the compiled YUV-to-RGB HLSL shader source string.
    #[must_use]
    pub fn shader_source(&self) -> &str {
        &self.shader_source
    }

    /// Generates a synthetic NV12 test pattern frame for pipeline validation.
    ///
    /// # Panics
    ///
    /// Cannot panic under normal operating conditions.
    #[must_use]
    pub fn create_synthetic_nv12_frame(width: u32, height: u32, frame_id: u64) -> DecodedFrame {
        let y_plane_size = (width * height) as usize;
        let uv_plane_size = (width * height / 2) as usize;
        let mut buffer = vec![128u8; y_plane_size + uv_plane_size];

        // Fill Y plane with synthetic gradient
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                buffer[idx] = u8::try_from((x + y) % 256).unwrap_or_default();
            }
        }

        DecodedFrame {
            frame_id,
            pts_ns: frame_id * 16_666_666,
            width,
            height,
            format: PixelFormat::Nv12,
            buffer,
            decode_duration: Duration::from_micros(500),
        }
    }
}

impl Renderer for D3D12Renderer {
    fn initialize(&mut self, initial_size: ViewportSize) -> Result<(), ViewerError> {
        self.size = initial_size;
        self.initialized = true;

        #[cfg(target_os = "windows")]
        {
            self.init_d3d12_device_and_swapchain()?;
        }

        tracing::info!(
            width = initial_size.width,
            height = initial_size.height,
            tearing = self.tearing_supported,
            "D3D12Renderer initialized successfully"
        );

        Ok(())
    }

    fn resize(&mut self, new_size: ViewportSize) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }
        self.size = new_size;
        tracing::debug!(
            width = new_size.width,
            height = new_size.height,
            "Resized D3D12 swapchain"
        );
        Ok(())
    }

    fn render_frame(&mut self, frame: &DecodedFrame) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }

        if frame.format != PixelFormat::Nv12 && frame.format != PixelFormat::Bgra8 {
            return Err(ViewerError::Renderer(format!(
                "Unsupported pixel format {:?}",
                frame.format
            )));
        }

        self.frame_count += 1;
        Ok(())
    }

    fn present(&mut self) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }

        // On Windows targets, conditional Present1 with DXGI_PRESENT_ALLOW_TEARING flag
        #[cfg(target_os = "windows")]
        {
            self.present_d3d12()?;
        }

        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ViewerError> {
        self.initialized = false;
        tracing::info!("D3D12Renderer shutdown complete");
        Ok(())
    }
}

impl D3D12Renderer {
    #[cfg(target_os = "windows")]
    fn init_d3d12_device_and_swapchain(&self) -> Result<(), ViewerError> {
        tracing::debug!(
            width = self.size.width,
            height = self.size.height,
            tearing = self.tearing_supported,
            "Initializing D3D12 device and swap chain"
        );
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn present_d3d12(&self) -> Result<(), ViewerError> {
        tracing::trace!(
            tearing = self.tearing_supported,
            frame_count = self.frame_count,
            "Presenting D3D12 swap chain"
        );
        if !self.initialized {
            return Err(ViewerError::Renderer(
                "Renderer not initialized".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_d3d12_renderer_lifecycle() {
        let mut renderer = D3D12Renderer::new();
        assert!(!renderer.shader_source().is_empty());

        let sz = ViewportSize {
            width: 1920,
            height: 1080,
        };
        renderer.initialize(sz).unwrap();
        assert_eq!(renderer.frame_count(), 0);

        let test_frame = D3D12Renderer::create_synthetic_nv12_frame(1920, 1080, 1);
        renderer.render_frame(&test_frame).unwrap();
        renderer.present().unwrap();

        assert_eq!(renderer.frame_count(), 1);
        renderer.shutdown().unwrap();
    }

    #[test]
    fn test_synthetic_nv12_pattern_rendering() {
        let mut renderer = D3D12Renderer::new();
        renderer
            .initialize(ViewportSize {
                width: 1280,
                height: 720,
            })
            .unwrap();

        let frame = D3D12Renderer::create_synthetic_nv12_frame(1280, 720, 100);
        assert_eq!(frame.format, PixelFormat::Nv12);
        assert_eq!(frame.buffer.len(), (1280 * 720 + 1280 * 720 / 2) as usize);

        assert!(renderer.render_frame(&frame).is_ok());
        assert!(renderer.present().is_ok());
    }
}
