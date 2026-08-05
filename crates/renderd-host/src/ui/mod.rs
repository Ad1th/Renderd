//! User interface module scaffold for macOS host agent.

pub mod devices_panel;
pub mod menubar;
pub mod notifications;

pub use menubar::{MenuBar, MenuItemAction};
pub use notifications::NotificationManager;

/// Host UI manager scaffold.
#[derive(Debug, Default, Clone)]
pub struct UiManager {
    /// Native macOS menu bar manager.
    pub menu_bar: MenuBar,
    /// User notification manager.
    pub notifications: NotificationManager,
}

impl UiManager {
    /// Create a new UI manager scaffold.
    #[must_use]
    pub fn new() -> Self {
        Self {
            menu_bar: MenuBar::new(),
            notifications: NotificationManager::new(),
        }
    }

    /// Handles user selection of menu bar items.
    pub fn handle_menu_action(&self, action: MenuItemAction) {
        self.menu_bar.handle_action(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notifications::Notification;

    #[test]
    fn test_ui_manager_notifications() {
        let ui = UiManager::new();
        ui.notifications.notify_device_paired("Test Device");
        let history = ui.notifications.history();
        assert_eq!(history.len(), 1);
        let notif: Notification = history[0].clone();
        assert_eq!(notif.title, "Renderd Device Paired");
    }
}
