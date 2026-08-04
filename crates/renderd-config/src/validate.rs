//! Configuration semantic validation rules and checks.

use crate::error::ConfigError;
use crate::schema::RenderdConfig;

/// Extension trait for validating configuration fields.
pub trait ValidateConfig {
    /// Performs semantic validation over all configuration sections.
    ///
    /// # Errors
    /// Returns [`ConfigError::ValidationError`] if any field fails range or sanity bounds.
    fn validate(&self) -> Result<(), ConfigError>;
}

impl ValidateConfig for RenderdConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        // Validate Host configuration
        if self.host.target_fps < 1 || self.host.target_fps > 240 {
            return Err(ConfigError::ValidationError {
                field: "host.target_fps",
                reason: format!(
                    "target_fps must be between 1 and 240 fps, got {}",
                    self.host.target_fps
                ),
            });
        }

        if self.host.max_bitrate_kbps < 1_000 || self.host.max_bitrate_kbps > 200_000 {
            return Err(ConfigError::ValidationError {
                field: "host.max_bitrate_kbps",
                reason: format!(
                    "max_bitrate_kbps must be between 1,000 and 200,000 kbps, got {}",
                    self.host.max_bitrate_kbps
                ),
            });
        }

        if self.host.codec != "hevc" && self.host.codec != "h264" {
            return Err(ConfigError::ValidationError {
                field: "host.codec",
                reason: format!("codec must be 'hevc' or 'h264', got '{}'", self.host.codec),
            });
        }

        // Validate Viewer configuration
        if self.viewer.window_width < 640 {
            return Err(ConfigError::ValidationError {
                field: "viewer.window_width",
                reason: format!(
                    "window_width must be at least 640 pixels, got {}",
                    self.viewer.window_width
                ),
            });
        }

        if self.viewer.window_height < 480 {
            return Err(ConfigError::ValidationError {
                field: "viewer.window_height",
                reason: format!(
                    "window_height must be at least 480 pixels, got {}",
                    self.viewer.window_height
                ),
            });
        }

        if self.viewer.jitter_buffer_ms > 500 {
            return Err(ConfigError::ValidationError {
                field: "viewer.jitter_buffer_ms",
                reason: format!(
                    "jitter_buffer_ms cannot exceed 500 ms, got {}",
                    self.viewer.jitter_buffer_ms
                ),
            });
        }

        // Validate Network configuration
        if self.network.quic_mtu < 1200 || self.network.quic_mtu > 1500 {
            return Err(ConfigError::ValidationError {
                field: "network.quic_mtu",
                reason: format!(
                    "quic_mtu must be between 1200 and 1500 bytes, got {}",
                    self.network.quic_mtu
                ),
            });
        }

        if self.network.listen_port == 0 {
            return Err(ConfigError::ValidationError {
                field: "network.listen_port",
                reason: "listen_port must be greater than zero".to_string(),
            });
        }

        // Validate ABR configuration
        if self.abr.min_bitrate_kbps > self.abr.max_bitrate_kbps {
            return Err(ConfigError::ValidationError {
                field: "abr.min_bitrate_kbps",
                reason: format!(
                    "min_bitrate_kbps ({}) cannot exceed max_bitrate_kbps ({})",
                    self.abr.min_bitrate_kbps, self.abr.max_bitrate_kbps
                ),
            });
        }

        if self.abr.step_kbps == 0 {
            return Err(ConfigError::ValidationError {
                field: "abr.step_kbps",
                reason: "step_kbps must be greater than zero".to_string(),
            });
        }

        if self.abr.loss_threshold <= 0.0 || self.abr.loss_threshold >= 1.0 {
            return Err(ConfigError::ValidationError {
                field: "abr.loss_threshold",
                reason: format!(
                    "loss_threshold must be strictly between 0.0 and 1.0, got {}",
                    self.abr.loss_threshold
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::schema::RenderdConfig;

    #[test]
    fn test_valid_default_config() {
        let config = RenderdConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_host_fps() {
        let mut config = RenderdConfig::default();
        config.host.target_fps = 300;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::ValidationError {
                field: "host.target_fps",
                ..
            })
        ));
    }

    #[test]
    fn test_invalid_abr_bounds() {
        let mut config = RenderdConfig::default();
        config.abr.min_bitrate_kbps = 60_000;
        config.abr.max_bitrate_kbps = 50_000;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::ValidationError {
                field: "abr.min_bitrate_kbps",
                ..
            })
        ));
    }
}
