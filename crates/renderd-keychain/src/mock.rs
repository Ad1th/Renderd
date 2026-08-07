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
    use std::sync::Arc;

    use super::*;

    /// Construct a reusable [`PairingEntry`] for a given viewer UUID.
    fn make_entry(host_id: Uuid, viewer_id: Uuid) -> PairingEntry {
        PairingEntry {
            host_id,
            viewer_id,
            pair_token: vec![10, 20, 30],
            paired_at: 100,
            cert_expires_at: 200,
        }
    }

    // ── Happy-path CRUD ───────────────────────────────────────────────────────

    #[test]
    fn test_mock_keychain_crud() {
        let mock = MockKeychain::new();
        let host_id = Uuid::new_v4();
        let viewer_id = Uuid::new_v4();
        let entry = make_entry(host_id, viewer_id);

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

    // ── Not-found error paths ─────────────────────────────────────────────────

    /// Loading an entry that was never saved must return `NotFound`.
    #[test]
    fn test_load_missing_returns_not_found() {
        let mock = MockKeychain::new();
        let unknown = Uuid::new_v4();
        let err = mock.load_pairing(unknown).unwrap_err();
        assert!(
            matches!(err, KeychainError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    /// Deleting an entry that was never saved must return `NotFound`.
    #[test]
    fn test_delete_missing_returns_not_found() {
        let mock = MockKeychain::new();
        let unknown = Uuid::new_v4();
        let err = mock.delete_pairing(unknown).unwrap_err();
        assert!(
            matches!(err, KeychainError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    /// Double-deleting the same viewer UUID must return `NotFound` on the second call.
    #[test]
    fn test_double_delete_returns_not_found() {
        let mock = MockKeychain::new();
        let host_id = Uuid::new_v4();
        let viewer_id = Uuid::new_v4();
        mock.save_pairing(&make_entry(host_id, viewer_id)).unwrap();
        mock.delete_pairing(viewer_id).unwrap();
        let err = mock.delete_pairing(viewer_id).unwrap_err();
        assert!(matches!(err, KeychainError::NotFound(_)));
    }

    // ── Overwrite semantics ───────────────────────────────────────────────────

    /// Saving a second entry for the same `viewer_id` must overwrite the first.
    #[test]
    fn test_save_overwrites_existing_entry() {
        let mock = MockKeychain::new();
        let host_id = Uuid::new_v4();
        let viewer_id = Uuid::new_v4();

        let entry_v1 = PairingEntry {
            host_id,
            viewer_id,
            pair_token: vec![1, 2, 3],
            paired_at: 1000,
            cert_expires_at: 2000,
        };
        let entry_v2 = PairingEntry {
            host_id,
            viewer_id,
            pair_token: vec![7, 8, 9],
            paired_at: 9999,
            cert_expires_at: 19999,
        };

        mock.save_pairing(&entry_v1).unwrap();
        mock.save_pairing(&entry_v2).unwrap();

        let loaded = mock.load_pairing(viewer_id).unwrap();
        assert_eq!(loaded, entry_v2, "second save must overwrite first");

        // Only one entry must exist in the store after an overwrite
        assert_eq!(
            mock.list_pairings().unwrap().len(),
            1,
            "overwrite must not create duplicate entries"
        );
    }

    // ── Empty store ───────────────────────────────────────────────────────────

    /// Listing an empty store must return an empty Vec, not an error.
    #[test]
    fn test_list_empty_store_returns_empty_vec() {
        let mock = MockKeychain::new();
        let list = mock.list_pairings().unwrap();
        assert!(list.is_empty(), "empty store must yield empty list");
    }

    // ── Multi-entry isolation ─────────────────────────────────────────────────

    /// Multiple distinct viewer UUIDs must be stored and retrieved independently.
    #[test]
    fn test_multiple_entries_are_independent() {
        let mock = MockKeychain::new();
        let host_id = Uuid::new_v4();

        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        for &v in &ids {
            mock.save_pairing(&make_entry(host_id, v)).unwrap();
        }

        assert_eq!(mock.list_pairings().unwrap().len(), 5);

        // Each viewer entry must load independently and be isolated from others
        for &v in &ids {
            let loaded = mock.load_pairing(v).unwrap();
            assert_eq!(loaded.viewer_id, v);
        }

        // Deleting one must not affect the others
        mock.delete_pairing(ids[2]).unwrap();
        assert_eq!(mock.list_pairings().unwrap().len(), 4);
        assert!(mock.load_pairing(ids[0]).is_ok());
        assert!(mock.load_pairing(ids[1]).is_ok());
        assert!(mock.load_pairing(ids[3]).is_ok());
        assert!(mock.load_pairing(ids[4]).is_ok());
    }

    // ── Shared-ownership via Arc ──────────────────────────────────────────────

    /// `MockKeychain` wrapped in `Arc` can be shared across multiple owners,
    /// matching how the real store is used in `HostSession`.
    #[test]
    fn test_mock_keychain_arc_shared_access() {
        let mock = Arc::new(MockKeychain::new());
        let host_id = Uuid::new_v4();
        let viewer_id = Uuid::new_v4();
        let entry = make_entry(host_id, viewer_id);

        let writer = Arc::clone(&mock);
        writer.save_pairing(&entry).unwrap();

        let reader = Arc::clone(&mock);
        assert_eq!(reader.load_pairing(viewer_id).unwrap(), entry);
    }
}
