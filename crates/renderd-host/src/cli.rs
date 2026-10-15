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

    /// How the viewer's screen is used.
    ///
    /// `extend` (default) creates a virtual display sized to the viewer's monitor
    /// and streams that, so the viewer becomes an additional desktop. `mirror`
    /// streams the display selected by `--display-id` instead.
    #[arg(long, value_name = "MODE")]
    pub mode: Option<DisplayMode>,

    /// Favour smooth motion over a sharp picture on a slow link.
    ///
    /// By default the stream stays at full size and drops its resolution only
    /// while the link is unstable. With `--smooth` it drops whenever the bitrate
    /// cannot feed the full size, and sharpens the screen once it is still.
    /// Same as `host.smooth_motion = true` in the config file.
    #[arg(long)]
    pub smooth: bool,
}

/// Whether the viewer extends or mirrors the host desktop.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// Add a virtual display and stream it (the viewer is a second monitor).
    #[default]
    Extend,
    /// Stream an existing display.
    Mirror,
}
