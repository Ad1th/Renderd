//! In-memory mock transport for testing control and data plane interactions without OS sockets.

use bytes::Bytes;
use renderd_proto::Envelope;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use crate::error::NetError;

/// In-memory paired transport endpoints simulating host and viewer QUIC connections.
pub struct MockConnection {
    control_tx: Sender<Envelope>,
    control_rx: Receiver<Envelope>,
    datagram_tx: Sender<Bytes>,
    datagram_rx: Receiver<Bytes>,
}

impl MockConnection {
    /// Creates a connected pair of [`MockConnection`] endpoints (`(host_endpoint, viewer_endpoint)`).
    #[must_use]
    pub fn pair(buffer_capacity: usize) -> (Self, Self) {
        let (h2v_ctrl_tx, h2v_ctrl_rx) = channel(buffer_capacity);
        let (v2h_ctrl_tx, v2h_ctrl_rx) = channel(buffer_capacity);

        let (h2v_data_tx, h2v_data_rx) = channel(buffer_capacity);
        let (v2h_data_tx, v2h_data_rx) = channel(buffer_capacity);

        let host = Self {
            control_tx: h2v_ctrl_tx,
            control_rx: v2h_ctrl_rx,
            datagram_tx: h2v_data_tx,
            datagram_rx: v2h_data_rx,
        };

        let viewer = Self {
            control_tx: v2h_ctrl_tx,
            control_rx: h2v_ctrl_rx,
            datagram_tx: v2h_data_tx,
            datagram_rx: h2v_data_rx,
        };

        (host, viewer)
    }

    /// Sends a control [`Envelope`] asynchronously to the paired peer.
    ///
    /// # Errors
    /// Returns [`NetError::Framing`] if the control channel is closed.
    pub async fn send_control(&self, env: &Envelope) -> Result<(), NetError> {
        self.control_tx
            .send(env.clone())
            .await
            .map_err(|_| NetError::Framing("Peer closed control channel".to_string()))
    }

    /// Receives a control [`Envelope`] asynchronously from the paired peer.
    ///
    /// # Errors
    /// Returns [`NetError::Framing`] if the control channel is closed before a message is received.
    pub async fn recv_control(&mut self) -> Result<Envelope, NetError> {
        self.control_rx
            .recv()
            .await
            .ok_or_else(|| NetError::Framing("Control channel closed".to_string()))
    }

    /// Sends a frame fragment datagram asynchronously to the paired peer.
    ///
    /// # Errors
    /// Returns [`NetError::Datagram`] if the datagram channel is closed.
    pub async fn send_datagram(&self, datagram: Bytes) -> Result<(), NetError> {
        self.datagram_tx
            .send(datagram)
            .await
            .map_err(|_| NetError::Datagram("Peer closed datagram channel".to_string()))
    }

    /// Receives a frame fragment datagram asynchronously from the paired peer.
    ///
    /// # Errors
    /// Returns [`NetError::Datagram`] if the datagram channel is closed.
    pub async fn recv_datagram(&mut self) -> Result<Bytes, NetError> {
        self.datagram_rx
            .recv()
            .await
            .ok_or_else(|| NetError::Datagram("Datagram channel closed".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_proto::generated::renderd::{envelope, SessionHello};

    #[tokio::test]
    async fn test_mock_connection_pair_exchange() {
        let (host, mut viewer) = MockConnection::pair(16);

        let env = Envelope {
            payload: Some(envelope::Payload::Hello(SessionHello {
                protocol_version: 1,
                min_required_version: 1,
                viewer_id: "viewer-uuid".to_string(),
                supported_codecs: vec!["hevc".to_string()],
                max_decode_bitrate_kbps: 30_000,
                display: None,
                hw_decode_available: true,
                session_nonce: "nonce".to_string(),
            })),
        };

        host.send_control(&env).await.unwrap();
        let recv_env = viewer.recv_control().await.unwrap();
        assert_eq!(recv_env, env);

        let datagram = Bytes::from_static(b"frame-fragment-data");
        host.send_datagram(datagram.clone()).await.unwrap();
        let recv_data = viewer.recv_datagram().await.unwrap();
        assert_eq!(recv_data, datagram);
    }
}
