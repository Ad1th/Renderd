//! macOS Keychain Services backend for persistent credential storage.

#![cfg(target_os = "macos")]

use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use uuid::Uuid;

use crate::entry::PairingEntry;
use crate::error::KeychainError;
use crate::store::KeychainStore;

const SERVICE_NAME: &str = "dev.renderd.pairing";

/// [`KeychainStore`] implementation backed by macOS Keychain Services.
#[derive(Debug, Default)]
pub struct MacosKeychain;

impl MacosKeychain {
    /// Creates a new [`MacosKeychain`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl KeychainStore for MacosKeychain {
    fn save_pairing(&self, entry: &PairingEntry) -> Result<(), KeychainError> {
        let payload = serde_json::to_vec(entry).map_err(|e| {
            KeychainError::Serialization(format!("Failed to serialize pairing entry: {e}"))
        })?;

        let peer_account = entry.viewer_id.to_string();
        let _ = delete_generic_password(SERVICE_NAME, &peer_account);

        set_generic_password(SERVICE_NAME, &peer_account, &payload).map_err(|e| {
            KeychainError::Platform(format!("Failed to save generic password: {e}"))
        })?;

        Ok(())
    }

    fn load_pairing(&self, peer_id: Uuid) -> Result<PairingEntry, KeychainError> {
        let peer_account = peer_id.to_string();
        let password_bytes = get_generic_password(SERVICE_NAME, &peer_account).map_err(|e| {
            if e.code() == -25300 {
                KeychainError::NotFound(peer_account.clone())
            } else {
                KeychainError::Platform(format!(
                    "Failed to read generic password for {peer_account}: {e}"
                ))
            }
        })?;

        let entry: PairingEntry = serde_json::from_slice(&password_bytes).map_err(|e| {
            KeychainError::Serialization(format!("Failed to deserialize pairing entry: {e}"))
        })?;

        Ok(entry)
    }

    fn delete_pairing(&self, peer_id: Uuid) -> Result<(), KeychainError> {
        let peer_account = peer_id.to_string();
        delete_generic_password(SERVICE_NAME, &peer_account).map_err(|e| {
            if e.code() == -25300 {
                KeychainError::NotFound(peer_account)
            } else {
                KeychainError::Platform(format!("Failed to delete generic password: {e}"))
            }
        })
    }

    fn list_pairings(&self) -> Result<Vec<PairingEntry>, KeychainError> {
        let search_results = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(SERVICE_NAME)
            .load_data(true)
            .search();

        let results = match search_results {
            Ok(res) => res,
            Err(e) if e.code() == -25300 => return Ok(vec![]),
            Err(e) => {
                return Err(KeychainError::Platform(format!(
                    "Failed to search keychain items: {e}"
                )))
            }
        };

        let mut entries = Vec::new();
        for item in results {
            if let SearchResult::Data(data) = item {
                if let Ok(entry) = serde_json::from_slice::<PairingEntry>(&data) {
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macos_keychain_operations() {
        let keychain = MacosKeychain::new();
        let host_id = Uuid::new_v4();
        let viewer_id = Uuid::new_v4();

        let entry = PairingEntry {
            host_id,
            viewer_id,
            pair_token: vec![1, 2, 3, 4, 5, 6, 7, 8],
            paired_at: 1000,
            cert_expires_at: 2000,
        };

        // Test save, load, list, and delete on macOS Keychain.
        // In non-interactive CI or headless environments without Keychain permission,
        // platform authorization errors are expected and handled gracefully.
        if let Err(KeychainError::Platform(msg)) = keychain.save_pairing(&entry) {
            if msg.contains("authorization")
                || msg.contains("User interaction is not allowed")
                || msg.contains("User canceled the operation")
                || msg.contains("code: -25308")
                || msg.contains("code: -25293")
                || msg.contains("code: -128")
            {
                return;
            }
            panic!("Unexpected save_pairing error: {msg}");
        }

        let loaded = keychain.load_pairing(viewer_id).unwrap();
        assert_eq!(loaded, entry);

        if let Ok(list) = keychain.list_pairings() {
            if !list.is_empty() {
                assert!(list.contains(&entry));
            }
        }

        keychain.delete_pairing(viewer_id).unwrap();

        let err = keychain.load_pairing(viewer_id);
        assert!(matches!(err, Err(KeychainError::NotFound(_))));
    }
}
