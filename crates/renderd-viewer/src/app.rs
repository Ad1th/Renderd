//! Application lifecycle and winit event loop handler.

use std::net::SocketAddr;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

use crate::config::ViewerAppConfig;
use crate::decoder::{Decoder, NullDecoder};
use crate::discovery::DiscoveryManager;
use crate::error::ViewerError;
use crate::frame_queue::FrameQueue;
use crate::platform::init_platform;
use crate::renderer::{NullRenderer, Renderer, ViewportSize};
use crate::state::AppState;
use crate::ui::SystemTrayManager;
use crate::window::WindowSystem;

/// Main application orchestrator managing lifecycle, windowing, rendering, and decoding.
pub struct App {
    config: ViewerAppConfig,
    state: AppState,
    window_system: Option<WindowSystem>,
    renderer: Box<dyn Renderer>,
    decoder: Box<dyn Decoder>,
    frame_queue: Arc<FrameQueue>,
    discovery: DiscoveryManager,
    tray: SystemTrayManager,
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
            discovery: DiscoveryManager::new(),
            tray: SystemTrayManager::new(),
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
    ///
    /// # Panics
    /// Panics if the tokio runtime cannot be built (OS resource exhaustion).
    pub fn run(mut self) -> Result<(), ViewerError> {
        init_platform()?;

        // ----------------------------------------------------------------
        // Issue #102: Start platform mDNS browser (or fall back to manual)
        // and wire discovered hosts into the system tray UI.
        // ----------------------------------------------------------------
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| ViewerError::Window(format!("Failed to create tokio runtime: {e}")))?;

        let discovery = self.discovery.clone();

        // Start the platform mDNS browser; on failure fall back to the
        // configured manual host address from the viewer config.
        rt.block_on(async {
            match discovery.start_platform_browse() {
                Ok(()) => {
                    tracing::info!(
                        "mDNS browser started — listening for _renderd._udp.local. services"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "mDNS browser unavailable ({e}); activating ManualBrowser fallback"
                    );
                    let fallback_addr = self.config.config.network.listen_port;
                    let addr: SocketAddr = format!("127.0.0.1:{fallback_addr}")
                        .parse()
                        .unwrap_or_else(|_| {
                            "127.0.0.1:4433".parse().expect("hardcoded addr is valid")
                        });
                    if let Err(e2) = discovery.add_manual(addr, "Manual Fallback") {
                        tracing::warn!("ManualBrowser fallback also failed: {e2}");
                    }
                }
            }
        });

        // Spawn a background task that watches for new discovery events and
        // updates the system tray host address whenever a new host appears.
        let discovery_watch = self.discovery.clone();
        let tray_watch = self.tray.clone();
        rt.spawn(async move {
            let mut last_count = 0usize;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                let snap = discovery_watch.snapshot();
                let count = snap.hosts.len();
                if count != last_count {
                    last_count = count;
                    if let Some(addr) = snap.primary_addr() {
                        tracing::info!(
                            host_addr = %addr,
                            "Discovery: primary host target updated in system tray"
                        );
                        tray_watch.set_host_address(addr);
                    }
                }
            }
        });

        let event_loop = EventLoop::new()
            .map_err(|e| ViewerError::Window(format!("Failed to create event loop: {e}")))?;

        event_loop
            .run_app(&mut self)
            .map_err(|e| ViewerError::Window(format!("Event loop failure: {e}")))?;

        // Shutdown the tokio runtime cleanly when the event loop exits.
        rt.shutdown_background();

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
