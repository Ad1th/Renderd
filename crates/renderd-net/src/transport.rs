//! Shared QUIC transport tuning for both ends of a Renderd session.
//!
//! The host pushes tens of megabits of video as short datagram bursts, so the
//! defaults that suit a request/response protocol are wrong here on several counts:
//!
//! * **UDP socket buffers.** Neither `quinn` nor the OS sizes them for bursty
//!   media. macOS ships a 9 KiB send buffer and Windows a 64 KiB receive buffer;
//!   a single keyframe is several hundred kilobytes, so the kernel would silently
//!   discard most of it and the viewer would ask for another keyframe, forever.
//! * **Standing queue depth.** `quinn` will happily hold megabytes of datagrams
//!   the application hasn't read yet, or sent-but-unpaced ones the network hasn't
//!   absorbed yet. That queue is exactly where seconds of latency hide: it grows
//!   silently whenever the far end is even slightly slower than the source, and
//!   nothing shrinks it back down. It is bounded tightly here on purpose — see
//!   [`DATAGRAM_BUFFER_BYTES`].
//! * **Congestion control.** Cubic is loss-based: on a path that queues instead of
//!   dropping (any home router, any Wi-Fi radio) it keeps growing the window
//!   because it never sees a loss signal, which is the textbook definition of
//!   bufferbloat — it was quietly manufacturing the multi-second lag this module
//!   now exists to prevent. BBR watches round-trip time and backs off the moment
//!   RTT starts climbing, whether or not anything was ever dropped, so a
//!   standing queue can't build unnoticed on a real (non-LAN, possibly Wi-Fi)
//!   path.
//! * **Initial MTU.** Starting at the 1200-byte QUIC minimum means every datagram
//!   is fragmented into ~1.1 KiB pieces until path MTU discovery finishes.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

/// UDP socket send and receive buffer size requested from the kernel.
///
/// This is a kernel-level burst-absorption buffer, not a standing queue the
/// application waits behind — `quinn` paces what it hands to the OS according to
/// the congestion window, so a generous value here only prevents packet loss
/// during a legitimate burst. It is *not* where the bufferbloat risk lives; see
/// [`DATAGRAM_BUFFER_BYTES`] for that.
pub const SOCKET_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Bytes of application datagrams `quinn` may hold, on each side, before it starts
/// dropping.
///
/// This *is* the standing-queue bound. At the ABR ceiling (60 Mbps = 7.5 MB/s) a
/// prior 8 MiB value could hold over a second of already-stale video before a
/// single byte was dropped — worse, on the receive side that video was decoded
/// and shown anyway, strictly in arrival order, which is precisely how a few
/// hundred milliseconds of decode being slower than real time turns into
/// multi-second, ever-growing lag. 2 MiB bounds the worst case to a few hundred
/// milliseconds — generous enough to absorb a keyframe burst, small enough that
/// the receive loop's backlog-skip logic (see `renderd-viewer::network::data`)
/// kicks in almost immediately instead of quietly queuing.
pub const DATAGRAM_BUFFER_BYTES: usize = 2 * 1024 * 1024;

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

    // BBR over Cubic: see the module docs above for why a loss-based controller
    // is the wrong choice for a link that is more likely to queue than to drop.
    // Its own default initial window (~234 KiB) is left alone rather than
    // widened — BBR corrects a too-small window within a round trip or two by
    // design, so there is no latency reason to start more aggressively, and
    // every extra byte of initial burst is extra bytes a slow path has to queue
    // before BBR's first RTT-based correction can act on it.
    let bbr = quinn::congestion::BbrConfig::default();
    transport.congestion_controller_factory(Arc::new(bbr));

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
