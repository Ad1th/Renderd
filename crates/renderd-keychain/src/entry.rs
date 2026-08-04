//! Data structures for stored pairing entries.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Persistent pairing entry containing Pair Token and peer identification metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingEntry {
    /// Host UUID.
    pub host_id: Uuid,

    /// Viewer UUID.
    pub viewer_id: Uuid,

    /// 32-byte secret Pair Token derived during SPAKE2+ pairing.
    pub pair_token: Vec<u8>,

    /// UNIX timestamp (seconds) when pairing ceremony was completed.
    pub paired_at: u64,

    /// UNIX timestamp (seconds) when derived TLS certificate expires.
    pub cert_expires_at: u64,
}
