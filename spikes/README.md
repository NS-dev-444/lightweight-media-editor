# Phase 1 — Technical Spikes

Per spec §35: **technical proof-of-concepts only, no polished UI, and none of
this code is kept.** Each spike has a pass condition defined in advance
(`docs/PHASE_0_VALIDATION.md` §6). A spike without a pass condition is a demo,
not a validation.

Results — including the disappointing ones — go to `docs/PHASE_1_RESULTS.md`
(§46 Rules 6 and 12).

## Status

| # | Spike | Pass condition | Status |
|---|---|---|---|
| **S3** | Reproducible LGPL FFmpeg build + CI licence gate | Build reproducible from clean checkout; gate **rejects** a GPL build | ✅ **PASS** |
| **S4** | Encode benchmark, VideoToolbox vs libx264, VMAF-scored | Dataset produced; preset bitrates chosen **from data** | ✅ **COMPLETE** — R-09 trigger fired |
| **S4b** | Constant-quality re-test (`-crf` vs `-q:v`) | Fair per-encoder comparison | ✅ **COMPLETE** — corrects S4; 2.31x/1.74x |
| **S2** | Timeline rendering: SwiftUI `Canvas` vs custom `NSView` | ≥60 fps with 200 clips + waveforms; no main-thread block >16 ms | ✅ **PASS (both)** — SwiftUI 3.29ms p99 worst case |
| **S2b** | Timeline **interaction**: drag, hit-test, multi-select | ≤16 ms response; no dropped frames during a drag | ✅ **PASS** — 0.74ms p99, AD-2 settled |
| **S1** | Rust → C ABI → Swift → VideoToolbox → Metal, zero-copy | 4K30 ≥29.5 fps; **zero CPU frame copies**; RSS <500 MB | ✅ **PASS** — 183fps steady, 0 copies, R-04 retired |
| **S6** | Worker-process crash isolation + fuzzed media corpus | 1000+ malformed files, **zero UI-process crashes** | ✅ **PASS** — 1027 files, 0 crashes, 0 hangs |
| **S7** | Async waveform generation + cache | 60-min WAV in ≤10 s; UI never blocked >16 ms | ✅ **PASS** — 0.35s WAV / 3.18s FLAC, 0 stalls |
| **S8** | macOS sandbox + security-scoped bookmarks + sandboxed worker | Project reopens after relaunch with media access intact | ✅ **PASS** — worker needs `com.apple.security.inherit` |
| **S10** | Proxy necessity: 4K30 H.264 / 4K HEVC 10-bit / 4K60 | Decides whether Tier 2 proxies ship in V1 (AD-5) | ⬜ partial — macOS half only |
| **S5** | Windows: WinUI 3 + SwapChainPanel + Rust P/Invoke + D3D11VA | 4K30 ≥29.5 fps; cold start ≤2.5 s; idle RSS ≤250 MB | 🔵 **deferred to V1.1** (O-1) — but see below |
| **S9** | Hardware capability probe across ≥3 GPU configurations | Probe rejects non-working paths; results cached | 🔵 macOS half in V1; multi-GPU in V1.1 |

## Platform decision (O-1, resolved 2026-09-09)

**macOS V1, Windows V1.1.** S5 and the multi-GPU half of S9 move to V1.1.

**This does not mean ignoring Windows.** Per `RISK_REGISTER.md` R-21, the Rust
core must compile for Windows in CI **throughout V1**, even with no Windows UI.
Compiling is the cheap 80% of staying portable, and it is what keeps the V1.1
"port" from becoming a rewrite — which would retroactively invalidate AD-1,
since the Rust core is justified by the second platform existing.

## Open questions and spikes

O-3 (colour policy) and O-4 (crash reporting) still gate **Phase 2**, not any
spike above. O-16 is resolved — captions are approved — and O-18 (model size)
gates caption UX in Phase 3, not Phase 1.
