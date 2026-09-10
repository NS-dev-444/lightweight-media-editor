# Phase 1 — Results

**Project:** Lightweight cross-platform video + audio editor
**Document status:** ✅ **COMPLETE** — Phase 1 closed 2026-09-09
**Started:** 2026-09-09
**Host:** macOS 26.6.2 (25G83), Apple Silicon arm64, 14 cores · Swift 6.3.3 · rustc 1.96.0

Per spec §35 and §46 Rule 12. Spikes are throwaway proof-of-concepts with pass
conditions fixed in advance (`PHASE_0_VALIDATION.md` §6). **Results that
disappoint are recorded, not hidden** (Rules 6 and 11).

---

## Spike status

| # | Spike | Pass condition | Result |
|---|---|---|---|
| S3 | Reproducible LGPL FFmpeg build + licence gate | Reproducible; gate rejects a GPL build | ✅ **PASS** |
| S4 | Encoder quality benchmark, VMAF-scored | Preset bitrates chosen from data | ✅ **COMPLETE — changes AD-12** |
| S2 | Timeline rendering approach | ≥60 fps, 200 clips | ✅ **PASS — both approaches** |
| S2b | Timeline **interaction** latency (drag, hit-test, select) | ≤16 ms response, no drop during drag | ✅ **PASS — AD-2 settled** |
| S1 | Zero-copy Rust→Swift→Metal frame path | 4K30 ≥29.5 fps, zero copies | ✅ **PASS** |
| S6 | Worker crash isolation + fuzz | 0 UI-process crashes | ✅ **PASS** |
| S7 | Async waveform + cache | 60-min WAV ≤10 s | ✅ **PASS** |
| S8 | Sandbox + security-scoped bookmarks | Reopen with media access | ✅ **PASS** |
| S10 | Proxy necessity | Decides AD-5 Tier 2 | ⬜ not started |
| S5 | Windows WinUI 3 + D3D | 4K30, cold start ≤2.5 s | 🚫 no Windows host available |
| S9 | HW probe across ≥3 GPUs | Rejects non-working paths | 🚫 insufficient hardware |

---

## S3 — Reproducible LGPL FFmpeg build and licence gate ✅ PASS

**Artifacts:** `third_party/ffmpeg/build.sh`, `tools/license_gate.py`

### Source provenance — verified, not assumed

| Component | Version | Verification |
|---|---|---|
| FFmpeg | 8.1.2 | SHA256 pinned; **GPG signature verified** against key `FCF986EA15E6E293A5644F10B4322F04D67658D8`, cross-checked against the fingerprint published on ffmpeg.org/download.html |
| LAME | 3.100 | SHA256 pinned; licence read from source |

FFmpeg publishes `.asc` signatures but **no `.sha256` files** — the SHA256 pin in `build.sh` is ours, established from a GPG-verified download. The build script re-verifies both on every run.

**Version rationale (R-11):** 8.1.x is the newest series with a published Rust binding (`rusty_ffmpeg 0.17.0+ffmpeg.8.1`). `ffmpeg-next` targets 3.4–8.0 and is in maintenance mode, so this pin deliberately commits to `rusty_ffmpeg` plus our own thin safe wrapper. FFmpeg 9.0.1 exists but has no confirmed binding support.

### The gate works in both directions

This is the pass condition, and both halves matter — a gate that only ever passes proves nothing.

**Rejects the Homebrew build on this machine** (exit 1) — the exact R-05 contamination scenario:

```
config: --prefix=/opt/homebrew/Cellar/ffmpeg/9.0.1 ... --enable-version3
        --enable-gpl --enable-libx264 --enable-libx265 --enable-openssl ...

LICENCE GATE: FAIL
  ✗ DENIED FLAG  --enable-gpl        -- makes all of FFmpeg GPL
  ✗ DENIED FLAG  --enable-version3   -- pulls in LGPLv3 terms
  ✗ DENIED COMPONENT  libx264        -- GPL
  ✗ DENIED COMPONENT  libx265        -- GPL
  ✗ DENIED COMPONENT  openssl        -- use platform TLS
```

**Passes our build** (exit 0):

```
config: --prefix=.../ffmpeg-lgpl --enable-shared --disable-static
        --disable-programs --disable-doc --disable-autodetect
        --install-name-dir='@rpath' --enable-libmp3lame
        --enable-videotoolbox --enable-audiotoolbox --enable-neon

LICENCE GATE: PASS
```

The gate inspects the **built artifact**, not the build script — it reads the configuration string embedded in `libavutil`. A build script can be edited; a shipped binary cannot lie about how it was configured. This is what the release checklist means by "verified from the shipped binary".

### Two build flags worth calling out

- **`--disable-autodetect`** — without it, `configure` silently absorbs whatever is on the build machine (Homebrew's OpenSSL, Opus, and so on), making builds non-reproducible and the dependency set unknowable. For a product with licence obligations this flag is not optional.
- **`--install-name-dir=@rpath`** — required for bundling dylibs into a `.app`. Trivial to set now; painful to discover during code-signing in Phase 9.

### Finding: a minimal LGPL build cannot encode MP3

Measured against our own build:

| Encoder | Present | Note |
|---|---|---|
| `libmp3lame` | ❌ → ✅ after fix | Not built unless explicitly added |
| `mp3_at` (AudioToolbox MP3) | ❌ | **Absent on modern macOS** — there is no system fallback |

§4 requires WAV→MP3, FLAC→MP3, M4A→MP3 and OGG→MP3. None of them were possible.

**Resolution:** LAME added to the build. Its `COPYING` is the *GNU **Library** General Public License v2* and its source headers say "either version 2 of the License, or (at your option) any later version" — LGPL, upgradeable, compatible with FFmpeg's LGPLv2.1+. **It does not make the build GPL.** MP3 patents have expired, so there is no patent layer either.

One patch is applied: LAME 3.100 exports `lame_init_old` in its symbol file without defining it, which breaks the shared link on modern clang. Per LGPL this patch must be published as a diff alongside the source.

**Two wrong turns, recorded because the build script is the deliverable and both are the kind of thing that resurfaces:**

1. `--disable-decoder` was passed to LAME first. The shared link failed: LAME's symbol file still exports the decoder API (`lame_decode*`, `hip_*`) even when the decoder is excluded. Building the decoder is the correct fix — it is small, and we simply do not call it.
2. The script cleaned the install prefix immediately before `make install`. Because LAME and FFmpeg share one prefix, that deleted LAME's headers *after* FFmpeg had configured against them, and `make install` then failed re-compiling `libavcodec/libmp3lame.o` with `'lame/lame.h' file not found`. The prefix must be cleaned exactly once, **before** anything installs into it.

### Capability coverage of the LGPL-only build

| Requirement | Result |
|---|---|
| Decode H.264, HEVC, VP8, VP9, AV1, ProRes | ✅ all present |
| **Demux MKV / WebM** | ✅ — closes the AVFoundation gap measured in Phase 0 (§R4) |
| Encode H.264 / HEVC via VideoToolbox | ✅ — **AD-12's hardware-only strategy is viable in an LGPL build** |
| Encode ProRes via VideoToolbox | ✅ (bonus; not required for V1) |
| `libx264` / `libx265` | ✅ correctly **absent** |
| Decode MP3, AAC, FLAC, ALAC, Vorbis, Opus, PCM | ✅ all present |
| Encode AAC, FLAC, ALAC, Opus, PCM | ✅ all present |
| Encode AAC via AudioToolbox (`aac_at`) | ✅ — Apple's encoder, generally better than FFmpeg's native AAC; prefer on macOS |
| Encode MP3 | ✅ **after adding LAME** |

### Functional verification — files, not symbols

§46 Rule 10 forbids claiming support for a codec that has not been tested. A registered encoder is not a working one, so the build was made to **produce real files**, which were then probed and decoded by an independent tool (the Homebrew ffprobe/ffmpeg):

| Encoded by our LGPL build | Probe result | Round-trip |
|---|---|---|
| MP3 via `libmp3lame` | `mp3`, 44100 Hz, 2 ch, 192 kbps, 1.04 s | ✅ decodes clean |
| H.264 via `h264_videotoolbox` | `h264`, **High profile**, 1920×1080, 30 fps | ✅ decodes clean |

Both were written through `libavformat` muxing, not raw streams, so the container path is exercised too.

### Bundling readiness

Verified with `otool` — all install names are `@rpath`-relative, including LAME after an `install_name_tool` fixup (LAME's configure has no `--install-name-dir`):

```
libavcodec.dylib   @rpath/libavcodec.62.dylib
libavutil.dylib    @rpath/libavutil.60.dylib
libmp3lame.dylib   @rpath/libmp3lame.0.dylib

libavcodec deps →  @rpath/libswresample.6.dylib
                   @rpath/libavutil.60.dylib
                   @rpath/libmp3lame.0.dylib
```

The library set is ready to drop into a `.app` and sign. Because we build every dylib ourselves, they sign with our Team ID and **Library Validation is satisfied without `disable-library-validation`** (AD-11).

### API note for the Rust wrapper

The test compiled with deprecation warnings on `AVCodec.sample_fmts`; FFmpeg 8.x replaces the static `sample_fmts`/`pix_fmts`/`supported_samplerates` arrays with `avcodec_get_supported_config()`. Recorded here rather than suppressed (§46 Rule 4). **The Rust safe wrapper should target the new API from the start** — it is a straightforward difference now and a migration later.

**Conclusion:** an LGPL-only build satisfies every format requirement in §2, §3 and §4, and produces valid files. The licensing posture in AD-11/AD-12 costs no user-visible capability. That was the central open question behind S3 and it is now answered with evidence.

### What S3 did not prove

- **Reproducibility is asserted, not yet demonstrated.** The script pins and verifies sources and builds cleanly from scratch, but a bit-for-bit reproducible build has not been tested across two machines. Follow-up.
- **Encode *quality* is untested.** S3 proves the encoders produce valid files, not that they produce good ones. That is S4's job.
- **Windows configuration is written but unbuilt.** The `d3d11va`/`dxva2` branch has never run. `nvenc`/`qsv`/`amf` remain 🔍 VERIFY in the audit and are deliberately not enabled.
- **External libraries beyond LAME are not yet added.** Opus, dav1d and SVT-AV1 follow the same pattern — pin, verify, read the licence, record, enable.

---

## S2 — Timeline rendering ✅ PASS (both approaches)

**Artifact:** `spikes/s2-timeline/` (SwiftPM, throwaway per §35)

Two renderers draw a **pixel-identical** scene with **identical viewport culling** — 200 clips across 4 tracks, decimated waveform envelopes, and thumbnail strips — driven by a `CADisplayLink` with continuous scrolling, at three zoom regimes. Culling is shared deliberately: comparing without it would only measure who is faster at wasted work.

Display refresh measured at **8.33 ms (120 Hz ProMotion)**.

### Draw time, p99 (ms)

| Zoom | Visible clips | SwiftUI `Canvas` | Custom `NSView` | 120 fps budget |
|---|---|---|---|---|
| 80 px/s (normal edit) | 18 | **0.90** | **0.66** | 8.33 |
| 24 px/s (zoomed out) | ~50 | **2.08** | **1.26** | 8.33 |
| 8 px/s (**whole timeline visible**) | 134 | **3.29** | **1.29** | 8.33 |

Dropped frames (interval > 1.5× refresh) were 0–6 out of 239 per run in both, attributable to window ordering at run start.

### Verdict — and a Phase 0 assumption overturned

**Both pass at every zoom level, with 120 fps headroom.** At the worst realistic case SwiftUI Canvas uses **40% of a 120 Hz frame budget**; NSView uses 15%.

`PHASE_0_ARCHITECTURE_RESEARCH.md` §R1 leaned the other way — it called the timeline "the problem case" and expected AppKit to be necessary, citing reports of SwiftUI stutter at large item counts. **That reasoning does not survive measurement here.** Those reports concern `LazyVStack`/`List` allocating a backing `NSView` per row; `Canvas` allocates nothing per clip — it is an immediate-mode drawing surface, so the per-view cost that makes SwiftUI lists stutter simply does not apply. This is exactly the assumption Phase 1 exists to catch.

AD-2's stated gate was: *"if a SwiftUI `Canvas` timeline spike sustains 120 fps with ~200 clips, waveforms and thumbnails, drop the AppKit timeline and simplify."* **It does.**

### What S2 does not prove — and why the decision is provisional

Drawing was never the whole story, and this measured only drawing:

1. **Interaction latency is untested** — hit-testing, trim-handle drags, selection, multi-select. This is where SwiftUI's per-update overhead typically shows, and it is the actual felt experience of a timeline.
2. **Live editing is untested** — dragging a clip pushes state changes through SwiftUI's diffing every frame, which is a different cost profile from scrolling a static scene.
3. **Thumbnails are 8 cached `CGImage`s.** A real timeline decodes them asynchronously and handles cache misses mid-scroll.
4. **One machine, one display.** Apple Silicon at 120 Hz. A low-end Intel Mac would be slower — though 2.5× headroom leaves room.

**NSView is consistently ~2.5× cheaper.** That margin does not matter today; it would matter if the timeline grows keyframe rows, effect indicators, or many more tracks.

**Recommendation:** provisionally adopt **SwiftUI `Canvas`** for the timeline and drop the planned AppKit timeline from AD-2 — a real simplification. Confirm with **S2b (interaction latency)** before it is final. The AppKit renderer stays in the spike as a fallback that is already known to work.

*(Methodology note: the first run reported "frames over 8.33 ms" as a drop metric, which was wrong — at a 120 Hz refresh the interval sits at 8.33–8.36 ms, so almost every frame trips a naive `>` comparison. The metric was corrected to interval > 1.5× the measured refresh.)*

---

## S2b — Timeline interaction latency ✅ PASS — AD-2 settled

S2 animated only the *viewport*. S2b mutates the **content**: a 20-clip selection is dragged, so clip positions change every frame and the change must travel through the framework's full update cycle before pixels appear. For SwiftUI that is `@Published` → invalidation → diff → `Canvas` redraw — the path where per-update overhead was expected to show.

### Mutation → pixels (the number that decides how a drag feels)

| Renderer | p50 | p95 | p99 | max | Budget |
|---|---|---|---|---|---|
| SwiftUI `Canvas` | 0.54 | 0.69 | **0.74** | 2.80 | 16.67 ms |
| Custom `NSView` | 0.33 | 0.40 | **0.41** | 0.44 | 16.67 ms |

### Hit-test — naive linear scan over 200 clips

| Renderer | p50 | p99 | max |
|---|---|---|---|
| SwiftUI `Canvas` | 7.9 µs | 29.7 µs | 52.0 µs |
| Custom `NSView` | 7.9 µs | 20.0 µs | 44.5 µs |

Dropped frames during the drag: **1/299** (SwiftUI), **0/299** (NSView).

### Verdict

**Both pass with roughly 22× headroom.** SwiftUI's mutation-to-pixels latency of 0.74 ms p99 is far better than the Phase 0 concern implied — the `@Published` → diff → redraw path is simply not the bottleneck at this scale. NSView remains ~1.8× cheaper, but both are trivially inside budget.

**Two decisions follow:**

1. **AD-2 (macOS) is settled: SwiftUI `Canvas` for the timeline.** The AppKit timeline is dropped. One renderer, not two.
2. **No spatial index is needed for V1.** Hit-testing 200 clips by linear scan costs ~8 µs. Building a quadtree would be premature optimisation (§46 Rule 8).

### What S2b still does not cover

- **Input events are synthesised** at display-link rate rather than delivered as real `NSEvent`s. The measurement isolates the *framework-attributable* latency, which is the right thing to compare, but total felt latency also includes event delivery and coalescing — which neither framework controls.
- Undo/redo command flow through the same path.
- Selections much larger than 20 clips.
- Real thumbnails with asynchronous decode and cache misses mid-drag.

None of these change the recommendation; all belong to Phase 3.

*(A second measurement bug, caught and fixed rather than reported: the first draw happens before any mutation, so the initial sample measured against an uninitialised timestamp and reported ~11,507,000 ms — machine uptime. Percentiles were unaffected; mean and max were meaningless. Pre-mutation samples are now discarded.)*

---

## S1 — Zero-copy frame path ✅ PASS

**Artifact:** `spikes/s1-zerocopy/` — Rust core (`rusty_ffmpeg` + our LGPL FFmpeg + VideoToolbox) → C ABI → Swift → `CVMetalTextureCache` → `MTLTexture`.

This is the mitigation for **R-04**, the risk that is cheap to get right now and very expensive later.

**Source:** 3840×2160 H.264, 30 fps, 12 s, **75 Mbps** — deliberately heavier than typical consumer 4K so the decode work is realistic.

### Throughput

| Metric | Result |
|---|---|
| Sustained throughput | **107.9 fps** (3.6× realtime) |
| Steady state p50 | **5.46 ms/frame → 183 fps** (6.1× realtime) |
| Steady state p99 | 6.38 ms |
| First frame | **1054 ms** |
| Warm-up (frames 1–30) | 52.5 ms mean |

Reproducible across runs (107.5 / 107.9 fps).

### Zero-copy evidence — three independent proofs

| Check | Result |
|---|---|
| Software-decoded frames | **0 / 360** (must be 0) |
| IOSurface-backed buffers | **360 / 360** |
| Bound as `MTLTexture` | **360 / 360** |
| Peak RSS | 337.7 MB (budget 500 MB) |
| Frame bytes never copied | **4,271 MB** across the run |

### The ownership contract across the FFI

Rust returns a **+1 retained** `CVPixelBufferRef` as an opaque pointer; Swift consumes it with `Unmanaged<CVPixelBuffer>.fromOpaque(raw).takeRetainedValue()`, after which ARC owns it. That is the idiomatic pattern and it should be standardised across the whole data plane — it makes the transfer of ownership explicit at the one place it matters, and it means no manual release call can be forgotten.

`spikes/s1-zerocopy/include/mediacore.h` is the architectural deliverable: control plane (POD structs, freely crossed) is visibly separated from data plane (handles only, never pixels).

### Three findings that change other decisions

**1. AD-5's Tier 2 proxies are probably unnecessary on Apple Silicon.** 183 fps steady state for 4K30 is ~6× the headroom needed. Even with effects and several tracks there is substantial room. This is strong evidence for the macOS half of S10 — the decision now hinges almost entirely on low-end Windows hardware, which is V1.1.

**2. The first frame costs ~1 second**, and that is the "open a file" experience. `PHASE_0_ARCHITECTURE_RESEARCH.md` §R20 budgets ≤1.0 s to first thumbnail — decoder init plus VideoToolbox session setup plus Metal warm-up consumes the entire budget before a single frame is drawn. **This must be designed around**, not optimised later: pre-warm the VideoToolbox session at app launch, and extract a first thumbnail on a separate path rather than through the full playback pipeline.

**3. Peak RSS is 337 MB for a *single* decode stream.** §R20 budgets ≤1.5 GB for a 20-clip project. At ~300 MB per active decoder that allows only ~5 concurrent streams, which directly constrains multi-track playback. The likely cause is the VideoToolbox IOSurface pool plus FFmpeg's frame pool; both are tunable. **Needs investigation before Phase 2 sets the memory architecture.**

### Integration finding: `rusty_ffmpeg` defaults to static linking

The crate emits `cargo:rustc-link-lib=static=...` by default, which fails against our shared-only build and would violate the LGPL dynamic-linking strategy if it succeeded. **`FFMPEG_LINK_MODE=dynamic` is required** and is now part of the build environment. This is exactly the integration friction R-11 anticipated — recorded so nobody rediscovers it.

### What S1 does not prove

- **No shaders ran.** The texture is created from the IOSurface and bound to a render command encoder, and the command buffer completes — but there is no pipeline state or draw call, so the GPU never *samples* it. The binding is the step that would copy if done wrong, so the zero-copy claim holds; a full composite pass with effect shaders is Phase 3 work.
- **Only the luma plane** is bound. NV12 needs a second `r8Unorm`-pair texture for chroma; trivial, but not exercised.
- **Single stream, no seeking.** Multi-stream contention and seek latency are separate questions.
- **Synthetic source.** High-entropy noise at 75 Mbps stresses the decoder, but real footage has different GOP structure and motion characteristics.

---

## S6 — Worker-process crash isolation ✅ PASS

**Artifact:** `spikes/s6-isolation/` — corpus generator, worker process, host process.

Spec §22 requires the app never to crash on an invalid media file, and §42 requires graceful failure across a matrix of broken input. AD-3 claims process isolation is what makes that achievable rather than aspirational. S6 tests that claim.

**Corpus:** 1024 deliberately malformed files + 3 valid controls — 300 truncations, 300 single-bit flips, 150 burst corruptions, 80 header-only files, random garbage, zero-length, one-byte, HTML, plain text, truncated MKV and WAV, container/extension mismatches, absurd box sizes, and §42's hostile filenames (spaces, unicode, 180-character names, leading dashes, trailing dots).

**Structure:** the host process links **no media libraries at all** (verified with `otool`: zero `libav*` or `mediacore` references). It spawns one worker per file, enforces a timeout, and classifies every outcome. Only the worker parses media.

### Results

| Outcome | Count |
|---|---|
| Opened and decoded (OK) | 509 |
| NOT_MEDIA | 470 |
| NO_VIDEO_STREAM | 43 |
| TRUNCATED | 1 |
| DECODE_FAILED | 4 |
| **Host process crashes** | **0** |
| **Worker crashes** | **0** |
| **Timeouts / hangs** | **0** |

1027 files in 18.2 s. Every file produced a classified result; none was unaccounted for.

### Error messages (spec §23)

The core maps raw `AVERROR` codes to classes with human wording, and preserves the raw code for the "technical details" disclosure. Classification lives in the core so both platforms produce identical text and identical codes (AD-8):

> *"This file does not appear to be a video or audio file. Its contents could not be recognised."*
> *"This file contains no video track. If you only need the audio, try importing it as audio."*
> *"This file appears to be incomplete, as if a copy or download did not finish."*
> *"This video could not be decoded. The file may be corrupted or use a codec that is not supported on this system."*

### The honest reading of this result

**FFmpeg 8.1.2 did not crash once**, on any of 1024 malformed files. That is a credit to FFmpeg, and it means S6 proves the isolation *works* — not that FFmpeg is dangerous.

So the case for the worker process is **insurance, not remediation**: it costs little, and it converts an entire class of unknown future failure (a codec bug, a hostile file we did not think of, a hang) into a recoverable job error. AD-3 stands, but it should be argued on those grounds rather than on observed crashes. §22's guarantee is now structurally supported by evidence.

### ⚠️ A false finding I nearly reported

The first run of this spike reported **66 hangs (6.4%) and took 240 seconds**. I was about to record that as a significant architectural finding about concurrent hardware decode.

It was a bug in my own test harness. Re-running the identical corpus and worker through a Python harness completed in **11.6 s with zero hangs** — a 20× discrepancy that could not be explained by the media stack. The cause was GCD thread-pool starvation in the Swift host: a concurrent `DispatchQueue` whose blocks blocked on a semaphore, *plus* a `DispatchQueue.global()` block per child doing a blocking pipe read. Blocked GCD threads are only replaced after a delay, so children sat unreaped long enough to trip the 20 s timeout. Rewriting the harness on dedicated `Thread`s gave **18.2 s and 0 timeouts**, confirming the diagnosis.

Two things worth taking from this:

1. **A measurement that disagrees with a second implementation is a bug until proven otherwise.** The finding would have been wrong, and it would have driven real architectural decisions about the proxy and conversion-queue design.
2. **There is a genuine product lesson inside the harness bug:** the process supervisor behind the conversion queue (§21) will manage many concurrent workers, and it **must not block GCD threads waiting on child processes**. Use dedicated threads or proper async I/O. This is recorded because it is exactly the mistake the real implementation would make.

### What S6 does not prove

- **No crash was ever induced**, so the recovery path (worker died → classify → offer retry) is exercised only by timeout-kill, not by a real signal. Worth adding a deliberately crashing worker to test the signal path.
- Corpus is synthetic-derived. Real-world damaged files (partial downloads of real footage, camera-card corruption) have different failure shapes.
- No test of a worker that leaks or grows unboundedly rather than crashing.
- The 4 K path was not fuzzed; the corpus is 640×480 for speed.

---

## S7 — Async waveform generation ✅ PASS

**Artifact:** `spikes/s7-waveform/`, plus `mc_waveform` in the Rust core.

Spec §17 requires waveforms generated asynchronously, never blocking the UI, with a cache reused between launches.

**Design:** peaks are **streamed through a callback**, not returned as one buffer. A 60-minute file must show a partial waveform immediately, not nothing for several seconds. The callback also carries progress and can cancel by returning 0.

### Generation — 60 minutes of audio, 100 buckets/sec

| Format | Wall time | vs realtime | First batch | Budget |
|---|---|---|---|---|
| WAV (uncompressed) | **0.35 s** | 10,315× | 30 ms | 10 s |
| MP3 192k | **2.26 s** | 1,592× | 10 ms | 10 s |
| FLAC | **3.18 s** | 1,131× | 10 ms | 10 s |
| AAC in MP4 (video container) | 0.01 s / 3 s clip | 239× | 12 ms | — |

Uncompressed WAV is the easy case, so compressed formats were tested too — those carry real decode cost and are what users actually import. **The slowest realistic case is ~3× under a 10-second budget for a full hour of audio.**

The **first batch arrives in 10–30 ms**, so a progressive waveform can start drawing essentially immediately regardless of file length.

### UI blocking — measured, not assumed

The main thread ran a 120 Hz heartbeat throughout and recorded its own jitter. A stalled main thread shows up as a late tick, exactly as a dropped frame would.

| Source | Ticks | p50 | p99 | max | Ticks > 16.67 ms |
|---|---|---|---|---|---|
| MP3 | 227 | 10.42 | 10.43 | 10.43 | **0** |
| FLAC | 316 | 10.42 | 10.43 | 10.43 | **0** |

The distribution is completely flat — p50, p95, p99 and max agree to 0.01 ms. That flatness is the result: a real stall would appear as a tail spike. (The 10.42 ms baseline against an 8.33 ms target is `usleep` granularity in the harness, not blocking.)

### Cache

| Metric | Value |
|---|---|
| Size, 60 min @ 100 buckets/s | 2.7 MB |
| Write | 1 ms |
| Read back | <1 ms (≈820× faster than regenerating) |

2.7 MB per hour of audio is small enough that cache size is not a design constraint at the timeline resolutions this product needs.

### What S7 does not prove

- **Content is synthetic** (tremolo'd sine). MP3/FLAC decode cost is largely content-independent, so this is a fair proxy — but real music has not been measured.
- **The cache format is a raw float dump**, with no versioning, checksum, or content-hash key. AD-10 requires all three; the spike deliberately skips them since it is measuring throughput, not designing the cache.
- **No cancellation test.** The callback supports it (return 0) but the path is not exercised.
- **Single file at a time.** Importing 20 files at once is the realistic load and was not tested.

---

## S8 — App Sandbox + security-scoped bookmarks ✅ PASS

**Artifact:** `spikes/s8-sandbox/` — a real `.app` bundle, ad-hoc signed with the App Sandbox and hardened runtime (`flags=0x10002(adhoc,runtime)`), plus a helper executable standing in for the media worker.

AD-9 puts security-scoped bookmarks in the project schema from v1 specifically so the format never needs migrating. S8 tests whether that is actually required and whether it works.

### The sandbox genuinely denies paths — bookmarks are load-bearing

Negative control, run first: the sandboxed app given an arbitrary path on the command line **fails to open it** ("no user-granted access to this path"). Access requires a user-intent grant — `NSOpenPanel`, drag-and-drop, or LaunchServices. **AD-9's decision to store bookmarks from schema v1 is confirmed necessary, not precautionary.**

### Bookmarks survive relaunch

| Step | Result |
|---|---|
| Bookmark created after user-intent grant | **864 bytes** |
| Resolved after simulated relaunch | ✅ correct path, `stale: false` |
| `startAccessingSecurityScopedResource()` | ✅ true |
| Parent process read | ✅ OK |

864 bytes per source reference is small enough that bookmarks add no meaningful weight to the project file.

### Bookmarks survive rename *and* move — a better answer to §42

The file was renamed **and** moved to a different directory, then resolved again:

```
resolved: .../media/moved/renamed_audio.mp3     <- new path, found automatically
stale:    true                                   <- signals "re-create and re-save me"
read:     OK
```

A path-based project format would have lost the file. **Bookmarks make §42's "missing media" case substantially rarer**, because they track the file rather than its name. The hash-based relink in AD-9 is still needed for cases bookmarks cannot cover — cross-volume moves, restores from backup, files replaced by a different copy — but it becomes the fallback rather than the primary mechanism.

**Implementation note:** when `stale` comes back true, the bookmark must be re-created and re-saved into the project. Skipping that works until it silently stops working.

### ⚠️ The finding that matters most for AD-3: the worker needs `com.apple.security.inherit`

First attempt: the helper was signed with `com.apple.security.app-sandbox` like the parent. **The kernel killed it at exec with SIGTRAP (exit 133), producing no output and no error message.** The harness initially reported this as "child has no access" — a wrong conclusion, since the child never ran at all.

Cause: a bare helper executable carrying `app-sandbox` without a container of its own is not a valid sandboxed process. The documented pattern for a child of a sandboxed app is:

```xml
<key>com.apple.security.app-sandbox</key><true/>
<key>com.apple.security.inherit</key><true/>
```

With that pair, everything works:

| Test | Result |
|---|---|
| Worker opens the bookmarked file **by path** | ✅ `read 16 bytes by path` |
| Worker reads an **inherited file descriptor** | ✅ `read 16 bytes from inherited descriptor` |

**Two consequences for AD-3:**

1. **The worker API can take a path.** Sandbox access is inherited, so `mc_open(const char*)` does not need redesigning around file descriptors. That was the open risk going in, and it is closed.
2. **Descriptor passing works as a fallback** if a future case ever needs it.

This failure mode is worth recording loudly: **a mis-entitled helper dies silently with no diagnostic**, and the obvious interpretation of the symptom is the wrong one. It would have cost real time in Phase 3.

### What S8 does not prove

- **Ad-hoc signed, not Developer ID, and not notarized.** The entitlements, sandbox and hardened runtime are real; the distribution path is not tested. Notarization requires a paid account (`RISK_REGISTER.md` R-13 covers the Windows equivalent; the macOS one should be started before Phase 3).
- The grant came via LaunchServices rather than `NSOpenPanel`. Both are user-intent mechanisms and produce the same class of access, but the shipping app will use the panel and drag-and-drop.
- No test of **many** bookmarks (a 20-clip project), nor of bookmark resolution cost at scale.
- No test of a **removable volume** disconnecting — §42 requires that case and it behaves differently.

---

## S4 — Encoder benchmark 🟡 harness ready, blocked on corpus

**Artifact:** `tools/encode_benchmark.sh`

Compares `libx264 -preset medium` against `h264_videotoolbox` and `hevc_videotoolbox` across a bitrate ladder, scoring each with VMAF, and emits CSV.

The script deliberately runs on the **Homebrew GPL ffmpeg** — that is correct and permitted: it is measurement tooling whose output is never distributed, and `libx264` is the reference we measure *against*, never something we ship.

### Corpus — acquired (`tools/fetch_corpus.sh`, `tools/prep_corpus.sh`)

VMAF on synthetic sources is meaningless: synthetic content flatters or punishes encoders arbitrarily, and preset bitrates derived from it would be fiction. A real corpus was therefore assembled from open-licensed sources:

| Clip | Content | Resolution | Source |
|---|---|---|---|
| `sintel_4k_a` | CGI, 120 frames | 4096×1744 | Sintel, CC BY 3.0 |
| `sintel_4k_b` | CGI, different scene | 4096×1744 | Sintel, CC BY 3.0 |
| `tos_1080_a` | Live action, graded, faces | 1920×800 | Tears of Steel, CC BY 3.0 |
| `tos_1080_b` | Live action, different scene | 1920×800 | Tears of Steel, CC BY 3.0 |
| `screencast_1080` | Sharp text, flat areas, scroll | 1920×1080 | Synthesised |

**Two deliberate choices:**

1. **Sources are lossless frame sequences, not compressed video.** xiph.org mirrors these films as PNG frames. A compressed reference would bake generation loss into every VMAF number. RGB→yuv420p conversion happens **once**, in the reference, so every encoder is scored against identical source pixels.
2. **The screencast is synthesised, not captured.** Screen recordings are a distinct encoding regime — high spatial frequency, large flat areas, low temporal change — and no open film corpus contains one. It is rendered with CoreGraphics rather than captured, because capturing would record the user's actual screen.

Licensing is recorded in `DEPENDENCY_AND_LICENSE_AUDIT.md` §5 even though the corpus is measurement tooling that is never distributed.

**The number that matters** is not absolute VMAF but the **bitrate ratio**: how much more bitrate VideoToolbox needs to match libx264 at equal quality. That ratio sets every export preset and quantifies R-09.

### Results — 60 encodes, 5 clips, 4-point bitrate ladder

**Headline bitrate penalty vs `libx264 -preset medium`:**

| Encoder | Median | Range |
|---|---|---|
| `h264_videotoolbox` | **1.38×** | 1.00–2.87× |
| `hevc_videotoolbox` | **1.21×** | 0.87–2.91× |

**VMAF by bitrate (selected):**

| sintel_4k_a (4K CGI) | 15 Mbps | 25 | 35 | 50 |
|---|---|---|---|---|
| libx264 medium | 92.22 | 94.68 | 95.75 | **96.61** |
| h264_videotoolbox | 82.21 | 88.79 | 91.95 | **94.57** |
| hevc_videotoolbox | 86.89 | 92.52 | 95.15 | **96.57** |

| tos_1080_a (live action, graded) | 4 Mbps | 6 | 8 | 12 |
|---|---|---|---|---|
| libx264 medium | 93.49 | 95.36 | 96.25 | **97.10** |
| h264_videotoolbox | 87.27 | 91.08 | 93.26 | **95.42** |
| hevc_videotoolbox | 90.63 | 93.34 | 95.18 | **96.74** |

### The finding that matters most: hardware encoders hit a quality CEILING

The bitrate ratio understates the problem. Within the tested ladder, **the hardware encoders cannot reach software quality at any bitrate**:

- `sintel_4k_a`: `h264_videotoolbox` plateaus at VMAF **94.57 even at 50 Mbps**; libx264 reaches 96.61. More bitrate does not close it.
- `tos_1080_a`: `h264_videotoolbox` plateaus at **95.42 at 12 Mbps**; libx264 reaches 97.10.
- `screencast_1080`: **both** hardware encoders plateau at **99.10**; libx264 reaches 99.30.

This is qualitatively different from "needs more bitrate". A ceiling means a **"Best Quality" preset built on hardware encoding cannot match a software encoder, at any file size.** That is a direct constraint on AD-12.

### Content dependence is enormous — the average hides the risk

| Clip | h264_vt penalty | Note |
|---|---|---|
| `sintel_4k_b` | **1.00×** | No penalty at all |
| `tos_1080_b` | 1.30–1.38× | Moderate |
| `tos_1080_a` | 1.98–2.11× | Severe |
| `sintel_4k_a` | 2.44× | Severe |
| `screencast_1080` | **2.87×** | Worst case, plus a hard ceiling |

**Screen recordings are the worst case for hardware encoding** — high spatial frequency and flat areas are exactly what fixed-function encoders handle least well. That matters directly: screen recordings are a primary creator input, and `PRODUCT_DIRECTION.md`'s workflow W1 assumes them.

### HEVC is clearly the better hardware path

`hevc_videotoolbox` beats `h264_videotoolbox` on every clip, and on `tos_1080_b` it **beats libx264 H.264 outright** (0.87–1.00×). This confirms the Phase 0 read that Apple's hardware HEVC encoder is strong. **HEVC should be the default wherever target compatibility permits**, with H.264 offered as the compatibility fallback.

### ⚠️ One caveat resolved (S4b below), one still standing

1. **Rate control was handicapping the comparison — but not in the direction expected.** Re-tested in S4b. **The ceiling was indeed an ABR artefact**, but the efficiency gap turned out to be *worse*, not better, once each encoder ran in its own native quality mode. See S4b.
2. **The speed comparison here is inconclusive**, and should not be quoted. Mean encode times (libx264 1.88 s, h264_vt 1.44 s, hevc_vt 1.58 s) are dominated by process startup on 5-second clips. Hardware encoding's speed advantage is real and well documented but is **not** demonstrated by this data; measuring it needs long clips and in-process timing.

---

## S4b — Constant-quality re-test ✅ COMPLETE (corrects S4)

**Artifact:** `tools/encode_benchmark_cq.sh`

S4 ran every encoder in single-pass ABR. That is the mode a naive "export at 8 Mbps" preset uses, but it is not VideoToolbox's native mode, and I flagged that the observed ceiling might be an artefact. S4b re-runs each encoder in **its own quality mode** — `libx264 -crf`, VideoToolbox `-q:v` — and compares the resulting rate-quality curves at **measured** (not requested) bitrate.

### Finding 1 — the ceiling WAS an ABR artefact

`h264_videotoolbox` reaches **VMAF 99.20** on 4K CGI in constant-quality mode, against the 94.57 plateau seen under ABR. There is no hard quality ceiling. **S4's ceiling claim is withdrawn.**

### Finding 2 — but the efficiency gap is WORSE, not better

| Encoder | ABR (S4) | **Constant quality (S4b)** |
|---|---|---|
| `h264_videotoolbox` | 1.38× median | **2.31×** (range 1.75–5.40×) |
| `hevc_videotoolbox` | 1.21× median | **1.74×** (range 1.39–4.48×) |

The reason: **libx264 gains a great deal from CRF, its native mode; VideoToolbox's `-q:v` gives no comparable lift.** Comparing each encoder at its best therefore *widens* the gap. The ABR numbers flattered the hardware encoders.

This is the more honest comparison, and 2.31×/1.74× are the numbers that should drive preset design.

### Finding 3 — screen recordings are pathological

| Content | h264_vt | hevc_vt |
|---|---|---|
| 4K CGI (`sintel_4k_a`) | 1.80–2.51× | 1.44–1.80× |
| 1080p live action (`tos_1080_a`) | 1.75–2.21× | 1.39–1.68× |
| **Screencast** | **4.85–5.40×** | **4.12–4.48×** |

libx264 encodes the screencast at 174–361 kbps for VMAF 94–99. VideoToolbox needs **720–2669 kbps** for the same quality. Hardware encoders are built for camera-like content; large flat areas with sharp text are their worst case.

This matters because screen recordings are a primary creator input and workflow W1 in `PRODUCT_DIRECTION.md` assumes them.

⚠️ **Caveat on this specific number:** VMAF is trained on natural video and is less reliable on synthetic screen content, so 4–5× may overstate the true perceptual gap. The direction is well established; the magnitude should be re-checked against a real screen recording before it drives a product decision.

### Consequences for AD-12 (policy already chosen)

The chosen policy — **default to HEVC, label "Best Quality" honestly** — is *reinforced*, not undermined:

- **HEVC's advantage over hardware H.264 is larger than S4 suggested** (1.74× vs 2.31×). Defaulting to HEVC is the single most effective mitigation available.
- **Preset multipliers must be revised upward**: roughly **2.3× (H.264)** and **1.75× (HEVC)** over software-encoder intuition, not the 1.4×/1.2× from S4.
- **Honest labelling matters more, not less.** At matched quality our files will be meaningfully larger than a software encoder's, and the "Smaller File" presets must not overpromise.
- **Screen-recording content may warrant a specific answer** — a higher-bitrate preset, or a warning — rather than being silently encoded at 5× the necessary bitrate.

The **patent posture is unaffected and remains the primary justification** for hardware-only encode.

### Consequences

1. **AD-12 needs revisiting** (see `ARCHITECTURE_DECISION.md`). Hardware-only encode was chosen for speed *and* patent posture. The patent argument is unchanged; the quality argument is now measurably weaker.
2. **Preset bitrates must carry roughly a 1.4× multiplier** over software-encoder intuition for H.264, ~1.2× for HEVC — and be set per content class if possible.
3. **R-09 is confirmed, not theoretical.** The Phase 0 trigger was "a gap larger than ~15% bitrate at matched quality". Measured median is 38% for H.264. **The trigger has fired.**

---

## Running notes

- The Homebrew FFmpeg on the dev machine is `--enable-gpl --enable-version3 --enable-libx264 --enable-libx265`. It stays useful as measurement tooling and as a way to produce test media. It must never reach the product. The gate exists precisely because that distinction depends on a human remembering it.


---

# Phase 1 — Closing report

Required by §46 Rule 12. **Verdict: Phase 1 is complete for the macOS target. Phase 2 is unblocked.**

## What was proven

| Claim | Evidence |
|---|---|
| An LGPL-only FFmpeg meets every format requirement in §2/§3/§4 | S3 — decode, demux and hardware encode all present; MP3 closed by adding LAME (LGPL) |
| A GPL build cannot be shipped by accident | S3 — CI gate rejects the Homebrew build, passes ours, inspects the built artifact |
| Frames reach the GPU with zero CPU copies | S1 — 360/360 IOSurface-backed and Metal-bound, 0 software frames, 4,271 MB never copied |
| 4K30 is comfortable on Apple Silicon | S1 — 183 fps steady state, ~6× the required headroom |
| The timeline can be built in SwiftUI | S2 — 3.29 ms p99 worst case; S2b — 0.74 ms mutation→pixels |
| §22's "never crash on invalid media" is achievable | S6 — 1027 malformed files, 0 host crashes, 0 hangs, all classified |
| §17's async waveform requirement is achievable | S7 — 60 min of FLAC in 3.18 s, 0 UI stalls, 2.7 MB cache |
| Non-destructive editing survives the sandbox | S8 — bookmarks resolve after relaunch and across a move; worker inherits access |
| HDR input can be detected (O-3) | Colour metadata verified against purpose-built PQ and HLG files |

## Assumptions this phase overturned

Phase 1 exists to catch reasoning that does not survive measurement. It caught three of mine:

1. **The timeline did not need AppKit.** `PHASE_0_ARCHITECTURE_RESEARCH.md` §R1 called it "the problem case" based on reports of SwiftUI list stutter. Those reports concern per-row view allocation; `Canvas` is immediate-mode and allocates nothing per clip. **AD-2 simplified: one renderer, not two.**
2. **S4's "quality ceiling" was an ABR artefact** — and correcting it made the picture *worse*, not better. In each encoder's native quality mode the gap widens to 2.31×/1.74×. The first measurement flattered the hardware.
3. **Tier 2 proxies look unnecessary on macOS.** AD-5 assumed they would be needed; S1's headroom says otherwise for the platform now in V1.

## Findings that create new work

| Finding | Consequence |
|---|---|
| **First frame costs 1054 ms** (S1) | Consumes the entire §R20 "open a file" budget. Must pre-warm the VideoToolbox session and pull first thumbnails off the playback path. |
| **337 MB per decode stream** (S1) | Only ~5 concurrent decoders inside the 1.5 GB project budget. Constrains multi-track. → **R-23** |
| **Worker needs `com.apple.security.inherit`** (S8) | Without it the helper dies at exec with SIGTRAP and no diagnostic. Recorded in AD-3. |
| **Screen recordings cost 4–5× bitrate** (S4b) | Pathological for hardware encoders, and a primary creator input. May need its own preset. |
| **Header drifted from the Rust struct** (O-3 work) | The C ABI header must be **generated with cbindgen**, not hand-maintained. |

## Risk movement

**Retired:** R-01 (two platforms in V1 — settled by O-1) · R-04 (FFI/zero-copy — proven in S1) · R-10 (sandbox retrofit — validated in S8)
**Mitigated:** R-05 (GPL build — CI gate) · R-15 (hostile media — downgraded to Low)
**Escalated:** R-09 (hardware encode quality — **trigger fired**, now High/High)
**Opened:** R-21 (Windows port decay) · R-22 (model size) · R-23 (per-stream memory)

## Decisions settled during Phase 1

AD-2 (SwiftUI Canvas timeline, AppKit only for the Metal surface) · AD-3 (worker entitlements) · AD-12 (HEVC default, honest labelling, S4b multipliers) · O-1, O-3, O-4, O-16.

## Two harness bugs that nearly became false findings

Recorded because the pattern matters more than the instances:

- **S6** reported 66 hangs and 240 s. It was GCD thread-pool starvation in my harness; a second implementation ran the same corpus in 11.6 s with zero hangs.
- **S8** reported "child has no access, redesign the API around file descriptors". The child had never run — it was killed at exec by a signing misconfiguration.

Both would have driven real architectural change. **A measurement that disagrees with a second implementation is a bug until proven otherwise**, and a failure with no output is a launch failure, not a behavioural result.

## Not done, and why

| Spike | Status |
|---|---|
| S5 (WinUI 3 + D3D) | Deferred to V1.1 with the platform (O-1). No Windows host available. |
| S9 (multi-GPU probe) | macOS portion implied by S1; multi-GPU needs hardware. |
| S10 (proxy necessity) | macOS half answered by S1. Windows half moves to V1.1. |

**R-21 still applies:** the Rust core must keep compiling for Windows in CI throughout V1, or the V1.1 port becomes a rewrite.

## Pre-Phase-2 work completed (2026-09-09)

Three prerequisites the closing report named as "carry forward" now exist rather than being intentions.

### 1. The C ABI header is generated, not hand-written

`cbindgen` runs from `build.rs` on every build, emitting `include/mediacore.h` from the Rust source. Verified by adding colour and timing fields to the Rust structs and watching the header regenerate; all three Phase 1 harnesses (S1, S6, S7) rebuild and pass against the generated header unchanged.

This closes the drift that already bit once. The mild symptom was a compile error; the severe version of the same failure — a struct **layout** mismatch — would be silent memory corruption across the FFI.

### 2. VFR is detected — and the detection proved it must not be relied on

VFR is absent from the master spec and silently desyncs audio on phone and screen-recorded footage. Detection was added and tested against purpose-built files:

| File | avg_fps | r_fps | Distinct durations | Verdict |
|---|---|---|---|---|
| `cfr_30.mp4` | 30/1 | 30/1 | 1 | CFR |
| `vfr_dropped.mp4` | 1530/59 | 30/1 | 2 | **VFR** |
| `vfr_mixed.mkv` (30fps + 10fps concat) | 30/1 | 30/1 | 2 | **VFR** |
| Tears of Steel, 4K test clip | 24/1 | 24/1 | 1 | CFR |

**Two findings that shape the design:**

- **The metadata heuristic is not sufficient.** Comparing `avg_frame_rate` with `r_frame_rate` catches dropped-frame VFR but **misses the mixed-rate file entirely** — its header claims 30/1 for both. Only observing actual packet timestamps catches it.
- **Detection depends on probe depth.** At 5 frames, *both* VFR files read as CFR. At 150 frames, both are caught. A shallow probe gives a confident wrong answer.

**Therefore correctness must not depend on detection at all.** The timeline is PTS-driven *unconditionally*; VFR detection exists only to warn the user and to choose an export strategy. A design that switches between "CFR fast path" and "VFR correct path" based on this flag would be wrong roughly whenever it mattered.

### 3. The core is split, and R-21 is partly enforceable locally

| Crate | Contents | Windows cross-check from macOS |
|---|---|---|
| `mediacore-model` | project, timeline, rational time, commands, cache policy | ✅ **passes** |
| `mediacore-media` | FFmpeg FFI | ❌ impossible — bindgen needs Windows system headers |

Attempted and confirmed: `cargo check --target x86_64-pc-windows-msvc` on the FFmpeg-bound crate fails on missing `errno.h`. **R-21 therefore requires a real Windows CI runner** for the media crate — that cannot be faked from a Mac. But the model crate, where most of Phase 2's work happens, is portability-checked on every build.

The timebase is implemented and unit-tested: `TICKS_PER_SECOND = 705_600_000`, chosen so 24, 25, 30, 48, 50, 60 and the 1001/24000-family rates all divide exactly.

**A wrong test worth recording:** the first version asserted that 107,892 frames at 30000/1001 is exactly one hour. It is not — it is 3599.9964 s, and that 3.6 ms belief is precisely the confusion drop-frame timecode exists to hide. The test was wrong, not the code. Had it been written the other way round, a false assumption would have been baked into the timeline and "proved" by a passing test.

## Entry conditions for Phase 2

All four blocking questions are answered (O-1, O-2 pending sign-off, O-3, O-4). Phase 2 builds the project model, timeline, commands, undo/redo and autosave, and it should carry forward:

- Rational timebase, PTS-driven (§R11) — VFR is still unhandled and still absent from the spec
- Security-scoped bookmarks in schema v1 (AD-9, confirmed necessary by S8)
- Colour fields in the asset model from the start (O-3)
- Content-hash cache keys (AD-10)
- A generated C ABI header (cbindgen), not a hand-written one
