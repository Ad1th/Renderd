//! Renderd Viewer executable entrypoint.

use renderd_viewer::{App, ViewerAppConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    tracing::info!("Starting Renderd Viewer...");
    let config = ViewerAppConfig::default();
    let app = App::new(config);
    app.run()?;

    Ok(())
}
