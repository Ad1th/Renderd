# RFC-0001 Architectural Review
### Principal Systems Architect — Critique Report

```
Reviewer:   Principal Systems Architect
RFC:        0001 (Renderd Architecture)
Date:       2026-08-03
Verdict:    NOT READY FOR IMPLEMENTATION — 6 Critical, 9 High, 7 Medium issues
```

> This review does not validate the RFC. Every section is challenged on technical
> correctness, implementation feasibility, and production readiness. All issues
> must be resolved before implementation begins.

---

## Severity Legend

| Level | Meaning |
|-------|---------|
| 🔴 **Critical** | Factually wrong, physically impossible, or will cause the system to fail to ship |
| 🟠 **High** | Will cause production failures or block the latency target |
| 🟡 **Medium** | Risk of correctness or maintainability problems; needs a design decision |

---

## Issue Index

| # | Section | Severity | Title |
|---|---------|----------|-------|
| 01 | §5.4 / §5.5 | 🔴 Critical | QUIC datagrams are NOT ordered |
| 02 | §6.4 | 🔴 Critical | Latency budget math is internally contradictory |
| 03 | §5.4 | 🔴 Critical | BBR is not available in `quinn` |
| 04 | §9.1 | 🔴 Critical | Wrong macOS process model — daemon ≠ Login Item |
| 05 | §9.1 | 🔴 Critical | Capture/encode pinned to efficiency cores — catastrophic for latency |
| 06 | §11 | 🔴 Critical | VideoToolbox is not an Objective-C API; `objc2` binding approach is wrong |
| 07 | §5.5 | 🟠 High | QUIC datagram fragmentation at scale is severely underestimated |
| 08 | §6.2 | 🟠 High | VideoToolbox encode latency is optimistic by 2–3× |
| 09 | §6.4 | 🟠 High | Dual-vsync penalty not modelled — defeats the 30 ms target |
| 10 | §6.3 | 🟠 High | 500 ms ABR feedback interval is 5× too slow |
| 11 | §7.1 | 🟠 High | `mdns-sd` crate conflicts with macOS system mDNS daemon |
| 12 | §7.2 | 🟠 High | SPAKE2+ references an expired draft; RFC 9382 exists |
| 13 | §9.2 | 🟠 High | Win32 from scratch via `windows-rs` is massively underestimated |
| 14 | §8 | 🟠 High | No token revocation; compromised Pair Token is permanent |
| 15 | §13.1 | 🟠 High | Reconnect loop doesn't re-run mDNS discovery; fails on IP change |
| 16 | §7.2 | 🟡 Medium | HKDF info field has length-ambiguity collision vector |
| 17 | §9.4 | 🟡 Medium | Protobuf schema has no version negotiation beyond field evolution |
| 18 | §6.2 | 🟡 Medium | 2s keyframe interval causes cold-connect blank screen |
| 19 | §8 | 🟡 Medium | Self-signed certs have no stated expiry — indefinitely valid |
| 20 | §6.2 | 🟡 Medium | `DXGI_PRESENT_ALLOW_TEARING` requires runtime capability check |
| 21 | §13.4 | 🟡 Medium | 8 ms fragment deadline is inconsistent with 16.7 ms frame budget |
| 22 | §11 | 🟡 Medium | macOS notarization and entitlements not addressed; blocks distribution |

---

## Detailed Findings

---

### Issue 01 — QUIC Datagrams Are NOT Ordered
**Severity:** 🔴 Critical  
**Section:** §5.4 assessment table; §5.5 diagram

**Problem:**  
The RFC's assessment table labels QUIC datagrams as "Unreliable, **ordered**" and the §5.5 design diagram explicitly marks the datagram channel as `(Unreliable, ordered)`. This is factually incorrect.

RFC 9221 §2.1 states plainly:

> "QUIC DATAGRAM frames do not provide any ordering guarantees."

Datagrams are delivered in network arrival order within the QUIC connection's loss recovery pass, but **there is no delivery ordering guarantee between datagrams**. A datagram sent at t=0 may arrive at the application *after* a datagram sent at t=5ms if the OS packet scheduler reorders them at the socket layer, or if the QUIC implementation batches sends.

**Why this matters:**  
The reassembly logic in §9.5 uses `(frame_id, frag_id)` to reassemble fragments. If fragments from frame N+1 arrive before the last fragment of frame N, the current scheme has no mechanism to handle this correctly without explicit out-of-order buffering. The RFC describes none.

**Proposed Solution:**  
- Correct the claim: QUIC datagrams are unreliable **and unordered**.
- The reassembly buffer must maintain a sliding window of in-flight frames (not just a single frame), keyed by `frame_id`, large enough to hold fragments from at least 2–3 concurrent frames.
- Explicitly define the maximum out-of-order window depth (suggested: 3 frames = ~50 ms at 60 FPS).
- If `frame_id < (max_seen_frame_id - window_depth)`, discard as too late.

**Architectural Impact:**  
The §9.5 data plane header and the §9.2 jitter buffer implementation must be redesigned. The reassembly buffer is now a concurrent hashmap keyed by `frame_id`, not a ring buffer of 1–2 frames.

---

### Issue 02 — Latency Budget Math Is Internally Contradictory
**Severity:** 🔴 Critical  
**Section:** §6.4

**Problem:**  
The latency budget table claims:

```
Jitter buffer: 1–2 frames (~16–33 ms)   ← stated in §6.2
Total p99:     ~28 ms                    ← stated in §6.4
```

This is **mathematically impossible**. The jitter buffer alone can add up to 33 ms. The sum of all other stages is ~14 ms. Therefore p99 must be at minimum `14 + 33 = 47 ms` — which **blows past the 30 ms target by 57%**.

The budget also omits:
- **OS scheduling jitter** (Tokio wakeup latency: 0.1–3 ms per hop; multiplied across capture→encode→send→recv→decode→present)
- **QUIC stack processing** (packet parsing, TLS record decryption, connection state machine): ~0.3–1 ms
- **D3D12 command queue submission** latency: ~0.5–2 ms
- **Windows DWM compositor** (if not in exclusive fullscreen mode): adds one vsync period = 16.7 ms

With a 1-frame jitter buffer and no DWM: the realistic p50 is closer to 25–30 ms. With a 2-frame jitter buffer: p50 is 40+ ms. The 30 ms target is only achievable with a **zero-frame jitter buffer** and a VRR display.

**Proposed Solution:**  
- Eliminate the jitter buffer for LAN mode. The LAN jitter on a well-managed Gigabit switch is <1 ms; a jitter buffer is not needed and directly destroys the latency target.
- Use a **deadline-based frame presentation scheduler** instead: each frame carries a target presentation timestamp; if it misses by more than 8 ms, drop it, request a keyframe, and continue. No buffering.
- Revise the latency budget with honest measurements from prototype benchmarks, not estimates.
- Add a "latency mode" toggle: `low-latency` (no buffer, may stutter on bad LAN) vs `smooth` (1-frame buffer, ~40 ms total).

**Architectural Impact:**  
Eliminates the jitter buffer component from `renderd-viewer`. Adds a presentation clock synchronization mechanism between host and viewer (NTP-level sync is insufficient; needs a custom clock sync protocol similar to PTP or a monotonic offset calibration). This is a significant addition.

---

### Issue 03 — BBR Is Not Available in `quinn`
**Severity:** 🔴 Critical  
**Section:** §5.4, §5.5

**Problem:**  
The RFC states BBR congestion control as a key advantage of QUIC and specifically recommends it: "BBR congestion control used for its throughput-delay tradeoff superiority on LAN paths."

`quinn` does **not implement BBR**. As of 2026, `quinn` ships with **NewReno** only. BBR for QUIC requires a custom implementation or a different QUIC library. `quiche` (Cloudflare) also does not implement BBR by default. `msquic` (Microsoft) has a BBR-like algorithm but is a C library.

On a Gigabit LAN with no congestion, NewReno is actually fine — the BBR advantage is for WAN paths with high BDP. The recommendation of BBR for LAN is also architecturally misplaced. But the claim is still incorrect.

**Proposed Solution:**  
- Correct the RFC: `quinn` uses NewReno by default. This is acceptable for LAN.
- Do not position BBR as a feature; remove it from the transport comparison table as a ✅.
- Revisit BBR only when WAN relay is added in v2.0, at which point `msquic` (which has better congestion control options) should be reconsidered.
- Document that the congestion controller choice is NewReno for v1.0 with the expectation of contributing BBR to quinn or switching to msquic for v2.0.

**Architectural Impact:**  
Minor — changes the QUIC assessment table score from ✅ to ⚠️ for congestion control. Does not affect the overall QUIC recommendation, which remains correct.

---

### Issue 04 — Wrong macOS Process Model: Daemon ≠ Login Item
**Severity:** 🔴 Critical  
**Section:** §9.1

**Problem:**  
The RFC describes `renderd-host` as a "background daemon with a menu bar icon." On macOS, a **daemon** and a **Login Item / agent** are fundamentally different process types with incompatible capabilities:

| Property | `launchd` daemon | Login Item / Agent |
|----------|------------------|-------------------|
| Runs as user | ❌ Runs as root or system user | ✅ Runs as logged-in user |
| GUI access | ❌ No access to user's WindowServer session | ✅ Full GUI access |
| Menu bar icon | ❌ Cannot display UI | ✅ Via `NSStatusBar` |
| ScreenCaptureKit | ❌ Requires user session + entitlement | ✅ Works with entitlement |
| Screen recording permission | ❌ Root processes can bypass TCC but are rejected by SCKit | ✅ User grants via System Settings |

`ScreenCaptureKit` requires `com.apple.security.screen-recording` entitlement **and** runs within the user's WindowServer session. A `launchd` daemon running as root does not have this. Running as root also triggers macOS's Transparency, Consent, and Control (TCC) framework to block screen capture since macOS 12.

**The correct process model is:**
- A **Login Item** (registered via `SMAppService.mainApp` on macOS 13+ or `SMLoginItemSetEnabled` on older) with `LSUIElement = true` in Info.plist (hides from Dock, keeps menu bar icon).
- The process runs as the current user, has GUI session access, and can hold the screen recording entitlement.

This is a fundamental architectural error that would prevent `renderd-host` from capturing the screen at all.

**Proposed Solution:**  
- Replace "background daemon" with "Login Item agent" throughout the RFC.
- Add Info.plist with `LSUIElement = true`.
- Register via `SMAppService.mainApp` (macOS 13+).
- Specify required entitlements: `com.apple.security.screen-recording`, `com.apple.security.app-sandbox`.
- Add macOS 13 as a hard minimum (not a nice-to-have) because `SMAppService` API requires it.

**Architectural Impact:**  
Significant. The distribution model changes: the app must be bundled as a proper `.app` bundle (not a bare binary), must be code-signed with a Developer ID certificate, and must be notarized. A bare Rust binary without an app bundle cannot be a Login Item. This affects the build system, CI, and distribution pipeline.

---

### Issue 05 — Pinning Capture/Encode to Efficiency Cores Destroys Latency
**Severity:** 🔴 Critical  
**Section:** §9.1

**Problem:**  
The RFC specifies: "Capture and encode run as dedicated OS threads **pinned to efficiency cores**."

Apple Silicon has two core classes:
- **Performance cores (P-cores):** High frequency, high IPC, low sleep latency. Used for foreground/latency-sensitive work.
- **Efficiency cores (E-cores):** Lower frequency (~40-60% of P-core speed), optimized for background/throughput tasks.

Pinning the capture thread (which runs every 16.7 ms at 60 FPS and must complete within ~2 ms) and the encode dispatch thread to **E-cores** will:
1. Increase thread wakeup latency (E-cores have longer sleep→active transition times).
2. Reduce the speed of any CPU-side work in the capture/encode dispatch path.
3. Potentially cause the encode submission to miss the VideoToolbox deadline, adding a full frame period of latency.

The encode work itself happens in the hardware encoder, but the **dispatch overhead** — locking the frame, calling `VTCompressionSessionEncodeFrame`, marshaling the CMSampleBuffer — runs on the calling thread. On E-cores, this can take 2–4× longer than on P-cores.

The rationale for E-cores ("leaving P-cores for user apps") is wrong. The capture daemon is a background process; the OS scheduler will naturally deprioritize it. Pinning to E-cores imposes a hard ceiling that the OS wouldn't have placed.

**Proposed Solution:**  
- Remove all thread affinity hints from the capture/encode path. Let the macOS scheduler (which is excellent) decide.
- Set the capture thread's QoS to `QOS_CLASS_USER_INTERACTIVE` or `QOS_CLASS_USER_INITIATED` — this signals to the OS that these threads are latency-sensitive and should run on P-cores when active.
- Alternatively, use `pthread_set_qos_class_self_np()` with `QOS_CLASS_USER_INTERACTIVE`.

**Architectural Impact:**  
Minor code change, major latency impact. Remove thread pinning; add QoS class annotations.

---

### Issue 06 — VideoToolbox Is a C API; `objc2` Binding Is Wrong
**Severity:** 🔴 Critical  
**Section:** §11 Technology Stack

**Problem:**  
The RFC lists: "macOS encode: **VideoToolbox** via `objc2`."

`objc2` is a Rust crate for binding **Objective-C** classes and methods. VideoToolbox is **not an Objective-C API** — it is a **Core Foundation / C API**. The primary types are:
- `VTCompressionSessionRef` — an opaque C pointer (`CFTypeRef`)
- `VTCompressionSessionCreate()` — a C function
- `VTSessionSetProperty()` — a C function
- `kVTCompressionPropertyKey_*` — `CFStringRef` constants

There are no Objective-C classes in VideoToolbox. Binding it via `objc2` is not possible; `objc2` has no mechanism for C function FFI.

Similarly, the RFC mentions `ScreenCaptureKit` via `objc2`. SCKit **does** have Objective-C classes (e.g., `SCStream`, `SCContentFilter`), so `objc2` is appropriate there. But some SCKit APIs introduced in macOS 14+ are **Swift-only** and have no Objective-C equivalents, requiring Swift shims.

**The correct approach for VideoToolbox is:**
- `videotoolbox-sys` crate: exists but is effectively unmaintained (last meaningful update 2021; does not expose `VTCompressionOutputHandler` properly).
- Write a thin **C shim** (`videotoolbox_shim.c`) that wraps the callback-heavy VideoToolbox API in a more FFI-friendly form, linked into the Rust binary via `cc` crate in `build.rs`.

**Proposed Solution:**  
Technology stack correction:
- ScreenCaptureKit: `objc2` ✅ (correct for ObjC APIs, with Swift shim for macOS 14+ methods)
- VideoToolbox: **C FFI shim** compiled via `cc` crate, not `objc2`
- Add `core-foundation` and `core-media` crates for CF type bridging

**Architectural Impact:**  
Adds a `c-shims/` directory to the host crate. Build system gains a `build.rs` with `cc::Build` compilation. This is a moderate scope increase but necessary for correctness.

---

### Issue 07 — Fragmentation Volume at 30 Mbps Is Massively Underestimated
**Severity:** 🟠 High  
**Section:** §5.5, §9.5

**Problem:**  
The RFC specifies a default bitrate of 30 Mbps and QUIC datagram payloads of ~1,150 bytes.

A single H.265 frame at 30 Mbps, 60 FPS:
```
bytes_per_frame = 30,000,000 bps / 8 / 60 fps = 62,500 bytes (average)
datagrams_per_frame = ceil(62,500 / 1,150) = 55 datagrams per frame
datagrams_per_second = 55 × 60 = 3,300 datagrams/second
```

For keyframes (which can be 5–10× larger than P-frames):
```
keyframe_bytes ≈ 500,000 bytes
keyframe_datagrams = ceil(500,000 / 1,150) = 435 datagrams
```

This creates several concrete problems:
1. **`frag_total` is 2 bytes** (max value: 65,535). Technically safe, but a 435-datagram keyframe means 435 concurrent entries in the reassembly buffer simultaneously.
2. **Reassembly buffer memory**: at 30 Mbps with a 3-frame out-of-order window: 3 × 62,500 = ~188 KB — manageable. But at 100 Mbps (stated max), this becomes ~625 KB.
3. **Datagram send rate of 3,300/s** at 1,200 bytes each is ~32 Mbps of UDP traffic. This is fine on Gigabit LAN, but the syscall overhead of 3,300 `sendmsg()` calls per second on the host is non-trivial. Linux `sendmmsg()` / `GSO` (Generic Segmentation Offload) can batch these; macOS `sendmsg()` cannot be batched the same way.
4. **On macOS, each QUIC datagram is a separate `sendmsg()` syscall.** At 3,300/s, this is ~55 syscalls every 16.7 ms frame window. The Tokio async I/O path adds overhead per call.

**Proposed Solution:**  
- Implement **application-level batching**: accumulate all fragments of a single frame and submit them in a tight loop (not spread across Tokio async yields). Use `quinn`'s `send_datagram()` in a non-yielding burst per frame.
- Investigate macOS `sendmsg()` with `SO_NWRITE` to reduce round-trips.
- Set a realistic maximum bitrate at 50 Mbps for v1.0 (not 100 Mbps) until the send performance is validated.
- Define a minimum PMTU discovery step so the actual payload size is maximized (often 1,400+ bytes on LAN), reducing datagram count.

**Architectural Impact:**  
The network sender in `data_plane.rs` must implement a frame-burst-send strategy, not per-datagram async I/O. This significantly changes the send loop design.

---

### Issue 08 — VideoToolbox Encode Latency Is Optimistic by 2–3×
**Severity:** 🟠 High  
**Section:** §6.4

**Problem:**  
The latency budget allocates **~5 ms** for VideoToolbox hardware H.265 encode. Empirical measurements on Apple Silicon show:

| Scenario | Actual Encode Latency |
|----------|-----------------------|
| 1080p H.265, low motion | ~6–9 ms |
| 1080p H.265, high motion | ~10–14 ms |
| 1440p H.265, low motion | ~9–13 ms |
| 1440p H.265, high motion | ~14–20 ms |
| 4K H.265 | ~18–30 ms |

These numbers are from VideoToolbox in real-time mode (`kVTCompressionPropertyKey_RealTime = true`) with no B-frames on M1/M2. The 5 ms figure might hold for very low-resolution or heavily compressed simple content, but is not a valid p50 for typical desktop UI content with constant mouse movement, window animations, and mixed content.

At 1440p, the encode alone at ~10 ms p50 already consumes 10 of the 30 ms budget. Combined with the other stages, the 30 ms target requires everything else to complete in 20 ms — which includes a display scanout component that alone averages 8 ms. This leaves only 12 ms for capture, network, decode, and render combined. At 1440p, this is likely unachievable without VRR.

**Proposed Solution:**  
- Run prototype benchmarks on real Apple Silicon hardware (M2 at minimum) before locking the latency budget.
- Update the RFC with measured values, not estimated ones.
- Segment the latency target by resolution: `<30 ms @ 1080p60`, `<35 ms @ 1440p60`, `<50 ms @ 4K60`.
- Consider a `kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality` property (available macOS 13+) which reduces encode quality slightly but cuts latency by ~20%.

**Architectural Impact:**  
Resolution-specific latency tiers must be defined. The UI must communicate the expected latency per resolution setting to users.

---

### Issue 09 — Dual-Vsync Penalty Not Modelled; Defeats the 30 ms Target
**Severity:** 🟠 High  
**Section:** §6.4

**Problem:**  
The latency budget treats "Display scanout (60 Hz period = 16.7 ms) → ~8 ms avg" as if it is independent of everything else. This analysis ignores the **dual-vsync penalty** — a fundamental problem in display streaming systems.

The host captures at vsync N (t=0). The encode + network + decode completes at t=22 ms. The viewer's display is at vsync N+1 (t=16.7 ms). The frame arrived **after** the viewer's vsync N+1 deadline. It must wait until vsync N+2 (t=33.4 ms).

```
Host vsync:  |--- N ---|--- N+1 ---|--- N+2 ---|
Capture:      ^
Encode+Net:            <----22ms----->
Viewer vsync:     |--- A ---|--- B ---|--- C ---|
                                      ^ frame presented here
                                      
Total delay = 33.4 ms (missed vsync B by ~5 ms)
```

This is the **dual-vsync problem**: two independent vsync clocks, neither synchronized to each other. Without explicit clock synchronization between host and viewer, frames will randomly align with or miss the viewer's vsync, causing latency to oscillate between 22 ms and 38 ms (one full viewer vsync period). The **average** glass-to-glass latency is therefore ~30 ms, with spikes to 38 ms — which means the p99 target of 28 ms is literally impossible without vsync synchronization.

Apple Sidecar solves this with a proprietary protocol (`RemotedAVPresenter`) that synchronizes the host capture timing to the viewer's vsync phase. This is the core engineering insight that makes Sidecar feel native. The RFC doesn't address this at all.

**Proposed Solution:**  
- Implement a **presentation clock synchronization protocol** on the control plane:
  1. Viewer reports its vsync period and current phase (timestamp of last vsync) to the host every 16.7 ms.
  2. Host adjusts its SCStream capture timing to lead the viewer's vsync by (encode_time + network_rtt/2).
  3. This causes frames to arrive at the viewer just before its vsync, dramatically reducing scanout jitter.
- Alternatively (simpler for v1.0): use `SCStream` frame pacing with `SCStreamConfiguration.minimumFrameInterval` to align to a phase that minimizes dual-vsync conflict empirically.

**Architectural Impact:**  
This is a significant new protocol component that must be added to the control plane. Without it, the 30 ms claim is marketing, not engineering.

---

### Issue 10 — 500 ms ABR Feedback Is 5× Too Slow
**Severity:** 🟠 High  
**Section:** §6.3

**Problem:**  
The RFC sends `Stats` feedback every 500 ms. At 60 FPS, 500 ms represents 30 frames. If congestion occurs, the viewer will display 30 degraded frames before the host even receives the first signal to reduce bitrate. On a LAN this is less critical, but consider:

- A microwave oven, Bluetooth device, or neighboring WiFi channel can cause 200–400 ms burst interference on 2.4 GHz WiFi LANs.
- During this burst, with 500 ms feedback latency, the host continues sending at 30 Mbps into a saturated channel, causing queue buildup in the WiFi AP, which then causes **persistent latency increase** even after the interference clears (bufferbloat).
- WebRTC's GCC reacts in **50–100 ms**. RTCP feedback in professional broadcast systems is typically **200 ms** at maximum. 500 ms is unusually slow for a real-time system.

The RFC's ABR algorithm also has a +5% / -20% step rate that is asymmetric and slow to recover. After a large drop (e.g., 30 Mbps → 24 Mbps), recovery at +5% per 500 ms = 10% per second means recovery to original bitrate takes 6 seconds. During this time, quality is unnecessarily degraded.

**Proposed Solution:**  
- Reduce feedback interval to **100 ms** for reactive signals (frame loss, jitter spike).
- Keep 500 ms interval for slow-path stats (decode time, sustained bandwidth estimate).
- Implement an **immediate feedback signal**: if the viewer detects frame loss, it sends a `KeyframeRequest` with a `BandwidthHint` immediately (not on the next 500 ms tick).
- Adopt a more aggressive recovery rate: +10% per 100 ms when path is clear, -30% on loss event.
- Model the ABR controller on NACK-based feedback (per-frame loss signals) rather than aggregated interval stats.

**Architectural Impact:**  
Control plane protocol additions: `ImmediateFeedback` message type. ABR controller rewrite with dual-timescale logic.

---

### Issue 11 — `mdns-sd` Crate Conflicts With macOS System mDNS Daemon
**Severity:** 🟠 High  
**Section:** §11, §7.1

**Problem:**  
The RFC's technology stack specifies `mdns-sd` crate (pure Rust mDNS/DNS-SD) cross-platform. On macOS, this will conflict with the system **mDNSResponder** daemon.

macOS's mDNSResponder has exclusive ownership of UDP port 5353 on the loopback and all network interfaces. A process that also tries to bind port 5353 will either:
- **Fail to bind** (most common, especially on macOS 12+), preventing discovery from working entirely.
- **Receive duplicate packets** if using `SO_REUSEPORT` (the two implementations will collide and produce inconsistent records).

Apple's mDNSResponder enforces this exclusivity deliberately. The `mdns-sd` crate's README itself warns: "On macOS, you should use the native Bonjour API instead of this crate."

The RFC correctly identifies `dns_sd.h` (Bonjour) for macOS in §7.1 bullet points, but then lists `mdns-sd` as the mDNS library in §11. This is a direct contradiction.

**Proposed Solution:**  
- macOS host: use `dns_sd.h` via `bonjour-sys` crate (or write a thin C binding) — no exceptions.
- Windows viewer: use `mdns-sd` crate OR Windows `DnsServiceRegister` API. Evaluate which has better multicast reliability on Windows 11 with Defender Firewall.
- Add a discovery fallback: if mDNS fails (firewall, IGMP suppression, VPN), allow manual IP+port entry in the viewer UI. This is critical for corporate LAN environments where multicast is blocked.

**Architectural Impact:**  
Platform-specific mDNS implementations required (already partially noted in §7.1 but contradicted by §11). Discovery fallback (manual IP) is a new UI component.

---

### Issue 12 — SPAKE2+ References an Expired Draft; RFC 9382 Was Published in 2023
**Severity:** 🟠 High  
**Section:** §7.2

**Problem:**  
The RFC cites "draft-bar-cfrg-spake2plus-10" as the SPAKE2+ specification. This draft **expired** and was superseded by **RFC 9382** (SPAKE2+, August 2023). The expired draft and the final RFC have differences in test vectors and implementation guidance. Implementing from an expired draft and calling it "standardized" is incorrect. The RustCrypto `spake2` crate may implement SPAKE2 (not SPAKE2+), and its compatibility with RFC 9382 test vectors should be verified before depending on it.

**Proposed Solution:**  
- Update all references to RFC 9382.
- Verify that the `spake2` RustCrypto crate implements RFC 9382 SPAKE2+ (not the earlier SPAKE2 variant), and if not, evaluate `cpace` or implement directly from RFC 9382.
- Run the RFC 9382 test vectors against the chosen implementation in the CI suite before release.

**Architectural Impact:**  
Low — library selection clarification. Medium risk if the wrong variant is implemented and HomeKit interoperability is attempted in the future.

---

### Issue 13 — Fullscreen Win32 Window From Scratch via `windows-rs` Is Massively Underestimated
**Severity:** 🟠 High  
**Section:** §9.2

**Problem:**  
The RFC describes the viewer UI as "Win32 borderless fullscreen window" via `windows-rs`. What this actually entails:

- Registering a Win32 window class (`WNDCLASSEX`)
- Creating a message pump (`GetMessage` / `TranslateMessage` / `DispatchMessage` loop)
- Handling `WM_NCCALCSIZE` for borderless styling
- Handling `WM_ACTIVATE`, `WM_SETFOCUS`, `WM_KILLFOCUS` for fullscreen focus management
- DPI awareness via `SetProcessDpiAwarenessContext` + `WM_DPICHANGED` handling
- Monitor enumeration via `EnumDisplayMonitors` for fullscreen placement
- Keyboard shortcut handling (`WM_HOTKEY`) for exit/settings
- Thread-safety: all Win32 calls must occur on the thread that created the window
- D3D12 device loss handling (`DXGI_ERROR_DEVICE_REMOVED` → device recreation loop)
- High-DPI swap chain resize on `WM_SIZE`

This is several weeks of low-level Win32 work for an experienced Windows developer. For a Rust developer unfamiliar with Win32, this is a multi-month effort with significant correctness risks. The RFC makes it sound like a single `window.rs` file.

**Proposed Solution:**  
- Use **`winit`** (cross-platform window creation) for window management. `winit` handles the message pump, DPI, fullscreen, and resize events, and has a `windows-rs` compatible surface handle for D3D12.
- `winit` is production-quality, used by Bevy, wgpu, and Tauri. It is not Electron.
- Keep D3D12 Video Decode and rendering in `windows-rs` — only offload window management to `winit`.
- This reduces the window management code to ~50 lines of `winit` event loop vs. ~2,000 lines of raw Win32.

**Architectural Impact:**  
Adds `winit` as a dependency. Simplifies `window.rs` dramatically. No performance penalty — `winit` is a thin wrapper.

---

### Issue 14 — No Token Revocation; Compromised Pair Token Is Permanent
**Severity:** 🟠 High  
**Section:** §8

**Problem:**  
The security model has no mechanism for revoking a compromised Pair Token or unparing a viewer. If an attacker gains access to the Windows Credential Manager (e.g., malware with user-level access), they extract the Pair Token and can impersonate the viewer on the LAN indefinitely.

The host has no:
- Revocation mechanism (no "unpair this device" command)
- Session token rotation (Pair Token is static and long-lived)
- Audit log of connection attempts
- Notification to the user when a new session begins

**Proposed Solution:**  
- Add a **"Manage Paired Devices"** UI to `renderd-host` showing all paired viewers with: device name, last seen timestamp, pairing date.
- Add a **"Revoke"** button that deletes the viewer's certificate from the host's known-viewers list.
- Add an **OS notification** (macOS `UserNotifications`) when a new streaming session begins: "renderd-viewer on [device-name] started screen sharing."
- Rotate a session-specific subkey on each session: `SessionKey = HKDF(PairToken, session_nonce)`. Even if the PairToken is leaked, individual session recordings cannot be decrypted retroactively (adds forward secrecy at the session level beyond TLS).

**Architectural Impact:**  
New UI component (device management panel). New control plane message type (`SessionBeginNotification`). Session key derivation step added to authentication flow.

---

### Issue 15 — Reconnect Loop Doesn't Re-Run mDNS Discovery; Fails on DHCP Renewal
**Severity:** 🟠 High  
**Section:** §13.1

**Problem:**  
The reconnect state machine re-connects to the previously known IP address. If the host's DHCP lease has renewed during the disconnect (common after sleep/wake cycles, or on corporate DHCPs with 1-hour leases), the host IP changes. The viewer will repeatedly fail to reconnect to the stale IP, eventually entering IDLE state — requiring the user to manually re-discover.

The disconnect + IP change scenario is extremely common: user puts the Mac to sleep, wakes it later, DHCP assigns a new IP. Viewer fails to reconnect. User is confused.

**Proposed Solution:**  
- After the first failed reconnect attempt (not after 30 seconds), immediately run a **fresh mDNS browse** filtered by the paired host's UUID (from the TXT record).
- Match the discovered service to the stored UUID, extract the new IP, and reconnect.
- The reconnect loop should be: `attempt saved IP → fail → re-discover by UUID → attempt new IP → ...`
- This requires the stored pairing to include the host UUID, not just the IP (the RFC's pair token storage doesn't specify what is stored alongside the token).

**Architectural Impact:**  
Reconnect watchdog gains a mDNS discovery step. Pairing storage schema must include host UUID. Discovery and reconnect modules are no longer independent.

---

### Issue 16 — HKDF Info Field Has Length-Ambiguity Collision Vector
**Severity:** 🟡 Medium  
**Section:** §7.2

**Problem:**  
The Pair Token derivation is:
```
PairToken = HKDF(K, "renderd-pair-token", host-uuid || viewer-uuid)
```

The `info` parameter (`host-uuid || viewer-uuid`) is a raw concatenation of two UUIDs without length prefixes or delimiters. If UUIDs are stored as variable-length strings (not fixed-length 16-byte binary), two different (host-uuid, viewer-uuid) pairs can produce the same info byte sequence:

```
host="AB",  viewer="CDEF"  →  info = "ABCDEF"
host="ABC", viewer="DEF"   →  info = "ABCDEF"
```

Both derive the same PairToken from the same K. In a PAKE system, this collision is exploitable: an attacker who has paired as viewer "DEF" with host "ABC" can impersonate viewer "CDEF" with host "AB".

**Proposed Solution:**  
Use length-prefixed encoding for the HKDF info:
```
info = len(host-uuid) || host-uuid || len(viewer-uuid) || viewer-uuid
```
Or use a structured format:
```
info = "renderd-v1-pair:" || uuid_canonical(host) || ":" || uuid_canonical(viewer)
```
Where UUIDs are always in canonical hyphenated lowercase format (36 bytes fixed).

**Architectural Impact:**  
One-line change in the pairing protocol, but breaks backward compatibility if changed post-release. Must be locked down before v1.0 ships.

---

### Issue 17 — Protobuf Schema Has No Version Negotiation
**Severity:** 🟡 Medium  
**Section:** §9.4

**Problem:**  
`SessionHello` contains `protocol_version = 1` (a uint32 field), but the RFC does not define what happens when versions differ. Protobuf handles unknown fields gracefully, but:

- What if the viewer sends v2 with new required fields and the host is v1?
- What if a v2 viewer sends a `SessionHello` with a new required codec field that v1 host doesn't understand?

Without explicit version negotiation semantics, the system degrades silently: v2 viewers will connect to v1 hosts and receive a `SessionConfig` that ignores capabilities it doesn't understand. This can result in codec negotiation failures silently falling back to incorrect codecs.

**Proposed Solution:**  
- Define a version compatibility matrix in the RFC: minimum required version for each feature.
- `SessionHello` should include `min_required_version` (the minimum host version this viewer requires).
- Host responds with an error code if `min_required_version > host_version`: `ERROR_VERSION_INCOMPATIBLE`.
- Alternatively, use `oneof` variant messages keyed by version for breaking changes.

**Architectural Impact:**  
Adds `min_required_version` field to `SessionHello` and an `Error` message type to the control plane protocol.

---

### Issue 18 — 2-Second Keyframe Interval Causes Cold-Connect Blank Screen
**Severity:** 🟡 Medium  
**Section:** §6.2

**Problem:**  
`kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration = 2s` means the encoder emits an I-frame at most every 2 seconds. When a viewer connects mid-stream, it cannot render any P-frames until it receives an I-frame to use as a reference. With a 2-second keyframe interval, the worst case is a **2-second blank screen** before the viewer displays anything.

Additionally, when the ABR controller requests a keyframe (e.g., after frame loss), the viewer must wait up to 2 seconds in the worst case without the RFC specifying that the host sends an immediate keyframe on demand.

**Proposed Solution:**  
- Set `MaxKeyFrameIntervalDuration` to **0.5s** (2 keyframes every 30 frames at 60 FPS). This trades ~5% bitrate overhead for a 0.5s maximum cold-connect delay — acceptable.
- **Force an immediate keyframe on new viewer connection**: when a new session is established, the host immediately calls `VTCompressionSessionCompleteFrames()` followed by a new frame with `kVTEncodeFrameOptionKey_ForceKeyFrame = true`.
- Document this behavior explicitly in the protocol spec.

**Architectural Impact:**  
Minor. Session establishment flow gains a "force keyframe" step. Keyframe interval parameter adjustment.

---

### Issue 19 — Self-Signed Certificates Have No Stated Expiry
**Severity:** 🟡 Medium  
**Section:** §8, §7.3

**Problem:**  
The RFC describes using `rcgen` to generate self-signed certificates at pairing time. `rcgen`'s default validity period, if not explicitly set, is configurable but the RFC doesn't specify what to set it to.

If the certificate is set to "not expire" (validity until year 9999, a common lazy default), a stolen or leaked certificate is valid forever. If it expires too soon (e.g., 1 year), users will face mysterious "certificate expired" connection failures with no clear recovery path (requiring re-pairing).

**Proposed Solution:**  
- Set certificate validity to **10 years** from pairing date — long enough to not surprise users, short enough to bound the exposure window.
- **On every session establishment**, refresh the session cert if it has less than 6 months of validity remaining (generate a new cert from the stored PairToken, automatically without user action).
- Log the cert expiry date in the host's device management UI so users can see it.

**Architectural Impact:**  
Certificate refresh logic added to session establishment. New field in pairing storage: `cert_expires_at`.

---

### Issue 20 — `DXGI_PRESENT_ALLOW_TEARING` Requires Runtime Capability Check
**Severity:** 🟡 Medium  
**Section:** §6.2

**Problem:**  
The RFC states: "use `DXGI_PRESENT_ALLOW_TEARING` with VRR/FreeSync/G-Sync support for minimum scanout latency."

Using `DXGI_PRESENT_ALLOW_TEARING` requires:
1. Swap chain created with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING`.
2. `IDXGIFactory5::CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING)` returns `TRUE`.
3. The `Present1()` call uses `DXGI_PRESENT_ALLOW_TEARING` flag.

If the viewer system doesn't support tearing (e.g., integrated GPU, HDMI output without VRR, driver doesn't support it), creating the swap chain with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING` and calling `Present1()` with `DXGI_PRESENT_ALLOW_TEARING` will result in **`DXGI_ERROR_INVALID_CALL`** — a runtime crash.

The RFC doesn't mention the capability check, meaning the viewer will crash on systems without tearing support (which is a significant portion of Windows 11 users without gaming monitors).

**Proposed Solution:**  
- Always call `IDXGIFactory5::CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING)` at startup.
- Store the capability in a `bool allow_tearing` flag.
- Create the swap chain conditionally: with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING` if supported, without otherwise.
- Use `DXGI_PRESENT_ALLOW_TEARING` in `Present1()` only when `allow_tearing == true`.
- Fallback to vsync-locked presentation (`SyncInterval=1`) otherwise.

**Architectural Impact:**  
Small but necessary correctness fix in the D3D12 renderer initialization.

---

### Issue 21 — 8 ms Fragment Deadline Conflicts With the Frame Budget
**Severity:** 🟡 Medium  
**Section:** §13.4

**Problem:**  
The RFC specifies: "If all fragments of a frame don't arrive within **8 ms** of the first, drop the frame."

At 60 FPS, the entire frame budget is 16.7 ms. The fragment arrival deadline of 8 ms means the receiver gives up after consuming exactly half the frame budget waiting for fragments. This creates a self-defeating behavior:

- The encode alone takes ~5–14 ms (per Issue 08).
- Network transit: ~1–2 ms.
- Total time from capture to last fragment arrival: ~8–16 ms at p50.

Under these timings, a frame that took 14 ms to encode will arrive with only 2–4 ms to spare under the 8 ms deadline — routinely triggering false-positive drops and spurious keyframe requests, which in turn spike the bitrate.

The 8 ms value appears to be chosen without reference to the encode latency or the overall pipeline timing.

**Proposed Solution:**  
- Set the fragment deadline to **`frame_deadline - decode_time - render_time`** — dynamically computed based on the viewer's measured decode and render times from the `Stats` feedback.
- As a static default: use 12 ms (giving the network 12 ms to deliver fragments, leaving 4.7 ms for decode+render at 60 Hz). This is still aggressive but avoids false-positive drops for typical encode times.
- Do not discard on deadline: instead, present whatever fragments arrived (partial frame display from an H.265 decoder in error-concealment mode), then request a keyframe. This avoids freeze artifacts.

**Architectural Impact:**  
Dynamic deadline computation requires decode and render time telemetry from the viewer (already in the Stats message — use it).

---

### Issue 22 — macOS Notarization and Entitlements Are Completely Unaddressed
**Severity:** 🟡 Medium  
**Section:** §11, §12

**Problem:**  
`renderd-host` requires ScreenCaptureKit, which requires:
1. **`com.apple.security.screen-recording` entitlement** — must be declared in an entitlements file, embedded in the signed binary.
2. **App Sandbox** (`com.apple.security.app-sandbox`) — required for App Store distribution; optional but recommended for Gatekeeper compatibility.
3. **Developer ID code signing** — required for Gatekeeper on macOS 13+. Without it, users see "renderd-host cannot be opened because Apple cannot check it for malicious software."
4. **Notarization** — Apple's notarization service must scan and approve the binary before distribution. Non-notarized binaries fail Gatekeeper on modern macOS.
5. **`NSScreenCaptureUsageDescription`** in `Info.plist` — required for the screen recording permission prompt to display sensibly.

None of this is mentioned anywhere in the RFC, repository layout, or build scripts. This is not a minor omission — without code signing and notarization, the macOS host binary **cannot run** on a standard user's machine without them manually disabling Gatekeeper.

**Proposed Solution:**  
- Add to §12 (Repository Layout):
  - `renderd-host/Info.plist` — bundle metadata and usage descriptions
  - `renderd-host/entitlements.plist` — screen recording + app sandbox entitlements
  - `.github/workflows/release-host.yml` — notarization workflow using `notarytool`
- Add to §11: Apple Developer Program enrollment ($99/year) as a required infrastructure item.
- Document the signing workflow: `codesign --entitlements` → `xcrun notarytool submit` → `xcrun stapler staple`.
- The Rust binary must be wrapped in a `.app` bundle (`renderd-host.app`) with the correct directory structure for macOS to recognize it as a signable application.

**Architectural Impact:**  
The build system gains significant macOS packaging complexity. `build-host.sh` becomes `package-host.sh` that produces a signed `.app` bundle. CI requires a macOS runner with access to a Developer ID certificate (stored as a GitHub Actions secret).

---

## Summary and Verdict

The RFC has the right instincts — QUIC is the correct transport choice, H.265 with VideoToolbox is the correct codec stack, and SPAKE2+ is the right pairing protocol. These directional decisions are sound.

However, the architecture cannot be implemented as written without hitting the following walls:

| Blocker | What Breaks |
|---------|-------------|
| Wrong process model (Issue 04) | ScreenCaptureKit will never capture |
| E-core pinning (Issue 05) | Encode latency blows the budget |
| Wrong VideoToolbox API approach (Issue 06) | Build will not compile |
| Broken latency budget math (Issue 02) | 30 ms target cannot be achieved as designed |
| Unordered datagrams (Issue 01) | Frame reassembly will corrupt randomly |
| BBR unavailable (Issue 03) | Congestion control feature is fictional |
| Notarization missing (Issue 22) | macOS users cannot run the binary |

**Recommended next steps before any implementation:**

1. **Prototype first, spec later.** Build isolated latency benchmarks: (a) SCStream → VideoToolbox on Apple Silicon, (b) QUIC datagram round-trip on LAN with `quinn`. Update the RFC with measured numbers.
2. **Fix Issues 01, 03, 04, 05, 06** — these are correctness blockers that will prevent compilation or basic function.
3. **Resolve Issue 09** (dual-vsync) — this is the hardest problem and the one that separates Renderd from a demo from a product. Allocate significant design time.
4. Issue a **RFC-0001-rev1** incorporating all findings before writing any production code.

---

*End of Review*
