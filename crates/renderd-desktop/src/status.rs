//! What the window shows about the running session.

use serde::Serialize;

/// Where a session is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Nothing is running.
    #[default]
    Stopped,
    /// The child process was started and has not reported in yet.
    Starting,
    /// The host is up and waiting for a viewer.
    Waiting,
    /// The two sides are finding each other or shaking hands.
    Connecting,
    /// Video is flowing.
    Streaming,
    /// The viewer lost the host and is trying to get it back.
    Reconnecting,
    /// The child was asked to stop.
    Stopping,
    /// The child ended on its own with a problem.
    Failed,
}
