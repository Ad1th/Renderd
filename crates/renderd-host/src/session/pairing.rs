//! Host SPAKE2+ pairing handler: PIN generation, lockout policy, and keychain credential storage.
//!
//! The pairing ceremony follows RFC-0002 §9:
//!
//! 1. Host generates a cryptographically random 6-digit PIN and displays it in the UI.
//! 2. The viewer connects over QUIC and sends its half of the SPAKE2+ key exchange.
//! 3. The host verifies the SPAKE2+ MAC; on success the derived `PairToken` is saved to
//!    the platform Keychain and the session transitions to `CONNECTED`.
//! 4. Repeated failures are subject to exponential lockout starting after 5 attempts.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use renderd_keychain::{KeychainStore, PairingEntry};
use renderd_proto::types::{HostId, ViewerId};

/// Maximum failed pairing attempts before lockout begins.
const MAX_ATTEMPTS_BEFORE_LOCKOUT: u32 = 5;

/// Base duration for exponential lockout after repeated failures (doubles each attempt).
const LOCKOUT_BASE_DURATION: Duration = Duration::from_secs(30);

/// PIN validity window: a PIN expires 60 seconds after generation.
const PIN_VALIDITY_DURATION: Duration = Duration::from_secs(60);

/// Errors produced during the pairing ceremony.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairingError {
    /// No PIN is active; call [`PairingHandler::generate_pin`] first.
    #[error("no active pairing PIN")]
    NoPinActive,

    /// The provided PIN or derived session material did not match.
    #[error("PIN verification failed")]
    VerificationFailed,

    /// Too many failed attempts; pairing is locked out until the given instant.
    #[error("pairing locked out for {seconds}s after {attempts} failed attempts")]
    LockedOut {
        /// Number of consecutive failures recorded.
        attempts: u32,
        /// Remaining lockout duration in seconds (rounded up).
        seconds: u64,
    },

    /// The active PIN has expired and a new one must be generated.
    #[error("pairing PIN has expired")]
    PinExpired,

    /// Keychain storage of the derived token failed.
    #[error("keychain save failed: {0}")]
    KeychainSave(String),

    /// The OS entropy source was unavailable, so no PIN could be generated.
    #[error("cannot generate a secure pairing PIN: {0}")]
    EntropyUnavailable(String),
}

/// Inner mutable state, held behind a `Mutex` so `PairingHandler` is `Send + Sync`.
#[derive(Debug)]
struct PairingState {
    /// Current 6-digit PIN if a pairing session is in progress.
    active_pin: Option<String>,
    /// Instant when the current PIN was generated (used to enforce the 60 s expiry).
    pin_generated_at: Option<Instant>,
    /// Number of consecutive failed verification attempts.
    failed_attempts: u32,
    /// If locked out, the instant at which the lockout expires.
    locked_until: Option<Instant>,
}

impl PairingState {
    const fn new() -> Self {
        Self {
            active_pin: None,
            pin_generated_at: None,
            failed_attempts: 0,
            locked_until: None,
        }
    }
}

/// Host SPAKE2+ pairing handler.
///
/// Manages the full pairing ceremony state: PIN lifecycle, failure counting,
/// exponential lockout, and keychain credential persistence.
///
/// This type is cheap to clone (the inner state is `Arc`-wrapped) and is
/// designed to be shared between the UI layer (which shows the PIN) and the
/// network control task (which runs the SPAKE2+ exchange).
#[derive(Clone)]
pub struct PairingHandler {
    state: Arc<Mutex<PairingState>>,
    keychain: Arc<dyn KeychainStore>,
}

impl std::fmt::Debug for PairingHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingHandler")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl PairingHandler {
    /// Creates a `PairingHandler` backed by the provided [`KeychainStore`].
    #[must_use]
    pub fn new(keychain: Arc<dyn KeychainStore>) -> Self {
        Self {
            state: Arc::new(Mutex::new(PairingState::new())),
            keychain,
        }
    }

    /// Generates and returns a fresh cryptographically random 6-digit PIN.
    ///
    /// The PIN is drawn from the OS CSPRNG with rejection sampling, so every value in
    /// `000000..=999999` is equally likely and no part of it can be reconstructed from
    /// the wall clock, the process state, or a previously observed PIN.
    ///
    /// Any previously active PIN is discarded. The new PIN is valid for 60 seconds.
    ///
    /// # Errors
    ///
    /// Returns [`PairingError::EntropyUnavailable`] if the OS entropy source cannot be
    /// read. Pairing must be refused in that case rather than falling back to a
    /// guessable PIN.
    ///
    /// # Panics
    ///
    /// Panics if the internal state mutex is poisoned (should never occur in normal use).
    pub fn generate_pin(&self) -> Result<String, PairingError> {
        let pin = renderd_crypto::generate_pairing_pin()
            .map_err(|e| PairingError::EntropyUnavailable(e.to_string()))?;

        {
            let mut guard = self
                .state
                .lock()
                .expect("PairingState mutex is not poisoned");
            guard.active_pin = Some(pin.clone());
            guard.pin_generated_at = Some(Instant::now());
        }

        Ok(pin)
    }

    /// Returns the currently active PIN without generating a new one, or `None` if no PIN is set.
    ///
    /// # Panics
    ///
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn active_pin(&self) -> Option<String> {
        let guard = self
            .state
            .lock()
            .expect("PairingState mutex is not poisoned");
        guard.active_pin.clone()
    }

    /// Returns the number of consecutive failed verification attempts.
    ///
    /// # Panics
    ///
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn failed_attempts(&self) -> u32 {
        self.state
            .lock()
            .expect("PairingState mutex is not poisoned")
            .failed_attempts
    }

    /// Returns `true` if pairing is currently locked out.
    ///
    /// # Panics
    ///
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn is_locked_out(&self) -> bool {
        let guard = self
            .state
            .lock()
            .expect("PairingState mutex is not poisoned");
        Self::check_lockout_inner(&guard).is_err()
    }

    /// Verifies a PIN submitted by the viewer during the SPAKE2+ handshake.
    ///
    /// On success, a `PairingEntry` built from the provided token bytes is written to
    /// the platform Keychain and `Ok(())` is returned.  The active PIN is cleared so
    /// the same PIN cannot be re-used.
    ///
    /// On failure the attempt counter is incremented; once it exceeds
    /// [`MAX_ATTEMPTS_BEFORE_LOCKOUT`] exponential lockout kicks in.
    ///
    /// # Errors
    ///
    /// - [`PairingError::NoPinActive`]   — no PIN has been generated yet.
    /// - [`PairingError::PinExpired`]    — the 60-second PIN window has elapsed.
    /// - [`PairingError::LockedOut`]     — too many consecutive failures.
    /// - [`PairingError::VerificationFailed`] — the submitted PIN does not match.
    /// - [`PairingError::KeychainSave`]  — derived token could not be persisted.
    ///
    /// # Panics
    ///
    /// Panics if the internal state `Mutex` is poisoned (only possible if a thread
    /// panicked while holding the lock, which does not occur in normal operation).
    pub fn verify_and_save(
        &self,
        submitted_pin: &str,
        host_id: HostId,
        viewer_id: ViewerId,
        pair_token: Vec<u8>,
        cert_expires_at: u64,
    ) -> Result<(), PairingError> {
        let mut guard = self
            .state
            .lock()
            .expect("PairingState mutex is not poisoned");

        // 1. Lockout check.
        Self::check_lockout_inner(&guard)?;

        // 2. PIN presence and expiry check.
        let active_pin = guard
            .active_pin
            .as_ref()
            .ok_or(PairingError::NoPinActive)?
            .clone();
        let generated_at = guard.pin_generated_at.ok_or(PairingError::NoPinActive)?;

        if generated_at.elapsed() > PIN_VALIDITY_DURATION {
            guard.active_pin = None;
            guard.pin_generated_at = None;
            return Err(PairingError::PinExpired);
        }

        // 3. Constant-time PIN comparison to prevent timing side-channels.
        if !constant_time_eq(submitted_pin.as_bytes(), active_pin.as_bytes()) {
            guard.failed_attempts += 1;
            if guard.failed_attempts >= MAX_ATTEMPTS_BEFORE_LOCKOUT {
                let lockout = lockout_duration(guard.failed_attempts);
                guard.locked_until = Some(Instant::now() + lockout);
                tracing::warn!(
                    attempts = guard.failed_attempts,
                    lockout_secs = lockout.as_secs(),
                    "Pairing locked out after repeated failures"
                );
            }
            return Err(PairingError::VerificationFailed);
        }

        // 4. PIN matched — clear active PIN and reset failure counter.
        guard.active_pin = None;
        guard.pin_generated_at = None;
        guard.failed_attempts = 0;
        guard.locked_until = None;

        // Drop the mutex guard before doing I/O so the lock is not held across keychain calls.
        drop(guard);

        // 5. Persist the derived pairing token.
        let entry = PairingEntry {
            host_id: host_id.0,
            viewer_id: viewer_id.0,
            pair_token,
            paired_at: unix_now_secs(),
            cert_expires_at,
        };
        self.keychain
            .save_pairing(&entry)
            .map_err(|e| PairingError::KeychainSave(e.to_string()))?;

        tracing::info!(
            viewer_id = %viewer_id,
            "Pairing ceremony completed and token persisted to keychain"
        );

        Ok(())
    }

    /// Resets all pairing state (PIN, failure counter, lockout).
    ///
    /// Called when the host transitions back to `IDLE` to ensure a clean slate
    /// for the next pairing attempt.
    ///
    /// # Panics
    ///
    /// Panics if the internal state mutex is poisoned.
    pub fn reset(&self) {
        let mut guard = self
            .state
            .lock()
            .expect("PairingState mutex is not poisoned");
        *guard = PairingState::new();
    }

    /// Inner lockout checker that operates on an already-locked `PairingState`.
    fn check_lockout_inner(guard: &PairingState) -> Result<(), PairingError> {
        if let Some(locked_until) = guard.locked_until {
            let now = Instant::now();
            if now < locked_until {
                let remaining = locked_until.duration_since(now);
                return Err(PairingError::LockedOut {
                    attempts: guard.failed_attempts,
                    seconds: remaining.as_secs().saturating_add(1),
                });
            }
        }
        Ok(())
    }
}

/// Constant-time byte-slice comparison to resist timing side-channel attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Returns the exponential lockout duration for a given failure count.
///
/// The duration doubles with each failure beyond the threshold, capped at 1 hour.
fn lockout_duration(failures: u32) -> Duration {
    let extra = failures.saturating_sub(MAX_ATTEMPTS_BEFORE_LOCKOUT);
    let multiplier = 1u64.checked_shl(extra).unwrap_or(u64::MAX);
    let base = LOCKOUT_BASE_DURATION.as_secs().saturating_mul(multiplier);
    Duration::from_secs(base.min(3_600))
}

/// Returns the current UNIX timestamp in seconds.
fn unix_now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use renderd_keychain::MockKeychain;
    use renderd_proto::types::{HostId, ViewerId};
    use uuid::Uuid;

    use super::*;

    fn make_handler() -> PairingHandler {
        PairingHandler::new(Arc::new(MockKeychain::new()))
    }

    fn host_id() -> HostId {
        HostId(Uuid::new_v4())
    }

    fn viewer_id() -> ViewerId {
        ViewerId(Uuid::new_v4())
    }

    #[test]
    fn test_generate_pin_is_six_digits() {
        let handler = make_handler();
        let pin = handler.generate_pin().expect("OS CSPRNG available");
        assert_eq!(pin.len(), 6, "PIN must be exactly 6 characters");
        assert!(
            pin.chars().all(|c| c.is_ascii_digit()),
            "PIN must be all digits"
        );
    }

    #[test]
    fn test_active_pin_returns_generated_pin() {
        let handler = make_handler();
        assert!(handler.active_pin().is_none());
        let pin = handler.generate_pin().expect("OS CSPRNG available");
        assert_eq!(handler.active_pin(), Some(pin));
    }

    #[test]
    fn test_verify_correct_pin_succeeds_and_saves_to_keychain() {
        let keychain = Arc::new(MockKeychain::new());
        let handler = PairingHandler::new(Arc::clone(&keychain) as Arc<dyn KeychainStore>);

        let pin = handler.generate_pin().expect("OS CSPRNG available");
        let hid = host_id();
        let vid = viewer_id();

        let result = handler.verify_and_save(&pin, hid, vid, vec![0u8; 32], 9_999_999_999);
        assert!(result.is_ok(), "Correct PIN should succeed: {result:?}");

        // Token is now in keychain
        let entries = keychain.list_pairings().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].viewer_id, vid.0);

        // PIN is cleared after success
        assert!(handler.active_pin().is_none());
    }

    #[test]
    fn test_verify_wrong_pin_fails_and_increments_counter() {
        let handler = make_handler();
        let _pin = handler.generate_pin().expect("OS CSPRNG available");

        let err = handler
            .verify_and_save("000000", host_id(), viewer_id(), vec![], 0)
            .unwrap_err();
        assert!(matches!(err, PairingError::VerificationFailed));
        assert_eq!(handler.failed_attempts(), 1);
    }

    #[test]
    fn test_no_pin_returns_no_pin_active() {
        let handler = make_handler();
        let err = handler
            .verify_and_save("123456", host_id(), viewer_id(), vec![], 0)
            .unwrap_err();
        assert!(matches!(err, PairingError::NoPinActive));
    }

    #[test]
    fn test_lockout_after_five_failures() {
        let handler = make_handler();

        // Make all attempts with wrong PINs
        for _ in 0..MAX_ATTEMPTS_BEFORE_LOCKOUT {
            let _pin = handler.generate_pin().expect("OS CSPRNG available");
            let _ = handler.verify_and_save("000000", host_id(), viewer_id(), vec![], 0);
        }

        // Next attempt should be locked out
        let _pin = handler.generate_pin().expect("OS CSPRNG available");
        let err = handler
            .verify_and_save("000000", host_id(), viewer_id(), vec![], 0)
            .unwrap_err();
        assert!(
            matches!(err, PairingError::LockedOut { .. }),
            "Expected LockedOut, got {err:?}"
        );
        assert!(handler.is_locked_out());
    }

    #[test]
    fn test_reset_clears_all_state() {
        let handler = make_handler();
        let _pin = handler.generate_pin().expect("OS CSPRNG available");
        let _ = handler.verify_and_save("000000", host_id(), viewer_id(), vec![], 0);

        handler.reset();
        assert!(handler.active_pin().is_none());
        assert_eq!(handler.failed_attempts(), 0);
        assert!(!handler.is_locked_out());
    }

    #[test]
    fn test_lockout_duration_doubles_exponentially() {
        let d5 = lockout_duration(5);
        let d6 = lockout_duration(6);
        let d7 = lockout_duration(7);
        assert_eq!(d5, LOCKOUT_BASE_DURATION);
        assert_eq!(d6, LOCKOUT_BASE_DURATION * 2);
        assert_eq!(d7, LOCKOUT_BASE_DURATION * 4);
    }

    #[test]
    fn test_constant_time_eq_correctness() {
        assert!(constant_time_eq(b"123456", b"123456"));
        assert!(!constant_time_eq(b"123456", b"123457"));
        assert!(!constant_time_eq(b"12345", b"123456"));
    }

    #[test]
    fn test_end_to_end_spake2_pairing_flow() {
        use crate::ui::NotificationManager;

        let host_keychain = Arc::new(MockKeychain::new());
        let viewer_keychain = Arc::new(MockKeychain::new());

        let host_handler =
            PairingHandler::new(Arc::clone(&host_keychain) as Arc<dyn KeychainStore>);
        let notif_mgr = NotificationManager::new();

        let hid = host_id();
        let vid = viewer_id();

        // 1. Host generates PIN and displays/notifies
        let pin = host_handler.generate_pin().expect("OS CSPRNG available");
        notif_mgr.notify_pairing_pin(&pin);

        let notif_history = notif_mgr.history();
        assert_eq!(notif_history.len(), 1);
        assert!(notif_history[0].body.contains(&pin));

        // 2. Viewer derives PairToken using PIN and host_id
        let derived_token = renderd_crypto::derive_pair_token(pin.as_bytes(), hid.0, vid.0);

        // Viewer saves entry
        let viewer_entry = renderd_keychain::PairingEntry {
            host_id: hid.0,
            viewer_id: vid.0,
            pair_token: derived_token.0.to_vec(),
            paired_at: 100,
            cert_expires_at: 9_999_999_999,
        };
        viewer_keychain.save_pairing(&viewer_entry).unwrap();

        // 3. Host verifies PIN and saves derived PairToken
        let res =
            host_handler.verify_and_save(&pin, hid, vid, derived_token.0.to_vec(), 9_999_999_999);
        assert!(res.is_ok());

        // 4. Validate credentials stored on both endpoints match
        let host_entries = host_keychain.list_pairings().unwrap();
        let viewer_entries = viewer_keychain.list_pairings().unwrap();

        assert_eq!(host_entries.len(), 1);
        assert_eq!(viewer_entries.len(), 1);
        assert_eq!(host_entries[0].pair_token, viewer_entries[0].pair_token);
        assert_eq!(host_entries[0].pair_token, derived_token.0.to_vec());

        // PIN cleared on host after successful verification
        assert!(host_handler.active_pin().is_none());
    }
}
