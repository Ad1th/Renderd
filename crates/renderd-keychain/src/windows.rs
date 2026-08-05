//! Windows Credential Manager backend for persistent credential storage.

#![cfg(target_os = "windows")]

use std::ptr;
use uuid::Uuid;
use windows::core::PCWSTR;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredEnumerateW, CredFree, CredReadW, CredWriteW, CREDENTIALW,
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

use crate::entry::PairingEntry;
use crate::error::KeychainError;
use crate::store::KeychainStore;

const TARGET_PREFIX: &str = "Renderd/Pairing/";

/// [`KeychainStore`] implementation backed by Windows Credential Manager.
#[derive(Debug, Default)]
pub struct WindowsCredentialManager;

impl WindowsCredentialManager {
    /// Creates a new [`WindowsCredentialManager`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl KeychainStore for WindowsCredentialManager {
    fn save_pairing(&self, entry: &PairingEntry) -> Result<(), KeychainError> {
        let payload = serde_json::to_vec(entry).map_err(|e| {
            KeychainError::Serialization(format!("Failed to serialize pairing entry: {e}"))
        })?;

        let target_name_str = format!("{TARGET_PREFIX}{}", entry.viewer_id);
        let target_name_u16 = to_utf16(&target_name_str);

        let cred = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PCWSTR(target_name_u16.as_ptr()),
            CredentialBlobSize: u32::try_from(payload.len()).map_err(|_| {
                KeychainError::Serialization("Payload length exceeds u32".to_string())
            })?,
            CredentialBlob: payload.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };

        // SAFETY: CredWriteW takes a pointer to a valid CREDENTIALW structure.
        unsafe {
            CredWriteW(&cred, 0).ok().map_err(|e| {
                KeychainError::Platform(format!("CredWriteW failed for {target_name_str}: {e}"))
            })?;
        }

        Ok(())
    }

    fn load_pairing(&self, peer_id: Uuid) -> Result<PairingEntry, KeychainError> {
        let target_name_str = format!("{TARGET_PREFIX}{peer_id}");
        let target_name_u16 = to_utf16(&target_name_str);
        let mut cred_ptr: *mut CREDENTIALW = ptr::null_mut();

        // SAFETY: CredReadW reads the credential for target_name into cred_ptr.
        unsafe {
            CredReadW(
                PCWSTR(target_name_u16.as_ptr()),
                CRED_TYPE_GENERIC,
                0,
                &mut cred_ptr,
            )
            .ok()
            .map_err(|e| KeychainError::NotFound(format!("{target_name_str} ({e})")))?;

            if cred_ptr.is_null() {
                return Err(KeychainError::NotFound(target_name_str));
            }

            let cred = &*cred_ptr;
            let slice =
                std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
            let entry: PairingEntry = serde_json::from_slice(slice)
                .map_err(|e| KeychainError::Serialization(format!("Deserialization error: {e}")))?;

            CredFree(cred_ptr.cast());
            Ok(entry)
        }
    }

    fn delete_pairing(&self, peer_id: Uuid) -> Result<(), KeychainError> {
        let target_name_str = format!("{TARGET_PREFIX}{peer_id}");
        let target_name_u16 = to_utf16(&target_name_str);

        // SAFETY: CredDeleteW deletes the target credential.
        unsafe {
            CredDeleteW(PCWSTR(target_name_u16.as_ptr()), CRED_TYPE_GENERIC, 0)
                .ok()
                .map_err(|e| KeychainError::NotFound(format!("{target_name_str} ({e})")))?;
        }

        Ok(())
    }

    fn list_pairings(&self) -> Result<Vec<PairingEntry>, KeychainError> {
        let filter_str = format!("{TARGET_PREFIX}*");
        let filter_u16 = to_utf16(&filter_str);

        let mut count: u32 = 0;
        let mut creds_ptr: *mut *mut CREDENTIALW = ptr::null_mut();

        // SAFETY: CredEnumerateW enumerates all credentials matching filter_u16.
        unsafe {
            if CredEnumerateW(PCWSTR(filter_u16.as_ptr()), 0, &mut count, &mut creds_ptr)
                .ok()
                .is_err()
                || creds_ptr.is_null()
            {
                return Ok(vec![]);
            }

            let mut entries = Vec::new();
            let cred_ptrs = std::slice::from_raw_parts(creds_ptr, count as usize);
            for &c_ptr in cred_ptrs {
                if !c_ptr.is_null() {
                    let cred = &*c_ptr;
                    let slice = std::slice::from_raw_parts(
                        cred.CredentialBlob,
                        cred.CredentialBlobSize as usize,
                    );
                    if let Ok(entry) = serde_json::from_slice::<PairingEntry>(slice) {
                        entries.push(entry);
                    }
                }
            }

            CredFree(creds_ptr.cast());
            Ok(entries)
        }
    }
}
