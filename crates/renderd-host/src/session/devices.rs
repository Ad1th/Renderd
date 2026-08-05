//! Known-viewers device registry and credential revocation manager (RFC-0002 §9.3).
//!
//! Provides querying and revocation of paired viewer devices backed by the system [`KeychainStore`].

use std::sync::Arc;

use renderd_keychain::{KeychainError, KeychainStore, PairingEntry};
use renderd_proto::types::ViewerId;

use crate::error::HostError;

/// Device registry manager for paired viewers.
///
/// Encapsulates operations to query paired viewers, check authorization status,
/// and revoke viewer credentials stored in the platform keychain.
#[derive(Clone)]
pub struct DeviceRegistry {
    keychain: Arc<dyn KeychainStore>,
}

impl std::fmt::Debug for DeviceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceRegistry").finish_non_exhaustive()
    }
}

impl DeviceRegistry {
    /// Creates a new `DeviceRegistry` instance backed by the given [`KeychainStore`].
    #[must_use]
    pub fn new(keychain: Arc<dyn KeychainStore>) -> Self {
        Self { keychain }
    }

    /// Retrieves all paired viewer records currently stored in the keychain.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if keychain access or entry listing fails.
    pub fn list(&self) -> Result<Vec<PairingEntry>, HostError> {
        self.keychain
            .list_pairings()
            .map_err(|e| HostError::Initialization(format!("Failed to list paired devices: {e}")))
    }

    /// Checks if a viewer with the specified `ViewerId` is currently paired.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if querying the keychain fails with an I/O error.
    pub fn is_paired(&self, viewer_id: ViewerId) -> Result<bool, HostError> {
        match self.keychain.load_pairing(viewer_id.0) {
            Ok(_) => Ok(true),
            Err(KeychainError::NotFound(_)) => Ok(false),
            Err(e) => Err(HostError::Initialization(format!(
                "Failed to check pairing status for {viewer_id}: {e}"
            ))),
        }
    }

    /// Loads the [`PairingEntry`] for a specific `ViewerId` if it exists.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if querying the keychain fails with an I/O error.
    pub fn get(&self, viewer_id: ViewerId) -> Result<Option<PairingEntry>, HostError> {
        match self.keychain.load_pairing(viewer_id.0) {
            Ok(entry) => Ok(Some(entry)),
            Err(KeychainError::NotFound(_)) => Ok(None),
            Err(e) => Err(HostError::Initialization(format!(
                "Failed to load pairing for {viewer_id}: {e}"
            ))),
        }
    }

    /// Revokes a paired viewer by deleting its pairing credentials from the keychain.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if deletion fails or if the viewer was not paired.
    pub fn revoke(&self, viewer_id: ViewerId) -> Result<(), HostError> {
        self.keychain.delete_pairing(viewer_id.0).map_err(|e| {
            HostError::Initialization(format!("Failed to revoke viewer {viewer_id}: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_keychain::MockKeychain;
    use renderd_proto::types::HostId;
    use uuid::Uuid;

    fn test_registry() -> (DeviceRegistry, Arc<MockKeychain>) {
        let mock = Arc::new(MockKeychain::new());
        let registry = DeviceRegistry::new(mock.clone());
        (registry, mock)
    }

    #[test]
    fn test_device_registry_empty_initial_list() {
        let (registry, _) = test_registry();
        let devices = registry.list().unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn test_device_registry_list_and_is_paired() {
        let (registry, mock) = test_registry();
        let host_id = HostId(Uuid::new_v4());
        let viewer_id = ViewerId(Uuid::new_v4());

        assert!(!registry.is_paired(viewer_id).unwrap());
        assert!(registry.get(viewer_id).unwrap().is_none());

        let entry = PairingEntry {
            host_id: host_id.0,
            viewer_id: viewer_id.0,
            pair_token: vec![1, 2, 3],
            paired_at: 1000,
            cert_expires_at: 2000,
        };

        mock.save_pairing(&entry).unwrap();

        assert!(registry.is_paired(viewer_id).unwrap());
        let list = registry.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].viewer_id, viewer_id.0);

        let retrieved = registry.get(viewer_id).unwrap().unwrap();
        assert_eq!(retrieved, entry);
    }

    #[test]
    fn test_device_registry_revoke() {
        let (registry, mock) = test_registry();
        let host_id = HostId(Uuid::new_v4());
        let viewer_id = ViewerId(Uuid::new_v4());

        let entry = PairingEntry {
            host_id: host_id.0,
            viewer_id: viewer_id.0,
            pair_token: vec![1, 2, 3],
            paired_at: 1000,
            cert_expires_at: 2000,
        };

        mock.save_pairing(&entry).unwrap();
        assert!(registry.is_paired(viewer_id).unwrap());

        registry.revoke(viewer_id).unwrap();
        assert!(!registry.is_paired(viewer_id).unwrap());
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn test_device_registry_revoke_non_existent_fails() {
        let (registry, _) = test_registry();
        let viewer_id = ViewerId(Uuid::new_v4());

        let err = registry.revoke(viewer_id).unwrap_err();
        assert!(matches!(err, HostError::Initialization(_)));
    }
}
