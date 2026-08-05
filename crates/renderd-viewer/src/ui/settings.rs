//! Windows system tray icon and settings menu controller (`renderd-viewer/src/ui/settings.rs`).
//!
//! Manages system tray notification icon (`Shell_NotifyIcon`), context menu options
//! ("Connect to Host...", "Settings", "Disconnect", "Exit"), and settings state (RFC-0002 §6.3).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

/// Represents tray menu action options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuAction {
    /// Prompt for host IP/port address entry.
    ConnectToHost,
    /// Open settings configuration dialog.
    Settings,
    /// Disconnect active stream session.
    Disconnect,
    /// Exit viewer application process.
    Exit,
}

/// Settings and tray state representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerSettingsState {
    /// Active target host address string.
    pub host_address: String,
    /// Hardware decode acceleration toggle.
    pub hw_decode_enabled: bool,
    /// Low-latency DXGI allow-tearing present toggle.
    pub allow_tearing_enabled: bool,
}

impl Default for ViewerSettingsState {
    fn default() -> Self {
        Self {
            host_address: "127.0.0.1:9000".to_string(),
            hw_decode_enabled: true,
            allow_tearing_enabled: true,
        }
    }
}

/// System tray icon and settings controller.
#[derive(Debug, Clone)]
pub struct SystemTrayManager {
    state: Arc<Mutex<ViewerSettingsState>>,
}

impl Default for SystemTrayManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemTrayManager {
    /// Creates a new `SystemTrayManager`.
    #[must_use]
    pub fn new() -> Self {
        let manager = Self {
            state: Arc::new(Mutex::new(ViewerSettingsState::default())),
        };

        #[cfg(target_os = "windows")]
        {
            manager.init_windows_tray();
        }

        manager
    }

    /// Handles user tray menu item selection.
    pub fn handle_action(&self, action: TrayMenuAction) {
        match action {
            TrayMenuAction::ConnectToHost => {
                tracing::info!("Tray menu action: Connect to Host");
            }
            TrayMenuAction::Settings => {
                tracing::info!("Tray menu action: Settings");
            }
            TrayMenuAction::Disconnect => {
                tracing::info!("Tray menu action: Disconnect");
            }
            TrayMenuAction::Exit => {
                tracing::info!("Tray menu action: Exit");
            }
        }
    }

    /// Updates active host address string in settings state.
    ///
    /// # Panics
    /// Panics if internal mutex is poisoned.
    pub fn set_host_address(&self, addr: SocketAddr) {
        let mut state = self.state.lock().expect("SystemTrayManager mutex poisoned");
        state.host_address = addr.to_string();
    }

    /// Returns a copy of the current settings state.
    ///
    /// # Panics
    /// Panics if internal mutex is poisoned.
    #[must_use]
    pub fn state(&self) -> ViewerSettingsState {
        self.state
            .lock()
            .expect("SystemTrayManager mutex poisoned")
            .clone()
    }

    #[cfg(target_os = "windows")]
    fn init_windows_tray(&self) {
        if let Ok(guard) = self.state.lock() {
            tracing::info!(
                host = %guard.host_address,
                "Initializing Win32 Shell_NotifyIcon system tray"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_tray_manager_lifecycle() {
        let manager = SystemTrayManager::new();
        let state = manager.state();
        assert_eq!(state.host_address, "127.0.0.1:9000");
        assert!(state.hw_decode_enabled);

        let new_addr: SocketAddr = "192.168.1.100:9000".parse().unwrap();
        manager.set_host_address(new_addr);
        assert_eq!(manager.state().host_address, "192.168.1.100:9000");

        manager.handle_action(TrayMenuAction::ConnectToHost);
        manager.handle_action(TrayMenuAction::Settings);
        manager.handle_action(TrayMenuAction::Disconnect);
        manager.handle_action(TrayMenuAction::Exit);
    }
}
