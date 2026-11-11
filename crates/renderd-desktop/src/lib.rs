//! Core of the Renderd desktop app.
//!
//! The app does not stream anything itself. It starts `renderd-host` or
//! `renderd-viewer` as a child process, reads the JSON log lines the child
//! writes, and turns them into a small status the window can show.

pub mod event;
pub mod logline;
pub mod problem;
pub mod role;
pub mod settings;
pub mod status;

pub use event::{host_event, viewer_event, Event, Metrics, StreamInfo};
pub use logline::{Level, LogLine};
pub use problem::{diagnose, Problem, ProblemKind};
pub use role::Role;
pub use settings::{Codec, DisplayMode, HostSettings, ViewerSettings, DEFAULT_PORT};
pub use status::{Phase, Status};
