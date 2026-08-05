//! macOS host display mirroring agent application entry point.

mod abr;
mod app;
mod autostart;
mod capture;
mod cli;
mod clock;
mod encode;
mod error;
mod network;
mod panic;
mod session;
mod ui;

pub use abr::AbrManager;
pub use app::HostApp;
pub use autostart::{AutoStart, AutoStartManager, AutoStartStatus};
pub use capture::CapturePipeline;
pub use cli::HostCli;
pub use clock::ClockController;
pub use encode::EncodePipeline;
pub use error::HostError;
pub use network::{
    control::ControlDispatcher, data::DataSender, server::HostServer, NetworkManager,
};
pub use panic::setup_panic_hook;
pub use session::{
    auth::AuthManager, devices::DeviceRegistry, pairing::PairingHandler, HostSession, SessionError,
    SessionState,
};
pub use ui::{
    devices_panel::DevicesPanel, menubar::MenuBar, notifications::NotificationManager, UiManager,
};

use clap::Parser;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn init_logging(log_level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .try_init();
}

fn main() -> Result<(), HostError> {
    setup_panic_hook();

    let cli = HostCli::parse();
    init_logging(&cli.log_level);

    tracing::info!(
        log_level = %cli.log_level,
        config_path = ?cli.config,
        "Starting renderd-host application"
    );

    let mut builder = renderd_config::ConfigBuilder::new();
    if let Some(ref path) = cli.config {
        builder = builder.add_file(path);
    }
    let config = builder.build()?;

    tracing::info!(
        display_id = config.host.display_id,
        target_fps = config.host.target_fps,
        "Configuration loaded successfully"
    );

    let app = HostApp::new();
    app.run()?;
    println!("Hello from renderd-host");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_app_scaffold() {
        let app = HostApp::new();
        assert!(app.run().is_ok());
    }

    #[test]
    fn test_cli_argument_parsing() {
        let cli = HostCli::try_parse_from(["renderd-host", "--log-level", "debug"]).unwrap();
        assert_eq!(cli.log_level, "debug");
        assert!(cli.config.is_none());
    }

    #[test]
    fn test_panic_hook_setup() {
        setup_panic_hook();
    }

    #[test]
    fn test_init_logging() {
        init_logging("warn");
    }

    #[test]
    fn test_autostart_scaffold() {
        let _autostart = AutoStart::new();
        let _status = AutoStart::status();
    }

    #[test]
    fn test_module_scaffolds_instantiation() {
        let _abr = AbrManager::new();
        let _autostart = AutoStartManager::new();
        let _capture = CapturePipeline::new();
        let _clock = ClockController::new();
        let _encode = EncodePipeline::new();
        let _network = NetworkManager::new();
        let _session = HostSession::new();
        let _ui = UiManager::new();
        let _control = ControlDispatcher::new();
        let _data = DataSender::new();
        let _server = HostServer::new();
        let _auth = AuthManager::new();
        let _devices = DeviceRegistry::new();
        let _pairing = PairingHandler::new();
        let _devices_panel = DevicesPanel::new();
        let _menubar = MenuBar::new();
        let _notifications = NotificationManager::new();
    }
}
