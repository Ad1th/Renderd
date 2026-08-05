//! User interface module scaffold for macOS host agent.

pub mod devices_panel;
pub mod menubar;
pub mod notifications;

pub use menubar::{MenuBar, MenuItemAction};

/// Host UI manager scaffold.
#[derive(Debug, Default, Clone)]
pub struct UiManager {
    /// Native macOS menu bar manager.
    pub menu_bar: MenuBar,
}

impl UiManager {
    /// Create a new UI manager scaffold.
    #[must_use]
    pub fn new() -> Self {
        Self {
            menu_bar: MenuBar::new(),
        }
    }

    /// Handles user selection of menu bar items.
    pub fn handle_menu_action(&self, action: MenuItemAction) {
        self.menu_bar.handle_action(action);
    }
}
