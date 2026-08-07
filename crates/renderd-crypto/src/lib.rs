//! `SPAKE2+` pairing, `HKDF` key derivation, and `TLS` cert generation.

use hkdf::Hkdf;
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroize;

/// 256-bit derived pairing token handle.
#[derive(Debug, Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct PairToken(pub [u8; 32]);

/// 256-bit derived session key handle.
#[derive(Debug, Clone, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct SessionKey(pub [u8; 32]);

/// Derives a 32-byte [`PairToken`] from a pairing PIN, host UUID, and viewer UUID using HKDF-SHA256.
///
/// # Panics
///
/// Cannot panic under normal execution.
#[must_use]
pub fn derive_pair_token(pin: &[u8], host_id: Uuid, viewer_id: Uuid) -> PairToken {
    let hk = Hkdf::<Sha256>::new(None, pin);
    let info = format!("renderd-v1-pair:{host_id}:{viewer_id}");
    let mut okm = [0u8; 32];
    let _ = hk.expand(info.as_bytes(), &mut okm);
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
}
