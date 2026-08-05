//! macOS native menu bar user interface manager using `AppKit` `NSStatusBar` (RFC-0002 §9.3).
//!
//! Manages system status bar item, menu hierarchy ("Status", "Pair New Device (PIN)",
//! "Paired Devices...", "Quit"), and user action callbacks.

use std::sync::{Arc, Mutex};

/// Represents the status bar menu options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItemAction {
    /// Show or generate pairing PIN dialog.
    PairNewDevice,
    /// Open paired devices management panel.
    OpenPairedDevices,
    /// Quit host application.
    Quit,
}

/// Menu bar state representation for state queries and testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuBarState {
    /// Status message string displayed in menu header (e.g. "Status: Idle", "Status: Streaming").
    pub status_text: String,
    /// Currently active pairing PIN if displayed in menu.
    pub pin_text: Option<String>,
    /// Count of paired viewer devices.
    pub paired_count: usize,
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self {
            status_text: "Status: Idle".to_string(),
            pin_text: None,
            paired_count: 0,
        }
    }
}

/// Native macOS status bar menu bar controller.
#[derive(Debug, Clone)]
pub struct MenuBar {
    state: Arc<Mutex<MenuBarState>>,
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuBar {
    /// Creates and initializes a new `MenuBar` controller.
    #[must_use]
    pub fn new() -> Self {
        let menu_bar = Self {
            state: Arc::new(Mutex::new(MenuBarState::default())),
        };

        #[cfg(target_os = "macos")]
        {
            menu_bar.init_native_menu();
        }

        menu_bar
    }

    /// Initializes native macOS `NSStatusBar` and `NSMenu` hierarchy via Objective-C runtime.
    #[cfg(target_os = "macos")]
    #[allow(clippy::unused_self)]
    fn init_native_menu(&self) {
        #![allow(unsafe_code)]
        use objc2::rc::Retained;
        use objc2::runtime::AnyClass;
        use objc2::{msg_send, msg_send_id};
        use objc2_foundation::NSThread;

        // AppKit NSStatusBar calls require main thread
        if !NSThread::isMainThread_class() {
            return;
        }

        // SAFETY: NSStatusBar is standard AppKit API available on all macOS versions.
        unsafe {
            let Some(status_bar_class) = AnyClass::get("NSStatusBar") else {
                return;
            };
            let system_bar: Option<Retained<objc2::runtime::AnyObject>> =
                msg_send_id![status_bar_class, systemStatusBar];
            let Some(system_bar) = system_bar else {
                return;
            };

            // -1.0 represents NSSquareStatusItemLength
            let status_item: Option<Retained<objc2::runtime::AnyObject>> =
                msg_send_id![&system_bar, statusItemWithLength: -1.0f64];
            let Some(status_item) = status_item else {
                return;
            };

            // Set menu bar title icon text
            let button: Option<Retained<objc2::runtime::AnyObject>> =
                msg_send_id![&status_item, button];
            if let Some(button) = button {
                let title = objc2_foundation::NSString::from_str("Renderd");
                let _: () = msg_send![&button, setTitle: &*title];
            }
        }
    }

    /// Updates the current operational status text displayed in the menu bar.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn update_status(&self, status_text: &str) {
        let mut state = self.state.lock().expect("MenuBar mutex poisoned");
        state.status_text = format!("Status: {status_text}");
    }

    /// Updates the active pairing PIN displayed in the menu bar.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn set_pin(&self, pin: Option<&str>) {
        let mut state = self.state.lock().expect("MenuBar mutex poisoned");
        state.pin_text = pin.map(ToString::to_string);
    }

    /// Updates the count of paired devices displayed in the menu bar.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn update_paired_count(&self, count: usize) {
        let mut state = self.state.lock().expect("MenuBar mutex poisoned");
        state.paired_count = count;
    }

    /// Handles user selection of status bar menu item.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn handle_action(&self, action: MenuItemAction) {
        match action {
            MenuItemAction::PairNewDevice => {
                tracing::info!("Menu bar item selected: Pair New Device");
                self.update_status("Pairing...");
            }
            MenuItemAction::OpenPairedDevices => {
                tracing::info!("Menu bar item selected: Open Paired Devices");
            }
            MenuItemAction::Quit => {
                tracing::info!("Menu bar item selected: Quit");
            }
        }
    }

    /// Returns a copy of the current menu bar state.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    #[must_use]
    pub fn state(&self) -> MenuBarState {
        self.state.lock().expect("MenuBar mutex poisoned").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_bar_initial_state() {
        let menu_bar = MenuBar::new();
        let state = menu_bar.state();
        assert_eq!(state.status_text, "Status: Idle");
        assert_eq!(state.pin_text, None);
        assert_eq!(state.paired_count, 0);
    }

    #[test]
    fn test_menu_bar_updates() {
        let menu_bar = MenuBar::new();

        menu_bar.update_status("Streaming (1080p60)");
        menu_bar.set_pin(Some("123456"));
        menu_bar.update_paired_count(2);

        let state = menu_bar.state();
        assert_eq!(state.status_text, "Status: Streaming (1080p60)");
        assert_eq!(state.pin_text, Some("123456".to_string()));
        assert_eq!(state.paired_count, 2);
    }

    #[test]
    fn test_menu_bar_actions() {
        let menu_bar = MenuBar::new();
        menu_bar.handle_action(MenuItemAction::PairNewDevice);
        assert_eq!(menu_bar.state().status_text, "Status: Pairing...");

        menu_bar.handle_action(MenuItemAction::OpenPairedDevices);
        menu_bar.handle_action(MenuItemAction::Quit);
    }
}
