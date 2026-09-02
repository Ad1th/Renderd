//! Configuration schema structs and default value definitions.

use serde::{Deserialize, Serialize};

/// Top-level application configuration container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RenderdConfig {
    /// macOS Host daemon configuration section.
    #[serde(default)]
    pub host: HostConfig,

    /// Windows Viewer client configuration section.
    #[serde(default)]
    pub viewer: ViewerConfig,

    /// Network transport configuration section.
    #[serde(default)]
    pub network: NetworkConfig,

    /// Cryptography and authentication configuration section.
    #[serde(default)]
    pub crypto: CryptoConfig,

    /// Adaptive Bitrate (ABR) algorithm configuration section.
    #[serde(default)]
    pub abr: AbrConfig,
}

/// Host display capture and video encoding parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConfig {
    /// Display index to capture (0 = primary display).
    pub display_id: u32,

    /// Target capture framerate in frames per second.
    pub target_fps: u32,

    /// Maximum allowed encoding bitrate in kbps.
    pub max_bitrate_kbps: u32,

    /// Hardware video encoder codec ("hevc" or "h264").
    pub codec: String,

    /// Enable vsync phase synchronization with viewer.
    pub vsync_phase_sync: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            display_id: 0,
            target_fps: 60,
            max_bitrate_kbps: 25_000,
            codec: "hevc".to_string(),
            vsync_phase_sync: true,
        }
    }
}

/// Viewer display window and rendering parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerConfig {
    /// Initial display window width in pixels.
    pub window_width: u32,

    /// Initial display window height in pixels.
    pub window_height: u32,

    /// Start viewer in exclusive fullscreen mode.
    pub fullscreen: bool,

    /// Enable vertical synchronization on D3D12 present.
    pub vsync: bool,

    /// Enable D3D12 hardware video decode acceleration.
    pub hw_accel: bool,

    /// Target jitter buffer delay in milliseconds.
    ///
    /// **TODO:** RFC-0002 §19.3 eliminates the jitter buffer entirely for wired LAN
    /// operation. This field is a placeholder; implementation should confirm whether
    /// this becomes a Wi-Fi-mode tunable or is removed before the first public release.
    pub jitter_buffer_ms: u32,
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            window_width: 1920,
            window_height: 1080,
            fullscreen: false,
            vsync: true,
            hw_accel: true,
            jitter_buffer_ms: 10,
        }
    }
}

/// QUIC network transport parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// IP address string to bind host daemon socket.
    pub bind_address: String,

    /// UDP port for host listener socket.
    pub listen_port: u16,

    /// QUIC Maximum Transmission Unit (MTU) in bytes.
    pub quic_mtu: u16,

    /// Peer connection timeout in milliseconds.
    pub connect_timeout_ms: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            listen_port: 4433,
            quic_mtu: 1350,
            connect_timeout_ms: 5000,
        }
    }
}

/// Cryptography and authentication settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptoConfig {
    /// Keychain / Credential Manager service name.
    pub keychain_service: String,

    /// Optional explicit pairing token hex string.
    pub pair_token: Option<String>,

    /// Require mutual TLS or pairing token authentication.
    pub require_auth: bool,
}

impl Default for CryptoConfig {
    fn default() -> Self {
        Self {
            keychain_service: "dev.renderd.daemon".to_string(),
            pair_token: None,
            require_auth: true,
        }
    }
}

/// Adaptive Bitrate (ABR) algorithm tuning parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbrConfig {
    /// Minimum allowed encoder bitrate in kbps.
    pub min_bitrate_kbps: u32,

    /// Maximum allowed encoder bitrate in kbps.
    ///
    /// **TODO:** RFC-0002 §13.3 sets the v1.0 maximum at **50,000 kbps** until the
    /// burst-send path is benchmarked and validated. This default (`100_000`) exceeds
    /// that limit and must be corrected before the first public release.
    pub max_bitrate_kbps: u32,

    /// Bitrate step size for incremental adjustments in kbps.
    pub step_kbps: u32,

    /// Packet loss rate threshold triggering down-step (0.0 - 1.0).
    pub loss_threshold: f32,
}

impl Default for AbrConfig {
    fn default() -> Self {
        Self {
            min_bitrate_kbps: 5_000,
            max_bitrate_kbps: 100_000,
            step_kbps: 2_000,
            loss_threshold: 0.02,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_renderd_config_defaults() {
        let config = RenderdConfig::default();
        assert_eq!(config.host.target_fps, 60);
        assert_eq!(config.viewer.window_width, 1920);
        assert_eq!(config.network.listen_port, 4433);
        assert!(config.crypto.require_auth);
        assert!((config.abr.loss_threshold - 0.02).abs() < f32::EPSILON);
    }
}
