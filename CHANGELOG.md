# Changelog

All notable changes to Renderd will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initialized Cargo workspace root with resolver v2 and declared all 14 member crates across 5 DAG layers (`crates/renderd-*` and `tools/latency-bench`) per REPO-0001. (#001)
- Added `rust-toolchain.toml` pinning Rust stable channel (MSRV 1.80+) with `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, and `aarch64-pc-windows-msvc` target support. (#001)
- Configured workspace-level dependency versions, lint policies, and build profiles (`dev`, `release`, `bench`). (#001)
