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
