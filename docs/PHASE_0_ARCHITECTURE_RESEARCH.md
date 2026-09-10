# Phase 0 — Architecture Research

**Project:** Lightweight cross-platform video + audio editor (working name TBD)
**Document status:** Phase 0 deliverable 1 of 5
**Date:** 2026-09-09
**Scope:** Research only. No application code has been written and none should be until `ARCHITECTURE_DECISION.md` and `PHASE_0_VALIDATION.md` are accepted.

---

## 0. How to read this document

The master spec (§46 Rule 2, §50) forbids architectural decisions based on assumptions. Every substantive claim below therefore carries a confidence tag:

| Tag | Meaning |
|---|---|
| `[VERIFIED]` | Read from a primary source (vendor documentation, licence text, or a command run on this machine) during Phase 0. |
| `[REPORTED]` | Multiple secondary sources agree; not confirmed against a primary source. Treat as probable, not proven. |
| `[ASSUMED]` | Engineering judgement based on prior art. Not verified. Must not be the sole basis for a decision. |
| `[MEASURE]` | Cannot be settled by reading. Phase 1 must measure it on real hardware. |
| `[COUNSEL]` | A legal question. Requires qualified counsel. Nothing in this document is legal advice. |

Anything tagged `[MEASURE]` or `[COUNSEL]` is an **open item**, and open items are tracked in `RISK_REGISTER.md`.

### Baseline measured on the development machine

```
macOS      26.6.2 (build 25G83), arm64 (Apple Silicon)
Swift      6.3.3 (swiftlang-6.3.3.1.3)
rustc      1.96.0
ffmpeg     9.0.1 (Homebrew)
```
`[VERIFIED]` — read from `sw_vers`, `swift --version`, `rustc --version`, `ffmpeg -version`.

The Homebrew FFmpeg on this machine is configured with `--enable-gpl --enable-version3 --enable-libx264 --enable-libx265`. **That build cannot be shipped in a proprietary product.** It is fine for research and for producing test media; it must never become the build we link against. This is not a hypothetical — it is the default FFmpeg an engineer on this project will have on their machine, and it is the single easiest way to accidentally poison the product's licensing. See §R18.

---

## R1. SwiftUI vs AppKit for the macOS UI

**Question:** Can the whole macOS UI be SwiftUI, or does the timeline/preview need AppKit?

**Findings**

- SwiftUI on current macOS is fully viable for the chrome: menus, inspector, media library, dialogs, preferences, export sheet. `[ASSUMED]` from general platform maturity; the risk here is low and reversible.
- The timeline is the problem case. A timeline is a virtualised, custom-drawn, high-frequency-redraw surface with pixel-precise hit testing, drag handles, and continuous scroll/zoom. Reports on high-refresh SwiftUI scrolling note that at 120 Hz the main thread has roughly 8.3 ms per frame and realistically ~5 ms of usable budget after system overhead, and that SwiftUI backing views are allocated per visible cell, producing stutter at large item counts where `NSCollectionView` does not. `[REPORTED]`
- The preview surface must be a `CAMetalLayer`. SwiftUI has no first-class Metal drawing surface for this; `MTKView`/`CAMetalLayer` is hosted through `NSViewRepresentable`. `[REPORTED]`
- Apple's own current guidance is mix-and-match rather than pick-a-side (WWDC 2026 session "Use SwiftUI with AppKit and UIKit"). `[REPORTED]`

**Implication**

Do not make this a religious choice. The realistic split is:

```
SwiftUI          app shell, menus, library, inspector, dialogs, converter UI
NSViewRepresentable
  └── AppKit     timeline canvas (custom NSView, manual drawing)
  └── AppKit     preview surface (CAMetalLayer)
```

**Open items**

- `[MEASURE]` Phase 1 must build a throwaway timeline spike with (a) pure SwiftUI `Canvas` and (b) a custom `NSView`, each rendering ~200 clips with waveforms and thumbnails, and measure sustained fps and scroll latency on ProMotion hardware. Decide from the numbers, not from this document.

---

## R2. Rust integration strategy

**Question:** Is a Rust shared core justified, and how does it talk to Swift and to C#?

**Findings**

- The spec's justification for Rust (§9) is sound *only because Windows is a V1 requirement*. If macOS were the only target, Swift end-to-end would be simpler and faster to build, and Rust would be net negative. The entire value of the Rust core is that it is the thing we do not write twice. This should be stated explicitly, because if Windows slips out of V1 (see the scope challenge in `PHASE_0_VALIDATION.md` §3), the Rust decision should be revisited rather than inherited.
- Rust↔Swift interop options: `[REPORTED]`
  - **UniFFI** (Mozilla) — generates Swift bindings from an interface definition; described as production-quality and used in Firefox's application-services.
  - **swift-bridge** — richer type bridging (Option, String, structs, async).
  - **Hand-written C ABI + `cbindgen`** — most control, least magic, no build-time codegen dependency.
  - All three bottom out in the C FFI.
- Critical nuance not addressed by any of the above tooling: **none of these are appropriate for per-frame data.** A 4K RGBA frame is ~33 MB. Passing frames across a generated binding, or copying them at all, is a non-starter. Frames must never cross the FFI as data — only as opaque handles to platform-native GPU-visible buffers (`CVPixelBuffer`/`IOSurface` on macOS, `ID3D11Texture2D`/`ID3D12Resource` on Windows). `[ASSUMED]` — but this is a well-established constraint and Phase 1 will confirm it by measurement.

**Implication — the control plane / data plane split**

This is the single most important structural decision in the project:

```
CONTROL PLANE  (crosses FFI, low frequency, ergonomics matter)
  open project, edit timeline, undo/redo, start export, query state,
  progress callbacks, error reporting
  → C ABI, small POD structs / serialized messages, generated or hand-written bindings

DATA PLANE     (never crosses FFI as data, high frequency, latency matters)
  decoded frames, GPU textures, audio buffers
  → platform-native buffer handles passed as opaque pointers/integers,
    lifetime managed by explicit acquire/release calls
```

Get this wrong and the "lightweight, fast" product goal (§47) is unreachable regardless of everything else.

**Open items**

- `[MEASURE]` Phase 1 spike: Rust core exposing a C ABI, called from Swift, handing back a `CVPixelBuffer` produced by VideoToolbox, rendered by Metal, with zero copies. Measure end-to-end latency and confirm zero-copy with Instruments.
- `[MEASURE]` Same spike on Windows from C# via P/Invoke handing back a D3D11 texture into a `SwapChainPanel`.
- Decide UniFFI vs cbindgen only after the spike. Bias: cbindgen for the control plane, because the FFI surface should be small enough that generation buys little and hand-written code keeps the boundary honest.

---

## R3. FFmpeg integration

**Question:** How do we consume FFmpeg — CLI subprocess, or linked libraries?

**Findings**

- Two integration models exist and they have very different properties:

| | `ffmpeg` CLI subprocess | libav* linked into our process |
|---|---|---|
| Crash isolation | Excellent (process dies, app lives) | None — a decoder bug kills the app |
| Frame-accurate seek/scrub | Poor (no persistent decoder state) | Good |
| Progress/cancel | Parse stderr; fragile | Native |
| Zero-copy to GPU | Impossible | Possible |
| Licensing | Easier (separate executable) | Requires careful dynamic linking |
| Suitability | Batch conversion | Editing/preview |

- Rust bindings: `ffmpeg-next` / `ffmpeg-sys-next` (high- and low-level, described as in maintenance mode, targeting FFmpeg 3.4–8.0) and `rusty_ffmpeg` (FFI bindings, published against FFmpeg 8.1). `[REPORTED]` Neither is confirmed against FFmpeg 9.x, which is what is installed on this machine. `[MEASURE]`
- FFmpeg's own guidance for LGPL compliance is to **link dynamically** and ship the FFmpeg source alongside binaries. `[VERIFIED]` from ffmpeg.org/legal.html.

**Implication — recommended hybrid**

- **Editing/preview path:** libav* linked dynamically, driven from Rust, running in a **separate media worker process** (see R20). This gives frame-accurate seeking *and* crash isolation, and satisfies §22's "never crash because of an invalid media file", which is not achievable in-process. Our own worker process is a plain child process we control; it is not the `ffmpeg` CLI.
- **Batch conversion path:** the same worker process, one job per worker, reusing the same code — not a shell-out to `ffmpeg`. Shelling out to the CLI would make progress, cancellation, and error classification (§23) worse, and would ship a GPL-flavoured binary by accident far too easily.

**Open items**

- `[MEASURE]` Pin the FFmpeg version. Building our own LGPL FFmpeg is required anyway (R18), so the binding crate must be validated against *our* build, not Homebrew's.
- `[MEASURE]` Decide whether to use `ffmpeg-next`'s safe wrapper or write our own thin safe layer over `rusty_ffmpeg`. Maintenance-mode status of `ffmpeg-next` is a supply-chain risk (see `RISK_REGISTER.md` R-11).

---

## R4. AVFoundation

**Question:** Should macOS use AVFoundation instead of, or alongside, FFmpeg?

**Findings**

- AVFoundation gives high-quality, well-integrated decode/encode/export for the formats Apple supports, plus `AVPlayer`, composition, and `AVAssetWriter`. `[ASSUMED]` from platform knowledge.
- AVFoundation does **not** cover the spec's required container list. `[VERIFIED]` — measured on this machine (macOS 26.6.2) during Phase 0:

  ```
  probe.mkv  : ERROR "Cannot Open"          (H.264 + AAC in Matroska)
  probe.mp4  : playable=true, 2 tracks      (identical streams, stream-copied)
  probe.webm : ERROR "Cannot Open"          (VP9)
  ```

  The MKV and the MP4 contain the *same* H.264 and AAC streams — the MP4 was produced from the MKV with `-c copy`. AVFoundation opens one and refuses the other, so this is purely a **container** limitation, not a codec one. §2 requires MKV and WebM.

  **Therefore FFmpeg is non-optional on macOS for demuxing.** This is now a measured fact, not an assumption, and it removes "use AVFoundation instead of FFmpeg on macOS" from the option space.
- Mixing two media engines means two seek models, two timestamp models, and two sets of format quirks.

**Implication**

Use **FFmpeg for demux/decode of everything**, and AVFoundation only where it is a clear win and does not fork the pipeline — realistically: none of the core path in V1. Since FFmpeg must ship anyway to satisfy §2, using AVFoundation as well would add a second engine without removing the first. Use VideoToolbox *directly* (R5) rather than through AVFoundation, so there is one decode path, not two. Reconsider AVFoundation only for a future ProRes/Apple-ecosystem export preset.

---

## R5. VideoToolbox

**Findings**

- VideoToolbox exposes hardware compression/decompression sessions and pixel-buffer processing. On Apple Silicon it drives the dedicated **Media Engine** (fixed-function blocks for H.264, HEVC and ProRes), *not* the GPU shader cores; Pro/Max/Ultra parts have multiple engines. `[REPORTED]`
- Speed: an M1/M2-class part encodes 1080p H.264 at roughly 5–10× realtime, versus roughly 1.5–3× for `libx264 -preset medium`. `[REPORTED]`
- Quality: hardware encoders have a worse rate-distortion trade-off than good software encoders; at a matched VMAF target VideoToolbox needs a higher bitrate than libx264, with the gap varying by chip generation and content. Apple's HEVC encoder is well regarded relative to other hardware encoders. `[REPORTED]`

**Implication**

This is a *fortunate* alignment with the licensing strategy (R18/R19): the encoder we want for speed is also the encoder that keeps a GPL software encoder out of the product. The quality gap is acceptable for a "fast, lightweight" delivery-oriented editor and unacceptable for archival mastering — which this product explicitly is not (§1, §32 "do not build professional colour grading / multicam").

We must, however, **not** silently ship visibly worse output than competitors at the same file size. Mitigation: default presets target quality-first bitrates rather than aggressive compression, and the "Smaller File" presets are honestly labelled.

**Open items**

- `[MEASURE]` Phase 1 must produce a VMAF/SSIM comparison of `h264_videotoolbox` and `hevc_videotoolbox` against the bitrates our presets will use, on this hardware, with our test corpus. Preset bitrates must be chosen from that data — not guessed.

---

## R6. Metal rendering

**Findings**

- The preview compositor should be Metal, drawing into a `CAMetalLayer`. `[ASSUMED]`
- Decoded frames arrive from VideoToolbox as `CVPixelBuffer`, typically NV12/`420v` (8-bit) or `420f`/`x420` (10-bit HDR). These are IOSurface-backed and can be bound as Metal textures via `CVMetalTextureCache` without a CPU copy. `[ASSUMED]` — standard practice, `[MEASURE]` to confirm zero-copy in Instruments.
- Effects in §18 (brightness/contrast/saturation/blur/sharpen/grayscale/temperature) are all trivially expressible as Metal fragment shaders or small compute kernels. There is no need for a third-party effects framework, and per §32 there should not be one.

**Implication**

The compositor is per-platform (Metal / D3D) and lives above the FFI. Its *inputs* (which clip, which time, which parameters) come from the Rust core as a small, serialisable "render plan" per frame. The core decides *what* to draw; the platform decides *how*. This keeps the shader code native and the edit semantics shared.

---

## R7. Windows UI framework options

**Findings**

- **WinUI 3 / Windows App SDK** is Microsoft's stated native platform for modern Windows apps; Microsoft publicly recommitted to it at Build 2026 with claims of reduced memory usage and new controls, and Visual Studio 2026 ships a WinUI visual designer. `[REPORTED]` — note this is vendor positioning and press coverage, not evidence from our own build.
- WinUI 3 supports `SwapChainPanel`, which is the supported route for hosting a DXGI swap chain inside XAML: cast the panel to `ISwapChainPanelNative` and call `SetSwapChain`. `[REPORTED]`
- Practical prior art exists for the exact shape we need — C# handling windowing/input/platform integration with P/Invoke to a native renderer using D3D12 + DXGI + DirectComposition. `[REPORTED]`
- Alternatives considered: WPF (mature, but a dead end for new work and weaker DX interop story), Win32 + C++ with Direct2D (maximum control, maximum cost), Qt (LGPL obligations plus a "not native" feel that contradicts §47).

**Implication**

Provisional choice: **WinUI 3 (C#) shell + Rust core via P/Invoke + D3D11 or D3D12 renderer in a `SwapChainPanel`.** This is provisional and must survive a Phase 1 spike before being ratified — the spec itself demands the Windows UI technology be validated in Phase 0/1 (§8), and the honest answer is that reading documentation is not sufficient validation here.

**Open items**

- `[MEASURE]` Windows spike: WinUI 3 window, `SwapChainPanel`, D3D texture produced by a Rust-called decoder, 4K playback, timeline scroll. Measure fps, memory, and startup time. If WinUI 3 startup or memory is poor enough to contradict "lightweight", fall back to Win32/C++ + Direct2D and accept the higher build cost.
- `[MEASURE]` Confirm C# P/Invoke marshalling overhead is irrelevant at control-plane frequencies (it should be) and that no frame data crosses the managed boundary.

---

## R8 / R9. Windows hardware acceleration and vendor paths

**Findings**

- Four practical encode paths on Windows: NVIDIA **NVENC**, Intel **Quick Sync**, AMD **AMF**, and CPU software. `[REPORTED]`
- Windows 11 exposes a **D3D12 video encoding** framework covering H.264 and HEVC with an extensible foundation for newer codecs, explicitly intended as a layer that higher-level media APIs (e.g. Media Foundation) can build on, abstracting hardware differences. `[VERIFIED]` — this matches the Microsoft documentation the spec cites.
- Vendor maturity is uneven but the gap has narrowed: NVENC is the most mature; AMD's RDNA 3/4 closed much of the quality gap and added AV1 hardware encode. `[REPORTED]`
- **HEVC decode is not guaranteed present on Windows.** Media Foundation HEVC support may require the Microsoft Store "HEVC Video Extensions" package (paid, ~$0.99) or the "HEVC Video Extensions from Device Manufacturer" variant (free where the OEM pre-paid). `[REPORTED]` This is a user-visible functionality gap on a required format.

**Implication**

- Prefer FFmpeg's own hwaccel integration (`d3d11va`/`dxva2` for decode, `h264_nvenc`/`hevc_nvenc`, `h264_qsv`/`hevc_qsv`, `h264_amf`/`hevc_amf` for encode) over writing three vendor SDKs ourselves. It is already abstracted, already tested, and keeps one code path.
- Do **not** depend on the Store HEVC extension. Our FFmpeg build includes HEVC decode, so we can decode HEVC without it. That solves the functionality gap and creates a patent question instead (R19) — an honest trade, not a free lunch.
- The spec's `HardwareAccelerationManager` sketch (§11) is directionally right but insufficient. **Driver-advertised capability is not proof of working capability.** The manager must probe by actually creating a short-lived encode and decode session for each candidate path at first run, cache the verified result keyed by GPU + driver version, and re-probe when that key changes. Falling back to CPU after a failure mid-export is a bad user experience; failing over at probe time is not.

**Open items**

- `[MEASURE]` Which of NVENC/QSV/AMF our FFmpeg build must enable, and whether each is redistributable in an LGPL configuration (NVENC appears to be — see R18).
- `[MEASURE]` Whether D3D12 video encode is worth using directly in V1 versus letting FFmpeg's vendor encoders handle it. Bias: **no** for V1; it is a lower-level path with more code and no user-visible benefit yet.

---

## R10. Audio processing libraries

**Findings**

- Rust audio ecosystem is adequate for this product: `[REPORTED]`
  - **Symphonia** — pure-Rust decode/demux for AAC, ALAC, FLAC, MP1/2/3, MP4, OGG/Vorbis, WAV, AIFF, CAF, MKV/WebM. Self-described maturity per-feature ("Great" = complete and acceptable for most applications).
  - **rubato** — sample-rate conversion.
  - **cpal** — cross-platform playback; explicitly does *not* resample.
- FFmpeg already provides `libswresample` (resampling), `libavfilter` (gain, EQ via `equalizer`/`highpass`/`lowpass`, `loudnorm` for EBU R128 normalisation, `atempo` for speed), all under LGPL when configured correctly.

**Implication — do not adopt two audio stacks**

The spec (§3) wants audio to be first-class and independent of video. That is a *UI* requirement, not a reason for a second decoding stack. Recommendation:

- **Decode/resample/filter:** FFmpeg (`libavcodec`/`libswresample`/`libavfilter`). Already present, already licensed, already validated for the video path, handles audio-in-video identically to standalone audio files. Symphonia would be a second implementation of the same thing with different bugs.
- **Playback device I/O:** `cpal` (cross-platform) or platform-native (CoreAudio / WASAPI). `[MEASURE]` — `cpal` first, native only if latency or device-change handling proves inadequate.
- **Normalisation:** `loudnorm` (EBU R128) — the correct meaning of "Normalize" for a media editor, better than naive peak normalisation.
- **Pitch shift (§3, "where technically reliable"):** this is the one audio feature with a licensing trap. Rubber Band Library is GPL with a separate commercial licence; SoundTouch is LGPL. `[COUNSEL]`/`[MEASURE]` — verify both before writing a line of pitch code. Recommendation: **cut pitch shifting from V1** (see scope challenge).

---

## R11. Timeline architecture

**Findings and recommended model** `[ASSUMED]` — this is design, informed by prior art in NLE construction.

- **Time is rational, not float and not frame-index.** Store times as `i64` ticks on a fixed high-resolution timebase (e.g. 1/705600000 s, divisible by both 24000/1001-family and 48 kHz-family rates) or as explicit rational `num/den`. Floating-point time in an NLE produces drift and off-by-one-frame bugs that are nearly impossible to fix later.
- **Variable frame rate (VFR) input is mandatory to handle and is entirely absent from the spec.** Screen recordings and phone video are commonly VFR. A frame-index-based timeline silently desyncs audio on VFR sources. The engine must be PTS-driven throughout.
- **Document + command pattern.** The project is an immutable-ish document; every edit is a command with `apply`/`invert`. Undo/redo (§28) is a stack of commands, never a copy of media — which is exactly what §28 asks for. Commands are also the natural unit for autosave journalling (§24) and for a future scripting/automation surface.
- **Derived render graph.** The timeline document is the source of truth; the render graph and any caches are derived and invalidated by *time range*, not wholesale. This satisfies §26's "never re-render everything when one timeline item changes".

**Implication for the FFI**

The Rust core owns document truth. The UI must not keep a second mutable copy. The UI receives either (a) immutable snapshots, or (b) change notifications describing what invalidated. Two mutable copies of the timeline on either side of an FFI boundary is the classic way this architecture fails.

---

## R12. Proxy architecture

**Findings**

- The spec (§6) mandates a proxy/preview representation. Correct instinct, but the naïve implementation — transcode every file to a proxy on import — directly contradicts §1's "feels fast" and would make importing a 4K file a multi-minute wait.
- Hardware decode of 4K H.264/HEVC on Apple Silicon is fast enough that proxies may be unnecessary for simple projects. `[MEASURE]` — this is the single most important unknown in the product.

**Recommended tiered strategy** `[ASSUMED]`, to be validated by measurement:

1. **Tier 0 — direct hardware decode at full resolution.** Try this first. If the timeline holds target fps, no proxy is ever created.
2. **Tier 1 — reduced-resolution decode/scale on the media engine or GPU.** Cheaper than a proxy, no disk cost, no wait.
3. **Tier 2 — background proxy generation**, triggered *only* when Tier 0/1 fail to hold frame rate for a given source, or when the user opts in. Proxy generation is a background job, cancellable, and the timeline keeps working (degraded) while it runs.
4. **Never block import on proxy generation.**

**Open items**

- `[MEASURE]` Phase 1: on M-series hardware, how many simultaneous 4K H.264 / 4K HEVC / 4K60 decode streams can we sustain at 30 fps preview? The answer determines whether Tier 2 exists in V1 at all.
- `[MEASURE]` Same question on a mid-range Windows laptop with Intel iGPU only — likely the worst realistic target and the one that decides the proxy policy.

---

## R13. Cache architecture

**Recommended model** `[ASSUMED]`

- Cache entries keyed by **content identity**, not path: `(hash of first+last N bytes + size + mtime, stream index, parameters, cache format version)`. Path-keyed caches break on the spec's own §42 failure cases (moved files, disconnected drives, renamed media).
- Cache classes with independent budgets and eviction: `thumbnails`, `waveforms`, `proxies`, `render`. Proxies are expensive to rebuild and cheap to keep; thumbnails are the reverse. One global "clear cache" button that nukes proxies is a bad experience.
- **Cache is never the source of truth** (§14, agreed) and every cache read must be checksum-validated or version-gated so a corrupt or half-written entry degrades to a regeneration, never to a corrupt project.
- Cache lives outside the project file, in a user-configurable location, defaulting to the platform cache directory. Placing proxies next to source media is convenient but violates §46 Rule 9's spirit by writing into the user's media folders.
- Writes are atomic (temp file + rename) so a crash or power loss leaves no partial entry.

---

## R14. Project file format

**Recommended model** `[ASSUMED]`

- **Single file**, not a bundle directory. Bundles are a macOS idiom that translates badly to Windows and to cloud-sync folders. A single file is trivially copyable, e-mailable, and version-controllable.
- **Format:** JSON (UTF-8, pretty-printed, stable key order) for V1. Human-readable and diffable, which is worth more during development than the binary size saving. Add a `schema_version` integer from day one and write a migration path before the first public build. If profiling later shows JSON parse time matters for very large projects, switch the on-disk encoding to CBOR behind the same schema — but that is a V1.1 optimisation, not a V1 concern.
- **Contents:** metadata, timeline (tracks/clips/transitions/text/effects), source references, export settings — matching §13.
- **Source references** store: absolute path, a relative path from the project file, content hash, duration, and format summary. Relinking (§42 "missing media") searches by hash then by filename. This must exist in V1; the spec implies it in §42 but never requires it.
- **macOS sandbox:** references must additionally store **security-scoped bookmarks**, or a sandboxed build loses access to the user's media on the next launch. See R15 — this is the detail that most often gets discovered late and forces a format change.
- **Saving is atomic:** write temp, fsync, rename. Autosave writes a *journal* of commands alongside the last full save rather than rewriting the whole document (§24 "must not block the UI").

---

## R15. macOS sandbox requirements

**Findings**

- The App Sandbox entitlement is `com.apple.security.app-sandbox`; sandboxed + notarized apps are increasingly the norm. `[REPORTED]`
- A sandboxed app only retains access to user-selected files across launches via **security-scoped bookmarks**. `[ASSUMED]` — well-established platform behaviour; confirm in Phase 1.
- A non-destructive editor is, by definition, an app that holds long-lived references to files the user chose weeks ago. Sandbox and non-destructive editing interact directly.
- Helper/worker processes inherit sandbox constraints and need their own entitlements and signing. `[ASSUMED]` — the media worker process design (R3/R20) must be validated inside the sandbox in Phase 1, not after.

**Implication**

- Sandbox from day one if we ever intend to ship on the Mac App Store, because retrofitting bookmarks into a project format and a multi-process design is expensive.
- Recommendation: **build sandboxed, distribute Developer ID first.** Sandboxing a Developer ID build costs little if designed in and preserves the App Store option.

---

## R16. macOS signing and notarization

**Findings**

- Notarization requires the **Hardened Runtime**; the notary service rejects builds without it. `[REPORTED]`
- **Library Validation** requires every dylib loaded into the process to be signed by Apple or by the same Team ID as the host. `[REPORTED]`
- `com.apple.security.cs.disable-library-validation` exists to load third-party libraries, but it deliberately weakens the hardened runtime and has been implicated in permission-stealing attacks against apps that use it. `[REPORTED]`

**Implication — a genuinely useful finding**

We are building our own FFmpeg dylibs (required anyway for LGPL, R18). Because *we* build them, *we* sign them with our Team ID, so **Library Validation is satisfied and `disable-library-validation` is not needed.** Anyone who reaches for that entitlement on this project has taken a wrong turn.

Every binary in the bundle — app, helper/worker executables, frameworks, every FFmpeg dylib — must be signed with the same Developer ID and hardened, then the bundle notarized and stapled.

---

## R17. Windows packaging and signing

**Findings** `[ASSUMED]` / `[MEASURE]` — least-researched area in Phase 0 and flagged as such.

- Code signing on Windows now effectively requires an EV or OV certificate with a hardware/HSM-backed key; SmartScreen reputation builds over time and unsigned or newly-signed installers are actively scary to users.
- Installer options: MSIX (modern, Store-compatible, sandboxed, more constraints) versus MSI/EXE via WiX or Inno Setup (traditional, fewer constraints). WinUI 3 supports both packaged and unpackaged deployment.
- Updates: Windows has no Sparkle equivalent; either MSIX auto-update, or a custom updater, or a third-party framework.

**Open items**

- `[MEASURE]` Decide packaged (MSIX) vs unpackaged early — it affects file access, update mechanism, and the Store option.
- `[MEASURE]` Certificate procurement lead time. This has historically blocked releases; start it during Phase 1, not Phase 9.

---

## R18. FFmpeg licensing

This is the highest-consequence section in the document.

**Findings** `[VERIFIED]` from ffmpeg.org/legal.html unless noted.

- FFmpeg's core is **LGPL v2.1 or later**. "FFmpeg incorporates several optional parts and optimizations that are covered by the GNU General Public License (GPL) version 2 or later. If those parts get used the GPL applies to all of FFmpeg."
- LGPL compliance requires building **without `--enable-gpl` and without `--enable-nonfree`**.
- `--enable-nonfree` produces a binary that is **not redistributable at all**. `[REPORTED]`
- Distribution obligations for LGPL use: **link dynamically**; distribute FFmpeg's source (modified or not) from the same server as the binaries; state that the software "uses code of FFmpeg licensed under the LGPLv2.1"; mention FFmpeg in the about box and EULA; do not rename the FFmpeg libraries to obscure them.
- `libx264` and `libx265` are on FFmpeg's GPL external-library list. Enabling either produces a GPL build. `[REPORTED]`
- **NVENC is in neither the GPL nor the nonfree list** — the NVIDIA headers (`nv-codec-headers`) are MIT-licensed, so NVENC support survives in an LGPL build. `[REPORTED]` — must be re-verified against our actual FFmpeg version at build time.
- FFmpeg's own page warns that patents are a separate matter: "once you start trying to make money from patented technologies, the owners of the patents will come after their licensing fees."

**Implication — the build we ship**

| Requirement | Setting |
|---|---|
| GPL components | **none** — never pass `--enable-gpl` |
| Nonfree components | **none** — never pass `--enable-nonfree` |
| `--enable-version3` | avoid unless a specific LGPLv3 component is needed and cleared |
| `libx264`, `libx265`, `libfdk-aac` | **excluded** |
| H.264/HEVC encode | hardware only: `videotoolbox` (macOS), `nvenc`/`qsv`/`amf` (Windows) |
| Linking | **dynamic**, dylibs/DLLs shipped in the bundle, unrenamed |
| Source availability | our exact FFmpeg source + build script published on the same server as downloads |
| Attribution | about box + EULA + third-party notices file |

The FFmpeg build must be a **checked-in, reproducible build script** producing a versioned artifact, not something an engineer configures by hand. The Homebrew FFmpeg on this machine (`--enable-gpl --enable-version3 --enable-libx264 --enable-libx265`) is the counter-example: it is exactly what we must never ship, and it is what will be on every developer's `PATH`.

**Open items**

- `[COUNSEL]` Full LGPL compliance review of the shipping build before first public distribution.
- `[MEASURE]` A CI check that fails the build if the linked FFmpeg reports GPL or nonfree configuration flags. This is cheap and prevents the most likely licensing accident.

---

## R19. Codec licensing and patent considerations

**This is a separate legal layer from R18 and is not solved by choosing LGPL.** Copyright licence (FFmpeg) and patent licence (AVC/HEVC) are independent obligations.

**Findings**

- **AVC/H.264 — Via LA.** Royalties are apportioned across the value chain with annual caps and thresholds below which royalties are not charged. `[REPORTED]` In 2026 Via LA restructured *streaming* fees into tiers topping out at $4.5M/yr for the largest platforms, applying only to newly licensed implementers from 2026 onward. `[REPORTED]` That tier change targets streaming services, not desktop software, but it signals a licensor that is actively repricing.
- **HEVC/H.265 — consolidating.** Access Advance administers the HEVC Advance pool (29,000+ patents) and has **acquired the administration of Via LA's HEVC/VVC pools**, folding them into a "VCL Advance" program. `[REPORTED]` HEVC has historically been the worst-case codec for licensing because of multiple pools plus unpooled holders; consolidation may simplify it, but it does not make it free.
- Access Advance licenses "HEVC Decoders and/or HEVC Encoders installed in devices **or software**", seeking one royalty per device/software copy at first sale. `[REPORTED]` **Desktop software is explicitly in scope.**
- **The key open question:** does using an OS/hardware-provided encoder (VideoToolbox, Media Foundation, NVENC) discharge the *application developer's* obligation, on the theory that Apple/Microsoft/NVIDIA already paid for that unit? Research did not find an authoritative statement either way; sources note it is up to each implementer to determine its own licensing needs. `[COUNSEL]` — **this must not be assumed.** It is widely relied upon in practice and it is not the same thing as being verified.
- **Cisco OpenH264** pays Via LA royalties for **binaries Cisco itself builds and distributes**, but only under conditions: the binary must be downloaded separately to the end user's device and **not pre-bundled** into third-party software, the user must be able to control its use, and the app must display "OpenH264 Video Codec provided by Cisco Systems, Inc." `[REPORTED]` This is a workable but awkward fallback for a *software* H.264 encoder; OpenH264's quality and profile support are also limited.
- **AV1** is royalty-free under the AOMedia Patent License 1.0. Sisvel operates an AV1 pool with non-AOMedia patent holders seeking per-device royalties; Sisvel has to date licensed hardware implementations and has explicitly kept software options open. No successful AV1 suit against a major implementer is reported. `[REPORTED]`
- **AAC** is separately licensed (Via LA). Note that FFmpeg's native AAC encoder is LGPL-clean from a copyright standpoint but carries the same patent-layer question. `[COUNSEL]`

**Implication — the codec strategy that minimises exposure**

1. **Ship no software H.264/HEVC encoder.** Encode via the OS/hardware encoder only. This reduces exposure and matches the performance strategy.
2. **Accept that decode is also in scope.** We decode H.264/HEVC in software via FFmpeg when no hardware path exists. This is a real, disclosed exposure, not something to paper over.
3. **Offer AV1 and Opus/FLAC/WAV as the "no-royalty-question" export lane** where hardware supports it, and be honest in the UI that MP4/H.264 remains the compatibility default.
4. **Budget for licences.** Treat AVC and HEVC royalties as a line item in the business model from day one, not a surprise. Per-copy royalties on a paid desktop app are usually survivable; discovering them after launch is not.
5. `[COUNSEL]` Obtain a written opinion on items 1–3 *before* first public distribution, and re-check when Access Advance's consolidated VCL Advance terms are published.

---

## R20. Memory and performance architecture

**Findings and recommended model** `[ASSUMED]` unless noted; all targets are `[MEASURE]`.

- **Process model:**
  - `App` (UI, Swift/C#) — never decodes, never encodes.
  - `MediaWorker` (Rust + FFmpeg), one or more — decode, encode, analysis, conversion jobs. Crash-isolated, hard-killable, memory reclaimed on exit.
  - This is what makes §22 ("never crash because of an invalid media file") and §42 (corrupt/truncated input) achievable rather than aspirational, and it makes cancelling a stuck export possible.
- **Never load a whole file.** Stream, decode on demand, keep a bounded ring of decoded frames around the playhead. §26 already says this; the architecture must make it structurally hard to violate.
- **Bounded caches with explicit budgets**, evicted LRU under memory pressure (macOS memory-pressure notifications; Windows equivalents).
- **No media on the main thread. Ever.** Enforce in code review and, where possible, with a debug assertion that fires if a decode call happens on the UI thread.

**Proposed performance budgets** — these are targets to be ratified or corrected by Phase 1 measurement (§41 lists tests but defines no pass/fail, which is a gap):

| Metric | Target |
|---|---|
| Cold start to interactive window | ≤ 1.5 s (Apple Silicon), ≤ 2.5 s (mid-range Windows) |
| Import 2-min 4K clip → visible with first thumbnail | ≤ 1.0 s |
| Scrub latency (drag → correct frame shown), p95 | ≤ 100 ms at preview quality |
| Timeline UI frame rate | ≥ 60 fps sustained; 120 fps on ProMotion; no main-thread block > 16 ms |
| Playback, 4K30 H.264, 1 track, no effects | ≥ 29.5 fps sustained on M1-class |
| Export, 2-min 4K → 1080p H.264, hardware | ≤ 60 s on M1-class (≥ 2× realtime) |
| Peak RSS, 20-clip 4K project, idle | ≤ 1.5 GB |
| Peak RSS during 4K export | ≤ 3 GB |
| Idle CPU with project open, not playing | < 1% |
| Crash-free session rate on the test corpus | ≥ 99.5% before release |

---

## 21. Gaps in the master specification

Areas the spec does not address that materially affect architecture. Each is expanded, with a recommendation, in `PHASE_0_VALIDATION.md` §3.

1. **Colour management and HDR.** 4K HEVC input is frequently 10-bit HLG or PQ. The spec never mentions colour space, transfer function, or tone mapping. Mishandling this produces washed-out or over-saturated exports — a highly visible V1 failure.
2. **Variable frame rate.** Absent from the spec; a primary cause of A/V desync with phone and screen-recorded footage.
3. **Smart/passthrough export.** §12 uses the words "Smart Export" but describes an ordinary export dialog. The real win — remuxing without re-encoding when the edit is cut-only and formats already match — is missing, and it is the largest possible perceived-speed improvement for the most common edit.
4. **Timecode, drop-frame, and pixel aspect ratio.** Unspecified.
5. **Audio channel layouts beyond stereo.** Unspecified; 5.1 sources are common in MKV.
6. **Crash reporting.** §25 forbids telemetry, but shipping with no crash reporting at all makes §44's "no known reproducible crashes" unverifiable in the field. Needs an explicit opt-in stance.
7. **Update mechanism.** Not mentioned. Affects packaging and signing decisions on both platforms.
8. **Licensing/activation** for a commercial product. Not mentioned; interacts with §25 privacy.
9. **Internationalisation.** Not mentioned; retrofitting is expensive.
10. **Accessibility** appears only in §43 "polish", which is too late for a custom-drawn timeline — accessibility for custom views must be designed in.

---

## Sources

- [FFmpeg License and Legal Considerations](https://ffmpeg.org/legal.html)
- [FFmpeg LICENSE.md](https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md)
- [Microsoft — D3D12 Video Encoding](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/video-encoding-d3d12)
- [Apple — VideoToolbox documentation](https://developer.apple.com/documentation/videotoolbox)
- [Via LA — AVC/H.264 licence fees](https://www.via-la.com/licensing-programs/avc-h-264/)
- [Access Advance — HEVC Advance](https://accessadvance.com/licensing-programs/hevc-advance/)
- [Access Advance / Via LA HEVC-VVC program acquisition](https://www.accessnewswire.com/newsroom/en/electronics-and-engineering/access-advance-and-via-licensing-alliance-announce-hevc%2Fvvc-program-acq-1117638)
- [Access Advance — what we license](https://accessadvance.com/topic-what-do-we-license/)
- [OpenH264 binary licence](https://www.openh264.org/BINARY_LICENSE.txt) and [OpenH264 FAQ](https://www.openh264.org/faq.html)
- [Sisvel — AV1 licensing programme](https://www.sisvel.com/licensing-programmes/audio-and-video-coding-decoding/video-coding-platform-av1/)
- [swift-bridge](https://github.com/chinedufn/swift-bridge) · [UniFFI Swift bindings](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html)
- [rusty_ffmpeg](https://crates.io/crates/rusty_ffmpeg) · [rust-ffmpeg (ffmpeg-next)](https://github.com/zmwangx/rust-ffmpeg)
- [Symphonia](https://crates.io/crates/symphonia) · [Rust audio crates index](https://lib.rs/multimedia/audio)
- [WinUI 3 SwapChainPanel](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swapchainpanel) · [DirectX and XAML interop](https://learn.microsoft.com/en-us/windows/uwp/gaming/directx-and-xaml-interop)
- [Build 2026: Microsoft makes WinUI the native production platform](https://windowsnews.ai/article/build-2026-microsoft-makes-winui-the-native-production-platform-for-windows-apps.422404)
- [Apple — Use SwiftUI with AppKit and UIKit (WWDC26)](https://developer.apple.com/videos/play/wwdc2026/272/)
- [SwiftUI scroll performance: the 120fps challenge](https://blog.jacobstechtavern.com/p/swiftui-scroll-performance-the-120fps)
- [Notarization: the hardened runtime](https://eclecticlight.co/2021/01/07/notarization-the-hardened-runtime/) · [Apple Developer Forums — disable-library-validation](https://developer.apple.com/forums/thread/799497)
- [Talos — macOS library validation abuse](https://blog.talosintelligence.com/how-multiple-vulnerabilities-in-microsoft-apps-for-macos-pave-the-way-to-stealing-permissions/)
- [Microsoft Learn — H.265/HEVC video decoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-265---hevc-video-decoder)
