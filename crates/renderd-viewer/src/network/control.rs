//! Viewer control stream client for Stream 0 session negotiation.

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
    /// Returns [`ViewerError`] if stream opening, framing, or validation fails.
    pub async fn negotiate(
        &self,
        connection: &quinn::Connection,
        display: DisplayInfo,
        supported_codecs: Vec<String>,
        max_decode_bitrate_kbps: u32,
        hw_decode_available: bool,
    ) -> Result<
        (
            SessionHello,
            SessionConfig,
            quinn::SendStream,
            quinn::RecvStream,
        ),
        ViewerError,
    > {
        let (mut send_stream, mut recv_stream) = connection
            .open_bi()
            .await
            .map_err(|e| ViewerError::Network(format!("Failed to open Stream 0 to host: {e}")))?;

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

        Ok((hello, config, send_stream, recv_stream))
    }
}
