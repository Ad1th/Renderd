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
