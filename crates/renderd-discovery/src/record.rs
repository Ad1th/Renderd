//! Discovery service records and event notifications.

use std::collections::HashMap;
use std::net::IpAddr;
use uuid::Uuid;

/// Service discovery record describing an advertised or discovered Renderd host daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRecord {
    /// Host UUID identifier.
    pub host_id: Uuid,

    /// Human-readable host display name.
    pub name: String,

    /// IP address of host.
    pub addr: IpAddr,

    /// UDP port number host is listening on.
    pub port: u16,

    /// Key-value pairs embedded in mDNS TXT record.
    pub txt: HashMap<String, String>,
}

/// Event emitted by a [`Browser`] when host services appear or disappear on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A new or updated host service was found.
    Found(ServiceRecord),

    /// A previously discovered host service went offline.
    Lost(Uuid),
}

/// Returns true if the given IPv6 address is in the link-local scope (`fe80::/10`).
#[must_use]
pub const fn is_link_local_v6(v6: &std::net::Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

/// Calculates a preference score for an IP address. Higher score means higher priority.
///
/// Preference ranking:
/// - 4: Usable IPv4 (non-loopback, non-unspecified)
/// - 3: IPv4 loopback (127.0.0.1)
/// - 3: Global / Routable IPv6 (non-link-local, non-loopback, non-unspecified)
/// - 2: IPv6 loopback (`::1`)
/// - 1: Unscoped IPv6 link-local (`fe80::/10`)
/// - 0: Unspecified (0.0.0.0 or ::)
#[must_use]
pub const fn address_score(addr: &IpAddr) -> u8 {
    match addr {
        IpAddr::V4(v4) => {
            if v4.is_unspecified() {
                0
            } else if v4.is_loopback() {
                3
            } else {
                4
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_unspecified() {
                0
            } else if is_link_local_v6(v6) {
                1
            } else if v6.is_loopback() {
                2
            } else {
                3
            }
        }
    }
}

/// Selects the best address from a slice of candidate IP addresses according to [`address_score`].
#[must_use]
pub fn select_best_address(addrs: &[IpAddr]) -> Option<IpAddr> {
    addrs.iter().copied().max_by_key(address_score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn test_address_score_ipv4_preferred_over_ipv6_link_local() {
        let v4: IpAddr = "10.243.73.235".parse().unwrap();
        let v6_link_local: IpAddr = "fe80::1cdb:c5ef:65b1:8ba1".parse().unwrap();

        assert!(address_score(&v4) > address_score(&v6_link_local));
    }

    #[test]
    fn test_select_best_address_prefer_ipv4() {
        let v4: IpAddr = "10.243.73.235".parse().unwrap();
        let v6_link_local: IpAddr = "fe80::1cdb:c5ef:65b1:8ba1".parse().unwrap();

        let addrs = vec![v6_link_local, v4];
        assert_eq!(select_best_address(&addrs), Some(v4));

        let addrs_rev = vec![v4, v6_link_local];
        assert_eq!(select_best_address(&addrs_rev), Some(v4));
    }

    #[test]
    fn test_select_best_address_global_v6_over_link_local() {
        let v6_global: IpAddr = "2001:db8::1".parse().unwrap();
        let v6_link_local: IpAddr = "fe80::1cdb:c5ef:65b1:8ba1".parse().unwrap();

        let addrs = vec![v6_link_local, v6_global];
        assert_eq!(select_best_address(&addrs), Some(v6_global));
    }

    #[test]
    fn test_select_best_address_empty() {
        assert_eq!(select_best_address(&[]), None);
    }

    #[test]
    fn test_is_link_local_v6() {
        let ll: Ipv6Addr = "fe80::1".parse().unwrap();
        let global: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let loopback: Ipv6Addr = "::1".parse().unwrap();

        assert!(is_link_local_v6(&ll));
        assert!(!is_link_local_v6(&global));
        assert!(!is_link_local_v6(&loopback));
    }
}
