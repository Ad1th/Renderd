//! Command-line argument parser for `renderd-host`.

use clap::Parser;
use std::path::PathBuf;

/// macOS host display mirroring agent daemon.
#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(name = "renderd-host", author, version, about)]
pub struct HostCli {
    /// Path to optional TOML configuration file.
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// Logging level (trace, debug, info, warn, error).
    #[arg(short, long, default_value = "info")]
    pub log_level: String,

    /// Target display ID override.
    #[arg(long)]
    pub display_id: Option<u32>,

    /// Listening UDP port override.
    #[arg(short, long)]
    pub port: Option<u16>,
}
