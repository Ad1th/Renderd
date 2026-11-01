//! Things that go wrong, in words a person can act on.

use serde::Serialize;

/// The kind of trouble, so the window can offer the right fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// macOS has not allowed screen recording yet.
    ScreenRecording,
    /// Another program already uses the port.
    PortInUse,
    /// The host or viewer binary is not where the app expects it.
    MissingBinary,
    /// The viewer could not reach the host.
    HostUnreachable,
    /// The child process ended without being asked to.
    Crashed,
    /// Anything else. The detail carries the raw message.
    Other,
}
