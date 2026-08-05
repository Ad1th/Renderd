//! Error types for the `renderd-discovery` crate.

use thiserror::Error;

/// Error type for mDNS registration, browsing, and manual IP resolution operations.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// Socket binding or network interface error.
    #[error("Bind error: {0}")]
    BindFailed(String),

    /// mDNS service registration error.
    #[error("Service registration error: {0}")]
    ServiceRegistrationFailed(String),

    /// mDNS browse or query resolution error.
    #[error("Browse error: {0}")]
    BrowseFailed(String),

    /// Invalid address or TXT record parsing error.
    #[error("Invalid record: {0}")]
    InvalidRecord(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_error_display() {
        let err = DiscoveryError::BindFailed("address in use".to_string());
        assert_eq!(err.to_string(), "Bind error: address in use");

        let err2 = DiscoveryError::ServiceRegistrationFailed("DNS failure".to_string());
        assert_eq!(err2.to_string(), "Service registration error: DNS failure");
    }
}
