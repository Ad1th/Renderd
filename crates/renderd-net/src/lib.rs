//! QUIC transport layer for Renderd.

pub mod burst;
pub mod client;
pub mod connection;
pub mod error;
pub mod framing;
pub mod server;
pub mod tls;

pub use burst::FragmentBurst;
pub use client::QuicClient;
pub use connection::QuicConnectionExt;
pub use error::NetError;
pub use framing::{recv_control, send_control, MAX_CONTROL_MESSAGE_SIZE};
pub use server::QuicServer;
pub use tls::{ClientTlsConfig, ServerTlsConfig};
