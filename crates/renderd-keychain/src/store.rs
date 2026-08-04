//! Trait abstraction for persistent keychain credential storage.

use uuid::Uuid;

use crate::entry::PairingEntry;
use crate::error::KeychainError;

/// Platform-agnostic interface for saving, reading, listing, and deleting pairing credentials.
pub trait KeychainStore: Send + Sync {
    /// Saves a [`PairingEntry`] to the persistent system keychain.
    ///
    /// # Errors
    /// Returns [`KeychainError`] if saving fails.
    fn save_pairing(&self, entry: &PairingEntry) -> Result<(), KeychainError>;

    /// Loads a [`PairingEntry`] for the given peer UUID (host or viewer).
    ///
    /// # Errors
    /// Returns [`KeychainError::NotFound`] if no entry matches, or [`KeychainError::Platform`] on I/O error.
    fn load_pairing(&self, peer_id: Uuid) -> Result<PairingEntry, KeychainError>;

    /// Deletes a [`PairingEntry`] associated with the given peer UUID.
    ///
    /// # Errors
    /// Returns [`KeychainError`] if deletion fails or entry does not exist.
    fn delete_pairing(&self, peer_id: Uuid) -> Result<(), KeychainError>;

    /// Lists all stored [`PairingEntry`] records in the keychain.
    ///
    /// # Errors
    /// Returns [`KeychainError`] if listing fails.
    fn list_pairings(&self) -> Result<Vec<PairingEntry>, KeychainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyStore;
    impl KeychainStore for DummyStore {
        fn save_pairing(&self, _entry: &PairingEntry) -> Result<(), KeychainError> {
            Ok(())
        }
        fn load_pairing(&self, _peer_id: Uuid) -> Result<PairingEntry, KeychainError> {
            Err(KeychainError::NotFound("dummy".to_string()))
        }
        fn delete_pairing(&self, _peer_id: Uuid) -> Result<(), KeychainError> {
            Ok(())
        }
        fn list_pairings(&self) -> Result<Vec<PairingEntry>, KeychainError> {
            Ok(vec![])
        }
    }

    #[test]
    fn test_keychain_store_trait_compiles() {
        let store = DummyStore;
        assert!(store.list_pairings().unwrap().is_empty());
    }
}
