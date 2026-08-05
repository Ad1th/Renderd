//! Graphics renderer abstraction for swapchain management and frame presentation.

use crate::decoder::DecodedFrame;
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
}
