//! Manual IP address resolution fallback for environments where mDNS multicast is blocked.

use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::mpsc::{channel, Receiver};
use uuid::Uuid;

use crate::error::DiscoveryError;
use crate::record::{DiscoveryEvent, ServiceRecord};
use crate::traits::Browser;

/// Fallback browser that produces a [`DiscoveryEvent::Found`] from a static IP/port string.
#[derive(Debug, Clone)]
pub struct ManualBrowser {
    addr: SocketAddr,
    name: String,
}

impl ManualBrowser {
    /// Creates a [`ManualBrowser`] from a `host:port` string (e.g. `"192.168.1.50:9000"`).
    ///
    /// # Errors
    /// Returns [`DiscoveryError::InvalidRecord`] if address parsing fails.
    pub fn parse(addr_str: &str) -> Result<Self, DiscoveryError> {
        let addr: SocketAddr = addr_str.parse().map_err(|e| {
            DiscoveryError::InvalidRecord(format!("Invalid socket address '{addr_str}': {e}"))
        })?;

        Ok(Self {
            addr,
            name: format!("Manual ({})", addr.ip()),
        })
    }

    /// Creates a [`ManualBrowser`] from an explicit [`SocketAddr`] and display name.
    #[must_use]
    pub fn new(addr: SocketAddr, name: impl Into<String>) -> Self {
        Self {
            addr,
            name: name.into(),
        }
    }
}

impl Browser for ManualBrowser {
    fn start_browse(&mut self) -> Result<Receiver<DiscoveryEvent>, DiscoveryError> {
        let (tx, rx) = channel(4);

        let host_id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, self.addr.to_string().as_bytes());
        let record = ServiceRecord {
            host_id,
            name: self.name.clone(),
            addr: self.addr.ip(),
            port: self.addr.port(),
            txt: HashMap::new(),
        };

        let _ = tx.try_send(DiscoveryEvent::Found(record));
        Ok(rx)
    }

    fn stop_browse(&mut self) -> Result<(), DiscoveryError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manual_browser_static_resolution() {
        let mut browser = ManualBrowser::parse("192.168.1.100:9000").unwrap();
        let mut rx = browser.start_browse().unwrap();

        let event = rx.recv().await.unwrap();
        if let DiscoveryEvent::Found(record) = event {
            assert_eq!(record.addr.to_string(), "192.168.1.100");
            assert_eq!(record.port, 9000);
            assert_eq!(record.name, "Manual (192.168.1.100)");
        } else {
            panic!("Expected DiscoveryEvent::Found");
        }
    }
}
