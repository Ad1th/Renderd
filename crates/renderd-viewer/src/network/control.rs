//! Viewer control stream client for Stream 0 session negotiation.
//!
//! `ViewerControlClient` opens a bidirectional QUIC stream (Stream 0) on an
//! existing connection, sends the viewer's [`SessionHello`], reads the host's
//! [`SessionConfig`] response, and validates it.
//!
//! Protocol sequence (RFC-0002 §7):
//!
//! ```text
//! Viewer ──[SessionHello]──────────────> Host
//! Viewer <─[SessionConfig]────────────── Host
//! ```

use renderd_net::framing::{recv_control, send_control};
use renderd_proto::{
    envelope::{ValidateConfig, ValidateHello},
    generated::renderd::{envelope::Payload, DisplayInfo, Envelope, SessionConfig, SessionHello},
    PROTOCOL_VERSION,
};
use uuid::Uuid;

use crate::error::ViewerError;

/// Viewer-side Stream 0 session handshake client.
#[derive(Debug)]
pub struct ViewerControlClient {
    viewer_id: Uuid,
}

impl ViewerControlClient {
    /// Creates a new [`ViewerControlClient`] with the given viewer UUID.
    #[must_use]
    pub const fn new(viewer_id: Uuid) -> Self {
        Self { viewer_id }
    }

    /// Opens Stream 0, sends [`SessionHello`], and reads the host's [`SessionConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`ViewerError`] if stream opening, framing, or validation fails.
    pub async fn negotiate(
        &self,
        connection: &quinn::Connection,
        display: DisplayInfo,
        supported_codecs: Vec<String>,
        max_decode_bitrate_kbps: u32,
        hw_decode_available: bool,
    ) -> Result<(SessionHello, SessionConfig), ViewerError> {
        // --- Step 1: Open the first bidirectional stream (Stream 0) ---
        let (mut send_stream, mut recv_stream) = connection
            .open_bi()
            .await
            .map_err(|e| ViewerError::Network(format!("Failed to open Stream 0 to host: {e}")))?;

        // --- Step 2: Build and send SessionHello ---
        let session_nonce = Uuid::new_v4().to_string();
        let hello = SessionHello {
            protocol_version: PROTOCOL_VERSION,
            min_required_version: 1,
            viewer_id: self.viewer_id.to_string(),
            supported_codecs: supported_codecs.clone(),
            max_decode_bitrate_kbps,
            display: Some(display),
            hw_decode_available,
            session_nonce,
        };

        // Self-validate before sending to catch programming errors early.
        hello
            .validate(PROTOCOL_VERSION)
            .map_err(|e| ViewerError::Network(format!("SessionHello is invalid: {e}")))?;

        let hello_env = Envelope {
            payload: Some(Payload::Hello(hello.clone())),
        };
        send_control(&mut send_stream, &hello_env)
            .await
            .map_err(|e| ViewerError::Network(format!("Failed to send SessionHello: {e}")))?;

        tracing::info!(
            viewer_id = %self.viewer_id,
            codecs = ?supported_codecs,
            "SessionHello sent to host — awaiting SessionConfig"
        );

        // --- Step 3: Read and validate the host's SessionConfig ---
        let config_env = recv_control(&mut recv_stream)
            .await
            .map_err(|e| ViewerError::Network(format!("Failed to read SessionConfig: {e}")))?;

        let config = match config_env.payload {
            Some(Payload::Config(c)) => c,
            Some(Payload::Error(err)) => {
                return Err(ViewerError::Network(format!(
                    "Host rejected session: code={} msg={}",
                    err.code, err.message
                )));
            }
            other => {
                return Err(ViewerError::Network(format!(
                    "Expected SessionConfig, got unexpected variant: {:?}",
                    other.map(|p| std::mem::discriminant(&p))
                )));
            }
        };

        config
            .validate()
            .map_err(|e| ViewerError::Network(format!("Received invalid SessionConfig: {e}")))?;

        tracing::info!(
            codec = %config.selected_codec,
            width = config.width,
            height = config.height,
            fps = config.frame_rate,
            bitrate_kbps = config.initial_bitrate_kbps,
            "SessionConfig received and validated — Stream 0 handshake complete"
        );

        Ok((hello, config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_proto::generated::renderd::DisplayInfo;

    /// Builds a display descriptor for test use.
    fn test_display() -> DisplayInfo {
        DisplayInfo {
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            vrr_supported: false,
        }
    }

    #[test]
    fn test_viewer_control_client_construction() {
        let vid = Uuid::new_v4();
        let client = ViewerControlClient::new(vid);
        assert_eq!(client.viewer_id, vid);
    }

    #[test]
    fn test_session_hello_fields_valid() {
        let display = test_display();
        let viewer_id = Uuid::new_v4();
        let hello = SessionHello {
            protocol_version: PROTOCOL_VERSION,
            min_required_version: 1,
            viewer_id: viewer_id.to_string(),
            supported_codecs: vec!["hevc".to_string()],
            max_decode_bitrate_kbps: 30_000,
            display: Some(display),
            hw_decode_available: true,
            session_nonce: Uuid::new_v4().to_string(),
        };

        assert!(hello.validate(PROTOCOL_VERSION).is_ok());
    }
}
