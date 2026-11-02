//! What the window shows about the running session.

use serde::Serialize;

use crate::event::{Event, Metrics, StreamInfo};
use crate::problem::{diagnose, Problem, ProblemKind};
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

impl Status {
    /// Applies one event from the child process.
    ///
    /// Once a stop has been requested the phase stays at stopping, so late
    /// lines from the child cannot make the window look alive again.
    pub fn apply(&mut self, event: &Event) {
        if self.phase == Phase::Stopping {
            return;
        }
        match event {
            Event::Listening { .. } => self.phase = Phase::Waiting,
            Event::Address(addr) => self.address = Some(addr.clone()),
            Event::Searching => self.phase = Phase::Connecting,
            Event::Connecting { addr } => {
                self.address = Some(addr.clone());
                if self.phase != Phase::Reconnecting {
                    self.phase = Phase::Connecting;
                }
            }
            Event::PeerConnected { peer } => {
                self.peer = Some(peer.clone());
                self.phase = Phase::Connecting;
            }
            Event::Configured(info) => self.stream = Some(info.clone()),
            Event::Streaming => {
                self.phase = Phase::Streaming;
                self.problem = None;
            }
            Event::Metrics(sample) => self.metrics = self.metrics.merged(*sample),
            Event::Resized { width, height } => {
                if let Some(stream) = self.stream.as_mut() {
                    stream.width = *width;
                    stream.height = *height;
                }
            }
            Event::PeerLost => self.peer_lost(),
            Event::ConnectFailed(detail) => {
                self.problem = Some(Problem::new(ProblemKind::HostUnreachable, detail.clone()));
            }
            Event::CaptureFailed(detail) => {
                self.problem = Some(Problem::new(ProblemKind::ScreenRecording, detail.clone()));
            }
            Event::Stopping => self.phase = Phase::Stopping,
        }
    }

    /// The other side went away: the host waits again, the viewer retries.
    fn peer_lost(&mut self) {
        self.clear_session();
        self.phase = match self.role {
            Role::Host => Phase::Waiting,
            Role::Viewer => Phase::Reconnecting,
        };
    }
}

impl Status {
    /// Records that the child process ended.
    ///
    /// A child that was asked to stop, or that exited cleanly, leaves the
    /// session stopped. Anything else is a failure, explained by the last
    /// error the child logged when there is one.
    pub fn exited(&mut self, success: bool, last_error: Option<&str>) {
        let wanted = self.phase == Phase::Stopping;
        self.clear_session();
        self.address = None;
        if wanted || success {
            self.phase = Phase::Stopped;
            self.problem = None;
            return;
        }
        self.phase = Phase::Failed;
        self.problem = Some(last_error.map_or_else(
            || {
                Problem::new(
                    ProblemKind::Crashed,
                    "The process ended without an error message.",
                )
            },
            diagnose,
        ));
    }

    /// Records a failure that happened before the child could even start.
    pub fn fail(&mut self, problem: Problem) {
        self.clear_session();
        self.address = None;
        self.phase = Phase::Failed;
        self.problem = Some(problem);
    }
}
