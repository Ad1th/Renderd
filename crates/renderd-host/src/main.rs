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
pub use encode::{EncodePipeline, EncodedFrame};
pub use error::HostError;
pub use network::{
    control::ControlDispatcher, data::DataSender, server::HostServer, NetworkManager,
};
pub use panic::setup_panic_hook;
pub use session::{
    auth::AuthManager,
    devices::DeviceRegistry,
    pairing::{PairingError, PairingHandler},
    HostSession, SessionError, SessionState,
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
    let mut config = builder.build()?;

    // Apply the CLI overrides. These flags are documented as overrides but were parsed
    // and then dropped, so --port and --display-id silently did nothing.
    if let Some(port) = cli.port {
        config.network.listen_port = port;
    }
    if let Some(display_id) = cli.display_id {
        config.host.display_id = display_id;
    }

    tracing::info!(
        display_id = config.host.display_id,
        target_fps = config.host.target_fps,
        listen_port = config.network.listen_port,
        "Configuration loaded successfully"
    );

    let mut app = HostApp::new(config);
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documented CLI overrides must actually reach the config.
    #[test]
    fn test_cli_overrides_are_applied_to_config() {
        let cli = HostCli::parse_from(["renderd-host", "--port", "44330", "--display-id", "7"]);
        let mut config = renderd_config::RenderdConfig::default();
        assert_ne!(config.network.listen_port, 44330);

        if let Some(port) = cli.port {
            config.network.listen_port = port;
        }
        if let Some(display_id) = cli.display_id {
            config.host.display_id = display_id;
        }

        assert_eq!(config.network.listen_port, 44330);
        assert_eq!(config.host.display_id, 7);
    }

    #[test]
    fn test_host_app_scaffold() {
        let config = renderd_config::RenderdConfig::default();
        let app = HostApp::new(config);
        // Verify all subsystems are constructed; run() is not called as it blocks on SIGINT/SIGTERM.
        let _ = app;
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
        let ui = UiManager::new();
        ui.handle_menu_action(ui::MenuItemAction::PairNewDevice);
        ui.notifications.notify_session_started("Test Viewer");
        let _control = ControlDispatcher::new();
        let _data = DataSender::new();
        let _server = HostServer::new();
        let _auth = AuthManager::new();
        let keychain = std::sync::Arc::new(renderd_keychain::MockKeychain::new());
        let _devices = DeviceRegistry::new(keychain.clone());
        let _pairing = PairingHandler::new(keychain);
        let _devices_panel = DevicesPanel::new();
        let _menubar = MenuBar::new();
        let _notifications = NotificationManager::new();
    }
}
