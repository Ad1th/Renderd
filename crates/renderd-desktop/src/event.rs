//! What a log line means for the session.
//!
//! The daemons already log every step of a session. This module picks out the
//! handful of lines that change what the window should show.

use serde::Serialize;

use crate::logline::LogLine;

/// The picture the two sides agreed on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamInfo {
    /// Codec name, `hevc` or `h264`.
    pub codec: String,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Frames per second the session asked for.
    pub fps: f64,
}

/// Live numbers from a running session.
///
/// Each log line carries only some of these, so every value is optional and a
/// later sample fills in what an earlier one left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Metrics {
    /// Frames sent (host) or shown (viewer) per second.
    pub fps: Option<f64>,
    /// Video bitrate on the wire, in kbit/s.
    pub bitrate_kbps: Option<f64>,
    /// Round trip time to the other side, in milliseconds.
    pub rtt_ms: Option<f64>,
    /// Video waiting to be sent on the host, in kilobytes.
    pub send_queue_kb: Option<f64>,
    /// Time to decode one frame on the viewer, in milliseconds.
    pub decode_ms: Option<f64>,
    /// Frames the viewer gave up on since it connected.
    pub lost_frames: Option<f64>,
}

impl Metrics {
    /// Overlays `newer` on this sample, keeping old values it does not carry.
    #[must_use]
    pub fn merged(self, newer: Self) -> Self {
        Self {
            fps: newer.fps.or(self.fps),
            bitrate_kbps: newer.bitrate_kbps.or(self.bitrate_kbps),
            rtt_ms: newer.rtt_ms.or(self.rtt_ms),
            send_queue_kb: newer.send_queue_kb.or(self.send_queue_kb),
            decode_ms: newer.decode_ms.or(self.decode_ms),
            lost_frames: newer.lost_frames.or(self.lost_frames),
        }
    }
}

/// Something that happened in the child process that the window should react to.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The host is listening for a viewer on this socket address.
    Listening {
        /// Address the QUIC server bound to.
        addr: String,
    },
    /// The address to type on the viewer machine.
    Address(String),
    /// The viewer is looking for a host on the network.
    Searching,
    /// The viewer is dialling a host.
    Connecting {
        /// Address being dialled.
        addr: String,
    },
    /// The other side connected.
    PeerConnected {
        /// Address of the other side.
        peer: String,
    },
    /// Both sides agreed on a picture format.
    Configured(StreamInfo),
    /// Video is flowing.
    Streaming,
    /// Fresh numbers from the running stream.
    Metrics(Metrics),
    /// The stream changed size to fit the link.
    Resized {
        /// New picture width.
        width: u32,
        /// New picture height.
        height: u32,
    },
    /// The other side went away.
    PeerLost,
    /// A connection attempt failed and will be retried.
    ConnectFailed(String),
    /// Screen capture could not start.
    CaptureFailed(String),
    /// The child was asked to stop and is shutting down.
    Stopping,
}

impl StreamInfo {
    /// Reads the stream description off a handshake line.
    fn from_line(line: &LogLine) -> Option<Self> {
        Some(Self {
            codec: line.text("codec")?.to_string(),
            width: line.integer("width")?,
            height: line.integer("height")?,
            fps: line.number("fps")?,
        })
    }
}

/// Text after `prefix` in a message that was built with `format!`.
fn after<'a>(message: &'a str, prefix: &str) -> Option<&'a str> {
    message.strip_prefix(prefix).map(str::trim)
}

/// Interprets a line written by `renderd-host`.
#[must_use]
pub fn host_event(line: &LogLine) -> Option<Event> {
    let message = line.message.as_str();
    match message {
        "QUIC server endpoint listening for incoming viewer connections" => {
            return Some(Event::Listening {
                addr: line.text("listen_addr")?.to_string(),
            });
        }
        "HostSession state transition to CONNECTED" => {
            return Some(Event::PeerConnected {
                peer: line.text("remote_addr")?.to_string(),
            });
        }
        "HostSession state transition to STREAMING" => return Some(Event::Streaming),
        "HostSession state transition to IDLE" => return Some(Event::PeerLost),
        "Resizing the stream to fit the link" => {
            return Some(Event::Resized {
                width: line.integer("width")?,
                height: line.integer("height")?,
            });
        }
        "HOST METRICS: transmit throughput" => {
            return Some(Event::Metrics(Metrics {
                fps: line.number("fps"),
                bitrate_kbps: line.number("bitrate_kbps"),
                rtt_ms: line.number("rtt_ms"),
                send_queue_kb: line.number("send_queue_kb"),
                ..Metrics::default()
            }));
        }
        _ => {}
    }

    if let Some(addr) = after(message, "Connect the viewer with:  renderd-viewer --host") {
        return Some(Event::Address(addr.to_string()));
    }
    if message.starts_with("SessionConfig sent") {
        return StreamInfo::from_line(line).map(Event::Configured);
    }
    if let Some(detail) = after(message, "Capture pipeline start failed:") {
        return Some(Event::CaptureFailed(detail.to_string()));
    }
    if message.starts_with("Received shutdown signal") {
        return Some(Event::Stopping);
    }
    None
}

/// Interprets a line written by `renderd-viewer`.
#[must_use]
pub fn viewer_event(line: &LogLine) -> Option<Event> {
    let message = line.message.as_str();
    match message {
        "Connecting to discovered host..." => {
            return Some(Event::Connecting {
                addr: line.text("host_addr")?.to_string(),
            });
        }
        "QUIC connection established with host" => {
            return Some(Event::PeerConnected {
                peer: line.text("peer")?.to_string(),
            });
        }
        "Stream 0 handshake completed with host" => {
            return StreamInfo::from_line(line).map(Event::Configured);
        }
        "DatagramReceiver: first decoded frame pushed into FrameQueue" => {
            return Some(Event::Streaming);
        }
        "VIEWER METRICS: presentation" => {
            return Some(Event::Metrics(Metrics {
                fps: line.number("present_fps"),
                decode_ms: line.number("decode_ms"),
                ..Metrics::default()
            }));
        }
        "VIEWER METRICS: network receive & decode throughput" => {
            return Some(Event::Metrics(Metrics {
                bitrate_kbps: line.number("recv_bitrate_kbps"),
                lost_frames: line.number("lost_total"),
                ..Metrics::default()
            }));
        }
        "Network connection lost; attempting reconnect to cached IP"
        | "Cached IP failed; initiating mDNS re-discovery" => return Some(Event::PeerLost),
        "Close requested by user; shutting down viewer app" => return Some(Event::Stopping),
        _ => {}
    }

    if message.starts_with("mDNS browser started") {
        return Some(Event::Searching);
    }
    if message.starts_with("Host stream ended") {
        return Some(Event::PeerLost);
    }
    if message.starts_with("QUIC connection to ") {
        return Some(Event::ConnectFailed(message.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST_LOG: &str = include_str!("../fixtures/host.jsonl");
    const VIEWER_LOG: &str = include_str!("../fixtures/viewer.jsonl");

    fn events(log: &str, interpret: fn(&LogLine) -> Option<Event>) -> Vec<Event> {
        log.lines()
            .filter_map(|line| interpret(&LogLine::parse(line)))
            .collect()
    }

    #[test]
    fn host_session_starts_by_listening() {
        let events = events(HOST_LOG, host_event);
        assert_eq!(
            events.first(),
            Some(&Event::Listening {
                addr: "0.0.0.0:44999".to_string()
            })
        );
    }

    #[test]
    fn host_reports_the_address_to_share() {
        let events = events(HOST_LOG, host_event);
        assert!(events.contains(&Event::Address("192.168.1.20:44999".to_string())));
    }

    #[test]
    fn host_session_connects_then_streams_then_goes_idle() {
        let events = events(HOST_LOG, host_event);
        let position = |wanted: fn(&Event) -> bool| events.iter().position(wanted).unwrap();
        let connected = position(|e| matches!(e, Event::PeerConnected { .. }));
        let streaming = position(|e| matches!(e, Event::Streaming));
        let metrics = position(|e| matches!(e, Event::Metrics(_)));
        let lost = position(|e| matches!(e, Event::PeerLost));
        assert!(connected < streaming);
        assert!(streaming < metrics);
        assert!(metrics < lost);
    }

    #[test]
    fn host_reports_the_negotiated_stream() {
        let events = events(HOST_LOG, host_event);
        let info = events
            .iter()
            .find_map(|e| match e {
                Event::Configured(info) => Some(info.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(info.codec, "hevc");
        assert_eq!((info.width, info.height), (640, 360));
        assert!((info.fps - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn host_metrics_carry_fps_and_bitrate() {
        let events = events(HOST_LOG, host_event);
        let sample = events
            .iter()
            .find_map(|e| match e {
                Event::Metrics(m) => Some(*m),
                _ => None,
            })
            .unwrap();
        assert!(sample.fps.unwrap() > 1.0);
        assert!(sample.bitrate_kbps.unwrap() > 1.0);
        assert!(sample.rtt_ms.is_some());
        assert!(sample.decode_ms.is_none());
    }

    #[test]
    fn host_shutdown_is_reported() {
        let events = events(HOST_LOG, host_event);
        assert!(events.contains(&Event::Stopping));
    }

    #[test]
    fn viewer_session_dials_connects_and_streams() {
        let events = events(VIEWER_LOG, viewer_event);
        let position = |wanted: fn(&Event) -> bool| events.iter().position(wanted).unwrap();
        let dialling = position(|e| matches!(e, Event::Connecting { .. }));
        let connected = position(|e| matches!(e, Event::PeerConnected { .. }));
        let configured = position(|e| matches!(e, Event::Configured(_)));
        let streaming = position(|e| matches!(e, Event::Streaming));
        assert!(dialling < connected);
        assert!(connected < configured);
        assert!(configured < streaming);
    }

    #[test]
    fn viewer_metrics_cover_both_kinds_of_sample() {
        let merged = events(VIEWER_LOG, viewer_event)
            .into_iter()
            .filter_map(|e| match e {
                Event::Metrics(m) => Some(m),
                _ => None,
            })
            .fold(Metrics::default(), Metrics::merged);
        assert!(merged.fps.is_some());
        assert!(merged.bitrate_kbps.is_some());
        assert!(merged.decode_ms.is_some());
        assert!(merged.lost_frames.is_some());
        assert!(merged.rtt_ms.is_none());
    }

    #[test]
    fn merging_keeps_values_the_new_sample_lacks() {
        let old = Metrics {
            fps: Some(60.0),
            rtt_ms: Some(2.0),
            ..Metrics::default()
        };
        let new = Metrics {
            fps: Some(30.0),
            ..Metrics::default()
        };
        let merged = old.merged(new);
        assert_eq!(merged.fps, Some(30.0));
        assert_eq!(merged.rtt_ms, Some(2.0));
    }
}
