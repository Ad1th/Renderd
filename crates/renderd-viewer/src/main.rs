//! Renderd Viewer executable entrypoint.

use clap::Parser;
use renderd_viewer::{parse_host_arg, App, ViewerAppConfig, ViewerCli};
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = ViewerCli::parse();

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut config = ViewerAppConfig::load().unwrap_or_else(|e| {
        tracing::warn!("Falling back to default viewer config: {e}");
        ViewerAppConfig::default()
    });

    if let Some(ref host) = cli.host {
        let addr = parse_host_arg(host)?;
        tracing::info!(host_addr = %addr, "Host address supplied on the command line; skipping mDNS discovery");
        config.manual_host = Some(addr);
    }

    config.decoder_backend = cli.decoder;
    config.codec_choice = cli.codec;

    if cli.fullscreen {
        config.fullscreen = true;
    }
    if let Some(width) = cli.width {
        config.window_width = width;
    }
    if let Some(height) = cli.height {
        config.window_height = height;
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        manual_host = ?config.manual_host,
        "Starting Renderd Viewer..."
    );

    let app = App::new(config);
    app.run()?;

    Ok(())
}
