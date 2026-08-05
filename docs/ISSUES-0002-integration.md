# ISSUES-0002: End-to-End Integration & System Validation Roadmap

```
Title:      Renderd — Integration & System Validation Roadmap
Doc:        ISSUES-0002
Status:     Active
Applies:    renderd-host, renderd-viewer, and all workspace crates
Created:    2026-08-06
Refs:       RFC-0002-architecture.md, REPO-0001-repository.md, ISSUES-0001-milestones.md
Total:      18 integration issues across 4 phases (#101–#118)
```

---

## Overview & Integration Roadmap

This document defines **Milestone 9: End-to-End Integration & System Validation** for the Renderd codebase.

With Milestones 1–8 complete (Issues #001–#100), all individual subsystem crates, platform FFI bridges, algorithms, and application entrypoints are fully implemented and verified via unit and component test suites (136 passing tests). 

The objective of Milestone 9 is **wiring, cross-subsystem integration, runtime validation, and performance verification**. No new architecture or helper crates are created in this milestone. Engineering effort is focused strictly on connecting existing components into a live, low-latency, peer-to-peer screen streaming daemon between macOS (`renderd-host`) and Windows 10+ (`renderd-viewer`).

```
┌────────────────────────────────────────────────────────────────────────┐
│ Phase 1: Service Layer Integration & Peer Discovery  (Issues #101–#104)│
├────────────────────────────────────────────────────────────────────────┤
│ Phase 2: Session Lifecycle & Media Pipeline Wiring   (Issues #105–#109)│
├────────────────────────────────────────────────────────────────────────┤
│ Phase 3: Control Loops & Telemetry Feedback          (Issues #110–#113)│
├────────────────────────────────────────────────────────────────────────┤
│ Phase 4: Validation, Fault Testing & Performance     (Issues #114–#118)│
└────────────────────────────────────────────────────────────────────────┘
```

---

## Phase 1: Service Layer Integration & Peer Discovery (Issues #101–#104)

Connects background discovery, network socket listening, control stream transport, and secure pairing between host and viewer daemons.

---

### Issue #101: Host Server Startup & mDNS Service Advertisement Wiring
- **Rationale:** `renderd-host` currently initializes its subsystem structs and blocks on signal shutdown, but does not launch `HostServer` (QUIC endpoint listening) or `BonjourAdvertiser` (mDNS service registration) upon startup.
- **Dependencies:** #049, #059, #078, #101
- **Acceptance Criteria:**
  - `HostApp::run()` spawns `HostServer` listening on the configured UDP port (default `9000`).
  - Spawns `BonjourAdvertiser` registering `_renderd._udp.local.` with `host_id`, display resolution, and listening port in TXT records.
  - Shutting down `renderd-host` via SIGINT unregisters mDNS advertisement and closes the QUIC socket cleanly.
- **Testing:** Running `cargo run -p renderd-host` starts QUIC listener and `dns-sd -B _renderd._udp` on macOS discovers the advertised host.
- **Estimated Effort:** 4 hours

---

### Issue #102: Viewer mDNS Discovery & Host Address Resolution Integration
- **Rationale:** `renderd-viewer` must automatically discover active hosts on the local network via mDNS browsing and present discovered hosts in the UI target list.
- **Dependencies:** #060, #089, #101
- **Acceptance Criteria:**
  - `renderd-viewer` launches `WinDnsBrowser` (or `BonjourBrowser` on macOS testing) on startup.
  - Discovered `ServiceRecord` items populate the host target list in the system tray menu (`SettingsMenu`) and UI overlay (`Overlay`).
  - Manual IP entry (`ManualBrowser`) remains available as a fallback when mDNS multicast is suppressed.
- **Testing:** Running `renderd-viewer` while `renderd-host` is active automatically populates the host target in the UI.
- **Estimated Effort:** 4 hours

---

### Issue #103: QUIC Control Stream 0 Handshake Integration
- **Rationale:** Establishing a QUIC connection between viewer and host requires opening Stream 0 and exchanging length-prefixed `Envelope` protobuf messages (`SessionHello`, `SessionConfig`).
- **Dependencies:** #050, #051, #101, #102
- **Acceptance Criteria:**
  - Selecting a discovered host in `renderd-viewer` initiates `QuicClient::connect()`.
  - Stream 0 is opened upon connection establishment; client sends `SessionHello`.
  - Host `ControlDispatcher` receives `SessionHello`, validates protocol version, and replies with `SessionConfig`.
- **Testing:** Integration test between viewer and host verifies successful Stream 0 framing and message exchange.
- **Estimated Effort:** 5 hours

---

### Issue #104: SPAKE2+ PIN Pairing End-to-End Handshake & Keychain Persistence
- **Rationale:** First-time connections require interactive 6-digit PIN pairing over SPAKE2+ P-256 to establish mutual authentication and store the derived `PairToken`.
- **Dependencies:** #032, #055, #056, #081, #096, #103
- **Acceptance Criteria:**
  - Host `PairingHandler` displays a 6-digit PIN in the macOS menu bar and posts a notification (`NotificationManager`).
  - Viewer `PairingUi` prompts the user for PIN entry and executes the `Spake2Prover` exchange over Stream 0 against `Spake2Verifier`.
  - Upon successful pairing, derived `PairToken` is saved to macOS Keychain (`MacosKeychain`) and Windows Credential Manager (`WindowsCredentialManager`).
  - Incorrect PIN entry triggers failure alert and increments lockout attempt counter.
- **Testing:** End-to-end pairing sequence between host and viewer succeeds with matching PIN and fails with wrong PIN.
- **Estimated Effort:** 6 hours

---

## Phase 2: Session Lifecycle & Media Pipeline Wiring (Issues #105–#109)

Connects session state transitions, screen capture, hardware encoding, datagram transmission, datagram reception, sliding-window reassembly, decoding, and rendering into a continuous frame presentation loop.

---

### Issue #105: Host Session State Transition Wiring (`Idle` → `Pairing` → `Connected` → `Streaming`)
- **Rationale:** Host lifecycle state transitions must be driven by authenticated control plane messages and reflected in host state machine and UI indicators.
- **Dependencies:** #080, #086, #103, #104
- **Acceptance Criteria:**
  - Stream 0 handshake transitions `HostSession` from `Idle` to `Connected`.
  - Viewer streaming request transitions `HostSession` to `Streaming`.
  - Disconnect or stream stop command resets `HostSession` back to `Idle`.
  - Status updates reflect in macOS menu bar item title and icon state.
- **Testing:** State transitions logged with structured `tracing` events and match RFC-0002 §9 lifecycle state machine.
- **Estimated Effort:** 4 hours

---

### Issue #106: Host Capture & VideoToolbox Hardware Encoding Loop Activation
- **Rationale:** When transitioning to `Streaming`, `renderd-host` must activate ScreenCaptureKit frame capture and wire `IOSurface` callbacks directly into VideoToolbox encoder.
- **Dependencies:** #038, #045, #083, #105
- **Acceptance Criteria:**
  - `HostSession::begin_streaming()` calls `CapturePipeline::start()`.
  - `SCStream` callbacks deliver GPU `IOSurface` frames directly to `EncodePipeline::encode_surface()` at target FPS (e.g., 60 FPS).
  - Encoded H.265 NAL units are pushed into the capacity-4 SPSC lock-free ring buffer.
  - `CapturePipeline::stop()` halts screen capture when session stops.
- **Testing:** Host logs confirm continuous frame capture and encoding at 60 FPS without ring buffer overflows.
- **Estimated Effort:** 6 hours

---

### Issue #107: Datagram Burst Sender & QUIC Datagram Channel Integration
- **Rationale:** Encoded NAL units in the host ring buffer must be fragmented into 16-byte datagram headers and transmitted over the active QUIC connection's datagram channel.
- **Dependencies:** #024, #052, #084, #106
- **Acceptance Criteria:**
  - `DataSender` task polls the `EncodePipeline` receiver channel.
  - Each frame payload is fragmented into datagrams with packed `FragmentHeader` (frame ID, fragment ID, total fragments, flags, PTS offset).
  - `FragmentBurst::send_all()` sends datagram bursts in a non-yielding loop over `quinn::Connection::send_datagram()`.
- **Testing:** Wireshark / trace logs confirm QUIC datagram bursts leaving host socket on frame emission.
- **Estimated Effort:** 5 hours

---

### Issue #108: Viewer Datagram Receiver & Reassembly Window Pipeline Integration
- **Rationale:** `renderd-viewer` must receive incoming QUIC datagrams, parse headers, reassemble complete video frames using `ReassemblyWindow`, and push completed frames to the decoder queue.
- **Dependencies:** #026, #093, #107
- **Acceptance Criteria:**
  - `DataReceiver` task reads datagrams from `quinn::Connection::read_datagram()`.
  - Parses 16-byte `FragmentHeader` and inserts fragments into `ReassemblyWindow<4>`.
  - When all fragments for a frame arrive, `CompleteFrame` is emitted and pushed to `FrameQueue`.
  - Expired fragments older than sliding window bound are cleanly evicted.
- **Testing:** Viewer logs verify frame reassembly rate matching host transmit frame rate.
- **Estimated Effort:** 5 hours

---

### Issue #109: Viewer Video Decoder & Direct3D 12 Renderer Frame Presentation Pass
- **Rationale:** Completed video frames in `FrameQueue` must be decoded by `ID3D12VideoDecoder` into GPU surfaces and rendered to the window via D3D12 YUV-to-RGB swap chain presentation.
- **Dependencies:** #091, #092, #108
- **Acceptance Criteria:**
  - `D3D12Decoder` pops NAL units from `FrameQueue` and decodes them into NV12 GPU surfaces.
  - `D3D12Renderer` executes HLSL YUV-to-RGB pixel shader pass on output surface.
  - DXGI swap chain presents frame to `winit` borderless fullscreen window surface.
  - Displays first live mirrored desktop frame from host on viewer screen.
- **Testing:** Running host and viewer on local network renders live macOS host screen on Windows viewer display.
- **Estimated Effort:** 8 hours

---

## Phase 3: Control Loops & Telemetry Feedback (Issues #110–#113)

Integrates presentation clock vsync phase alignment, dual-timescale ABR feedback, keyframe recovery on loss, and automatic reconnection watchdogs.

---

### Issue #110: DWM Vsync Telemetry & Host Capture Pacing Integration
- **Rationale:** To eliminate frame stutter and tear-line artifacts, viewer DWM vsync timing telemetry must actively pace host screen capture.
- **Dependencies:** #071, #085, #094, #109
- **Acceptance Criteria:**
  - Viewer `VsyncReporter` queries `DwmGetCompositionTimingInfo` every frame and transmits `VsyncReport` protobuf message over Stream 0.
  - Host `ClockController` processes `VsyncReport` and computes optimal target capture timestamp.
  - Host calls `CapturePipeline::set_target_interval()` to adjust `minimumFrameInterval` dynamically.
- **Testing:** Vsync phase delta converges within ±2 ms of viewer vsync boundary after 30-frame warmup.
- **Estimated Effort:** 5 hours

---

### Issue #111: Dual-Timescale ABR Telemetry Feedback Loop Integration
- **Rationale:** Viewer network and decode telemetry must continuously inform host encoding bitrate to prevent buffer bloat and packet loss congestion.
- **Dependencies:** #066, #085, #095, #107
- **Acceptance Criteria:**
  - Viewer `FeedbackExporter` transmits `ReactiveStats` every 100 ms and `PeriodicStats` every 500 ms over Stream 0.
  - Host `AbrManager` processes reports: 10% packet loss reduces bitrate by 25%; severe loss triggers panic backoff.
  - Host calls `EncodePipeline::set_bitrate()` updating `VTCompressionSession` encoder bitrate dynamically.
- **Testing:** Simulated packet loss reduces encoder bitrate within 100 ms of loss detection.
- **Estimated Effort:** 5 hours

---

### Issue #112: Frame Loss Detection & Keyframe Request Control Loop
- **Rationale:** When datagram loss causes missing frame fragments that cannot be reassembled before deadline expiry, viewer must request an immediate IDR keyframe from host.
- **Dependencies:** #027, #085, #095, #108
- **Acceptance Criteria:**
  - Viewer `ReassemblyWindow` detects unrecoverable frame gap and emits `KeyframeRequest` control message over Stream 0.
  - Host receives `KeyframeRequest` and calls `EncodePipeline::force_keyframe()`.
  - VideoToolbox emits immediate IDR keyframe, restoring viewer decoder sync.
- **Testing:** Dropping a keyframe datagram sequence triggers immediate keyframe request and stream recovery within < 50 ms.
- **Estimated Effort:** 4 hours

---

### Issue #113: Reconnect Watchdog & mDNS Re-Discovery Integration
- **Rationale:** Network interruptions or host DHCP IP changes must not crash the viewer; the connection watchdog must auto-recover transparently.
- **Dependencies:** #097, #098, #103
- **Acceptance Criteria:**
  - QUIC connection drop triggers viewer `Watchdog` and displays `Overlay` ("Reconnecting...").
  - Viewer attempts cached IP connection once; if unreachable, initiates mDNS browsing filtered by stored `host_uuid`.
  - Upon discovering new host IP, viewer re-establishes QUIC session, authenticates via stored `PairToken`, and resumes video stream without user intervention.
- **Testing:** Changing host IP address during active streaming session results in automatic reconnection within < 3 seconds.
- **Estimated Effort:** 6 hours

---

## Phase 4: Validation, Fault Testing & Performance (Issues #114–#118)

Performs comprehensive multi-monitor validation, network fault injection, sub-30ms glass-to-glass latency verification, long-haul stability testing, and final smoke test signoff.

---

### Issue #114: Multi-Monitor Target Selection & Display Geometry Validation
- **Rationale:** Host machine may have multiple displays attached; user must be able to select which display is captured and mirrored.
- **Dependencies:** #044, #086, #106
- **Acceptance Criteria:**
  - macOS menu bar UI lists all active host displays.
  - Selecting a display updates `ContentFilter` in `CapturePipeline` dynamically without restarting daemon.
  - Viewer updates aspect ratio and window dimensions cleanly (`WM_DPICHANGED`).
- **Testing:** Host with primary retina screen and secondary 4K display correctly switches capture stream between displays.
- **Estimated Effort:** 4 hours

---

### Issue #115: Network Fault & Latency Degradation Resilience Testing
- **Rationale:** Validates system stability and graceful degradation under real-world imperfect Wi-Fi / Ethernet conditions.
- **Dependencies:** #111, #112, #113
- **Acceptance Criteria:**
  - System tested under simulated network conditions (5% loss, 15% loss, 50 ms jitter, bandwidth restriction to 10 Mbps) using `tools/latency-bench` and netem/network link conditioner.
  - Application does not panic, deadlock, or crash under any network condition.
  - ABR drops bitrate to min bound (5 Mbps); stream recovers when network clears.
- **Testing:** 15-minute fault injection test run completes cleanly without process aborts or thread deadlocks.
- **Estimated Effort:** 6 hours

---

### Issue #116: Sub-30ms Glass-to-Glass Latency Measurement & Telemetry Benchmarking
- **Rationale:** Verifies compliance with Renderd's core design goal: $\le 30 \text{ ms}$ glass-to-glass latency at 1080p60 over Gigabit LAN.
- **Dependencies:** #075, #109, #110
- **Acceptance Criteria:**
  - Execute benchmark runs using `tools/latency-bench` stage hooks across full host-to-viewer pipeline.
  - Verify stage breakdown targets (RFC-0002 §19):
    - Capture: $\le 5.0\text{ ms}$
    - Encode: $\le 8.0\text{ ms}$
    - Transport: $\le 8.0\text{ ms}$
    - Reassembly: $\le 2.0\text{ ms}$
    - Decode: $\le 7.0\text{ ms}$
    - Present: $\le 4.0\text{ ms}$
  - Total 95th percentile glass-to-glass latency measured $\le 30.0\text{ ms}$ @ 1080p60.
- **Testing:** `cargo run --manifest-path tools/latency-bench/Cargo.toml` emits passing latency report.
- **Estimated Effort:** 6 hours

---

### Issue #117: Continuous 1-Hour Long-Haul Stability & Memory Leak Audit
- **Rationale:** Ensures long-running display sessions do not accumulate heap allocations, open handles, or thread leaks over time.
- **Dependencies:** #106, #109, #111
- **Acceptance Criteria:**
  - Execute continuous 1-hour active streaming session at 1080p60 (216,000 frames).
  - RSS memory usage remains flat (variance $< 5\%$) across host and viewer processes.
  - Zero unhandled dropped frames, zero socket descriptor leaks, zero D3D12/Metal memory leaks.
- **Testing:** `valgrind` / `leaks` / Task Manager performance monitor confirms flat memory profile.
- **Estimated Effort:** 4 hours

---

### Issue #118: End-to-End Interactive System Smoke Test & Final Acceptance
- **Rationale:** Performs final user-level validation of the complete peer-to-peer display daemon ecosystem.
- **Dependencies:** #101–#117
- **Acceptance Criteria:**
  - Full end-to-end workflow executed:
    1. Start `renderd-host` on macOS Workstation.
    2. Start `renderd-viewer` on Windows 10+ PC.
    3. Auto-discover host via mDNS.
    4. Connect and complete 6-digit PIN SPAKE2+ pairing.
    5. Screen sharing session starts automatically; host desktop renders on viewer window at 60 FPS with sub-30ms latency.
    6. Resize viewer window / DPI scaling adjusts smoothly.
    7. Disconnect network cable -> "Reconnecting..." overlay -> reconnect cable -> auto-resume stream.
    8. Quit viewer -> host session resets to `Idle`.
- **Testing:** Manual end-to-end smoke test passes without errors, warnings, or performance hiccups.
- **Estimated Effort:** 4 hours

---

*End of ISSUES-0002 — Milestone 9 Roadmap*
