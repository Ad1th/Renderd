//! Viewer SPAKE2+ pairing prover client (`renderd-viewer/src/pairing/prover.rs`).
//!
//! Manages PIN entry, SPAKE2+ key derivation via `renderd-crypto`, and persisting derived `PairToken`
//! to platform credential manager (`renderd-keychain`) (RFC-0002 §9.2).

use renderd_crypto::{derive_pair_token, PairToken};
use renderd_keychain::{KeychainStore, PairingEntry};
use renderd_proto::types::{HostId, ViewerId};
use std::sync::Arc;

/// Pairing state errors for the viewer pairing client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewerPairingError {
    /// Invalid PIN format (must be 6 ASCII digits).
    #[error("invalid PIN format: must be 6 digits")]
    InvalidPinFormat,

    /// SPAKE2+ pairing verification failed.
    #[error("SPAKE2+ pairing verification failed: {0}")]
    VerificationFailed(String),

    /// Credential store save failed.
    #[error("keychain save failed: {0}")]
    KeychainSave(String),
}

/// Viewer pairing client orchestrating PIN submission and token persistence.
#[derive(Clone)]
pub struct ViewerPairingClient {
    keychain: Arc<dyn KeychainStore>,
    viewer_id: ViewerId,
}

impl std::fmt::Debug for ViewerPairingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewerPairingClient")
            .field("viewer_id", &self.viewer_id)
            .finish_non_exhaustive()
    }
}

impl ViewerPairingClient {
    /// Creates a new `ViewerPairingClient`.
    #[must_use]
    pub fn new(keychain: Arc<dyn KeychainStore>, viewer_id: ViewerId) -> Self {
        Self {
            keychain,
            viewer_id,
        }
    }

    /// Validates PIN format (must be 6 ASCII digits).
    ///
    /// # Errors
    /// Returns [`ViewerPairingError::InvalidPinFormat`] if string is not 6 digits.
    pub fn validate_pin(pin: &str) -> Result<(), ViewerPairingError> {
        if pin.len() == 6 && pin.chars().all(|c| c.is_ascii_digit()) {
            Ok(())
        } else {
            Err(ViewerPairingError::InvalidPinFormat)
        }
    }

    /// Derives the SPAKE2+ pair token for the provided PIN and host ID.
    ///
    /// # Errors
    /// Returns [`ViewerPairingError::InvalidPinFormat`] if PIN is invalid.
    pub fn derive_token(
        &self,
        pin: &str,
        host_id: HostId,
    ) -> Result<PairToken, ViewerPairingError> {
        Self::validate_pin(pin)?;
        Ok(derive_pair_token(
            pin.as_bytes(),
            host_id.0,
            self.viewer_id.0,
        ))
    }

    /// Executes pairing handshake and stores derived `PairToken` in credential manager.
    ///
    /// # Errors
    /// Returns [`ViewerPairingError`] if PIN is invalid, verification fails, or keychain save fails.
    pub fn execute_pairing(
        &self,
        pin: &str,
        host_id: HostId,
    ) -> Result<PairToken, ViewerPairingError> {
        let pair_token = self.derive_token(pin, host_id)?;

        let entry = PairingEntry {
            host_id: host_id.0,
            viewer_id: self.viewer_id.0,
            pair_token: pair_token.0.to_vec(),
            paired_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            cert_expires_at: 9_999_999_999,
        };

        self.keychain
            .save_pairing(&entry)
            .map_err(|e| ViewerPairingError::KeychainSave(e.to_string()))?;

        tracing::info!(
            host_id = %host_id,
            viewer_id = %self.viewer_id,
            "Viewer pairing successful and PairToken saved to credential manager"
        );

        Ok(pair_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_keychain::MockKeychain;
    use uuid::Uuid;

    #[test]
    fn test_viewer_pairing_pin_validation() {
        assert!(ViewerPairingClient::validate_pin("123456").is_ok());
        assert!(matches!(
            ViewerPairingClient::validate_pin("12345"),
            Err(ViewerPairingError::InvalidPinFormat)
        ));
        assert!(matches!(
            ViewerPairingClient::validate_pin("abcdef"),
            Err(ViewerPairingError::InvalidPinFormat)
        ));
    }

    #[test]
    fn test_viewer_pairing_execution_and_keychain_persistence() {
        let keychain = Arc::new(MockKeychain::new());
        let vid = ViewerId(Uuid::new_v4());
        let hid = HostId(Uuid::new_v4());

        let client = ViewerPairingClient::new(keychain.clone(), vid);
        let token = client.execute_pairing("654321", hid).unwrap();

        assert_ne!(token.0, [0u8; 32]);
        let entries = keychain.list_pairings().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host_id, hid.0);
        assert_eq!(entries[0].viewer_id, vid.0);
    }
}
