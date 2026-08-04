# Contributing to Renderd

Thank you for your interest in contributing to Renderd.

This file is a quick-start reference. The **complete and authoritative** contribution
guide is §21 of [`docs/REPO-0001-repository.md`](docs/REPO-0001-repository.md).

---

## Before You Start

Read these documents **in order** before writing any code:

1. **[RFC-0002](docs/RFC-0002-architecture.md)** — the canonical architecture.
   Understand every design decision and why it was made.
2. **[REPO-0001](docs/REPO-0001-repository.md)** — engineering standards, coding
   rules, and CI requirements. Every standard here is mechanically enforced.
3. **Open Issues** — check whether your planned change addresses an open issue or
   is a duplicate. If no issue exists, open one and wait for a maintainer to label
   it `accepted` before implementing.

---

## Development Setup

**macOS (host development):**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cd renderd
cargo build --package renderd-frame   # verify setup with a leaf crate

cargo install cargo-nextest --locked
cargo install cargo-llvm-cov  --locked
cargo install cargo-deny      --locked
cargo install typos-cli       --locked
```

**Windows (viewer development):**
```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools
cargo build --package renderd-viewer
```

**Library crates** (`renderd-frame`, `renderd-crypto`, `renderd-abr`, …) compile on
any platform.

---

## Workflow

```bash
git checkout -b feat/your-feature

# Make changes, then:
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo nextest run --package <affected-crate>

git commit -m "feat(renderd-frame): add configurable window depth"
git push origin feat/your-feature
# Open a pull request on GitHub
```

---

## Pull Request Checklist

- [ ] PR description explains **what** changed and **why**
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] All tests pass (`cargo nextest run`)
- [ ] New behaviour is covered by new tests
- [ ] `cargo deny check` passes
- [ ] If the data plane changed: benchmark numbers are included
- [ ] If public API changed: `cargo doc` builds cleanly
- [ ] If a protocol message changed: RFC-0002 (or a new RFC) is updated
- [ ] `CHANGELOG.md` has an entry under `[Unreleased]`

See [REPO-0001 §21.4](docs/REPO-0001-repository.md#214-pull-request-requirements)
for the full requirements.

---

## What Not to Contribute

- Dependencies not licensed MIT or Apache-2.0
- `unsafe` code outside `renderd-vt-sys` / `renderd-sc-sys` (requires design discussion first)
- Cryptographic protocol changes without an accompanying RFC and specialist review
- `unwrap()` in library code — use `Result`
- Features listed in RFC-0002 §20 (Future Work) without prior discussion

---

## Security Vulnerabilities

**Do not open a public GitHub issue for security vulnerabilities.**

Email `security@renderd.dev` with subject `[SECURITY] <brief description>`.
See [`SECURITY.md`](SECURITY.md) for the full disclosure process.

---

## Code of Conduct

Renderd adopts the [Contributor Covenant v2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
Reports of violations go to `conduct@renderd.dev`.
