//! Window management and display subsystem wrapping `winit`.

use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Fullscreen, Window, WindowAttributes};

use crate::error::ViewerError;
use crate::renderer::ViewportSize;

/// Application window wrapper providing resize, DPI, and fullscreen controls.
#[derive(Debug)]
pub struct WindowSystem {
    window: Arc<Window>,
    fullscreen: bool,
}

impl WindowSystem {
    /// Creates a new [`WindowSystem`] within the given `winit` active event loop.
    ///
    /// # Errors
    /// Returns [`ViewerError::Window`] if window creation fails.
    pub fn new(
        event_loop: &ActiveEventLoop,
        title: &str,
        width: u32,
        height: u32,
        fullscreen: bool,
    ) -> Result<Self, ViewerError> {
        let size = PhysicalSize::new(width, height);
        let attributes = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(size)
            .with_min_inner_size(PhysicalSize::new(640, 480));

        let window = event_loop
            .create_window(attributes)
            .map_err(|e| ViewerError::Window(format!("Failed to create window: {e}")))?;

        let window = Arc::new(window);

        if fullscreen {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        Ok(Self {
            window,
            fullscreen,
        })
    }

    /// Returns a reference to the inner `winit` [`Window`].
    #[must_use]
    pub const fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// Returns the current inner viewport size in physical pixels.
    #[must_use]
    pub fn viewport_size(&self) -> ViewportSize {
        let size = self.window.inner_size();
        ViewportSize {
            width: size.width,
            height: size.height,
        }
    }

    /// Checks if the window is currently in fullscreen mode.
    #[must_use]
    pub const fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Toggles borderless fullscreen display mode.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            self.window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            self.window.set_fullscreen(None);
        }
    }

    /// Requests a window redraw.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
}
