## What & Why

<!-- A concise summary of the changes and why they are needed. Reference relevant issue numbers. -->

Closes #

---

## PR Type

Please check the type of change your PR introduces:

- [ ] `feat` (New feature or public API capability)
- [ ] `fix` (Bug fix)
- [ ] `perf` (Latency or throughput performance optimization)
- [ ] `refactor` (Internal code reorganization without external behavior change)
- [ ] `docs` (Documentation additions or fixes)
- [ ] `test` (Adding or updating tests)
- [ ] `chore` (Dependencies, CI/CD, build system)
- [ ] `security` (Security fix or hardening)

---

## Affected Scope

Select affected crate(s) or component(s):

- [ ] `renderd-proto` (Protobuf schema / envelope dispatch)
- [ ] `renderd-config` (Layered configuration loader)
- [ ] `renderd-frame` (Fragment codec / reassembly state machine)
- [ ] `renderd-crypto` (Noise Protocol / AES-256-GCM)
- [ ] `renderd-vt-sys` / `renderd-sc-sys` (macOS hardware FFI)
- [ ] `renderd-net` (QUIC transport / Datagram pipeline)
- [ ] `renderd-keychain` / `renderd-discovery` (Security & mDNS)
- [ ] `renderd-abr` / `renderd-clock` (Algorithms)
- [ ] `renderd-host` / `renderd-viewer` (Daemons)
- [ ] `ci` / `docs` / `deps` / `release`

---

## Benchmark Impact

*Required for data-plane crates (`renderd-frame`, `renderd-net`, `renderd-crypto`, `renderd-abr`, `renderd-clock`).*

```text
Before: bench_reassembly_burst (55 frags): -- µs
After:  bench_reassembly_burst (55 frags): -- µs
Delta:  --%
```

---

## Breaking Changes

- [ ] This PR contains a **breaking protocol change** (`renderd.proto` wire-format or schema modification).
- [ ] This PR contains a **breaking API change** in a library crate.

*If checked, describe migration path and bump requirements below.*

---

## Verification Checklist

- [ ] Code follows project formatting standards (`cargo fmt --check`)
- [ ] Clippy passes with zero warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- [ ] Test suite passes cleanly (`cargo nextest run --workspace` or `cargo test --workspace`)
- [ ] Proto generated code is up-to-date (`cargo run --manifest-path tools/proto-gen/Cargo.toml`)
- [ ] Dependency policy passes (`cargo deny check`)
- [ ] `CHANGELOG.md` updated in Keep-a-Changelog format
- [ ] PR title follows Conventional Commits format (`type(scope): description`)
