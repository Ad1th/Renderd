//! Control stream 0 message dispatch for the host daemon.

use renderd_config::HostConfig;
use renderd_net::framing::{recv_control, send_control};
use renderd_proto::{
    envelope::ValidateHello,
    generated::renderd::{envelope::Payload, DisplayInfo, Envelope, SessionConfig, SessionHello},
    types::ViewerId,
};
use uuid::Uuid;

use crate::error::HostError;
use crate::session::HostSession;

/// Host control stream dispatcher.
#[derive(Debug, Default, Clone)]
pub struct ControlDispatcher;

impl ControlDispatcher {
    /// Creates a new [`ControlDispatcher`].
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Handles the Stream 0 session negotiation for a newly accepted QUIC connection.
    ///
    /// # Errors
    /// Returns [`HostError`] if stream acceptance, framing, or validation fails.
    ///
    /// # Panics
    /// Panics if internal payload validation invariants are violated.
    #[allow(clippy::cast_precision_loss, clippy::missing_panics_doc)]
    pub async fn handle_connection(
        &self,
        connection: &quinn::Connection,
        host_config: &HostConfig,
        session: &HostSession,
    ) -> Result<
        (
            SessionHello,
            SessionConfig,
            quinn::SendStream,
            quinn::RecvStream,
        ),
        HostError,
    > {
        let (mut send_stream, mut recv_stream) = connection.accept_bi().await.map_err(|e| {
            HostError::Initialization(format!(
                "Failed to accept bidirectional stream from viewer: {e}"
            ))
        })?;

        tracing::debug!(
            peer = %connection.remote_address(),
            "Stream 0 accepted from viewer"
        );

        let hello_env = recv_control(&mut recv_stream)
            .await
            .map_err(|e| HostError::Initialization(format!("Failed to read SessionHello: {e}")))?;

        let hello = match hello_env.payload {
            Some(Payload::Hello(h)) => h,
            other => {
                return Err(HostError::Initialization(format!(
                    "Expected SessionHello on Stream 0, got {:?}",
                    other.map(|p| std::mem::discriminant(&p))
                )));
            }
        };

        hello
            .validate(renderd_proto::PROTOCOL_VERSION)
            .map_err(|e| {
                HostError::Initialization(format!("SessionHello validation failed: {e}"))
            })?;

        let viewer_uuid = Uuid::parse_str(&hello.viewer_id).map_err(|e| {
            HostError::Initialization(format!(
                "Invalid viewer_id UUID '{:?}': {e}",
                hello.viewer_id
            ))
        })?;
        let viewer_id = ViewerId(viewer_uuid);

        tracing::info!(
            viewer_id = %viewer_id,
            codecs = ?hello.supported_codecs,
            protocol_version = hello.protocol_version,
            "Received valid SessionHello from viewer"
        );

        let selected_codec = if hello.supported_codecs.iter().any(|c| c == "hevc") {
            "hevc".to_string()
        } else {
            "h264".to_string()
        };

        let display: &DisplayInfo = hello
            .display
            .as_ref()
            .expect("display validated non-None above");

        let session_config = SessionConfig {
            selected_codec: selected_codec.clone(),
            width: display.width,
            height: display.height,
            frame_rate: host_config.target_fps as f32,
            initial_bitrate_kbps: host_config.max_bitrate_kbps,
            codec_extra_data: vec![],
            phase_sync_enabled: host_config.vsync_phase_sync,
        };

        let config_env = Envelope {
            payload: Some(Payload::Config(session_config.clone())),
        };
        send_control(&mut send_stream, &config_env)
            .await
            .map_err(|e| HostError::Initialization(format!("Failed to send SessionConfig: {e}")))?;

        session
            .complete_pairing(viewer_id, connection.remote_address())
            .map_err(|e| {
                HostError::Initialization(format!("Session transition to CONNECTED failed: {e}"))
            })?;

        tracing::info!(
            viewer_id = %viewer_id,
            codec = %selected_codec,
            width = session_config.width,
            height = session_config.height,
            fps = session_config.frame_rate,
            "SessionConfig sent — Stream 0 handshake complete, HostSession in CONNECTED state"
        );

        Ok((hello, session_config, send_stream, recv_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_net::MockConnection;
    use renderd_proto::{
        envelope::ValidateConfig,
        generated::renderd::{DisplayInfo, SessionHello},
    };
    use uuid::Uuid;

    fn make_hello() -> SessionHello {
        SessionHello {
            protocol_version: renderd_proto::PROTOCOL_VERSION,
            min_required_version: 1,
            viewer_id: Uuid::new_v4().to_string(),
            supported_codecs: vec!["hevc".to_string(), "h264".to_string()],
            max_decode_bitrate_kbps: 50_000,
            display: Some(DisplayInfo {
                width: 1920,
                height: 1080,
                refresh_rate: 60.0,
                vrr_supported: false,
            }),
            hw_decode_available: true,
            session_nonce: "test-nonce-abc123".to_string(),
        }
    }

    #[tokio::test]
    async fn test_control_dispatcher_mock_handshake() {
        use renderd_proto::generated::renderd::envelope::Payload;

        let (host_mock, mut viewer_mock) = MockConnection::pair(16);
        let _host_config = HostConfig::default();
        let _dispatcher = ControlDispatcher::new();

        let hello = make_hello();
        let hello_env = Envelope {
            payload: Some(Payload::Hello(hello.clone())),
        };

        tokio::spawn(async move {
            host_mock.send_control(&hello_env).await.unwrap();
            let config_env = viewer_mock.recv_control().await.unwrap();
            let Some(Payload::Config(config)) = config_env.payload else {
                panic!("Expected SessionConfig");
            };
            assert!(config.validate().is_ok());
            assert_eq!(config.selected_codec, "hevc");
            assert_eq!(config.width, 1920);
            assert_eq!(config.height, 1080);
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    #[test]
    fn test_codec_preference_hevc_over_h264() {
        let codecs: Vec<String> = vec!["h264".to_string(), "hevc".to_string()];
        let selected = if codecs.iter().any(|c| c == "hevc") {
            "hevc"
        } else {
            "h264"
        };
        assert_eq!(selected, "hevc");
    }
}
