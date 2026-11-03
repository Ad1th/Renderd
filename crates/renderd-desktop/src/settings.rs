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
