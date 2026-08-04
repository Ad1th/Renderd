# Cryptographic and Security Audit Report — Renderd v0.3.0

**Audit Date:** August 5, 2026  
**Target Specifications:** `RFC-0002`, `REPO-0001`, `README.md`  
**Target Codebase:** Workspace crates (`renderd-crypto`, `renderd-proto`, `renderd-config`, `renderd-net`)  
**Auditor:** Senior Rust Systems & Cryptography Security Engineer  

---

## Executive Summary

A comprehensive cryptographic and security architecture audit was conducted for the Renderd peer-to-peer display streaming project. The audit evaluated SPAKE2+ pairing, HKDF key derivation, certificate generation, pairing protocols, trust models, replay and downgrade attack mitigations, MITM resistance, entropy, unsafe code boundaries, and FFI safety.

### Summary of Findings
- **Critical Findings:** **0** (No critical vulnerabilities identified; no immediate code changes required).
- **High Findings:** **0**
- **Medium Findings:** **2**
- **Low Findings:** **2**
- **Nitpick Findings:** **2**

All cryptographic architectural decisions in `RFC-0002` (QUIC/TLS 1.3, SPAKE2+ RFC 9382 PAKE, mutual mTLS authentication, HKDF-SHA256 with canonical UUID info strings) are sound. Recommendations for implementation in Milestone 5 are detailed below.

---

## Detailed Findings & Risk Analysis

### FIND-01: SPAKE2+ RFC 9382 Compliance & Test Vector Validation
- **Classification:** **Medium**
- **Component:** `crates/renderd-crypto/src/spake2plus/` (Milestone 5)
- **Description:** `RFC-0002 §8.2` specifies SPAKE2+ per **RFC 9382** (August 2023). Standard SPAKE2 crates (e.g. `spake2`) implement earlier SPAKE2 drafts that lack explicit mutual verifier setup ($w_0, w_1, L, Y$) and role-differentiating MAC confirmations ($confirmP$, $confirmV$).
- **Impact:** Attempting to use a standard SPAKE2 crate without RFC 9382 test vector validation would produce protocol incompatibility and break mutual prover/verifier binding.
- **Exploitability:** Low in pre-release; Medium during protocol implementation.
- **Recommendation:** Enforce official IETF test vectors from RFC 9382 §4 in `crates/renderd-crypto/tests/spake2plus_vectors.rs` before deploying the pairing state machine.

---

### FIND-02: Immediate Active Connection Termination on Certificate Revocation
- **Classification:** **Medium**
- **Component:** `renderd-host/src/session/devices.rs` (Milestone 7)
- **Description:** `RFC-0002 §9.3` defines viewer certificate revocation by removing the viewer's certificate from the host's `known-viewers` registry. However, an established TLS 1.3 session already past the handshake phase will continue streaming unless active socket handles are explicitly killed.
- **Impact:** A revoked viewer client could potentially continue receiving stream fragments on an active connection until socket disconnect.
- **Exploitability:** Low (requires active stream at the exact moment of revocation).
- **Recommendation:** When `revoke_viewer(viewer_id)` is executed, `renderd-host` must trigger an immediate forced shutdown (`ApplicationClose(0x04)`) on all active QUIC connections matching `viewer_id`.

---

### FIND-03: HKDF Key Derivation Domain Separation
- **Classification:** **Low**
- **Component:** `renderd-crypto/src/hkdf.rs` (Milestone 5)
- **Description:** In `RFC-0002 §8.2` & `§9.4`:
  - `PairToken = HKDF-SHA256(ikm = K, salt = "renderd-v1-pair-token", info = "renderd-v1-pair:" || host_uuid || ":" || viewer_uuid)`
  - `SessionKey = HKDF-SHA256(PairToken, session_nonce, "renderd-v1-session")`
  The UUID context binding uses fixed 36-byte canonical strings, successfully preventing length-extension collisions. Passing `session_nonce` as salt in `SessionKey` derivation is cryptographically sound, but parameter ordering should be strictly enforced via type wrappers.
- **Impact:** Low risk of context confusion if info strings are not strictly domain-separated.
- **Exploitability:** Low.
- **Recommendation:** Use strongly-typed wrappers (`PairTokenSecrets`, `SessionKeySecrets`) ensuring `hkdf::Hkdf` calls explicitly bind domain tags (`b"renderd-v1-pair-token"`, `b"renderd-v1-session-key"`).

---

### FIND-04: Pairing Rate-Limiting State Persistence
- **Classification:** **Low**
- **Component:** `renderd-host/src/pairing/` (Milestone 7)
- **Description:** `RFC-0002 §8.2` specifies exponential lockout for failed PIN attempts (5 attempts $\rightarrow$ 120s). If rate-limiting counters are tied solely to single connection lifetimes, an attacker could bypass lockout by opening new UDP sockets for each attempt.
- **Impact:** Increased susceptibility to online PIN brute-force attacks across network reconnects.
- **Exploitability:** Low (60-second PIN expiration window limits online attempt volume).
- **Recommendation:** Store failed attempt counters and lockout timestamps in host daemon global memory keyed by remote IP address and viewer UUID.

---

### FIND-05: Warning Guard for `require_auth = false` Config Override
- **Classification:** **Nitpick**
- **Component:** `crates/renderd-config/src/validate.rs`
- **Description:** `CryptoConfig` allows setting `require_auth = false`. While useful for local testing, unauthenticated connections in production bypass mutual TLS.
- **Impact:** Potential stream interception if authentication is disabled in production.
- **Exploitability:** Low (requires manual config file modification).
- **Recommendation:** Log a `tracing::warn!` alert whenever `require_auth` is set to `false`.

---

### FIND-06: Key Material Memory Zeroization (`zeroize`)
- **Classification:** **Nitpick**
- **Component:** `crates/renderd-crypto`
- **Description:** Secrets stored in heap/stack memory (`PairToken`, session keys, ephemeral SPAKE2+ scalars) should be zeroized upon drop to prevent key material from persisting in RAM dumps.
- **Impact:** Residual secret bytes in memory.
- **Exploitability:** Low.
- **Recommendation:** Derive `Zeroize` and `ZeroizeOnDrop` for all key wrappers in `renderd-crypto`.

---

## Categorized Security Analysis

| Category | Assessment & Verification Status |
|----------|----------------------------------|
| **SPAKE2+** | Protocol choice (RFC 9382 PAKE) is mathematically robust against offline PIN dictionary attacks. Test vector validation mandatory in Milestone 5. |
| **HKDF** | Uses SHA-256 with static domain separators and canonical fixed-length 36-byte UUID strings. Prevents length-ambiguity collisions. |
| **Certificate Generation** | Self-signed TLS 1.3 certificates generated via `rcgen`, derived from `PairToken`, with 10-year validity and 180-day advance auto-renewal. |
| **Pairing Protocol** | 6-digit random PIN displayed on host UI; 60s expiration; 5-attempt exponential lockout (120s); dual MAC confirmation ($confirmP, confirmV$). |
| **Trust Model** | Peer-to-peer mutual authentication (mTLS). Keychain storage (`macOS Keychain Services` / `Windows Credential Manager`). Zero cloud dependencies. |
| **Replay Attacks** | Mitigated by TLS 1.3 record sequence numbers, QUIC packet sequence numbers, and per-session random nonces in `SessionHello`. |
| **Downgrade Attacks** | TLS 1.3 mandatory in `quinn`; no unencrypted or TLS 1.2 fallback paths. `require_auth` enforced by default. |
| **MITM Resistance** | Attacker without PIN cannot complete SPAKE2+ exchange. Post-pairing mTLS verifies certificates pinned during pairing. |
| **Entropy** | System CSPRNG (`ring` / `getrandom`) used for PIN generation, session nonces, and ephemeral keypairs. |
| **Unsafe Code** | `#![deny(unsafe_code)]` enforced across `renderd-crypto` and all non-FFI crates. 0 `unsafe` blocks present. |
| **FFI Boundaries** | FFI crates (`renderd-vt-sys`, `renderd-sc-sys`) strictly separated from security/crypto logic. |

---

## Conclusion

The security architecture specified in `RFC-0002` and `REPO-0001` is sound, robust, and free of Critical defects. Since **0 Critical findings** were identified, no emergency code fixes are required at this stage. Milestone 5 may proceed when scheduled, incorporating the recommendations from FIND-01 through FIND-06.
