# Changelog

All notable changes to Renderd will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initialized Cargo workspace root with resolver v2 and declared all 14 member crates across 5 DAG layers (`crates/renderd-*` and `tools/latency-bench`) per REPO-0001. (#001)
- Added `rust-toolchain.toml` pinning Rust stable channel (MSRV 1.80+) with `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, and `aarch64-pc-windows-msvc` target support. (#001)
- Configured workspace-level dependency versions, lint policies, and build profiles (`dev`, `release`, `bench`). (#001)
- Configured workspace-level `clippy.toml` setting `msrv = "1.80"`, disallowing `std::process::exit` and `std::env::var`, and restricting raw array pair-tokens. (#002)
- Added per-crate lint overrides setting `unsafe_code = "deny"` for non-FFI crates and `unsafe_code = "warn"` for FFI crates (`renderd-vt-sys`, `renderd-sc-sys`) per REPO-0001 §9. (#002)
- Added root `.rustfmt.toml` workspace formatting configuration per REPO-0001 §10. (#003)
- Configured cargo-deny policy in `deny.toml` for license checking, dependency bans, and security advisory checking per REPO-0001 §9. (#004)
- Added test runner configuration in `nextest.toml` per REPO-0001 §14.7. (#005)
- Added primary CI workflow in `.github/workflows/ci.yml` per REPO-0001 §18.1. (#006)
- Added security audit workflow in `.github/workflows/security.yml` per REPO-0001 §18.2. (#007)
- Added nightly benchmark workflow in `.github/workflows/bench.yml` per REPO-0001 §18.3. (#008)
- Added code ownership rules in `.github/CODEOWNERS` per REPO-0001 §7.1. (#009)
- Added PR template and GitHub Issue forms for bug reports, latency regressions, and feature requests. (#010)
- Added spell-checking configuration in `_typos.toml` and `.github/workflows/typos.yml`. (#011)
- Added documentation deployment workflow in `.github/workflows/docs.yml` per REPO-0001 §18.5. (#012)
- Added protobuf protocol schema definition in `proto/renderd.proto` per RFC-0002 §11. (#013)
- Added `tools/proto-gen` code generator and generated `prost` Rust types in `renderd-proto` crate. (#014)
- Added protocol domain newtypes (`FrameId`, `FragmentId`, `BitrateKbps`, `VsyncPeriodNs`, `ViewerId`, `HostId`) in `renderd-proto` crate. (#015)
- Added envelope dispatch helpers (`MessageKind`) and message validation logic for `SessionHello` and `SessionConfig` in `renderd-proto` crate. (#016)
- Enabled proto freshness CI check step in `.github/workflows/ci.yml`. (#017)
- Added configuration schema structs (`RenderdConfig`, `HostConfig`, `ViewerConfig`, `NetworkConfig`, `CryptoConfig`, `AbrConfig`) in `renderd-config` crate. (#018)
- Added Figment-based layered config loader (`ConfigBuilder`) supporting defaults, TOML files, environment variables, and CLI overrides in `renderd-config` crate. (#019)
- Added configuration validation rules (`ValidateConfig` trait) for `RenderdConfig` in `renderd-config` crate. (#020)
- Added `ConfigError` enum hierarchy (`FileNotFound`, `ParseError`, `ValidationError`) and `Display` implementations in `renderd-config` crate. (#021)
