//! macOS host display mirroring agent application entry point.

mod abr;
mod app;
mod autostart;
mod capture;
mod clock;
mod encode;
mod error;
mod network;
mod session;
mod ui;

pub use abr::AbrManager;
pub use app::HostApp;
pub use autostart::AutoStartManager;
pub use capture::CapturePipeline;
pub use clock::ClockController;
pub use encode::EncodePipeline;
pub use error::HostError;
pub use network::{
    control::ControlDispatcher, data::DataSender, server::HostServer, NetworkManager,
};
pub use session::{
    auth::AuthManager, devices::DeviceRegistry, pairing::PairingHandler, HostSession,
};
pub use ui::{
    devices_panel::DevicesPanel, menubar::MenuBar, notifications::NotificationManager, UiManager,
};

fn main() -> Result<(), HostError> {
    tracing::info!("Initializing renderd-host application scaffold");
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
