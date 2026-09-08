//! Shared QUIC transport tuning for both ends of a Renderd session.
//!
//! The host pushes tens of megabits of video as short datagram bursts, so the
//! defaults that suit a request/response protocol are wrong here on three counts:
//!
//! * **UDP socket buffers.** Neither `quinn` nor the OS sizes them for bursty
//!   media. macOS ships a 9 KiB send buffer and Windows a 64 KiB receive buffer;
//!   a single keyframe is several hundred kilobytes, so the kernel would silently
//!   discard most of it and the viewer would ask for another keyframe, forever.
//! * **Congestion window.** Cubic starts at ~14 KiB and takes many round trips to
//!   open up. The first keyframe would trickle out over hundreds of milliseconds.
//!   A large initial window lets a LAN link run at line rate from the first frame.
//! * **Initial MTU.** Starting at the 1200-byte QUIC minimum means every datagram
//!   is fragmented into ~1.1 KiB pieces until path MTU discovery finishes.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

/// UDP socket send and receive buffer size requested from the kernel.
pub const SOCKET_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Bytes of application datagrams `quinn` may hold before dropping the oldest.
pub const DATAGRAM_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Initial congestion window, in bytes. Generous on purpose: this is a LAN tool.
pub const INITIAL_CONGESTION_WINDOW: u64 = 4 * 1024 * 1024;

/// Connection is considered dead after this long without any packet.
pub const IDLE_TIMEOUT_MS: u32 = 10_000;

/// Keep-alive cadence while idle (a static desktop can go seconds without video).
pub const KEEP_ALIVE: Duration = Duration::from_secs(2);

/// Builds the `quinn` transport configuration used by both host and viewer.
///
/// `initial_mtu` is the packet size assumed before path MTU discovery finishes;
/// callers pass the configured `network.quic_mtu`.
#[must_use]
pub fn transport_config(initial_mtu: u16) -> quinn::TransportConfig {
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(100_u32.into());
    transport.max_concurrent_uni_streams(100_u32.into());
    transport.max_idle_timeout(Some(quinn::VarInt::from_u32(IDLE_TIMEOUT_MS).into()));
    transport.keep_alive_interval(Some(KEEP_ALIVE));
    transport.datagram_receive_buffer_size(Some(DATAGRAM_BUFFER_BYTES));
    transport.datagram_send_buffer_size(DATAGRAM_BUFFER_BYTES);
    transport.initial_mtu(initial_mtu.clamp(1200, 1500));

    let mut cubic = quinn::congestion::CubicConfig::default();
    cubic.initial_window(INITIAL_CONGESTION_WINDOW);
    transport.congestion_controller_factory(Arc::new(cubic));

    transport
}

/// Binds a UDP socket with buffers sized for video bursts.
///
/// Buffer sizing is best-effort: some kernels clamp the request, and a socket
/// with default buffers still works, just with more loss under burst.
///
/// # Errors
/// Returns the underlying I/O error if the socket cannot be created or bound.
pub fn bind_udp(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if addr.is_ipv6() {
        let _ = socket.set_only_v6(false);
    }
    if let Err(e) = socket.set_recv_buffer_size(SOCKET_BUFFER_BYTES) {
        tracing::debug!(error = %e, "could not enlarge UDP receive buffer");
    }
    if let Err(e) = socket.set_send_buffer_size(SOCKET_BUFFER_BYTES) {
        tracing::debug!(error = %e, "could not enlarge UDP send buffer");
    }
    socket.bind(&addr.into())?;

    let std_socket: UdpSocket = socket.into();
    if let (Ok(rx), Ok(tx)) = (
        Socket::from(std_socket.try_clone()?).recv_buffer_size(),
        Socket::from(std_socket.try_clone()?).send_buffer_size(),
    ) {
        tracing::info!(
            recv_buffer_bytes = rx,
            send_buffer_bytes = tx,
            "UDP socket bound for QUIC transport"
        );
    }
    Ok(std_socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_udp_ephemeral_enlarges_buffers_best_effort() {
        let socket = bind_udp("127.0.0.1:0".parse().unwrap()).unwrap();
        let s = Socket::from(socket);
        // The kernel may clamp, but must not shrink below its own default.
        assert!(s.recv_buffer_size().unwrap() >= 64 * 1024);
    }

    #[test]
    fn test_transport_config_clamps_mtu() {
        let _ = transport_config(100);
        let _ = transport_config(9000);
    }
}
