# Phase 3 — macOS application

**Status:** ✅ **macOS V1 feature set complete** — 2026-09-10
**Outstanding:** audio *editing* (V1.1 per PRODUCT_DIRECTION §6), the converter/tools modes (Phase 6), and Developer ID signing (needs a paid account)
**Scope:** §37 — native UI, smooth timeline, Metal preview, VideoToolbox, native menus and shortcuts, save/recovery, 4K playback.
**Code:** `app/macos/` (Swift) + `core/mediacore-media/src/session.rs` (the C ABI the app drives).

---

## Running

```
app/macos/build_app.sh          # builds Editor.app
open app/macos/Editor.app
./Editor.app/Contents/MacOS/Editor file1.mp4 file2.mov   # import at launch
```

Launch-argument import exists so the app is testable end-to-end without driving a file panel — useful for CI and for §42's failure matrix.

## What works

- **§30's layout**: media library, Metal preview, inspector, timeline with transport bar.
- **Import** via panel or launch arguments, with probing through the LGPL FFmpeg build.
- **Metal preview** rendering real decoded frames through the S1 zero-copy path.
- **Timeline** drawing clips, ruler, playhead; click to select, drag to move, drag the ruler to scrub.
- **Native menus and §29 shortcuts**: ⌘O, ⌘I, ⌘S, ⌘⇧S, ⌘Z, ⌘⇧Z, `S` split, Delete, ⇧Delete ripple, ⌘= / ⌘−, Home/End, ←/→ frame stepping (⇧ for 10).
- **Undo/redo** driving the Phase 2 command stack.
- **Import notices** for HDR and VFR, in plain language.

## The FFI surface (`session.rs`)

`mediacore-model` is pure Rust with no FFI, deliberately (R-21). `session.rs` is the single place it is exposed. Two rules shape it:

1. **Bulk reads, not per-item calls.** The timeline reads every frame; S2 measured the draw itself at 0.74–3.3 ms, so per-clip FFI calls would dominate. `mcs_clips` and `mcs_plan_at` each fill a caller-owned array in **one** call, into a buffer the Swift side reuses.
2. **Pixels never cross.** Frames stay platform buffer handles (S1). This surface carries ids, times and small PODs only.

The Swift `Core` type holds **no copy of the timeline** — the Rust core owns document truth (AD-1). Two mutable copies either side of the FFI is precisely the state-divergence bug the architecture avoids.

## Findings

### Library Validation rejected the bundle — AD-11 confirmed, with a wrinkle

The app died at launch: *"code signature not valid for use in process: mapping process and mapped file (non-platform) have different Team IDs"*.

AD-11's reasoning holds exactly — Library Validation requires every loaded dylib to share the host's **Team ID**, and because we build the FFmpeg dylibs ourselves, a Developer ID signature on all of them satisfies it and `disable-library-validation` is never needed.

The wrinkle is that **ad-hoc signatures carry no Team ID at all**, so under the hardened runtime they can never match. Local development therefore signs ad-hoc *without* the hardened runtime; release sets `DEVELOPER_ID` and turns it back on. Both paths are in `build_app.sh`.

### `mc_seek` was missing, and scrubbing needs it

The preview seeks per frame. Decoding from the start each time is not viable, so `mc_seek` was added — and it **flushes the decoder**, because a decoder holds reference frames from before the seek and would otherwise emit corrupt frames from the old position.

Seeks land on the preceding **keyframe**, so the caller decodes forward to the exact frame. That is inherent to inter-frame codecs, not a shortcoming of the seek.

### Decoders are pooled, bounded by the S1 memory measurement

Opening a decoder costs ~**1054 ms** (S1: decoder init + VideoToolbox session + Metal warm-up), so re-opening per scrub would make the preview unusable. They are cached — and the pool is capped at 4, because S1 measured ~**337 MB resident per open 4K stream** (R-23).

### A stale-header bug in my own build script

`build.rs` watched only `src/lib.rs`, so adding a function in `session.rs` left the generated header stale. The symptom — *"cannot find mcs_asset_path in scope"* — points at the Swift code rather than the build cache. Now watches all of `src/`.

## Added since

- **Playback** (§16). Space toggles play/pause. The playhead advances by
  **wall-clock delta**, not a fixed increment per tick, so a dropped frame costs
  a dropped frame rather than a drift in time — which is what keeps long
  playback in sync with the audio clock. Scrubbing pauses, as any editor does.
- **Colour** (O-3). Full NV12 → RGB using the **Rec.709 limited-range** matrix,
  which is the V1 pipeline O-3 fixes. Letterboxed so the frame keeps its aspect.
- **Audio is first-class** (§3). Audio-only files route to an **audio track**
  rather than stacking on video, and draw their **waveform** (§15), generated
  off the UI thread and cached per asset (§17).
- **Drag and drop from Finder** onto the timeline (§22).
- **Autosave and crash recovery** (§24). Autosave arms as soon as the project
  has a home on disk. Opening a project with a surviving journal offers
  *"Recover unsaved changes?"* — recover, or open the last saved version.

## Two rendering bugs worth recording

**1. The full-screen-triangle trick is incompatible with letterboxing.**
The preview showed a diagonal tear across the frame. The diagonal *was the
triangle's hypotenuse*: that trick relies on an oversized triangle covering the
viewport with UVs derived from clip position, and scaling its vertices to
letterbox breaks both halves — it no longer covers, and the UVs no longer
correspond. Replaced with a quad and explicit UVs.

*(A texture-lifetime fix was made first, on the assumption the tear was a
recycled IOSurface. It was not the cause — but it was a real bug regardless:
a `CVMetalTexture` released before the command buffer completes lets its
surface be reused while the GPU is still sampling. Both fixes stand.)*

**2. Audio-only files could not be imported at all.**
`mc_probe` looks for a *video* stream, so an MP3 or WAV returned
`NO_VIDEO_STREAM` and import refused it — while §3 makes audio a first-class
import. `NO_VIDEO_STREAM` is now treated as "this is an audio file", not as a
failure.

## Compositor: multi-layer, effects, tone mapping

**Multi-layer compositing.** Every video layer in the render plan is drawn,
bottom-most first, with alpha blending — so an overlay track composites over
the base rather than replacing it. The core supplies the order (AD-4).

**§18 effects** — brightness, contrast, saturation, temperature, tint,
grayscale — live on the clip, travel through the render plan, and are applied
in the shader. Order matters and is deliberate: brightness, then contrast about
mid-grey, then temperature, then saturation. Applying saturation before contrast
makes the two fight each other.

Every effect change is an ordinary **undoable edit** (`Edit::SetEffects`) — §28
covers effects too. Effects carry no geometry, so unlike `SetSpeed` they need no
overlap check.

The list stops at §18's set, deliberately: *"do not build a giant effects
marketplace."* Writing each effect twice (MSL and HLSL) from one specification
is only affordable because the list is short — AD-4 says revisit that if it ever
passes ~20.

**HDR tone mapping (O-3).** PQ (SMPTE 2084) and HLG transfers are undone to
linear, tone-mapped, and returned to display range. Curve: **Hable filmic** —
provisional, since O-3 leaves the exact curve to a visual comparison, but it
rolls off highlights gracefully rather than clipping them, which is the failure
that actually looks bad.

10-bit HDR decodes to **16-bit** planes, so the texture formats switch to
`r16Unorm`/`rg16Unorm`. Reading those as `r8Unorm` would read the wrong bytes
entirely.

⚠️ **What the HDR test does and does not prove.** PQ and HLG files decode,
tone-map, render, and raise their notice — the *path* is verified. The *curve*
is not: the fixtures are synthetic patterns carrying HDR metadata, not real HDR
footage, and their values land in a range that makes the result look darker
than real content would. Judging the curve needs genuine HDR camera footage.

## Export (§12) — the workflow now closes

**§48's first success workflow is achievable end to end**: import → edit →
title → preview → **export**. An exported file was verified to contain the
composited result — source footage, title overlay, effects, correct
letterboxing — not just a valid container.

**Presets** are named by resolution and intent (§5, §12), and their bitrates
come from **S4b's measurements**, not intuition: hardware HEVC needs ~1.74×
libx264's bitrate for equal quality, hardware H.264 ~2.31×. Quoting
software-encoder numbers here would ship visibly worse files under
confident-sounding names. The H.264 preset is labelled "compatibility" and
carries the higher bitrate honestly.

**Preview and export share ONE compositor.** R-18 is the risk that exported
output diverges from what the preview showed; the surest guard is that there is
only one implementation to diverge from.

### Five findings, each from a test rather than an assumption

**1. The hardware encoder only accepts frames from its OWN pool.**
Pushing a `CVPixelBuffer` from a caller-owned pool is rejected outright. The API
had to become `acquire` → render → `submit`: the encoder hands out a surface,
the compositor renders into it. That is *more* zero-copy than the original
design, not less — but it was found by writing a test, not by reasoning.

**2. The muxer rewrites the stream timebase.** `avformat_write_header` replaced
the timebase set beforehand, so rescaling packets to the old value produced a
1-second clip reporting **0.0019 s**. The stream timebase must be read back
*after* the header is written.

**3. FFmpeg decoders are not safe for concurrent use.** Sharing the preview's
decoder pool with the export thread tripped an internal assertion
(`fctx->async_lock failed`) and produced a 44-byte file. Export now uses its own
pool, and `FrameSource` is lock-guarded.

**4. Seeking before every frame makes export pathological.** A seek lands on the
preceding keyframe and the decoder must run forward from there, so seeking
per-frame pays that cost repeatedly — export was visibly crawling. Sequential
requests now decode forward instead, which is the normal case for both playback
and export.

**5. BGRA hardware surfaces are Metal-compatible**, so the compositor renders
straight into the encoder's buffer in a single pass — no NV12 conversion pass,
no readback.

## Overlays, effects and audio — completing the V1 feature set

**§19 text overlays** with presets that are *finished looks* rather than fonts.
§19's own list (font/size/weight/alignment/position/opacity) produces the exact
look that says "made in a free editor", so tracking, line height, outline,
shadow and a padded background are included — those are what actually separate
good type from bad (PRODUCT_DIRECTION §7).

**§20 image overlays** with PNG transparency verified: an opaque ring, a
semi-transparent core showing the video through it, and no black box around it.

**§18 effects** complete, including blur and sharpen. Those need neighbouring
samples, unlike every other effect, so they use a 3×3 kernel — enough for the
"basic blur / basic sharpen" §18 describes and cheap at 4K. Kernel taps use the
SOURCE texel size, so they land on real neighbouring pixels regardless of how
the frame is scaled on screen.

**Export follows the PROJECT's frame rate**, not a hard-coded 30. Exporting a
24 fps project at 30 changes every clip's duration.

**Audio** (§3, §4): decode + resample to the project rate, mix with per-clip
gain and fades, and encode into the export as AAC.

## Four more findings

**6. One overlay track cannot hold a title AND a logo.** Clips on a track may
not overlap, so adding both put the second one nowhere — silently, because the
add was correctly refused. V1 now has **two overlay tracks**, which is the
smallest fix that makes §19 and §20 usable together.

**7. FFmpeg is the wrong tool for still images.** Its image demuxers report
"unspecified size" for a perfectly valid PNG and refuse to open it. Overlays now
decode through **ImageIO** — the platform's own decoder, which handles PNG
alpha, JPEG, WebP, TIFF and HEIC correctly. FFmpeg remains the video path.
(Windows has the same option in WIC.)

**8. Letterboxing distorts an overlay if applied per-axis.** Scaling an image's
height by `scaleY` and width by `scaleX` independently turned a circular logo
into an ellipse. Width must be derived from the already-scaled height.

**9. `aac_at` accepts s16 only — and a failed audio codec broke the whole file.**
Two bugs in one: hard-coding FLTP made AudioToolbox's AAC fail to open, and
because the stream had already been created, the muxer then refused to write
*anything* ("codec none in stream #1") — a failed audio codec took the video
down with it. The encoder now **asks the codec which formats it accepts**
(FFmpeg 8 replaced `codec->sample_fmts` with `avcodec_get_supported_config`,
noted back in Phase 1) and **opens the codec before creating the stream**, so
failing audio degrades to a silent video rather than no video.

## Still outstanding

- **Developer ID signing and notarization** — needs a paid Apple account, so
  this one is not something the build can resolve on its own.
**Audio playback** is in. A source node pulls mixed samples on the audio render
thread, which has a hard deadline and must never block — so it does no decoding
at all, only a copy from a ring buffer a background thread keeps filled.
Decoding on the render thread is the classic cause of audio glitching.

The preview and the exporter use **separate mixers**, for the same reason they
use separate decoder pools: FFmpeg decoders are not safe for concurrent use.

## Where §48's workflows stand

| Workflow | Status |
|---|---|
| Import 4K → trim → split → add music → adjust volume → add text → preview → **export** | ✅ works end to end |
| Import audio → edit → export | ⚠️ import, place, mix and export work; the dedicated audio *editing* operations are V1.1 |
| Drop 20 files → convert → done | ❌ Phase 6 |

## Still outstanding

- **Audio editing** (§3): trim, fade and gain exist on the model; normalize, EQ
  and silence removal are V1.1 per PRODUCT_DIRECTION §6.
- **The converter and tools modes** (§4, §31) — Phase 6.
- **Developer ID signing and notarization** — needs a paid Apple account, so it
  is not something the build can resolve on its own.
- **Windows** — V1.1 per O-1; `WINDOWS_BRINGUP.md` is ready.

## Verified, not asserted

Every claim above was checked by producing a file and inspecting it:

| Check | Evidence |
|---|---|
| Video export | HEVC 1280×720, frame counts and durations matching the source |
| Composited overlays reach the FILE | A frame extracted from the export shows the title and logo |
| PNG transparency (§20) | Semi-transparent core shows video through it; no black box |
| Audio export (§4) | 567 AAC packets over 12.03 s; decoded mean −24.3 dB, max −18.0 dB |
| Licence posture (R-05) | Gate passes on the shipped build, rejects a GPL one |
| Portability (R-21) | `mediacore-model` cross-checks for Windows on every build |
| No crash surface (§22) | Zero `unwrap`/`expect`/`panic!` in non-test library code |
