//! Layered configuration management for Renderd host and viewer daemons.
//!
//! This crate defines the complete configuration schema for both the macOS host daemon
//! and the Windows viewer client, and provides validated loading via a Figment-based
//! priority stack: CLI flags → environment variables → user config file →
//! compiled-in defaults.
//!
//! # Architecture
//!
//! The crate is organized into four modules:
//!
//! - [`schema`] — [`RenderdConfig`], [`HostConfig`], [`ViewerConfig`],
//!   [`NetworkConfig`], [`AbrConfig`] structs with `serde` defaults and documentation.
//! - [`load`] — [`ConfigBuilder`] implementing the Figment-based priority loader.
//!   Config file path resolution is the **binary's** responsibility; this module
//!   accepts a resolved `Option<&Path>`.
//! - [`validate`] — [`ValidateConfig`] trait with cross-field invariant checks
//!   (e.g., `min_bitrate_kbps < max_bitrate_kbps`).
//! - [`error`] — [`ConfigError`] enum covering I/O, parse, and validation failures.
//!
//! # Usage
//!
//! ```rust,no_run
//! use renderd_config::ConfigBuilder;
//! use std::path::Path;
//!
//! // In the host binary: resolve config path first, then load and validate
//! let config = ConfigBuilder::new()
//!     .add_file(Path::new("/path/to/host.toml"))
//!     .build()
//!     .expect("configuration error");
//! ```
//!
//! # Panics
//!
//! This crate does not panic. All fallible operations return [`Result<T, ConfigError>`].
//!
//! # Platform Support
//!
//! The schema and loader are cross-platform. Platform-specific config file path
//! resolution (`~/Library/Application Support/...` on macOS,
//! `%APPDATA%\...` on Windows) is implemented in the application binaries
//! (`renderd-host`, `renderd-viewer`).

pub mod error;
pub mod load;
pub mod schema;
pub mod validate;

pub use error::*;
pub use load::*;
pub use schema::*;
pub use validate::*;
