# Contributing to Renderd

Thank you for your interest in contributing to Renderd! This document provides guidelines and standards for contributing to the repository.

Renderd is built to feel like a world-class, mature open-source project. All contributions are expected to adhere to our design principles, quality standards, and code of conduct.

---

## Table of Contents

- [1. Before You Start](#1-before-you-start)
- [2. Environment Setup](#2-environment-setup)
- [3. Development Workflow](#3-development-workflow)
- [4. Branch Strategy](#4-branch-strategy)
- [5. Conventional Commits](#5-conventional-commits)
- [6. Pull Request Guidelines](#6-pull-request-guidelines)
- [7. Labels & Milestones](#7-labels--milestones)
- [8. GitHub Discussions](#8-github-discussions)

---

## 1. Before You Start

Before writing code or opening a pull request:

1. **Architecture & Standards:** Read [`docs/RFC-0002-architecture.md`](docs/RFC-0002-architecture.md) and [`docs/REPO-0001-repository.md`](docs/REPO-0001-repository.md).
2. **Search Existing Issues:** Check if an issue already exists for your proposed change or bug fix.
3. **Open an Issue:** For new features or non-trivial refactors, please open an issue first to discuss the design with maintainers before submitting a pull request.

---

## 2. Environment Setup

### Prerequisites

- **Rust:** Stable toolchain 1.80 or higher (`rustup toolchain install stable`).
- **macOS (Host Development):** Xcode Command Line Tools (`xcode-select --install`). Target architecture: `aarch64-apple-darwin`.
- **Windows (Viewer Development):** Visual Studio 2022 C++ Workload (Windows 10/11 SDK). Target architecture: `x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc`.

### Cloning & Initial Verification

```bash
git clone https://github.com/Ad1th/renderd.git
cd renderd

# Verify toolchain and check workspace compilation
cargo check --workspace

# Install cargo-nextest test runner (recommended)
cargo install cargo-nextest --locked
```

---

## 3. Development Workflow

### Protobuf Code Generation

If you modify `proto/renderd.proto`, you must run the code generator tool to regenerate Rust prost bindings in `crates/renderd-proto/src/generated/`:

```bash
cargo run --manifest-path tools/proto-gen/Cargo.toml
```

CI will verify that generated proto code matches the schema.

### Quality Checks

Before submitting a PR, ensure all verification steps pass locally:

```bash
# 1. Format check
cargo fmt --check

# 2. Strict workspace Clippy lints
cargo clippy --workspace --all-targets -- -D warnings

# 3. Unit and property-based test execution
cargo nextest run --workspace

# 4. Dependency & license policy audit
cargo deny check
```

---

## 4. Branch Strategy

Renderd follows **GitHub Flow** with release branches (`release/X.Y`).

### Branch Naming Convention

| Prefix | Use Case | Example |
| ------ | -------- | ------- |
| `feat/` | New user-facing features or API additions | `feat/dual-vsync-sync` |
| `fix/` | Bug fixes | `fix/fragment-deadline-arithmetic` |
| `perf/` | Latency or throughput optimizations | `perf/burst-send-batching` |
| `refactor/` | Code refactoring without behavioral change | `refactor/extract-abr-ramp` |
| `chore/` | Dependency bumps, CI updates, tooling | `chore/bump-quinn-0.12` |
| `docs/` | Documentation additions or updates | `docs/rfc-0003-audio` |
| `release/` | Release branch stabilization | `release/0.3` |
| `security/` | Security fixes | `security/hkdf-info-fix` |

---

## 5. Conventional Commits

Renderd strictly enforces **Conventional Commits** (https://www.conventionalcommits.org). Every commit message and PR title must follow this format:

```text
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Allowed Types

- `feat`: New feature or user-visible capability.
- `fix`: Bug fix.
- `perf`: Performance or latency improvement (must cite benchmark numbers in commit body).
- `refactor`: Internal code structure change with no external behavior change.
- `docs`: Documentation changes only.
- `test`: Adding or modifying tests.
- `chore`: Dependency updates, build script changes, CI updates.
- `ci`: Changes to GitHub Actions workflows or automation scripts.
- `security`: Security fixes.
- `revert`: Reverting a previous commit.

### Allowed Scopes

`renderd-proto`, `renderd-config`, `renderd-frame`, `renderd-crypto`, `renderd-vt-sys`, `renderd-sc-sys`, `renderd-net`, `renderd-keychain`, `renderd-discovery`, `renderd-abr`, `renderd-clock`, `renderd-host`, `renderd-viewer`, `deps`, `ci`, `docs`, `release`.

---

## 6. Pull Request Guidelines

1. **Title:** Must adhere to Conventional Commits (e.g., `feat(renderd-net): implement QUIC datagram congestion window observer`).
2. **Template:** Complete all sections of the [Pull Request Template](.github/PULL_REQUEST_TEMPLATE.md).
3. **Benchmark Numbers:** Required for any changes touching data-plane crates (`renderd-frame`, `renderd-net`, `renderd-crypto`, `renderd-abr`, `renderd-clock`).
4. **Clean Commits:** Keep commits atomic, logical, and well-described.
5. **CI Compliance:** All CI status checks must be green prior to review approval.

---

## 7. Labels & Milestones

- Issues and PRs are categorized using standard label taxonomies (`type/*`, `crate/*`, `platform/*`, `priority/*`, `status/*`).
- Development is structured into numbered engineering milestones (`Milestone 1` through `Milestone 10`). Refer to [`docs/ISSUES-0001-milestones.md`](docs/ISSUES-0001-milestones.md) for current progress.

---

## 8. GitHub Discussions

We use GitHub Discussions for community engagement, questions, design brainstorming, and showcase:

- **Announcements:** Project releases, roadmap updates, and news (Maintainers).
- **Q&A / Help:** Questions on usage, architecture, or building Renderd.
- **Ideas & RFC Proposals:** Feature proposals and architectural discussion before formal RFC drafting.
- **Show and Tell:** Community setups, custom hardware display rigs, and benchmark results.
- **General:** Open community discussion.
