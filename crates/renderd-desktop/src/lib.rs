//! Core of the Renderd desktop app.
//!
//! The app does not stream anything itself. It starts `renderd-host` or
//! `renderd-viewer` as a child process, reads the JSON log lines the child
//! writes, and turns them into a small status the window can show.

pub mod logline;
pub mod role;

pub use logline::Level;
pub use role::Role;
