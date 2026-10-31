//! One line of output from a child process.

use serde::{Deserialize, Serialize};

/// How serious a log line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Very detailed tracing.
    Trace,
    /// Details useful when debugging.
    Debug,
    /// Normal progress.
    Info,
    /// Something looked wrong but work continues.
    Warn,
    /// Something failed.
    Error,
}

impl Level {
    /// Reads a level name as tracing writes it, in any letter case.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}
