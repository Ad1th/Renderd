# RFC-0001: Renderd Architecture

```
Title:    Renderd — Peer-to-Peer Display Daemon
RFC:      0001
Status:   Draft
Authors:  Renderd Contributors
Created:  2026-08-03
Revised:  —
```

---

## Abstract

Renderd is an open-source, peer-to-peer display daemon that turns any networked computer into a low-latency external display receiver. Unlike remote desktop software (e.g., TeamViewer, RDP), Renderd does not aim to expose or control the remote machine's desktop. Instead, it mirrors a host's screen to a viewer machine with display-class latency (target: <30 ms glass-to-glass on a LAN), comparable to Apple Sidecar or Luna Display, but implemented as a fully cross-platform, open, and modular system.

This document defines the version 1.0 architecture, covering transport selection rationale, encoding pipeline, discovery and pairing, security model, and component boundaries.

---

## Table of Contents

1. [Motivation and Goals](#1-motivation-and-goals)
2. [Non-Goals](#2-non-goals)
3. [Terminology](#3-terminology)
4. [System Overview](#4-system-overview)
5. [Transport Layer Analysis](#5-transport-layer-analysis)
6. [Encoding Pipeline](#6-encoding-pipeline)
7. [Discovery and Pairing](#7-discovery-and-pairing)
8. [Security Model](#8-security-model)
9. [Component Architecture](#9-component-architecture)
10. [Data Flow](#10-data-flow)
11. [Technology Stack](#11-technology-stack)
12. [Repository Layout](#12-repository-layout)
13. [Failure Modes and Reconnect Strategy](#13-failure-modes-and-reconnect-strategy)
14. [Future Work](#14-future-work)
15. [Open Questions](#15-open-questions)
16. [References](#16-references)

---

## 1. Motivation and Goals

### 1.1 Problem Statement

Apple Sidecar and Luna Display solve display extension with sub-30 ms latency over a local network, but they are:

- Locked to Apple hardware (Sidecar) or require proprietary hardware dongles (Luna).
- Not open-source.
- Not extensible to Linux or Windows host scenarios.

Existing open alternatives (VNC, RDP, Parsec) are designed for *remote control*, not *display extension*. They carry the full overhead of input remoting, session management, and connection brokering — none of which are needed for a display receiver.

### 1.2 Goals for v1.0

| Goal | Target |
|------|--------|
| Glass-to-glass latency | ≤ 30 ms on Gigabit LAN |
| Frame rate | 60 FPS minimum, 120 FPS stretch goal |
| Host platform | macOS 13+ (Apple Silicon) |
| Viewer platform | Windows 11 |
| Transport | Peer-to-peer, no relay by default |
| Discovery | Automatic LAN discovery (no manual IP entry) |
| Pairing | Secure one-time pairing (PIN or QR code) |
| Encoding | Hardware-accelerated (VideoToolbox on host, D3D12VA on viewer) |
| UI | Native, modern (no Electron) |
| Reconnect | Automatic, transparent |
| License | MIT or Apache 2.0 |

---

## 2. Non-Goals

The following are explicitly **out of scope for v1.0**:

- Remote keyboard and mouse input
- Clipboard sync
- Audio streaming
- Multi-monitor support
- Virtual display creation
- Internet (WAN) relay / NAT traversal
- Mobile platforms (iOS, Android)
- GPU passthrough or virtualization

---

## 3. Terminology

| Term | Definition |
|------|------------|
| **Host** | The macOS machine whose screen is being shared. Runs `renderd-host`. |
| **Viewer** | The Windows machine that receives and renders the display. Runs `renderd-viewer`. |
| **Control Plane** | The reliable, low-bandwidth channel for signaling, pairing, configuration, and statistics. |
| **Data Plane** | The high-throughput, low-latency channel for compressed video frames. |
| **Frame** | A single compressed video unit (I-frame or P-frame). |
| **Capture** | The act of reading the framebuffer from the host GPU. |
| **Session** | A paired, authenticated streaming connection between one Host and one Viewer. |
| **Pair Token** | A secret derived during the pairing ceremony, used to authenticate future sessions. |

---

## 4. System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│  macOS Host (Apple Silicon)                                         │
│                                                                     │
│  ┌──────────────┐    ┌──────────────────────┐    ┌───────────────┐  │
│  │  ScreenCap   │───▶│ Encoder              │───▶│ renderd-host  │  │
│  │ (SCKit /     │    │ (VideoToolbox H.265) │    │               │  │
│  │  CGDisplay)  │    └──────────────────────┘    │ Control: QUIC │  │
│  └──────────────┘                                │ Data:    QUIC │  │
│                                                  └───────┬───────┘  │
└──────────────────────────────────────────────────────────┼──────────┘
                                                           │ LAN (UDP/QUIC)
┌──────────────────────────────────────────────────────────┼──────────┐
│  Windows 11 Viewer                                        │          │
│                                                  ┌────────▼───────┐ │
│                                                  │ renderd-viewer │ │
│  ┌──────────────┐    ┌──────────────┐            │               │ │
│  │   Renderer   │◀───│   Decoder    │◀───────────│ Control: QUIC │ │
│  │ (D3D12/DXGI) │    │ (D3D12VA /  │            │ Data:    QUIC │ │
│  │              │    │  NVDEC/QSV)  │            └───────────────┘ │
│  └──────────────┘    └──────────────┘                              │
└─────────────────────────────────────────────────────────────────────┘
```

Both `renderd-host` and `renderd-viewer` communicate over two logical channels within one QUIC connection:

- **Stream 0 (Control):** Pairing, capability negotiation, codec parameters, stats feedback.
- **QUIC Datagrams (Data):** Compressed video frames — unreliable, encrypted, minimal overhead.

---

## 5. Transport Layer Analysis

This section evaluates candidate transport protocols against Renderd's requirements: sub-30 ms latency, 60+ FPS, hardware-encoded video on LAN, and P2P operation without a broker.

### 5.1 Raw UDP

**Description:** Custom framing directly over UDP sockets. Used by Luna Display and early Parsec versions.

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ Theoretical minimum — no protocol overhead |
| Reliability | ❌ Must implement retransmit, reorder, FEC from scratch |
| Congestion control | ❌ None built-in; risks saturating the LAN |
| Encryption | ❌ Must implement manually (DTLS or custom) |
| Multiplexing | ❌ Must implement stream IDs and framing |
| Implementation cost | ❌ Very high — reinventing transport layer |
| Firewall traversal | ⚠️ Blocked by many NAT configurations |

**Verdict:** Best performance ceiling, but impractical to build correctly. Not recommended unless QUIC proves insufficient after benchmarking.

---

### 5.2 RTP / RTSP

**Description:** Real-time Transport Protocol (RFC 3550), originally designed for multimedia streaming. Used in WebRTC's media path, VoIP, and IPTV.

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ Designed for real-time; no inherent buffering |
| Reliability | ⚠️ RTP is UDP-based; RTCP provides feedback but no retransmit |
| Congestion control | ⚠️ RTCP-based feedback; not as sophisticated as BBR |
| Encryption | ⚠️ SRTP is mature but requires DTLS key exchange setup |
| Multiplexing | ⚠️ Separate RTP sessions per stream; no unified connection |
| Codec integration | ✅ Standard payload formats for H.264/H.265/AV1 exist |
| Tooling | ✅ GStreamer, FFmpeg natively output RTP |
| P2P complexity | ❌ Needs RTSP signaling layer; typically involves a server |
| Implementation cost | ⚠️ Medium — libraries exist but integration is non-trivial |

**Verdict:** Viable for the data plane, but pairing RTP with a custom P2P signaling layer adds complexity. Loses congestion-control and multiplexing advantages vs. QUIC. Not recommended as primary transport.

---

### 5.3 WebRTC

**Description:** A browser-native stack combining ICE (NAT traversal), DTLS (key exchange), SRTP (media), and SCTP (data channels). Adopted outside browsers via libwebrtc.

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ Designed for real-time; modern implementations achieve <50 ms |
| NAT traversal | ✅ Best-in-class via ICE/STUN/TURN |
| Congestion control | ✅ GCC (Google Congestion Control) + transport-cc |
| Encryption | ✅ Mandatory DTLS-SRTP |
| Codec support | ✅ H.264, VP8/VP9, AV1; H.265 via non-standard extensions |
| Hardware accel | ⚠️ libwebrtc has partial HW accel; requires careful integration |
| Binary size | ❌ libwebrtc is 100–200 MB compiled; complex Chromium-derived build |
| Customizability | ❌ Very difficult to modify internals |
| LAN-only use | ⚠️ ICE machinery is overhead for pure LAN; can be disabled but awkward |
| H.265 support | ❌ Not standardized in WebRTC spec |
| Implementation cost | ⚠️ Medium-High — libwebrtc integration is notoriously painful |

**Verdict:** Excellent for WAN / NAT scenarios (future v2 feature). For pure LAN v1.0, its complexity and binary size are unjustified. H.265 not standardized. Not recommended for v1.0; revisit for WAN relay in a future version.

---

### 5.4 QUIC

**Description:** IETF-standardized (RFC 9000) multiplexed, encrypted transport over UDP. Designed to replace TCP+TLS for HTTP/3 but fully usable as a general transport. Implementations: `quiche` (Cloudflare, Rust/C), `msquic` (Microsoft, C), `quinn` (pure Rust).

| Dimension | Assessment |
|-----------|------------|
| Latency | ✅ 0-RTT resumption; no TCP handshake; stream-level delivery |
| Reliability | ✅ Per-stream reliable delivery; unreliable datagrams via RFC 9221 |
| Congestion control | ✅ Pluggable (NewReno, CUBIC, BBR); BBR excellent for LAN |
| Encryption | ✅ TLS 1.3 mandatory, built-in |
| Multiplexing | ✅ First-class streams; no HOL blocking between streams |
| Binary size | ✅ `quinn` ~5 MB compiled |
| H.265 support | ✅ Not a transport concern — carries any bytes |
| Datagram support | ✅ RFC 9221 QUIC datagrams = unreliable, encrypted UDP |
| Customizability | ✅ Full control over framing and stream usage |
| LAN performance | ✅ Minimal overhead vs. raw UDP; TLS 1.3 HW offload on modern NICs |
| P2P | ✅ Peer-initiated connections; no broker needed on LAN |
| Implementation cost | ✅ Low-Medium with `quinn` in Rust |

**Verdict:** **Strongly recommended.** QUIC gives Renderd raw UDP performance with production-grade encryption, multiplexing, and congestion control at no extra implementation cost.

---

### 5.5 Recommendation: QUIC with Dual-Channel Design

```
┌─────────────────────────────────────────────────────────┐
│  Single QUIC Connection (UDP, TLS 1.3)                  │
│                                                         │
│  ┌──────────────────────┐  ┌─────────────────────────┐  │
│  │  Stream 0            │  │  QUIC Datagrams         │  │
│  │  (Reliable, ordered) │  │  (Unreliable, ordered)  │  │
│  │                      │  │                         │  │
│  │  • Handshake/pairing │  │  • Video frames         │  │
│  │  • Codec negotiation │  │  • Frame metadata       │  │
│  │  • Stats (RTCP-like) │  │  • Timestamp / seq num  │  │
│  │  • Keepalive         │  │                         │  │
│  └──────────────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

- **Control messages** travel on a reliable QUIC stream. Occasional 1–5 ms retransmit delay is acceptable.
- **Video frames** travel as **QUIC datagrams** (RFC 9221). If a frame is lost, the encoder's next P-frame naturally recovers, or the viewer requests a keyframe via Stream 0. This avoids TCP's "wait for retransmit → stutter" problem.
- **BBR congestion control** used for its throughput-delay tradeoff superiority on LAN paths with shallow buffers.

---

## 6. Encoding Pipeline

### 6.1 Codec Selection

| Codec | HW Encoder (macOS AS) | HW Decoder (Win11) | Latency | Compression | Notes |
|-------|-----------------------|--------------------|---------|-------------|-------|
| H.264 | ✅ VideoToolbox | ✅ DXVA2 / D3D12VA / NVDEC / QSV | ✅ Low | ⚠️ Medium | Universal; 8-bit only |
| H.265 | ✅ VideoToolbox (Apple Silicon) | ✅ NVDEC / QSV / D3D12VA | ✅ Low | ✅ High | 10-bit HDR; **recommended** |
| AV1 | ✅ VideoToolbox (M3+) | ⚠️ RTX 30+, Arc — not universal | ⚠️ Higher | ✅ Best | Future codec; not safe for v1.0 |
| VP9 | ❌ No HW encoder on macOS | ✅ Software only | ❌ High | ✅ Good | No HW path; unsuitable |

**Recommendation: H.265 (HEVC) primary, H.264 fallback.**

Rationale:
- All Apple Silicon M-series chips have a VideoToolbox H.265 hardware encoder.
- Windows 11 with any modern AMD/NVIDIA/Intel GPU has a hardware H.265 decoder.
- H.265 achieves ~40% better compression vs H.264, directly reducing frame transmission time.
- 10-bit encoding eliminates banding on typical UI content.
- Codec is negotiated at session start; H.264 fallback ensures broad compatibility.

### 6.2 Hardware Acceleration

**Host (macOS, Apple Silicon):**

```
Display → ScreenCaptureKit (SCStream)
       → CMSampleBuffer (IOSurface-backed, GPU-resident)
       → VTCompressionSession (VideoToolbox)
            ├── RequireHardwareAcceleratedVideoEncoder = true
            ├── RealTime = true
            ├── MaxKeyFrameIntervalDuration = 2s
            ├── AverageBitRate = adaptive
            └── ExpectedFrameRate = 60
       → NAL unit stream (Annex-B)
       → QUIC datagram framing
```

Key points:
- `SCStream` provides GPU-resident `IOSurface` frames with minimal CPU involvement.
- VideoToolbox encoder accepts `IOSurface` directly — entire capture→encode pipeline stays on-GPU.
- `RealTime = true` prioritizes latency over quality.
- No B-frames: I + P frames only for lowest encode latency.
- If no hardware encoder is available, fail loudly rather than silent software fallback.

**Viewer (Windows 11):**

```
QUIC datagram → Frame reassembly buffer
             → D3D12 Video Decode (ID3D12VideoDecoder)
                  ├── Primary: D3D12 Video Decode API (driver-agnostic)
                  ├── Fallback: NVDEC (CUDA/NV12) for NVIDIA GPUs
                  └── Fallback: Intel QSV (MFX) for Intel Arc
             → Decoded NV12/P010 surface (GPU memory)
             → D3D12 render pass (YUV→RGB shader, aspect-ratio letterbox)
             → DXGI swap chain (fullscreen, DXGI_PRESENT_ALLOW_TEARING)
```

Key points:
- `DXGI_PRESENT_ALLOW_TEARING` with VRR/FreeSync/G-Sync support for minimum scanout latency.
- Decoded surfaces stay in GPU memory; zero CPU readback in the display path.
- Jitter buffer: 1–2 frames (~16–33 ms) absorbs network jitter while meeting the 30 ms target.

### 6.3 Adaptive Bitrate

Renderd implements a simplified ABR loop:

```
Viewer → Stats feedback (Stream 0, every 500ms):
         • Estimated receive bandwidth (kbps)
         • Frame loss rate (last 1 second, 0.0–1.0)
         • Jitter (microseconds)
         • Decode time (microseconds)

Host → Adjusts:
        • Target bitrate: -20% on loss spike, +5% on clear path
        • Keyframe injection on sudden loss burst
        • Frame skip if encoder queue depth > 1 frame
```

Bitrate range:
- Minimum: 5 Mbps (720p equivalent quality)
- Default: 30 Mbps (1440p HEVC)
- Maximum: 100 Mbps (4K/120 FPS future support)

### 6.4 Latency Budget

Target: **≤ 30 ms glass-to-glass** on Gigabit LAN.

| Stage | Budget |
|-------|--------|
| Screen capture (SCKit vsync → IOSurface ready) | ~2 ms |
| VideoToolbox HW encode (real-time mode) | ~5 ms |
| QUIC framing + kernel UDP send | ~0.5 ms |
| Network transmission (Gigabit LAN, ~1 ms RTT) | ~1 ms |
| QUIC receive + frame reassembly | ~0.5 ms |
| D3D12 hardware decode | ~3 ms |
| D3D12 render + present | ~2 ms |
| Display scanout (60 Hz period = 16.7 ms) | ~8 ms avg |
| **Total (p50)** | **~22 ms** |
| **Total (p99 with jitter)** | **~28 ms** |

> The largest variable is display scanout timing. A VRR display on the viewer can reduce this component near zero.

---

## 7. Discovery and Pairing

### 7.1 LAN Discovery (mDNS / DNS-SD)

Renderd uses **mDNS** (RFC 6762) and **DNS-SD** (RFC 6763) for zero-configuration discovery — the same mechanism used by AirPlay and Bonjour.

**Service type:** `_renderd._udp.local.`

**TXT records advertised by Host:**
```
version=1
id=<host-uuid>           # Stable UUID, persisted across reboots
name=<hostname>          # User-visible name (e.g., "Adith's Mac")
display=1                # Number of displays available
auth=pin                 # Authentication method: pin | cert
```

**Discovery flow:**
1. `renderd-host` registers `_renderd._udp.local.` via system mDNS.
2. `renderd-viewer` browses for `_renderd._udp.local.` on startup and shows discovered hosts in UI.
3. User selects a host in the Viewer UI to initiate pairing.

**Libraries:**
- macOS: `dns_sd.h` (system Bonjour, zero dependency)
- Windows: `DnsServiceRegister` (Win32 API, available since Win10 v1703)
- Cross-platform Rust: `mdns-sd` crate

### 7.2 Secure Pairing (SPAKE2+)

Pairing is a **one-time ceremony** that produces a long-lived shared secret (Pair Token) stored in the system keychain on both sides.

**Protocol: SPAKE2+** (draft-bar-cfrg-spake2plus) — a password-authenticated key exchange (PAKE) that:
- Is resistant to offline dictionary attacks even with a 6-digit PIN.
- Provides mutual authentication (both sides prove knowledge of the PIN).
- Is standardized and used in Apple's HomeKit pairing protocol.

```
Pairing Ceremony:

1. User clicks "Pair" in Viewer UI.
   Viewer generates ephemeral SPAKE2+ keypair.

2. Host displays a 6-digit PIN in its menu bar UI.

3. User types PIN into Viewer UI.

4. Viewer initiates QUIC connection to Host's mDNS-discovered address.

5. SPAKE2+ exchange over QUIC Stream 0:
     Viewer → Host:  SPAKE2+ message A
     Host   → Viewer: SPAKE2+ message B
     Both sides independently derive shared key K.
     Viewer → Host:  HMAC-SHA256(K, "viewer-verify")
     Host   → Viewer: HMAC-SHA256(K, "host-verify")

6. Both derive:
     PairToken = HKDF(K, "renderd-pair-token", host-uuid || viewer-uuid)

7. PairToken stored in:
     macOS:   Keychain Services (kSecClassGenericPassword)
     Windows: Windows Credential Manager (CredWrite)

8. Host stores viewer's derived public certificate for future sessions.
```

### 7.3 Session Authentication (Post-Pairing)

After the one-time pairing, every subsequent session uses **mutual TLS** with certificates derived from the Pair Token:

```
1. Viewer discovers Host via mDNS.
2. Viewer initiates QUIC connection.
3. QUIC's TLS 1.3 handshake authenticates both sides using stored certs.
4. Host verifies Viewer cert against its known-viewers list.
5. Streaming begins immediately on success; connection rejected on cert mismatch.
```

Rate limiting: 5 failed PIN attempts → 60-second lockout.

---

## 8. Security Model

| Concern | Mitigation |
|---------|------------|
| Eavesdropping | All data encrypted via QUIC/TLS 1.3 (mandatory, no downgrade) |
| MITM during pairing | SPAKE2+ PAKE — attacker without PIN cannot derive session key |
| Replay attacks | TLS 1.3 record sequence numbers; QUIC packet numbers |
| Unauthorized connection | Mutual TLS after pairing; unknown certs rejected |
| PIN brute-force | Rate limiting: 5 failures → 60s lockout |
| Display data leakage | No cloud relay in v1.0; data stays on LAN |
| Key storage | System keychain (macOS Keychain / Windows Credential Manager) |
| DoS amplification | QUIC stateless retry tokens prevent reflection attacks |

**Out of scope for v1.0:** WAN relay, TURN servers, end-to-end encryption over untrusted networks.

---

## 9. Component Architecture

### 9.1 Host Agent (`renderd-host`)

Runs as a **background daemon with a menu bar icon** on macOS.

```
renderd-host/
├── capture/
│   └── screencapture_kit.rs    # SCStream integration via objc2
├── encode/
│   └── videotoolbox.rs         # VTCompressionSession wrapper
├── network/
│   ├── quic_server.rs          # quinn QUIC server
│   ├── control_plane.rs        # Stream 0 handler
│   └── data_plane.rs           # Datagram sender
├── discovery/
│   └── mdns_advertise.rs       # mDNS service registration
├── pairing/
│   └── spake2.rs               # SPAKE2+ pairing ceremony
├── keychain/
│   └── macos_keychain.rs       # Pair token storage
├── abr/
│   └── controller.rs           # Adaptive bitrate logic
└── ui/
    └── menubar.rs              # macOS menu bar item (tray-icon crate)
```

**Process model:** Single process, async (Tokio). Capture and encode run as dedicated OS threads pinned to efficiency cores; network I/O runs on Tokio. Communication via lock-free ring buffers.

### 9.2 Viewer Client (`renderd-viewer`)

Runs as a **fullscreen borderless window** on Windows.

```
renderd-viewer/
├── network/
│   ├── quic_client.rs          # quinn QUIC client
│   ├── control_plane.rs        # Stream 0 handler
│   └── data_plane.rs           # Datagram receiver + jitter buffer
├── decode/
│   └── d3d12_decode.rs         # D3D12 Video Decode (windows-rs)
├── render/
│   └── d3d12_renderer.rs       # D3D12 swap chain + YUV→RGB shader
├── discovery/
│   └── mdns_browse.rs          # mDNS host discovery
├── pairing/
│   └── spake2.rs               # SPAKE2+ client side
├── keychain/
│   └── windows_credential.rs   # Windows Credential Manager
├── abr/
│   └── feedback.rs             # Bandwidth estimation + feedback sender
├── reconnect/
│   └── watchdog.rs             # Auto-reconnect state machine
└── ui/
    ├── window.rs               # Win32 borderless fullscreen window
    └── settings.rs             # Settings panel (native Win32 dialogs)
```

**Jitter Buffer:** Ring buffer of max 2 frames. If a frame arrives >16 ms past its deadline, it is dropped and a keyframe is requested.

### 9.3 Shared Library (`librenderd`)

A Rust crate shared between host and viewer:

- `protocol/` — Control plane message definitions (protobuf/prost)
- `codec_params/` — SPS/PPS/VPS parsing and negotiation
- `frame_id/` — Frame sequence number management and loss detection
- `stats/` — Ring-buffer statistics collection
- `error/` — Unified error types

### 9.4 Control Plane Protocol (protobuf)

```protobuf
syntax = "proto3";
package renderd;

// Viewer → Host: immediately after TLS handshake
message SessionHello {
  uint32 protocol_version = 1;
  string viewer_id = 2;
  repeated string supported_codecs = 3;   // ["hevc", "h264"]
  uint32 max_decode_bitrate_kbps = 4;
  DisplayInfo display = 5;
}

message DisplayInfo {
  uint32 width = 1;
  uint32 height = 2;
  float refresh_rate = 3;
  bool vrr_supported = 4;
}

// Host → Viewer: session configuration
message SessionConfig {
  string selected_codec = 1;     // "hevc"
  uint32 width = 2;
  uint32 height = 3;
  float frame_rate = 4;
  uint32 initial_bitrate_kbps = 5;
  bytes codec_extra_data = 6;    // HEVC VPS+SPS+PPS or H.264 SPS+PPS
}

// Viewer → Host: every 500ms
message Stats {
  float receive_bandwidth_kbps = 1;
  float loss_rate = 2;           // 0.0–1.0
  uint32 jitter_us = 3;
  uint32 decode_time_us = 4;
  uint64 last_frame_id = 5;
}

// Viewer → Host: on loss spike
message KeyframeRequest {
  uint64 after_frame_id = 1;
}

// Host → Viewer: ABR adjustment
message BitrateAdjust {
  uint32 new_bitrate_kbps = 1;
}
```

### 9.5 Data Plane Frame Format

Video frames are transmitted as **QUIC datagrams** with a minimal 16-byte header:

```
Frame Datagram Layout (per datagram):
┌────────────────────────────────────────────────────────────────────┐
│  Header (16 bytes)                                                 │
│  ┌──────────┬──────────┬──────────┬──────────┬──────────────────┐  │
│  │ frame_id │  frag_id │frag_total│  flags   │  capture_ts_us   │  │
│  │  8 bytes │  2 bytes │  2 bytes │  2 bytes │    2 bytes (lo)  │  │
│  └──────────┴──────────┴──────────┴──────────┴──────────────────┘  │
│  Payload: NAL unit fragment (~1150 bytes max)                       │
└────────────────────────────────────────────────────────────────────┘

Flags:
  bit 0: is_keyframe
  bit 1: is_last_fragment
  bit 2: end_of_sequence
```

- **Fragmentation:** Frames exceeding QUIC PMTU (~1200 bytes) are split into fragments, reassembled by `(frame_id, frag_id)`.
- **Frame deadline:** If all fragments of a frame don't arrive within 8 ms of the first, the frame is dropped and a keyframe requested.

---

## 10. Data Flow

```
                HOST                                     VIEWER
                ────                                     ──────
Display vsync
     │
     ▼
SCStream callback
(IOSurface, GPU)
     │
     ▼ (on-GPU)
VideoToolbox HW encode
(H.265, real-time)
     │
     ▼
NAL unit stream
     │
Fragment datagrams ──────────── UDP / QUIC ──────────▶ Receive datagrams
                                                              │
                                                       Reassemble frame
                                                              │
                                                      D3D12 HW decode
                                                      (NV12 GPU surface)
                                                              │
                                                     D3D12 render + present
                                                     (YUV→RGB, DXGI swap)
                                                              │
                                                       Display output

Feedback path (every 500ms):
                ◀──────── Stream 0 ──────── Stats / KeyframeRequest
     │
ABR Controller
     │
Adjust VTCompressionSession bitrate / inject keyframe
```

---

## 11. Technology Stack

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | **Rust** | Memory safety without GC; excellent async/systems libs; cross-platform |
| Async runtime | **Tokio** | De facto standard; integrates natively with quinn |
| QUIC | **quinn** (pure Rust) | RFC 9000 + RFC 9221 datagrams; clean async API |
| Protobuf | **prost** | Rust-native protobuf; no C++ dependency |
| macOS capture | **ScreenCaptureKit** via `objc2` | Lowest-latency macOS capture API; GPU-resident frames |
| macOS encode | **VideoToolbox** via `objc2` | Hardware H.265; mandatory for latency target |
| Windows decode | **windows-rs** + D3D12 Video | Microsoft's official Rust WinAPI bindings |
| Windows render | **D3D12** via `windows-rs` | Low-level; minimal driver overhead |
| mDNS | **mdns-sd** crate | Pure Rust, cross-platform |
| PAKE | **spake2** crate (RustCrypto) | SPAKE2+ implementation |
| TLS certs | **rcgen** crate | Self-signed cert generation for mutual TLS |
| Keychain (macOS) | **security-framework** crate | Safe Rust wrapper for Keychain Services |
| Keychain (Windows) | **windows-rs** CredentialManager | Native credential storage |
| UI (macOS) | **tray-icon** crate + AppKit | System-native menu bar icon |
| UI (Windows) | **windows-rs** Win32 | Native Win32 window; no framework |
| Build | **Cargo workspace** + **cargo-cross** | Standard Rust; cross-compilation support |
| CI | **GitHub Actions** | macOS runner (host), Windows runner (viewer) |

---

## 12. Repository Layout

```
renderd/
├── Cargo.toml                       # Workspace root
├── crates/
│   ├── librenderd/                  # Shared protocol + utilities
│   │   ├── src/
│   │   │   ├── protocol/            # Protobuf-generated types (prost)
│   │   │   ├── codec_params.rs      # SPS/PPS/VPS parsing
│   │   │   ├── frame_id.rs          # Sequence number management
│   │   │   ├── stats.rs             # Ring-buffer statistics
│   │   │   └── error.rs             # Unified error types
│   │   └── Cargo.toml
│   ├── renderd-host/                # macOS host daemon
│   │   ├── src/
│   │   │   ├── capture/
│   │   │   ├── encode/
│   │   │   ├── network/
│   │   │   ├── discovery/
│   │   │   ├── pairing/
│   │   │   ├── keychain/
│   │   │   ├── abr/
│   │   │   └── ui/
│   │   └── Cargo.toml
│   └── renderd-viewer/              # Windows viewer client
│       ├── src/
│       │   ├── network/
│       │   ├── decode/
│       │   ├── render/
│       │   ├── discovery/
│       │   ├── pairing/
│       │   ├── keychain/
│       │   ├── abr/
│       │   ├── reconnect/
│       │   └── ui/
│       └── Cargo.toml
├── docs/
│   ├── RFC-0001-architecture.md     # This document
│   └── RFC-0002-protocol.md         # Detailed protocol spec (future)
├── proto/
│   └── control.proto                # Protobuf definitions
├── shaders/
│   └── yuv_to_rgb.hlsl              # HLSL YUV→RGB shader for viewer
├── scripts/
│   ├── build-host.sh                # macOS build script
│   └── build-viewer.ps1             # Windows build script (PowerShell)
├── .github/
│   └── workflows/
│       ├── host-ci.yml              # macOS CI
│       └── viewer-ci.yml            # Windows CI
├── LICENSE                          # MIT or Apache 2.0
└── README.md
```

---

## 13. Failure Modes and Reconnect Strategy

### 13.1 Network Interruption

The viewer's reconnect watchdog monitors QUIC connection state:

```
State Machine:

  CONNECTED ──(disconnect)──▶ RECONNECTING
  RECONNECTING ──(success)──▶ CONNECTED
  RECONNECTING ──(30s timeout)──▶ IDLE
  IDLE ──(user action or host rediscovered)──▶ RECONNECTING
```

- **Schedule:** Exponential backoff starting at 500 ms, capped at 5 seconds.
- **UX:** Viewer displays a semi-transparent "Reconnecting…" overlay rather than closing the window, preserving workspace arrangement.

### 13.2 Encoder Overload

If VideoToolbox encoder queue depth exceeds 1 frame:
1. Drop oldest unencoded capture frame.
2. Log warning with timestamp.
3. If persistent >1 second: reduce capture rate to 30 FPS temporarily; notify viewer via control plane.

### 13.3 Decoder Overload

If D3D12 decoder is saturated:
1. Viewer sends `Stats` with elevated `decode_time_us`.
2. ABR controller on host reduces bitrate.
3. If saturated for >2 seconds: host also reduces frame rate.

### 13.4 Frame Loss Recovery

| Scenario | Recovery |
|----------|----------|
| Single frame loss | Decoder conceals with previous frame; monitor for block corruption |
| Block corruption detected | Request keyframe via Stream 0 |
| Burst loss (>3 consecutive frames) | Immediate keyframe request; viewer freezes on last good frame during recovery |
| Fragment timeout (>8 ms incomplete) | Drop partial frame; request keyframe |

---

## 14. Future Work

| Feature | Version | Notes |
|---------|---------|-------|
| Linux host | v1.1 | PipeWire + VA-API capture/encode |
| Linux viewer | v1.1 | VA-API decode + Vulkan render |
| Audio streaming | v1.2 | Opus codec over QUIC reliable stream |
| Clipboard sync | v1.2 | Text + image via control plane messages |
| Remote input (kbd/mouse) | v1.3 | HID event stream; opt-in on host |
| Multi-monitor | v1.3 | Multiple SCStream sessions, multiple viewers |
| Virtual display | v1.4 | CoreDisplay virtual framebuffer (no physical display needed) |
| WAN relay | v2.0 | TURN-like relay + WebRTC ICE fallback |
| AV1 codec | v2.0 | When HW decoders are sufficiently universal |
| iOS/iPadOS viewer | v2.0 | VideoToolbox decode + Metal render |

---

## 15. Open Questions

1. **QUIC implementation:** `quinn` vs `quiche` — `quinn` is fully async-Rust (cleaner Tokio integration) while `quiche` is a C library (potentially higher raw throughput). Benchmark both at the prototype stage before finalizing.

2. **Frame pacing:** Should the host pace frame delivery to the viewer's reported refresh rate, or send at maximum rate and let the viewer handle pacing? Pacing at source reduces jitter buffer requirements.

3. **Color space / HDR:** macOS EDR supports extended dynamic range. Should Renderd pass through HDR10 metadata, or tone-map to SDR? HDR passthrough requires end-to-end coordination with the DXGI HDR swap chain on Windows.

4. **Display resolution matching:** When the viewer goes fullscreen, should Renderd attempt to match the viewer display resolution/refresh exactly to the host? Requires DXGI mode change (disruptive) on Windows.

5. **Certificate model:** Current design uses self-signed certs stored post-pairing. Is this sufficient, or should a lightweight Renderd PKI with short-lived certs issued at pairing time be introduced?

6. **Build system:** Cargo workspace is ideal for pure Rust. However, VideoToolbox and D3D12 FFI may need thin C shims for APIs not yet covered by `objc2` or `windows-rs`. If this grows significant, evaluate hybrid Cargo+CMake.

---

## 16. References

| Reference | Notes |
|-----------|-------|
| [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000) | QUIC Transport Protocol |
| [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221) | An Unreliable Datagram Extension to QUIC |
| [RFC 6762](https://www.rfc-editor.org/rfc/rfc6762) | Multicast DNS |
| [RFC 6763](https://www.rfc-editor.org/rfc/rfc6763) | DNS-Based Service Discovery |
| [SPAKE2+](https://www.ietf.org/archive/id/draft-bar-cfrg-spake2plus-10.txt) | Password-Authenticated Key Exchange |
| [ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) | Apple low-latency capture API |
| [VideoToolbox](https://developer.apple.com/documentation/videotoolbox) | Apple HW encode/decode API |
| [D3D12 Video](https://learn.microsoft.com/en-us/windows/win32/direct3d12/video-decoding) | Microsoft HW decode API |
| [quinn](https://github.com/quinn-rs/quinn) | Rust QUIC implementation |
| [windows-rs](https://github.com/microsoft/windows-rs) | Rust bindings for Windows APIs |
| [objc2](https://github.com/madsmtm/objc2) | Rust bindings for Objective-C / Apple frameworks |
| GCC (Google Congestion Control) | WebRTC congestion algorithm; inspiration for Renderd ABR |

---

*End of RFC-0001*

---

> **Immediate next steps:**
> - [ ] Prototype QUIC datagram round-trip latency on LAN (`quinn` datagram echo bench)
> - [ ] Prototype SCStream → VideoToolbox pipeline on Apple Silicon (measure capture+encode latency)
> - [ ] Prototype D3D12 Video Decode → swap chain on Windows (measure decode+present latency)
> - [ ] Draft RFC-0002: Detailed Control Plane Protocol Specification
> - [ ] Choose `quinn` vs `quiche` based on benchmark results
> - [ ] Initialize GitHub repo with CI for both platforms
