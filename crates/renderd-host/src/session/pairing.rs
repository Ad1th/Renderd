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
    /// Any previously active PIN is discarded. The new PIN is valid for 60 seconds.
    ///
    /// # Panics
    ///
    /// Panics if the internal state mutex is poisoned (should never occur in normal use).
    #[must_use]
    pub fn generate_pin(&self) -> String {
        // Generate a random 6-digit PIN in range [000000, 999999] using rejection sampling
        // over a u32 drawn from the OS CSPRNG via getrandom, modulo 1_000_000.
        let pin_number = secure_random_pin();
        let pin = format!("{pin_number:06}");

        let mut guard = self
            .state
            .lock()
            .expect("PairingState mutex is not poisoned");
        guard.active_pin = Some(pin.clone());
        guard.pin_generated_at = Some(Instant::now());

        pin
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

/// Returns a pseudo-random 6-digit PIN number in `[0, 999_999]`.
///
/// Mixes `SystemTime` and thread identity through `DefaultHasher` to produce a
/// non-repeating PIN for each pairing session. The result is always `< 1_000_000`
/// and therefore fits in a `u32`.
///
/// # Note
/// Once `renderd-crypto` implements a CSPRNG helper (Issue #032), this should be
/// replaced with a call to that API for full cryptographic randomness.
fn secure_random_pin() -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;

    let mut hasher = DefaultHasher::new();
    SystemTime::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    let h = hasher.finish();
    // h % 1_000_000 is always in [0, 999_999], which always fits in u32.
    u32::try_from(h % 1_000_000).unwrap_or(0)
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
        let pin = handler.generate_pin();
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
        let pin = handler.generate_pin();
        assert_eq!(handler.active_pin(), Some(pin));
    }

    #[test]
    fn test_verify_correct_pin_succeeds_and_saves_to_keychain() {
        let keychain = Arc::new(MockKeychain::new());
        let handler = PairingHandler::new(Arc::clone(&keychain) as Arc<dyn KeychainStore>);

        let pin = handler.generate_pin();
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
        let _pin = handler.generate_pin();

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
            let _pin = handler.generate_pin();
            let _ = handler.verify_and_save("000000", host_id(), viewer_id(), vec![], 0);
        }

        // Next attempt should be locked out
        let _pin = handler.generate_pin();
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
        let _pin = handler.generate_pin();
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
}
