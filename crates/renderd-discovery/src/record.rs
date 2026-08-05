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
