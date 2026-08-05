//! Renderd Viewer display client crate.
//!
//! Provides the application lifecycle, Win32/cross-platform windowing subsystem,
//! thread-safe frame queue, and abstractions for hardware video decoding and graphics rendering.

#![warn(missing_docs)]

pub mod app;
pub mod config;
pub mod decoder;
pub mod error;
pub mod frame_queue;
pub mod platform;
pub mod renderer;
pub mod state;
pub mod window;

pub use app::App;
pub use config::ViewerAppConfig;
pub use decoder::{DecodedFrame, Decoder, NullDecoder, PixelFormat};
pub use error::ViewerError;
pub use frame_queue::FrameQueue;
pub use renderer::{NullRenderer, Renderer, ViewportSize};
pub use state::{AppState, ConnectionState, ViewerMetrics};
pub use window::WindowSystem;
