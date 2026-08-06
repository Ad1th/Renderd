//! Renderd Viewer display client crate.
//!
//! Provides the application lifecycle, Win32/cross-platform windowing subsystem,
//! thread-safe frame queue, and abstractions for hardware video decoding and graphics rendering.

#![allow(unsafe_code)]
#![warn(missing_docs)]

pub mod abr;
pub mod app;
pub mod clock_sync;
pub mod config;
pub mod decode;
pub mod decoder;
pub mod discovery;
pub mod error;
pub mod frame_queue;
pub mod network;
pub mod pairing;
pub mod platform;
pub mod reconnect;
pub mod render;
pub mod renderer;
pub mod state;
pub mod tearing;
pub mod ui;
pub mod window;

pub use abr::FeedbackExporter;
pub use app::App;
pub use clock_sync::VsyncReporter;
pub use config::ViewerAppConfig;
pub use decode::D3D12Decoder;
#[cfg(target_os = "macos")]
pub use decode::VideoToolboxDecoder;
pub use decoder::{DecodedFrame, Decoder, NullDecoder, PixelFormat};
pub use discovery::{DiscoveredHosts, DiscoveryManager};
pub use error::ViewerError;
pub use frame_queue::FrameQueue;
pub use network::{DatagramReceiver, ViewerControlClient};
pub use pairing::{PairingUi, PairingUiState, ViewerPairingClient, ViewerPairingError};
pub use reconnect::{ReconnectWatchdog, WatchdogState};
pub use render::D3D12Renderer;
pub use renderer::{NullRenderer, Renderer, SoftRenderer, ViewportSize};
pub use state::{AppState, ConnectionState, ViewerMetrics};
pub use tearing::check_tearing_support;
pub use ui::{StatusOverlay, SystemTrayManager, TrayMenuAction, ViewerSettingsState};
pub use window::WindowSystem;
