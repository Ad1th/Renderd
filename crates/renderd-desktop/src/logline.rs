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
