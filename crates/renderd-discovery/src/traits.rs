//! Trait abstractions for mDNS service advertisement and browsing.

use tokio::sync::mpsc::Receiver;

use crate::error::DiscoveryError;
use crate::record::{DiscoveryEvent, ServiceRecord};

/// Trait for advertising a Renderd service on the local network via mDNS.
pub trait Advertiser: Send + Sync {
    /// Starts advertising the specified service record.
    ///
    /// # Errors
    /// Returns [`DiscoveryError`] if service registration or advertisement fails.
    fn register(&mut self, record: &ServiceRecord) -> Result<(), DiscoveryError>;

    /// Stops advertising the service record and unregisters from mDNS.
    ///
    /// # Errors
    /// Returns [`DiscoveryError`] if unregistration fails.
    fn unregister(&mut self) -> Result<(), DiscoveryError>;
}

/// Trait for browsing and monitoring active Renderd host services on the local network.
pub trait Browser: Send + Sync {
    /// Starts browsing for Renderd hosts, returning a channel for [`DiscoveryEvent`] notifications.
    ///
    /// # Errors
    /// Returns [`DiscoveryError`] if browser initialization fails.
    fn start_browse(&mut self) -> Result<Receiver<DiscoveryEvent>, DiscoveryError>;

    /// Stops browsing and cleans up background resolution tasks.
    ///
    /// # Errors
    /// Returns [`DiscoveryError`] if stopping fails.
    fn stop_browse(&mut self) -> Result<(), DiscoveryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::Ipv4Addr;
    use uuid::Uuid;

    struct MockAdvertiser;
    impl Advertiser for MockAdvertiser {
        fn register(&mut self, _record: &ServiceRecord) -> Result<(), DiscoveryError> {
            Ok(())
        }
        fn unregister(&mut self) -> Result<(), DiscoveryError> {
            Ok(())
        }
    }

    #[test]
    fn test_advertiser_trait_compiles() {
        let mut adv = MockAdvertiser;
        let record = ServiceRecord {
            host_id: Uuid::new_v4(),
            name: "test-host".to_string(),
            addr: std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 9000,
            txt: HashMap::new(),
        };

        assert!(adv.register(&record).is_ok());
        assert!(adv.unregister().is_ok());
    }
}
