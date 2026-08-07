# Changelog

All notable changes to Renderd will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

> Pre-release hardening, cross-platform Windows viewer integration, and input forwarding in progress.

### Changed
- **Workspace version** bumped from `0.1.0` to `0.9.0-integration` to align `cargo metadata` with the
  CHANGELOG milestone tracking and release tag convention.
- **README.md** corrected: workspace member count (`16` → `15`), and the CI section now accurately
  lists each job with its runner OS rather than claiming a generic "Linux/macOS/Windows" matrix.
- **CI (`ci.yml`)**: Added JUnit XML artifact upload after nextest runs on both macOS and Windows
  jobs (enables GitHub Actions test summary UI). Added a new `deny` job so `cargo deny check`
  runs on every PR in addition to the weekly `security.yml` schedule.
- **Security (`security.yml`)**: Added documentation comment explaining the relationship between
  the per-PR `deny` job in `ci.yml` and the weekly advisory-db drift check in `security.yml`.

### Documentation
- **`docs/BENCHMARK-ROADMAP.md`**: Replaced stale Gantt chart (dates in the past) with an accurate
  status table distinguishing implemented benchmarks from hardware-gated deferred benchmarks.
  Added rationale for deferrals (live display session / GPU runner requirements).

### Tests
- **`renderd-crypto`**: Expanded unit tests from 1 to 9. New tests cover UUID cross-isolation
  (viewer UUID, host UUID), adjacent PIN isolation, output length invariants, non-zero output
  guarantee, and explicit `Zeroize::zeroize()` invocation for both `PairToken` and `SessionKey`.
- **`renderd-keychain`**: Expanded `MockKeychain` tests from 1 to 7. New tests cover
  `NotFound` error paths (load-missing, delete-missing, double-delete), overwrite semantics
  (save-twice deduplicates), empty-store list, multi-entry isolation, and `Arc<MockKeychain>`
  shared access pattern.

---

## [0.9.0-integration] — 2026-08-07

> Milestone 9 complete: First End-to-End macOS Display Streaming Milestone.

### Added / Milestone Achievements
- **First End-to-End macOS Display Streaming**: Achieved fully operational live desktop streaming from host capture to viewer presentation.
- **Hardware Decoder (`VideoToolboxDecoder`) Integration**: Implemented `renderd_VTDecompressionSessionCreateFromNAL` and `renderd_VTDecompressionSessionDecodeFrame` in `renderd-vt-sys`, replacing the placeholder decoder on macOS.
- **Bitstream NAL & Format Description Handling**: Added automated VPS (NAL 32), SPS (NAL 33), and PPS (NAL 34) parameter set extraction for HEVC streams to generate valid `CMVideoFormatDescriptionRef` objects for VideoToolbox decompression sessions.
- **Sample Buffer & Length Prefix Conversion**: Implemented Annex-B startcode conversion (`0x00000001` → 4-byte big-endian NAL length prefixes) matching VideoToolbox HVCC container expectations for hardware decompression.
- **Live Desktop Presentation**: Verified zero-copy `ScreenCaptureKit` capture, `VideoToolbox` HEVC hardware encoding, QUIC datagram transmission, sliding-window reassembly, `VideoToolbox` hardware decoding, BGRA pixel buffer extraction, and continuous rendering via `SoftRenderer`.

### Next Steps & Upcoming Roadmap
- **Cross-Platform Support**: Integrate Windows D3D12/MediaFoundation hardware decoding & swapchain presentation with macOS host daemon.
- **Input Forwarding**: Mouse, keyboard, and touch event injection from viewer to host.
- **Audio & Clipboard**: CoreAudio / WASAPI audio streaming and bidirectional pasteboard sync.
- **NAT Traversal**: STUN/TURN/ICE signaling fallbacks for WAN connections.

> Milestone 8 complete: Viewer Application (`renderd-viewer`).

### Added
- Scaffolded `renderd-viewer` application with native `winit` event loop, borderless
  fullscreen window management, and Per-Monitor v2 DPI awareness on Windows. (#089)
- Implemented DXGI Allow-Tearing feature check (`check_tearing_support()`) for safe swap
  chain creation. (#090)
- Implemented D3D12 swap chain and renderer with YUV-to-RGB HLSL pixel shader and
  conditional `DXGI_PRESENT_ALLOW_TEARING` presentation. (#091)
- Implemented `ID3D12VideoDecoder` hardware H.265/H.264 video decoder integration with
  GPU VRAM surface output. (#092)
- Implemented QUIC datagram receiver and sliding-window `ReassemblyWindow` task forwarding
  completed frames to the decoder. (#093)
- Implemented DWM vsync phase reporter querying `DwmGetCompositionTimingInfo` and
  transmitting `VsyncReport` over QUIC Stream 0. (#094)
- Implemented dual-timescale ABR feedback exporter: `ReactiveStats` at 100 ms,
  `PeriodicStats` at 500 ms, and immediate `KeyframeRequest` on loss. (#095)
- Implemented viewer pairing UI with SPAKE2+ prover handshake over QUIC Stream 0,
  storing derived `PairToken` in Windows Credential Manager on success. (#096)
- Implemented reconnect watchdog with mDNS re-discovery filtering by stored `host_uuid`
  for automatic recovery on host IP change. (#097)
- Implemented semi-transparent `Reconnecting...` status UI overlay during disconnect
  state without closing window. (#098)
- Implemented Windows system tray icon via `Shell_NotifyIcon` with context menu:
  "Connect to Host...", "Settings", "Disconnect", "Exit". (#099)
- Added Windows viewer release packaging CI workflow on `windows-2025` runner
  (`release-viewer.yml`) producing standalone installer artifact. (#100)
- Wired `HostApp::run()` with full subsystem initialization (capture, encode, clock,
  ABR, session, network, UI) and persistent SIGINT/SIGTERM signal handler loop
  replacing placeholder print-and-exit entrypoint. (#101)

---

## [0.7.0-host] — 2026-08-05

> Milestone 7 complete: Host Application (`renderd-host`).

### Added
- Scaffolded `renderd-host` application crate with `Info.plist`, entitlements, and macOS
  app bundle layout. (#076)
- Added macOS app bundle packaging script (`tools/bundle-host/assemble.sh`). (#077)
- Implemented app startup, CLI argument parsing, Figment config loading, and structured
  panic hook via `tracing-subscriber`. (#078)
- Implemented macOS login item auto-start using `SMAppService.mainApp` with enable,
  disable, and status query. (#079)
- Implemented `HostSession` state machine (`IDLE → PAIRING → CONNECTED → STREAMING`)
  with typed `SessionError` transitions. (#080)
- Implemented pairing handler with 6-digit PIN generation, SPAKE2+ verifier protocol,
  exponential failure lockout, and keychain storage. (#081)
- Implemented `DeviceRegistry` for paired viewer listing and keychain revocation. (#082)
- Implemented `CapturePipeline` and `EncodePipeline` wiring ScreenCaptureKit frames
  directly to VideoToolbox encoder via lock-free SPSC ring buffer. (#083)
- Implemented datagram burst sender task fragmenting NAL units and sending non-yielding
  QUIC datagram bursts per frame. (#084)
- Connected `ClockController` and `AbrManager` to host control loop processing
  `VsyncReport`, `ReactiveStats`, and `PeriodicStats` messages. (#085)
- Implemented macOS menu bar UI with status icon, pairing PIN display, paired device
  list, and quit action via `MenuBar`. (#086)
- Implemented `NotificationManager` posting macOS user notifications on session start
  and device pairing events. (#087)
- Added host release packaging CI workflow (`release-host.yml`) with code signing,
  notarization, stapling, and DMG artifact creation. (#088)

---

## [0.6.0-algorithms] — 2026-08-04

> Milestone 6 complete: Algorithm Layer (ABR & Clock Sync).

### Added
- Scaffolded `renderd-abr` crate with pure `RampPolicy` step-up/step-down bitrate math
  clamped to [5 Mbps, 50 Mbps]. (#064)
- Implemented `BandwidthEstimator` exponential moving average filter for receive bandwidth
  samples. (#065)
- Implemented `AbrEngine` dual-timescale state machine: reactive (100 ms, loss-triggered)
  and proactive (500 ms, bandwidth-degradation) bitrate adjustment. (#066)
- Added `proptest` property tests asserting `AbrEngine` bitrate decisions remain in
  configured bounds for arbitrary inputs. (#067)
- Added Criterion benchmark for `on_reactive` and `on_proactive` ABR decision calls. (#068)
- Scaffolded `renderd-clock` crate with `ClockOffset::compute()` for host/viewer clock
  domain translation using QUIC RTT. (#069)
- Implemented `JitterFilter` median-window outlier rejection for vsync phase reports. (#070)
- Implemented `ClockSync` state machine with 30-frame warmup, phase tracking, and
  capture pacing output. (#071)
- Added `proptest` property tests asserting pacing interval is bounded in [8 ms, 33 ms]. (#072)
- Added Criterion benchmark for `on_vsync_report` and `next_capture_time`. (#073)
- Created `tools/latency-bench` CLI skeleton with `--frames`, `--resolution`, and
  `--codec` flag support. (#074)
- Implemented pipeline microsecond stage telemetry with p50/p95/p99/max latency
  reporting across all pipeline stages. (#075)

---

## [0.5.0-services] — 2026-08-04

> Milestone 5 complete: Service Layer (Net, Keychain & Discovery).

### Added
- Implemented `ServerTlsConfig` and `ClientTlsConfig` with strict mutual TLS 1.3 and
  pinned certificate verification in `renderd-net`. (#048)
- Implemented `QuicServer` and QUIC connection listener in `renderd-net`. (#049)
- Implemented `QuicClient` with QUIC connection initiation and TLS handshake in
  `renderd-net`. (#050)
- Implemented 4-byte length-prefixed control stream framing (`send_control` /
  `recv_control`) in `renderd-net`. (#051)
- Implemented non-yielding `FragmentBurst::send_all()` datagram burst sender in
  `renderd-net`. (#052)
- Implemented RTT telemetry exporter via `quinn::ConnectionStats` in `renderd-net`. (#053)
- Scaffolded `renderd-keychain` crate with `KeychainStore` trait and `PairingEntry`
  struct. (#054)
- Implemented macOS Keychain Services backend (`MacosKeychain`) using
  `kSecClassGenericPassword`. (#055)
- Implemented Windows Credential Manager backend (`WindowsCredentialManager`) using
  `CredWrite` / `CredRead` / `CredDelete`. (#056)
- Implemented `MockKeychain` in-memory store for headless testing. (#057)
- Scaffolded `renderd-discovery` crate with `Advertiser` and `Browser` traits and
  `ServiceRecord` struct. (#058)
- Implemented macOS Bonjour discovery backend using `dns_sd.h` bindings. (#059)
- Implemented Windows Win32 mDNS backend using `DnsServiceRegister` / `DnsServiceBrowse`. (#060)
- Implemented `ManualBrowser` static IP fallback for corporate network environments. (#061)
- Implemented `DiscoveryError` hierarchy (`BindFailed`, `ServiceRegistrationFailed`,
  `BrowseFailed`). (#062)
- Implemented `MockConnection` in-memory transport for integration testing. (#063)

---

## [0.4.0-ffi] — 2026-08-04

> Milestone 4 complete: FFI Layer (VideoToolbox & ScreenCaptureKit shims).

### Added
- Scaffolded `renderd-vt-sys` crate with `build.rs` linking VideoToolbox, CoreMedia, and
  CoreFoundation frameworks. (#036)
- Implemented `renderd_VTCompressionSessionCreate` C bridge shim for Rust FFI callback
  compatibility. (#037)
- Implemented `CompressionSession` safe Rust wrapper with H.265 real-time hardware encoder
  initialization and `Drop` cleanup. (#038)
- Implemented dynamic bitrate control (`set_bitrate`) and force-keyframe trigger in
  `CompressionSession`. (#039)
- Implemented `IOSurface` Rust wrapper with `CFRetain`/`CFRelease` lifetime management. (#040)
- Implemented `VtError` OSStatus decoder with human-readable error messages. (#041)
- Scaffolded `renderd-sc-sys` crate with ObjC2 ScreenCaptureKit framework imports. (#042)
- Implemented `ScreenRecordingPermission::check()` TCC authorization query. (#043)
- Implemented `ContentFilter::main_display()` display enumeration and selection. (#044)
- Implemented `ScreenStream` wrapper with GPU-resident frame callback on
  `QOS_CLASS_USER_INTERACTIVE` GCD queue. (#045)
- Implemented dynamic `minimumFrameInterval` vsync pacing controls on `ScreenStream`. (#046)
- Implemented `ScError` hierarchy (`PermissionDenied`, `NoDisplaysFound`,
  `StreamFailed`). (#047)

---

## [0.3.0-primitives] — 2026-08-03

> Milestone 3 complete: Primitive Layer (Frame & Crypto).

### Added
- Added 16-byte fixed binary fragment header codec (`FragmentHeader`) in `renderd-frame`
  crate per RFC-0002 §12.1. (#023)
- Added type-safe fragment header bitfield flags (`FragmentFlags`) in `renderd-frame`
  crate. (#024)
- Added `Fragment` and `CompleteFrame` types in `renderd-frame` crate. (#025)
- Added sliding-window fragment reassembly buffer state machine (`ReassemblyWindow`) in
  `renderd-frame` crate per RFC-0002 §12.2. (#026)
- Added dynamic fragment deadline computer (`DeadlineComputer`) in `renderd-frame`. (#027)
- Added `proptest` property tests for reassembly window safety under arbitrary reordering. (#028)
- Added Criterion benchmark for reassembly window throughput. (#029)
- Scaffolded `renderd-crypto` crate with `PairToken` and `SessionKey` types with
  `Zeroize` on drop. (#030)
- Added RFC 9382 SPAKE2+ test vectors as unit test suite. (#031)
- Implemented SPAKE2+ `Prover` and `Verifier` state machines for P-256 pairing. (#032)
- Implemented HKDF-SHA256 key derivation helpers (`derive_pair_token`,
  `derive_session_key`) with canonical info strings. (#033)
- Implemented TLS certificate generator (`generate_cert`) using `rcgen` with pair token
  derived key material. (#034)
- Added Criterion benchmarks for crypto operations (`derive_pair_token`,
  `generate_cert`). (#035)

---

## [0.2.0-foundation] — 2026-08-03

> Milestone 2 complete: Foundation Layer.

### Added
- Added Protobuf protocol schema definition in `proto/renderd.proto` per RFC-0002 §11.
  (#013)
- Added `tools/proto-gen` code generator and generated `prost` Rust types in
  `renderd-proto` crate. (#014)
- Added protocol domain newtypes (`FrameId`, `FragmentId`, `BitrateKbps`,
  `VsyncPeriodNs`, `ViewerId`, `HostId`) in `renderd-proto` crate. (#015)
- Added envelope dispatch helpers (`MessageKind`) and message validation logic for
  `SessionHello` and `SessionConfig` in `renderd-proto` crate. (#016)
- Enabled proto freshness CI check step in `.github/workflows/ci.yml`. (#017)
- Added configuration schema structs (`RenderdConfig`, `HostConfig`, `ViewerConfig`,
  `NetworkConfig`, `CryptoConfig`, `AbrConfig`) in `renderd-config` crate. (#018)
- Added Figment-based layered config loader (`ConfigBuilder`) supporting defaults, TOML
  files, environment variables, and CLI overrides in `renderd-config` crate. (#019)
- Added configuration validation rules (`ValidateConfig` trait) for `RenderdConfig` in
  `renderd-config` crate. (#020)
- Added `ConfigError` enum hierarchy (`FileNotFound`, `ParseError`, `ValidationError`)
  and `Display` implementations in `renderd-config` crate. (#021)
- Added canonical host and viewer default configuration templates
  (`renderd-host.default.toml`, `renderd-viewer.default.toml`) and template validation
  integration tests. (#022)

---

## [0.1.0-bootstrap] — 2026-08-03

> Milestone 1 complete: Repository Bootstrap & Infrastructure.

### Added
- Initialized Cargo workspace root with resolver v2 and declared all 15 member crates
  across 5 DAG layers (`crates/renderd-*`, `tools/latency-bench`, `tools/proto-gen`)
  per REPO-0001. (#001)
- Added `rust-toolchain.toml` pinning Rust stable channel (MSRV 1.80) with
  `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, and `aarch64-pc-windows-msvc`
  target support. (#001)
- Configured workspace-level dependency versions, lint policies, and build profiles
  (`dev`, `release`, `bench`). (#001)
- Configured workspace-level `clippy.toml` setting `msrv = \"1.80\"`, disallowing
  `std::process::exit` and `std::env::var`, and restricting raw array pair-tokens.
  (#002)
- Added per-crate lint overrides: `unsafe_code = \"deny\"` for non-FFI crates;
  `unsafe_code = \"warn\"` for FFI crates (`renderd-vt-sys`, `renderd-sc-sys`) per
  REPO-0001 §9. (#002)
- Added root `.rustfmt.toml` workspace formatting configuration per REPO-0001 §10.
  (#003)
- Configured `cargo-deny` policy in `deny.toml` for license checking, dependency bans,
  and security advisory checking per REPO-0001 §9. (#004)
- Added test runner configuration in `nextest.toml` per REPO-0001 §14.7. (#005)
- Added primary CI workflow in `.github/workflows/ci.yml` per REPO-0001 §18.1. (#006)
- Added security audit workflow in `.github/workflows/security.yml` per
  REPO-0001 §18.2. (#007)
- Added nightly benchmark workflow in `.github/workflows/bench.yml` per
  REPO-0001 §18.3. (#008)
- Added code ownership rules in `.github/CODEOWNERS` per REPO-0001 §7.1. (#009)
- Added PR template and GitHub Issue forms for bug reports, latency regressions, and
  feature requests. (#010)
- Added spell-checking configuration in `_typos.toml` and
  `.github/workflows/typos.yml`. (#011)
- Added documentation deployment workflow in `.github/workflows/docs.yml` per
  REPO-0001 §18.5. (#012)

---

[Unreleased]: https://github.com/Ad1th/renderd/compare/v0.9.0-integration...HEAD
[0.9.0-integration]: https://github.com/Ad1th/renderd/compare/v0.8.0-viewer...v0.9.0-integration
[0.8.0-viewer]: https://github.com/Ad1th/renderd/compare/v0.7.0-host...v0.8.0-viewer
[0.7.0-host]: https://github.com/Ad1th/renderd/compare/v0.6.0-algorithms...v0.7.0-host
[0.6.0-algorithms]: https://github.com/Ad1th/renderd/compare/v0.5.0-services...v0.6.0-algorithms
[0.5.0-services]: https://github.com/Ad1th/renderd/compare/v0.4.0-ffi...v0.5.0-services
[0.4.0-ffi]: https://github.com/Ad1th/renderd/compare/v0.3.0-primitives...v0.4.0-ffi
[0.3.0-primitives]: https://github.com/Ad1th/renderd/compare/v0.2.0-foundation...v0.3.0-primitives
[0.2.0-foundation]: https://github.com/Ad1th/renderd/compare/v0.1.0-bootstrap...v0.2.0-foundation
[0.1.0-bootstrap]: https://github.com/Ad1th/renderd/releases/tag/v0.1.0-bootstrap
