//! QUIC transport layer for Renderd.

pub mod client;
pub mod error;
pub mod framing;
pub mod server;
pub mod tls;

pub use client::QuicClient;
pub use error::NetError;
pub use framing::{recv_control, send_control, MAX_CONTROL_MESSAGE_SIZE};
pub use server::QuicServer;
pub use tls::{ClientTlsConfig, ServerTlsConfig};
