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

impl LogLine {
    /// Parses one line of child output.
    ///
    /// A JSON line from `--log-format json` keeps its level, target and fields.
    /// Anything else is kept as plain text, so nothing the child says is dropped.
    #[must_use]
    pub fn parse(line: &str) -> Self {
        Self::from_json(line).unwrap_or_else(|| Self::plain(line))
    }

    fn from_json(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let object = value.as_object()?;
        let mut fields = object.get("fields")?.as_object()?.clone();
        let message = match fields.remove("message") {
            Some(Value::String(text)) => text,
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let level = object
            .get("level")
            .and_then(Value::as_str)
            .and_then(Level::parse)
            .unwrap_or(Level::Info);
        let target = object
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Some(Self {
            level,
            message,
            target,
            fields,
        })
    }
}

impl LogLine {
    /// A field as text, if it is a string.
    #[must_use]
    pub fn text(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// A field as a number.
    ///
    /// The daemons format some numbers as strings (`fps = "59.8"`), so a string
    /// that parses as a number counts too.
    #[must_use]
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.fields.get(key)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// A field as a whole number that fits in `u32`.
    #[must_use]
    pub fn integer(&self, key: &str) -> Option<u32> {
        match self.fields.get(key)? {
            Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: &str = r#"{"timestamp":"2026-11-11T10:10:10.1Z","level":"INFO","fields":{"message":"HOST METRICS: transmit throughput","fps":"44.5","bitrate_kbps":"2712","send_queue_kb":0,"rtt_ms":"0.66"},"target":"renderd_host::network::data"}"#;

    #[test]
    fn parses_a_json_line() {
        let line = LogLine::parse(METRICS);
        assert_eq!(line.level, Level::Info);
        assert_eq!(line.message, "HOST METRICS: transmit throughput");
        assert_eq!(line.target, "renderd_host::network::data");
        assert!(!line.fields.contains_key("message"));
    }

    #[test]
    fn reads_numbers_written_as_strings() {
        let line = LogLine::parse(METRICS);
        assert_eq!(line.number("fps"), Some(44.5));
        assert_eq!(line.number("send_queue_kb"), Some(0.0));
        assert_eq!(line.integer("bitrate_kbps"), Some(2712));
        assert_eq!(line.number("missing"), None);
    }

    #[test]
    fn keeps_plain_text_lines() {
        let line = LogLine::parse("some stray output");
        assert_eq!(line.level, Level::Info);
        assert_eq!(line.message, "some stray output");
        assert!(line.fields.is_empty());
    }

    #[test]
    fn marks_error_text_as_an_error() {
        let line = LogLine::parse("Error: Initialization(\"Failed to bind QUIC server\")");
        assert_eq!(line.level, Level::Error);
        let line = LogLine::parse("thread 'main' panicked at src/main.rs:1:1");
        assert_eq!(line.level, Level::Error);
    }

    #[test]
    fn strips_ansi_colour_codes() {
        let line = LogLine::parse("\u{1b}[32m INFO\u{1b}[0m ready");
        assert_eq!(line.message, "INFO ready");
    }

    #[test]
    fn json_without_fields_is_treated_as_text() {
        let line = LogLine::parse("{\"hello\":1}");
        assert_eq!(line.message, "{\"hello\":1}");
        let line = LogLine::parse("[1,2,3]");
        assert_eq!(line.message, "[1,2,3]");
    }
}
