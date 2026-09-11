# Windows bring-up

**Purpose:** close `RISK_REGISTER.md` **R-21** — the deferred Windows port decaying into a rewrite — and unblock spikes **S5** and **S9**.

**Context:** O-1 put Windows in V1.1, not V1. This is *not* about building the Windows app. It is about proving the shared core stays portable while macOS development continues. Compiling is the cheap 80% of that.

---

## Why this cannot be done from the Mac

Verified during Phase 1, not assumed:

| Crate | Windows cross-check from macOS | Why |
|---|---|---|
| `core/mediacore-model` | ✅ **passes today**, on every check | No FFmpeg. Pure Rust. |
| `mediacore-media` | ❌ **impossible** | bindgen needs Windows system headers; `cargo check --target x86_64-pc-windows-msvc` fails on missing `errno.h`. |

So the model crate is already covered. **Only the FFmpeg-bound crate needs the real machine.**

---

## What Phase 4 and Phase 6 added to the port surface

Written down here because the list grew substantially after this document was
first drafted, and a port plan that describes the old code is worse than none.

**Still pure Rust, still cross-checks from the Mac today:**

| Added | Where |
|---|---|
| `Geometry` — crop, rotation, flip, reframe | `mediacore-model/src/timeline.rs` |
| Gain envelopes (`GainPoint`, `automation_at`) | same |
| `LutSpec` | same |
| `SetGeometry`, `SetGainPoints` edits | `command.rs` |
| The `ripple_delete_range` split fix | `ops.rs` |

**Needs the Windows machine (FFmpeg-bound):**

| Added | Where | Note |
|---|---|---|
| Video transcoding | `mediacore-media/src/transcode.rs` | Names `hevc_videotoolbox` / `h264_videotoolbox` **directly**. Windows needs its own encoder selection — see AD-12 and §3.3's 🔍 VERIFY entries before enabling nvenc/qsv/amf. |
| Merge | `merge.rs` | Same encoder question on the re-encode path. |
| Loudness and silence analysis | `analysis.rs` | **Portable as written** — it is arithmetic over `mc_audio_read`, with no platform API. Should need nothing. |
| Transcription | `transcribe.rs` + `csrc/whisper_shim.c` | whisper.cpp builds on Windows with cmake + MSVC. `GGML_METAL` becomes `GGML_VULKAN` or CPU-only; measure before choosing. The shim and the Rust side need no changes. |

**Captions are largely free on Windows**, which is the payoff for choosing
whisper.cpp over `SFSpeechRecognizer`: the model, the chunking, the SRT/WebVTT
handling and the whole caption UI are already cross-platform. Only the ggml
backend selection is a Windows decision.

**Platform work, not a port — these are macOS frameworks with Windows
equivalents that have to be written rather than translated:**

| Feature | macOS | Windows equivalent |
|---|---|---|
| Extract Frames | ImageIO (`CGImageDestination`) | WIC |
| LUT 3D textures | Metal `texture3d` | D3D11 `Texture3D` — trilinear sampling is equivalent |
| Voiceover recording | AVAudioEngine input tap | WASAPI capture |
| Safe-area guides | SwiftUI overlay | pure UI, no media API |
| Finder open / document types | `AppDelegate` + `CFBundleDocumentTypes` | file associations in the installer |

### One warning carried over from Phase 4

**R-24: shader constant-buffer layout.** The Metal shaders had a Swift/Metal
struct alignment mismatch that silently corrupted blur and sharpen for weeks.
HLSL's packing rules are *different again and stricter* — a `float3` cannot
straddle a 16-byte boundary. Port the shader parameter structs as **scalars
only**, exactly as the Metal ones now are, and do not reintroduce vector fields.

---

## Order of work

### 0. Before anything: the deployment floor

`tools/deployment.sh` holds the macOS floor. **Windows needs its own equivalent**
and the same discipline — see `RISK_REGISTER.md` R-27, where every binary in the
macOS bundle turned out to be built for the developer's own OS while the plist
claimed otherwise. Nothing warned, because each piece was individually correct.

The Windows analogue is the subsystem version and the MSVC toolset; decide the
floor explicitly, put it in one place, and have `tools/check.sh` assert it
against the shipped binaries as it now does on macOS.

### 1. Toolchain
- Visual Studio Build Tools (MSVC + Windows SDK) — required by `bindgen` and the MSVC target
- `rustup` with `x86_64-pc-windows-msvc`
- LLVM/Clang — `bindgen` needs `libclang`
- Git, Python 3, NASM

### 2a. Build whisper.cpp

`third_party/whisper/build.sh` is cmake-driven and pinned to a commit. It has
**never run on Windows.** Expect the backend flags to need work: `GGML_METAL`
is Apple-only, and the choice between Vulkan, DirectML and CPU should be
measured rather than assumed. Everything else — the shim, the Rust binding, the
chunking, the model — is platform-neutral.

Then `tools/fetch_models.sh` for the SHA256-pinned model.

### 2. Build FFmpeg — LGPL only

`third_party/ffmpeg/build.sh` has a Windows branch (`--enable-d3d11va
--enable-dxva2`) that **has still never run**. But five macOS assumptions that
would have stopped it dead were found by *reading* it rather than by a failed CI
run, and are now fixed:

| Was | Now |
|---|---|
| `shasum -a 256` | `sha256()` — picks `shasum` or `sha256sum`, dies if neither exists rather than skipping verification |
| `sysctl -n hw.ncpu` | `ncpu()` — prefers `nproc` |
| `sed -i ''` (BSD form) | `sed_inplace()` — GNU sed takes `-i` with no argument |
| "is LAME built?" tested for `.dylib`/`.so` | tests `$SHLIB_EXT`, and `bin/*.dll` for Windows |
| `--install-name-dir=@rpath` and `-mmacosx-version-min` unconditional | moved into a `Darwin)` branch — the first is a Mach-O concept, the second is not a flag any Windows compiler accepts |

They are written as **capability checks rather than `uname` branches** wherever
possible: "does this machine have `sha256sum`" survives a platform nobody has
thought of; "is this macOS" does not.

The macOS path was rebuilt from scratch afterwards to prove the refactor changed
nothing — a licence-critical script is not somewhere to take a refactor on
faith.

**What is still unproven on Windows:** whether MSYS2's toolchain builds FFmpeg
at all here, whether LAME's configure works, and whether the DLL layout the
media crate expects matches what `--enable-shared` produces. Those need the
runner.

Non-negotiable, exactly as on macOS:
- **never** `--enable-gpl`, `--enable-nonfree`, `--enable-version3`
- **never** `libx264`, `libx265`, `libfdk-aac`
- LAME **is** required (LGPL) — without it there is no MP3 export (§4)
- **`nvenc` and `amf` are now cleared on copyright** (§3.3a, verified 2026-09-10
  against our pinned FFmpeg). Enabling them needs `nv-codec-headers` added to
  this build — a build task, not a licensing one. Do it AFTER the base Windows
  build is proven; adding a dependency to a build that has never succeeded makes
  two unknowns out of one.
- **`qsv` stays disabled.** It links `libmfx` rather than loading the driver's
  encoder, which is a real redistribution question and still open. An Intel-only
  machine therefore has no hardware encoder until that is answered — a gap worth
  stating rather than papering over.

**Then run the gate:**
```
python tools/license_gate.py --prefix build/ffmpeg-lgpl
```
It must PASS. Point it at any GPL build (e.g. a downloaded one) and confirm it FAILS — a gate that only ever passes proves nothing.

### 3. Build the media crate
```
set FFMPEG_INCLUDE_DIR=...\build\ffmpeg-lgpl\include
set FFMPEG_LIBS_DIR=...\build\ffmpeg-lgpl\lib
set FFMPEG_LINK_MODE=dynamic
cargo build --release
```
`FFMPEG_LINK_MODE=dynamic` is **required** — `rusty_ffmpeg` defaults to static linking, which fails against a shared build and would violate the LGPL dynamic-linking strategy if it succeeded.

### 4. Re-run the Phase 1 spikes that were deferred

| Spike | Pass condition |
|---|---|
| **S5** | **Re-scoped.** AD-2's Windows half is now decided (WinUI 3 + `SwapChainPanel`), and the target machine is an RTX 5080 — so "can Windows hit 4K30" is no longer the question. What S5 must still establish is that the **zero-copy path works end to end**: NVDEC → D3D11 texture → `SwapChainPanel`, with no CPU round trip. Measure on real hardware; a CI runner has no GPU and would measure WARP. |
| **S9** | Hardware probe across ≥3 GPU configurations; must reject non-working paths, cache by GPU + driver version |
| **S10** (Windows half) | 4K30 H.264 / 4K HEVC 10-bit / 4K60 on min-spec hardware. **Decides whether AD-5's Tier 2 proxies ship at all.** |
| **S1** (Windows) | Zero-copy: D3D11 texture from decoder → `SwapChainPanel`, no CPU frame copies |

### 5. Stand up CI — the actual R-21 mitigation

A Windows job that runs on **every commit**:
```
cargo check --target x86_64-pc-windows-msvc   (model crate — already green)
cargo build                                    (media crate)
cargo test
python tools/license_gate.py --prefix build/ffmpeg-lgpl
```
**Trigger (R-21):** if this stays red for more than a week, the port is decaying. That is the number to watch.

---

## Carry these findings across — they cost real time to learn

| Finding | Where |
|---|---|
| `FFMPEG_LINK_MODE=dynamic` is required | S3 |
| The C header is **generated** by cbindgen — never hand-edit `include/mediacore.h` | Phase 1 |
| The media worker needs sandbox-inherit entitlements on macOS; Windows has **no equivalent**, so the worker/parent access model must be re-validated there | S8 |
| Encoder quality: hardware needs ~2.31× (H.264) / ~1.74× (HEVC) the bitrate of libx264. **Re-measure on Windows** — NVENC/QSV/AMF differ from VideoToolbox and the preset multipliers may not transfer | S4b |
| 337 MB RSS per 4K decode stream on macOS (R-23). Measure the Windows figure; it constrains multi-track | S1 |

## Definition of done

- [ ] LGPL FFmpeg builds on Windows from the checked-in script
- [ ] Licence gate passes on the Windows build, and rejects a GPL one
- [ ] `mediacore-media` builds and its tests pass
- [ ] S5 run, with a verdict recorded for AD-2's Windows half
- [ ] S10 Windows half run, with a verdict recorded for AD-5's proxy tier
- [ ] Windows CI green on every commit
- [ ] `PHASE_1_RESULTS.md` updated with the Windows numbers
