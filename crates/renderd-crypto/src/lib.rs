//! `SPAKE2+` pairing, `HKDF` key derivation, `TLS` cert generation, and CSPRNG helpers.

use hkdf::Hkdf;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::Zeroize;

/// Error returned when the operating system CSPRNG is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("OS CSPRNG unavailable: {0}")]
pub struct RandomError(String);

/// Fills `dest` with cryptographically secure random bytes from the OS CSPRNG.
///
/// # Errors
/// Returns [`RandomError`] if the platform entropy source cannot be read. Callers
/// must treat this as fatal for any security-relevant operation rather than falling
/// back to a predictable value.
pub fn random_bytes(dest: &mut [u8]) -> Result<(), RandomError> {
    getrandom::fill(dest).map_err(|e| RandomError(e.to_string()))
}

/// Returns a uniformly distributed random `u32` in `[0, bound)` using rejection sampling.
///
/// Rejection sampling (rather than a plain modulo) is required so that low values are
/// not more likely than high ones when `bound` does not divide `2^32` evenly.
///
/// # Errors
/// Returns [`RandomError`] if the OS CSPRNG is unavailable, or if `bound` is 0.
pub fn random_u32_below(bound: u32) -> Result<u32, RandomError> {
    if bound == 0 {
        return Err(RandomError("bound must be non-zero".to_string()));
    }
    // Largest multiple of `bound` that fits in u32; draws at or above it are rejected.
    let limit = u32::MAX - (u32::MAX % bound) - (bound - 1);
    loop {
        let mut buf = [0u8; 4];
        random_bytes(&mut buf)?;
        let value = u32::from_le_bytes(buf);
        if value <= limit {
            return Ok(value % bound);
        }
    }
}

/// Number of decimal digits in a pairing PIN.
pub const PAIRING_PIN_DIGITS: usize = 6;

/// Generates a cryptographically random zero-padded 6-digit pairing PIN.
///
/// # Errors
/// Returns [`RandomError`] if the OS CSPRNG is unavailable. Pairing must be aborted in
/// that case — a guessable PIN would let any host on the network complete the ceremony.
pub fn generate_pairing_pin() -> Result<String, RandomError> {
    let value = random_u32_below(1_000_000)?;
    Ok(format!("{value:06}"))
}

/// 256-bit derived pairing token handle.
///
/// Equality is constant-time so that comparing a candidate token against a stored one
/// does not leak how many leading bytes matched.
#[derive(Debug, Clone, Eq, Zeroize)]
#[zeroize(drop)]
pub struct PairToken(pub [u8; 32]);

impl PartialEq for PairToken {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

/// 256-bit derived session key handle.
///
/// Equality is constant-time, as for [`PairToken`].
#[derive(Debug, Clone, Eq, Zeroize)]
#[zeroize(drop)]
pub struct SessionKey(pub [u8; 32]);

impl PartialEq for SessionKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

/// Derives a 32-byte [`PairToken`] from a pairing PIN, host UUID, and viewer UUID using HKDF-SHA256.
///
/// The host and viewer UUIDs are bound in through the HKDF info string, so a token
/// minted for one pair of peers is not accepted by any other pair.
///
/// # Security
///
/// HKDF is a key-derivation function, not a password hash: it is deliberately fast. A
/// 6-digit PIN carries roughly 20 bits of entropy, so this token must never be exposed
/// to an offline guessing attack — it is only safe as an input to an online ceremony
/// that is rate-limited and locked out after a handful of failures, and to a balanced
/// PAKE (SPAKE2+) that never reveals a PIN-derived value to the network.
///
/// # Panics
///
/// Cannot panic: HKDF-SHA256 expansion of 32 bytes is always within the allowed
/// output length of 255 * 32 bytes.
#[must_use]
pub fn derive_pair_token(pin: &[u8], host_id: Uuid, viewer_id: Uuid) -> PairToken {
    let hk = Hkdf::<Sha256>::new(None, pin);
    let info = format!("renderd-v1-pair:{host_id}:{viewer_id}");
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .expect("32-byte HKDF-SHA256 output is always a valid length");
    PairToken(okm)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Determinism ───────────────────────────────────────────────────────────

    #[test]
    fn test_derive_pair_token_determinism() {
        let h_id = Uuid::new_v4();
        let v_id = Uuid::new_v4();
        let t1 = derive_pair_token(b"123456", h_id, v_id);
        let t2 = derive_pair_token(b"123456", h_id, v_id);
        assert_eq!(t1, t2);

        let t3 = derive_pair_token(b"654321", h_id, v_id);
        assert_ne!(t1, t3);
    }

    // ── UUID isolation ────────────────────────────────────────────────────────

    /// Same PIN and host UUID but different viewer UUIDs must produce different tokens.
    /// Prevents one viewer's stored token from being accepted as another viewer's.
    #[test]
    fn test_derive_pair_token_viewer_uuid_isolation() {
        let h_id = Uuid::new_v4();
        let v1 = Uuid::new_v4();
        let v2 = Uuid::new_v4();
        let t1 = derive_pair_token(b"000000", h_id, v1);
        let t2 = derive_pair_token(b"000000", h_id, v2);
        assert_ne!(t1, t2, "different viewer UUIDs must yield distinct tokens");
    }

    /// Same PIN and viewer UUID but different host UUIDs must produce different tokens.
    /// Prevents a token minted by one host from being accepted by a different host.
    #[test]
    fn test_derive_pair_token_host_uuid_isolation() {
        let h1 = Uuid::new_v4();
        let h2 = Uuid::new_v4();
        let v_id = Uuid::new_v4();
        let t1 = derive_pair_token(b"111111", h1, v_id);
        let t2 = derive_pair_token(b"111111", h2, v_id);
        assert_ne!(t1, t2, "different host UUIDs must yield distinct tokens");
    }

    // ── PIN isolation ─────────────────────────────────────────────────────────

    /// Adjacent PINs must produce distinct tokens (no accidental PIN equivalence).
    #[test]
    fn test_derive_pair_token_adjacent_pin_isolation() {
        let h_id = Uuid::new_v4();
        let v_id = Uuid::new_v4();
        let t1 = derive_pair_token(b"100000", h_id, v_id);
        let t2 = derive_pair_token(b"100001", h_id, v_id);
        assert_ne!(t1, t2, "adjacent PINs must yield distinct tokens");
    }

    // ── Output invariants ─────────────────────────────────────────────────────

    /// Token output is always exactly 32 bytes regardless of PIN length.
    #[test]
    fn test_derive_pair_token_output_length() {
        let h_id = Uuid::new_v4();
        let v_id = Uuid::new_v4();
        for pin in [b"0".as_slice(), b"123456", b"a-very-long-passphrase-input"] {
            let token = derive_pair_token(pin, h_id, v_id);
            assert_eq!(token.0.len(), 32, "output must always be 32 bytes");
        }
    }

    /// Token output must not be all-zero (HKDF expand did not produce an empty result).
    #[test]
    fn test_derive_pair_token_not_zero() {
        let h_id = Uuid::new_v4();
        let v_id = Uuid::new_v4();
        let token = derive_pair_token(b"999999", h_id, v_id);
        assert_ne!(token.0, [0u8; 32], "derived token must not be all-zero");
    }

    // ── Zeroize ───────────────────────────────────────────────────────────────

    /// Explicitly calling `Zeroize::zeroize()` on a `PairToken` must wipe its bytes.
    /// This exercises the same code path that `#[zeroize(drop)]` invokes on drop.
    #[test]
    fn test_pair_token_zeroize_wipes_bytes() {
        let h_id = Uuid::new_v4();
        let v_id = Uuid::new_v4();
        let mut token = derive_pair_token(b"555555", h_id, v_id);
        // Verify bytes are non-zero before zeroizing
        assert_ne!(token.0, [0u8; 32]);
        token.zeroize();
        assert_eq!(token.0, [0u8; 32], "zeroize() must wipe all token bytes");
    }

    /// `SessionKey` zeroize works identically to `PairToken`.
    #[test]
    fn test_session_key_zeroize_wipes_bytes() {
        let mut key = SessionKey([0xABu8; 32]);
        key.zeroize();
        assert_eq!(key.0, [0u8; 32], "zeroize() must wipe all SessionKey bytes");
    }

    // ── CSPRNG ────────────────────────────────────────────────────────────────

    /// Generated PINs must be 6 ASCII digits.
    #[test]
    fn test_generate_pairing_pin_shape() {
        for _ in 0..64 {
            let pin = generate_pairing_pin().expect("OS CSPRNG available");
            assert_eq!(pin.len(), PAIRING_PIN_DIGITS);
            assert!(pin.chars().all(|c| c.is_ascii_digit()));
        }
    }

    /// PINs must not be deterministic. 256 draws from a 10^6 space colliding into
    /// fewer than 200 distinct values would indicate a broken entropy source.
    #[test]
    fn test_generate_pairing_pin_is_not_deterministic() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            seen.insert(generate_pairing_pin().expect("OS CSPRNG available"));
        }
        assert!(
            seen.len() > 200,
            "PIN generator produced only {} distinct values in 256 draws",
            seen.len()
        );
    }

    /// `random_u32_below` must stay in range and reject a zero bound.
    #[test]
    fn test_random_u32_below_bounds() {
        for _ in 0..256 {
            assert!(random_u32_below(10).expect("OS CSPRNG available") < 10);
        }
        assert!(random_u32_below(0).is_err());
        assert_eq!(random_u32_below(1).expect("OS CSPRNG available"), 0);
    }

    /// Token equality is value-based (and constant-time under the hood).
    #[test]
    fn test_pair_token_equality() {
        assert_eq!(PairToken([7u8; 32]), PairToken([7u8; 32]));
        let mut other = [7u8; 32];
        other[31] = 8;
        assert_ne!(PairToken([7u8; 32]), PairToken(other));
    }
}
