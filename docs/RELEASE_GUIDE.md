# Release Engineering & Release Guide

This guide details the release engineering procedures, semantic versioning policy, release cadence, and automation workflows for Renderd.

---

## 1. Versioning Policy

Renderd follows **Semantic Versioning 2.0.0** (`MAJOR.MINOR.PATCH[-PRERELEASE]`).

### Host Daemon & Viewer Client

The macOS Host Daemon (`renderd-host`) and Windows Viewer Client (`renderd-viewer`) share a unified workspace version number and are released synchronously.

- **MAJOR:** Incompatible protocol schema or wire-format changes (e.g. `renderd.proto` envelope changes requiring both sides to upgrade simultaneously).
- **MINOR:** Backward-compatible new capabilities, new codecs, or additional control messages with fallback.
- **PATCH:** Bug fixes, latency optimizations, performance enhancements, and security patches.
- **PRERELEASE:** Pre-release milestones (e.g., `v0.3.0-primitives`, `v0.4.0-capture`).

---

## 2. Release Cadence

- **Patch Releases:** As needed for security vulnerability fixes or critical regressions.
- **Minor Releases:** Every 6–8 weeks following milestone stabilization on `main`.
- **Major Releases:** Scheduled only when breaking protocol schema changes are required.

---

## 3. Automated Release Automation

Releases are driven by GitHub Actions when a version tag (`v*`) is pushed to the repository.

### Workflow Pipeline (`.github/workflows/release.yml`)

1. **Tag Push Trigger:** Pushing `vX.Y.Z` triggers the `Release` workflow.
2. **Release Notes Extraction:** Automatically extracts the release notes section matching `## [vX.Y.Z]` from `CHANGELOG.md`.
3. **macOS Host Build:** Builds `renderd-host` for `aarch64-apple-darwin`, packages tarball archive, and generates SHA256 checksum.
4. **Windows Viewer Build:** Builds `renderd-viewer` for `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, packages zip archives, and generates SHA256 checksums.
5. **GitHub Release Publication:** Aggregates `SHA256SUMS.txt`, creates the GitHub Release, attaches all binary archives, and publishes release notes.

---

## 4. Release Execution Checklist

### Step 1: Pre-Release Verification

Run pre-release checks locally:

```bash
# Verify workspace compilation
cargo check --workspace

# Run quality checks
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
```

### Step 2: Update CHANGELOG.md

Ensure all changes since the previous tag are listed under `## [vX.Y.Z] — YYYY-MM-DD` in `CHANGELOG.md`.

You can generate or preview entries using:

```bash
bash scripts/generate-changelog.sh
```

### Step 3: Run Release Preparation Script

Execute the interactive release preparation script:

```bash
bash scripts/prepare-release.sh 0.3.0
```

This script will:
- Verify that git working tree is clean.
- Run all workspace lints, tests, and policy audits.
- Update version in `Cargo.toml` and sync `Cargo.lock`.
- Create a Conventional Commit `chore(release): prepare v0.3.0`.
- Create annotated git tag `v0.3.0`.

### Step 4: Push Branch and Release Tag

```bash
git push origin main
git push origin v0.3.0
```

Once pushed, GitHub Actions will automatically compile binaries, generate checksums, and publish the GitHub Release.
