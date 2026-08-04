# RFC-0002: Renderd Architecture

```
Title:      Renderd — Peer-to-Peer Display Daemon
RFC:        0002
Status:     Draft (supersedes RFC-0001)
Supersedes: RFC-0001
Authors:    Renderd Contributors
Created:    2026-08-03
Reviewed:   2026-08-03 (RFC-0001-review.md — 22 issues, 6 Critical, 9 High, 7 Medium)
```

---

## Abstract

Renderd is an open-source, peer-to-peer display daemon that turns any networked computer into a low-latency external display receiver. It mirrors a host machine's screen to a viewer machine with display-class latency over a local area network, comparable in feel to Apple Sidecar or Luna Display but implemented as a fully cross-platform, open, and modular system.

This document supersedes RFC-0001 in its entirety. It preserves all correct directional decisions from RFC-0001 — QUIC as the transport, H.265 hardware encoding, SPAKE2+ pairing — and resolves every architectural defect identified in the RFC-0001 review, including incorrect platform API assumptions, a mathematically inconsistent latency model, an incomplete security model, and a missing dual-vsync synchronization design.

RFC-0002 is intended to serve as the definitive source of truth before implementation begins.

---

## Table of Contents

1. [Motivation and Goals](#1-motivation-and-goals)
2. [Non-Goals](#2-non-goals)
3. [Terminology](#3-terminology)
4. [System Overview](#4-system-overview)
5. [Transport Layer](#5-transport-layer)
6. [Encoding Pipeline](#6-encoding-pipeline)
7. [Presentation Clock Synchronization](#7-presentation-clock-synchronization)
8. [Discovery and Pairing](#8-discovery-and-pairing)
9. [Security Model](#9-security-model)
10. [Component Architecture](#10-component-architecture)
11. [Control Plane Protocol](#11-control-plane-protocol)
12. [Data Plane Protocol](#12-data-plane-protocol)
13. [Adaptive Bitrate](#13-adaptive-bitrate)
14. [Data Flow](#14-data-flow)
15. [Technology Stack](#15-technology-stack)
16. [Repository Layout](#16-repository-layout)
17. [macOS Distribution Requirements](#17-macos-distribution-requirements)
18. [Failure Modes and Reconnect Strategy](#18-failure-modes-and-reconnect-strategy)
19. [Latency Budget](#19-latency-budget)
20. [Future Work](#20-future-work)
21. [Open Questions](#21-open-questions)
22. [Changes from RFC-0001](#22-changes-from-rfc-0001)
23. [References](#23-references)

---

## 1. Motivation and Goals

### 1.1 Problem Statement

Apple Sidecar and Luna Display deliver display extension at sub-30 ms glass-to-glass latency on a local network. Both are commercially successful and technically impressive. Neither is open-source, cross-platform, or extensible.

Existing open alternatives — VNC, RDP, Parsec — solve remote desktop, not display extension. They carry mandatory overhead for input remoting, session management, and connection brokering that is architecturally incompatible with display-class latency.

Renderd is designed from first principles as a display receiver, not a remote desktop system. The design constraint is simple: a frame captured on the host must appear on the viewer's physical display in under 30 ms at 1080p60, with a graceful latency gradient at higher resolutions.

### 1.2 Goals for v1.0

| Goal | Target |
|------|--------|
| Glass-to-glass latency | ≤ 30 ms @ 1080p60 on Gigabit LAN |
| Glass-to-glass latency | ≤ 40 ms @ 1440p60 on Gigabit LAN |
| Frame rate | 60 FPS minimum |
| Host platform | macOS 13.0+ (Apple Silicon) |
| Viewer platform | Windows 10 or later (primary supported platform); Windows 11 expected to work; Windows 7 planned where feasible |
| Transport | Peer-to-peer, no relay |
| Discovery | Automatic LAN discovery (zero manual configuration) |
| Pairing | Secure one-time pairing (6-digit PIN) |
| Encoding | Hardware-accelerated on both sides |
| UI | Native, modern, no Electron |
| Reconnect | Automatic, transparent, IP-change aware |
| License | MIT |

> **Rationale for resolution-tiered latency targets:** encode latency on Apple Silicon
> hardware scales with resolution and is bounded below by the hardware encoder's
> pipeline depth, not software. Advertising a single flat "30 ms" target across all
> resolutions would be dishonest. Measured encode latency for H.265 in real-time mode
> on M2 is approximately 7–10 ms at 1080p and 10–16 ms at 1440p. These measurements
> must be validated by the implementation team before publication; the targets above
> reflect the expected achievable values with a correct pipeline, not aspirational ones.

---

## 2. Non-Goals

The following are explicitly out of scope for v1.0:

- Remote keyboard and mouse input
- Clipboard synchronization
- Audio streaming
- Multi-monitor support
- Virtual display creation (no physical host display required)
- Internet (WAN) relay or NAT traversal
- Mobile platforms (iOS, Android)
- GPU passthrough or virtualization
- Linux host or viewer

---

## 3. Terminology

| Term | Definition |
|------|------------|
| **Host** | The macOS machine whose screen is being mirrored. Runs `renderd-host`. |
| **Viewer** | The Windows machine that receives and renders the display. Runs `renderd-viewer`. |
| **Control Plane** | Reliable, low-bandwidth channel for pairing, negotiation, statistics, and clock synchronization. |
| **Data Plane** | High-throughput, low-latency channel for compressed video frame fragments. |
| **Frame** | A single compressed video unit (I-frame or P-frame). |
| **Fragment** | A portion of a frame that fits within one QUIC datagram. |
| **Session** | A paired, authenticated, active streaming connection. |
| **Pair Token** | A long-lived secret derived during the SPAKE2+ pairing ceremony, stored in the system keychain. |
| **Presentation Timestamp (PTS)** | The target wall-clock time at which a decoded frame should be displayed on the viewer. |
| **Vsync Phase** | The phase offset of a display's vertical blanking interval relative to a shared reference clock. |

---

## 4. System Overview

```
┌──────────────────────────────────────────────────────────────────────────┐
│  macOS 13+ Host (Apple Silicon)                                          │
│                                                                          │
│  ┌─────────────────┐  ┌──────────────────────┐  ┌────────────────────┐  │
│  │  ScreenCapture  │  │ VideoToolbox Encoder │  │   renderd-host     │  │
│  │  Kit (SCStream) │─▶│ (C FFI shim, H.265)  │─▶│ Login Item Agent   │  │
│  │  IOSurface GPU  │  │  VTCompressionSession│  │                    │  │
│  └─────────────────┘  └──────────────────────┘  │  Control: QUIC S0  │  │
│                                                  │  Data:    QUIC DG  │  │
│                                                  │  PTS Sync: QUIC S0 │  │
│                                                  └─────────┬──────────┘  │
└────────────────────────────────────────────────────────────┼─────────────┘
                                                             │ LAN (UDP/QUIC)
┌────────────────────────────────────────────────────────────┼─────────────┐
│  Windows 10+ Viewer                                          │             │
│                                                  ┌──────────▼──────────┐ │
│                                                  │  renderd-viewer     │ │
│  ┌──────────────┐  ┌──────────────────┐          │                     │ │
│  │  D3D12       │  │ D3D12 Video      │◀─────────│  Control: QUIC S0   │ │
│  │  Renderer    │◀─│ Decode           │          │  Data:    QUIC DG   │ │
│  │  (winit +    │  │ (NV12/P010)      │          │  Vsync Report: S0   │ │
│  │   DXGI swap) │  └──────────────────┘          └─────────────────────┘ │
│  └──────────────┘                                                         │
└───────────────────────────────────────────────────────────────────────────┘
```

The host and viewer communicate over a single QUIC connection containing two logical channels:

- **QUIC Stream 0 (Control):** Reliable, ordered. Used for pairing, codec negotiation, statistics, vsync phase reports, keyframe requests, and session management.
- **QUIC Datagrams (Data):** Unreliable and unordered per RFC 9221. Used for video frame fragments. The data plane protocol handles out-of-order fragment arrival explicitly.

---

## 5. Transport Layer

### 5.1 Protocol Comparison

The four realistic candidates for Renderd's transport layer are evaluated below.

**Raw UDP**

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ Theoretical minimum |
| Reliability | ❌ FEC, retransmit, reorder must be built from scratch |
| Congestion control | ❌ None |
| Encryption | ❌ Manual DTLS or custom |
| Implementation cost | ❌ Extremely high |

Verdict: Dismissed. The implementation cost of building a correct custom transport is not justified when QUIC provides equivalent raw latency with a complete feature set.

---

**RTP / RTSP**

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ Real-time oriented |
| Congestion control | ⚠️ RTCP feedback; not sophisticated |
| Encryption | ⚠️ SRTP requires separate DTLS key exchange |
| Multiplexing | ⚠️ Separate sessions per stream |
| P2P | ❌ Requires RTSP server for signaling |

Verdict: Dismissed. Lacks integrated P2P support and requires significantly more signaling infrastructure than QUIC.

---

**WebRTC (libwebrtc)**

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ Real-time oriented |
| NAT traversal | ✅ Best-in-class ICE/STUN/TURN |
| H.265 support | ❌ Not standardized in the WebRTC specification |
| Binary size | ❌ 100–200 MB compiled |
| Congestion control | ✅ GCC + transport-cc |
| Customizability | ❌ Difficult; Chromium-derived codebase |

Verdict: Appropriate for WAN scenarios where NAT traversal is required (planned for v2.0). H.265 exclusion and binary size make it unsuitable for v1.0 LAN focus.

---

**QUIC (RFC 9000 + RFC 9221)**

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ 0-RTT resumption; no TCP handshake overhead |
| Reliability | ✅ Reliable streams; unreliable datagrams (RFC 9221) |
| Ordering | ⚠️ Streams are ordered; datagrams are unreliable AND unordered |
| Congestion control | ✅ NewReno (default in `quinn`); pluggable |
| Encryption | ✅ TLS 1.3 mandatory |
| Multiplexing | ✅ No HOL blocking between streams |
| H.265 support | ✅ Transport-agnostic; carries any bytes |
| Binary size | ✅ ~5 MB (`quinn`) |
| P2P | ✅ No broker required on LAN |
| Implementation cost | ✅ Low-to-medium with `quinn` in Rust |

Verdict: **Selected.** QUIC provides raw UDP performance with production-grade security, ordered reliable control, and unordered unreliable datagrams for video — exactly the two channels Renderd needs, in a single connection.

> **Note on congestion control:** `quinn` implements **NewReno** by default, not BBR.
> On a Gigabit LAN with no bottleneck link, NewReno is entirely adequate. BBR's
> advantages manifest on high-BDP WAN paths. BBR will be revisited when WAN relay
> is added in v2.0, at which point the QUIC implementation may be reconsidered.

### 5.2 Dual-Channel QUIC Design

```
┌────────────────────────────────────────────────────────────┐
│  Single QUIC Connection (UDP, TLS 1.3, NewReno)            │
│                                                            │
│  ┌────────────────────────┐  ┌──────────────────────────┐  │
│  │  Stream 0              │  │  QUIC Datagrams          │  │
│  │  (Reliable, ordered)   │  │  (Unreliable, UNORDERED) │  │
│  │                        │  │                          │  │
│  │  • Session handshake   │  │  • Video frame fragments │  │
│  │  • Codec negotiation   │  │  • 16-byte header        │  │
│  │  • Vsync phase reports │  │  • Sliding-window reasm  │  │
│  │  • Reactive feedback   │  │                          │  │
│  │  • Periodic stats      │  │                          │  │
│  │  • Keyframe requests   │  │                          │  │
│  │  • Session events      │  │                          │  │
│  └────────────────────────┘  └──────────────────────────┘  │
└────────────────────────────────────────────────────────────┘
```

QUIC datagrams (RFC 9221) are explicitly **unreliable and unordered**. The data plane protocol is designed with this guarantee — or lack thereof — as a first-class constraint.

---

## 6. Encoding Pipeline

### 6.1 Codec Selection

| Codec | HW Encoder (macOS AS) | HW Decoder (Windows) | Encode Latency | Notes |
|-------|-----------------------|--------------------|----------------|-------|
| **H.265 (HEVC)** | ✅ VideoToolbox | ✅ D3D12VA / NVDEC / QSV | Low | **Primary; 10-bit HDR capable** |
| H.264 (AVC) | ✅ VideoToolbox | ✅ D3D12VA / NVDEC / QSV / DXVA2 | Low | Fallback; 8-bit only |
| AV1 | ✅ VideoToolbox (M3+) | ⚠️ RTX 30+, Intel Arc only | Higher | Future (v2.0) |
| VP9 | ❌ No HW encoder | — | — | Eliminated |

**H.265 is selected as the primary codec; H.264 as fallback.** Codec selection is negotiated during `SessionHello` / `SessionConfig` exchange. The host proposes H.265; if the viewer signals it cannot hardware-decode H.265, the host falls back to H.264. If neither hardware codec is available, the session is rejected with `ERROR_NO_HW_CODEC` — software encode/decode is never used, as it is CPU-prohibitive at 60 FPS.

H.265 provides approximately 40% better compression efficiency than H.264 at equal perceptual quality, directly reducing average frame size, datagram count per frame, and network transmission time. 10-bit encoding eliminates banding artifacts common in UI content rendered at reduced color depth.

### 6.2 Host Capture and Encode Pipeline

**Process model:** `renderd-host` runs as a macOS **Login Item Agent** — not a launchd daemon. The distinction is fundamental: ScreenCaptureKit requires the process to run as the logged-in user within an active WindowServer session, with the `com.apple.security.screen-recording` entitlement granted and a user-interactive process capable of presenting the TCC permission dialog. A launchd daemon running as root satisfies none of these conditions and will be blocked by TCC. See §17 for entitlements and distribution requirements.

```
[Login Item Agent — runs as user, user session]

Display vsync (host)
    │
    ▼
SCStream callback (QOS_CLASS_USER_INTERACTIVE)
  IOSurface-backed CMSampleBuffer (GPU-resident)
    │
    ▼ (GPU bus, no CPU copy)
VTCompressionSession (VideoToolbox C API via C shim)
  Properties:
    RequireHardwareAcceleratedVideoEncoder = TRUE     ← fail loudly if unavailable
    RealTime = TRUE                                  ← latency over quality
    PrioritizeEncodingSpeedOverQuality = TRUE        ← macOS 13+; ~20% latency reduction
    MaxKeyFrameIntervalDuration = 0.5s               ← max 500ms blank on cold connect
    BaseLayerFrameRateFraction = 1.0                 ← no temporal scalability
    AllowFrameReordering = FALSE                     ← no B-frames
    AverageBitRate = <ABR-controlled>
    ExpectedFrameRate = 60
    │
    ▼
NAL unit stream (Annex-B, H.265 or H.264)
    │
    ▼
Fragment into QUIC datagrams (burst send, §12)
```

**Thread model:** The SCStream callback and VTCompressionSession dispatch run on a dedicated OS thread with `QOS_CLASS_USER_INTERACTIVE`. No thread affinity is set. The macOS scheduler routes QoS-interactive threads to P-cores when active and does so more reliably than manual affinity for bursty periodic workloads. The QUIC network I/O runs on the Tokio async runtime on a separate thread pool. Inter-thread communication uses a lock-free SPSC ring buffer sized for 4 frames.

> **Why no E-core pinning:** Apple Silicon E-cores run at 40–60% of P-core frequency
> and have longer sleep-to-active transition latency. The capture thread fires every
> 16.7 ms and must complete IOSurface handoff and VTCompressionSessionEncodeFrame
> dispatch within ~2 ms. On E-cores, this dispatch overhead increases by 2–4×,
> causing missed encode deadlines. The macOS QoS scheduler already implements the
> correct policy for periodic interactive workloads. Manual pinning overrides it
> with a worse policy.

**Keyframe on connect:** When a new viewer session is established, the host immediately forces a keyframe by setting `kVTEncodeFrameOptionKey_ForceKeyFrame = TRUE` on the next frame submission. This eliminates the cold-connect blank screen that would otherwise persist until the next natural I-frame (up to 500 ms).

### 6.3 Viewer Decode and Render Pipeline

```
[renderd-viewer — Windows 10+]

QUIC datagrams arrive (unordered)
    │
    ▼
Sliding-window reassembly buffer (§12.2)
  Keyed by frame_id; window depth = 4 frames
  Dynamic fragment deadline (§12.3)
    │
    ▼
D3D12 Video Decode (ID3D12VideoDecoder)
  Primary:  D3D12 Video Decode API (driver-agnostic, Windows 10+)
  Fallback: Direct to NVDEC via CUDA (NVIDIA-only path)
  Fallback: Intel MFX (Intel Arc/Iris Xe)
  Output: NV12 (8-bit) or P010 (10-bit) GPU surface
    │
    ▼ (GPU memory, no CPU copy)
D3D12 Render Pass
  Shader: YUV→RGB BT.709 / BT.2020 conversion
  Scaling: GPU bilinear to viewer resolution
  Aspect ratio: letterbox or pillarbox
    │
    ▼
DXGI Swap Chain
  Mode: borderless fullscreen (winit, not exclusive fullscreen)
  Tearing: IDXGIFactory5::CheckFeatureSupport → conditional ALLOW_TEARING
  VRR: if tearing supported and display VRR-capable
  Fallback: SyncInterval=1 (vsync-locked) if tearing not supported
```

**DXGI tearing is conditional, not assumed.** On startup, the renderer calls `IDXGIFactory5::CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING)`. The swap chain is created with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING` only when this check returns `TRUE`. `Present1()` uses `DXGI_PRESENT_ALLOW_TEARING` only when the flag was set. Failure to perform this check results in `DXGI_ERROR_INVALID_CALL` on systems without tearing support — a runtime crash.

**Window management via `winit`:** The Win32 message pump, DPI handling, borderless fullscreen state, monitor enumeration, and resize events are handled by the `winit` crate. The D3D12 device, swap chain, and video decode pipeline are managed directly via `windows-rs`. This division — `winit` for window lifecycle, `windows-rs` for GPU — eliminates ~2,000 lines of error-prone raw Win32 while imposing zero rendering overhead.

---

## 7. Presentation Clock Synchronization

This section describes the mechanism that prevents the dual-vsync penalty — the single most important differentiator between a display extension tool and a screen casting demo.

### 7.1 The Dual-Vsync Problem

The host and viewer maintain independent display vsync clocks that are not synchronized. If the host captures at its vsync (t=0) and the encode + network + decode pipeline completes at t=22 ms, the viewer's next vsync may occur at t=16.7 ms (missed) or t=33.4 ms (next one). Without synchronization, frame delivery randomly straddles vsync boundaries, causing latency to oscillate by up to one full frame period (16.7 ms at 60 Hz). The average glass-to-glass latency is therefore the pipeline latency plus half a frame period — not the pipeline latency alone.

Apple Sidecar eliminates this penalty by synchronizing host capture phase to the viewer's vsync phase. Renderd implements an equivalent mechanism.

### 7.2 Vsync Phase Protocol

```
Phase synchronization runs continuously on QUIC Stream 0.

Viewer → Host (every frame period, ~16.7 ms):
  VsyncReport {
    vsync_period_ns:   uint64   // e.g., 16666666 for 60 Hz
    vsync_phase_ns:    uint64   // monotonic timestamp of last vsync
    clock_epoch_ns:    uint64   // shared reference for clock offset calculation
  }

Host (on receipt):
  1. Compute clock_offset = (local_recv_time - clock_epoch_ns) - (rtt / 2)
  2. Compute viewer_vsync_phase_local = vsync_phase_ns + clock_offset
  3. Compute next_viewer_vsync = viewer_vsync_phase_local + N × vsync_period_ns
     where N is the smallest integer such that next_viewer_vsync > now + pipeline_budget_ns
  4. Schedule the next SCStream capture to begin at:
     capture_time = next_viewer_vsync - encode_latency_ns - network_rtt_ns/2
  5. Set this as the SCStream minimumFrameInterval deadline
```

The pipeline budget (`encode_latency_ns`) is the exponential moving average of recent measured encode durations from the VideoToolbox output callback. The network RTT is measured from QUIC connection statistics.

This protocol ensures frames arrive at the viewer with a predictable lead time before the viewer's vsync, typically 2–5 ms, allowing consistent single-vsync presentation. The result is stable, low-jitter glass-to-glass latency rather than the oscillating pattern of an unsynchronized pipeline.

### 7.3 VRR Interaction

When the viewer reports `vrr_supported = true`, the host may send frames at irregular intervals (e.g., matching exact content frame rate rather than display refresh rate). In this mode, the vsync phase protocol is not needed for latency; it is still run for clock offset calibration.

### 7.4 Phase Sync Startup

On session establishment, the host operates in an unsynchronized "fast start" mode for the first 30 frames: capture runs at the native SCStream vsync rate and frames are sent immediately. After 30 frames, sufficient RTT and encode latency samples have accumulated, and phase synchronization activates.

---

## 8. Discovery and Pairing

### 8.1 LAN Discovery

Renderd uses mDNS (RFC 6762) and DNS-SD (RFC 6763) for zero-configuration LAN discovery.

**Service type:** `_renderd._udp.local.`

**TXT records advertised by Host:**
```
version=1
id=<host-uuid>          # Stable UUID-v4, persisted to disk; never changes
name=<hostname>         # e.g., "Adith's Mac Pro"
paired=<count>          # Number of currently paired viewers
```

**Platform-specific mDNS implementations:**

| Platform | API | Notes |
|----------|-----|-------|
| macOS (host) | `dns_sd.h` (Bonjour) | System API; `DNSServiceRegister` for advertisement. Cannot use pure-Rust mDNS crates — they conflict with mDNSResponder's exclusive ownership of UDP port 5353. |
| Windows (viewer) | `DnsServiceRegister` (Win32) | Available since Windows 10 v1703; native, no dependency |

The `mdns-sd` pure-Rust crate is **not used on macOS**. On macOS, mDNSResponder holds exclusive ownership of port 5353. Any process that attempts to bind this port directly — as `mdns-sd` does — will either fail to bind or produce packet collisions. The correct approach is to delegate to mDNSResponder via `dns_sd.h`.

**Discovery fallback (manual IP):** If mDNS fails — due to corporate firewall rules, IGMP snooping on managed switches, or VPN tunnels that suppress multicast — the viewer UI provides a "Connect manually…" option accepting a host IP address and port. This is required for real-world deployment in enterprise environments.

### 8.2 Secure Pairing (SPAKE2+, RFC 9382)

Pairing is a one-time ceremony that establishes mutual authentication between a specific host and a specific viewer. The result is a long-lived Pair Token stored in the system keychain on both sides.

**Protocol:** SPAKE2+ as specified in **RFC 9382** (published August 2023). SPAKE2+ provides:
- Resistance to offline dictionary attacks, even with a 6-digit PIN.
- Mutual authentication: neither side can complete the protocol without knowing the PIN.
- Perfect forward secrecy for the pairing session.

> **SPAKE2 vs SPAKE2+:** The RustCrypto `spake2` crate implements the SPAKE2 protocol,
> not SPAKE2+ (RFC 9382). SPAKE2+ adds explicit mutual authentication and role
> differentiation that SPAKE2 lacks. The implementation team must verify RFC 9382
> test vector compliance before adopting any library. If no compliant library exists,
> implement directly from RFC 9382 §3 and add the test vectors from RFC 9382 §4 to
> the CI suite.

```
Pairing Ceremony

1. Host generates a cryptographically random 6-digit PIN and displays it
   in the menu bar UI alongside a countdown timer (60 seconds before expiry).

2. User selects the host from the Viewer's discovery list and enters the PIN.

3. Viewer initiates a QUIC connection to the host's mDNS-discovered address.
   Connection is accepted in "pairing mode" (unauthenticated, rate-limited).

4. SPAKE2+ exchange (RFC 9382) on QUIC Stream 0:
     Viewer → Host:  SPAKE2+ shareP (prover message)
     Host   → Viewer: SPAKE2+ shareV (verifier message)
     Both independently compute shared key K.
     Viewer → Host:  confirmP = MAC(K_confirmP, shareV)
     Host   → Viewer: confirmV = MAC(K_confirmV, shareP)
   If either confirmation fails, the connection is closed immediately.

5. Both sides derive the Pair Token using HKDF-SHA256:
     PairToken = HKDF-SHA256(
       ikm  = K,
       salt = "renderd-v1-pair-token",
       info = "renderd-v1-pair:" || uuid_canonical(host_id) || ":" || uuid_canonical(viewer_id)
     )
   UUIDs are in lowercase hyphenated canonical form (36 bytes, fixed length).
   This eliminates the length-ambiguity collision described in the RFC-0001 review.

6. Each side generates a self-signed TLS certificate using the PairToken as the
   seed for key derivation (rcgen + HKDF). Certificates have a 10-year validity
   period from the pairing date.

7. PairToken and cert_expires_at are stored in the system keychain:
     macOS:   Keychain Services (kSecClassGenericPassword)
     Windows: Windows Credential Manager (CredWrite)
   Stored alongside the token: host_uuid, viewer_uuid, paired_at, cert_expires_at.

8. Host stores viewer's certificate in its known-viewers registry.
   Host emits a macOS UserNotifications notification: "Paired with [viewer hostname]."
```

Rate limiting: 5 failed PIN attempts triggers a 120-second exponential lockout (not a flat 60-second reset, which allows brute-force to resume immediately).

### 8.3 Session Authentication (Post-Pairing)

Every subsequent session uses mutual TLS within QUIC, with no PIN re-entry required:

```
1. Viewer discovers Host by UUID via mDNS browse (or cached IP).
2. Viewer initiates QUIC connection.
3. QUIC's TLS 1.3 handshake performs mutual certificate verification.
   Host checks Viewer certificate against known-viewers registry.
   Viewer checks Host certificate against stored host cert.
4. On cert mismatch: connection rejected, reason logged, optional notification.
5. On success: SessionHello exchange begins immediately.
```

**Certificate auto-renewal:** On every session establishment, each side checks whether its certificate has fewer than 180 days of remaining validity. If so, a new certificate is derived from the stored PairToken via HKDF and exchanged over the authenticated QUIC session, replacing the old certificate in both keystores. Users never see certificate expiry warnings.

---

## 9. Security Model

### 9.1 Threat Model

Renderd v1.0 operates exclusively on a LAN. The threat model assumes:
- The LAN is not fully trusted (other LAN devices may be compromised).
- The host and viewer machines' keychains are trusted (keychain compromise is out of scope; it grants arbitrary local access regardless of Renderd).
- WAN communication does not occur in v1.0.

### 9.2 Security Properties

| Property | Mechanism |
|----------|-----------|
| Confidentiality | All data encrypted via QUIC/TLS 1.3 (mandatory; no downgrade path) |
| Authentication | Mutual TLS with certificates derived from Pair Token |
| Pairing security | SPAKE2+ (RFC 9382) PAKE — attacker without PIN cannot derive session key |
| Replay prevention | TLS 1.3 record sequence numbers; QUIC packet numbers |
| PIN brute-force | Exponential lockout: 5 attempts → 120s, 10 attempts → 240s, etc. |
| Pair Token longevity | 10-year cert expiry; auto-renewed from stored PairToken with 180-day advance |
| Key storage | System keychain (macOS Keychain / Windows Credential Manager) |
| DoS amplification | QUIC stateless retry tokens (RFC 9000 §8.1) prevent reflection attacks |
| HKDF info collision | Canonical UUID format (fixed 36-byte strings with delimiter) |

### 9.3 Token Revocation and Device Management

The host exposes a **Paired Devices** panel in its menu bar UI showing, for each paired viewer: device name, pairing date, last connection timestamp, and certificate expiry date. A **Revoke** button removes the viewer's certificate from the known-viewers registry. After revocation, the viewer's mutual TLS handshake will fail immediately; the viewer must re-pair to reconnect.

When a new streaming session begins, the host emits a macOS `UserNotifications` alert: `"[Viewer hostname] began screen sharing."` This gives the user awareness of active sessions and acts as an early warning for unauthorized use.

### 9.4 Session Key Isolation

Each session derives a short-lived session key from the Pair Token and a random per-session nonce:
```
SessionKey = HKDF-SHA256(PairToken, session_nonce, "renderd-v1-session")
```
The session nonce is exchanged as part of `SessionHello`. This means that even if a session's TLS traffic is captured and the PairToken is later compromised, past session recordings cannot be decrypted retroactively. TLS 1.3 ephemeral keys already provide this property; the SessionKey derivation adds a Renderd-layer binding.

---

## 10. Component Architecture

### 10.1 Host Agent (`renderd-host`)

**Process type:** macOS Login Item Agent (`LSUIElement = true`).  
**Registered via:** `SMAppService.mainApp` (macOS 13+).  
**Entitlements:** `com.apple.security.screen-recording`, `com.apple.security.app-sandbox`.  
**Distribution:** Signed `.app` bundle, notarized via `xcrun notarytool`.

```
renderd-host/
├── c-shims/
│   └── videotoolbox_shim.c      # C FFI bridge for VTCompressionSession
│   └── build.rs                 # cc::Build compilation of C shim
├── src/
│   ├── capture/
│   │   └── screencapture_kit.rs # SCStream via objc2 (ObjC API)
│   ├── encode/
│   │   └── videotoolbox.rs      # VTCompressionSession via C shim (C API)
│   ├── network/
│   │   ├── quic_server.rs       # quinn QUIC server
│   │   ├── control_plane.rs     # Stream 0 message handler
│   │   └── data_plane.rs        # Datagram burst sender
│   ├── discovery/
│   │   └── bonjour.rs           # dns_sd.h via DNSServiceRegister (macOS system)
│   ├── pairing/
│   │   └── spake2plus.rs        # RFC 9382 SPAKE2+ ceremony (prover/verifier)
│   ├── session/
│   │   ├── auth.rs              # Mutual TLS cert management
│   │   ├── devices.rs           # Known-viewers registry + revocation
│   │   └── session_key.rs       # Per-session HKDF key derivation
│   ├── keychain/
│   │   └── macos_keychain.rs    # Keychain Services (security-framework crate)
│   ├── clock_sync/
│   │   └── vsync_scheduler.rs   # Presentation clock sync (§7)
│   ├── abr/
│   │   └── controller.rs        # Dual-timescale ABR controller (§13)
│   └── ui/
│       ├── menubar.rs           # NSStatusBar menu bar agent
│       ├── paired_devices.rs    # Device management panel
│       └── notifications.rs     # UserNotifications (session start, pairing)
└── Cargo.toml
```

**Thread model:**

```
Thread 1:  SCStream callback + VTCompressionSession dispatch
           QoS: QOS_CLASS_USER_INTERACTIVE
           Writes to: frame_ring_buffer (SPSC, capacity 4)

Thread 2:  QUIC send loop (Tokio async)
           Reads from: frame_ring_buffer
           Sends: datagram bursts (one burst per frame)

Thread 3:  QUIC Stream 0 handler (Tokio async)
           Handles: all control messages inbound/outbound

Thread 4:  Clock sync scheduler
           Runs: vsync phase computation (§7)
           Writes to: SCStream capture schedule

Main:      NSRunLoop (menu bar UI, AppKit event loop)
```

### 10.2 Viewer Client (`renderd-viewer`)

**Window management:** `winit` (cross-platform, production-quality, used by Bevy/wgpu/Tauri).  
**GPU pipeline:** `windows-rs` + D3D12 (decode, render, swap chain).  
**Distribution:** Standalone `.exe` + installer (NSIS or WiX).

```
renderd-viewer/
├── src/
│   ├── network/
│   │   ├── quic_client.rs       # quinn QUIC client
│   │   ├── control_plane.rs     # Stream 0 message handler
│   │   └── data_plane.rs        # Datagram receiver + sliding-window reassembly
│   ├── decode/
│   │   └── d3d12_decode.rs      # D3D12 Video Decode (ID3D12VideoDecoder)
│   ├── render/
│   │   ├── d3d12_renderer.rs    # Swap chain, YUV→RGB shader, present
│   │   └── tearing_check.rs     # IDXGIFactory5 ALLOW_TEARING capability check
│   ├── discovery/
│   │   ├── dns_service.rs       # DnsServiceBrowse (Win32 mDNS browse)
│   │   └── manual_connect.rs    # Manual IP/port fallback
│   ├── pairing/
│   │   └── spake2plus.rs        # RFC 9382 SPAKE2+ ceremony (client side)
│   ├── session/
│   │   ├── auth.rs              # Mutual TLS cert management
│   │   └── session_key.rs       # Per-session HKDF key derivation
│   ├── keychain/
│   │   └── windows_credential.rs # Windows Credential Manager
│   ├── clock_sync/
│   │   └── vsync_reporter.rs    # DWM vsync phase capture + reporting (§7)
│   ├── abr/
│   │   └── feedback.rs          # Dual-timescale feedback sender (§13)
│   ├── reconnect/
│   │   └── watchdog.rs          # UUID-aware reconnect with mDNS re-discovery
│   └── ui/
│       ├── app.rs               # winit event loop + fullscreen window
│       ├── overlay.rs           # Reconnecting / pairing status overlay
│       └── settings.rs          # Settings panel (tray icon on Windows)
└── Cargo.toml
```

**Vsync phase reporting:** The viewer captures the DWM vsync timestamp using `DwmGetCompositionTimingInfo` and reports it to the host via `VsyncReport` on Stream 0 at every frame deadline. This feeds the host's presentation clock scheduler (§7).

### 10.3 Shared Library (`librenderd`)

A Rust crate shared between host and viewer containing:

- `protocol/` — All protobuf-generated message types (prost)
- `codec_params/` — SPS/PPS/VPS parsing and negotiation
- `frame_id/` — Sliding-window fragment reassembly state machine
- `hkdf_util/` — Canonical HKDF helpers with fixed info-string encoding
- `stats/` — Ring-buffer statistics primitives
- `error/` — Unified error type hierarchy

---

## 11. Control Plane Protocol

All control messages on Stream 0 are serialized with **Protocol Buffers** (proto3, prost crate). The stream uses a simple 4-byte length-prefix framing: `[u32 length][protobuf bytes]`.

### 11.1 Version Negotiation

Version incompatibility is handled explicitly, not silently:

```protobuf
message SessionHello {
  uint32 protocol_version     = 1;  // Sender's own version
  uint32 min_required_version = 2;  // Minimum version the sender requires from peer
  string viewer_id            = 3;  // Viewer UUID
  repeated string supported_codecs = 4;  // ["hevc", "h264"]
  uint32 max_decode_bitrate_kbps   = 5;
  DisplayInfo display              = 6;
  bool   hw_decode_available       = 7;
  string session_nonce             = 8;  // Random; for SessionKey derivation
}

message DisplayInfo {
  uint32 width          = 1;
  uint32 height         = 2;
  float  refresh_rate   = 3;
  bool   vrr_supported  = 4;
}
```

If `min_required_version` in `SessionHello` exceeds the host's `protocol_version`, the host responds with an `Error` message and closes the connection. This prevents silent degradation.

### 11.2 Session Configuration

```protobuf
message SessionConfig {
  string selected_codec        = 1;  // "hevc" or "h264"
  uint32 width                 = 2;
  uint32 height                = 3;
  float  frame_rate            = 4;
  uint32 initial_bitrate_kbps  = 5;
  bytes  codec_extra_data      = 6;  // HEVC VPS+SPS+PPS or H.264 SPS+PPS
  bool   phase_sync_enabled    = 7;  // Whether vsync phase sync is active
}
```

### 11.3 Ongoing Control Messages

```protobuf
// Viewer → Host: vsync phase (every frame period, ~16.7 ms)
message VsyncReport {
  uint64 vsync_period_ns   = 1;
  uint64 vsync_phase_ns    = 2;  // Monotonic; local to viewer
  uint64 clock_epoch_ns    = 3;  // Shared reference for offset computation
}

// Viewer → Host: periodic stats (every 100 ms)
message ReactiveStats {
  float  loss_rate       = 1;  // Frame loss rate since last report (0.0–1.0)
  uint32 jitter_us       = 2;  // Fragment arrival jitter, microseconds
  uint64 last_frame_id   = 3;
}

// Viewer → Host: slow-path stats (every 500 ms)
message PeriodicStats {
  float  receive_bandwidth_kbps = 1;  // Estimated receive bandwidth
  uint32 decode_time_us         = 2;  // Mean hardware decode time
  uint32 render_time_us         = 3;  // Mean D3D12 render + present time
  uint64 frames_displayed       = 4;
  uint64 frames_dropped         = 5;
}

// Viewer → Host: immediate, on frame loss detection
message KeyframeRequest {
  uint64 after_frame_id       = 1;
  uint32 bandwidth_hint_kbps  = 2;  // Viewer's current estimated receive BW
}

// Host → Viewer: ABR decision
message BitrateAdjust {
  uint32 new_bitrate_kbps = 1;
  bool   force_keyframe   = 2;
}

// Host → Viewer: resolution / framerate change notification
message StreamReconfigure {
  uint32 new_width      = 1;
  uint32 new_height     = 2;
  float  new_frame_rate = 3;
  bytes  new_codec_extra_data = 4;
}

// Bidirectional: protocol error
message Error {
  enum Code {
    UNKNOWN                = 0;
    VERSION_INCOMPATIBLE   = 1;
    NO_HW_CODEC            = 2;
    AUTH_FAILED            = 3;
    PAIRING_FAILED         = 4;
    RATE_LIMITED           = 5;
  }
  Code   code    = 1;
  string message = 2;
}
```

### 11.4 Message Envelope

Each message on Stream 0 is wrapped in an envelope that identifies its type, avoiding a separate multiplexing layer:

```protobuf
message Envelope {
  oneof payload {
    SessionHello      hello               = 1;
    SessionConfig     config              = 2;
    VsyncReport       vsync_report        = 3;
    ReactiveStats     reactive_stats      = 4;
    PeriodicStats     periodic_stats      = 5;
    KeyframeRequest   keyframe_request    = 6;
    BitrateAdjust     bitrate_adjust      = 7;
    StreamReconfigure stream_reconfigure  = 8;
    Error             error               = 9;
  }
}
```

---

## 12. Data Plane Protocol

### 12.1 Frame Datagram Header

Each QUIC datagram carries one fragment of one encoded frame, prefixed with a 16-byte header:

```
┌─────────────────────────────────────────────────────────────────────┐
│  Fragment Header (16 bytes, little-endian)                          │
│  ┌──────────┬──────────┬──────────┬────────┬────────────────────┐   │
│  │ frame_id │  frag_id │frag_total│ flags  │   pts_offset_us    │   │
│  │  8 bytes │  2 bytes │  2 bytes │ 1 byte │      3 bytes       │   │
│  └──────────┴──────────┴──────────┴────────┴────────────────────┘   │
│  Payload: NAL unit bytes (up to PMTU - 16 bytes, typically ~1200 B) │
└─────────────────────────────────────────────────────────────────────┘

frame_id:     Monotonically increasing, 64-bit. Unique per encoded frame.
frag_id:      0-indexed fragment number within this frame.
frag_total:   Total fragment count for this frame (1 for unfragmented frames).
flags:
  bit 0:  is_keyframe
  bit 1:  is_last_fragment (equivalent to frag_id == frag_total - 1)
  bit 2:  phase_sync_valid  (PTS is meaningful; phase sync is active)
pts_offset_us: Unsigned 24-bit (3-byte) little-endian offset in microseconds
               from the most recently clock-synced base PTS (exchanged on
               Stream 0 every 1 second). Maximum range: ~16.7 s (0x00FF_FFFF µs).
```

### 12.2 Sliding-Window Fragment Reassembly

QUIC datagrams are explicitly **unreliable and unordered** (RFC 9221). The reassembly buffer is designed accordingly.

The viewer maintains a sliding window of `W = 4` in-flight frames (at 60 FPS, this covers ~67 ms of pipeline depth). The window is a hash map keyed by `frame_id`:

```
Window state (per in-flight frame):
  frame_id:       u64
  fragments:      HashMap<u16, Bytes>  // frag_id → payload
  frag_total:     u16                  // known once any fragment arrives
  received_count: u16
  first_arrival:  Instant
  is_keyframe:    bool
  pts:            Instant              // computed from pts_offset_us + base PTS
```

Fragment arrival logic:
1. On datagram receipt, extract `frame_id`.
2. If `frame_id < (max_seen_frame_id - W)`: discard (too old; missed the window).
3. Otherwise: insert fragment into the window entry for `frame_id`.
4. When `received_count == frag_total`: frame is complete — pass to D3D12 decode.
5. On frame completion, advance the window minimum if the completed frame was the oldest.

### 12.3 Dynamic Fragment Deadline

Rather than a static timeout, the fragment deadline is computed per-frame from the most recent `PeriodicStats`:

```
deadline_budget = frame_period - mean_decode_time_us - mean_render_time_us - vsync_lead_us
                = 16,666 µs  - decode_time_us        - render_time_us       - 2,000 µs

Static default (before first PeriodicStats received): 12,000 µs (12 ms)
Minimum enforced: 8,000 µs (8 ms) to prevent runaway keyframe requests
Maximum enforced: 14,000 µs (14 ms)
```

**On deadline expiry:** The viewer does not freeze or blank. Instead, it passes whatever fragments have arrived to the D3D12 decoder in error-concealment mode. The decoder produces a concealed frame (using the previous frame's reference). Simultaneously, a `KeyframeRequest` is sent with the frame loss timestamp. This approach avoids the visible freeze that discarding produces.

### 12.4 Fragment Sending Strategy

At 30 Mbps default bitrate and 60 FPS, a typical P-frame is ~62,500 bytes, requiring ~55 QUIC datagrams at 1,150-byte payload. On macOS, each datagram requires a separate `sendmsg()` syscall. At 55 datagrams × 60 FPS = 3,300 syscalls/second.

To minimize syscall overhead, the host sends all fragments of a single frame in a **tight synchronous burst** — not spread across Tokio async yields. The Tokio task responsible for sending reads all fragments from the ring buffer and calls `quinn`'s `send_datagram()` in a non-yielding loop, submitting all fragments before yielding back to the async runtime. This batches socket writes into the OS's UDP output queue, allowing the NIC's hardware segmentation offload to process them efficiently.

Maximum bitrate for v1.0 is set at **50 Mbps** until the burst-send path is benchmarked and validated on the target hardware. The theoretical maximum supported by the framing is much higher.

Path MTU discovery: The QUIC connection performs path MTU discovery at session start. On a typical Gigabit LAN with standard 1500-byte Ethernet MTU, the QUIC datagram payload is approximately 1,200 bytes (1500 - IP header 20 - UDP header 8 - QUIC overhead ~250 - fragment header 16). On networks with jumbo frames, this increases to ~8,960 bytes, reducing fragment count by 7×.

---

## 13. Adaptive Bitrate

Renderd's ABR controller operates on two timescales to separate reactive loss handling from proactive bandwidth estimation.

### 13.1 Reactive Path (100 ms timescale)

Triggered by `ReactiveStats` (sent every 100 ms by the viewer) and by immediate `KeyframeRequest` messages:

| Signal | Action |
|--------|--------|
| `loss_rate` > 0.05 (5%) | Reduce bitrate by 25%; request keyframe |
| `loss_rate` > 0.20 (20%) | Reduce bitrate by 50%; request immediate keyframe |
| `jitter_us` > 8,000 µs | Reduce bitrate by 15% |
| `KeyframeRequest` received | Inject keyframe immediately; reduce bitrate by 25% |
| `loss_rate` == 0.0 for 3 consecutive intervals | Increase bitrate by 10% |

### 13.2 Proactive Path (500 ms timescale)

Driven by `PeriodicStats` (sent every 500 ms):

| Signal | Action |
|--------|--------|
| `receive_bandwidth_kbps` < current bitrate | Reduce to 80% of estimated bandwidth |
| `decode_time_us` > 0.8 × frame_period | Signal decoder overload; request reduced frame rate |
| `receive_bandwidth_kbps` > current bitrate × 1.25 | Increase bitrate by 15% |

### 13.3 Bitrate Bounds

| Parameter | Value |
|-----------|-------|
| Minimum | 5 Mbps |
| Default (1080p60) | 20 Mbps |
| Default (1440p60) | 30 Mbps |
| Maximum (v1.0) | 50 Mbps |
| Ramp-up rate | +10% per reactive interval when clear |
| Ramp-down rate | −25% on first loss; −50% on burst loss |

The asymmetric ramp ensures rapid recovery from congestion while avoiding the slow 6-second recovery that a symmetric ±5% scheme produces.

---

## 14. Data Flow

```
                HOST                                       VIEWER
                ────                                       ──────

Phase sync scheduler computes
next capture time (§7)
    │
    ▼ (at scheduled time)
SCStream callback
IOSurface (GPU-resident)
QoS: USER_INTERACTIVE thread
    │
    ▼ (GPU bus, zero copy)
VTCompressionSession
(H.265 HW encode, C shim)
    │
    ▼
NAL unit stream
    │
Burst send all fragments ──────── UDP/QUIC datagrams ─────▶ Fragments arrive (unordered)
                                                                  │
                                                        Sliding-window reassembly
                                                        (§12.2, W=4 frame window)
                                                                  │
                                                         Frame complete or deadline
                                                                  │
                                                          D3D12 HW Video Decode
                                                          (NV12/P010 GPU surface)
                                                                  │
                                                         D3D12 render (YUV→RGB)
                                                                  │
                                                         DXGI present (±tearing)
                                                         at computed PTS deadline
                                                                  │
                                                          Display output

Feedback path:
                ◀── Stream 0 (VsyncReport, every ~16.7 ms)
                ◀── Stream 0 (ReactiveStats, every 100 ms)
                ◀── Stream 0 (PeriodicStats, every 500 ms)
                ◀── Stream 0 (KeyframeRequest, immediate on loss)
    │
    ▼
Vsync scheduler update +
ABR controller update +
VTCompressionSession bitrate / keyframe adjustment
```

---

## 15. Technology Stack

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | **Rust** | Memory safety without GC; first-class FFI for C APIs; async support |
| Async runtime | **Tokio** | De facto standard; integrates with quinn |
| QUIC | **quinn** (pure Rust) | RFC 9000 + RFC 9221; clean async API; NewReno congestion control |
| Serialization | **prost** (protobuf3) | Rust-native; no C++ dependency; forward-compatible |
| macOS capture | **ScreenCaptureKit** via `objc2` | ObjC API; GPU-resident IOSurface frames |
| macOS encode | **VideoToolbox** via **C shim** | VT is a C/CoreFoundation API, not ObjC; `objc2` does not apply here |
| macOS C shim build | **`cc` crate** in `build.rs` | Compiles `videotoolbox_shim.c` into the Rust binary |
| CF/CM types | **`core-foundation`** + **`core-media`** crates | Rust-safe wrappers for CFTypeRef, CMSampleBufferRef |
| Windows decode | **`windows-rs`** + D3D12 Video | Microsoft's official Rust WinAPI bindings |
| Windows render | **D3D12** via `windows-rs` | Low-level; minimal driver overhead; full GPU pipeline control |
| Window management | **`winit`** | Production-quality; handles DPI, fullscreen, message pump; not Electron |
| mDNS (macOS) | **`dns_sd.h`** (system Bonjour) | Must use system API; pure-Rust crates conflict with mDNSResponder |
| mDNS (Windows) | **`DnsServiceBrowse`** (Win32) | Native; no external dependency |
| PAKE | **RFC 9382-compliant SPAKE2+** | Verify test vectors before adopting any crate |
| TLS certs | **`rcgen`** crate | Self-signed cert generation; HKDF-seeded key material |
| Keychain (macOS) | **`security-framework`** crate | Safe Rust wrapper for Keychain Services |
| Keychain (Windows) | **`windows-rs`** CredentialManager | Native credential storage |
| UI (macOS) | **`tray-icon`** + AppKit | NSStatusBar menu bar agent |
| UI (Windows) | **`winit`** + Win32 tray | System tray icon via Win32 Shell_NotifyIcon |
| Build | **Cargo workspace** | Standard Rust workspace; platform-conditional compilation via `cfg` |
| CI | **GitHub Actions** | macOS runner (host + notarization), Windows runner (viewer) |

---

## 16. Repository Layout

> **Canonical reference:** The authoritative workspace layout is specified in
> [REPO-0001 §2](REPO-0001-repository.md#2-workspace-layout). The summary below
> reflects the final structure; consult REPO-0001 for full crate responsibilities.

```
renderd/
├── Cargo.toml                          # Workspace root (15 members)
├── rust-toolchain.toml                 # Pinned stable channel + targets
├── clippy.toml                         # Workspace-wide lint overrides
├── .rustfmt.toml                       # Workspace-wide code formatting
├── deny.toml                           # cargo-deny: licenses, advisories
├── nextest.toml                        # cargo-nextest configuration
│
├── crates/
│   │   ── Foundation Layer ────────────────────────────────────────────
│   ├── renderd-proto/                  # Protobuf types + domain newtypes
│   ├── renderd-config/                 # Configuration schema + loader
│   │
│   │   ── Primitive Layer ────────────────────────────────────────────
│   ├── renderd-frame/                  # Fragment header codec + reassembly
│   ├── renderd-crypto/                 # SPAKE2+ (RFC 9382), HKDF, certs
│   │
│   │   ── FFI Layer (unsafe; macOS only) ─────────────────────────────
│   ├── renderd-vt-sys/                 # VideoToolbox C FFI bindings
│   ├── renderd-sc-sys/                 # ScreenCaptureKit ObjC bindings
│   │
│   │   ── Service Layer ────────────────────────────────────────────
│   ├── renderd-net/                    # QUIC connection + datagram pipeline
│   ├── renderd-keychain/               # Keychain abstraction + platform impls
│   ├── renderd-discovery/              # mDNS advertisement + browsing
│   │
│   │   ── Algorithm Layer ───────────────────────────────────────────
│   ├── renderd-abr/                    # Dual-timescale ABR controller
│   ├── renderd-clock/                  # Presentation clock synchronization
│   │
│   │   ── Application Layer ──────────────────────────────────────────
│   ├── renderd-host/                   # macOS Login Item Agent binary
│   │   ├── c-shims/videotoolbox_shim.c # VTCompressionSession C bridge
│   │   ├── build.rs                    # Compiles C shim via cc crate
│   │   ├── Info.plist                  # LSUIElement=true; NSScreenCaptureUsageDescription
│   │   ├── entitlements.plist          # screen-recording; app-sandbox
│   │   └── src/                        # capture/, encode/, network/, ui/, …
│   └── renderd-viewer/                 # Windows viewer binary
│       └── src/                        # network/, decode/, render/, ui/, …
│
├── tools/
│   ├── latency-bench/                  # End-to-end pipeline latency benchmark
│   ├── proto-gen/                      # Regenerates renderd-proto from .proto
│   └── bundle-host/                    # Assembles and signs macOS .app bundle
│
├── proto/
│   └── renderd.proto                   # Source of truth for all control-plane messages
│
├── shaders/
│   └── yuv_to_rgb.hlsl                 # HLSL BT.709 / BT.2020 YUV→RGB shader
│
├── templates/
│   ├── renderd-host.default.toml       # Canonical macOS host configuration
│   └── renderd-viewer.default.toml     # Canonical Windows viewer configuration
│
├── docs/
│   ├── RFC-0001-architecture.md        # Superseded by RFC-0002
│   ├── RFC-0001-review.md              # Review that motivated RFC-0002
│   ├── RFC-0002-architecture.md        # This document (active)
│   ├── REPO-0001-repository.md         # Engineering and repository standards
│   └── ISSUES-0001-milestones.md       # 100-issue milestone roadmap
│
├── .github/
│   ├── CODEOWNERS
│   ├── PULL_REQUEST_TEMPLATE.md
│   ├── ISSUE_TEMPLATE/
│   └── workflows/
│       ├── ci.yml                      # Build, test, lint (all platforms)
│       ├── bench.yml                   # Criterion benchmark runner (nightly)
│       ├── security.yml                # cargo-deny + cargo-audit
│       ├── docs.yml                    # rustdoc generation + deploy
│       ├── release-host.yml            # macOS .app build, sign, notarize
│       ├── release-viewer.yml          # Windows .exe build + installer
│       ├── proto-check.yml             # Verify generated proto is up to date
│       └── typos.yml                   # Spell checking (typos-cli)
│
├── LICENSE                             # MIT
├── README.md
├── CHANGELOG.md                        # Keep-a-Changelog format
└── SECURITY.md                         # Vulnerability disclosure policy
```

---

## 17. macOS Distribution Requirements

This section is new relative to RFC-0001 and is mandatory for the host to function on any standard user's machine.

### 17.1 App Bundle Structure

`renderd-host` must be distributed as a properly structured `.app` bundle:

```
renderd-host.app/
└── Contents/
    ├── MacOS/
    │   └── renderd-host            # Rust binary
    ├── Info.plist                  # Bundle metadata
    └── Resources/                  # Assets (icons, etc.)
```

A bare Rust binary cannot be registered as a Login Item via `SMAppService`. The `.app` bundle structure is required by macOS for Login Item registration, code signing, and Gatekeeper validation.

### 17.2 Required Entitlements

```xml
<!-- entitlements.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" ...>
<plist version="1.0"><dict>
  <key>com.apple.security.app-sandbox</key><true/>
  <key>com.apple.security.screen-recording</key><true/>
  <key>com.apple.security.network.client</key><true/>
  <key>com.apple.security.network.server</key><true/>
</dict></plist>
```

`NSScreenCaptureUsageDescription` must be set in `Info.plist` with a clear user-facing description, e.g.: `"renderd requires screen recording to mirror your display to a paired viewer device on your local network."`

### 17.3 Code Signing and Notarization

```
Build workflow:

1. cargo build --release --target aarch64-apple-darwin
2. Assemble .app bundle
3. codesign --sign "Developer ID Application: ..."
            --entitlements entitlements.plist
            --options runtime
            renderd-host.app
4. xcrun notarytool submit renderd-host.zip
            --apple-id $APPLE_ID
            --team-id $TEAM_ID
            --password $APP_SPECIFIC_PASSWORD
            --wait
5. xcrun stapler staple renderd-host.app
```

Steps 3–5 require:
- Membership in the Apple Developer Program ($99/year).
- A Developer ID Application certificate stored as a GitHub Actions secret.
- An app-specific password for notarytool.

Without notarization, Gatekeeper on macOS 13+ will block the app with "cannot be opened because Apple cannot check it for malicious software." There is no workaround for end users other than manually disabling Gatekeeper, which is unacceptable for a production tool.

---

## 18. Failure Modes and Reconnect Strategy

### 18.1 Network Interruption

The viewer's reconnect watchdog is UUID-aware and integrates mDNS re-discovery:

```
State Machine:

  CONNECTED ──(QUIC close / timeout)──▶ RECONNECTING
  RECONNECTING ──(attempt cached IP, fail)──▶ REDISCOVERING
  REDISCOVERING ──(mDNS browse by host UUID)──▶
    ├── (found, new IP) ──▶ RECONNECTING (with new IP)
    └── (not found after 15s) ──▶ IDLE
  RECONNECTING ──(TLS success)──▶ CONNECTED
  IDLE ──(host rediscovered or user action)──▶ RECONNECTING

Reconnect schedule: 500ms → 1s → 2s → 4s → 5s (capped)
```

**Why mDNS re-discovery is integrated into reconnect:** The most common disconnect scenario on a LAN is the host machine sleeping and waking, which triggers a DHCP lease renewal and IP address change. The cached IP becomes stale on the first reconnect attempt. Without mDNS re-discovery by UUID (not by IP), the viewer enters a 30-second failure loop before giving up. With UUID-based discovery, the new IP is found within 1–2 seconds.

The pairing storage schema includes `host_uuid` alongside the Pair Token so that the reconnect loop can filter mDNS browse results by UUID rather than hostname (which may also change).

During reconnect, the viewer displays a semi-transparent overlay: `"Reconnecting to [host name]…"` with an animated indicator. The window is not closed, preserving the workspace arrangement on the viewer's desktop.

### 18.2 Host Display Change

If the host's display resolution or refresh rate changes (external monitor connect/disconnect, System Settings change), the host:
1. Reconfigures the SCStream to the new display parameters.
2. Sends `StreamReconfigure` on Stream 0 with the new dimensions and codec extra data.
3. Forces an immediate keyframe.

The viewer receives `StreamReconfigure`, reconfigures the D3D12 video decoder and swap chain, and resumes without dropping the connection.

### 18.3 Encoder Overload

If the VTCompressionSession encode queue depth exceeds 1 frame (encoder cannot keep up with capture rate):
1. Drop the oldest unencoded IOSurface frame.
2. If the condition persists for more than 500 ms, reduce the SCStream `minimumFrameInterval` to target 30 FPS and send `StreamReconfigure` to the viewer.
3. Log the condition with microsecond timestamps for debugging.

### 18.4 Decoder Overload

If `decode_time_us` in `PeriodicStats` exceeds 80% of the frame period:
1. ABR controller reduces bitrate.
2. If bitrate reduction does not resolve the condition within 2 seconds, the host sends `StreamReconfigure` targeting a lower frame rate.

### 18.5 Frame Loss Recovery

| Scenario | Response |
|----------|----------|
| Single fragment timeout | Error-concealment decode of partial frame; `KeyframeRequest` sent |
| Single frame loss (all other frames intact) | Decoder conceals; no keyframe request (P-frame temporal reference is still intact for the next frame) |
| Burst loss (≥ 3 consecutive frames) | Immediate `KeyframeRequest`; viewer freezes on last good frame during recovery |
| `frame_id` below window minimum | Discard silently; frame has already been concealed |
| `frame_id` above window maximum | Admit; window advances, dropping oldest incomplete frame |

---

## 19. Latency Budget

Latency targets are segmented by resolution because VideoToolbox encode latency is a hardware-bounded function of resolution and motion, not a tunable parameter.

### 19.1 1080p60 — Target: ≤ 30 ms glass-to-glass

| Stage | Expected (p50) | Expected (p99) |
|-------|---------------|---------------|
| SCStream capture (vsync → IOSurface ready) | ~2 ms | ~3 ms |
| VTCompressionSession HW encode | ~7 ms | ~11 ms |
| QUIC framing + burst send | ~0.5 ms | ~1 ms |
| Network (Gigabit LAN, wired) | ~0.5 ms | ~1.5 ms |
| QUIC recv + fragment reassembly | ~0.3 ms | ~0.8 ms |
| D3D12 HW video decode | ~2 ms | ~4 ms |
| D3D12 render + present | ~1.5 ms | ~2.5 ms |
| Display scanout (with phase sync, §7) | ~2 ms | ~4 ms |
| **Total** | **~16 ms** | **~28 ms** |

Phase sync (§7) reduces the scanout component from an average of 8.3 ms (half a frame period, unsynchronized) to approximately 2 ms (synchronized lead-in). This is the mechanism that makes the 30 ms target achievable.

### 19.2 1440p60 — Target: ≤ 40 ms glass-to-glass

| Stage | Expected (p50) |
|-------|---------------|
| SCStream capture | ~2 ms |
| VTCompressionSession HW encode | ~12 ms |
| Network + QUIC | ~1 ms |
| D3D12 decode + render | ~3.5 ms |
| Display scanout (phase sync) | ~2 ms |
| **Total** | **~21 ms** |

> These estimates are based on community-reported VideoToolbox benchmarks on M1/M2.
> The implementation team **must** validate these numbers with a prototype benchmark
> (`SCStream → VideoToolbox → frame output callback, measure first-byte-out latency`)
> before this RFC is finalized. If measured values differ significantly from the table
> above, the targets must be revised. Shipping with incorrect latency claims damages
> trust more than honest revised targets.

### 19.3 Factors Not in the Critical Path

- **No jitter buffer.** The jitter buffer has been eliminated. LAN jitter on a wired Gigabit switch is consistently below 200 µs. A jitter buffer is not needed and adds directly to glass-to-glass latency. On Wi-Fi LANs, the recommendation is to use 5 GHz 802.11ax where jitter is typically below 2 ms.
- **No DWM compositor latency.** The viewer uses a borderless fullscreen window (not exclusive fullscreen). Windows DWM adds approximately one vsync period of latency in compositor mode. This is mitigated by the presentation clock synchronization (§7) scheduling frame delivery to arrive at the viewer just before its DWM vsync deadline.
- **No OS scheduling jitter in the critical path.** The capture thread's `QOS_CLASS_USER_INTERACTIVE` QoS class reduces wakeup jitter to typically < 1 ms on unloaded Apple Silicon. This is included in the capture stage estimate above.

---

## 20. Future Work

| Feature | Version | Notes |
|---------|---------|-------|
| Linux host | v1.1 | PipeWire capture + VA-API encode; `avahi` for mDNS |
| Linux viewer | v1.1 | VA-API decode + Vulkan render; `avahi` for mDNS |
| Audio streaming | v1.2 | Opus codec; separate QUIC reliable stream |
| Clipboard sync | v1.2 | Text and image via control plane |
| Remote input (keyboard/mouse) | v1.3 | Opt-in HID event stream on host |
| Multi-monitor | v1.3 | One SCStream session per display; viewer window per display |
| Virtual display | v1.4 | CoreDisplay virtual framebuffer; no physical display required on host |
| WAN relay + NAT traversal | v2.0 | TURN-like relay server; consider switching QUIC impl to `msquic` for BBR |
| AV1 codec | v2.0 | When HW decoders are sufficiently universal on both platforms |
| iOS / iPadOS viewer | v2.0 | VideoToolbox decode + Metal render |
| Internet pairing (no LAN required) | v2.0 | Relay-assisted SPAKE2+ pairing; WireGuard-style key exchange |

---

## 21. Open Questions

1. **Phase sync clock offset accuracy:** The vsync phase protocol (§7) uses one-way delay estimation from QUIC RTT. On a LAN, RTT is typically 0.5–1 ms, making the offset estimate accurate to ±0.5 ms. Is this sufficient, or should a PTP-style multi-packet clock synchronization be used? PTP achieves sub-microsecond accuracy but adds significant implementation complexity.

2. **HDR passthrough:** macOS EDR (Extended Dynamic Range) content exists outside the SDR gamut. Should Renderd tone-map to SDR before encoding (simple, universally compatible) or pass HDR10 metadata through to a DXGI HDR swap chain on Windows (correct but complex, requires end-to-end coordination)? Tone-mapping is recommended for v1.0.

3. **`quinn` vs `msquic` for v1.0:** `quinn` is pure Rust and ideal for development velocity. `msquic` is Microsoft's production QUIC implementation with more advanced congestion control and kernel-mode offload support on Windows. For v1.0 LAN focus, `quinn` is recommended. If WAN relay becomes a priority earlier than v2.0, `msquic` should be re-evaluated.

4. **SPAKE2+ library selection:** The RustCrypto `spake2` crate implements SPAKE2, not SPAKE2+ (RFC 9382). A compliant SPAKE2+ implementation must be identified, validated against RFC 9382 test vectors, and added to the CI test suite before the pairing implementation begins.

5. **4K host displays:** A 4K display at 60 FPS with H.265 at 30 Mbps is a stretch goal. The encode latency at 4K (estimated 18–30 ms on M2) may make the 30 ms total target unachievable without VRR on the viewer. Should 4K be supported with a relaxed latency target (e.g., ≤ 60 ms) or excluded until Apple Silicon encode latency improves?

---

## 22. Changes from RFC-0001

This section documents every substantive change relative to RFC-0001 and its rationale.

| Change | RFC-0001 | RFC-0002 | Rationale |
|--------|----------|----------|-----------|
| macOS process model | Launchd daemon | Login Item Agent (SMAppService) | Daemon has no user session; ScreenCaptureKit requires user session + TCC |
| Thread scheduling | Pinned to E-cores | QOS_CLASS_USER_INTERACTIVE | E-cores are 40–60% slower; QoS class is the correct macOS primitive |
| VideoToolbox binding | `objc2` | C shim via `cc` crate | VideoToolbox is a C/CoreFoundation API, not Objective-C |
| QUIC datagram ordering | Claimed "ordered" | Explicitly unordered (RFC 9221 §2.1) | RFC 9221 is explicit; reassembly must handle out-of-order arrival |
| Reassembly buffer | Ring buffer (1–2 frames) | Sliding window (W=4 frames, HashMap) | Out-of-order datagrams require a window keyed by frame_id |
| Jitter buffer | 1–2 frames (16–33 ms) | Eliminated | LAN jitter < 1 ms; jitter buffer made p99 math impossible |
| Latency model | Flat "≤ 30 ms" all resolutions | Tiered by resolution (30 ms / 40 ms) | Encode latency is bounded below by hardware at each resolution |
| Dual-vsync | Not addressed | §7 presentation clock sync protocol | Without sync, average latency = pipeline + half frame period |
| Congestion control | Claimed BBR | NewReno (what quinn actually ships) | BBR is not implemented in quinn as of 2026 |
| ABR feedback interval | 500 ms only | 100 ms reactive + 500 ms proactive | 500 ms is 30 frames; too slow for any congestion on Wi-Fi |
| mDNS (macOS) | `mdns-sd` crate | `dns_sd.h` (Bonjour) | mdns-sd conflicts with mDNSResponder's exclusive port 5353 ownership |
| mDNS fallback | Not mentioned | Manual IP entry in viewer UI | Enterprise networks frequently block multicast |
| SPAKE2+ reference | Expired draft-bar-cfrg-spake2plus-10 | RFC 9382 (August 2023) | Draft expired; RFC 9382 has updated test vectors |
| Window management | Raw Win32 via windows-rs | `winit` + windows-rs | Raw Win32 message loop is ~2,000 lines of error-prone code |
| Token revocation | Not present | Paired Devices panel + Revoke | Compromised Pair Token was permanent with no user recourse |
| Session notification | Not present | macOS UserNotifications on session start | Users must know when screen sharing begins |
| Reconnect + IP change | Retried cached IP only | mDNS re-discovery by UUID on first failure | DHCP renewal after sleep changes the host IP; cached IP is stale |
| HKDF info encoding | Raw UUID concatenation | Canonical UUID with fixed delimiter | Length-ambiguity allows (host-a + viewer-bc) == (host-ab + viewer-c) |
| Protocol versioning | `protocol_version` field, no semantics | `min_required_version` + `Error` message | Silent degradation on version mismatch; explicit rejection required |
| Keyframe interval | 2 seconds | 0.5 seconds + forced on connect | 2s interval causes up to 2s blank screen on viewer connect |
| Certificate expiry | Unspecified | 10 years; auto-renewed at < 180 days remaining | Unspecified expiry defaults to "never" or causes surprise failures |
| DXGI tearing | Unconditionally enabled | Runtime capability check required | Unconditional use crashes on non-VRR displays |
| Fragment deadline | Static 8 ms | Dynamic (frame_period − decode − render); default 12 ms | 8 ms caused false-positive drops when encode took > 8 ms |
| macOS distribution | Not addressed | §17: entitlements, bundle, signing, notarization | Non-notarized binary fails Gatekeeper on every standard user's Mac |
| Max bitrate v1.0 | 100 Mbps | 50 Mbps | 100 Mbps unvalidated; burst-send performance must be benchmarked first |
| Keyframe on connect | Not specified | Forced immediately on session establishment | Without it, viewer waits up to 500 ms for a usable reference frame |

---

## 23. References

| Reference | Notes |
|-----------|-------|
| [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000) | QUIC: A UDP-Based Multiplexed and Secure Transport |
| [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221) | An Unreliable Datagram Extension to QUIC |
| [RFC 9382](https://www.rfc-editor.org/rfc/rfc9382) | SPAKE2+, an Augmented Password-Authenticated Key Exchange (PAKE) Protocol |
| [RFC 6762](https://www.rfc-editor.org/rfc/rfc6762) | Multicast DNS |
| [RFC 6763](https://www.rfc-editor.org/rfc/rfc6763) | DNS-Based Service Discovery |
| [RFC 5869](https://www.rfc-editor.org/rfc/rfc5869) | HMAC-based Extract-and-Expand Key Derivation Function (HKDF) |
| [ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) | Apple low-latency display capture API (macOS 12.3+) |
| [VideoToolbox](https://developer.apple.com/documentation/videotoolbox) | Apple hardware encode/decode framework (C API) |
| [SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice) | macOS 13+ Login Item registration API |
| [D3D12 Video](https://learn.microsoft.com/en-us/windows/win32/direct3d12/video-decoding) | Direct3D 12 hardware video decode API |
| [DwmGetCompositionTimingInfo](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmgetcompositiontiminginfo) | DWM vsync timing API |
| [DXGI_FEATURE_PRESENT_ALLOW_TEARING](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_5/ne-dxgi1_5-dxgi_feature) | DXGI tearing capability check |
| [quinn](https://github.com/quinn-rs/quinn) | Pure Rust QUIC implementation |
| [windows-rs](https://github.com/microsoft/windows-rs) | Rust bindings for Windows APIs |
| [objc2](https://github.com/madsmtm/objc2) | Rust bindings for Objective-C (used for ScreenCaptureKit, not VideoToolbox) |
| [winit](https://github.com/rust-windowing/winit) | Cross-platform window management (used by Bevy, wgpu, Tauri) |
| [cc crate](https://github.com/rust-lang/cc-rs) | Build-time C/C++ compilation from Cargo build scripts |
| Apple Developer: Notarizing macOS software | [developer.apple.com](https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution) |

---

*End of RFC-0002*
