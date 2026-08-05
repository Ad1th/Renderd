//! Application lifecycle and winit event loop handler.

use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

use crate::config::ViewerAppConfig;
use crate::decoder::{Decoder, NullDecoder};
use crate::error::ViewerError;
use crate::frame_queue::FrameQueue;
use crate::platform::init_platform;
use crate::renderer::{NullRenderer, Renderer, ViewportSize};
use crate::state::AppState;
use crate::window::WindowSystem;

/// Main application orchestrator managing lifecycle, windowing, rendering, and decoding.
pub struct App {
    config: ViewerAppConfig,
    state: AppState,
    window_system: Option<WindowSystem>,
    renderer: Box<dyn Renderer>,
    decoder: Box<dyn Decoder>,
    frame_queue: Arc<FrameQueue>,
}

impl App {
    /// Creates a new [`App`] instance with the provided configuration and default null engines.
    #[must_use]
    pub fn new(config: ViewerAppConfig) -> Self {
        Self {
            config,
            state: AppState::new(),
            window_system: None,
            renderer: Box::new(NullRenderer::new()),
            decoder: Box::new(NullDecoder::new()),
            frame_queue: Arc::new(FrameQueue::new(4)),
        }
    }

    /// Sets a custom graphics renderer implementation.
    #[must_use]
    pub fn with_renderer(mut self, renderer: Box<dyn Renderer>) -> Self {
        self.renderer = renderer;
        self
    }

    /// Sets a custom video decoder implementation.
    #[must_use]
    pub fn with_decoder(mut self, decoder: Box<dyn Decoder>) -> Self {
        self.decoder = decoder;
        self
    }

    /// Returns a reference to the shared [`AppState`].
    #[must_use]
    pub const fn state(&self) -> &AppState {
        &self.state
    }

    /// Returns a reference to the shared [`FrameQueue`].
    #[must_use]
    pub const fn frame_queue(&self) -> &Arc<FrameQueue> {
        &self.frame_queue
    }

    /// Runs the application main event loop.
    ///
    /// # Errors
    /// Returns [`ViewerError`] if event loop execution fails.
    pub fn run(mut self) -> Result<(), ViewerError> {
        init_platform()?;

        let event_loop = EventLoop::new()
            .map_err(|e| ViewerError::Window(format!("Failed to create event loop: {e}")))?;

        event_loop
            .run_app(&mut self)
            .map_err(|e| ViewerError::Window(format!("Event loop failure: {e}")))?;

        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window_system.is_none() {
            tracing::info!("Initializing viewer window and renderer...");
            match WindowSystem::new(
                event_loop,
                &self.config.window_title,
                self.config.window_width,
                self.config.window_height,
                self.config.fullscreen,
            ) {
                Ok(ws) => {
                    let viewport = ws.viewport_size();
                    if let Err(e) = self.renderer.initialize(viewport) {
                        tracing::error!("Failed to initialize renderer: {e}");
                    }
                    if let Err(e) = self
                        .decoder
                        .initialize("hevc", viewport.width, viewport.height)
                    {
                        tracing::error!("Failed to initialize decoder: {e}");
                    }
                    self.window_system = Some(ws);
                }
                Err(e) => {
                    tracing::error!("Failed to create window system: {e}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("Close requested by user; shutting down viewer app");
                self.state.stop();
                if let Err(e) = self.renderer.shutdown() {
                    tracing::warn!("Error shutting down renderer: {e}");
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                let viewport = ViewportSize {
                    width: size.width,
                    height: size.height,
                };
                if let Err(e) = self.renderer.resize(viewport) {
                    tracing::error!("Failed to resize renderer: {e}");
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(frame) = self.frame_queue.pop() {
                    if let Err(e) = self.renderer.render_frame(&frame) {
                        tracing::error!("Error rendering frame: {e}");
                    }
                }
                if let Err(e) = self.renderer.present() {
                    tracing::error!("Error presenting frame: {e}");
                }
            }
            _ => {}
        }
    }
}
