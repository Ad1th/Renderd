//! macOS User Notifications integration (RFC-0002 §9.3).
//!
//! Posts system notifications when streaming sessions start or new devices are paired.

use std::sync::{Arc, Mutex};

/// Represents a posted user notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Notification title text.
    pub title: String,
    /// Notification body message.
    pub body: String,
}

/// Notification history state for state inspection and testing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NotificationState {
    /// Log of all posted notifications.
    pub history: Vec<Notification>,
}

/// User notification manager handling system alerts.
#[derive(Debug, Clone)]
pub struct NotificationManager {
    state: Arc<Mutex<NotificationState>>,
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationManager {
    /// Creates a new `NotificationManager`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(NotificationState::default())),
        }
    }

    /// Emits a system notification when a viewer starts screen sharing.
    ///
    /// Message format: `"[Viewer Name] started screen sharing"`
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn notify_session_started(&self, viewer_name: &str) {
        let title = "Renderd Screen Sharing";
        let body = format!("{viewer_name} started screen sharing");
        self.post_notification(title, &body);
    }

    /// Emits a system notification when a new device is paired.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn notify_device_paired(&self, viewer_name: &str) {
        let title = "Renderd Device Paired";
        let body = format!("New device paired: {viewer_name}");
        self.post_notification(title, &body);
    }

    /// Posts a user notification with title and body text.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    pub fn post_notification(&self, title: &str, body: &str) {
        tracing::info!(title = %title, body = %body, "Posting user notification");

        let notification = Notification {
            title: title.to_string(),
            body: body.to_string(),
        };

        {
            let mut state = self
                .state
                .lock()
                .expect("NotificationManager mutex poisoned");
            state.history.push(notification);
        }

        #[cfg(target_os = "macos")]
        {
            self.send_native_notification(title, body);
        }
    }

    /// Sends native macOS notification via `NSUserNotificationCenter`.
    #[cfg(target_os = "macos")]
    #[allow(clippy::unused_self)]
    fn send_native_notification(&self, title: &str, body: &str) {
        #![allow(unsafe_code)]
        use objc2::rc::Retained;
        use objc2::runtime::AnyClass;
        use objc2::{msg_send, msg_send_id};

        unsafe {
            let Some(center_class) = AnyClass::get("NSUserNotificationCenter") else {
                return;
            };
            let center: Option<Retained<objc2::runtime::AnyObject>> =
                msg_send_id![center_class, defaultUserNotificationCenter];
            let Some(center) = center else {
                return;
            };

            let Some(notif_class) = AnyClass::get("NSUserNotification") else {
                return;
            };
            let notification: Option<Retained<objc2::runtime::AnyObject>> =
                msg_send_id![notif_class, new];
            let Some(notification) = notification else {
                return;
            };

            let ns_title = objc2_foundation::NSString::from_str(title);
            let ns_body = objc2_foundation::NSString::from_str(body);

            let _: () = msg_send![&notification, setTitle: &*ns_title];
            let _: () = msg_send![&notification, setInformativeText: &*ns_body];

            let _: () = msg_send![&center, deliverNotification: &*notification];
        }
    }

    /// Returns all posted notifications in history.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    #[must_use]
    pub fn history(&self) -> Vec<Notification> {
        self.state
            .lock()
            .expect("NotificationManager mutex poisoned")
            .history
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notify_session_started() {
        let manager = NotificationManager::new();
        manager.notify_session_started("iPad Pro");

        let history = manager.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "Renderd Screen Sharing");
        assert_eq!(history[0].body, "iPad Pro started screen sharing");
    }

    #[test]
    fn test_notify_device_paired() {
        let manager = NotificationManager::new();
        manager.notify_device_paired("Studio Display Viewer");

        let history = manager.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "Renderd Device Paired");
        assert_eq!(history[0].body, "New device paired: Studio Display Viewer");
    }
}
