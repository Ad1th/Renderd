# REPO-0001: Renderd Repository Design

```
Title:   Renderd — Repository and Engineering Standards
Doc:     REPO-0001
Status:  Draft
Applies: All crates in the renderd workspace
Created: 2026-08-03
Refs:    RFC-0002 (canonical architecture specification)
```

---

## Abstract

This document specifies the complete repository design for Renderd, including workspace
layout, crate boundaries, dependency rules, module hierarchy, coding standards, tooling
configuration, testing strategy, release process, and contribution guidelines. It is
intended to enable multiple engineers to begin independent work on separate crates
without coordination hazards.

The repository follows the standards established by production-quality open-source Rust
projects: Tokio, Zed, Helix, ripgrep, and Servo. It prioritizes long-term maintainability,
clear ownership, and mechanical enforcement of quality over development velocity.

---

## Table of Contents

1. [Design Philosophy](#1-design-philosophy)
2. [Workspace Layout](#2-workspace-layout)
3. [Crate Responsibilities](#3-crate-responsibilities)
4. [Dependency Graph](#4-dependency-graph)
5. [Module Hierarchy](#5-module-hierarchy)
6. [Workspace Cargo Configuration](#6-workspace-cargo-configuration)
7. [Ownership and Review Rules](#7-ownership-and-review-rules)
8. [Coding Standards](#8-coding-standards)
9. [Lint Configuration](#9-lint-configuration)
10. [Formatting](#10-formatting)
11. [Logging and Telemetry](#11-logging-and-telemetry)
12. [Configuration System](#12-configuration-system)
13. [Error Handling](#13-error-handling)
14. [Testing Strategy](#14-testing-strategy)
15. [Benchmarking](#15-benchmarking)
16. [Documentation Standards](#16-documentation-standards)
17. [Release Process](#17-release-process)
18. [GitHub Actions](#18-github-actions)
19. [Branch Strategy](#19-branch-strategy)
20. [Commit Conventions](#20-commit-conventions)
21. [Contribution Guide](#21-contribution-guide)

---

## 1. Design Philosophy

### 1.1 Core Principles

**1. Every crate has exactly one responsibility.** A crate that does two things should
be two crates. Crates are the unit of ownership, the unit of testing, and the unit of
review. Monolithic crates produce monolithic PRs and monolithic blame.

**2. Dependencies flow in one direction only.** The dependency graph is a DAG. No
layer may import from a layer above it. Circular dependencies are impossible to introduce
because the workspace enforces this at compile time through the crate boundary design.

**3. Libraries make no decisions; binaries make all decisions.** Library crates expose
types, traits, and algorithms. They do not read environment variables, open files, spawn
threads, or initialize runtimes. Those are binary responsibilities. This keeps library
crates testable in isolation.

**4. Errors are typed at library boundaries; erased at binary boundaries.** Every public
function in a library crate returns `Result<T, E>` where `E` is a specific, documented
error type. Binaries convert these to user-facing messages using `anyhow`. Error strings
never cross crate boundaries.

**5. The latency path is sacred.** Any change that touches the capture→encode→send or
receive→decode→present pipeline must include a benchmark regression test. No PR that
increases p99 latency by more than 5% on the benchmark suite may be merged.

**6. Platform code is isolated.** Platform-specific implementations (macOS, Windows) live
in dedicated modules behind trait abstractions. Cross-platform logic never contains
`#[cfg(target_os)]` guards. The trait implementation module contains all platform guards.

**7. The public API surface is minimal.** Everything is `pub(crate)` by default. A
function, type, or module becomes `pub` only when there is a documented, tested consumer
in another crate. Unexported implementation details cannot become API surface accidentally.

### 1.2 Non-Negotiables

- No `unsafe` code outside of designated FFI boundary crates (`renderd-vt-sys`,
  `renderd-sc-sys`). All `unsafe` in those crates requires a safety comment per block.
- No `unwrap()` or `expect()` in library code. Use `?` or explicit error handling.
  `expect()` is permitted in tests with a descriptive message. `unwrap()` is never permitted.
- No `std::sync::Mutex` in the hot path (capture→network thread). Use lock-free
  primitives from `crossbeam` or `atomic` types.
- No runtime reflection, dynamic dispatch, or `dyn Trait` in the data plane. The
  decode/render pipeline is monomorphized at compile time.
- No silent panics. `panic::set_hook` is installed by both binaries to log panics
  through the tracing subscriber before aborting.

---

## 2. Workspace Layout

```
renderd/
│
├── Cargo.toml                        # Workspace root — no [package] section
├── Cargo.lock                        # Committed; reproducible builds
├── rust-toolchain.toml               # Pinned nightly/stable channel
├── .rustfmt.toml                     # Workspace-wide formatting
├── clippy.toml                       # Workspace-wide lint overrides
├── deny.toml                         # cargo-deny: licenses, advisories, duplicates
├── nextest.toml                      # cargo-nextest configuration
│
├── crates/
│   │
│   │   ── Foundation Layer (no internal dependencies) ──────────────────────
│   ├── renderd-proto/                # Protobuf types (generated + handwritten wrappers)
│   ├── renderd-config/               # Configuration schema and loading
│   │
│   │   ── Primitive Layer (deps: proto, config) ──────────────────────────
│   ├── renderd-frame/                # Frame/fragment types; sliding-window reassembly
│   ├── renderd-crypto/               # SPAKE2+ (RFC 9382), HKDF, cert generation
│   │
│   │   ── FFI Layer (deps: none internal; wraps C/ObjC APIs) ────────────
│   ├── renderd-vt-sys/               # VideoToolbox C FFI (macOS only; unsafe)
│   ├── renderd-sc-sys/               # ScreenCaptureKit ObjC bridge (macOS only; unsafe)
│   │
│   │   ── Service Layer (deps: proto, frame, crypto, config) ────────────
│   ├── renderd-net/                  # QUIC connection, stream, datagram abstractions
│   ├── renderd-keychain/             # Keychain abstraction + platform implementations
│   ├── renderd-discovery/            # mDNS discovery abstraction + platform implementations
│   │
│   │   ── Algorithm Layer (deps: proto, frame, net) ──────────────────────
│   ├── renderd-abr/                  # Adaptive bitrate controller (dual-timescale)
│   ├── renderd-clock/                # Presentation clock synchronization (§7, RFC-0002)
│   │
│   │   ── Application Layer (deps: everything relevant) ──────────────────
│   ├── renderd-host/                 # macOS host binary (Login Item Agent)
│   └── renderd-viewer/               # Windows viewer binary
│
├── tools/
│   ├── latency-bench/                # Standalone latency benchmark binary
│   ├── proto-gen/                    # Script: re-generates renderd-proto from .proto files
│   └── bundle-host/                  # Script: assembles and signs macOS .app bundle
│
├── proto/
│   └── renderd.proto                 # Source of truth for all control plane messages
│
├── shaders/
│   └── yuv_to_rgb.hlsl               # HLSL YUV→RGB shader (compiled in renderd-viewer build)
│
├── docs/
│   ├── RFC-0001-architecture.md      # Superseded
│   ├── RFC-0001-review.md            # Architectural review
│   ├── RFC-0002-architecture.md      # Canonical architecture spec
│   └── REPO-0001-repository.md       # This document
│
├── .github/
│   ├── CODEOWNERS
│   ├── PULL_REQUEST_TEMPLATE.md
│   ├── ISSUE_TEMPLATE/
│   │   ├── bug_report.yml
│   │   ├── latency_regression.yml
│   │   └── feature_request.yml
│   └── workflows/
│       ├── ci.yml                    # Build, test, lint (all platforms)
│       ├── bench.yml                 # Criterion benchmark runner
│       ├── security.yml              # cargo-deny + cargo-audit
│       ├── docs.yml                  # rustdoc generation + deploy
│       ├── release-host.yml          # macOS .app build, sign, notarize
│       ├── release-viewer.yml        # Windows .exe build + installer
│       ├── proto-check.yml           # Verify generated proto code is up to date
│       └── typos.yml                 # Spell checking (typos-cli)
│
├── LICENSE                           # MIT
├── README.md
├── CHANGELOG.md                      # Keep-a-Changelog format
├── CONTRIBUTING.md                   # Symlink to docs/CONTRIBUTING.md
└── SECURITY.md                       # Vulnerability disclosure policy
```

---

## 3. Crate Responsibilities

Each crate has exactly one sentence defining its responsibility. Anything not covered by
that sentence belongs in a different crate.

### Foundation Layer

---

#### `renderd-proto`
**Responsibility:** Owns all protobuf message types exchanged on the QUIC control plane,
plus newtype wrappers that add semantic meaning to raw proto fields.

**What it contains:**
- `build.rs` that runs `prost-build` against `proto/renderd.proto`
- `src/generated/` — generated prost code (committed; proto-gen tool regenerates it)
- `src/envelope.rs` — `Envelope` oneof dispatch and `MessageKind` enum
- `src/types.rs` — Newtypes: `FrameId(u64)`, `FragmentId(u16)`, `BitrateKbps(u32)`,
  `VsyncPeriodNs(u64)`, `ViewerId(uuid::Uuid)`, `HostId(uuid::Uuid)`
- `src/validate.rs` — Field validation (`SessionHello::validate()` returns `Result`)

**What it must not contain:** Network I/O, serialization formats other than protobuf,
platform code, or business logic.

**External dependencies:** `prost`, `uuid`

---

#### `renderd-config`
**Responsibility:** Defines the complete configuration schema for both host and viewer,
and provides validated loading from TOML files and environment variable overrides.

**What it contains:**
- `src/host.rs` — `HostConfig` struct (all host settings with defaults)
- `src/viewer.rs` — `ViewerConfig` struct (all viewer settings with defaults)
- `src/common.rs` — Shared sub-configs: `NetworkConfig`, `AbrConfig`, `LogConfig`
- `src/load.rs` — `Config::load(path: Option<&Path>) -> Result<Config, ConfigError>`
- `src/error.rs` — `ConfigError` enum

**Schema example (host):**
```
[network]
port = 7373                    # QUIC listen port
max_bitrate_kbps = 50_000      # Maximum encode bitrate
min_bitrate_kbps = 5_000

[encode]
codec = "hevc"                 # "hevc" | "h264"
keyframe_interval_ms = 500
prioritize_speed = true        # kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality

[abr]
reactive_interval_ms = 100
proactive_interval_ms = 500
ramp_up_pct = 10
ramp_down_pct = 25

[log]
level = "info"                 # trace|debug|info|warn|error
format = "text"                # "text" | "json"
```

**What it must not contain:** File watching, network I/O, or platform-specific code
(path resolution for the config file location is a binary concern).

**External dependencies:** `serde`, `serde_derive`, `toml`, `figment` (layered config)

---

### Primitive Layer

---

#### `renderd-frame`
**Responsibility:** Defines the wire format for video frame fragments and implements the
sliding-window fragment reassembly state machine.

**What it contains:**
- `src/header.rs` — `FragmentHeader` (16-byte packed struct, little-endian)
- `src/fragment.rs` — `Fragment { header: FragmentHeader, payload: Bytes }` and
  `Fragment::parse(datagram: &[u8]) -> Result<Fragment, FrameError>`
- `src/frame.rs` — `CompleteFrame { frame_id: FrameId, is_keyframe: bool, data: Bytes }`
- `src/window.rs` — `ReassemblyWindow` — the sliding-window state machine:
  - `fn insert(&mut self, frag: Fragment) -> Option<CompleteFrame>`
  - `fn advance_deadline(&mut self, now: Instant) -> Vec<PartialFrameTimeout>`
  - Window depth `W = 4`; configurable via const generic
- `src/deadline.rs` — `DeadlineComputer` — computes dynamic fragment deadlines from
  decode and render time telemetry (§12.3, RFC-0002)
- `src/error.rs` — `FrameError` enum

**What it must not contain:** Network I/O, encoding/decoding logic, or platform code.

**External dependencies:** `bytes`, `renderd-proto`

---

#### `renderd-crypto`
**Responsibility:** Implements all cryptographic operations required by Renderd: SPAKE2+
pairing ceremony (RFC 9382), HKDF key derivation with canonical info strings, and
self-signed TLS certificate generation seeded from key material.

**What it contains:**
- `src/spake2plus/` — RFC 9382 SPAKE2+ implementation:
  - `mod.rs` — `Prover` and `Verifier` state machines
  - `vectors.rs` — RFC 9382 §4 test vectors (used in `#[test]`)
  - `primitives.rs` — P-256 curve operations, MAC construction
- `src/hkdf.rs` — `derive_pair_token(k, host_id, viewer_id) -> PairToken`,
  `derive_session_key(pair_token, nonce) -> SessionKey`,
  `derive_cert_key(pair_token) -> CertKeyMaterial` — all with canonical info strings
- `src/cert.rs` — `generate_cert(key_material, valid_days: u32) -> (Certificate, PrivateKey)`,
  `cert_days_remaining(cert: &Certificate) -> i64`
- `src/types.rs` — `PairToken([u8; 32])`, `SessionKey([u8; 32])`, `CertKeyMaterial([u8; 64])`
- `src/error.rs` — `CryptoError` enum

**Critical requirement:** The SPAKE2+ implementation must pass all test vectors from
RFC 9382 §4. The test suite must run these vectors as `#[test]` cases before any other
test. If the implementation does not produce matching output for the RFC vectors, it must
not be merged.

**What it must not contain:** Network I/O, keychain operations, or configuration loading.

**External dependencies:** `p256`, `hmac`, `sha2`, `hkdf`, `rcgen`, `rustls`, `zeroize`

---

### FFI Layer

The FFI layer contains `unsafe` code. These crates exist solely to provide safe (or
deliberately unsafe-marked) Rust bindings to platform C/ObjC APIs. They are platform-
gated: `renderd-vt-sys` compiles only on `target_os = "macos"`, `renderd-sc-sys`
compiles only on `target_os = "macos"`.

---

#### `renderd-vt-sys`
**Responsibility:** Provides raw Rust FFI bindings to the VideoToolbox C API and a thin
safe wrapper around `VTCompressionSession` lifecycle and callbacks.

**What it contains:**
- `c-shims/videotoolbox_shim.c` — C bridge for callback-heavy VT APIs:
  ```c
  // Converts VTCompressionOutputHandler callback to a C function pointer
  // so that Rust can register a static callback without closure capture issues.
  typedef void (*RenderD_VTOutputCallback)(
      void *ctx, OSStatus status, VTEncodeInfoFlags flags,
      CMSampleBufferRef sample_buffer
  );
  OSStatus renderd_VTCompressionSessionCreate(
      CFAllocatorRef allocator, int32_t width, int32_t height,
      CMVideoCodecType codec, CFDictionaryRef encoder_spec,
      CFDictionaryRef source_attrs, CFAllocatorRef compressed_data_allocator,
      RenderD_VTOutputCallback callback, void *callback_ctx,
      VTCompressionSessionRef *session_out
  );
  ```
- `build.rs` — `cc::Build` compilation of the C shim; links VideoToolbox.framework
- `src/bindings.rs` — `bindgen`-generated or hand-written FFI declarations
- `src/session.rs` — `CompressionSession` safe wrapper:
  - `CompressionSession::new(config: &EncodeConfig) -> Result<Self, VtError>`
  - `CompressionSession::encode_frame(surface: IOSurface, pts: CMTime, ...) -> Result`
  - `fn set_bitrate(&self, kbps: u32) -> Result`
  - `fn force_keyframe(&self) -> Result`
  - Implements `Drop` (calls `VTCompressionSessionInvalidate`)
- `src/surface.rs` — `IOSurface` wrapper with `Drop`
- `src/error.rs` — `VtError(OSStatus)` with human-readable messages for common codes

**`unsafe` policy:** Every `unsafe` block must have a `// SAFETY:` comment explaining
why the operation is sound. Blocks must be as narrow as possible — wrap the single unsafe
call, not an entire function.

**External dependencies:** `core-foundation`, `core-media`, `cc` (build-dep), `bindgen` (build-dep)

---

#### `renderd-sc-sys`
**Responsibility:** Provides safe Rust bindings to ScreenCaptureKit via `objc2`, plus a
thin abstraction over `SCStream` that delivers `IOSurface`-backed frames to a Rust
callback.

**What it contains:**
- `src/stream.rs` — `ScreenStream` abstraction:
  - `ScreenStream::new(config: &CaptureConfig, callback: impl Fn(CaptureFrame) + Send + 'static) -> Result<Self, ScError>`
  - `ScreenStream::start() -> Result`
  - `ScreenStream::stop() -> Result`
  - Configures SCStream with correct `minimumFrameInterval` for phase sync
- `src/frame.rs` — `CaptureFrame { surface: IOSurface, pts: CMTime, vsync_time: CMTime }`
- `src/filter.rs` — `ContentFilter` builder wrapping `SCContentFilter`
- `src/error.rs` — `ScError` enum (covers permission denied, no displays, stream error)
- `src/permission.rs` — `ScreenRecordingPermission::check() -> PermissionStatus`,
  `ScreenRecordingPermission::request_and_wait() -> Result`

**External dependencies:** `objc2`, `objc2-foundation`, `objc2-screen-capture-kit`,
`core-foundation`, `core-media`

---

### Service Layer

---

#### `renderd-net`
**Responsibility:** Owns the QUIC connection lifecycle and exposes typed, async send/receive
APIs for Stream 0 (control) and datagrams (data), abstracting `quinn` internals from all
other crates.

**What it contains:**
- `src/connection.rs` — `Connection` wrapper over `quinn::Connection`:
  - `Connection::connect(addr, tls_config) -> Result<Connection, NetError>`
  - `Connection::accept(incoming, tls_config) -> Result<Connection, NetError>`
  - `fn send_control(&self, msg: &Envelope) -> Result`
  - `fn recv_control(&self) -> impl Future<Output = Result<Envelope, NetError>>`
  - `fn send_fragment(&self, frag: &[u8]) -> Result`
  - `fn recv_fragment(&self) -> impl Future<Output = Result<Bytes, NetError>>`
  - `fn rtt(&self) -> Duration` — measured QUIC RTT
  - `fn close(&self, code: VarInt, reason: &[u8])`
- `src/tls.rs` — TLS configuration builders:
  - `ServerTlsConfig::from_cert(cert, key) -> ServerConfig`
  - `ClientTlsConfig::with_pinned_cert(server_cert) -> ClientConfig`
  Both use rustls with TLS 1.3 only; no downgrade.
- `src/framing.rs` — Length-prefix framing for Stream 0:
  `frame_message(msg: &Envelope) -> Bytes`,
  `parse_message(buf: &mut BytesMut) -> Option<Result<Envelope, NetError>>`
- `src/burst.rs` — `FragmentBurst::send_all(conn, frags: &[Bytes]) -> Result`
  Implements synchronous-loop burst send without Tokio yield points between fragments.
- `src/error.rs` — `NetError` enum

**External dependencies:** `quinn`, `rustls`, `bytes`, `tokio`, `renderd-proto`

---

#### `renderd-keychain`
**Responsibility:** Defines a platform-agnostic trait for storing and retrieving Pair
Tokens and certificate material, with concrete implementations for macOS Keychain and
Windows Credential Manager.

**What it contains:**
- `src/store.rs` — `KeychainStore` trait:
  ```rust
  pub trait KeychainStore: Send + Sync {
      fn save_pairing(&self, entry: &PairingEntry) -> Result<(), KeychainError>;
      fn load_pairing(&self, host_id: &HostId) -> Result<Option<PairingEntry>, KeychainError>;
      fn delete_pairing(&self, host_id: &HostId) -> Result<(), KeychainError>;
      fn list_pairings(&self) -> Result<Vec<PairingEntry>, KeychainError>;
  }
  ```
- `src/entry.rs` — `PairingEntry { host_id, viewer_id, pair_token, host_cert, viewer_cert, paired_at, cert_expires_at }`
- `src/macos.rs` — `MacosKeychain` implementing `KeychainStore` via `security-framework`
  (`cfg(target_os = "macos")`)
- `src/windows.rs` — `WindowsCredentialManager` implementing `KeychainStore` via
  `windows-rs` CredWrite/CredRead/CredDelete (`cfg(target_os = "windows")`)
- `src/error.rs` — `KeychainError` enum
- `src/lib.rs` — `fn platform_store() -> Box<dyn KeychainStore>` — returns the
  correct platform implementation at runtime

**External dependencies:** `security-framework` (macOS), `windows-sys` (Windows),
  `zeroize` (for PairToken zeroing on drop), `renderd-crypto`, `renderd-proto`

---

#### `renderd-discovery`
**Responsibility:** Defines a platform-agnostic trait for mDNS service advertisement and
browsing, with implementations for macOS (Bonjour/dns_sd.h) and Windows (DnsServiceBrowse).

**What it contains:**
- `src/advertise.rs` — `Advertiser` trait:
  ```rust
  pub trait Advertiser: Send {
      fn start(&self, record: &ServiceRecord) -> Result<(), DiscoveryError>;
      fn stop(&self);
      fn update_txt(&self, kv: &[(&str, &str)]) -> Result<(), DiscoveryError>;
  }
  ```
- `src/browse.rs` — `Browser` trait:
  ```rust
  pub trait Browser: Send {
      fn start(&self, tx: mpsc::Sender<DiscoveryEvent>) -> Result<(), DiscoveryError>;
      fn stop(&self);
  }
  pub enum DiscoveryEvent {
      Found(ServiceRecord),
      Lost(HostId),
      Resolved { host_id: HostId, addr: SocketAddr },
  }
  ```
- `src/record.rs` — `ServiceRecord { host_id, name, addr, port, txt: HashMap<String,String> }`
- `src/macos.rs` — `BonjourAdvertiser` / `BonjourBrowser` via `dns_sd.h` bindings
  (`cfg(target_os = "macos")`)
- `src/windows.rs` — `WinDnsAdvertiser` / `WinDnsBrowser` via `DnsServiceRegister` /
  `DnsServiceBrowse` (`cfg(target_os = "windows")`)
- `src/error.rs` — `DiscoveryError` enum
- `src/lib.rs` — `fn platform_advertiser() -> Box<dyn Advertiser>`,
  `fn platform_browser() -> Box<dyn Browser>`

**External dependencies:** `dns-sd` crate (for macOS Bonjour C bindings), `windows-sys`
  (Windows), `tokio`, `renderd-proto`

---

### Algorithm Layer

---

#### `renderd-abr`
**Responsibility:** Implements the dual-timescale adaptive bitrate controller that adjusts
encode bitrate based on reactive (100 ms) and proactive (500 ms) feedback signals.

**What it contains:**
- `src/controller.rs` — `AbrController`:
  ```rust
  pub struct AbrController { /* internal state */ }
  impl AbrController {
      pub fn new(config: AbrConfig) -> Self;
      // Called by the control plane on ReactiveStats receipt
      pub fn on_reactive(&mut self, stats: &ReactiveStats) -> AbrDecision;
      // Called by the control plane on PeriodicStats receipt
      pub fn on_proactive(&mut self, stats: &PeriodicStats) -> AbrDecision;
      // Called immediately on KeyframeRequest receipt
      pub fn on_keyframe_request(&mut self, hint_kbps: Option<u32>) -> AbrDecision;
      pub fn current_bitrate_kbps(&self) -> u32;
  }
  pub struct AbrDecision {
      pub new_bitrate_kbps: u32,
      pub force_keyframe: bool,
      pub reduce_framerate: bool,
  }
  ```
- `src/estimator.rs` — `BandwidthEstimator` — exponential moving average of receive
  bandwidth; produces estimate with confidence interval
- `src/ramp.rs` — `RampPolicy` — pure function implementing the ramp-up/ramp-down rules
  from RFC-0002 §13; separately testable
- `src/error.rs` — `AbrError` (currently empty; reserved for future extension)

**Design note:** `AbrController` is a pure state machine. It does not spawn tasks, hold
timers, or perform I/O. The caller (host control plane) drives it by calling the three
`on_*` methods on the correct thread. All internal state transitions are deterministic
given the same input sequence — this enables property-based testing.

**External dependencies:** `renderd-proto`, `renderd-config`

---

#### `renderd-clock`
**Responsibility:** Implements the presentation clock synchronization protocol from
RFC-0002 §7 — computing the host-side capture schedule from viewer vsync phase reports
and measured pipeline latency.

**What it contains:**
- `src/sync.rs` — `ClockSync`:
  ```rust
  pub struct ClockSync { /* internal state */ }
  impl ClockSync {
      pub fn new() -> Self;
      // Call on each VsyncReport from the viewer
      pub fn on_vsync_report(&mut self, report: &VsyncReport, recv_time: Instant);
      // Call on each completed encode; updates encode latency EMA
      pub fn on_encode_complete(&mut self, encode_duration: Duration);
      // Returns the target capture time for the next frame
      pub fn next_capture_time(&self, rtt: Duration) -> Option<Instant>;
      // Returns the target presentation timestamp to embed in the fragment header
      pub fn next_pts(&self) -> u64;
      pub fn is_synchronized(&self) -> bool;
      // True after WARMUP_FRAMES (30) encode samples have been collected
  }
  ```
- `src/offset.rs` — `ClockOffset` — one-way delay estimation from QUIC RTT; converts
  viewer-local vsync timestamps to host-local time domain
- `src/filter.rs` — `JitterFilter` — median filter over offset estimates to reject
  outliers from OS scheduling jitter
- `src/error.rs` — `ClockError` (reserved)

**Design note:** Like `renderd-abr`, `ClockSync` is a pure state machine. The host
application drives it; the clock sync module makes no scheduling decisions directly.

**External dependencies:** `renderd-proto`

---

### Application Layer

---

#### `renderd-host`
**Responsibility:** The macOS Login Item Agent binary — orchestrates all host-side crates
into a running application with a menu bar UI.

**Contains:** Orchestration only. No algorithms, no business logic, no protocol parsing.
The host binary wires crates together: configures the capture pipeline, connects the ABR
controller to the network layer, drives the clock sync from encoder callbacks, and owns
the application event loop.

**External dependencies:** All internal crates; `tokio`, `tray-icon`, `objc2`, `clap`

---

#### `renderd-viewer`
**Responsibility:** The Windows viewer binary — orchestrates all viewer-side crates into
a running application with a fullscreen window and tray icon.

**Contains:** Orchestration only. Wires together the QUIC client, reassembly window,
D3D12 decode/render pipeline, ABR feedback sender, vsync reporter, and reconnect watchdog.

**External dependencies:** All viewer-relevant internal crates; `tokio`, `winit`,
`windows-rs`, `clap`

---

## 4. Dependency Graph

The graph below is the complete allowed dependency matrix. An entry ✅ means the row
crate may depend on the column crate. An entry — means it may not. Circular dependencies
are structurally impossible given this matrix.

```
               proto  config  frame  crypto  vt-sys  sc-sys  net  keychain  discovery  abr  clock  host  viewer
renderd-proto    —      —       —      —       —       —      —      —         —         —     —      —     —
renderd-config   ✅     —       —      —       —       —      —      —         —         —     —      —     —
renderd-frame    ✅     —       —      —       —       —      —      —         —         —     —      —     —
renderd-crypto   ✅     —       —      —       —       —      —      —         —         —     —      —     —
renderd-vt-sys   —      —       —      —       —       —      —      —         —         —     —      —     —
renderd-sc-sys   —      —       ✅     —       ✅      —      —      —         —         —     —      —     —
renderd-net      ✅     —       ✅     ✅      —       —      —      —         —         —     —      —     —
renderd-keychain ✅     —       —      ✅      —       —      —      —         —         —     —      —     —
renderd-discovery✅     ✅      —      —       —       —      —      —         —         —     —      —     —
renderd-abr      ✅     ✅      —      —       —       —      —      —         —         —     —      —     —
renderd-clock    ✅     —       —      —       —       —      ✅     —         —         —     —      —     —
renderd-host     ✅     ✅      ✅     ✅      ✅      ✅     ✅     ✅        ✅         ✅    ✅     —     —
renderd-viewer   ✅     ✅      ✅     ✅      —       —      ✅     ✅        ✅         ✅    ✅     —     —
```

**Enforcement:** The dependency matrix is enforced by the workspace's `[dependencies]`
declarations. Cargo prevents a crate from importing another crate not listed in its
`Cargo.toml`. A CI step (`cargo tree --format {p} | sort -u`) checks that no dependency
edges exist that are not in the approved matrix.

---

## 5. Module Hierarchy

Conventions:
- `src/lib.rs` re-exports the public API with `pub use` — do not make consumers navigate
  into submodules.
- `src/error.rs` always contains the crate's `Error` type.
- Internal implementation modules are `pub(crate)` or private.
- `src/tests/` contains integration tests within the crate (distinct from `tests/`
  which are external integration tests using only the public API).

### Example: `renderd-frame` full module tree

```
renderd-frame/
├── Cargo.toml
├── benches/
│   └── reassembly.rs          # Criterion: throughput of window.insert()
├── src/
│   ├── lib.rs                 # pub use {Fragment, CompleteFrame, ReassemblyWindow, ...}
│   ├── error.rs               # pub enum FrameError { InvalidHeader, PayloadTooLarge, ... }
│   ├── header.rs              # pub(crate) struct FragmentHeader; parse/serialize
│   ├── fragment.rs            # pub struct Fragment; pub fn parse(bytes) -> Result
│   ├── frame.rs               # pub struct CompleteFrame; pub struct PartialFrameTimeout
│   ├── window.rs              # pub struct ReassemblyWindow<const W: usize = 4>
│   ├── deadline.rs            # pub struct DeadlineComputer
│   └── tests/
│       ├── header_tests.rs    # Unit tests for header parse/serialize roundtrip
│       ├── window_tests.rs    # Unit + property tests for reassembly state machine
│       └── deadline_tests.rs  # Unit tests for dynamic deadline computation
└── tests/
    └── integration.rs         # Black-box tests via public API only
```

### Example: `renderd-host` full module tree

```
renderd-host/
├── Cargo.toml
├── c-shims/
│   └── videotoolbox_shim.c
├── build.rs
├── Info.plist
├── entitlements.plist
├── src/
│   ├── main.rs                # Entry point; panic hook; config load; runtime spawn
│   ├── app.rs                 # Application state machine; owns all sub-systems
│   ├── capture.rs             # SCStream lifecycle; QoS thread; frame→ring_buffer
│   ├── encode.rs              # VTCompressionSession wrapper; bitrate updates; keyframe
│   ├── session/
│   │   ├── mod.rs             # Session state machine (IDLE→PAIRING→CONNECTED)
│   │   ├── pairing.rs         # SPAKE2+ ceremony orchestration
│   │   ├── auth.rs            # Cert validation; known-viewers registry
│   │   └── devices.rs         # PairedDevice list; revocation; notifications
│   ├── network/
│   │   ├── mod.rs
│   │   ├── server.rs          # QUIC server loop; accept new connections
│   │   ├── control.rs         # Stream 0 dispatch; route incoming Envelope to handlers
│   │   └── data.rs            # Datagram burst-send loop
│   ├── clock.rs               # Drives renderd-clock::ClockSync from encoder + VsyncReport
│   ├── abr.rs                 # Drives renderd-abr::AbrController; applies decisions
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── menubar.rs         # NSStatusBar item; menu construction
│   │   ├── devices_panel.rs   # Paired devices window
│   │   └── notifications.rs   # UserNotifications (session start, pairing)
│   └── error.rs               # HostError (anyhow-wrapping; binary-only)
└── tests/
    └── (none — integration tested via renderd-viewer against a mock host)
```

---

## 6. Workspace Cargo Configuration

### `Cargo.toml` (workspace root)

```toml
[workspace]
resolver = "2"
members = [
    "crates/renderd-proto",
    "crates/renderd-config",
    "crates/renderd-frame",
    "crates/renderd-crypto",
    "crates/renderd-vt-sys",
    "crates/renderd-sc-sys",
    "crates/renderd-net",
    "crates/renderd-keychain",
    "crates/renderd-discovery",
    "crates/renderd-abr",
    "crates/renderd-clock",
    "crates/renderd-host",
    "crates/renderd-viewer",
    "tools/latency-bench",
]

[workspace.package]
version     = "0.1.0"
edition     = "2021"
rust-version = "1.80"        # MSRV; updated each release cycle
license     = "MIT"
authors     = ["Renderd Contributors"]
repository  = "https://github.com/renderd/renderd"
homepage    = "https://renderd.dev"

# All internal dependency versions are declared here once.
# Crates reference them as: renderd-proto = { workspace = true }
[workspace.dependencies]
# Internal
renderd-proto     = { path = "crates/renderd-proto" }
renderd-config    = { path = "crates/renderd-config" }
renderd-frame     = { path = "crates/renderd-frame" }
renderd-crypto    = { path = "crates/renderd-crypto" }
renderd-vt-sys    = { path = "crates/renderd-vt-sys" }
renderd-sc-sys    = { path = "crates/renderd-sc-sys" }
renderd-net       = { path = "crates/renderd-net" }
renderd-keychain  = { path = "crates/renderd-keychain" }
renderd-discovery = { path = "crates/renderd-discovery" }
renderd-abr       = { path = "crates/renderd-abr" }
renderd-clock     = { path = "crates/renderd-clock" }

# External — single canonical version for all crates
tokio         = { version = "1",  features = ["full"] }
quinn         = { version = "0.11" }
rustls        = { version = "0.23", default-features = false, features = ["tls12"] }
bytes         = { version = "1" }
prost         = { version = "0.13" }
prost-build   = { version = "0.13" }
serde         = { version = "1", features = ["derive"] }
serde_derive  = { version = "1" }
toml          = { version = "0.8" }
figment       = { version = "0.10", features = ["toml", "env"] }
tracing       = { version = "0.1" }
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
thiserror     = { version = "2" }
anyhow        = { version = "1" }          # binaries only; see §13
uuid          = { version = "1", features = ["v4", "serde"] }
p256          = { version = "0.13", features = ["ecdh", "ecdsa"] }
hmac          = { version = "0.12" }
sha2          = { version = "0.10" }
hkdf          = { version = "0.12" }
rcgen         = { version = "0.13" }
zeroize       = { version = "1", features = ["derive"] }
crossbeam-channel = { version = "0.5" }
criterion     = { version = "0.5", features = ["html_reports"] }
proptest      = { version = "1" }
cc            = { version = "1" }
bindgen       = { version = "0.70" }
clap          = { version = "4", features = ["derive", "env"] }
# macOS-specific
objc2         = { version = "0.5" }
objc2-foundation = { version = "0.2" }
objc2-screen-capture-kit = { version = "0.2" }
core-foundation  = { version = "0.10" }
core-media       = { version = "0.2" }
security-framework = { version = "2" }
# Windows-specific
winit         = { version = "0.30" }
windows       = { version = "0.58", features = [
    "Win32_Media_MediaFoundation",
    "Win32_Graphics_Direct3D12",
    "Win32_Graphics_Dxgi",
    "Win32_UI_WindowsAndMessaging",
    "Win32_System_Credentials",
    "Win32_NetworkManagement_Dns",
    "Win32_System_Com",
] }

[workspace.lints.rust]
# Applied to all crates; see §9
unsafe_code          = "warn"   # upgraded to "deny" in non-FFI crates via per-crate override
missing_docs         = "warn"
unused_must_use      = "deny"
rust_2018_idioms     = "warn"
nonstandard_style    = "deny"
future_incompatible  = "deny"

[workspace.lints.clippy]
all          = "warn"
pedantic     = "warn"
nursery      = "warn"
# Allowances applied workspace-wide (see §9 for rationale)
module_name_repetitions = "allow"
must_use_candidate      = "allow"
missing_errors_doc      = "allow"   # enforced separately by CI doc check

[profile.release]
opt-level     = 3
lto           = "thin"
codegen-units = 1
strip         = "debuginfo"

[profile.bench]
inherits = "release"
debug    = true          # keep debug info for flamegraphs

[profile.dev]
opt-level     = 1        # faster incremental builds; sufficient for tests
debug         = true
```

### `rust-toolchain.toml`

```toml
[toolchain]
channel  = "stable"
components = ["rustfmt", "clippy", "rust-src", "llvm-tools-preview"]
targets  = [
    "aarch64-apple-darwin",    # macOS host
    "x86_64-pc-windows-msvc",  # Windows viewer (x86_64)
    "aarch64-pc-windows-msvc", # Windows viewer (ARM64, future)
]
```

---

## 7. Ownership and Review Rules

### 7.1 CODEOWNERS

```
# .github/CODEOWNERS
# Global fallback — every PR requires at least one maintainer review
*                               @renderd/maintainers

# Foundation layer — any maintainer can review
/crates/renderd-proto/          @renderd/maintainers
/crates/renderd-config/         @renderd/maintainers

# Cryptography — requires crypto-qualified reviewer
/crates/renderd-crypto/         @renderd/crypto-reviewers

# FFI layer — requires platform specialist
/crates/renderd-vt-sys/         @renderd/macos-team
/crates/renderd-sc-sys/         @renderd/macos-team

# Algorithm layer — requires latency-domain reviewer
/crates/renderd-abr/            @renderd/algorithms-team
/crates/renderd-clock/          @renderd/algorithms-team

# Host binary — macOS specialists
/crates/renderd-host/           @renderd/macos-team

# Viewer binary — Windows specialists
/crates/renderd-viewer/         @renderd/windows-team

# CI/CD — any maintainer
/.github/                       @renderd/maintainers

# Protocol — all protocol changes need RFC update
/proto/                         @renderd/maintainers
```

### 7.2 Review Requirements

| Change Type | Required Reviews |
|-------------|-----------------|
| Protocol buffer changes | 2 maintainers + RFC-0002 update |
| Cryptography changes | 1 crypto-reviewer + 1 maintainer |
| FFI crate changes | 1 platform specialist + 1 maintainer |
| Data plane changes | Benchmark regression report required |
| Config schema changes | Backward compatibility analysis required |
| Release process changes | All maintainers |

### 7.3 Ownership Rules

- A crate maintainer is responsible for keeping its dependencies up to date (monthly audit).
- No crate may add a new external dependency without a comment in the PR explaining:
  - What the dependency does
  - Why an existing dependency or std stdlib cannot serve the same purpose
  - License compatibility (MIT/Apache-2.0 only)
- Dependency additions must pass `cargo deny check` before merge.

---

## 8. Coding Standards

### 8.1 General Style

- **Edition:** 2021 in all crates.
- **MSRV:** Currently `1.80`. MSRV bumps require a changelog entry and a justification.
- **Max line length:** 100 characters (enforced by rustfmt).
- **Max function length:** No hard limit, but functions exceeding 60 lines should be
  justified in a comment. Functions exceeding 100 lines require a PR comment explaining
  why refactoring is not possible.
- **Nesting depth:** Maximum 4 levels. Prefer early returns (`?`, `return Err(...)`)
  over nested `if let` chains.

### 8.2 Naming Conventions

Follow Rust API Guidelines (https://rust-lang.github.io/api-guidelines):

| Item | Convention | Example |
|------|-----------|---------|
| Types, traits, enums | `UpperCamelCase` | `ReassemblyWindow`, `KeychainStore` |
| Functions, methods, variables | `snake_case` | `insert_fragment`, `pair_token` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_FRAGMENT_SIZE`, `WARMUP_FRAMES` |
| Crate feature flags | `kebab-case` | `macos-bonjour`, `windows-dxgi` |
| Modules | `snake_case` | `clock_sync`, `burst_send` |
| Error variants | `UpperCamelCase`, no `Error` suffix | `KeychainStore::Save`, not `SaveError` |
| Lifetime names | Single letter for generic: `'a`; descriptive for complex: `'session` |

### 8.3 Error Handling Rules

See §13 for the complete error handling strategy. Summary:
- Library crates: `thiserror`; typed `Error` enums; no `anyhow`.
- Binary crates: `anyhow` at the outermost boundary only; never in helper functions.
- No `unwrap()` in library code, ever.
- No `panic!()` in library code unless documenting a violated invariant in a comment.
- `expect("reason")` is allowed in tests. The reason must be a complete sentence.

### 8.4 Concurrency Rules

- Shared state between threads must use `Arc<Mutex<T>>` or `Arc<RwLock<T>>`, but not
  in the capture/encode/send hot path. Hot path uses SPSC channels (crossbeam).
- Never hold a `Mutex` across an `.await` point. Use `tokio::sync::Mutex` only if the
  lock must be held across await; strongly prefer `std::sync::Mutex` + releasing before
  any await.
- `tokio::spawn` tasks must be named: `tokio::task::Builder::new().name("capture").spawn(...)`.
  Named tasks appear in tracing output and in process inspection tools.

### 8.5 Documentation Rules

See §16 for full documentation standards. All `pub` items must have a doc comment.
All `pub(crate)` items should have a doc comment. `///` for items, `//!` for modules.

### 8.6 Feature Flags

Feature flags are used only for:
- Platform-specific code (`cfg(target_os)` is preferred over features for platform gates)
- Optional telemetry integrations (e.g., `opentelemetry` export)
- Test utilities (`#[cfg(test)]` is preferred for test-only code within the same crate)

Do not use feature flags to gate correctness-affecting behavior. Feature flags that
change protocol behavior are forbidden.

---

## 9. Lint Configuration

### `clippy.toml`

```toml
# clippy.toml — workspace root
msrv = "1.80"

# Disallowed types (use our wrappers instead)
disallowed-types = [
    # Use renderd-crypto types, not raw arrays
    { path = "[u8; 32]", reason = "Use PairToken or SessionKey newtype" },
]

# Disallowed methods
disallowed-methods = [
    { path = "std::process::exit", reason = "Use proper application shutdown; never call exit() directly" },
    { path = "std::env::var",      reason = "Use renderd-config's figment-based loading" },
]
```

### Per-crate lint overrides

Each crate's `Cargo.toml` overrides workspace lints as needed. The key override is
for FFI crates:

```toml
# crates/renderd-vt-sys/Cargo.toml
[lints]
workspace = true

[lints.rust]
# FFI crates necessarily use unsafe; warn on each block rather than deny the module
unsafe_code = "warn"
```

All other crates inherit `unsafe_code = "deny"` from the workspace.

### Standard lint attributes in `lib.rs`

Every library crate begins with:

```rust
//! Crate-level doc comment describing the single responsibility.
#![warn(
    missing_docs,
    rust_2018_idioms,
    clippy::pedantic,
    clippy::nursery,
)]
#![deny(
    unsafe_code,          // removed for renderd-vt-sys, renderd-sc-sys
    nonstandard_style,
    future_incompatible,
    unused_must_use,
)]
#![allow(
    clippy::module_name_repetitions,  // acceptable in Rust library design
)]
```

### `deny.toml` (cargo-deny)

```toml
[licenses]
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "ISC", "BSD-2-Clause", "BSD-3-Clause", "Unicode-DFS-2016"]
deny  = ["GPL-2.0", "GPL-3.0", "LGPL-2.0", "LGPL-3.0", "AGPL-3.0"]
exceptions = []

[bans]
multiple-versions = "warn"   # upgraded to "deny" for known-stable crates
wildcards         = "deny"   # no wildcard version specs

[advisories]
db-path   = "~/.cargo/advisory-db"
db-urls   = ["https://github.com/rustsec/advisory-db"]
vulnerability = "deny"
unmaintained  = "warn"
unsound       = "deny"

[sources]
unknown-registry = "deny"
unknown-git      = "deny"
allow-registry   = ["https://github.com/rust-lang/crates.io-index"]
```

---

## 10. Formatting

### `.rustfmt.toml`

```toml
edition             = "2021"
max_width           = 100
hard_tabs           = false
tab_spaces          = 4
newline_style       = "Unix"
use_small_heuristics = "Default"

# Imports
imports_granularity  = "Crate"   # Group all from same crate: use std::{io, fmt, ...}
group_imports        = "StdExternalCrate"  # std → external → crate-local

# Control flow
control_brace_style  = "AlwaysSameLine"
brace_style          = "SameLineWhere"
where_single_line    = false

# Functions
fn_params_layout     = "Tall"
fn_single_line       = false

# Trailing commas
trailing_comma        = "Vertical"
trailing_semicolon    = true

# Comments
comment_width         = 100
normalize_comments    = true
wrap_comments         = false   # don't auto-wrap; maintain deliberate line breaks

# Misc
format_macro_matchers = true
format_strings        = false
```

Formatting is enforced in CI with `cargo fmt --check`. A failed format check blocks
merge. Developers must run `cargo fmt` before pushing.

---

## 11. Logging and Telemetry

### 11.1 Framework: `tracing`

All logging uses the `tracing` crate with structured fields. `println!` and `eprintln!`
are forbidden in library code. In binaries, they are permitted only for the initial
startup message before the tracing subscriber is initialized.

### 11.2 Span and Event Conventions

```rust
// Spans: use for operations with measurable duration
let _span = tracing::info_span!(
    "encode_frame",
    frame_id = %frame_id,
    is_keyframe = is_keyframe,
    bitrate_kbps = bitrate,
).entered();

// Events: use for discrete occurrences
tracing::debug!(
    frame_id = %frame_id,
    frag_count = frag_count,
    encode_us = encode_duration.as_micros(),
    "frame encoded"
);

tracing::warn!(
    frame_id = %frame_id,
    fragments_received = received,
    fragments_expected = total,
    "fragment deadline exceeded; requesting keyframe"
);
```

### 11.3 Level Guidelines

| Level | Use for |
|-------|---------|
| `error` | Unrecoverable errors; session termination events |
| `warn` | Recoverable anomalies: frame drop, keyframe request, ABR step-down |
| `info` | Session lifecycle: connect, pair, disconnect, ABR decisions |
| `debug` | Per-frame events: encode complete, fragment send, reassembly complete |
| `trace` | Sub-frame events: individual datagram send, clock sync updates |

**Rule:** The `debug` level must be usable in production for 60-FPS streaming without
perceptible overhead. `trace` is for debugging sessions only.

### 11.4 Subscriber Configuration

Both binaries initialize the tracing subscriber in `main.rs` before any other code:

```rust
fn init_tracing(config: &LogConfig) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.level));
    match config.format.as_str() {
        "json" => tracing_subscriber::registry()
            .with(fmt::layer().json())
            .with(filter)
            .init(),
        _ => tracing_subscriber::registry()
            .with(fmt::layer().compact())
            .with(filter)
            .init(),
    }
}
```

The `RUST_LOG` environment variable overrides the config file setting, following
the `EnvFilter` convention (`RUST_LOG=renderd_net=debug,info`).

### 11.5 Latency-Critical Spans

Spans in the capture→encode→send and receive→decode→present paths must use
`tracing::trace_span!` (not `info_span!`) to be no-ops when trace is disabled.
The overhead of a no-op trace span is approximately 5 ns — acceptable in the hot path.
An enabled `info_span` at 60 FPS costs approximately 300 µs/second — not acceptable.

### 11.6 Metrics

Version 1.0 does not integrate an external metrics system. Statistics are computed
in `renderd-abr`'s `BandwidthEstimator` and reported on the tracing span as structured
fields. A `METRICS.md` document will be added in v1.1 when Prometheus/OpenTelemetry
export is added.

---

## 12. Configuration System

### 12.1 Loading Priority (highest to lowest)

1. Command-line flags (`--bitrate-kbps 40000`) via `clap`
2. Environment variables (`RENDERD_BITRATE_KBPS=40000`) via `figment`
3. User config file (`~/.config/renderd/host.toml` / `%APPDATA%\renderd\viewer.toml`)
4. System config file (`/etc/renderd/host.toml` / not applicable on Windows)
5. Compiled-in defaults (hardcoded in `renderd-config` structs via `#[serde(default)]`)

### 12.2 Config File Location Resolution

Config file path resolution is the binary's responsibility — not `renderd-config`'s.
The binary passes a resolved `Option<&Path>` to `Config::load()`.

```
macOS host:   ~/Library/Application Support/dev.renderd.host/host.toml
Windows viewer: %APPDATA%\Renderd\viewer.toml
```

### 12.3 Schema Stability

Config file schema is versioned. Breaking changes (renamed keys, removed keys) require:
1. A migration note in `CHANGELOG.md`
2. A deprecation warning when an old key is found (use `serde_ignored` to detect unknown fields)
3. A one-version grace period where the old key is still accepted with a warning

### 12.4 Validation

`Config::load()` calls `Config::validate()` after deserialization. `validate()` returns
`Result<(), ConfigError>` and checks:
- `min_bitrate_kbps < max_bitrate_kbps`
- `reactive_interval_ms < proactive_interval_ms`
- Port is in range 1024–65535
- `codec` is `"hevc"` or `"h264"` (case-insensitive)
- No values are zero where zero is nonsensical

Invalid configuration causes the binary to print a human-readable error and exit with
code 78 (`EX_CONFIG` from `<sysexits.h>`).

---

## 13. Error Handling

### 13.1 Strategy

**Library crates use `thiserror`.** Every public function returns `Result<T, E>` where
`E` is a crate-specific, `#[derive(thiserror::Error)]` enum. Error variants carry
structured context. Error strings are never formatted into the `Err` variant — the
consumer decides how to display errors.

**Binary crates use `anyhow` only at the application boundary.** Helper functions
within binaries use typed errors from the library crates they call. Only `main()`,
the top-level task handlers, and the panic hook use `anyhow::Result` for ergonomic
error propagation. No library crate may take `anyhow` as a dependency.

### 13.2 Error Hierarchy

```
RenderdError (conceptual; not a real type — handled by anyhow in binaries)
├── NetError (renderd-net)
│   ├── ConnectionFailed(quinn::ConnectionError)
│   ├── TlsHandshakeFailed(rustls::Error)
│   ├── ControlStreamClosed
│   ├── InvalidEnvelope(prost::DecodeError)
│   └── FragmentTooLarge { size: usize, max: usize }
│
├── FrameError (renderd-frame)
│   ├── InvalidHeader { reason: &'static str }
│   ├── PayloadTooLarge { size: usize }
│   └── DuplicateFragment { frame_id: FrameId, frag_id: FragmentId }
│
├── CryptoError (renderd-crypto)
│   ├── Spake2VerificationFailed
│   ├── HkdfExpandFailed
│   └── CertGenerationFailed(rcgen::Error)
│
├── KeychainError (renderd-keychain)
│   ├── NotFound(HostId)
│   ├── SaveFailed { reason: String }   # platform error message
│   └── DeleteFailed { reason: String }
│
├── DiscoveryError (renderd-discovery)
│   ├── BindFailed(std::io::Error)
│   ├── ServiceRegistrationFailed { reason: String }
│   └── BrowseFailed(std::io::Error)
│
├── VtError (renderd-vt-sys)
│   ├── SessionCreateFailed(OSStatus)
│   ├── EncodeFrameFailed(OSStatus)
│   ├── HardwareEncoderUnavailable
│   └── InvalidSurface
│
├── ScError (renderd-sc-sys)
│   ├── PermissionDenied
│   ├── NoDisplaysFound
│   └── StreamFailed(String)
│
└── ConfigError (renderd-config)
    ├── ParseFailed(toml::de::Error)
    ├── IoFailed(std::io::Error)
    └── ValidationFailed { field: &'static str, reason: String }
```

### 13.3 Context Addition

When propagating errors across crate boundaries in binaries, always add context:

```rust
// Good: context explains what was being attempted
capture.start().context("failed to start ScreenCaptureKit stream")?;

// Bad: naked ? loses the operation context
capture.start()?;
```

### 13.4 Panic Policy

- Library crates: `panic!` is permitted only for violated invariants that indicate a
  programming error (not a runtime error). Every panic must be preceded by a comment
  explaining the invariant.
- Binaries: Install a panic hook in `main()` that logs via `tracing::error!` before
  the default panic handler runs. This ensures panics appear in structured logs.
- Do not use `panic!` for error paths that can be reached through normal operation
  (e.g., malformed network data). Use `Result`.

---

## 14. Testing Strategy

### 14.1 Test Categories

| Category | Location | Runner | Purpose |
|----------|----------|--------|---------|
| Unit | `src/tests/` or `#[cfg(test)]` inline | `nextest` | Test individual functions in isolation |
| Integration | `tests/` (crate root) | `nextest` | Test public API with real dependencies |
| Property | `src/tests/` using `proptest` | `nextest` | Test invariants over random inputs |
| Simulation | `tests/simulation/` in host/viewer | `nextest` | Test protocol state machines against mock |
| Latency | `tools/latency-bench/` | Manual + CI nightly | Measure glass-to-glass pipeline latency |
| Benchmark | `benches/` per crate | `criterion` | Measure throughput of hot-path functions |

### 14.2 Unit Test Conventions

```rust
// In src/window.rs:
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::make_fragment;

    #[test]
    fn single_fragment_frame_completes_immediately() {
        let mut window = ReassemblyWindow::default();
        let frag = make_fragment(frame_id: 1, frag_id: 0, frag_total: 1);
        let result = window.insert(frag);
        assert!(result.is_some(), "single-fragment frame must complete on first insert");
    }

    #[test]
    fn out_of_order_fragments_complete_frame() {
        let mut window = ReassemblyWindow::default();
        // Fragment 1 arrives before fragment 0
        window.insert(make_fragment(frame_id: 1, frag_id: 1, frag_total: 2));
        let result = window.insert(make_fragment(frame_id: 1, frag_id: 0, frag_total: 2));
        assert!(result.is_some(), "frame must complete when all fragments arrive regardless of order");
    }
}
```

Rules:
- Test names are complete sentences describing the expected behavior.
- Each test has a single assertion or a cohesive set of assertions for one behavior.
- No `sleep()` in tests. Use explicit state machine advancement.
- Test helpers live in `src/tests/helpers.rs` — not in `mod tests` itself.

### 14.3 Property Tests

Property tests use `proptest` for the reassembly window, HKDF derivation, and ABR ramp functions:

```rust
proptest! {
    #[test]
    fn window_never_returns_duplicate_frames(
        frame_ids in proptest::collection::vec(0u64..100, 1..50),
        frag_counts in proptest::collection::vec(1u16..10, 1..50),
    ) {
        let mut window = ReassemblyWindow::default();
        let mut completed = std::collections::HashSet::new();
        for (frame_id, frag_total) in frame_ids.iter().zip(frag_counts.iter()) {
            for frag_id in 0..*frag_total {
                if let Some(frame) = window.insert(make_fragment(*frame_id, frag_id, *frag_total)) {
                    let inserted = completed.insert(frame.frame_id);
                    prop_assert!(inserted, "duplicate frame completion for frame_id={}", frame.frame_id);
                }
            }
        }
    }
}
```

### 14.4 Protocol Simulation Tests

The host and viewer state machines are tested against each other using in-process mock
transports. A `MockTransport` in `renderd-net` (behind `#[cfg(any(test, feature = "test-utils"))]`)
replaces the QUIC connection with in-memory channels.

```
tests/simulation/
├── pairing_flow.rs        # Full SPAKE2+ pairing ceremony, both sides
├── session_flow.rs        # SessionHello/SessionConfig exchange
├── abr_flow.rs            # ReactiveStats → BitrateAdjust → encode update loop
├── reconnect_flow.rs      # Disconnect → mDNS re-discover → reconnect
└── clock_sync_flow.rs     # VsyncReport → ClockSync.next_capture_time
```

### 14.5 RFC 9382 Test Vectors

`renderd-crypto` must contain the following test as the very first test in the module:

```rust
#[test]
fn rfc9382_test_vectors() {
    // Test vectors from RFC 9382 §4.
    // This test MUST pass before any other test in this module runs.
    // If this test fails, the SPAKE2+ implementation is wrong.
    // Do not merge changes to renderd-crypto that cause this test to fail.
    for vector in RFC9382_VECTORS.iter() {
        let result = run_spake2plus_vector(vector);
        assert_eq!(result.confirm_p, vector.expected_confirm_p,
            "RFC 9382 test vector {} confirm_p mismatch", vector.id);
        assert_eq!(result.confirm_v, vector.expected_confirm_v,
            "RFC 9382 test vector {} confirm_v mismatch", vector.id);
    }
}
```

### 14.6 Coverage

Coverage is measured with `cargo-llvm-cov` on every CI run. Coverage requirements:
- `renderd-frame`: ≥ 90% line coverage
- `renderd-crypto`: ≥ 95% line coverage (cryptographic code must be thoroughly tested)
- `renderd-abr`: ≥ 85% line coverage
- `renderd-clock`: ≥ 85% line coverage
- All other library crates: ≥ 75% line coverage

Coverage gates are enforced in CI. A PR that drops coverage below the threshold requires
a documented justification to merge.

### 14.7 `nextest.toml`

```toml
[profile.default]
retries = 1                       # flaky test retry; investigate if retried twice in a row
test-threads = "num-cpus"
failure-output = "immediate-final"
status-level = "fail"

[profile.ci]
retries = 2
slow-timeout = { period = "30s", terminate-after = 3 }
test-threads = 4                  # constrained in CI

[profile.default.junit]
path = "target/nextest/junit.xml"
```

---

## 15. Benchmarking

### 15.1 Framework: `criterion`

All benchmarks use `criterion` with HTML report generation. Benchmarks live in the
`benches/` directory of the crate that owns the benchmarked code.

### 15.2 Required Benchmarks

The following benchmarks are required. A PR that removes or significantly degrades any
of these benchmarks must be justified in the review.

| Crate | Benchmark | What it measures |
|-------|-----------|-----------------|
| `renderd-frame` | `bench_reassembly_single` | `window.insert()` for a single-fragment frame |
| `renderd-frame` | `bench_reassembly_burst` | `window.insert()` for 55-fragment frame (30 Mbps / 1080p60) |
| `renderd-frame` | `bench_header_parse` | `FragmentHeader::parse()` throughput |
| `renderd-net` | `bench_fragment_burst_send` | `FragmentBurst::send_all()` for 55 fragments |
| `renderd-crypto` | `bench_hkdf_derive` | `derive_pair_token()` latency |
| `renderd-abr` | `bench_abr_reactive` | `AbrController::on_reactive()` throughput |
| `renderd-clock` | `bench_clock_sync` | `ClockSync::on_vsync_report()` latency |

### 15.3 Benchmark Conventions

```rust
// benches/reassembly.rs
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use renderd_frame::{ReassemblyWindow, Fragment};

fn bench_reassembly_burst(c: &mut Criterion) {
    let mut group = c.benchmark_group("reassembly");
    for frag_count in [1u16, 10, 55, 100].iter() {
        group.bench_with_input(
            BenchmarkId::new("insert_burst", frag_count),
            frag_count,
            |b, &count| {
                let fragments: Vec<Fragment> = (0..count)
                    .map(|i| make_fragment(1, i, count))
                    .collect();
                b.iter(|| {
                    let mut window = ReassemblyWindow::default();
                    for frag in &fragments {
                        window.insert(frag.clone());
                    }
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_reassembly_burst);
criterion_main!(benches);
```

### 15.4 Benchmark CI

Benchmarks run nightly on the CI `bench.yml` workflow. Results are published to
GitHub Pages via `critcmp`. A PR that introduces a regression of more than 10% on
any required benchmark requires a `perf:` labeled explanation in the PR description.

### 15.5 Latency Benchmark Tool

`tools/latency-bench/` is a standalone binary that measures end-to-end pipeline latency
with microsecond timestamps at every stage:

```
latency-bench --frames 1000 --resolution 1920x1080 --codec hevc

Stage                      p50       p95       p99       max
──────────────────────────────────────────────────────────
Capture (vsync→IOSurface)   1.8ms     2.4ms     3.1ms     4.2ms
Encode (VT→callback)        7.1ms     9.4ms    11.2ms    14.8ms
Burst send (55 datagrams)   0.4ms     0.6ms     0.9ms     1.4ms
Network RTT (loopback)      0.3ms     0.4ms     0.5ms     0.8ms
Reassembly (last frag→frame) 0.1ms    0.2ms     0.3ms     0.4ms
D3D12 decode                2.1ms     2.8ms     3.4ms     4.1ms
D3D12 render + present      1.4ms     1.9ms     2.3ms     3.0ms
────────────────────────────────────────────────────────────
Total (excl. scanout)      13.2ms    17.7ms    21.7ms    28.7ms
```

This tool runs against a loopback network, not a real LAN. Its primary purpose is to
identify individual stage regressions in isolation.

---

## 16. Documentation Standards

### 16.1 Rustdoc Requirements

All `pub` items must have a doc comment. The comment must include:
- A one-sentence summary (the first line, ending with a period)
- For functions: what it does, what it returns, and when it returns `Err`
- For types: what it represents and its invariants
- For traits: the contract the implementor must uphold

```rust
/// Inserts a fragment into the reassembly window.
///
/// Returns a [`CompleteFrame`] if the insertion completes a frame (i.e., all
/// expected fragments have arrived). Returns `None` if the frame is still incomplete.
///
/// Fragments whose `frame_id` is older than `(max_seen - W)` are silently discarded.
///
/// # Panics
///
/// Does not panic. Malformed fragments return `None` rather than panicking.
pub fn insert(&mut self, fragment: Fragment) -> Option<CompleteFrame> { ... }
```

### 16.2 Module-Level Documentation

Every module file begins with a `//!` module doc comment describing:
- The module's responsibility (one paragraph)
- Key types and their relationships
- Usage example where non-obvious

### 16.3 Crate-Level Documentation

`crates/*/src/lib.rs` begins with:
- A `//!` comment with the crate's single-sentence responsibility
- A `# Architecture` section explaining the internal module structure
- A `# Usage` section with a complete, compilable example (using `# ` to hide boilerplate)
- A `# Panics` section listing all possible panic conditions
- A `# Platform Support` section if relevant

### 16.4 `docs/` Folder

The `docs/` folder contains RFC documents and this engineering spec. All documents:
- Are written in Markdown with GitHub Flavored Markdown extensions
- Use ATX headings (# not underline style)
- Reference related documents with relative links: `[RFC-0002](RFC-0002-architecture.md)`
- Do not contain absolute local file paths

### 16.5 Documentation Deployment

`docs.yml` workflow builds rustdoc for all crates and deploys to GitHub Pages at
`https://renderd.dev/docs/`. Coverage report is included. The workflow runs on every
push to `main`.

---

## 17. Release Process

### 17.1 Versioning

Renderd follows **Semantic Versioning 2.0** (semver.org). The host and viewer share a
version number — they are released together. Library crates are versioned independently.

Binary version format: `MAJOR.MINOR.PATCH[-prerelease]`
- **MAJOR:** Breaking protocol change (old viewer cannot connect to new host or vice versa)
- **MINOR:** Backward-compatible feature addition (new codec, new control message with graceful fallback)
- **PATCH:** Bug fixes, latency improvements, security patches

Library crate versioning follows the Cargo semver compatibility rules independently.

### 17.2 Release Cadence

- **Patch releases:** As needed for security fixes or critical bugs. No waiting.
- **Minor releases:** Every 6–8 weeks on the `main` branch stabilizing.
- **Major releases:** Only when a protocol breaking change is necessary.

### 17.3 Release Checklist

```
Pre-release (at least 48 hours before):
  [ ] All planned PR merged to main
  [ ] CHANGELOG.md updated (Keep-a-Changelog format)
  [ ] Version bumped in workspace Cargo.toml
  [ ] `cargo deny check` passes
  [ ] All CI workflows green on main
  [ ] Latency benchmark baseline updated
  [ ] `cargo doc --no-deps` builds cleanly

Release:
  [ ] Create signed git tag: git tag -s v0.2.0 -m "Release v0.2.0"
  [ ] Push tag: git push origin v0.2.0
  [ ] GitHub Actions release-host.yml runs automatically (builds + notarizes macOS .app)
  [ ] GitHub Actions release-viewer.yml runs automatically (builds + packages Windows .exe)
  [ ] GitHub Release created automatically from tag with CHANGELOG section as notes
  [ ] Artifacts uploaded: renderd-host-v0.2.0-aarch64-apple-darwin.dmg
                          renderd-viewer-v0.2.0-x86_64-windows-installer.exe

Post-release:
  [ ] Create new [Unreleased] section in CHANGELOG.md
  [ ] Announce on GitHub Discussions and project website
  [ ] Update README.md with new version in badges
```

### 17.4 CHANGELOG Format

Follows Keep-a-Changelog (https://keepachangelog.com):

```markdown
## [Unreleased]

## [0.2.0] — 2026-09-15

### Added
- Presentation clock synchronization (§7, RFC-0002): reduces average latency by ~8 ms
  at 1080p60 on synchronized displays.
- Paired devices management panel in host menu bar UI.
- Session start notification via macOS UserNotifications.

### Changed
- Fragment deadline is now dynamic (computed from decode + render telemetry) rather
  than a fixed 8 ms. Default is 12 ms.
- ABR feedback interval split into reactive (100 ms) and proactive (500 ms) channels.

### Fixed
- Viewer no longer fails to reconnect after host IP changes due to DHCP renewal.
  mDNS re-discovery by UUID is now integrated into the reconnect loop.

### Security
- HKDF info string now uses canonical UUID format with fixed delimiter, eliminating
  the length-ambiguity collision vector.
```

---

## 18. GitHub Actions

### 18.1 `ci.yml` — Primary CI (runs on every PR and push to main)

```yaml
name: CI
on:
  push:
    branches: [main, "release/*"]
  pull_request:

jobs:
  build-and-test-host:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: "clippy,rustfmt" }
      - uses: Swatinem/rust-cache@v2
      - name: Format check
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy --package renderd-host --package renderd-proto
             --package renderd-config --package renderd-frame --package renderd-crypto
             --package renderd-vt-sys --package renderd-sc-sys --package renderd-net
             --package renderd-keychain --package renderd-discovery
             --package renderd-abr --package renderd-clock
             -- -D warnings
      - name: Test (host crates)
        run: cargo nextest run --package renderd-proto --package renderd-config
             --package renderd-frame --package renderd-crypto --package renderd-abr
             --package renderd-clock --package renderd-net
             --profile ci
      - name: Coverage
        run: cargo llvm-cov nextest --package renderd-frame --package renderd-crypto
             --package renderd-abr --lcov --output-path lcov.info
      - uses: codecov/codecov-action@v4
        with: { files: lcov.info }

  build-and-test-viewer:
    runs-on: windows-2025
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: "clippy,rustfmt" }
      - uses: Swatinem/rust-cache@v2
      - name: Format check
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy --package renderd-viewer -- -D warnings
      - name: Test (viewer crates)
        run: cargo nextest run --package renderd-frame --package renderd-crypto
             --package renderd-abr --package renderd-clock --package renderd-net
             --package renderd-keychain --package renderd-discovery
             --profile ci

  proto-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Regenerate proto
        run: cargo run --manifest-path tools/proto-gen/Cargo.toml
      - name: Check no diff
        run: |
          if ! git diff --quiet crates/renderd-proto/src/generated/; then
            echo "Generated proto code is out of date. Run tools/proto-gen and commit."
            git diff crates/renderd-proto/src/generated/
            exit 1
          fi

  typos:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: crate-ci/typos@master
```

### 18.2 `security.yml` — Dependency Security Audit (weekly + on PR)

```yaml
name: Security
on:
  schedule: [{ cron: "0 6 * * 1" }]  # Monday 06:00 UTC
  pull_request:
    paths: ["Cargo.toml", "Cargo.lock", "**/Cargo.toml"]

jobs:
  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
        with: { command: check }

  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: rustsec/audit-check@v1
        with: { token: ${{ secrets.GITHUB_TOKEN }} }
```

### 18.3 `bench.yml` — Nightly Benchmark Runner

```yaml
name: Benchmarks
on:
  schedule: [{ cron: "0 2 * * *" }]  # 02:00 UTC nightly
  workflow_dispatch:

jobs:
  bench-host:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Run benchmarks
        run: cargo bench --package renderd-frame --package renderd-net
             --package renderd-crypto --package renderd-abr --package renderd-clock
             -- --output-format bencher | tee bench-output.txt
      - name: Store results
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: cargo
          output-file-path: bench-output.txt
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
          alert-threshold: "110%"   # alert if 10% regression
          comment-on-alert: true
          fail-on-alert: false      # nightly; don't fail, just alert
```

### 18.4 `release-host.yml` — macOS Release Build and Notarization

```yaml
name: Release Host
on:
  push:
    tags: ["v*"]

jobs:
  release-host:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: "aarch64-apple-darwin" }
      - name: Build release binary
        run: cargo build --release --target aarch64-apple-darwin --package renderd-host
      - name: Assemble .app bundle
        run: bash tools/bundle-host/assemble.sh
      - name: Import signing certificate
        uses: apple-actions/import-codesign-certs@v3
        with:
          p12-file-base64: ${{ secrets.MACOS_CERTIFICATE }}
          p12-password: ${{ secrets.MACOS_CERTIFICATE_PWD }}
      - name: Sign .app bundle
        run: |
          codesign --sign "${{ secrets.MACOS_SIGN_IDENTITY }}" \
                   --entitlements crates/renderd-host/entitlements.plist \
                   --options runtime \
                   --deep \
                   renderd-host.app
      - name: Notarize
        uses: apple-actions/notarize-app@v1
        with:
          product-path: renderd-host.app
          apple-id: ${{ secrets.APPLE_ID }}
          app-password: ${{ secrets.APPLE_APP_PASSWORD }}
          team-id: ${{ secrets.APPLE_TEAM_ID }}
      - name: Staple
        run: xcrun stapler staple renderd-host.app
      - name: Package DMG
        run: hdiutil create -volname "Renderd Host" -srcfolder renderd-host.app
             -ov -format UDZO renderd-host-${{ github.ref_name }}-aarch64.dmg
      - name: Upload to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: renderd-host-${{ github.ref_name }}-aarch64.dmg

  release-viewer:
    runs-on: windows-2025
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build release binary
        run: cargo build --release --target x86_64-pc-windows-msvc --package renderd-viewer
      - name: Package installer
        run: pwsh tools/package-viewer/package.ps1 -Version "${{ github.ref_name }}"
      - name: Upload to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: renderd-viewer-${{ github.ref_name }}-x86_64-windows.exe
```

### 18.5 `docs.yml` — Documentation Deployment

```yaml
name: Docs
on:
  push:
    branches: [main]

jobs:
  deploy-docs:
    runs-on: ubuntu-latest
    permissions:
      pages: write
      id-token: write
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build rustdoc
        run: cargo doc --no-deps --workspace --exclude renderd-host --exclude renderd-viewer
             # Exclude binaries; their internals are not public API
      - name: Deploy to GitHub Pages
        uses: actions/deploy-pages@v4
        with:
          artifact-name: github-pages
```

### 18.6 PR Merge Requirements

Branch protection rules for `main`:
- All CI jobs must pass.
- At least 1 approved review from a team member in CODEOWNERS for the changed paths.
- No unresolved conversations.
- Branch must be up-to-date with main before merge.
- Squash merge required (no merge commits on main).

---

## 19. Branch Strategy

### 19.1 Branch Model: GitHub Flow + Release Branches

```
main                    Stable, releasable at any commit
 │
 ├── feat/clock-sync    Feature branches: short-lived, off main
 ├── fix/fragment-oom   Bug fix branches: short-lived, off main
 ├── chore/bump-deps    Maintenance branches: short-lived, off main
 │
 └── release/0.2        Release branches: created when a release is imminent
                         Only patch bug fixes merged into release branches.
                         Tagged as v0.2.0, v0.2.1, etc.
```

### 19.2 Branch Naming Conventions

| Prefix | Use for | Example |
|--------|---------|---------|
| `feat/` | New features | `feat/dual-vsync-sync` |
| `fix/` | Bug fixes | `fix/fragment-deadline-arithmetic` |
| `perf/` | Performance improvements | `perf/burst-send-batching` |
| `refactor/` | Refactors with no behavior change | `refactor/extract-abr-ramp` |
| `chore/` | Dependency bumps, CI, tooling | `chore/bump-quinn-0.12` |
| `docs/` | Documentation only | `docs/rfc-0003-audio` |
| `release/` | Release stabilization | `release/0.2` |
| `security/` | Security fixes (may be private until patch released) | `security/hkdf-info-fix` |

### 19.3 Rules

- Feature branches live for at most 2 weeks. Branches older than 2 weeks without a PR
  are automatically closed by a GitHub Action with a comment.
- No direct commits to `main`. All changes via pull request.
- Force-push is prohibited on `main` and `release/*` branches (branch protection).
- Release branches accept only cherry-picked bug fixes from main — no new features.

---

## 20. Commit Conventions

### 20.1 Conventional Commits

Renderd uses **Conventional Commits** (https://www.conventionalcommits.org) enforced by
`commitlint` in CI.

Format:
```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### 20.2 Types

| Type | Use for |
|------|---------|
| `feat` | A new feature visible to users or downstream callers |
| `fix` | A bug fix |
| `perf` | A performance improvement (must cite benchmark numbers) |
| `refactor` | Code change that neither adds a feature nor fixes a bug |
| `docs` | Documentation only |
| `test` | Adding or fixing tests |
| `chore` | Dependency bumps, CI changes, build system |
| `ci` | Changes to GitHub Actions workflows |
| `security` | Security fixes (use with care; see SECURITY.md) |
| `revert` | Reverts a prior commit |

### 20.3 Scopes

Scope is the affected crate or system component:

```
feat(renderd-frame): add configurable window depth via const generic
fix(renderd-net): prevent burst-send starvation under high load
perf(renderd-abr): reduce allocation in BandwidthEstimator::update
docs(renderd-crypto): add usage example for derive_pair_token
chore(deps): bump quinn from 0.11.2 to 0.11.3
ci: add typos spell-checking workflow
```

Valid scopes: `renderd-proto`, `renderd-config`, `renderd-frame`, `renderd-crypto`,
`renderd-vt-sys`, `renderd-sc-sys`, `renderd-net`, `renderd-keychain`,
`renderd-discovery`, `renderd-abr`, `renderd-clock`, `renderd-host`, `renderd-viewer`,
`deps`, `ci`, `docs`, `release`.

### 20.4 Breaking Changes

Breaking changes (protocol changes, public API removals) are marked:

```
feat(renderd-proto)!: add min_required_version to SessionHello

Adds min_required_version field (field 2) to SessionHello. Hosts receiving
a SessionHello where min_required_version > host version must respond with
Error::VERSION_INCOMPATIBLE and close the connection.

BREAKING CHANGE: Viewer v0.2+ cannot connect to host v0.1 (missing field handling).
Pair both sides simultaneously when upgrading.
```

The `!` after the scope and the `BREAKING CHANGE:` footer both trigger MAJOR version
bump in automated versioning tools.

### 20.5 Commit Message Body

The body is required when:
- The change affects latency (explain the mechanism and cite numbers)
- The change is a bug fix (explain the root cause and the fix)
- The change is non-obvious (explain the design decision)

The body is optional for:
- Pure dependency bumps
- Documentation changes
- Test additions for existing behavior

---

## 21. Contribution Guide

*This section is also published as `CONTRIBUTING.md` at the repository root.*

### 21.1 Before You Start

Read the following documents in order before writing any code:

1. **RFC-0002** (`docs/RFC-0002-architecture.md`) — the canonical architecture.
   Understand every design decision and why it was made.
2. **REPO-0001** (this document) — engineering standards. Every standard here is
   enforced mechanically; understanding them prevents rejected PRs.
3. **Open issues** — check whether your planned contribution addresses an open issue
   or is a duplicate. If no issue exists, open one describing the change and wait for
   a maintainer to label it `accepted` before implementing.

### 21.2 Setting Up the Development Environment

**macOS (host development):**
```bash
# Install Rust via rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# The toolchain is pinned in rust-toolchain.toml; rustup will use it automatically
cd renderd
cargo build --package renderd-frame  # start with a leaf crate to verify setup

# Install required tools
cargo install cargo-nextest --locked
cargo install cargo-llvm-cov --locked
cargo install cargo-deny --locked
cargo install typos-cli --locked
```

**Windows (viewer development):**
```powershell
# Install Rust via rustup
winget install Rustlang.Rustup

# Install Visual Studio Build Tools with C++ workload (required for windows-rs)
winget install Microsoft.VisualStudio.2022.BuildTools

# Verify setup
cargo build --package renderd-viewer
```

**Cross-platform (library crate development):**
Library crates (`renderd-frame`, `renderd-crypto`, `renderd-abr`, etc.) compile on
any platform. Develop them on whichever OS you prefer.

### 21.3 Development Workflow

```bash
# 1. Create a branch
git checkout -b feat/your-feature-name

# 2. Make changes

# 3. Format (required before commit)
cargo fmt

# 4. Lint (fix all warnings; CI fails on any warning)
cargo clippy --all-targets -- -D warnings

# 5. Test the affected crate
cargo nextest run --package renderd-frame

# 6. Run benchmarks if you touched a hot path
cargo bench --package renderd-frame -- reassembly

# 7. Update documentation if you changed public API
cargo doc --package renderd-frame --no-deps --open

# 8. Commit following Conventional Commits format
git commit -m "feat(renderd-frame): add configurable window depth"

# 9. Push and open a PR
git push origin feat/your-feature-name
```

### 21.4 Pull Request Requirements

A PR is ready for review when all of the following are true:

- [ ] The PR description explains **what** changed and **why** (not just **how**)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -D warnings` passes for all affected crates
- [ ] All existing tests pass (`cargo nextest run`)
- [ ] New behavior is covered by new tests
- [ ] `cargo deny check` passes
- [ ] If the data plane was changed: benchmark numbers are included in the PR description
- [ ] If public API was changed: `cargo doc` builds cleanly and new items are documented
- [ ] If a protocol message was changed: RFC-0002 or a new RFC is updated
- [ ] CHANGELOG.md has an entry under `[Unreleased]`

### 21.5 PR Description Template

```markdown
## What

A brief description of the change.

## Why

Why is this change needed? Which issue does it address?

Closes #<issue-number>

## How

Explanation of the implementation approach. Why this approach over alternatives?

## Benchmark Impact

(Required for data-plane changes)
Before: bench_reassembly_burst (55 frags): 18.2 µs ± 0.3 µs
After:  bench_reassembly_burst (55 frags): 16.8 µs ± 0.2 µs
Delta: -7.7% (improvement)

## Checklist

- [ ] Tests added / updated
- [ ] Documentation updated
- [ ] CHANGELOG.md updated
- [ ] cargo fmt passes
- [ ] cargo clippy passes
- [ ] cargo deny check passes
```

### 21.6 What Not to Contribute

The following contributions will not be accepted:

- **Dependencies that are not MIT or Apache-2.0 licensed.** Run `cargo deny check`
  before adding any dependency.
- **`unsafe` code outside of `renderd-vt-sys` and `renderd-sc-sys`.** Every proposed
  use of `unsafe` elsewhere requires a maintainer design discussion issue first.
- **Changes to the cryptographic protocol** (SPAKE2+ implementation, HKDF info strings,
  certificate derivation) without an accompanying updated RFC and a cryptography
  specialist review.
- **Silent panics or `unwrap()`s in library code.** All fallible operations must return
  `Result`. This is enforced by CI and code review.
- **New external dependencies for functionality already provided by an existing
  workspace dependency.** Check the workspace `[dependencies]` before adding.
- **Features that belong to a future version.** Check RFC-0002 §20 (Future Work).
  If a feature is listed there, implementing it now creates scope creep and
  diverges from the current RFC. Open a discussion issue first.

### 21.7 Reporting Security Vulnerabilities

Do **not** open a public GitHub issue for security vulnerabilities. Follow the process
in `SECURITY.md`:

1. Email `security@renderd.dev` with subject: `[SECURITY] <brief description>`.
2. Use the PGP key published at `https://renderd.dev/security.asc`.
3. Include: affected version(s), reproduction steps, and your assessment of severity.
4. Maintainers will respond within 48 hours with a remediation timeline.
5. CVE assignment and public disclosure occur after the patch is released.

### 21.8 Code of Conduct

Renderd adopts the Contributor Covenant v2.1. All contributors are expected to abide
by it. Reports of violations go to `conduct@renderd.dev`.

---

## Appendix A: Crate Summary Table

| Crate | Layer | Platform | LOC estimate | Primary author |
|-------|-------|----------|-------------|----------------|
| `renderd-proto` | Foundation | All | ~500 | Protocol team |
| `renderd-config` | Foundation | All | ~600 | Any |
| `renderd-frame` | Primitive | All | ~800 | Protocol team |
| `renderd-crypto` | Primitive | All | ~1,200 | Crypto reviewer |
| `renderd-vt-sys` | FFI | macOS only | ~1,500 | macOS team |
| `renderd-sc-sys` | FFI | macOS only | ~800 | macOS team |
| `renderd-net` | Service | All | ~900 | Protocol team |
| `renderd-keychain` | Service | macOS + Win | ~600 | Platform teams |
| `renderd-discovery` | Service | macOS + Win | ~700 | Platform teams |
| `renderd-abr` | Algorithm | All | ~700 | Algorithms team |
| `renderd-clock` | Algorithm | All | ~600 | Algorithms team |
| `renderd-host` | Application | macOS only | ~3,000 | macOS team |
| `renderd-viewer` | Application | Windows only | ~3,500 | Windows team |

---

## Appendix B: First Implementation Order

The dependency graph imposes a natural implementation order. Engineers can begin work
on crates at the same layer in parallel; work on higher layers cannot begin until
lower-layer crates expose a stable `pub` API.

```
Wave 1 (parallel):  renderd-proto, renderd-config
Wave 2 (parallel):  renderd-frame, renderd-crypto
Wave 3 (parallel):  renderd-vt-sys, renderd-sc-sys, renderd-net, renderd-keychain, renderd-discovery
Wave 4 (parallel):  renderd-abr, renderd-clock
Wave 5 (parallel):  renderd-host, renderd-viewer
```

Within each wave, the latency-bench tool and protocol simulation tests should be
developed alongside the crates they depend on, not deferred to the end.

---

*End of REPO-0001*
