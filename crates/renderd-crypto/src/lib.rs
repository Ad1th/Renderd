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
}
