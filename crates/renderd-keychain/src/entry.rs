//! Data structures for stored pairing entries.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

/// Persistent pairing entry containing Pair Token and peer identification metadata.
///
/// `pair_token` is the one field here that is actually secret; the rest is
/// identifying metadata. Unlike `renderd-crypto`'s own `PairToken`/
/// `SessionKey` types (which derive `Zeroize`/`ZeroizeOnDrop`), this struct
/// used to have no `Drop` at all, so the raw secret — and every `Clone` of it
/// made while loading/saving to the platform keychain — persisted in freed
/// heap memory after use. `Drop` below wipes just that field; the identifiers
/// and timestamps are not sensitive and are left as plain derives.
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

impl Drop for PairingEntry {
    fn drop(&mut self) {
        self.pair_token.zeroize();
    }
}
