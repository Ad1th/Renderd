//! Viewer pairing UI controller (`renderd-viewer/src/pairing/ui.rs`).
//!
//! Coordinates user PIN entry, SPAKE2+ prover execution via [`ViewerPairingClient`],
//! and state representation (`Prompting`, `Verifying`, `Success`, `Failed`).

use std::sync::{Arc, Mutex};

use renderd_crypto::PairToken;
use renderd_proto::types::HostId;

use crate::pairing::prover::{ViewerPairingClient, ViewerPairingError};

/// UI state for the viewer PIN pairing dialog / prompt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PairingUiState {
    /// No pairing prompt active.
    #[default]
    Idle,
    /// Prompting user for 6-digit PIN entry.
    Prompting,
    /// Verifying submitted PIN and negotiating credentials.
    Verifying,
    /// Pairing succeeded; `PairToken` stored in credential manager.
    Success,
    /// Pairing failed with error message.
    Failed(String),
}

/// Pairing UI controller managing PIN prompt state and prover dispatch.
#[derive(Debug, Clone)]
pub struct PairingUi {
    client: ViewerPairingClient,
    state: Arc<Mutex<PairingUiState>>,
}

impl PairingUi {
    /// Creates a new [`PairingUi`] backed by the provided [`ViewerPairingClient`].
    #[must_use]
    pub fn new(client: ViewerPairingClient) -> Self {
        Self {
            client,
            state: Arc::new(Mutex::new(PairingUiState::Idle)),
        }
    }

    /// Returns the current [`PairingUiState`].
    ///
    /// # Panics
    /// Panics if internal mutex is poisoned.
    #[must_use]
    pub fn state(&self) -> PairingUiState {
        self.state.lock().expect("PairingUi mutex poisoned").clone()
    }

    /// Sets the prompt into `Prompting` state.
    ///
    /// # Panics
    /// Panics if internal mutex is poisoned.
    pub fn start_prompt(&self) {
        let mut guard = self.state.lock().expect("PairingUi mutex poisoned");
        *guard = PairingUiState::Prompting;
    }

    /// Submits a PIN for verification and executes pairing ceremony.
    ///
    /// # Errors
    /// Returns [`ViewerPairingError`] if PIN format is invalid or pairing fails.
    ///
    /// # Panics
    /// Panics if internal mutex is poisoned.
    pub fn submit_pin(&self, pin: &str, host_id: HostId) -> Result<PairToken, ViewerPairingError> {
        {
            let mut guard = self.state.lock().expect("PairingUi mutex poisoned");
            *guard = PairingUiState::Verifying;
        }

        match self.client.execute_pairing(pin, host_id) {
            Ok(token) => {
                let mut guard = self.state.lock().expect("PairingUi mutex poisoned");
                *guard = PairingUiState::Success;
                drop(guard);
                Ok(token)
            }
            Err(err) => {
                let mut guard = self.state.lock().expect("PairingUi mutex poisoned");
                *guard = PairingUiState::Failed(err.to_string());
                drop(guard);
                Err(err)
            }
        }
    }

    /// Resets the pairing UI state back to `Idle`.
    ///
    /// # Panics
    /// Panics if internal mutex is poisoned.
    pub fn reset(&self) {
        let mut guard = self.state.lock().expect("PairingUi mutex poisoned");
        *guard = PairingUiState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_keychain::MockKeychain;
    use renderd_proto::types::ViewerId;
    use uuid::Uuid;

    #[test]
    fn test_pairing_ui_lifecycle_success() {
        let keychain = Arc::new(MockKeychain::new());
        let vid = ViewerId(Uuid::new_v4());
        let hid = HostId(Uuid::new_v4());

        let client = ViewerPairingClient::new(keychain, vid);
        let ui = PairingUi::new(client);

        assert_eq!(ui.state(), PairingUiState::Idle);

        ui.start_prompt();
        assert_eq!(ui.state(), PairingUiState::Prompting);

        let res = ui.submit_pin("123456", hid);
        assert!(res.is_ok());
        assert_eq!(ui.state(), PairingUiState::Success);

        ui.reset();
        assert_eq!(ui.state(), PairingUiState::Idle);
    }

    #[test]
    fn test_pairing_ui_lifecycle_failure() {
        let keychain = Arc::new(MockKeychain::new());
        let vid = ViewerId(Uuid::new_v4());
        let hid = HostId(Uuid::new_v4());

        let client = ViewerPairingClient::new(keychain, vid);
        let ui = PairingUi::new(client);

        let res = ui.submit_pin("abc", hid);
        assert!(res.is_err());
        assert!(matches!(ui.state(), PairingUiState::Failed(_)));
    }
}
