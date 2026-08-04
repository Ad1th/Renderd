//! In-memory mock keychain store for headless testing environments.

use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

use crate::entry::PairingEntry;
use crate::error::KeychainError;
use crate::store::KeychainStore;

/// In-memory implementation of [`KeychainStore`] for testing.
#[derive(Debug, Default)]
pub struct MockKeychain {
    store: RwLock<HashMap<Uuid, PairingEntry>>,
}

impl MockKeychain {
    /// Creates a new empty [`MockKeychain`].
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn new() -> Self {
        Self {
            store: RwLock::new(HashMap::new()),
        }
    }
}

impl KeychainStore for MockKeychain {
    fn save_pairing(&self, entry: &PairingEntry) -> Result<(), KeychainError> {
        let mut map = self
            .store
            .write()
            .map_err(|_| KeychainError::Platform("Lock poisoned".to_string()))?;
        map.insert(entry.viewer_id, entry.clone());
        drop(map);
        Ok(())
    }

    fn load_pairing(&self, peer_id: Uuid) -> Result<PairingEntry, KeychainError> {
        let map = self
            .store
            .read()
            .map_err(|_| KeychainError::Platform("Lock poisoned".to_string()))?;
        let entry = map
            .get(&peer_id)
            .cloned()
            .ok_or_else(|| KeychainError::NotFound(peer_id.to_string()));
        drop(map);
        entry
    }

    fn delete_pairing(&self, peer_id: Uuid) -> Result<(), KeychainError> {
        let mut map = self
            .store
            .write()
            .map_err(|_| KeychainError::Platform("Lock poisoned".to_string()))?;
        let res = map
            .remove(&peer_id)
            .map(|_| ())
            .ok_or_else(|| KeychainError::NotFound(peer_id.to_string()));
        drop(map);
        res
    }

    fn list_pairings(&self) -> Result<Vec<PairingEntry>, KeychainError> {
        let map = self
            .store
            .read()
            .map_err(|_| KeychainError::Platform("Lock poisoned".to_string()))?;
        Ok(map.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_keychain_crud() {
        let mock = MockKeychain::new();
        let host_id = Uuid::new_v4();
        let viewer_id = Uuid::new_v4();

        let entry = PairingEntry {
            host_id,
            viewer_id,
            pair_token: vec![10, 20, 30],
            paired_at: 100,
            cert_expires_at: 200,
        };

        mock.save_pairing(&entry).unwrap();

        let loaded = mock.load_pairing(viewer_id).unwrap();
        assert_eq!(loaded, entry);

        let list = mock.list_pairings().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], entry);

        mock.delete_pairing(viewer_id).unwrap();

        let loaded_err = mock.load_pairing(viewer_id);
        assert!(matches!(loaded_err, Err(KeychainError::NotFound(_))));
    }
}
