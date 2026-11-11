//! What the window shows about the running session.

use serde::Serialize;

use crate::event::{Metrics, StreamInfo};
use crate::problem::Problem;
use crate::role::Role;

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

impl Phase {
    /// Whether a child process is expected to be alive in this phase.
    #[must_use]
    pub const fn is_running(self) -> bool {
        !matches!(self, Self::Stopped | Self::Failed)
    }
}

/// Everything the window needs to draw the current session.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// Which side this computer plays.
    pub role: Role,
    /// Where the session is.
    pub phase: Phase,
    /// Host: the address to give the viewer. Viewer: the address being dialled.
    pub address: Option<String>,
    /// Address of the other computer once it is connected.
    pub peer: Option<String>,
    /// The picture format in use.
    pub stream: Option<StreamInfo>,
    /// Latest live numbers.
    pub metrics: Metrics,
    /// The most recent problem, if it still applies.
    pub problem: Option<Problem>,
}

impl Status {
    /// A stopped session for `role`.
    #[must_use]
    pub fn new(role: Role) -> Self {
        Self {
            role,
            phase: Phase::Stopped,
            address: None,
            peer: None,
            stream: None,
            metrics: Metrics::default(),
            problem: None,
        }
    }
}

impl Status {
    /// Marks the session as starting and forgets the previous run.
    pub fn begin(&mut self) {
        *self = Self::new(self.role);
        self.phase = Phase::Starting;
    }

    /// Records that the user asked the session to stop.
    pub fn begin_stop(&mut self) {
        if self.phase.is_running() {
            self.phase = Phase::Stopping;
        }
    }

    /// Forgets everything about the other side.
    fn clear_session(&mut self) {
        self.peer = None;
        self.stream = None;
        self.metrics = Metrics::default();
    }
}
