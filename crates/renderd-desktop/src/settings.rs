//! Choices the app remembers between runs.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::role::Role;

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

/// Options for showing another computer's screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerSettings {
    /// Open the stream in borderless fullscreen.
    pub fullscreen: bool,
    /// Codec to ask the host for.
    pub codec: Codec,
    /// Hosts connected to by address before, newest first.
    pub recent_hosts: Vec<String>,
}

/// Everything the app saves to disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which side this computer plays.
    pub role: Role,
    /// Options used when sharing.
    pub host: HostSettings,
    /// Options used when viewing.
    pub viewer: ViewerSettings,
}

/// How many recently used hosts are kept.
pub const MAX_RECENT_HOSTS: usize = 5;

impl ViewerSettings {
    /// Puts `addr` at the front of the recent hosts, without duplicates.
    pub fn remember_host(&mut self, addr: &str) {
        self.recent_hosts.retain(|known| known != addr);
        self.recent_hosts.insert(0, addr.to_string());
        self.recent_hosts.truncate(MAX_RECENT_HOSTS);
    }
}

impl Settings {
    /// Returns a copy with anything unusable replaced by a default.
    ///
    /// A settings file can be edited by hand or come from another machine, so
    /// a role this platform cannot run, a zero port, or an overlong recent
    /// list are all repaired rather than rejected.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        if !self.role.supported_here() {
            self.role = Role::default();
        }
        if self.host.port == 0 {
            self.host.port = DEFAULT_PORT;
        }
        self.viewer.recent_hosts.retain(|addr| !addr.trim().is_empty());
        self.viewer.recent_hosts.truncate(MAX_RECENT_HOSTS);
        self
    }
}

impl Settings {
    /// Loads settings from `path`.
    ///
    /// A missing or unreadable file gives the defaults: losing a preference
    /// is better than refusing to open.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default()
            .sanitized()
    }
}

impl Settings {
    /// Writes the settings to `path`, creating its folder if needed.
    ///
    /// The file is written next to its destination and renamed into place, so
    /// a crash halfway cannot leave a truncated file behind.
    ///
    /// # Errors
    /// Returns the I/O error if the folder or file cannot be written.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, path)
    }
}
