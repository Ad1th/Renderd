# ISSUES-0001: Milestone-by-Milestone GitHub Issues Breakdown

```
Title:      Renderd — GitHub Issues & Project Roadmap
Doc:        ISSUES-0001
Status:     Draft
Applies:    All milestones and crates
Created:    2026-08-04
Refs:       RFC-0002-architecture.md, REPO-0001-repository.md
Total:      100 granular issues across 8 milestones (< 1 day effort each)
```

---

## Overview & Milestone Roadmap

This document breaks down the implementation of Renderd into **100 actionable, bite-sized GitHub Issues** across **8 milestones**. Each issue represents less than one day of engineering effort, with strict acceptance criteria, dependency tracking, and testing requirements to enable concurrent execution by multiple engineers.

```
┌────────────────────────────────────────────────────────────────────────┐
│ Milestone 1: Repository Bootstrap & Infrastructure     (Issues #001–#012)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 2: Foundation Layer (Proto & Config)         (Issues #013–#022)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 3: Primitive Layer (Frame & Crypto)          (Issues #023–#035)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 4: FFI Layer (VideoToolbox & SCKit Shims)    (Issues #036–#047)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 5: Service Layer (Net, Keychain & Discovery) (Issues #048–#063)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 6: Algorithm Layer (ABR & Clock Sync)        (Issues #064–#075)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 7: Host Application (`renderd-host`)         (Issues #076–#088)│
├────────────────────────────────────────────────────────────────────────┤
│ Milestone 8: Viewer Application (`renderd-viewer`)     (Issues #089–#100)│
└────────────────────────────────────────────────────────────────────────┘
```

---

## Milestone 1: Repository Bootstrap & Infrastructure (Issues #001–#012)

Focuses strictly on workspace configuration, tooling enforcement, CI workflows, and build automation. No feature code is included.

---

### Issue #001: Initialize Cargo Workspace Root and `rust-toolchain.toml`
- **Rationale:** Establishes the workspace boundary and pins the Rust compiler version across environments.
- **Dependencies:** None
- **Acceptance Criteria:**
  - Root `Cargo.toml` created with `resolver = "2"` and all 13 member crates declared.
  - `rust-toolchain.toml` created pinning Rust `stable` channel (MSRV 1.80+) with `aarch64-apple-darwin` and `x86_64-pc-windows-msvc` target support.
  - Workspace-level dependency versions specified in `[workspace.dependencies]`.
- **Testing:** `cargo metadata --format-version 1` succeeds cleanly.
- **Estimated Effort:** 2 hours

---

### Issue #002: Configure Workspace Lints and `clippy.toml`
- **Rationale:** Enforces uniform code safety and quality standards across all crates from day zero.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - Workspace-level lints set in `Cargo.toml` (`unsafe_code = "warn"`, `missing_docs = "warn"`, `unused_must_use = "deny"`).
  - `clippy.toml` created with `msrv = "1.80"`, disallowing `std::process::exit` and `std::env::var`.
- **Testing:** `cargo clippy --workspace -- -D warnings` runs without syntax errors.
- **Estimated Effort:** 2 hours

---

### Issue #003: Configure Workspace Code Formatting (`.rustfmt.toml`)
- **Rationale:** Eliminates formatting debates and ensures consistent diffs across contributions.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `.rustfmt.toml` configured with `max_width = 100`, `edition = "2021"`, `imports_granularity = "Crate"`, and `group_imports = "StdExternalCrate"`.
- **Testing:** `cargo fmt --check` executes without configuration errors.
- **Estimated Effort:** 1 hour

---

### Issue #004: Configure License & Dependency Audit Policy (`deny.toml`)
- **Rationale:** Prevents GPL/AGPL dependency contamination and flags vulnerable/duplicate crates automatically.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `deny.toml` created allowing MIT, Apache-2.0, ISC, and BSD licenses.
  - Vulnerability and duplicate checking configured.
- **Testing:** `cargo deny check` runs successfully against workspace dependencies.
- **Estimated Effort:** 2 hours

---

### Issue #005: Setup Test Runner Configuration (`nextest.toml`)
- **Rationale:** Accelerates test execution and standardizes retry policies for CI.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `nextest.toml` configured with default profile retries (`1`), CI profile retries (`2`), and JUnit XML output path `target/nextest/junit.xml`.
- **Testing:** `cargo nextest run --workspace` detects config file without warnings.
- **Estimated Effort:** 2 hours

---

### Issue #006: Create Primary CI Workflow (`.github/workflows/ci.yml`)
- **Rationale:** Guarantees every pull request is automatically tested, formatted, and linted across macOS and Windows runners.
- **Dependencies:** #001, #002, #003, #005
- **Acceptance Criteria:**
  - GitHub Actions matrix setup for `macos-15` and `windows-2025`.
  - Enforces `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`.
- **Testing:** Workflow syntax validated via `actionlint` or manual trigger.
- **Estimated Effort:** 4 hours

---

### Issue #007: Setup Security Audit Workflow (`.github/workflows/security.yml`)
- **Rationale:** Automates weekly supply chain security audits for Rust dependencies.
- **Dependencies:** #004
- **Acceptance Criteria:**
  - Workflow runs `cargo-deny` and `cargo-audit` on PRs modifying `Cargo.lock` and on a weekly cron schedule (Mondays 06:00 UTC).
- **Testing:** Workflow triggers successfully on branch push.
- **Estimated Effort:** 2 hours

---

### Issue #008: Setup Nightly Benchmark Workflow (`.github/workflows/bench.yml`)
- **Rationale:** Tracks latency and throughput regressions automatically over time.
- **Dependencies:** #006
- **Acceptance Criteria:**
  - Nightly workflow runs `cargo bench` on `macos-15`.
  - Publishes benchmark summaries via `github-action-benchmark` with a 10% regression alert threshold.
- **Testing:** Workflow file created and validated.
- **Estimated Effort:** 3 hours

---

### Issue #009: Setup Code Ownership Rules (`.github/CODEOWNERS`)
- **Rationale:** Directs pull request reviews automatically to crate specialists (macOS, Windows, Crypto, Algorithms).
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `.github/CODEOWNERS` created mapping `/crates/renderd-crypto/`, `/crates/renderd-vt-sys/`, `/crates/renderd-viewer/`, etc., to team handles per REPO-0001 §7.
- **Testing:** GitHub UI recognizes code ownership rules.
- **Estimated Effort:** 1 hour

---

### Issue #010: Add Issue Templates and Pull Request Template
- **Rationale:** Standardizes bug reports, latency regressions, and PR descriptions with benchmark requirements.
- **Dependencies:** None
- **Acceptance Criteria:**
  - `.github/PULL_REQUEST_TEMPLATE.md` added with checklist and benchmark impact section per REPO-0001 §21.5.
  - `.github/ISSUE_TEMPLATE/bug_report.yml` and `latency_regression.yml` created.
- **Testing:** Formats render properly in GitHub web UI.
- **Estimated Effort:** 2 hours

---

### Issue #011: Setup Spell-Checking and Typos Workflow (`.github/workflows/typos.yml`)
- **Rationale:** Catches typos in code, comments, and documentation automatically.
- **Dependencies:** None
- **Acceptance Criteria:**
  - `typos.yml` workflow created using `crate-ci/typos`.
  - `_typos.toml` configured to ignore false positives (e.g., codec names, FFI symbols).
- **Testing:** `typos` binary passes locally on repository root.
- **Estimated Effort:** 2 hours

---

### Issue #012: Setup Documentation Deployment Workflow (`.github/workflows/docs.yml`)
- **Rationale:** Publishes rustdoc output for all library crates automatically to GitHub Pages.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - Workflow runs `cargo doc --no-deps --workspace` excluding binary crates (`renderd-host`, `renderd-viewer`).
  - Deploys output to GitHub Pages on pushes to `main`.
- **Testing:** Local `cargo doc` execution produces no errors.
- **Estimated Effort:** 3 hours

---

## Milestone 2: Foundation Layer (Issues #013–#022)

Builds `renderd-proto` (protobuf definitions) and `renderd-config` (validated settings).

---

### Issue #013: Create Protobuf Protocol Schema (`proto/renderd.proto`)
- **Rationale:** Defines the single source of truth for all control plane messages across network boundaries.
- **Dependencies:** None
- **Acceptance Criteria:**
  - `proto/renderd.proto` created containing `Envelope`, `SessionHello`, `SessionConfig`, `VsyncReport`, `ReactiveStats`, `PeriodicStats`, `KeyframeRequest`, `BitrateAdjust`, `StreamReconfigure`, and `Error` messages per RFC-0002 §11.
- **Testing:** `protoc --encode` succeeds against test payload.
- **Estimated Effort:** 4 hours

---

### Issue #014: Scaffold `renderd-proto` Crate and Code Generator Tool (`tools/proto-gen`)
- **Rationale:** Generates Rust prost types from `.proto` definitions into `renderd-proto`.
- **Dependencies:** #001, #013
- **Acceptance Criteria:**
  - `crates/renderd-proto/` scaffolded.
  - `tools/proto-gen` tool uses `prost-build` to emit Rust code into `crates/renderd-proto/src/generated/`.
- **Testing:** `tools/proto-gen` produces valid Rust files.
- **Estimated Effort:** 4 hours

---

### Issue #015: Implement Protocol Newtypes (`renderd-proto/src/types.rs`)
- **Rationale:** Prevents type-confusion bugs (e.g., passing a frame ID as a bitrate) by wrapping raw primitive types.
- **Dependencies:** #014
- **Acceptance Criteria:**
  - Implement `FrameId(u64)`, `FragmentId(u16)`, `BitrateKbps(u32)`, `VsyncPeriodNs(u64)`, `ViewerId(Uuid)`, and `HostId(Uuid)`.
  - Display, Copy, Clone, PartialEq, and Eq traits implemented.
- **Testing:** Unit tests verify conversions to and from raw primitives.
- **Estimated Effort:** 3 hours

---

### Issue #016: Implement Envelope Dispatch and Validation (`renderd-proto/src/envelope.rs`)
- **Rationale:** Provides safe pattern matching over incoming protobuf `oneof` payloads.
- **Dependencies:** #014, #015
- **Acceptance Criteria:**
  - `Envelope::kind(&self) -> MessageKind` implemented.
  - `SessionHello::validate(&self) -> Result<(), ProtoError>` checks non-empty strings and valid ranges.
- **Testing:** Unit tests verify invalid `SessionHello` fields trigger validation errors.
- **Estimated Effort:** 4 hours

---

### Issue #017: Setup Proto Freshness CI Check (`.github/workflows/proto-check.yml`)
- **Rationale:** Guarantees checked-in generated code is always in sync with `renderd.proto`.
- **Dependencies:** #014
- **Acceptance Criteria:**
  - CI job runs `proto-gen` and executes `git diff --exit-code`. Fails if generated code is stale.
- **Testing:** Trigger CI job with modified `.proto` file to confirm failure.
- **Estimated Effort:** 2 hours

---

### Issue #018: Scaffold `renderd-config` Crate & Schema Structs
- **Rationale:** Defines host and viewer configuration models with strong typing.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `crates/renderd-config/` scaffolded with `HostConfig`, `ViewerConfig`, `NetworkConfig`, `EncodeConfig`, `AbrConfig`, and `LogConfig`.
  - Serde `#[serde(default)]` applied to all fields.
- **Testing:** Default configurations deserialize from empty TOML string.
- **Estimated Effort:** 4 hours

---

### Issue #019: Implement Layered Config Loader (`renderd-config/src/load.rs`)
- **Rationale:** Supports config loading from default TOML files, custom paths, and environment variable overrides (`RENDERD_*`).
- **Dependencies:** #018
- **Acceptance Criteria:**
  - `Config::load(path: Option<&Path>) -> Result<Config, ConfigError>` implemented using `figment`.
- **Testing:** Unit test verifies environment variable `RENDERD_NETWORK_PORT=9000` overrides file setting.
- **Estimated Effort:** 4 hours

---

### Issue #020: Implement Config Validation (`renderd-config/src/validate.rs`)
- **Rationale:** Catches invalid user configurations at startup before initializing network or video subsystems.
- **Dependencies:** #018, #019
- **Acceptance Criteria:**
  - Validates `min_bitrate < max_bitrate`, port range `1024..=65535`, codec values `"hevc" | "h264"`.
  - Returns `ConfigError::ValidationFailed`.
- **Testing:** Unit tests verify invalid configurations fail validation with exact field name.
- **Estimated Effort:** 3 hours

---

### Issue #021: Implement `ConfigError` Hierarchy (`renderd-config/src/error.rs`)
- **Rationale:** Exposes strongly typed errors for configuration parsing and validation failures.
- **Dependencies:** #018
- **Acceptance Criteria:**
  - `ConfigError` implemented using `thiserror` (`ParseFailed`, `IoFailed`, `ValidationFailed`).
- **Testing:** Unit tests verify error string representations formatting.
- **Estimated Effort:** 2 hours

---

### Issue #022: Add Default TOML Config Templates and Tests
- **Rationale:** Provides reference configuration files for packaging and integration testing.
- **Dependencies:** #019
- **Acceptance Criteria:**
  - `templates/host.default.toml` and `templates/viewer.default.toml` created with documented comments.
- **Testing:** Test ensures templates parse and pass validation without errors.
- **Estimated Effort:** 2 hours

---

## Milestone 3: Primitive Layer (Issues #023–#035)

Builds `renderd-frame` (wire header & sliding-window reassembly) and `renderd-crypto` (SPAKE2+ & HKDF).

---

### Issue #023: Scaffold `renderd-frame` Crate & Header Definition
- **Rationale:** Defines the packed 16-byte datagram header for frame fragments per RFC-0002 §12.1.
- **Dependencies:** #015
- **Acceptance Criteria:**
  - `FragmentHeader` struct created (16 bytes packed, little-endian: `frame_id: u64`, `frag_id: u16`, `frag_total: u16`, `flags: u16`, `pts_offset_us: i16`).
- **Testing:** `std::mem::size_of::<FragmentHeader>() == 16` static assertion.
- **Estimated Effort:** 3 hours

---

### Issue #024: Implement `FragmentHeader` Serialization & Parsing
- **Rationale:** Encodes and decodes datagram headers with zero memory allocations.
- **Dependencies:** #023
- **Acceptance Criteria:**
  - `FragmentHeader::to_bytes(&self) -> [u8; 16]` and `FragmentHeader::parse(slice: &[u8]) -> Result<FragmentHeader, FrameError>` implemented.
- **Testing:** Roundtrip property test using `proptest`.
- **Estimated Effort:** 4 hours

---

### Issue #025: Implement `Fragment` and `CompleteFrame` Types
- **Rationale:** Represents individual fragments and reassembled frames in memory.
- **Dependencies:** #024
- **Acceptance Criteria:**
  - `Fragment { header: FragmentHeader, payload: Bytes }` created.
  - `CompleteFrame { frame_id: FrameId, is_keyframe: bool, data: Bytes, pts: Instant }` created.
- **Testing:** Unit tests verify constructor allocations and byte slice slicing.
- **Estimated Effort:** 3 hours

---

### Issue #026: Implement Sliding-Window Reassembly Engine (`renderd-frame/src/window.rs`)
- **Rationale:** Reassembles unordered, unreliable QUIC datagram fragments into complete frames (RFC-0002 §12.2).
- **Dependencies:** #025
- **Acceptance Criteria:**
  - `ReassemblyWindow<const W: usize = 4>` implemented using a HashMap keyed by `FrameId`.
  - `window.insert(fragment)` returns `Option<CompleteFrame>` when all fragments arrive.
  - Fragments older than `(max_seen - W)` are discarded.
- **Testing:** Unit test verifies out-of-order fragment arrival completes frame correctly.
- **Estimated Effort:** 6 hours

---

### Issue #027: Implement Dynamic Fragment Deadline Computer (`renderd-frame/src/deadline.rs`)
- **Rationale:** Computes per-frame fragment deadlines based on decode and render telemetry (§12.3).
- **Dependencies:** #026
- **Acceptance Criteria:**
  - `DeadlineComputer::compute(frame_period, decode_time, render_time) -> Duration` implemented.
  - Output bounded between 8 ms and 14 ms.
- **Testing:** Unit test verifies boundary clamps under extreme decode times.
- **Estimated Effort:** 3 hours

---

### Issue #028: Add `proptest` Property Tests for Reassembly Window
- **Rationale:** Proves reassembly window safety under arbitrary packet reordering and duplication.
- **Dependencies:** #026
- **Acceptance Criteria:**
  - `proptest` suite asserts `window` never emits duplicate `CompleteFrame`s regardless of fragment insertion order.
- **Testing:** Property test executes 1,000 random iterations in CI.
- **Estimated Effort:** 4 hours

---

### Issue #029: Benchmark Reassembly Window Throughput (`renderd-frame/benches/reassembly.rs`)
- **Rationale:** Measures reassembly latency per frame to guard against data-plane regressions.
- **Dependencies:** #026
- **Acceptance Criteria:**
  - Criterion benchmark measures single-fragment and 55-fragment (30 Mbps / 1080p60) insertion rates.
- **Testing:** `cargo bench --package renderd-frame` executes cleanly.
- **Estimated Effort:** 3 hours

---

### Issue #030: Scaffold `renderd-crypto` Crate & Types
- **Rationale:** Establishes domain types for cryptographic key material (`PairToken`, `SessionKey`, `CertKeyMaterial`).
- **Dependencies:** #015
- **Acceptance Criteria:**
  - `PairToken([u8; 32])`, `SessionKey([u8; 32])` created with `Zeroize` on drop.
- **Testing:** Unit test confirms memory zeroization on drop.
- **Estimated Effort:** 3 hours

---

### Issue #031: Implement RFC 9382 SPAKE2+ Test Vectors (`renderd-crypto/src/spake2plus/vectors.rs`)
- **Rationale:** Guarantees SPAKE2+ compliance against official IETF test vectors before implementing protocol logic.
- **Dependencies:** #030
- **Acceptance Criteria:**
  - All test vectors from RFC 9382 §4 embedded as unit test suite `rfc9382_test_vectors()`.
  - Must pass before any pairing code is merged.
- **Testing:** `cargo test -p renderd-crypto rfc9382_test_vectors` passes.
- **Estimated Effort:** 4 hours

---

### Issue #032: Implement SPAKE2+ Prover and Verifier State Machines (`renderd-crypto/src/spake2plus/`)
- **Rationale:** Implements mutual password-authenticated key exchange over P-256 for pairing.
- **Dependencies:** #031
- **Acceptance Criteria:**
  - `Prover` and `Verifier` structs created implementing share generation and MAC verification steps.
- **Testing:** Unit test performs full exchange between Prover and Verifier using matching and non-matching PINs.
- **Estimated Effort:** 8 hours

---

### Issue #033: Implement HKDF Key Derivation Helpers (`renderd-crypto/src/hkdf.rs`)
- **Rationale:** Derives tokens and session keys using canonical fixed-length info strings to prevent length-ambiguity collisions (RFC-0002 §8.2).
- **Dependencies:** #030
- **Acceptance Criteria:**
  - `derive_pair_token(k, host_id, viewer_id)` uses `"renderd-v1-pair:" || host_uuid_canonical || ":" || viewer_uuid_canonical`.
  - `derive_session_key(pair_token, nonce)` implemented.
- **Testing:** Unit tests confirm distinct inputs produce distinct non-colliding keys.
- **Estimated Effort:** 4 hours

---

### Issue #034: Implement Certificate Generator (`renderd-crypto/src/cert.rs`)
- **Rationale:** Generates self-signed TLS certificates for mTLS derived deterministically from the Pair Token.
- **Dependencies:** #033
- **Acceptance Criteria:**
  - `generate_cert(key_material, valid_days) -> (Certificate, PrivateKey)` implemented using `rcgen`.
  - `cert_days_remaining(cert) -> i64` returns remaining validity days.
- **Testing:** Unit test verifies generated certificate parses with `rustls`.
- **Estimated Effort:** 4 hours

---

### Issue #035: Benchmark Crypto Operations (`renderd-crypto/benches/crypto.rs`)
- **Rationale:** Ensures key derivation and certificate handling do not introduce latency spikes during pairing or session start.
- **Dependencies:** #033, #034
- **Acceptance Criteria:**
  - Criterion benchmark for `derive_pair_token` and `generate_cert`.
- **Testing:** `cargo bench --package renderd-crypto` executes.
- **Estimated Effort:** 2 hours

---

## Milestone 4: FFI Layer (Issues #036–#047)

Builds `renderd-vt-sys` (VideoToolbox C shim) and `renderd-sc-sys` (ScreenCaptureKit ObjC bridge).

---

### Issue #036: Scaffold `renderd-vt-sys` Crate & Build Script (macOS)
- **Rationale:** Establishes the FFI crate boundary for Apple's VideoToolbox framework.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `crates/renderd-vt-sys/` created; gated on `target_os = "macos"`.
  - `build.rs` configured to link `VideoToolbox.framework`, `CoreMedia.framework`, `CoreFoundation.framework`.
- **Testing:** Crate builds cleanly on macOS target.
- **Estimated Effort:** 3 hours

---

### Issue #037: Implement VideoToolbox C Bridge Shim (`renderd-vt-sys/c-shims/videotoolbox_shim.c`)
- **Rationale:** Bridges VideoToolbox's C-callback API into a safe function pointer suitable for Rust FFI (RFC-0002 §6.2).
- **Dependencies:** #036
- **Acceptance Criteria:**
  - `renderd_VTCompressionSessionCreate` C function implemented wrapping `VTCompressionSessionCreate`.
  - Passes C callback context `void *ctx` to output callback wrapper.
- **Testing:** `build.rs` compiles C file with `cc::Build` without warnings.
- **Estimated Effort:** 5 hours

---

### Issue #038: Implement `CompressionSession` Safe Wrapper (`renderd-vt-sys/src/session.rs`)
- **Rationale:** Wraps opaque `VTCompressionSessionRef` in a safe Rust struct with `Drop` cleanup.
- **Dependencies:** #037
- **Acceptance Criteria:**
  - `CompressionSession::new(width, height, codec)` initializes hardware H.265 encoder with `RealTime = TRUE`, `AllowFrameReordering = FALSE`, and `MaxKeyFrameIntervalDuration = 0.5s`.
  - Implements `Drop` calling `VTCompressionSessionInvalidate`.
- **Testing:** Integration test creates and drops session without memory leaks.
- **Estimated Effort:** 6 hours

---

### Issue #039: Implement Bitrate and Keyframe Controls in `CompressionSession`
- **Rationale:** Exposes runtime controls to alter bitrate dynamically and force immediate keyframes.
- **Dependencies:** #038
- **Acceptance Criteria:**
  - `session.set_bitrate(kbps)` calls `VTSessionSetProperty` with `kVTCompressionPropertyKey_AverageBitRate`.
  - `session.force_keyframe()` submits frame with `kVTEncodeFrameOptionKey_ForceKeyFrame = TRUE`.
- **Testing:** Unit test asserts API calls return `noErr` (`OSStatus 0`).
- **Estimated Effort:** 4 hours

---

### Issue #040: Implement `IOSurface` Rust Wrapper (`renderd-vt-sys/src/surface.rs`)
- **Rationale:** Wraps macOS `IOSurfaceRef` GPU memory handles safely.
- **Dependencies:** #036
- **Acceptance Criteria:**
  - `IOSurface` struct created wrapping CF type; implements `Clone` and `Drop` (`CFRetain`/`CFRelease`).
- **Testing:** Unit test verifies retain count increments and decrements correctly.
- **Estimated Effort:** 3 hours

---

### Issue #041: Implement `VtError` OSStatus Decoder (`renderd-vt-sys/src/error.rs`)
- **Rationale:** Translates cryptic negative `OSStatus` error codes into human-readable messages.
- **Dependencies:** #036
- **Acceptance Criteria:**
  - `VtError(OSStatus)` implements `std::fmt::Display` mapping common codes (`kVTInvalidSessionErr`, `kVTHardwareAcceleratedVideoEncoderNotAvailableErr`).
- **Testing:** Unit test verifies formatting of known status codes.
- **Estimated Effort:** 2 hours

---

### Issue #042: Scaffold `renderd-sc-sys` Crate (macOS)
- **Rationale:** Establishes FFI crate boundary for Apple's ScreenCaptureKit API.
- **Dependencies:** #001, #023
- **Acceptance Criteria:**
  - `crates/renderd-sc-sys/` created; gated on `target_os = "macos"`.
  - Imports `objc2` and `objc2-screen-capture-kit`.
- **Testing:** Crate builds on macOS target.
- **Estimated Effort:** 3 hours

---

### Issue #043: Implement Screen Recording Permission Checker (`renderd-sc-sys/src/permission.rs`)
- **Rationale:** Checks TCC screen capture authorization status before starting capture stream.
- **Dependencies:** #042
- **Acceptance Criteria:**
  - `ScreenRecordingPermission::check() -> PermissionStatus` queries system TCC status.
- **Testing:** Unit test returns current permission state without crashing.
- **Estimated Effort:** 3 hours

---

### Issue #044: Implement Content Filter Builder (`renderd-sc-sys/src/filter.rs`)
- **Rationale:** Selects main display for capture using `SCContentFilter`.
- **Dependencies:** #042
- **Acceptance Criteria:**
  - `ContentFilter::main_display() -> Result<ContentFilter, ScError>` enumerates displays and selects primary display.
- **Testing:** Integration test retrieves primary display ID on macOS.
- **Estimated Effort:** 4 hours

---

### Issue #045: Implement `ScreenStream` Wrapper (`renderd-sc-sys/src/stream.rs`)
- **Rationale:** Wraps `SCStream` to capture GPU-resident frames via callback on `QOS_CLASS_USER_INTERACTIVE` thread.
- **Dependencies:** #040, #044
- **Acceptance Criteria:**
  - `ScreenStream::new(filter, config, frame_callback)` configures stream.
  - Passes `IOSurface` and `CMSampleBuffer` presentation timestamps to callback.
- **Testing:** Integration test receives at least 5 frames from screen capture stream.
- **Estimated Effort:** 8 hours

---

### Issue #046: Add Vsync Phase Pacing Controls to `ScreenStream`
- **Rationale:** Allows setting `minimumFrameInterval` dynamically for presentation clock synchronization (§7).
- **Dependencies:** #045
- **Acceptance Criteria:**
  - `stream.set_target_interval(duration)` updates `SCStreamConfiguration`.
- **Testing:** Unit test verifies configuration property update.
- **Estimated Effort:** 3 hours

---

### Issue #047: Implement `ScError` Hierarchy (`renderd-sc-sys/src/error.rs`)
- **Rationale:** Provides typed errors for ScreenCaptureKit failure modes.
- **Dependencies:** #042
- **Acceptance Criteria:**
  - `ScError` handles `PermissionDenied`, `NoDisplaysFound`, and `StreamFailed(String)`.
- **Testing:** Unit test verifies error formatting.
- **Estimated Effort:** 2 hours

---

## Milestone 5: Service Layer (Issues #048–#063)

Builds `renderd-net` (QUIC transport), `renderd-keychain` (credential storage), and `renderd-discovery` (mDNS).

---

### Issue #048: Scaffold `renderd-net` Crate & TLS Configuration Builders
- **Rationale:** Configures `rustls` for strict mutual TLS 1.3 authentication using paired certificates.
- **Dependencies:** #014, #034
- **Acceptance Criteria:**
  - `ServerTlsConfig::from_cert()` and `ClientTlsConfig::with_pinned_cert()` implemented.
  - Disables TLS 1.2 and weaker cipher suites.
- **Testing:** Unit test validates TLS config initialization.
- **Estimated Effort:** 4 hours

---

### Issue #049: Implement QUIC Server Wrapper (`renderd-net/src/server.rs`)
- **Rationale:** Encapsulates `quinn::Endpoint` server socket setup and incoming connection listening.
- **Dependencies:** #048
- **Acceptance Criteria:**
  - `QuicServer::bind(addr, tls_config)` starts listening on UDP port.
  - Accepts incoming QUIC connections returning `Connection`.
- **Testing:** Integration test connects client endpoint to server endpoint on loopback.
- **Estimated Effort:** 5 hours

---

### Issue #050: Implement QUIC Client Wrapper (`renderd-net/src/client.rs`)
- **Rationale:** Handles client connection initiation and server certificate verification.
- **Dependencies:** #048
- **Acceptance Criteria:**
  - `QuicClient::connect(addr, server_name, tls_config)` establishes QUIC connection.
- **Testing:** Loopback integration test verifies successful handshake.
- **Estimated Effort:** 4 hours

---

### Issue #051: Implement Control Stream Framing (`renderd-net/src/framing.rs`)
- **Rationale:** Handles 4-byte length-prefixed message serialization on QUIC Stream 0.
- **Dependencies:** #016, #049
- **Acceptance Criteria:**
  - `send_control(msg: &Envelope)` and `recv_control() -> Envelope` implemented.
- **Testing:** Unit test verifies framing and parsing of multiple sequential envelopes over mock stream.
- **Estimated Effort:** 4 hours

---

### Issue #052: Implement Fragment Datagram Burst Sender (`renderd-net/src/burst.rs`)
- **Rationale:** Sends frame fragment datagrams in a non-yielding loop to optimize kernel UDP socket writes (RFC-0002 §12.4).
- **Dependencies:** #024, #049
- **Acceptance Criteria:**
  - `FragmentBurst::send_all(conn, fragments: &[Bytes])` calls `conn.send_datagram()` without yielding Tokio async task between items.
- **Testing:** Benchmark verifies burst sending 55 datagrams completes within < 0.5 ms on loopback.
- **Estimated Effort:** 5 hours

---

### Issue #053: Implement RTT Telemetry Exporter (`renderd-net/src/connection.rs`)
- **Rationale:** Exposes real-time QUIC smoothed RTT for clock synchronization and ABR algorithms.
- **Dependencies:** #049
- **Acceptance Criteria:**
  - `conn.rtt() -> Duration` queries `quinn::ConnectionStats`.
- **Testing:** Unit test reads RTT from active loopback connection.
- **Estimated Effort:** 2 hours

---

### Issue #054: Scaffold `renderd-keychain` Crate & `KeychainStore` Trait
- **Rationale:** Defines platform-agnostic interface for persistent credential storage.
- **Dependencies:** #030
- **Acceptance Criteria:**
  - `KeychainStore` trait defined (`save_pairing`, `load_pairing`, `delete_pairing`, `list_pairings`).
  - `PairingEntry` struct created.
- **Testing:** Unit test compiles trait definition.
- **Estimated Effort:** 3 hours

---

### Issue #055: Implement macOS Keychain Backend (`renderd-keychain/src/macos.rs`)
- **Rationale:** Persists Pair Tokens securely in macOS Keychain Services using `security-framework`.
- **Dependencies:** #054
- **Acceptance Criteria:**
  - `MacosKeychain` implements `KeychainStore` using `kSecClassGenericPassword`.
- **Testing:** Integration test on macOS saves, reads, and deletes test pairing entry.
- **Estimated Effort:** 5 hours

---

### Issue #056: Implement Windows Credential Manager Backend (`renderd-keychain/src/windows.rs`)
- **Rationale:** Persists Pair Tokens securely in Windows Credential Manager using `windows-rs`.
- **Dependencies:** #054
- **Acceptance Criteria:**
  - `WindowsCredentialManager` implements `KeychainStore` using `CredWrite`, `CredRead`, `CredDelete`.
- **Testing:** Integration test on Windows saves, reads, and deletes test pairing entry.
- **Estimated Effort:** 5 hours

---

### Issue #057: Implement Mock Keychain Store for Headless Testing
- **Rationale:** Enables running integration tests on platforms without keychain access (e.g. Linux CI runners).
- **Dependencies:** #054
- **Acceptance Criteria:**
  - `MockKeychain` implements `KeychainStore` using in-memory `HashMap`.
- **Testing:** Unit tests verify save/read/delete operations.
- **Estimated Effort:** 2 hours

---

### Issue #058: Scaffold `renderd-discovery` Crate & Traits
- **Rationale:** Defines platform-agnostic traits for mDNS service advertisement and browsing.
- **Dependencies:** #015
- **Acceptance Criteria:**
  - `Advertiser` and `Browser` traits defined per REPO-0001 §3.
  - `ServiceRecord` struct created containing `host_id`, `name`, `addr`, `port`, `txt`.
- **Testing:** Trait definitions compile cleanly.
- **Estimated Effort:** 3 hours

---

### Issue #059: Implement macOS Bonjour Discovery Backend (`renderd-discovery/src/macos.rs`)
- **Rationale:** Registers `_renderd._udp.local.` via macOS system `dns_sd.h` (Bonjour) to avoid port 5353 conflicts with `mDNSResponder`.
- **Dependencies:** #058
- **Acceptance Criteria:**
  - `BonjourAdvertiser` and `BonjourBrowser` implemented using `dns-sd` crate bindings to `dns_sd.h`.
- **Testing:** Integration test on macOS registers service and browses own advertisement.
- **Estimated Effort:** 6 hours

---

### Issue #060: Implement Windows mDNS Discovery Backend (`renderd-discovery/src/windows.rs`)
- **Rationale:** Registers and browses mDNS services on Windows using Win32 `DnsServiceRegister` and `DnsServiceBrowse`.
- **Dependencies:** #058
- **Acceptance Criteria:**
  - `WinDnsAdvertiser` and `WinDnsBrowser` implemented using `windows-sys`.
- **Testing:** Integration test on Windows discovers local test service.
- **Estimated Effort:** 6 hours

---

### Issue #061: Implement Manual IP Discovery Fallback (`renderd-discovery/src/manual.rs`)
- **Rationale:** Allows connecting directly via IP when mDNS multicast is suppressed by corporate network firewalls.
- **Dependencies:** #058
- **Acceptance Criteria:**
  - `ManualBrowser` emits `DiscoveryEvent::Found` for user-entered IP/port string.
- **Testing:** Unit test verifies static address resolution.
- **Estimated Effort:** 2 hours

---

### Issue #062: Implement `DiscoveryError` Hierarchy (`renderd-discovery/src/error.rs`)
- **Rationale:** Exposes typed errors for discovery registration and resolution failures.
- **Dependencies:** #058
- **Acceptance Criteria:**
  - `DiscoveryError` handles `BindFailed`, `ServiceRegistrationFailed`, `BrowseFailed`.
- **Testing:** Unit test verifies error messages.
- **Estimated Effort:** 2 hours

---

### Issue #063: Create In-Memory Network Mock Transport for Integration Testing
- **Rationale:** Enables testing control and data plane interactions in unit tests without opening OS sockets.
- **Dependencies:** #051
- **Acceptance Criteria:**
  - `MockConnection` simulates `send_control`, `recv_control`, and datagram sends via `tokio::sync::mpsc`.
- **Testing:** Test simulates message exchange between mock host and mock viewer.
- **Estimated Effort:** 4 hours

---

## Milestone 6: Algorithm Layer (Issues #064–#075)

Builds `renderd-abr` (adaptive bitrate) and `renderd-clock` (presentation clock sync).

---

### Issue #064: Scaffold `renderd-abr` Crate & Pure Ramp Policy (`renderd-abr/src/ramp.rs`)
- **Rationale:** Implements deterministic step-up/step-down bitrate math independently from state management.
- **Dependencies:** #018
- **Acceptance Criteria:**
  - `RampPolicy::step_down(current, pct) -> u32` and `step_up(current, pct) -> u32` implemented.
  - Clamped between `min_bitrate_kbps` (5 Mbps) and `max_bitrate_kbps` (50 Mbps).
- **Testing:** Unit tests verify exact percentage calculations and boundary clamps.
- **Estimated Effort:** 3 hours

---

### Issue #065: Implement Exponential Moving Average Bandwidth Estimator (`renderd-abr/src/estimator.rs`)
- **Rationale:** Filters noisy receive bandwidth samples to produce a stable estimate.
- **Dependencies:** #064
- **Acceptance Criteria:**
  - `BandwidthEstimator::update(sample_kbps)` computes EMA.
- **Testing:** Unit test verifies convergence on step-change input sequence.
- **Estimated Effort:** 3 hours

---

### Issue #066: Implement `AbrController` State Machine (`renderd-abr/src/controller.rs`)
- **Rationale:** Implements dual-timescale ABR loop handling 100 ms reactive and 500 ms proactive signals (RFC-0002 §13).
- **Dependencies:** #016, #065
- **Acceptance Criteria:**
  - `on_reactive(&ReactiveStats)` drops bitrate by 25% if loss > 5%, 50% if loss > 20%.
  - `on_proactive(&PeriodicStats)` adjusts bitrate to 80% of estimated bandwidth if degraded.
  - `on_keyframe_request()` triggers immediate keyframe and 25% bitrate reduction.
- **Testing:** Comprehensive unit test suite covers all state transitions.
- **Estimated Effort:** 6 hours

---

### Issue #067: Add Property Tests for `AbrController` State Invariants
- **Rationale:** Proves `AbrController` bitrate decisions never exceed configured bounds under random feedback inputs.
- **Dependencies:** #066
- **Acceptance Criteria:**
  - `proptest` asserts `decision.new_bitrate_kbps` stays strictly within `[min_bitrate, max_bitrate]` for arbitrary inputs.
- **Testing:** Property test executes 1,000 iterations.
- **Estimated Effort:** 3 hours

---

### Issue #068: Benchmark `AbrController` Throughput (`renderd-abr/benches/abr.rs`)
- **Rationale:** Verifies ABR decision calculation execution time is sub-microsecond.
- **Dependencies:** #066
- **Acceptance Criteria:**
  - Criterion benchmark for `on_reactive` and `on_proactive`.
- **Testing:** Benchmark reports latency < 1 µs per decision.
- **Estimated Effort:** 2 hours

---

### Issue #069: Scaffold `renderd-clock` Crate & Clock Offset Calculator (`renderd-clock/src/offset.rs`)
- **Rationale:** Translates viewer-local vsync timestamps into host-local monotonic time domain using QUIC RTT.
- **Dependencies:** #016
- **Acceptance Criteria:**
  - `ClockOffset::compute(viewer_vsync_ns, host_recv_time, rtt) -> Instant` implemented per RFC-0002 §7.2.
- **Testing:** Unit test verifies offset calculation with simulated network delay.
- **Estimated Effort:** 4 hours

---

### Issue #070: Implement Outlier Rejection Filter (`renderd-clock/src/filter.rs`)
- **Rationale:** Filters out OS thread scheduling noise from vsync phase reports using a median window filter.
- **Dependencies:** #069
- **Acceptance Criteria:**
  - `JitterFilter<const N: usize = 5>::filter(sample) -> Duration` returns median of rolling window.
- **Testing:** Unit test verifies single-sample latency spikes are rejected.
- **Estimated Effort:** 3 hours

---

### Issue #071: Implement `ClockSync` State Machine (`renderd-clock/src/sync.rs`)
- **Rationale:** Computes optimal target capture timestamps to align host frame generation with viewer vsync deadlines (§7.2).
- **Dependencies:** #070
- **Acceptance Criteria:**
  - `on_vsync_report(&VsyncReport, recv_time)` updates phase alignment.
  - `next_capture_time(rtt) -> Option<Instant>` returns target capture deadline.
  - `is_synchronized()` returns `false` during initial 30-frame warmup phase.
- **Testing:** Unit tests simulate 60 Hz vsync phase tracking over 100 frames.
- **Estimated Effort:** 6 hours

---

### Issue #072: Add Property Tests for `ClockSync` Pacing Outputs
- **Rationale:** Ensures capture pacing interval never drops below hardware limits or produces negative durations.
- **Dependencies:** #071
- **Acceptance Criteria:**
  - `proptest` asserts target capture interval is bounded within `[8ms, 33ms]` for 60 Hz target.
- **Testing:** Property test executes 1,000 runs.
- **Estimated Effort:** 3 hours

---

### Issue #073: Benchmark `ClockSync` Update Calculation (`renderd-clock/benches/clock.rs`)
- **Rationale:** Confirms clock sync updates on incoming vsync reports introduce negligible overhead.
- **Dependencies:** #071
- **Acceptance Criteria:**
  - Criterion benchmark for `on_vsync_report` and `next_capture_time`.
- **Testing:** Benchmark confirms execution time < 500 ns.
- **Estimated Effort:** 2 hours

---

### Issue #074: Create Latency Benchmark Tool Skeleton (`tools/latency-bench`)
- **Rationale:** Builds internal benchmarking CLI to measure isolated pipeline stage latencies.
- **Dependencies:** #024, #066, #071
- **Acceptance Criteria:**
  - CLI binary scaffolded supporting `--frames`, `--resolution`, `--codec` flags.
- **Testing:** `--help` displays usage documentation.
- **Estimated Effort:** 4 hours

---

### Issue #075: Implement Pipeline Microsecond Stage Telemetry in `tools/latency-bench`
- **Rationale:** Measures capture, encode, send, receive, reassembly, decode, and render stage latencies on loopback.
- **Dependencies:** #074
- **Acceptance Criteria:**
  - Outputs p50, p95, p99, and max latency table in terminal output per REPO-0001 §15.5.
- **Testing:** Execution prints latency report for 100 test frames.
- **Estimated Effort:** 5 hours

---

## Milestone 7: Host Application (`renderd-host`) (Issues #076–#088)

Builds `renderd-host` (macOS Login Item Agent) by composing underlying crates.

---

### Issue #076: Scaffold `renderd-host` Application & `Info.plist`
- **Rationale:** Configures macOS app bundle metadata with `LSUIElement = true` (menu bar agent) and permission usage strings.
- **Dependencies:** #001
- **Acceptance Criteria:**
  - `crates/renderd-host/Info.plist` created with `LSUIElement = true` and `NSScreenCaptureUsageDescription`.
  - `entitlements.plist` created with `com.apple.security.screen-recording` and `app-sandbox`.
- **Testing:** `cargo build -p renderd-host` succeeds.
- **Estimated Effort:** 3 hours

---

### Issue #077: Implement macOS App Bundle Packaging Script (`tools/bundle-host/assemble.sh`)
- **Rationale:** Assembles compiled Rust binary into `renderd-host.app` directory structure required for macOS code signing and notarization.
- **Dependencies:** #076
- **Acceptance Criteria:**
  - Shell script creates `renderd-host.app/Contents/MacOS/` and copies binary and `Info.plist`.
- **Testing:** Output directory matches standard macOS `.app` bundle layout.
- **Estimated Effort:** 3 hours

---

### Issue #078: Implement App Startup & Panic Hook Setup (`renderd-host/src/main.rs`)
- **Rationale:** Initializes tracing logger, parses CLI args, loads config, and sets up panic hook before launching event loop.
- **Dependencies:** #019, #076
- **Acceptance Criteria:**
  - Logging initialized via `tracing-subscriber`.
  - Panic hook logs error details before process aborts.
- **Testing:** Triggering intentional panic prints structured error log.
- **Estimated Effort:** 3 hours

---

### Issue #079: Implement macOS Login Item Auto-Start (`renderd-host/src/autostart.rs`)
- **Rationale:** Registers `renderd-host` to launch automatically at user login using macOS 13+ `SMAppService.mainApp`.
- **Dependencies:** #076
- **Acceptance Criteria:**
  - `AutoStart::enable()` and `disable()` call `SMAppService.mainApp`.
- **Testing:** Integration test queries service status on macOS 13+.
- **Estimated Effort:** 4 hours

---

### Issue #080: Implement Host Session State Machine (`renderd-host/src/session/mod.rs`)
- **Rationale:** Manages host lifecycle transitions (`IDLE` -> `PAIRING` -> `CONNECTED` -> `STREAMING`).
- **Dependencies:** #049, #051
- **Acceptance Criteria:**
  - `HostSession` struct manages current state and connected viewer details.
- **Testing:** Unit tests verify state transition flow.
- **Estimated Effort:** 5 hours

---

### Issue #081: Implement Pairing Handler (`renderd-host/src/session/pairing.rs`)
- **Rationale:** Displays 6-digit PIN in UI and executes SPAKE2+ verifier protocol over QUIC Stream 0.
- **Dependencies:** #032, #055, #080
- **Acceptance Criteria:**
  - Generates 60-second random PIN.
  - On successful SPAKE2+ exchange, saves derived `PairToken` to Keychain and transitions to paired state.
  - Implements exponential lockout after 5 failures.
- **Testing:** Simulation test pairs mock client with host pairing handler.
- **Estimated Effort:** 6 hours

---

### Issue #082: Implement Known-Viewers Registry & Revocation (`renderd-host/src/session/devices.rs`)
- **Rationale:** Allows viewing and revoking paired viewer devices from the host keychain (RFC-0002 §9.3).
- **Dependencies:** #055, #081
- **Acceptance Criteria:**
  - `DeviceRegistry::list()` loads paired devices.
  - `DeviceRegistry::revoke(viewer_id)` deletes certificate from Keychain.
- **Testing:** Unit test verifies revoked viewer certificate is removed.
- **Estimated Effort:** 4 hours

---

### Issue #083: Implement Capture & Encode Dispatch Pipeline (`renderd-host/src/capture.rs`, `encode.rs`)
- **Rationale:** Connects ScreenCaptureKit frames directly to VideoToolbox encoder on `QOS_CLASS_USER_INTERACTIVE` thread.
- **Dependencies:** #038, #045
- **Acceptance Criteria:**
  - `SCStream` callback passes `IOSurface` to `VTCompressionSession` without CPU copies.
  - Output NAL units placed into SPSC lock-free ring buffer (capacity 4).
- **Testing:** Pipeline encodes 100 screen capture frames without frame drops.
- **Estimated Effort:** 8 hours

---

### Issue #084: Implement Datagram Burst Sender Task (`renderd-host/src/network/data.rs`)
- **Rationale:** Pulls encoded NAL units from ring buffer, fragments them, and sends datagram bursts over QUIC.
- **Dependencies:** #024, #052, #083
- **Acceptance Criteria:**
  - Fragments frame into datagrams with 16-byte headers.
  - Sends all fragments in a single non-yielding burst per frame.
- **Testing:** Integration test captures, encodes, and transmits stream over loopback QUIC server.
- **Estimated Effort:** 6 hours

---

### Issue #085: Connect Clock Sync & ABR Controllers to Host Control Loop (`renderd-host/src/abr.rs`, `clock.rs`)
- **Rationale:** Updates capture pacing from `VsyncReport` messages and updates encode bitrate from `ReactiveStats`/`PeriodicStats`.
- **Dependencies:** #039, #066, #071, #084
- **Acceptance Criteria:**
  - `VsyncReport` updates `ClockSync` capture interval.
  - `ReactiveStats` triggers `session.set_bitrate()`.
- **Testing:** Simulation test sends `ReactiveStats` with 10% loss and asserts `VTCompressionSession` bitrate is reduced.
- **Estimated Effort:** 6 hours

---

### Issue #086: Implement macOS Menu Bar User Interface (`renderd-host/src/ui/menubar.rs`)
- **Rationale:** Provides native macOS status bar menu for pairing PIN display, paired device list, and quit.
- **Dependencies:** #082
- **Acceptance Criteria:**
  - Menu bar icon created using `tray-icon` / AppKit.
  - Menu options: "Status", "Pair New Device (PIN)", "Paired Devices...", "Quit".
- **Testing:** Manual test confirms status bar icon renders and responds to clicks.
- **Estimated Effort:** 6 hours

---

### Issue #087: Implement User Notifications Integration (`renderd-host/src/ui/notifications.rs`)
- **Rationale:** Alerts user via macOS `UserNotifications` when a streaming session starts or a new device pairs (RFC-0002 §9.3).
- **Dependencies:** #080
- **Acceptance Criteria:**
  - Emits system notification: `"[Viewer Name] started screen sharing"`.
- **Testing:** Integration test posts notification on test session start.
- **Estimated Effort:** 3 hours

---

### Issue #088: Setup Host Release Packaging Workflow (`.github/workflows/release-host.yml`)
- **Rationale:** Automates code signing, Apple notarization, stapling, and DMG packaging on release tag push (RFC-0002 §17).
- **Dependencies:** #077
- **Acceptance Criteria:**
  - CI job signs bundle using Developer ID certificate, submits to Apple `notarytool`, staples ticket, and creates DMG artifact.
- **Testing:** Trigger dry-run release workflow in repository CI.
- **Estimated Effort:** 6 hours

---

## Milestone 8: Viewer Application (`renderd-viewer`) (Issues #089–#100)

Builds `renderd-viewer` (Windows 11 Client) by composing underlying crates.

---

### Issue #089: Scaffold `renderd-viewer` Application & Window Manager (`winit`)
- **Rationale:** Initializes Windows 11 application and handles window creation, borderless fullscreen, and message loop using `winit` (RFC-0002 §6.3).
- **Dependencies:** #001
- **Acceptance Criteria:**
  - Borderless fullscreen window created on Windows target using `winit`.
  - Handles DPI scaling (`WM_DPICHANGED`) and window close events.
- **Testing:** Window opens in borderless fullscreen and closes cleanly on Escape key.
- **Estimated Effort:** 5 hours

---

### Issue #090: Implement DXGI Allow-Tearing Feature Check (`renderd-viewer/src/render/tearing_check.rs`)
- **Rationale:** Queries system DXGI capabilities before creating swap chain to prevent `DXGI_ERROR_INVALID_CALL` crashes (§6.3).
- **Dependencies:** #089
- **Acceptance Criteria:**
  - `check_tearing_support() -> bool` calls `IDXGIFactory5::CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING)`.
- **Testing:** Unit test executes query on Windows test machine.
- **Estimated Effort:** 3 hours

---

### Issue #091: Implement D3D12 Swap Chain & Renderer (`renderd-viewer/src/render/d3d12_renderer.rs`)
- **Rationale:** Manages Direct3D 12 swap chain and executes YUV-to-RGB pixel shader rendering pass.
- **Dependencies:** #090
- **Acceptance Criteria:**
  - D3D12 swap chain created (with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING` if supported).
  - Compiles `shaders/yuv_to_rgb.hlsl` and executes render pass.
  - `Present1()` uses `DXGI_PRESENT_ALLOW_TEARING` conditionally.
- **Testing:** Renderer renders synthetic NV12 test pattern to window surface.
- **Estimated Effort:** 8 hours

---

### Issue #092: Implement D3D12 Video Decoder Integration (`renderd-viewer/src/decode/d3d12_decode.rs`)
- **Rationale:** Decodes incoming H.265/H.264 video frame buffers into NV12/P010 GPU surfaces using `ID3D12VideoDecoder`.
- **Dependencies:** #091
- **Acceptance Criteria:**
  - Hardware decoder initialized via `windows-rs`.
  - Output decoded surfaces remain in GPU VRAM for direct rendering input.
- **Testing:** Decodes H.265 test bitstream file into GPU surface without errors.
- **Estimated Effort:** 8 hours

---

### Issue #093: Implement Datagram Receiver & Sliding-Window Reassembly Task (`renderd-viewer/src/network/data.rs`)
- **Rationale:** Receives QUIC datagrams from network, feeds `ReassemblyWindow`, and forwards completed frames to decoder.
- **Dependencies:** #026, #050, #092
- **Acceptance Criteria:**
  - Receives datagrams, parses 16-byte header, inserts into `ReassemblyWindow`.
  - On complete frame, hands frame payload to D3D12 video decoder.
- **Testing:** Integration test streams test fragments over loopback and verifies video playback.
- **Estimated Effort:** 6 hours

---

### Issue #094: Implement DWM Vsync Phase Reporter (`renderd-viewer/src/clock_sync/vsync_reporter.rs`)
- **Rationale:** Captures Windows DWM vsync phase timestamps and transmits `VsyncReport` to host on Stream 0 (RFC-0002 §7.2).
- **Dependencies:** #051, #089
- **Acceptance Criteria:**
  - Queries `DwmGetCompositionTimingInfo` every frame.
  - Transmits `VsyncReport` protobuf message over QUIC Stream 0.
- **Testing:** Integration test logs vsync period (~16.66 ms) and phase timestamps.
- **Estimated Effort:** 4 hours

---

### Issue #095: Implement Dual-Timescale Feedback Exporter (`renderd-viewer/src/abr/feedback.rs`)
- **Rationale:** Computes loss rate and decode timing telemetry and transmits `ReactiveStats` (100 ms) and `PeriodicStats` (500 ms).
- **Dependencies:** #051, #093
- **Acceptance Criteria:**
  - Sends `ReactiveStats` every 100 ms.
  - Sends `PeriodicStats` every 500 ms containing mean decode and render times.
  - Sends immediate `KeyframeRequest` on frame loss detection.
- **Testing:** Unit test validates transmission schedules and contents.
- **Estimated Effort:** 5 hours

---

### Issue #096: Implement Viewer Pairing UI & SPAKE2+ Prover Handshake (`renderd-viewer/src/pairing/`)
- **Rationale:** Prompts user for 6-digit PIN and executes SPAKE2+ prover handshake over QUIC Stream 0.
- **Dependencies:** #032, #056, #050
- **Acceptance Criteria:**
  - PIN entry dialog rendered in UI.
  - Executes SPAKE2+ prover protocol; on success stores `PairToken` in Windows Credential Manager.
- **Testing:** End-to-end pairing test against `renderd-host` pairing handler over loopback.
- **Estimated Effort:** 6 hours

---

### Issue #097: Implement Reconnect Watchdog with mDNS Re-Discovery (`renderd-viewer/src/reconnect/watchdog.rs`)
- **Rationale:** Re-discovers host IP via mDNS by UUID when connection drops due to host DHCP renewal (RFC-0002 §18.1).
- **Dependencies:** #050, #060
- **Acceptance Criteria:**
  - On disconnect, attempts cached IP once; on failure initiates mDNS browse filtered by stored `host_uuid`.
  - Reconnects to newly discovered IP address automatically.
- **Testing:** Integration test changes host IP during active connection and verifies automatic recovery.
- **Estimated Effort:** 6 hours

---

### Issue #098: Implement Reconnecting & Status UI Overlay (`renderd-viewer/src/ui/overlay.rs`)
- **Rationale:** Displays semi-transparent "Reconnecting..." overlay over last displayed video frame without closing window.
- **Dependencies:** #091, #097
- **Acceptance Criteria:**
  - Renders semi-transparent status message overlay during `RECONNECTING` state.
  - Preserves window bounds and position.
- **Testing:** Visual verification during simulated network disconnect.
- **Estimated Effort:** 4 hours

---

### Issue #099: Implement Windows Tray Icon & Settings Menu (`renderd-viewer/src/ui/settings.rs`)
- **Rationale:** Provides system tray icon on Windows for configuration access, manual IP entry, and exit.
- **Dependencies:** #061, #089
- **Acceptance Criteria:**
  - System tray icon created via Win32 `Shell_NotifyIcon`.
  - Context menu options: "Connect to Host...", "Settings", "Disconnect", "Exit".
- **Testing:** Manual test confirms tray icon functions on Windows 11.
- **Estimated Effort:** 5 hours

---

### Issue #100: Setup Windows Viewer Release Packaging Workflow (`.github/workflows/release-viewer.yml`)
- **Rationale:** Automates Windows `.exe` binary compilation and installer packaging on release tag push.
- **Dependencies:** #089
- **Acceptance Criteria:**
  - GitHub Actions workflow compiles release binary on `windows-2025`.
  - Packages standalone executable installer using PowerShell packaging script (`tools/package-viewer/package.ps1`).
  - Attaches installer artifact to GitHub Release.
- **Testing:** Trigger release workflow dry run on Windows runner.
- **Estimated Effort:** 4 hours

---

*End of ISSUES-0001*
