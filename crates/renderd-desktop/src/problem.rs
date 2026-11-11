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

/// A problem with a short title and the detail behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// What kind of problem this is.
    pub kind: ProblemKind,
    /// One short sentence for the window.
    pub title: String,
    /// The underlying message, for the details view.
    pub detail: String,
}

impl Problem {
    /// Builds a problem of `kind` with its standard title.
    #[must_use]
    pub fn new(kind: ProblemKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            title: kind.title().to_string(),
            detail: detail.into(),
        }
    }
}

impl ProblemKind {
    /// The sentence shown for this kind of problem.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::ScreenRecording => "Renderd needs permission to record the screen",
            Self::PortInUse => "Another app is already using this port",
            Self::MissingBinary => "Part of Renderd is missing",
            Self::HostUnreachable => "Could not reach that computer",
            Self::Crashed => "Renderd stopped unexpectedly",
            Self::Other => "Something went wrong",
        }
    }
}

/// Works out what kind of problem an error message describes.
#[must_use]
pub fn diagnose(message: &str) -> Problem {
    let lowered = message.to_ascii_lowercase();
    let kind = if lowered.contains("address already in use") || lowered.contains("os error 10048") {
        ProblemKind::PortInUse
    } else if lowered.contains("screen recording")
        || lowered.contains("screen capture")
        || lowered.contains("not authorized")
        || lowered.contains("tcc")
    {
        ProblemKind::ScreenRecording
    } else if lowered.contains("timed out") || lowered.contains("unreachable") {
        ProblemKind::HostUnreachable
    } else {
        ProblemKind::Other
    };
    Problem::new(kind, message.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_port_is_recognised() {
        let problem = diagnose(
            "Error: Initialization(\"Failed to bind UDP socket on 0.0.0.0:4433: Address already in use (os error 48)\")",
        );
        assert_eq!(problem.kind, ProblemKind::PortInUse);
        assert!(problem.detail.contains("0.0.0.0:4433"));
    }

    #[test]
    fn the_windows_port_error_is_recognised() {
        let problem = diagnose("Only one usage of each socket address (os error 10048)");
        assert_eq!(problem.kind, ProblemKind::PortInUse);
    }

    #[test]
    fn a_capture_permission_error_is_recognised() {
        let problem = diagnose("stream start failed: user is not authorized for screen capture");
        assert_eq!(problem.kind, ProblemKind::ScreenRecording);
    }

    #[test]
    fn a_timeout_means_the_host_is_unreachable() {
        let problem = diagnose("QUIC connection to 10.0.0.9:4433 failed: timed out");
        assert_eq!(problem.kind, ProblemKind::HostUnreachable);
    }
}
