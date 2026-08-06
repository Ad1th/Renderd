//! Viewer-side host discovery orchestration (`renderd-viewer/src/discovery/mod.rs`).
//!
//! Launches the platform mDNS browser on startup, collects [`ServiceRecord`]s
//! as hosts appear or disappear, and surfaces the current host list so the
//! UI layer (`SystemTrayManager`, `StatusOverlay`) can present them to the user.
//!
//! The [`DiscoveryManager`] is designed to be cloned freely — the inner state
//! is held behind an `Arc<Mutex<…>>` so it is safe to share across the tokio
//! task that receives mDNS events and the winit event loop that reads the list.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use renderd_discovery::{Browser, DiscoveryEvent, ManualBrowser, ServiceRecord};
use uuid::Uuid;

/// Snapshot of currently known hosts as seen via mDNS or manual entry.
#[derive(Debug, Clone, Default)]
pub struct DiscoveredHosts {
    /// Map from host UUID to most recently seen [`ServiceRecord`].
    pub hosts: HashMap<Uuid, ServiceRecord>,
}

impl DiscoveredHosts {
    /// Returns the socket address of the most recently discovered host, or `None`
    /// if no hosts are known.
    #[must_use]
    pub fn primary_addr(&self) -> Option<SocketAddr> {
        self.hosts
            .values()
            .next()
            .map(|r| SocketAddr::new(r.addr, r.port))
    }
}

/// Thread-safe viewer host discovery manager.
///
/// Wraps a platform mDNS browser, a `ManualBrowser` fallback, and a
/// shared [`DiscoveredHosts`] snapshot that is updated as events arrive.
#[derive(Clone)]
pub struct DiscoveryManager {
    hosts: Arc<Mutex<DiscoveredHosts>>,
}

impl std::fmt::Debug for DiscoveryManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveryManager")
            .field(
                "host_count",
                &self.hosts.lock().map(|g| g.hosts.len()).unwrap_or_default(),
            )
            .finish()
    }
}

impl DiscoveryManager {
    /// Creates a new [`DiscoveryManager`] with an empty host table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hosts: Arc::new(Mutex::new(DiscoveredHosts::default())),
        }
    }

    /// Returns a snapshot of all currently discovered hosts.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    #[must_use]
    pub fn snapshot(&self) -> DiscoveredHosts {
        self.hosts
            .lock()
            .expect("DiscoveryManager mutex poisoned")
            .clone()
    }

    /// Registers a manual fallback host address, emitting an immediate
    /// `DiscoveryEvent::Found` so it appears in the host list.
    ///
    /// # Errors
    /// Returns a `String` error if the `ManualBrowser` fails to start.
    pub fn add_manual(&self, addr: SocketAddr, name: impl Into<String>) -> Result<(), String> {
        let mut browser = ManualBrowser::new(addr, name);
        let mut rx = browser
            .start_browse()
            .map_err(|e| format!("ManualBrowser start failed: {e}"))?;

        let hosts = Arc::clone(&self.hosts);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                Self::apply_event_to(&hosts, event);
            }
        });
        Ok(())
    }

    /// Starts the platform mDNS browser in a background tokio task, updating the
    /// internal host table as `DiscoveryEvent`s arrive.
    ///
    /// # Errors
    /// Returns a `String` error description if the browser fails to start.
    pub fn start_platform_browse(&self) -> Result<(), String> {
        let mut browser = Self::new_platform_browser()?;
        let rx = browser
            .start_browse()
            .map_err(|e| format!("Platform browser start failed: {e}"))?;

        let hosts = Arc::clone(&self.hosts);
        tokio::spawn(async move {
            // Hold `browser` alive for the duration so it is not dropped / unregistered.
            let _browser = browser;
            let mut rx = rx;
            while let Some(event) = rx.recv().await {
                Self::apply_event_to(&hosts, event);
            }
        });
        Ok(())
    }

    /// Applies a single [`DiscoveryEvent`] to the shared host table.
    fn apply_event_to(hosts: &Arc<Mutex<DiscoveredHosts>>, event: DiscoveryEvent) {
        let mut guard = hosts.lock().expect("DiscoveryManager mutex poisoned");
        match event {
            DiscoveryEvent::Found(record) => {
                tracing::info!(
                    host_id = %record.host_id,
                    addr = ?record.addr,
                    port = record.port,
                    name = %record.name,
                    "mDNS: host discovered"
                );
                guard.hosts.insert(record.host_id, record);
            }
            DiscoveryEvent::Lost(host_id) => {
                tracing::info!(%host_id, "mDNS: host went offline");
                guard.hosts.remove(&host_id);
            }
        }
    }

    /// Constructs the platform-appropriate mDNS browser.
    ///
    /// Returns `Err` on platforms where no mDNS browser is compiled in.
    fn new_platform_browser() -> Result<Box<dyn Browser>, String> {
        Self::platform_browser_impl()
    }

    #[cfg(target_os = "macos")]
    #[allow(clippy::unnecessary_wraps)]
    fn platform_browser_impl() -> Result<Box<dyn Browser>, String> {
        use renderd_discovery::BonjourBrowser;
        Ok(Box::new(BonjourBrowser::new()))
    }

    #[cfg(target_os = "windows")]
    #[allow(clippy::unnecessary_wraps)]
    fn platform_browser_impl() -> Result<Box<dyn Browser>, String> {
        use renderd_discovery::WinDnsBrowser;
        Ok(Box::new(WinDnsBrowser::new()))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn platform_browser_impl() -> Result<Box<dyn Browser>, String> {
        Err("No platform mDNS browser available on this OS".to_string())
    }
}

impl Default for DiscoveryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_manager_manual_add() {
        let mgr = DiscoveryManager::new();
        let addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();
        mgr.add_manual(addr, "Local Test Host").unwrap();

        // Give the background task a tick to run
        tokio::task::yield_now().await;

        let snap = mgr.snapshot();
        assert_eq!(snap.hosts.len(), 1);
        let primary = snap.primary_addr().unwrap();
        assert_eq!(primary, addr);
    }

    #[test]
    fn test_discovered_hosts_primary_addr_empty() {
        let hosts = DiscoveredHosts::default();
        assert!(hosts.primary_addr().is_none());
    }
}
