//! Host application state machine and run loop.
//!
//! `HostApp` initializes and orchestrates all host subsystems:
//! - Screen capture pipeline (`CapturePipeline`)
//! - Hardware video encoder (`EncodePipeline`)
//! - Presentation clock controller (`ClockController`)
//! - Adaptive bitrate controller (`AbrManager`)
//! - Session lifecycle manager (`HostSession`)
//! - Network manager (`NetworkManager`)
//! - UI manager (`UiManager`)
//! - QUIC server listener (`QuicServer`)
//! - mDNS Bonjour advertiser (`BonjourAdvertiser`)
//!
//! The run loop blocks until the process receives SIGINT or SIGTERM.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use rcgen::generate_simple_self_signed;
use renderd_config::RenderdConfig;
use renderd_discovery::{Advertiser, BonjourAdvertiser, ServiceRecord};
use renderd_net::{QuicServer, ServerTlsConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use uuid::Uuid;

use crate::abr::AbrManager;
use crate::capture::CapturePipeline;
use crate::clock::ClockController;
use crate::encode::EncodePipeline;
use crate::error::HostError;
use crate::network::{ControlDispatcher, DataSender, NetworkManager};
use crate::session::HostSession;
use crate::ui::UiManager;

/// Main host application orchestrator.
///
/// Owns all long-lived subsystem components and drives the run loop until shutdown.
#[derive(Debug)]
pub struct HostApp {
    config: RenderdConfig,
    capture: Arc<Mutex<CapturePipeline>>,
    encode: Arc<EncodePipeline>,
    clock: ClockController,
    abr: AbrManager,
    session: HostSession,
    network: NetworkManager,
    ui: UiManager,
}

impl HostApp {
    /// Creates a new `HostApp` with all subsystems in their default initialized states,
    /// using the provided [`RenderdConfig`].
    #[must_use]
    pub fn new(config: RenderdConfig) -> Self {
        Self {
            config,
            capture: Arc::new(Mutex::new(CapturePipeline::new())),
            encode: Arc::new(EncodePipeline::new()),
            clock: ClockController::new(),
            abr: AbrManager::new(),
            session: HostSession::new(),
            network: NetworkManager::new(),
            ui: UiManager::new(),
        }
    }

    /// Runs the host application run loop.
    ///
    /// Initializes all subsystems, spawns the QUIC server listener on the configured
    /// port, registers the mDNS `_renderd._udp.local.` service advertisement, and
    /// blocks until the process receives SIGINT or SIGTERM.
    ///
    /// On shutdown, unregisters the mDNS advertisement and closes the QUIC socket
    /// cleanly.
    ///
    /// # Errors
    ///
    /// Returns a [`HostError`] if any subsystem fails to initialize or the session
    /// is in an unexpected state on entry.
    ///
    /// # Panics
    ///
    /// Panics if the tokio runtime fails to initialize or a mutex is poisoned.
    #[allow(clippy::too_many_lines)]
    pub fn run(&mut self) -> Result<(), HostError> {
        tracing::info!("renderd-host starting subsystem initialization");

        // Verify session starts in Idle — fail fast if state is corrupted.
        if !self.session.is_idle() {
            return Err(HostError::Initialization(format!(
                "HostSession expected Idle on startup, got {}",
                self.session.state()
            )));
        }

        // Log capture pipeline status (real start is deferred until a viewer connects)
        let capture_is_running = self
            .capture
            .lock()
            .expect("CapturePipeline mutex poisoned")
            .is_running();
        tracing::info!(
            capture_running = capture_is_running,
            "Capture pipeline ready (starts on first viewer connection)"
        );

        // Confirm encode pipeline is ready (receiver handle proves the ring buffer is live)
        let _encode_rx = self.encode.receiver();
        tracing::info!("Encode pipeline ready");

        // Log initial clock controller target interval (60 Hz default)
        let clock_interval = self.clock.target_interval();
        tracing::info!(
            target_interval_ns = clock_interval.as_nanos(),
            "Presentation clock controller initialized"
        );

        // Log initial ABR target bitrate
        let initial_bitrate = self.abr.current_bitrate();
        tracing::info!(
            initial_bitrate_kbps = initial_bitrate.0,
            "ABR controller initialized"
        );

        // Confirm network manager is ready
        tracing::info!(
            network_manager = ?self.network,
            "Network manager initialized"
        );

        // Confirm UI manager is ready and post startup notification
        self.ui.notifications.notify_session_started("renderd-host");
        tracing::info!("UI manager initialized (menu bar and notifications)");

        tracing::info!(
            session_state = %self.session.state(),
            "Session state machine ready — host is listening for viewer connections"
        );

        // ----------------------------------------------------------------
        // Issue #101, #105, #106: Spawn QUIC server, accept loop, mDNS advert,
        // and wire ScreenCaptureKit capture to VideoToolbox encoder.
        // ----------------------------------------------------------------

        // Generate a self-signed TLS certificate for this host's QUIC endpoint.
        let cert_gen =
            generate_simple_self_signed(vec!["renderd-host".to_string()]).map_err(|e| {
                HostError::Initialization(format!("Failed to generate self-signed cert: {e}"))
            })?;
        let cert_der = CertificateDer::from(cert_gen.cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());

        let tls_config = ServerTlsConfig::from_cert(vec![cert_der], key_der, None)
            .map_err(|e| HostError::Initialization(format!("TLS configuration failed: {e}")))?;

        // Bind QUIC server on the configured address and port.
        let listen_port = self.config.network.listen_port;
        let bind_addr: SocketAddr = format!("{}:{listen_port}", self.config.network.bind_address)
            .parse()
            .map_err(|e| {
                HostError::Initialization(format!(
                    "Invalid bind address '{}:{}': {e}",
                    self.config.network.bind_address, listen_port
                ))
            })?;

        let quic_server = QuicServer::bind(bind_addr, tls_config).map_err(|e| {
            HostError::Initialization(format!("Failed to bind QUIC server on {bind_addr}: {e}"))
        })?;

        let actual_addr = quic_server.local_addr().map_err(|e| {
            HostError::Initialization(format!("Failed to query QUIC server address: {e}"))
        })?;

        tracing::info!(
            listen_addr = %actual_addr,
            "QUIC server endpoint listening for incoming viewer connections"
        );

        // Register mDNS _renderd._udp.local. service advertisement via BonjourAdvertiser.
        let host_id = Uuid::new_v4();
        let mut txt = HashMap::new();
        txt.insert("width".to_string(), "1920".to_string());
        txt.insert("height".to_string(), "1080".to_string());
        txt.insert("fps".to_string(), self.config.host.target_fps.to_string());
        txt.insert("id".to_string(), host_id.to_string());

        let service_record = ServiceRecord {
            host_id,
            name: format!("renderd-{}", &host_id.to_string()[..8]),
            addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: actual_addr.port(),
            txt,
        };

        let mut advertiser = BonjourAdvertiser::new();
        advertiser.register(&service_record).map_err(|e| {
            HostError::Initialization(format!("mDNS service registration failed: {e}"))
        })?;

        tracing::info!(
            host_id = %host_id,
            service_name = %service_record.name,
            port = actual_addr.port(),
            "mDNS service _renderd._udp.local. registered"
        );

        // Build tokio runtime for connection accept loop and control stream processing
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| {
                HostError::Initialization(format!("Failed to build tokio runtime: {e}"))
            })?;

        let session = self.session.clone();
        let host_cfg = self.config.host.clone();
        let menu_bar = self.ui.menu_bar.clone();
        let capture = self.capture.clone();
        let encode = self.encode.clone();
        let clock = self.clock.clone();

        let quic_server = Arc::new(quic_server);
        let quic_server_task = Arc::clone(&quic_server);

        rt.spawn(async move {
            let dispatcher = ControlDispatcher::new();
            while let Ok(conn) = quic_server_task.accept().await {
                let session = session.clone();
                let host_cfg = host_cfg.clone();
                let menu_bar = menu_bar.clone();
                let dispatcher = dispatcher.clone();
                let capture = capture.clone();
                let encode = encode.clone();
                let clock = clock.clone();

                tokio::spawn(async move {
                    let clock = clock.clone();
                    match dispatcher
                        .handle_connection(&conn, &host_cfg, &session)
                        .await
                    {
                        Ok((_hello, cfg, mut _send_stream, mut recv_stream)) => {
                            // Transition session state to STREAMING once connected
                            if let Err(e) = session.begin_streaming() {
                                tracing::warn!(
                                    "Failed to transition session state to STREAMING: {e}"
                                );
                            }

                            menu_bar.update_status(&format!("Streaming ({})", session.state()));

                            // Activate VideoToolbox encoder & ScreenCaptureKit capture pipeline (#106)
                            if let Err(e) =
                                encode.init(cfg.width, cfg.height, cfg.initial_bitrate_kbps)
                            {
                                tracing::warn!("Encode pipeline init failed: {e}");
                            }

                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            let target_fps = cfg.frame_rate as u32;

                            {
                                let mut capture_guard =
                                    capture.lock().expect("CapturePipeline mutex poisoned");
                                if let Err(e) = capture_guard.start(
                                    cfg.width,
                                    cfg.height,
                                    target_fps,
                                    encode.clone(),
                                ) {
                                    tracing::warn!("Capture pipeline start failed: {e}");
                                } else {
                                    tracing::info!(
                                        width = cfg.width,
                                        height = cfg.height,
                                        fps = target_fps,
                                        "ScreenCaptureKit capture and VideoToolbox encoder active"
                                    );
                                }
                            }

                            // Spawn Control stream reader to process VsyncReport & telemetry (#110)
                            let capture_for_ctrl = capture.clone();
                            tokio::spawn(async move {
                                use renderd_net::framing::recv_control;
                                use renderd_proto::generated::renderd::envelope::Payload;
                                while let Ok(envelope) = recv_control(&mut recv_stream).await {
                                    if let Some(Payload::VsyncReport(report)) = envelope.payload {
                                        let capture_guard = capture_for_ctrl
                                            .lock()
                                            .expect("CapturePipeline mutex poisoned");
                                        let _ = clock.on_vsync_report(&report, &capture_guard);
                                    }
                                }
                            });

                            // Spawn DataSender task to transmit encoded ring buffer frames over QUIC datagrams (#107)
                            let data_sender = DataSender::new();
                            let data_conn = conn.clone();
                            let encode_rx = encode.receiver();
                            let shutdown_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
                            tokio::spawn(async move {
                                data_sender
                                    .run_loop(data_conn, encode_rx, shutdown_flag)
                                    .await;
                            });
                        }
                        Err(e) => {
                            tracing::warn!("Stream 0 handshake failed: {e}");
                            let _ = capture
                                .lock()
                                .expect("CapturePipeline mutex poisoned")
                                .stop();
                            session.reset();
                            menu_bar.update_status("Idle — Listening");
                        }
                    }
                });
            }
        });

        // Reflect listening state in the menu bar.
        self.ui.menu_bar.update_status("Idle — Listening");

        tracing::info!(
            "renderd-host subsystems initialized — entering run loop (press Ctrl+C to stop)"
        );

        // Block the current thread until SIGINT or SIGTERM is received.
        Self::wait_for_shutdown()?;

        // ----------------------------------------------------------------
        // Graceful shutdown: unregister mDNS, stop capture, close QUIC socket.
        // ----------------------------------------------------------------
        tracing::info!("Unregistering mDNS service advertisement");
        if let Err(e) = advertiser.unregister() {
            tracing::warn!("mDNS unregister error (non-fatal): {e}");
        }

        if let Ok(mut guard) = self.capture.lock() {
            let _ = guard.stop();
        }
        self.session.reset();

        tracing::info!(addr = %actual_addr, "Closing QUIC server endpoint");
        quic_server.close(0, b"host-shutdown");
        rt.shutdown_background();

        tracing::info!("renderd-host shutdown complete");
        Ok(())
    }

    /// Blocks until SIGINT (Ctrl+C) or SIGTERM is received.
    fn wait_for_shutdown() -> Result<(), HostError> {
        use std::sync::atomic::{AtomicBool, Ordering};

        let stop = Arc::new(AtomicBool::new(false));

        let stop_clone = Arc::clone(&stop);
        ctrlc::set_handler(move || {
            tracing::info!("Received shutdown signal — stopping renderd-host");
            stop_clone.store(true, Ordering::SeqCst);
        })
        .map_err(|e| HostError::Initialization(format!("Failed to install signal handler: {e}")))?;

        while !stop.load(Ordering::SeqCst) {
            std::thread::park_timeout(std::time::Duration::from_millis(250));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_app_instantiation() {
        let config = RenderdConfig::default();
        let app = HostApp::new(config);
        assert!(app.session.is_idle());
    }
}
