//! Network transport, TLS configuration, and control stream framing for Renderd.

pub mod burst;
pub mod client;
pub mod connection;
pub mod error;
pub mod framing;
pub mod mock;
pub mod server;
pub mod tls;

pub use burst::FragmentBurst;
pub use client::QuicClient;
pub use connection::QuicConnectionExt;
pub use error::NetError;
pub use framing::{recv_control, send_control};
pub use mock::MockConnection;
pub use server::QuicServer;
pub use tls::{ClientTlsConfig, ServerTlsConfig};
