//! Dispatch helpers and validation logic for protocol `Envelope` messages.

use uuid::Uuid;

use crate::error::ProtoError;
use crate::generated::renderd::{envelope::Payload, Envelope, SessionConfig, SessionHello};

/// Identifies the variant of a control plane `Envelope` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKind {
    /// Handshake hello message
    SessionHello,
    /// Session configuration message
    SessionConfig,
    /// Viewer vsync report message
    VsyncReport,
    /// Short-term reactive statistics message
    ReactiveStats,
    /// Long-term periodic statistics message
    PeriodicStats,
    /// Immediate keyframe request message
    KeyframeRequest,
    /// Bitrate adjustment decision message
    BitrateAdjust,
    /// Stream reconfiguration message
    StreamReconfigure,
    /// Protocol error notification message
    Error,
    /// Unknown or empty payload
    Unknown,
}

impl Envelope {
    /// Returns the [`MessageKind`] of the payload contained inside this envelope.
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        match self.payload {
            Some(Payload::Hello(_)) => MessageKind::SessionHello,
            Some(Payload::Config(_)) => MessageKind::SessionConfig,
            Some(Payload::VsyncReport(_)) => MessageKind::VsyncReport,
            Some(Payload::ReactiveStats(_)) => MessageKind::ReactiveStats,
            Some(Payload::PeriodicStats(_)) => MessageKind::PeriodicStats,
            Some(Payload::KeyframeRequest(_)) => MessageKind::KeyframeRequest,
            Some(Payload::BitrateAdjust(_)) => MessageKind::BitrateAdjust,
            Some(Payload::StreamReconfigure(_)) => MessageKind::StreamReconfigure,
            Some(Payload::Error(_)) => MessageKind::Error,
            None => MessageKind::Unknown,
        }
    }
}

/// Validation extension trait for `SessionHello`.
pub trait ValidateHello {
    /// Validates `SessionHello` field constraints.
    ///
    /// # Errors
    /// Returns [`ProtoError`] if protocol version is incompatible or fields are invalid.
    fn validate(&self, local_version: u32) -> Result<(), ProtoError>;
}

impl ValidateHello for SessionHello {
    fn validate(&self, local_version: u32) -> Result<(), ProtoError> {
        if self.min_required_version > local_version {
            return Err(ProtoError::IncompatibleVersion {
                required: self.min_required_version,
                supported: local_version,
            });
        }

        if self.viewer_id.is_empty() {
            return Err(ProtoError::MissingField("viewer_id"));
        }

        if Uuid::parse_str(&self.viewer_id).is_err() {
            return Err(ProtoError::InvalidValue {
                field: "viewer_id",
                reason: "Must be a valid UUID string".to_string(),
            });
        }

        if self.supported_codecs.is_empty() {
            return Err(ProtoError::MissingField("supported_codecs"));
        }

        for codec in &self.supported_codecs {
            if codec != "hevc" && codec != "h264" {
                return Err(ProtoError::InvalidValue {
                    field: "supported_codecs",
                    reason: format!("Unsupported codec '{codec}'; must be 'hevc' or 'h264'"),
                });
            }
        }

        if self.session_nonce.is_empty() {
            return Err(ProtoError::MissingField("session_nonce"));
        }

        let display = self
            .display
            .as_ref()
            .ok_or(ProtoError::MissingField("display"))?;

        if display.width == 0 || display.height == 0 {
            return Err(ProtoError::InvalidValue {
                field: "display",
                reason: "Display dimensions must be greater than zero".to_string(),
            });
        }

        if display.refresh_rate <= 0.0 {
            return Err(ProtoError::InvalidValue {
                field: "display.refresh_rate",
                reason: "Refresh rate must be positive".to_string(),
            });
        }

        Ok(())
    }
}

/// Validation extension trait for `SessionConfig`.
pub trait ValidateConfig {
    /// Validates `SessionConfig` field constraints.
    ///
    /// # Errors
    /// Returns [`ProtoError`] if selected codec or dimensions are invalid.
    fn validate(&self) -> Result<(), ProtoError>;
}

impl ValidateConfig for SessionConfig {
    fn validate(&self) -> Result<(), ProtoError> {
        if self.selected_codec != "hevc" && self.selected_codec != "h264" {
            return Err(ProtoError::InvalidValue {
                field: "selected_codec",
                reason: format!(
                    "Invalid selected codec '{}'; must be 'hevc' or 'h264'",
                    self.selected_codec
                ),
            });
        }

        if self.width == 0 || self.height == 0 {
            return Err(ProtoError::InvalidValue {
                field: "dimensions",
                reason: "Width and height must be greater than zero".to_string(),
            });
        }

        if self.frame_rate <= 0.0 {
            return Err(ProtoError::InvalidValue {
                field: "frame_rate",
                reason: "Frame rate must be positive".to_string(),
            });
        }

        if self.initial_bitrate_kbps == 0 {
            return Err(ProtoError::InvalidValue {
                field: "initial_bitrate_kbps",
                reason: "Initial bitrate must be greater than zero".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::generated::renderd::{DisplayInfo, SessionConfig, SessionHello};

    #[test]
    fn test_envelope_kind() {
        let env_empty = Envelope { payload: None };
        assert_eq!(env_empty.kind(), MessageKind::Unknown);

        let env_hello = Envelope {
            payload: Some(Payload::Hello(SessionHello::default())),
        };
        assert_eq!(env_hello.kind(), MessageKind::SessionHello);
    }

    #[test]
    fn test_validate_hello() {
        let mut hello = SessionHello {
            protocol_version: 1,
            min_required_version: 1,
            viewer_id: Uuid::new_v4().to_string(),
            supported_codecs: vec!["hevc".to_string()],
            max_decode_bitrate_kbps: 50000,
            display: Some(DisplayInfo {
                width: 1920,
                height: 1080,
                refresh_rate: 60.0,
                vrr_supported: false,
            }),
            hw_decode_available: true,
            session_nonce: "nonce_123".to_string(),
        };

        assert!(hello.validate(1).is_ok());

        // Incompatible version
        hello.min_required_version = 2;
        assert!(matches!(
            hello.validate(1),
            Err(ProtoError::IncompatibleVersion { .. })
        ));
        hello.min_required_version = 1;

        // Missing viewer_id
        hello.viewer_id = String::new();
        assert_eq!(
            hello.validate(1),
            Err(ProtoError::MissingField("viewer_id"))
        );
    }

    #[test]
    fn test_validate_config() {
        let mut config = SessionConfig {
            selected_codec: "hevc".to_string(),
            width: 1920,
            height: 1080,
            frame_rate: 60.0,
            initial_bitrate_kbps: 30000,
            codec_extra_data: vec![],
            phase_sync_enabled: true,
        };

        assert!(config.validate().is_ok());

        config.selected_codec = "vp9".to_string();
        assert!(matches!(
            config.validate(),
            Err(ProtoError::InvalidValue { .. })
        ));
    }
}
