//! Deciding what to run when the user presses start.

use crate::address::normalize_host;
use crate::command::{host_args, viewer_args};
use crate::role::Role;
use crate::settings::Settings;

/// What to start, worked out from the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Which binary to run.
    pub role: Role,
    /// Its arguments.
    pub args: Vec<String>,
    /// Viewer only: the address to dial, already normalised. `None` lets the
    /// viewer find a host on the network itself.
    pub host: Option<String>,
}
