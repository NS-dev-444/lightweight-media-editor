# Architecture Decision Record

**Project:** Lightweight cross-platform video + audio editor
**Document status:** Phase 0 deliverable 2 of 5
**Date:** 2026-09-09
**Depends on:** `PHASE_0_ARCHITECTURE_RESEARCH.md`

---

## Status of this document

These decisions are **provisional pending Phase 1 measurement**. Spec §34 requires the decisions to be internally consistent before proceeding; it does not require them to be irreversible, and three of them explicitly cannot be finalised by reading (marked **GATED**). A GATED decision states the intended answer *and* the measurement that would overturn it.

Decision states used below:

- **DECIDED** — settled in Phase 0; changing it later is expensive.
- **GATED** — provisional; Phase 1 measurement confirms or overturns it.
- **DEFERRED** — deliberately not decided yet; deciding early buys nothing.

---

## AD-1. What language? — **DECIDED**

**Decision:** Rust for the shared core; Swift for the macOS application; C# for the Windows application; Metal Shading Language and HLSL for shaders.

**Scope of the Rust core — what it owns:**

- Project document model, serialisation, migration
- Timeline/EDL semantics, clip and track model, rational time
- Command/undo/redo stack and autosave journal
- Media probing, format/codec identification, capability reporting
- Decode/encode orchestration over FFmpeg (`libav*`)
- Waveform analysis and thumbnail extraction
- Cache and proxy management, eviction
- Job scheduler and the conversion queue
- Error classification (raw codec errors → stable, translatable error codes)

**What the Rust core deliberately does not own:**

- Any UI
- The frame compositor and effect shaders (native per platform: Metal / D3D)
- Audio device I/O
- Platform integration: file dialogs, drag-and-drop, sandbox bookmarks, menus, notarization-sensitive behaviour

**Rationale:** Rust's value here is precisely the code we would otherwise write twice. Compositing and device I/O are *not* that code — they are inherently platform-specific and would be worse if abstracted. Splitting on this line keeps the shared core large enough to be worth the FFI cost and small enough that neither platform fights it.

**Caveat, and its resolution (2026-09-09):** the stated caveat was that if Windows left V1, this decision should be re-opened rather than inherited — a macOS-only product is meaningfully simpler in Swift end-to-end, and the Rust core is justified *by the Windows requirement* alone.

O-1 resolved to **macOS V1, Windows V1.1**. Windows is **deferred, not dropped**, so the premise holds and **AD-1 stands**. But it now rests on an ongoing discipline rather than a one-time decision: the core must stay genuinely portable through a long stretch of macOS-only development. If the Windows CI build is allowed to rot, the V1.1 "port" becomes a rewrite and this decision becomes retroactively wrong. Tracked as `RISK_REGISTER.md` **R-21**, with the concrete requirement that a Windows build of the core stays green in CI throughout V1.

**Alternatives rejected:**

| Alternative | Why rejected |
|---|---|
| C++ shared core | Larger safety surface in exactly the code parsing untrusted media; worse dependency tooling; the concurrency this app needs is where C++ hurts most. |
| Electron / web stack | Directly contradicts §47 ("should NOT feel web-based") and the 4K memory budget. |
| Two independent native implementations | Doubles every media bug and guarantees behavioural divergence between platforms. |
| Swift on both platforms | Swift on Windows is not a credible foundation for a shipping GUI product today. |

### AD-1 re-examined after Phase 1 (2026-09-09) — verdict: **keep Rust, for a narrower reason**

Phase 1 produced real evidence rather than expectations, and it changes the *argument* for Rust even though it does not change the choice.

**What building it actually showed:**

1. **The FFmpeg-facing layer is almost entirely `unsafe`.** Every line of `mc_open`, `mc_next_frame`, `mc_probe` and `mc_waveform` is inside an `unsafe` block, because it is raw C FFI. **Rust's safety guarantees buy very little there** — that code is as dangerous as C, just with more ceremony.
2. **AD-3's worker process weakens the safety argument further** for that layer: a crash is already contained. (Not eliminated — a memory bug in the worker is still a *security* problem, since it parses untrusted input, even when it is no longer an availability problem.)
3. **But the FFI layer is the small part.** The timeline, project document, command/undo stack, cache policy and job scheduler are large, long-lived, concurrent, and entirely our own logic. That is where memory safety and data-race freedom actually pay, and it is essentially all of Phase 2 and Phase 4.

**Consequence — split the core into two crates** (validated during Phase 1):

| Crate | Contents | Properties |
|---|---|---|
| `mediacore-model` | project document, timeline, rational time, commands/undo, cache policy, scheduler | **No FFmpeg. Safe Rust. Unit-testable. Cross-checks for Windows from any machine.** |
| `mediacore-media` | FFmpeg FFI, decode/encode, probing, waveform | `unsafe` by nature; needs real Windows CI |

This was demonstrated: `mediacore-model` compiles for `x86_64-pc-windows-msvc` from macOS and its tests pass, so **R-21 becomes partially enforceable without a Windows runner**. The FFmpeg-bound crate still requires one.

**Alternatives, re-assessed with Phase 1 evidence:**

| Language | Honest assessment |
|---|---|
| **C++** | The serious alternative, and what the media industry actually uses. FFmpeg interop is trivial with no `unsafe` ceremony, and the talent pool is far larger. It loses on dependency tooling and build reproducibility — which matter more than usual here, because the licensing gates in `DEPENDENCY_AND_LICENSE_AUDIT.md` §7 depend on a controlled dependency graph — and on safety in the model layer, where most of our own bugs will live. **If the team is deep in C++ and thin in Rust, C++ ships sooner and the architecture is unchanged.** That is a team-composition decision (R-19), not a technical one. |
| **Zig** | Technically the *best* fit for a codebase that is largely C interop: `@cImport` consumes the FFmpeg headers directly, with no binding crate and none of the `rusty_ffmpeg` friction Phase 1 hit. **Disqualified on maturity** — pre-1.0, breaking changes between releases, tiny ecosystem and hiring pool. Not a defensible bet for a multi-year commercial product. Worth revisiting if it reaches 1.0 during the product's life. |
| **Go** | Wrong for this domain. GC pauses conflict with real-time preview, cgo has per-call overhead that matters at frame rates, and GC plus large frame buffers is a poor combination. |
| **Swift everywhere** | Still not credible on Windows. |
| **C# everywhere** | Would unify with the Windows UI but adds a runtime to macOS and P/Invoke on every FFmpeg call. |

**Verdict:** Rust stays, justified by the *model* layer rather than the FFI layer, and the crate split makes that justification explicit rather than assumed. The friction Phase 1 found is real but bounded: `rusty_ffmpeg` defaults to static linking (fixed with `FFMPEG_LINK_MODE=dynamic`), and the hand-written C header drifted (fixed by generating it with cbindgen).

---

## AD-2. What UI framework? — **macOS DECIDED · Windows GATED (V1.1)**

**macOS:** SwiftUI for the application shell; AppKit (`NSViewRepresentable`) for the Metal preview surface.

> **Updated 2026-09-09 after spike S2.** The planned **AppKit timeline is dropped**. Measured, SwiftUI `Canvas` renders 200 clips with waveforms and thumbnails at 3.29 ms p99 even with the entire timeline on screen — 40% of a 120 Hz frame budget. The Phase 0 concern (§R1) was based on reports of `List`/`LazyVStack` stutter, which stem from a backing view allocated per row; `Canvas` is immediate-mode and allocates nothing per clip, so that cost does not apply. **Confirmed by S2b:** dragging a 20-clip selection costs 0.74 ms p99 from model mutation to pixels, against a 16.67 ms budget, with 1 dropped frame in 299. Hit-testing 200 clips by linear scan costs ~8 µs, so no spatial index is needed for V1. The AppKit renderer is retained in the spike as a known-working fallback, and is ~2.5× cheaper if the timeline later grows keyframe rows or many more tracks. The Metal preview surface still needs `NSViewRepresentable`.

**Windows:** WinUI 3 / Windows App SDK (C#), with the preview hosted in a `SwapChainPanel` via `ISwapChainPanelNative.SetSwapChain`, and the timeline custom-drawn.

**Sequencing (O-1, resolved):** macOS V1, Windows V1.1. The macOS half of this decision is on the Phase 1 critical path; **the Windows half is now a V1.1 gate** and is no longer urgent. The WinUI 3 spike (S5) is deferred, but the Rust core still compiles for Windows in CI throughout V1 (R-21).

**Gate — what would overturn this:**

- *macOS:* if a SwiftUI `Canvas` timeline spike sustains 120 fps with ~200 clips, waveforms and thumbnails, drop the AppKit timeline and simplify. If neither approach holds 60 fps, the timeline becomes a Metal-drawn surface — which is a bigger change and must be known in Phase 1, not Phase 3.
- *Windows:* if a WinUI 3 spike shows startup time or idle memory that contradicts "lightweight" (§47), or if `SwapChainPanel` interop proves unstable, fall back to Win32 + C++ with Direct2D/DirectComposition and accept the higher build cost.

**Note on evidence quality:** Microsoft's Build 2026 recommitment to WinUI is a positive signal but it is vendor positioning, not evidence from our own build. It does not satisfy §46 Rule 2.

---

## AD-3. What media backend? — **DECIDED**

**Decision:** FFmpeg (`libavformat`, `libavcodec`, `libavfilter`, `libswscale`, `libswresample`), **dynamically linked**, built by us in an **LGPL-only configuration**, driven from Rust, running in a **separate media worker process**.

**Explicitly excluded from the build:** `--enable-gpl`, `--enable-nonfree`, `libx264`, `libx265`, `libfdk-aac`, and every other GPL or nonfree component.

**Hardware codecs enabled:** `videotoolbox` (macOS); `nvenc`, `qsv`, `amf`, `d3d11va`/`dxva2` (Windows) — all believed LGPL-compatible (`[REPORTED]`, to be re-verified against the shipping build).

**AVFoundation:** not used in the core media path. Verified during Phase 0 on macOS 26.6.2: AVFoundation refuses to open MKV and WebM, including an MKV whose H.264/AAC streams it opens happily when stream-copied into MP4 — a container limitation, not a codec one. §2 requires both containers, so FFmpeg must ship regardless; adding AVFoundation would mean two seek models and two timestamp models without removing either. Reconsider only for a future ProRes export preset. (Evidence: `PHASE_0_ARCHITECTURE_RESEARCH.md` §R4.)

> **Confirmed by spike S8 (2026-09-09):** the media worker must be signed with **`com.apple.security.app-sandbox` + `com.apple.security.inherit`**. Carrying `app-sandbox` alone kills the helper at exec with SIGTRAP and no diagnostic. With `inherit`, the worker inherits the parent's sandbox access and **can take a path** — so the C ABI needs no redesign around file descriptors. Descriptor passing was verified to work as a fallback.

**Rationale for the worker process:** §22 requires that the app never crash on an invalid or partially corrupted media file. That guarantee is not achievable with a codec library parsing untrusted input inside the UI process. Process isolation converts a crash into a recoverable job failure, makes stuck exports killable, and returns memory to the OS deterministically. It also keeps the LGPL dynamic-linking boundary clean.

---

## AD-4. What rendering backend? — **DECIDED**

**Decision:** Metal on macOS, Direct3D on Windows (D3D11 baseline, D3D12 only if a spike shows a reason). Effects are implemented twice — once in MSL, once in HLSL — from a single documented specification of each effect's maths.

**Rationale:** there are eight effects in §18. Writing eight small shaders twice is cheaper and more predictable than adopting a cross-platform shader abstraction, and it keeps each platform's path native and debuggable. This decision is only defensible *because* §32 forbids a large effects library; if the effect count ever grows past roughly 20, revisit with a cross-compilation approach.

**Frame path (zero-copy, both platforms):**

```
FFmpeg + hardware decoder
        ↓  (platform buffer, no CPU copy)
macOS:   CVPixelBuffer / IOSurface  →  CVMetalTextureCache  →  MTLTexture
Windows: ID3D11Texture2D            →  SRV
        ↓
Effect chain (GPU)
        ↓
Composite → CAMetalLayer / SwapChainPanel
```

Frame pixel data never crosses the FFI and never touches the UI language's heap.

---

## AD-5. How will 4K work? — **GATED**

**Decision (provisional):** a tiered preview strategy, not unconditional proxies.

1. **Tier 0** — full-resolution hardware decode. Used whenever it holds target frame rate.
2. **Tier 1** — reduced-resolution decode/scale on the GPU or media engine. No disk cost, no wait.
3. **Tier 2** — background proxy generation, triggered only when Tiers 0/1 cannot hold frame rate for a given source, or on explicit user request. Never blocks import; always cancellable.
4. Preview quality degrades automatically and visibly-but-unobtrusively under load; **timeline responsiveness always wins over preview fidelity** (§16).
5. **Export always reads original source media**, never proxies. Enforced structurally: the export path has no code path that can reach a proxy.

> **Partial result from spike S1 (2026-09-09):** 4K30 H.264 at 75 Mbps decodes at **183 fps steady state** on Apple Silicon through the zero-copy VideoToolbox path — roughly 6× the headroom required. **Tier 2 proxies look unnecessary on macOS.** With Windows now in V1.1 (O-1), the proxy decision is effectively deferred with it. Still to measure: 4K HEVC 10-bit and 4K60. Note the countervailing finding — 337 MB RSS per stream (R-23) — which constrains how many of these can run at once.

**Gate:** Phase 1 must measure sustained decode throughput for 4K30 H.264, 4K HEVC 10-bit, and 4K60, on (a) Apple Silicon and (b) a mid-range Intel-iGPU Windows laptop. If Tier 0/1 hold on both, Tier 2 may be cut from V1 entirely — which would be a significant simplification. If they fail on Windows, proxies become mandatory infrastructure and the import UX must be redesigned around them.

---

## AD-6. How will hardware acceleration work? — **DECIDED**

**Decision:** FFmpeg's hwaccel integration, wrapped in a `HardwareAccelerationManager` in the Rust core, with **runtime probing by actual use**.

Refinement of the spec's §11 sketch:

```
probe()          → attempt a real short encode+decode session per candidate path
capabilities()   → verified results only, never driver-advertised claims
selectDecoder()  / selectEncoder() / selectRenderPath()
fallbackToCPU()  → always available; never silently degrades without telling the user
```

**Key rule:** driver-advertised capability is not proof of working capability. Probe results are cached keyed by `(GPU model, driver version, OS build, app version)` and re-probed when that key changes. Probing costs a few hundred milliseconds once; discovering a broken encoder halfway through a user's 4K export costs the user their evening.

**Never assume a GPU exists** (§11) — a working CPU path must exist for every operation, and the UI must state plainly when it is in use, because the speed difference is user-visible.

---

## AD-7. How will audio work? — **DECIDED**

- **One decoding stack.** FFmpeg for decode, resample (`libswresample`), and filtering (`libavfilter`). Symphonia is not adopted: it would be a second implementation of decoding we already have, with different bugs and a second set of format quirks.
- **Internal format:** 32-bit float, planar, non-interleaved.
- **Project sample rate:** fixed per project (default 48 kHz), everything resampled on import. Mixing sample rates on a timeline is a large source of subtle bugs for no user benefit.
- **Timeline audio is sample-accurate**, video is frame-accurate, and both are expressed on the same rational timebase.
- **Normalisation** means EBU R128 loudness normalisation (`loudnorm`), not peak normalisation.
- **Device I/O:** `cpal` first; platform-native (CoreAudio/WASAPI) only if latency or device-hotplug handling proves inadequate. **GATED** on Phase 4.
- **The audio editor is a UI mode over the same engine**, not a separate engine. §3's "should work independently of the video editor" is a workflow requirement, satisfied by an audio-only project type.
- **Pitch shifting is cut from V1.** Rubber Band is GPL with a paid commercial licence; SoundTouch is LGPL but lower quality. The spec already hedges ("where technically reliable"), and neither the licensing nor the quality question is worth resolving for V1.

---

## AD-8. How will Windows differ from macOS? — **DECIDED**

| Layer | macOS | Windows |
|---|---|---|
| UI framework | SwiftUI + AppKit | WinUI 3 (C#) |
| Preview surface | `CAMetalLayer` | `SwapChainPanel` + DXGI swap chain |
| Renderer | Metal / MSL | Direct3D 11 / HLSL |
| HW decode | VideoToolbox | D3D11VA / DXVA2 |
| HW encode | VideoToolbox | NVENC / QSV / AMF |
| Audio I/O | CoreAudio (via cpal) | WASAPI (via cpal) |
| File access | security-scoped bookmarks | plain paths (unpackaged) |
| Packaging | Developer ID, notarized, stapled | signed installer, **DEFERRED**: MSIX vs MSI |
| Updates | Sparkle (licence to be verified) | **DEFERRED** |

**Identical across platforms:** project file format, timeline semantics, undo semantics, error codes and messages, export preset definitions and their resulting bitrates, cache layout, keyboard-shortcut *actions* (bindings follow platform convention).

Per §39: consistent user experience, platform-appropriate implementation. We do not force identical implementation details, and we do not allow the *output* to differ — a project exported on Windows must produce a byte-comparable-quality result to the same project exported on macOS, allowing for encoder differences.

---

## AD-9. How will projects be stored? — **DECIDED**

- **Single file**, JSON (UTF-8), `schema_version` from the first commit, with a migration function required before any format change ships.
- Contents per §13: metadata, timeline, source references, export settings.
- **Source references** carry: absolute path, project-relative path, content hash, duration, format summary, and (macOS) a security-scoped bookmark. Relink resolves by hash first, then filename, then user prompt.
- **Non-destructive by construction**: the project stores references and edit instructions only. No code path writes to a source file. This is enforced by making the source-file handle read-only at the type level in the Rust core, so "never destroy source media" (§46 Rule 9) is a compile-time property rather than a coding convention.
- **Atomic save:** temp file → fsync → rename.
- **Autosave** appends the command journal rather than rewriting the document, so it never blocks the UI (§24). Recovery replays the journal onto the last full save.
- **Cache lives outside the project file** and is never required to open a project.

---

## AD-10. How will cache and proxies work? — **DECIDED**

- Four independently budgeted classes: `thumbnails`, `waveforms`, `proxies`, `render`.
- **Keyed by content identity**, not path: `(content hash, stream index, parameters, cache format version)`. Survives moves and renames; distinguishes two files with the same name.
- Every entry is version-gated and integrity-checked on read; a bad entry regenerates rather than corrupting anything.
- Atomic writes (temp + rename). A crash mid-write leaves no partial entry.
- User-configurable location, default to the platform cache directory. **Never written next to the user's source media.**
- Clearable per class, with sizes shown. Proxies are expensive to rebuild and must not be silently discarded with thumbnails.
- **Cache is never the source of truth** (§14). Deleting the entire cache must never change a project's content or its export output — only its speed.

---

## AD-11. How will FFmpeg be distributed? — **DECIDED**

- **Our own build**, from a checked-in, reproducible build script, pinned to an exact FFmpeg version, produced by CI as a versioned artifact.
- **LGPL-only configuration** (AD-3). **Dynamically linked.** Libraries shipped inside the app bundle / install directory under their normal names — not renamed, not statically linked, not obscured.
- On macOS every dylib is signed with **our** Developer ID, which satisfies Library Validation. `com.apple.security.cs.disable-library-validation` **must not be used** — the entitlement weakens the hardened runtime and is unnecessary for libraries we build ourselves.
- **Published alongside downloads:** the exact FFmpeg source we built, our build script, and the diff if we ever patch it.
- **In-product attribution:** about box and EULA state that the software uses code from FFmpeg licensed under LGPL v2.1, with a full third-party notices screen.
- **CI gate:** the build fails if the linked FFmpeg's `configuration` string contains `--enable-gpl`, `--enable-nonfree`, or any denied component. This single check prevents the most likely licensing accident on the project — an engineer linking their Homebrew FFmpeg, which on this machine is a GPL build.

---

## AD-12. What are the licensing risks? — **DECIDED (posture), OPEN (legal)**

> ### Encoder strategy revision, 2026-09-09 (after spike S4)
>
> S4 measured a real quality gap between hardware and software encoding, firing the R-09 trigger (15% threshold, 38% measured median for H.264). The product owner selected the following, and it is now the encoder policy:
>
> 1. **HEVC is the default** wherever target compatibility permits. It beat hardware H.264 on every clip tested and on one clip beat software H.264 outright. H.264 remains the compatibility fallback, not the default.
> 2. ~~Re-test with quality-targeted rate control~~ **Done (S4b).** The ceiling was an ABR artefact and is withdrawn — but the efficiency gap is **worse** in each encoder's native mode: **2.31× (H.264), 1.74× (HEVC)**, not 1.38×/1.21×. Preset multipliers must use the S4b figures. HEVC's advantage over hardware H.264 is larger than first measured, which strengthens item 1.
> 3. **"Best Quality" is labelled honestly** as the best available *on this hardware*, not the best achievable. No preset name implies parity with a software encoder.
>
> **The patent posture is unchanged and remains the primary reason for hardware-only encode.** Shipping no software H.264/HEVC encoder is what keeps the exposure in §6 bounded; the quality argument was always secondary. Option 3 (adding a software encoder for a quality tier) was rejected: it reopens the whole audit, and `libx264` is GPL and therefore unavailable to this product regardless.


> ### Royalty-free codecs, 2026-09-10 (opened by §4's WebM requirement)
>
> AD-12's hardware-only rule exists for **patent** reasons, and that rationale
> does **not** reach VP8, VP9 or AV1: being royalty-free is the entire point of
> those formats, and there is no pool to owe. So "hardware encoders only" should
> be read as **"no software AVC or HEVC encoder"**, which is what it was always
> for, rather than as a rule about software encoding in general.
>
> That matters because §4 lists **MP4 → WebM**, and macOS has no hardware VP9
> encoder — so WebM export is either a software encoder or nothing.
>
> **Not decided yet, and deliberately not decided implicitly.** Adding libvpx
> means rebuilding FFmpeg and re-running the audit for a new component (BSD,
> LGPL-compatible, so the licence answer is expected to be easy). It also means
> shipping a slow encoder, which needs to be labelled honestly the way item 3
> above requires. **WebM → MP4 already works** through the Phase 6 transcode
> path, which is the more commonly wanted direction.

Two independent layers. Both must be cleared.

**Layer 1 — copyright (FFmpeg and other dependencies).** Addressed by AD-11 and `DEPENDENCY_AND_LICENSE_AUDIT.md`. Manageable with discipline and a CI gate.

**Layer 2 — patents (AVC, HEVC, AAC).** *Not* addressed by choosing LGPL, and not eliminated by any technical decision.

Our posture:

1. Ship **no software H.264/HEVC encoder**. Encode through the OS/hardware encoder only.
2. **Acknowledge decode exposure**: we decode H.264/HEVC in software when no hardware path exists. Access Advance licenses decoders in software explicitly. This is disclosed and budgeted, not hidden.
3. Provide **AV1 / Opus / FLAC / WAV** as the lane with no pool royalty demand, where hardware supports it, while keeping MP4/H.264 as the honest compatibility default.
4. **Budget AVC and HEVC royalties as a business line item** from day one.
5. `[COUNSEL]` — the assumption that an OS-provided encoder discharges the *app developer's* obligation is **widely relied upon and not verified**. It must be confirmed in writing by qualified counsel before first public distribution. Phase 0 research found no authoritative statement either way, and §46 Rule 3 forbids assuming a library is safe to distribute commercially.

**Cisco OpenH264** remains a documented fallback for a software H.264 encoder, but its conditions (separately downloaded, not pre-bundled, user-controllable, attributed) shape the product's installer design. Do not plan around it unless item 5 comes back badly.

---

## AD-13. What are the biggest technical risks? — **DECIDED (identification)**

Ranked. Full entries with mitigations and triggers in `RISK_REGISTER.md`.

1. **Two native applications in V1** (R-01) — schedule risk, larger than any technical risk here.
2. **Patent licensing exposure** (R-02) — could invalidate the business model rather than the architecture.
3. **4K preview performance on low-end Windows hardware** (R-03) — decides whether proxies are optional or mandatory.
4. **FFI boundary design, especially zero-copy frame handoff** (R-04) — cheap to get right now, very expensive to fix later.
5. **Accidentally shipping a GPL or nonfree FFmpeg build** (R-05) — low likelihood with the CI gate, severe impact without it.
6. **Colour management / HDR handling** (R-06) — an unspecified area that produces visibly wrong output.
7. **Variable frame rate handling** (R-07) — unspecified; causes A/V desync on very common footage.
8. **WinUI 3 interop and "lightweight" claim** (R-08) — provisional decision resting on vendor claims.
9. **`ffmpeg-next` maintenance-mode dependency** (R-11).
10. **Scope of the V1 MUST-HAVE list versus "lightweight"** (R-12).

---

## Decisions revisited after Phase 0

| Trigger | Effect |
|---|---|
| **O-1 → macOS V1, Windows V1.1** | AD-1 stands (Windows deferred, not dropped). AD-2's Windows half becomes a V1.1 gate. AD-8's platform table is unchanged, but its right-hand column is now V1.1 work. R-01 retired; **R-21 opened** — the port must not decay into a rewrite. |
| **O-16 → local processing approved** | **No architectural change.** Adds an MIT-licensed transcription runtime and model to the dependency set; both verified clean. Model-size strategy is O-18 / R-22. |

---

## Internal consistency check

§34 requires the decisions to be internally consistent. Checked explicitly:

| Claim | Consistent? | Reasoning |
|---|---|---|
| "Lightweight" (§47) vs Rust core + FFmpeg + two UIs | ✅ | Lightweight is about binary size, memory, and startup, not about internal structure. The multi-process design *helps* memory (workers exit and return memory). Bundle size is dominated by FFmpeg (tens of MB), which is acceptable. |
| "Never crash on bad media" (§22) vs in-process codecs | ✅ *after* AD-3 | Only achievable because decode was moved out of process. Had we linked codecs into the UI process, this requirement would have been undeliverable and the spec would have been quietly violated. |
| "Export uses original source" (§6) vs proxies (AD-5) | ✅ | Export path structurally cannot reach a proxy. |
| Hardware-only H.264/HEVC encode (AD-12) vs "excellent 4K performance" (§1) | ✅ | Same decision serves both; hardware encode is the fast path. |
| Hardware-only encode vs export quality expectations | ⚠️ | Hardware encoders are less efficient at matched quality. Mitigated by quality-first default bitrates and honest preset labels. Must be validated with VMAF in Phase 1. |
| Non-destructive editing (§13) vs macOS sandbox | ✅ *after* AD-9 | Only because security-scoped bookmarks are in the project format from v1 of the schema. Discovered late, this would force a format migration. |
| Privacy: no telemetry (§25) vs "no known crashes" (§44) | ⚠️ | Unresolved by the spec. Recommendation: opt-in, explicit, crash-report-only, no media, no file paths, off by default. Needs a product decision. |
| Single shared core vs native compositors (AD-4) | ✅ | The split is at the render-plan boundary; edit semantics stay shared, pixels stay native. |
| §2 "multiple video tracks where practical" vs §32 MUST-HAVE omitting them | ❌ | **The spec contradicts itself.** §30's UI mock shows V1/V2. Must be resolved before Phase 2. Recommendation in `PHASE_0_VALIDATION.md` §3.3: V1 ships one video track plus one overlay track; full multi-track in V1.1. |

Two ⚠️ items and one ❌ item remain. None blocks Phase 1 — all three are Phase 2 gating decisions, and the ❌ is a product decision the spec's author must make, not one an implementer should make silently.

**Update 2026-09-09:** the ❌ (multi-track contradiction) has a proposed resolution in `PRODUCT_DIRECTION.md` §6 — one video track plus one overlay track in V1 — pending sign-off. The privacy-vs-crash-reporting ⚠️ is O-4 and still open. The hardware-encode-quality ⚠️ is quantified by spike S4, which is written but awaiting a real test corpus.
