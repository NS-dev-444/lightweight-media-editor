# Risk Register

**Project:** Lightweight cross-platform video + audio editor
**Document status:** Phase 0 deliverable 3 of 5
**Date:** 2026-09-09
**Review cadence:** at every phase gate, and whenever a risk's trigger fires.

---

## Scoring

**Likelihood:** Low (<20%) · Medium (20–60%) · High (>60%)
**Impact:** Low (days) · Medium (weeks) · High (months, or a shipped defect users notice) · Critical (product cannot ship, or ships and must be withdrawn)

Each risk has a **trigger** — an observable event that means the risk is materialising and the mitigation must start. A risk without a trigger is a worry, not a managed risk.

---

## Summary

| ID | Risk | L | I | Owner | Phase to resolve |
|---|---|---|---|---|---|
| ~~R-01~~ | ~~Two native apps in V1~~ — **RETIRED**, decision taken: macOS V1, Windows V1.1 | — | — | Product | ✅ resolved |
| R-21 | Windows port decays into a rewrite | Medium | High | Eng | Throughout V1 |
| R-22 | Transcription model size vs "lightweight" | Medium | Medium | Product | ✅ **RETIRED** — O-18 resolved, 57 MB bundled |
| R-02 | AVC/HEVC patent royalties | High | Critical | Product + Counsel | Before first distribution |
| R-03 | 4K preview fails on low-end Windows | Medium | High | Eng | Phase 1 |
| ~~R-04~~ | ~~FFI/zero-copy design wrong~~ — **RETIRED**, proven in spike S1 | — | — | Eng | ✅ resolved |
| R-23 | Per-stream memory limits multi-track playback | Medium | High | Eng | Phase 2 |
| R-05 | GPL/nonfree FFmpeg shipped by accident | Low | Critical | Eng | Phase 1 (CI gate) |
| R-06 | Colour management / HDR mishandled | High | High | Eng | Phase 2 |
| R-07 | Variable frame rate desync | High | High | Eng | Phase 2 |
| R-08 | WinUI 3 fails "lightweight" | Medium | High | Eng | Phase 1 |
| R-09 | Hardware encode quality complaints | **High** | **High** | Product | ⚠️ **TRIGGER FIRED** (S4) |
| ~~R-10~~ | ~~macOS sandbox retrofit~~ — **RETIRED**, validated in S8 | — | — | Eng | ✅ resolved |
| R-11 | `ffmpeg-next` unmaintained | Medium | Medium | Eng | Phase 1 |
| R-12 | V1 scope vs "lightweight" | High | High | Product | Now |
| R-13 | Windows code-signing lead time | Medium | Medium | Ops | Phase 1 |
| R-14 | No crash reporting vs privacy stance | Medium | Medium | Product | Phase 3 |
| R-15 | Corrupt/hostile media crashes app | **Low** | High | Eng | ✅ mitigated in S6 |
| R-16 | Undo/redo memory growth | **Low** | Medium | Eng | ✅ mitigated in Phase 2 |
| R-17 | HEVC decode gap on Windows | Low | Medium | Eng | Phase 5 |
| R-18 | Export A/B divergence between platforms | Medium | Medium | Eng | Phase 5 |
| R-19 | Team lacks Windows/media depth | Medium | High | Product | Now |
| R-20 | Effect shaders diverge between MSL/HLSL | Medium | Medium | Eng | Phase 3/5 |

---

## R-01 — Building two native applications in V1 — ✅ RETIRED 2026-09-09

**Resolved.** The product owner adopted the recommended sequencing: **macOS V1, Windows V1.1.**
The residual risk — that the deferred port decays into a rewrite — is tracked as **R-21**.
The original analysis is kept below because it is the rationale for R-21.

**Likelihood:** High · **Impact:** Critical · **Owner:** Product

§32 puts "Native macOS application" and "Native Windows application" both in MUST-HAVE V1. That is two UI codebases, two renderers, two hardware-acceleration paths, two packaging and signing pipelines, and two QA matrices — roughly double the engineering, and more than double the QA, of a single-platform V1.

Everything else in the spec pushes toward a small, fast product shipped by a small team. This one line pushes the opposite way, and it is the single largest threat to the schedule.

**Mitigation:** Ship macOS V1 first, Windows as V1.1, with the Rust core written cross-platform from day one so the Windows port is a port, not a rewrite. Phase 1 still spikes Windows (so the core does not accrete macOS assumptions), but Windows polish is deferred.

**If not mitigated:** plan for roughly 2× the schedule and say so explicitly up front.

**Trigger:** Phase 2 ends and the Windows spike from Phase 1 has not been kept building in CI.

---

## R-21 — The deferred Windows port decays into a rewrite

**Likelihood:** Medium · **Impact:** High · **Owner:** Eng

Deferring Windows to V1.1 is only cheap if the shared core stays genuinely portable. The failure mode is gradual and quiet: macOS assumptions leak into the Rust core — path handling, threading, colour, pixel formats, file access — and by V1.1 the "port" is a rewrite. That would retroactively invalidate AD-1, because the Rust core is justified *by the second platform*.

**Mitigation:**
- Rust core written cross-platform from the first commit; no `#[cfg(target_os = "macos")]` in core logic, only in the platform adapters.
- **A Windows build of the core stays green in CI throughout V1**, even with no Windows UI. Compiling is the cheap 80% of staying portable.
- **Split the core in two** (done in Phase 1): `mediacore-model` has no FFmpeg dependency and **cross-checks for Windows from a Mac**, verified — so most of Phase 2's work is portability-checked locally on every build. `mediacore-media` carries the FFmpeg FFI and **cannot** be cross-checked (bindgen needs Windows system headers; attempted and confirmed failing), so it needs a **real Windows CI runner**. Provisioning that runner is a concrete prerequisite, not a nice-to-have.
- The Phase 1 Windows spike (S5) is kept building rather than deleted.
- Any platform-specific code in the core requires a written justification in review.

**Trigger:** the Windows CI build stays red for more than one week, or any core module acquires a macOS-only code path without a documented reason.

---

## R-22 — Transcription model size contradicts "lightweight"

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Product

Captions are approved (O-16) and licensing is clean (MIT on both runtime and weights). The risk is not legal, it is product: transcription models are large, and bundling a big one directly contradicts §47 and the "lightweight" positioning that is the product's whole pitch.

**Mitigation:** decide the model strategy before Phase 3 (O-18). Preferred shape — ship a small model for instant out-of-box captions, offer larger models as an explicit opt-in download, and show the accuracy/size trade honestly in the UI. Downloading a model is a one-time cost, not a subscription, so it stays inside the governing principle — but it does mean the first caption run may need a network connection, which must be stated plainly rather than discovered.

**Trigger:** the installer exceeds ~200 MB, or captions are specified in a way that requires network access on every use.

### ✅ RETIRED, 2026-09-10

O-18 resolved: `ggml-base-q5_1` is bundled — 57 MB, multilingual, MIT weights.
The app comes to about 120 MB all in, well inside the 200 MB trigger, and the
network half of the risk is gone entirely because nothing is downloaded at
runtime. Larger models stay an opt-in for people who ask for them.

The trigger stays live for a future change: **if a larger model is ever made the
default, this risk reopens.**

---

## R-02 — AVC/HEVC patent royalty exposure

**Likelihood:** High · **Impact:** Critical · **Owner:** Product + Counsel

Patent licensing is independent of FFmpeg's copyright licence. Access Advance licenses HEVC decoders *and* encoders "installed in devices or software", seeking one royalty per copy at first sale, and has now absorbed the administration of Via LA's HEVC/VVC pools. Via LA licenses AVC. The common industry assumption — that using an OS-provided encoder discharges the app developer's obligation — was **not verifiable** during Phase 0 research.

This risk can invalidate the *business model* even if the architecture is perfect. It is not an engineering risk.

**Mitigation:**
1. Hardware/OS encoders only; no software H.264/HEVC encoder shipped (AD-12).
2. Written opinion from qualified counsel before first public distribution.
3. Budget royalties as a line item; check Via LA and Access Advance thresholds against projected unit volumes.
4. Keep an AV1/Opus/FLAC/WAV lane that has no pool demanding software royalties.
5. Keep Cisco OpenH264 documented as a fallback if a software H.264 encoder becomes necessary.

**Trigger:** any of — counsel's opinion is unfavourable; Access Advance publishes consolidated VCL Advance terms covering desktop software; a competitor in this segment receives a licensing demand.

---

## R-03 — 4K preview performance fails on low-end Windows hardware

**Likelihood:** Medium · **Impact:** High · **Owner:** Eng

4K playback on Apple Silicon is expected to be comfortable. A mid-range Windows laptop with only an Intel iGPU is the realistic worst case, and it decides whether the Tier 2 proxy system is optional infrastructure or mandatory infrastructure — which in turn decides whether import can stay instant (§1 "feels fast").

**Mitigation:** measure in Phase 1 on the worst target we intend to support, before the timeline is built. Define the minimum supported hardware explicitly and publish it. Build the tiered preview ladder (AD-5) so degradation is automatic and graceful.

**Trigger:** Phase 1 measurement shows < 24 fps sustained 4K30 H.264 preview on the minimum-spec Windows target.

---

## R-04 — FFI boundary or zero-copy frame handoff designed wrong — ✅ RETIRED 2026-09-09

**Proven in spike S1.** 360/360 frames arrived IOSurface-backed and bound directly as Metal textures; 0 software-decoded frames; 4,271 MB of frame data never copied; 4K30 sustained at 183 fps steady state. The ownership contract (Rust returns +1 retained, Swift consumes with `takeRetainedValue()`) is established and should be standardised across the data plane. Original analysis retained below as the rationale.

**Likelihood:** Medium · **Impact:** High · **Owner:** Eng

A 4K RGBA frame is ~33 MB. Copying frames across the FFI, or routing them through the UI language's heap, destroys both the performance and the memory budget. Equally damaging: letting the UI hold a second mutable copy of the timeline document, which produces state-divergence bugs that are extremely hard to diagnose.

**Mitigation:** the control-plane / data-plane split is decided in Phase 0 and proven in Phase 1 with a spike that renders a decoded frame end-to-end with zero copies, verified in Instruments / PIX. The Rust core owns document truth; the UI observes snapshots.

**Trigger:** any spike that shows a CPU-side frame copy, or any design discussion that proposes mutable timeline state in the UI layer.

---

## R-23 — Per-stream memory limits multi-track playback

**Likelihood:** Medium · **Impact:** High · **Owner:** Eng

Spike S1 measured **337 MB peak RSS for a single 4K decode stream**. `PHASE_0_ARCHITECTURE_RESEARCH.md` §R20 budgets ≤1.5 GB for a 20-clip project, which at this rate allows only about five concurrent decoders. Multi-track playback, and AD-5's tiered preview, both assume more than that.

The likely causes are the VideoToolbox IOSurface pool and FFmpeg's internal frame pool, both of which are tunable — but the number must be understood before Phase 2 fixes the memory architecture, not after.

**Mitigation:** measure where the 337 MB goes; tune pool sizes; consider a shared decoder pool with eviction rather than one decoder per clip; re-measure with 4 concurrent streams before committing to a multi-track design.

**Trigger:** two concurrent 4K streams exceed 700 MB combined.

---

## R-05 — Shipping a GPL or nonfree FFmpeg build by accident

**Likelihood:** Low (with the gate) / High (without it) · **Impact:** Critical · **Owner:** Eng

The FFmpeg installed on the development machine right now is built with `--enable-gpl --enable-version3 --enable-libx264 --enable-libx265`. Linking against a developer's system FFmpeg during a rushed build is a completely ordinary mistake, and it would make the shipped product's licensing untenable.

**Mitigation:** CI gate that parses the linked FFmpeg's `configuration` string and fails the build on `--enable-gpl`, `--enable-nonfree`, or any denied component. Build FFmpeg from our own pinned script only. Never resolve FFmpeg from `PATH` or Homebrew in any build configuration, including local development.

**Trigger:** the gate fires, or any build succeeds without the gate having run.

---

## R-06 — Colour management and HDR mishandled

**Likelihood:** High · **Impact:** High · **Owner:** Eng

The spec never mentions colour space, transfer function, or tone mapping, yet requires 4K HEVC support — and a large share of 4K HEVC in the wild is 10-bit HLG or PQ, straight off a phone. Treating it as Rec.709 produces washed-out, grey-looking exports. This is one of the most visible possible V1 defects and one of the most common in small editors.

**Mitigation:** define an explicit V1 colour policy before Phase 2:
- Internal pipeline: Rec.709 SDR, 8-bit or 16-bit float.
- HDR input: detect, tone-map to SDR with a documented curve, and tell the user in the UI that a conversion happened.
- HDR *output*: explicitly out of scope for V1, stated in the product docs.

**Trigger:** any HDR test clip appearing in the Phase 1 corpus without a defined expected result.

---

## R-07 — Variable frame rate sources desync

**Likelihood:** High · **Impact:** High · **Owner:** Eng

Screen recordings and phone video are commonly VFR. A frame-index-based timeline silently desyncs audio, often only noticeably at the end of a long clip — meaning it escapes short-clip testing and reaches users.

**Mitigation:** rational timebase and PTS-driven engine throughout (AD-9, R11). Add VFR sources to the Phase 1 test corpus and assert A/V sync at the *end* of long exports, not just the start.

**Trigger:** any A/V sync defect reported at all — treat the first one as evidence of a systemic timebase problem, not a one-off.

---

## R-08 — WinUI 3 fails the "lightweight" bar

**Likelihood:** Medium · **Impact:** High · **Owner:** Eng

AD-2's Windows choice rests on vendor claims from Build 2026, not on our own measurements. WinUI 3 has a history of rough edges, and a .NET runtime plus Windows App SDK carries startup and memory cost that must be checked against §47.

**Mitigation:** Phase 1 Windows spike measures cold start, idle memory, and `SwapChainPanel` 4K playback. Fall-back plan (Win32 + C++ + Direct2D) is identified now so choosing it later is a decision, not a crisis.

**Trigger:** spike shows cold start > 2.5 s or idle RSS > 250 MB for an empty window.

---

## R-09 — Users complain about hardware-encode quality

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Product

Hardware encoders need a higher bitrate than a good software encoder to reach the same perceived quality. Shipping hardware-only encode (a deliberate choice, for both speed and patent reasons) means our files are larger at matched quality — the opposite of what the "Smaller File" presets promise.

**Mitigation:** choose preset bitrates from Phase 1 VMAF measurements, not from guesses. Label presets honestly. Prefer HEVC where the target supports it, since Apple's hardware HEVC encoder is well regarded. Accept the trade-off publicly rather than being caught out by a reviewer's comparison.

**⚠️ TRIGGER FIRED — spike S4, 2026-09-09.** Measured median penalty is **38%** for `h264_videotoolbox` (range 1.00–2.87×) and **21%** for `hevc_videotoolbox`, against a 15% trigger threshold. Worse, both hardware encoders show a **quality ceiling** they cannot pass at any bitrate: on 4K CGI, `h264_videotoolbox` plateaus at VMAF 94.57 at 50 Mbps where libx264 reaches 96.61. Screen recordings are the worst case at 2.87×.

Likelihood and impact both raised to High.

**Revised mitigation:**
1. **Default to HEVC** wherever target compatibility permits — it is markedly better than hardware H.264 and occasionally beats software H.264.
2. **Re-test with quality-targeted rate control** (`-q:v`) rather than ABR before treating the ceiling as final; the current numbers may understate the hardware encoders.
3. **Set preset bitrates from the measured data**, with roughly 1.4× (H.264) and 1.2× (HEVC) multipliers over software-encoder intuition.
4. **Label "Best Quality" presets honestly.** With hardware-only encode, "Best Quality" means best available *on this hardware*, not best achievable.
5. Escalate to AD-12: decide whether an LGPL-compatible software encoder is warranted for a quality tier, accepting the patent implications documented in the audit.

**Original trigger (now historical):** Phase 1 VMAF shows a gap larger than roughly 15% bitrate at matched quality versus `libx264 -preset medium` at our preset targets.

---

## R-10 — macOS sandbox retrofit — ✅ RETIRED 2026-09-09

**Validated in spike S8.** A real sandboxed, hardened, signed `.app` was built; security-scoped bookmarks were created (864 bytes), stored, resolved after relaunch, and survived a rename plus a directory move. The sandboxed **worker process** was confirmed to inherit access via `com.apple.security.inherit` and can take a path, so the worker API needs no redesign. AD-9's decision to carry bookmarks in schema v1 is confirmed necessary — the negative control showed the sandbox denies arbitrary paths outright.

Residual, tracked but not blocking: Developer ID signing and notarization are untested (ad-hoc only), and removable-volume disconnection is untested.

**Likelihood:** Low · **Impact:** High · **Owner:** Eng

If the project format ships without security-scoped bookmarks and the product later needs sandboxing (Mac App Store, or a future platform requirement), every existing project loses access to its media on relaunch and the format must be migrated.

**Mitigation:** bookmarks in the schema from version 1 (AD-9); build sandboxed from the start even while distributing via Developer ID. Validate the media worker process inside the sandbox in Phase 1 — helper processes and sandbox entitlements interact and this is easy to discover too late.

**Trigger:** a Phase 1 spike that runs the worker unsandboxed "for now".

---

## R-11 — `ffmpeg-next` in maintenance mode

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Eng

`ffmpeg-next` is reported to be in maintenance mode targeting FFmpeg 3.4–8.0; the FFmpeg on this machine is 9.0.1. A binding that lags the FFmpeg version we want to ship is a real supply-chain constraint.

**Mitigation:** keep our own thin safe wrapper over raw FFI (`rusty_ffmpeg` or generated bindings) so we are never blocked on a third-party crate's release schedule. Our FFI surface into FFmpeg is narrower than the full API — we need demux, decode, filter, encode, mux, and seek, not everything.

**Trigger:** the chosen crate does not build against our pinned FFmpeg version.

---

## R-12 — V1 scope contradicts "lightweight"

**Likelihood:** High · **Impact:** High · **Owner:** Product

The §32 MUST-HAVE list is large: two native apps, timeline editing, text, image overlays, crop/rotate/speed, audio editing, audio and video conversion, batch conversion, presets, hardware acceleration, project save/load, autosave/recovery, undo/redo, drag-and-drop. That is close to the scope of a first release of a commercial NLE, and it sits directly against §47's "should NOT feel bloated" and §1's "lightweight".

**Mitigation:** cut V1 to the three workflows §48 defines as success, and move the rest to V1.1. Specific proposal in `PHASE_0_VALIDATION.md` §3.2.

**Trigger:** Phase 2 estimates exceed the schedule by more than 25%.

---

## R-13 — Windows code-signing certificate lead time

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Ops

OV/EV certificates with hardware-backed keys have procurement and identity-verification lead times, and SmartScreen reputation accrues only after real download volume. Discovering this in Phase 9 delays release.

**Mitigation:** start procurement during Phase 1. Sign every internal Windows build with the real certificate from the first CI pipeline so reputation accrues and the signing pipeline is exercised continuously.

**Trigger:** Phase 5 begins without a certificate in hand.

---

## R-14 — No crash reporting versus the privacy stance

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Product

§25 forbids telemetry for core functionality; §44 requires "no known reproducible crashes" at release. With zero field reporting, crashes are only known if users write in, and §42's failure matrix cannot be validated against real-world media.

**Mitigation:** opt-in crash reporting, off by default, explicitly explained at first run, containing stack traces only — no media, no file paths, no file names, no user content. This is compatible with §25's own carve-out for explicit opt-in.

**Trigger:** the first beta ships with no mechanism to learn about crashes.

---

## R-15 — Corrupt or hostile media crashes the application — MITIGATED, likelihood lowered

**Spike S6:** 1024 malformed files through the worker-process design produced **0 host crashes, 0 worker crashes, 0 hangs**, with every file classified and given human-readable wording. Note the honest reading — FFmpeg 8.1.2 never crashed, so the isolation is **insurance against unknown failures**, not a response to observed ones. The risk is not retired, because absence of a crash in one corpus is not proof of safety; likelihood drops from Medium to Low.

**Still to do:** exercise the recovery path with a deliberately crashing worker (a real signal, not a timeout kill), and fuzz the 4K path.

**Likelihood:** ~~Medium~~ **Low** · **Impact:** High · **Owner:** Eng

Codec libraries parsing untrusted input are a classic crash and memory-safety surface. §22 requires the app never to crash on an invalid file; §42 requires graceful failure on corrupt video and audio.

**Mitigation:** all decode/encode in the isolated worker process (AD-3); worker crash becomes a classified job error, and the app offers retry. Fuzz the probe/demux path with a corpus of truncated and mutated files during Phase 1. Never parse media in the UI process, including for thumbnails.

**Trigger:** any crash report whose stack passes through `libav*` inside the UI process.

---

## R-16 — Undo/redo memory growth — MITIGATED in Phase 2

`History` stores only ids and small values — never document snapshots, never media — and is bounded by **both** an entry count and a byte budget, trimming oldest-first. A scripted 200-edit session with a 20-entry bound holds at 20. Likelihood drops to Low.

**Likelihood:** ~~Medium~~ **Low** · **Impact:** Medium · **Owner:** Eng

§28 requires everything to be undoable without duplicating media. Command objects are small, but a careless implementation that snapshots the whole document per edit will grow memory steadily during a long session — the exact failure §27 warns about.

**Mitigation:** command pattern with inverse operations, not document snapshots. Bounded history depth with a memory budget. Measure history memory in a scripted 500-edit session during Phase 2.

**Trigger:** RSS grows monotonically in a long scripted editing session with no media loaded.

---

## R-17 — HEVC decode gap on Windows

**Likelihood:** Low · **Impact:** Medium · **Owner:** Eng

Windows may lack HEVC support unless the Store "HEVC Video Extensions" package is installed. If we ever depend on Media Foundation for HEVC decode, some users simply cannot open their files.

**Mitigation:** decode HEVC with our own FFmpeg build plus D3D11VA hardware acceleration; never require the Store package. (This trades a functionality gap for patent-layer exposure, which is tracked in R-02 — an honest trade, made deliberately.)

**Trigger:** any code path that calls Media Foundation for decode.

---

## R-18 — Export output diverges between macOS and Windows

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Eng

Different hardware encoders, different filter implementations, and different colour conversion paths can make the same project export differently on each platform — different loudness, different colour, different duration by a frame.

**Mitigation:** a golden-file test suite run on both platforms, asserting duration, frame count, audio loudness (LUFS), and per-frame colour within a tolerance. Any divergence outside tolerance is a bug, not a platform difference.

**Trigger:** the first user report of "it looks different on my other computer".

---

## R-19 — Team depth in media engineering and Windows

**Likelihood:** Medium · **Impact:** High · **Owner:** Product

This architecture needs Rust, Swift/AppKit, Metal, C#/WinUI, D3D, FFmpeg internals, codec/colour knowledge, and two packaging pipelines. That is a wide skill surface for a small team, and the media-specific parts (timebases, colour, codec quirks) are where inexperience produces defects that survive to release.

**Mitigation:** confirm honestly which of these the team has before committing to the two-platform V1. This risk and R-01 compound each other — mitigating R-01 also mitigates this one.

**Trigger:** Phase 1 spikes take more than 2× their estimate.

---

## R-20 — Effect shaders diverge between MSL and HLSL

**Likelihood:** Medium · **Impact:** Medium · **Owner:** Eng

Writing each effect twice means brightness on Windows can quietly differ from brightness on macOS.

**Mitigation:** a single written specification of each effect's maths (input range, colour space, formula, clamping), plus a shared reference test: a synthetic input image, a fixed parameter set, and an expected output compared within tolerance on both platforms in CI.

**Trigger:** any effect implemented on the second platform without a reference test.

---

## R-24 — Swift struct layouts do not match Metal's constant-buffer rules

**Likelihood:** High · **Impact:** High · **Owner:** Eng · **Status:** 🔴 FIRED, then mitigated

Metal aligns `float2` to 8 bytes and `float4` to 16 inside a constant buffer;
a Swift struct of `Float`s packs at 4. Any vector field after an odd number of
scalars therefore reads from a different offset than Swift wrote — and every
value involved is a plausible float, so **nothing errors and nothing looks
obviously wrong.**

This already happened. `float2 texel` sat after thirteen floats, so blur and
sharpen had been sampling with the wrong step since they were written. It only
surfaced when adding crop and rotation put a *visible* symptom on the same bug
(`PHASE_4_EDITING.md` §1).

**Mitigation, in place:** both shader parameter structs are **scalars only**, in
the same order as their Swift counterparts. That makes the layouts identical by
construction rather than by inspection. Any new field must be a scalar.

**Trigger:** a `float2`, `float3`, `float4` or `packed_*` appearing in a shader
struct that is filled from Swift. This applies equally to HLSL on Windows, where
the constant-buffer packing rules are different again and stricter.

---

## R-25 — Editing operations that silently discard content

**Likelihood:** Medium · **Impact:** High · **Owner:** Eng · **Status:** 🔴 FIRED, then mitigated

`ripple_delete_range` — the operation PRODUCT_DIRECTION §7 calls the primary
gesture — kept the head of any clip it cut into and **threw the tail away**. A
four-second take with a two-second gap came back as a 1.1-second one.

What makes this a risk class rather than one bug: a comment described the
behaviour as intentional ("splitting into head+tail is a separate, explicit
operation"), so it read as a decision rather than a defect, and the existing
tests only covered ranges that spanned clip boundaries.

**Mitigation, in place:** tests that assert on **total timeline duration** after
an edit, not only on the shape of the clips involved. Duration is the quantity a
user notices immediately and a boundary-focused test never checks.

**Trigger:** any new edit that can shorten a clip. Ask what happens to the part
after the cut, and write the duration assertion first.

---

## R-26 — Verifying features that need the user's hardware or consent

**Likelihood:** High · **Impact:** Low · **Owner:** Eng

Voiceover recording is written but has never been run: exercising it means
capturing audio from the user's microphone, which is not something to do while
they are away from the machine. The same will apply to camera capture, to any
cloud account, and to Windows until the machine exists.

**Mitigation:** build the parts that *can* be checked without the user (file
writing, permission refusal, error paths, the level meter's arithmetic), and
**say plainly in `STATE_OF_PLAY.md` which paths are unverified** rather than
letting "implemented" imply "tested".

**Trigger:** any feature whose only test requires a person present.

---

## R-27 — The build's macOS floor drifts from what the app claims

**Likelihood:** High · **Impact:** High · **Owner:** Eng · **Status:** 🔴 FIRED, then mitigated

`Info.plist` said `LSMinimumSystemVersion 14.0`. The Swift binary, all six
FFmpeg dylibs, LAME, whisper.cpp and the C shim were every one of them built
for **26.0** — the version of macOS this machine happens to run — because no
build script set a deployment target and every compiler defaults to the host.

Launch Services would have allowed the app onto a macOS 14 machine and dyld
would then have refused the binaries. **Nothing warned about it.** Each piece
was individually correct; only the combination was wrong, which is the property
that made it survive.

Two further traps found while fixing it, both worth knowing:

- **`make` relinks only what changed.** Re-running the FFmpeg build with a new
  floor produced `libavcodec` at 14.0 and `libavutil` at 26.0 — a prefix in two
  minds. The build script now stamps what it built for and wipes the prefix when
  that changes.
- **`build.rs` is cached.** The C shim's object file survived from before the
  floor existed, and only the linker's "built for newer macOS" warning revealed
  it. The flag is now passed explicitly rather than inherited from the
  environment.

**Mitigation, in place:** `tools/deployment.sh` holds the floor as **one
definition**, sourced by all three build scripts and substituted into the plist,
so the compiler and the plist cannot disagree. `tools/check.sh` then asserts
that the plist and **every binary in the shipped bundle** agree with it — the
gate was verified to fail by moving the floor without rebuilding.

**The floor is 14.0 because of `CADisplayLink`**, which the playback clock uses
and which arrived in macOS 14.

**Trigger:** any new compiled dependency, on either platform. Windows has the
same class of problem with its own subsystem version.

---

## R-28 — Nothing runs the gates

**Likelihood:** High · **Impact:** High · **Owner:** Eng · **Status:** 🟡 Mitigated

The licence gate, the Windows cross-check, the §22 lint and 150 tests all
existed and all ran **only when somebody remembered**. R-05 — accidentally
linking the GPL FFmpeg that sits on the default `PATH` — is precisely the
failure a forgotten gate does not catch, and it is unfixable after release.

**Mitigation, in place:** `tools/check.sh` runs everything, and
`.github/workflows/ci.yml` runs *that script and nothing else* — so CI and a
developer's machine cannot disagree about what passing means. CI additionally
re-asserts the licence gate on the **built artefact**, because R-05 is about
what gets linked rather than what gets written in a script.

Each gate was verified to **fail** as well as pass: a planted `unwrap()`, a
planted unused variable, a floor moved without rebuilding, and the Homebrew GPL
FFmpeg pointed at the licence gate (correctly rejected, `--enable-gpl`).

**Still open:** CI has never actually run — there is no remote yet. The workflow
is written against `macos-15` runners and the first real run should be expected
to need adjustment.

---

## Risks explicitly accepted

| Risk | Why accepted |
|---|---|
| Hardware encoders are less efficient than `libx264` at matched quality | Deliberate trade for speed and patent posture; mitigated by preset tuning and honest labelling (R-09). |
| No HDR output in V1 | Out of scope; V1.1 candidate. Must be documented so it is not perceived as a defect. |
| No pitch shifting in V1 | Licensing and quality both unresolved; the spec already hedged on it. |
| AV1 Sisvel pool uncertainty | No successful suit against a major implementer is reported; large implementers have declined to pay. Revisit if that changes. |
| JSON project format is larger than a binary format | Debuggability during development outweighs size; CBOR migration path exists behind the same schema. |
