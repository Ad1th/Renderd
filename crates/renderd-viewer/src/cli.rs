//! Command-line argument parser for `renderd-viewer`.

use clap::Parser;
use std::net::SocketAddr;

/// Default UDP port the host listens on.
pub const DEFAULT_HOST_PORT: u16 = 4433;

/// Windows viewer display client.
#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(name = "renderd-viewer", author, version, about)]
pub struct ViewerCli {
    /// Host address to connect to, e.g. `192.168.1.42` or `192.168.1.42:4433`.
    ///
    /// Skips mDNS discovery entirely. Use this whenever the two machines cannot
    /// see each other's multicast traffic — a different subnet, a VPN, or a
    /// firewall that blocks mDNS will all prevent automatic discovery.
    #[arg(short = 'H', long, value_name = "ADDR")]
    pub host: Option<String>,

    /// Logging level (trace, debug, info, warn, error).
    #[arg(short, long, default_value = "info")]
    pub log_level: String,

    /// Start in borderless fullscreen.
    #[arg(short, long)]
    pub fullscreen: bool,

    /// Initial window width in physical pixels.
    #[arg(long)]
    pub width: Option<u32>,

    /// Initial window height in physical pixels.
    #[arg(long)]
    pub height: Option<u32>,

    /// Force a codec instead of using this platform's preference order.
    ///
    /// `auto` (default) offers H.264 first on Windows and HEVC first elsewhere.
    /// Pin this when one codec misbehaves on a particular machine.
    #[arg(long, value_name = "CODEC", default_value = "auto")]
    pub codec: CodecChoice,

    /// Video decoder backend to use on Windows.
    ///
    /// `mf` (default) uses a Media Foundation decoder MFT, which parses the bitstream
    /// itself. `d3d12` uses the `ID3D12VideoDecoder` path, which needs DXVA picture
    /// parameters this build does not yet supply — it is kept only for development.
    #[arg(long, value_name = "BACKEND", default_value = "mf")]
    pub decoder: DecoderBackend,
}

/// Codec preference override.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodecChoice {
    /// Use the platform's default preference order.
    #[default]
    Auto,
    /// Offer only H.264.
    H264,
    /// Offer only HEVC.
    Hevc,
}

impl CodecChoice {
    /// Returns the codec list to advertise in `SessionHello`.
    #[must_use]
    pub fn codecs(self) -> Vec<String> {
        match self {
            Self::Auto => crate::decode::preferred_codecs(),
            Self::H264 => vec!["h264".to_string()],
            Self::Hevc => vec!["hevc".to_string()],
        }
    }
}

/// Selectable video decoder implementation.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecoderBackend {
    /// Media Foundation decoder MFT (default on Windows).
    #[default]
    Mf,
    /// Direct3D 12 video decoder.
    D3d12,
}

/// Parses a `--host` value into a socket address, defaulting the port when absent.
///
/// Accepts `host:port`, a bare IPv4 address, or a bracketed IPv6 literal.
///
/// # Errors
/// Returns a human-readable message naming the value that could not be parsed.
pub fn parse_host_arg(value: &str) -> Result<SocketAddr, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("host address is empty".to_string());
    }

    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Ok(addr);
    }

    if let Ok(ip) = trimmed.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_HOST_PORT));
    }

    Err(format!(
        "'{value}' is not a valid host address; expected 1.2.3.4, 1.2.3.4:4433, or [::1]:4433"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bare_ipv4_uses_default_port() {
        let addr = parse_host_arg("192.168.1.42").unwrap();
        assert_eq!(addr.port(), DEFAULT_HOST_PORT);
        assert_eq!(addr.ip().to_string(), "192.168.1.42");
    }

    #[test]
    fn test_parse_ipv4_with_port() {
        let addr = parse_host_arg("10.0.0.7:5000").unwrap();
        assert_eq!(addr.port(), 5000);
    }

    #[test]
    fn test_parse_ipv6_forms() {
        assert_eq!(parse_host_arg("::1").unwrap().port(), DEFAULT_HOST_PORT);
        assert_eq!(parse_host_arg("[::1]:9000").unwrap().port(), 9000);
    }

    #[test]
    fn test_parse_rejects_garbage_with_a_useful_message() {
        let err = parse_host_arg("not-an-address").unwrap_err();
        assert!(
            err.contains("not-an-address"),
            "message names the input: {err}"
        );
        assert!(parse_host_arg("   ").is_err());
    }

    #[test]
    fn test_codec_choice_auto_matches_platform_preference() {
        assert_eq!(
            CodecChoice::Auto.codecs(),
            crate::decode::preferred_codecs()
        );
    }

    #[test]
    fn test_codec_choice_pins_a_single_codec() {
        assert_eq!(CodecChoice::H264.codecs(), vec!["h264".to_string()]);
        assert_eq!(CodecChoice::Hevc.codecs(), vec!["hevc".to_string()]);
    }

    #[test]
    fn test_decoder_backend_defaults_to_media_foundation() {
        let cli = ViewerCli::parse_from(["renderd-viewer"]);
        assert_eq!(cli.decoder, DecoderBackend::Mf);
    }

    #[test]
    fn test_decoder_backend_can_be_overridden() {
        let cli = ViewerCli::parse_from(["renderd-viewer", "--decoder", "d3d12"]);
        assert_eq!(cli.decoder, DecoderBackend::D3d12);
    }

    #[test]
    fn test_cli_parses_host_and_flags() {
        let cli = ViewerCli::parse_from([
            "renderd-viewer",
            "--host",
            "192.168.1.42",
            "--fullscreen",
            "--width",
            "2560",
        ]);
        assert_eq!(cli.host.as_deref(), Some("192.168.1.42"));
        assert!(cli.fullscreen);
        assert_eq!(cli.width, Some(2560));
    }
}
