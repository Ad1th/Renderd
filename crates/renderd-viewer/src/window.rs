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
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn new(
        event_loop: &ActiveEventLoop,
        title: &str,
        width: u32,
        height: u32,
        fullscreen: bool,
    ) -> Result<Self, ViewerError> {
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next());

        let (init_width, init_height) = if fullscreen {
            (width, height)
        } else {
            monitor.as_ref().map_or((width, height), |mon| {
                let mon_size = mon.size();
                // Ensure the window fits within 88% of the monitor work area to account for taskbar and decorations
                let max_w = (f64::from(mon_size.width) * 0.88).round() as u32;
                let max_h = (f64::from(mon_size.height) * 0.88).round() as u32;

                if width > max_w || height > max_h {
                    let scale_w = f64::from(max_w) / f64::from(width.max(1));
                    let scale_h = f64::from(max_h) / f64::from(height.max(1));
                    let scale = scale_w.min(scale_h);
                    let w = ((f64::from(width) * scale).round() as u32).max(640);
                    let h = ((f64::from(height) * scale).round() as u32).max(480);
                    (w, h)
                } else {
                    (width, height)
                }
            })
        };

        let size = PhysicalSize::new(init_width, init_height);
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
        } else if let Some(ref mon) = monitor {
            // Explicitly center window on the active monitor work area
            let mon_pos = mon.position();
            let mon_size = mon.size();
            let actual_size = window.outer_size();
            let center_x = mon_pos.x
                + (i32::try_from(mon_size.width).unwrap_or(0)
                    - i32::try_from(actual_size.width).unwrap_or(0))
                    / 2;
            let center_y = mon_pos.y
                + (i32::try_from(mon_size.height).unwrap_or(0)
                    - i32::try_from(actual_size.height).unwrap_or(0))
                    / 2;
            window.set_outer_position(winit::dpi::PhysicalPosition::new(
                center_x.max(mon_pos.x),
                center_y.max(mon_pos.y),
            ));
        }

        Ok(Self { window, fullscreen })
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
            self.window
                .set_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            self.window.set_fullscreen(None);
        }
    }

    /// Requests a window redraw.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
}
