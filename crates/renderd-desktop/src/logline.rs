//! One line of output from a child process.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

/// A log line split into the parts the app cares about.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    /// Severity.
    pub level: Level,
    /// The human-readable message.
    pub message: String,
    /// Module that wrote the line, empty when unknown.
    pub target: String,
    /// Structured values attached to the message.
    #[serde(skip)]
    pub fields: Map<String, Value>,
}

impl LogLine {
    /// Wraps text that is not JSON, such as a panic or a clap usage error.
    ///
    /// Lines that start with `error` are marked as errors so they are not lost
    /// among the informational ones.
    #[must_use]
    pub fn plain(text: &str) -> Self {
        let message = strip_ansi(text).trim().to_string();
        let lowered = message.to_ascii_lowercase();
        let level = if lowered.starts_with("error") || lowered.contains("panicked at") {
            Level::Error
        } else {
            Level::Info
        };
        Self {
            level,
            message,
            target: String::new(),
            fields: Map::new(),
        }
    }
}

/// Removes ANSI colour escapes so text logs read cleanly in the window.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Skip "[", the parameters, and the final letter of the sequence.
        for next in chars.by_ref() {
            if next.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}
