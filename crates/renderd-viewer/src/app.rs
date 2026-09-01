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
use crate::network::{DatagramReceiver, ViewerControlClient};
use crate::platform::init_platform;
use crate::renderer::{Renderer, SoftRenderer, ViewportSize};
use crate::state::AppState;
use crate::ui::SystemTrayManager;
use crate::window::WindowSystem;

/// Delay before the first reconnect attempt after a host stream ends.
const INITIAL_RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// Upper bound on the exponential reconnect backoff.
const MAX_RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);

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
        #[cfg(target_os = "macos")]
        let decoder: Box<dyn Decoder> = Box::new(crate::decode::VideoToolboxDecoder::new());
        #[cfg(target_os = "windows")]
        let decoder: Box<dyn Decoder> = match config.decoder_backend {
            crate::cli::DecoderBackend::Mf => {
                Box::new(crate::decode::MediaFoundationDecoder::new())
            }
            crate::cli::DecoderBackend::D3d12 => Box::new(crate::decode::D3D12Decoder::new()),
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let decoder: Box<dyn Decoder> = Box::new(NullDecoder::new());

        Self {
            config,
            state: AppState::new(),
            window_system: None,
            renderer: Box::new(SoftRenderer::new()),
            decoder,
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
    #[allow(clippy::too_many_lines)]
    pub fn run(mut self) -> Result<(), ViewerError> {
        init_platform()?;

        // ----------------------------------------------------------------
        // Issue #102 & #109: Start platform mDNS browser, wire discovered hosts,
        // and connect QUIC Stream 0 + Datagram Receiver into FrameQueue & Renderer.
        // ----------------------------------------------------------------
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| ViewerError::Window(format!("Failed to create tokio runtime: {e}")))?;

        let discovery = self.discovery.clone();

        // An explicit --host wins outright: it is the path that works when the two
        // machines cannot see each other's multicast traffic. Otherwise browse mDNS,
        // and fall back to loopback only so a single-machine smoke test still works.
        let manual_host = self.config.manual_host;
        rt.block_on(async {
            if let Some(addr) = manual_host {
                if let Err(e) = discovery.add_manual(addr, "Command-line host") {
                    tracing::error!("Failed to register --host {addr}: {e}");
                } else {
                    tracing::info!(host_addr = %addr, "Using host address from --host");
                }
                return;
            }

            match discovery.start_platform_browse() {
                Ok(()) => {
                    tracing::info!(
                        "mDNS browser started — listening for _renderd._udp.local. services"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "mDNS browser unavailable ({e}); falling back to loopback. \
                         Pass --host <address> to reach a host on another machine."
                    );
                    let addr: SocketAddr =
                        "127.0.0.1:4433".parse().expect("hardcoded addr is valid");
                    if let Err(e2) = discovery.add_manual(addr, "Loopback fallback") {
                        tracing::warn!("Loopback fallback also failed: {e2}");
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

        // Spawn background task to connect to host and receive video datagrams into FrameQueue (#109)
        let frame_queue = self.frame_queue.clone();
        let discovery_conn = self.discovery.clone();
        let viewer_id = uuid::Uuid::new_v4();
        let offered_codecs = self.config.codec_choice.codecs();

        // Hand the app's decoder to the receive task rather than constructing a second
        // one there. Two decoders meant a whole extra hardware decode device was created
        // and initialized but never fed, and `with_decoder` had no effect on the pipeline.
        let mut decoder = std::mem::replace(&mut self.decoder, Box::new(NullDecoder::new()));

        rt.spawn(async move {
            let control_client = ViewerControlClient::new(viewer_id);
            let mut backoff = INITIAL_RECONNECT_BACKOFF;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                if let Some(target_addr) = discovery_conn.snapshot().primary_addr() {
                    tracing::info!(host_addr = %target_addr, "Connecting to discovered host...");

                    let tls_config = match renderd_net::ClientTlsConfig::with_insecure_skip_verify() {
                        Ok(cfg) => cfg,
                        Err(e) => {
                            tracing::warn!("Failed to create ClientTlsConfig: {e}");
                            continue;
                        }
                    };

                    let client = match renderd_net::QuicClient::bind_ephemeral() {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!("Failed to bind QuicClient: {e}");
                            continue;
                        }
                    };

                    let conn = match client.connect(target_addr, "renderd-host", tls_config).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!("QUIC connection to {target_addr} failed: {e}");
                            continue;
                        }
                    };

                    tracing::info!(peer = %conn.remote_address(), "QUIC connection established with host");

                    let display = renderd_proto::generated::renderd::DisplayInfo {
                        width: 1920,
                        height: 1080,
                        refresh_rate: 60.0,
                        vrr_supported: false,
                    };

                    match control_client
                        .negotiate(&conn, display, offered_codecs.clone(), 50_000, true)
                        .await
                    {
                        Ok((_hello, session_config, mut send_stream, mut _recv_stream)) => {
                            tracing::info!(
                                codec = %session_config.selected_codec,
                                width = session_config.width,
                                height = session_config.height,
                                fps = session_config.frame_rate,
                                "Stream 0 handshake completed with host — starting datagram receiver and vsync reporter"
                            );

                            // Spawn VsyncReporter & FeedbackExporter task to send VsyncReport, ReactiveStats, and PeriodicStats over Stream 0 (#110, #111)
                            tokio::spawn(async move {
                                use renderd_net::framing::send_control;
                                use renderd_proto::generated::renderd::{envelope::Payload, Envelope};
                                let mut vsync_reporter = crate::clock_sync::VsyncReporter::new();
                                let mut feedback_exporter = crate::abr::FeedbackExporter::new();
                                loop {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(16)).await;
                                    let report = vsync_reporter.create_vsync_report();
                                    let env = Envelope {
                                        payload: Some(Payload::VsyncReport(report)),
                                    };
                                    if send_control(&mut send_stream, &env).await.is_err() {
                                        break;
                                    }

                                    if let Some(reactive) = feedback_exporter.maybe_export_reactive() {
                                        let env = Envelope {
                                            payload: Some(Payload::ReactiveStats(reactive)),
                                        };
                                        if send_control(&mut send_stream, &env).await.is_err() {
                                            break;
                                        }
                                    }

                                    if let Some(periodic) = feedback_exporter.maybe_export_periodic() {
                                        let env = Envelope {
                                            payload: Some(Payload::PeriodicStats(periodic)),
                                        };
                                        if send_control(&mut send_stream, &env).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            });

                            backoff = INITIAL_RECONNECT_BACKOFF;

                            let mut receiver = DatagramReceiver::new(4);

                            // Discard any state left by a previous session before
                            // re-initializing for the newly negotiated stream.
                            if let Err(e) = decoder.reset() {
                                tracing::warn!("Decoder reset error: {e}");
                            }
                            if let Err(e) = decoder.initialize(&session_config.selected_codec, session_config.width, session_config.height) {
                                tracing::warn!("Decoder initialization error: {e}");
                            }

                            if let Err(e) = receiver.run_receive_loop(&conn, decoder.as_mut(), &frame_queue).await {
                                tracing::warn!("Datagram receiver loop ended: {e}");
                            }

                            tracing::info!(
                                peer = %conn.remote_address(),
                                "Host stream ended — clearing stale frames and re-discovering"
                            );
                            frame_queue.clear();
                        }
                        Err(e) => {
                            tracing::warn!("Stream 0 negotiation failed: {e}");
                        }
                    }

                    // Fall through to the next iteration rather than breaking: the loop
                    // is the viewer's reconnect path, and exiting it left the viewer dead
                    // for the rest of the process lifetime after any disconnect.
                    backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
                    tokio::time::sleep(backoff).await;
                } else {
                    backoff = INITIAL_RECONNECT_BACKOFF;
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
                    if let Err(e) = self.renderer.attach_window(ws.window().clone()) {
                        tracing::error!("Failed to attach window to renderer: {e}");
                    }
                    if let Err(e) = self.renderer.initialize(viewport) {
                        tracing::error!("Failed to initialize renderer: {e}");
                    } else {
                        tracing::info!(
                            width = viewport.width,
                            height = viewport.height,
                            "Renderer initialized successfully"
                        );
                    }
                    // The decoder is owned by the receive task and initialized from the
                    // codec and dimensions the host negotiates, not from the window size.
                    self.window_system = Some(ws);
                }
                Err(e) => {
                    tracing::error!("Failed to create window system: {e}");
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(ref ws) = self.window_system {
            ws.window().request_redraw();
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
                    tracing::error!("Error resizing renderer: {e}");
                }
            }
            WindowEvent::RedrawRequested if self.window_system.is_some() => {
                static RENDER_COUNT: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                static INTERVAL_START: std::sync::Mutex<Option<std::time::Instant>> =
                    std::sync::Mutex::new(None);
                static INTERVAL_PRESENTED: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                static INTERVAL_STALE: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);

                let (maybe_frame, stale_count) = self.frame_queue.pop_latest();
                if stale_count > 0 {
                    INTERVAL_STALE
                        .fetch_add(stale_count as u64, std::sync::atomic::Ordering::Relaxed);
                }

                if let Some(frame) = maybe_frame {
                    let count = RENDER_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    INTERVAL_PRESENTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if count == 1 {
                        tracing::info!(
                            count = count,
                            frame_id = frame.frame_id,
                            width = frame.width,
                            height = frame.height,
                            "Renderer: popped first decoded frame from FrameQueue and presenting to swapchain"
                        );
                    }

                    let render_start = std::time::Instant::now();
                    if let Err(e) = self.renderer.render_frame(&frame) {
                        tracing::error!("Error rendering frame: {e}");
                    }
                    if let Err(e) = self.renderer.present() {
                        tracing::error!("Error presenting frame: {e}");
                    }
                    let render_duration = render_start.elapsed();

                    let mut start_guard = INTERVAL_START.lock().unwrap();
                    let start = start_guard.get_or_insert_with(std::time::Instant::now);
                    let elapsed = start.elapsed();
                    if elapsed >= std::time::Duration::from_secs(1) {
                        let elapsed_sec = elapsed.as_secs_f64();
                        let pres = INTERVAL_PRESENTED.swap(0, std::sync::atomic::Ordering::Relaxed);
                        let stale = INTERVAL_STALE.swap(0, std::sync::atomic::Ordering::Relaxed);
                        #[allow(clippy::cast_precision_loss)]
                        let pres_fps = (pres as f64) / elapsed_sec;
                        #[allow(clippy::cast_precision_loss)]
                        let stale_fps = (stale as f64) / elapsed_sec;

                        tracing::info!(
                            present_fps = format!("{pres_fps:.1}"),
                            stale_drop_fps = format!("{stale_fps:.1}"),
                            frame_id = frame.frame_id,
                            decode_ms =
                                format!("{:.2}", frame.decode_duration.as_secs_f64() * 1000.0),
                            render_ms = format!("{:.2}", render_duration.as_secs_f64() * 1000.0),
                            total_presented = count,
                            total_stale_dropped = self.frame_queue.stale_dropped(),
                            "VIEWER METRICS: display presentation & render latency"
                        );
                        *start_guard = Some(std::time::Instant::now());
                    }
                }
            }
            _ => {}
        }
    }
}
