//! Core of the Renderd desktop app.
//!
//! The app does not stream anything itself. It starts `renderd-host` or
//! `renderd-viewer` as a child process, reads the JSON log lines the child
//! writes, and turns them into a small status the window can show.
//!
//! The pieces, in the order a line of output passes through them:
//!
//! - [`supervisor`] starts the child and reads its output.
//! - [`logline`] parses each line.
//! - [`event`] decides whether the line changes the session.
//! - [`status`] holds what the window draws.
//! - [`problem`] explains failures in plain words.
//!
//! [`settings`], [`command`], [`address`] and [`sidecar`] cover what happens
//! before a start: what the user chose, the command line that follows from
//! it, and where the binary to run lives.

pub mod address;
pub mod command;
pub mod event;
pub mod logline;
pub mod problem;
pub mod role;
pub mod settings;
pub mod sidecar;
pub mod status;
pub mod supervisor;

pub use address::normalize_host;
pub use command::{host_args, viewer_args};
pub use event::{host_event, viewer_event, Event, Metrics, StreamInfo};
pub use logline::{Level, LogLine};
pub use problem::{diagnose, Problem, ProblemKind};
pub use role::Role;
pub use settings::{Codec, DisplayMode, HostSettings, Settings, ViewerSettings, DEFAULT_PORT};
pub use sidecar::{locate, locate_installed};
pub use status::{Phase, Status};
pub use supervisor::{Supervisor, Update};
