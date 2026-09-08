//! Application lifecycle and winit event loop handler.
//!
//! # Redraw model
//!
//! The viewer presents exactly when there is something new to show. A decoded
//! frame arriving on the receive task sends a [`WakeReason::Frame`] through the
//! event-loop proxy, which is the only thing that schedules a redraw; between
//! frames the event loop sleeps in `ControlFlow::Wait`.
//!
//! The previous version called `request_redraw()` unconditionally from
//! `about_to_wait`, which is invoked every time the loop wakes for any reason. On
//! a machine with nothing else to do that is a spin loop: it pinned a CPU core at
//! 100%, starved the decode and network tasks, and was the main reason scrolling
//! looked like a slideshow.

use std::net::SocketAddr;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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

/// Messages the background tasks send to wake the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    /// A decoded frame was pushed into the queue; present it.
    Frame,
    /// The connection state changed; refresh the status overlay.
    ConnectionChanged,
}

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
            frame_queue: Arc::new(FrameQueue::new(3)),
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

        let event_loop = EventLoop::<WakeReason>::with_user_event()
            .build()
            .map_err(|e| ViewerError::Window(format!("Failed to create event loop: {e}")))?;
        let proxy = event_loop.create_proxy();

        // ----------------------------------------------------------------
        // Issue #102 & #109: Start platform mDNS browser, wire discovered hosts,
        // and connect QUIC Stream 0 + Datagram Receiver into FrameQueue & Renderer.
        // ----------------------------------------------------------------
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
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

        // Watch for newly discovered hosts and keep the tray target current.
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
                        tracing::info!(host_addr = %addr, "Discovery: primary host target updated");
                        tray_watch.set_host_address(addr);
                    }
                }
            }
        });

        // Connect to the host and pump video datagrams into the FrameQueue.
        let frame_queue = self.frame_queue.clone();
        let discovery_conn = self.discovery.clone();
        let state_conn = self.state.clone();
        let viewer_id = uuid::Uuid::new_v4();
        let offered_codecs = self.config.codec_choice.codecs();

        // Hand the app's decoder to the receive task rather than constructing a second
        // one there. Two decoders meant a whole extra hardware decode device was created
        // and initialized but never fed, and `with_decoder` had no effect on the pipeline.
        let mut decoder = std::mem::replace(&mut self.decoder, Box::new(NullDecoder::new()));
        let frame_proxy = proxy;

        rt.spawn(async move {
            let control_client = ViewerControlClient::new(viewer_id);
            let mut backoff = INITIAL_RECONNECT_BACKOFF;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                let Some(target_addr) = discovery_conn.snapshot().primary_addr() else {
                    backoff = INITIAL_RECONNECT_BACKOFF;
                    continue;
                };

                state_conn.set_connection_state(crate::state::ConnectionState::Handshaking);
                let _ = frame_proxy.send_event(WakeReason::ConnectionChanged);
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

                let conn = match client
                    .connect(target_addr, "renderd-host", tls_config)
                    .await
                {
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
                            "Stream 0 handshake completed with host"
                        );

                        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<u64>(16);

                        // VsyncReporter & FeedbackExporter task (#110, #111).
                        tokio::spawn(async move {
                            use renderd_net::framing::send_control;
                            use renderd_proto::generated::renderd::{envelope::Payload, Envelope};
                            const KF_DEBOUNCE: std::time::Duration =
                                std::time::Duration::from_millis(500);

                            let mut vsync_reporter = crate::clock_sync::VsyncReporter::new();
                            let mut feedback_exporter = crate::abr::FeedbackExporter::new();
                            // The host now throttles its own capture reconfiguration, so
                            // there is no reason to shout vsync reports at it 60 times a
                            // second; a report every ~100 ms is plenty for phase tracking
                            // and keeps the control stream almost silent.
                            let mut interval = tokio::time::interval(
                                tokio::time::Duration::from_millis(100),
                            );
                            let mut last_kf_req = std::time::Instant::now()
                                .checked_sub(std::time::Duration::from_secs(5))
                                .unwrap_or_else(std::time::Instant::now);

                            loop {
                                tokio::select! {
                                    _ = interval.tick() => {
                                        let report = vsync_reporter.create_vsync_report();
                                        let env = Envelope {
                                            payload: Some(Payload::VsyncReport(report)),
                                        };
                                        if send_control(&mut send_stream, &env).await.is_err() {
                                            break;
                                        }

                                        if let Some(reactive) =
                                            feedback_exporter.maybe_export_reactive()
                                        {
                                            let env = Envelope {
                                                payload: Some(Payload::ReactiveStats(reactive)),
                                            };
                                            if send_control(&mut send_stream, &env).await.is_err() {
                                                break;
                                            }
                                        }

                                        if let Some(periodic) =
                                            feedback_exporter.maybe_export_periodic()
                                        {
                                            let env = Envelope {
                                                payload: Some(Payload::PeriodicStats(periodic)),
                                            };
                                            if send_control(&mut send_stream, &env).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                    Some(loss_count) = loss_rx.recv() => {
                                        feedback_exporter.record_frame_loss(loss_count.max(1));
                                        if last_kf_req.elapsed() >= KF_DEBOUNCE {
                                            last_kf_req = std::time::Instant::now();
                                            let kf_req = feedback_exporter.create_keyframe_request();
                                            let env = Envelope {
                                                payload: Some(Payload::KeyframeRequest(kf_req)),
                                            };
                                            if send_control(&mut send_stream, &env).await.is_err() {
                                                break;
                                            }
                                            tracing::info!(
                                                "Sent KeyframeRequest over Stream 0 due to frame loss"
                                            );
                                        }
                                    }
                                }
                            }
                        });

                        backoff = INITIAL_RECONNECT_BACKOFF;
                        state_conn.set_connection_state(crate::state::ConnectionState::Connected);
                        let _ = frame_proxy.send_event(WakeReason::ConnectionChanged);

                        let mut receiver = DatagramReceiver::new(4);

                        if let Err(e) = decoder.reset() {
                            tracing::warn!("Decoder reset error: {e}");
                        }
                        if let Err(e) = decoder.initialize(
                            &session_config.selected_codec,
                            session_config.width,
                            session_config.height,
                        ) {
                            tracing::warn!("Decoder initialization error: {e}");
                        }

                        let wake_proxy = frame_proxy.clone();
                        let on_frame = move || {
                            let _ = wake_proxy.send_event(WakeReason::Frame);
                        };
                        if let Err(e) = receiver
                            .run_receive_loop_with_wake(
                                &conn,
                                decoder.as_mut(),
                                &frame_queue,
                                Some(loss_tx),
                                &on_frame,
                            )
                            .await
                        {
                            tracing::warn!("Datagram receiver loop ended: {e}");
                        }

                        tracing::info!(
                            peer = %conn.remote_address(),
                            "Host stream ended — clearing stale frames and re-discovering"
                        );
                        frame_queue.clear();
                        state_conn
                            .set_connection_state(crate::state::ConnectionState::Reconnecting);
                        let _ = frame_proxy.send_event(WakeReason::ConnectionChanged);
                    }
                    Err(e) => {
                        tracing::warn!("Stream 0 negotiation failed: {e}");
                    }
                }

                backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
                tokio::time::sleep(backoff).await;
            }
        });

        event_loop
            .run_app(&mut self)
            .map_err(|e| ViewerError::Window(format!("Event loop failure: {e}")))?;

        rt.shutdown_background();
        Ok(())
    }

    /// Pops the freshest decoded frame and presents it, dropping any stale backlog.
    fn present_next_frame(&mut self) {
        static PRESENTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static WINDOW_PRESENTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static WINDOW_START: std::sync::Mutex<Option<std::time::Instant>> =
            std::sync::Mutex::new(None);

        let (Some(frame), _stale) = self.frame_queue.pop_latest() else {
            return;
        };

        let render_start = std::time::Instant::now();
        if let Err(e) = self.renderer.render_frame(&frame) {
            tracing::error!("Error rendering frame: {e}");
            return;
        }
        if let Err(e) = self.renderer.present() {
            tracing::error!("Error presenting frame: {e}");
            return;
        }
        let count = PRESENTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        WINDOW_PRESENTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut guard) = WINDOW_START.lock() {
            let start = guard.get_or_insert_with(std::time::Instant::now);
            let elapsed = start.elapsed();
            if elapsed >= std::time::Duration::from_secs(5) {
                let window = WINDOW_PRESENTED.swap(0, std::sync::atomic::Ordering::Relaxed);
                #[allow(clippy::cast_precision_loss)]
                let fps = window as f64 / elapsed.as_secs_f64();
                tracing::info!(
                    present_fps = format!("{fps:.1}"),
                    total_presented = count,
                    stale_dropped = self.frame_queue.stale_dropped(),
                    last_frame_id = frame.frame_id,
                    decode_ms =
                        format!("{:.2}", frame.decode_duration.as_secs_f64() * 1000.0),
                    render_ms =
                        format!("{:.2}", render_start.elapsed().as_secs_f64() * 1000.0),
                    "VIEWER METRICS: presentation"
                );
                *guard = Some(std::time::Instant::now());
            }
        }
    }
}

impl ApplicationHandler<WakeReason> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window_system.is_some() {
            return;
        }
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
                self.window_system = Some(ws);
            }
            Err(e) => {
                tracing::error!("Failed to create window system: {e}");
                event_loop.exit();
            }
        }
    }

    /// Sleep until something actually happens. A decoded frame, an OS window event,
    /// or a reconnect all wake the loop; nothing here schedules work on its own.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: WakeReason) {
        match event {
            WakeReason::Frame => {
                if let Some(ref ws) = self.window_system {
                    ws.window().request_redraw();
                }
            }
            WakeReason::ConnectionChanged => {
                tracing::debug!(state = ?self.state.connection_state(), "connection state changed");
                if let Some(ref ws) = self.window_system {
                    ws.window().request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
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
                if let Some(ref ws) = self.window_system {
                    ws.window().request_redraw();
                }
            }
            WindowEvent::RedrawRequested if self.window_system.is_some() => {
                self.present_next_frame();
            }
            _ => {}
        }
    }
}
