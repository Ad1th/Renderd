//! Application configuration for the Renderd Viewer client.

use crate::error::ViewerError;
use renderd_config::{ConfigBuilder, RenderdConfig, ValidateConfig};

/// Viewer application runtime configuration options.
#[derive(Debug, Clone)]
pub struct ViewerAppConfig {
    /// Inner unified [`RenderdConfig`].
    pub config: RenderdConfig,
    /// Window title string.
    pub window_title: String,
    /// Initial window width in physical pixels.
    pub window_width: u32,
    /// Initial window height in physical pixels.
    pub window_height: u32,
    /// Whether to start in borderless fullscreen mode.
    pub fullscreen: bool,
    /// Explicit host address supplied on the command line.
    ///
    /// When set, discovery is skipped and the viewer connects straight to this
    /// address — the reliable path when mDNS cannot cross the network between
    /// the two machines.
    pub manual_host: Option<std::net::SocketAddr>,
    /// Which decoder implementation to construct.
    pub decoder_backend: crate::cli::DecoderBackend,
    /// Codec preference to advertise during the handshake.
    pub codec_choice: crate::cli::CodecChoice,
}

impl Default for ViewerAppConfig {
    fn default() -> Self {
        Self {
            config: RenderdConfig::default(),
            window_title: "Renderd Viewer".to_string(),
            window_width: 1920,
            window_height: 1080,
            fullscreen: false,
            manual_host: None,
            decoder_backend: crate::cli::DecoderBackend::Mf,
            codec_choice: crate::cli::CodecChoice::Auto,
        }
    }
}

impl ViewerAppConfig {
    /// Loads configuration from default templates and layered environment overrides.
    ///
    /// # Errors
    /// Returns [`ViewerError::Config`] if configuration loading or validation fails.
    pub fn load() -> Result<Self, ViewerError> {
        let config = ConfigBuilder::new()
            .build()
            .map_err(|e| ViewerError::Config(format!("Failed to load config: {e}")))?;

        // The doc comment above has always claimed this can fail on invalid
        // config, but nothing actually called validate() — a config with, say,
        // window_width = 0 or an out-of-range quic_mtu loaded successfully and
        // was used unchecked. main.rs falls back to ViewerAppConfig::default()
        // on any Err from this function, so surfacing the failure here is safe.
        config
            .validate()
            .map_err(|e| ViewerError::Config(format!("Invalid config: {e}")))?;

        Ok(Self {
            window_title: format!("Renderd Viewer v{}", env!("CARGO_PKG_VERSION")),
            window_width: config.viewer.window_width,
            window_height: config.viewer.window_height,
            fullscreen: config.viewer.fullscreen,
            manual_host: None,
            decoder_backend: crate::cli::DecoderBackend::Mf,
            codec_choice: crate::cli::CodecChoice::Auto,
            config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewer_config_default() {
        let cfg = ViewerAppConfig::default();
        assert_eq!(cfg.window_width, 1920);
        assert_eq!(cfg.window_height, 1080);
        assert!(!cfg.fullscreen);
        assert!(cfg.manual_host.is_none());
    }
}
