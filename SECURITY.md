# Security Policy

## Security Overview

`Renderd` is a high-performance, peer-to-peer macOS host to Windows viewer display streaming system. Security and cryptographic integrity are primary design requirements. Renderd employs Noise Protocol Framework handshakes, AES-256-GCM authenticated payload encryption, and strictly validated control-plane Protobuf schemas.

We take security vulnerabilities seriously and appreciate the open-source community's help in disclosing issues responsibly.

---

## Supported Versions

Only the latest minor release and active pre-release branches receive security updates.

| Version | Supported | Security Maintenance |
| ------- | --------- | -------------------- |
| `0.3.x` | ✅ Yes    | Active development   |
| `0.2.x` | ✅ Yes    | Critical fixes only  |
| `0.1.x` | ❌ No     | Unsupported          |

---

## Reporting a Vulnerability

**Please do NOT report security vulnerabilities through public GitHub issues.**

Instead, please report security issues using one of the following methods:

1. **GitHub Private Vulnerability Reporting (Preferred):**  
   Navigate to the [Security tab](https://github.com/Ad1th/renderd/security) of the repository and click **"Report a vulnerability"**.

2. **Security Email:**  
   Send an encrypted email detailing the vulnerability to `security@renderd.dev`.

### What to Include in Your Report

To help us investigate and resolve the issue quickly, please include:
- A description of the vulnerability and its potential impact.
- The specific crate or subsystem affected (e.g., `renderd-crypto`, `renderd-net`, `renderd-proto`).
- Step-by-step reproduction instructions or a minimal Proof of Concept (PoC).
- Any proposed remediation or patch, if available.
- Environment details (macOS host version, Windows viewer version, Rust compiler version).

---

## Response & Disclosure Process

1. **Acknowledgement:** We will acknowledge receipt of your vulnerability report within **48 hours**.
2. **Triage:** The Renderd maintainers and security team will assess the severity and impact within **7 days**.
3. **Remediation:** We will develop and test a fix. If necessary, we will coordinate a backport to supported release branches.
4. **Fix & Release:** A patch release will be published along with a Security Advisory (GHSA).
5. **Public Credit:** Reporter(s) will be credited in the release notes and security advisory, unless anonymity is requested.

We follow a **90-day coordinated disclosure policy**.

---

## Automated Security Audits

Renderd continuously validates dependency security through automated CI pipelines:
- **`cargo-deny`:** Audits dependency licenses, banned crates, and security advisories on every PR.
- **`cargo-audit`:** Scans `Cargo.lock` against the RustSec Advisory Database weekly.
- **Cryptographic Audit:** Formal audits are documented in [`docs/CRYPTO-AUDIT.md`](docs/CRYPTO-AUDIT.md).
