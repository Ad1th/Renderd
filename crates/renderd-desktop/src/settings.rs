//! Choices the app remembers between runs.

use serde::{Deserialize, Serialize};

/// How the viewer's screen is used by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    /// The viewer becomes an extra display next to the Mac's own.
    #[default]
    Extend,
    /// The viewer shows the same picture as one of the Mac's displays.
    Mirror,
}

impl DisplayMode {
    /// The value `renderd-host --mode` expects.
    #[must_use]
    pub const fn as_arg(self) -> &'static str {
        match self {
            Self::Extend => "extend",
            Self::Mirror => "mirror",
        }
    }
}

/// Which codec the viewer asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// Let the viewer pick what this machine decodes best.
    #[default]
    Auto,
    /// Only offer H.264.
    H264,
    /// Only offer HEVC.
    Hevc,
}

impl Codec {
    /// The value `renderd-viewer --codec` expects.
    #[must_use]
    pub const fn as_arg(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }
}

/// UDP port the host listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 4433;

/// Options for sharing this computer's screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostSettings {
    /// Extend the desktop onto the viewer, or mirror a display.
    pub mode: DisplayMode,
    /// Favour smooth motion over sharpness on a slow link.
    pub smooth: bool,
    /// UDP port to listen on.
    pub port: u16,
    /// Display to mirror. Only used in mirror mode.
    pub display_id: u32,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            mode: DisplayMode::default(),
            smooth: false,
            port: DEFAULT_PORT,
            display_id: 0,
        }
    }
}
