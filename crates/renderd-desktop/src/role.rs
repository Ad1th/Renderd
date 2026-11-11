//! Which side of the stream this computer plays.

use serde::{Deserialize, Serialize};

/// The part this computer plays in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Shares its screen. Runs `renderd-host`.
    Host,
    /// Shows another computer's screen. Runs `renderd-viewer`.
    Viewer,
}

impl Role {
    /// File name of the binary that does the work, without an extension.
    #[must_use]
    pub const fn binary_name(self) -> &'static str {
        match self {
            Self::Host => "renderd-host",
            Self::Viewer => "renderd-viewer",
        }
    }
}

impl Role {
    /// The role a fresh install starts in.
    ///
    /// The host only runs on macOS, so a Mac shares and everything else views.
    #[must_use]
    pub const fn default_for_platform() -> Self {
        if cfg!(target_os = "macos") {
            Self::Host
        } else {
            Self::Viewer
        }
    }

    /// Whether this role can run on the current operating system at all.
    #[must_use]
    pub const fn supported_here(self) -> bool {
        match self {
            Self::Host => cfg!(target_os = "macos"),
            Self::Viewer => true,
        }
    }
}

impl Default for Role {
    fn default() -> Self {
        Self::default_for_platform()
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Host => "host",
            Self::Viewer => "viewer",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_names_match_the_workspace_binaries() {
        assert_eq!(Role::Host.binary_name(), "renderd-host");
        assert_eq!(Role::Viewer.binary_name(), "renderd-viewer");
    }

    #[test]
    fn default_role_is_supported_on_this_platform() {
        assert!(Role::default().supported_here());
    }

    #[test]
    fn viewer_is_supported_everywhere() {
        assert!(Role::Viewer.supported_here());
    }
}
