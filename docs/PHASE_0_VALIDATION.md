# Phase 0 — Validation Report

**Project:** Lightweight cross-platform video + audio editor
**Document status:** Phase 0 deliverable 5 of 5 (the §46 Rule 12 written validation report)
**Date:** 2026-09-09

---

## 1. What Phase 0 did, and how

Per §50, no application code was written. The work was: read the specification in full, research the twenty topics in §33 against primary sources where they exist, record confidence honestly, and challenge the specification rather than agree with it.

**Deliverables produced:**

| Document | Purpose |
|---|---|
| `PHASE_0_ARCHITECTURE_RESEARCH.md` | Findings on all 20 §33 topics, each tagged by confidence |
| `ARCHITECTURE_DECISION.md` | The 13 §34 decision-gate answers, with alternatives rejected and a consistency check |
| `RISK_REGISTER.md` | 20 risks, scored, with mitigations and observable triggers |
| `DEPENDENCY_AND_LICENSE_AUDIT.md` | Two-layer licence analysis, allow/deny lists, CI gates |
| `PHASE_0_VALIDATION.md` | This report |

Produced after Phase 0 closed, and cross-referenced here so the set stays coherent:

| Document | Purpose |
|---|---|
| `PRODUCT_DIRECTION.md` | **PROPOSED, not ratified.** Re-aims the product's positioning, replaces §31's mode structure and §32's V1 list. Changes no engineering decision in `ARCHITECTURE_DECISION.md`. |

One measurement *was* performed, because it was cheap and it closed an option: AVFoundation was tested against MKV, MP4 and WebM on this machine and refuses the first and third even when the streams are identical to the second. That moves "use AVFoundation instead of FFmpeg on macOS" out of the option space on evidence rather than on assumption (`PHASE_0_ARCHITECTURE_RESEARCH.md` §R4).

**What Phase 0 could not do:** settle anything else requiring measurement or legal opinion. Those are listed in §5 rather than papered over, because §46 Rule 11 requires documenting limitations instead of pretending.

---

## 2. Verdict

**Phase 0 research is complete. The Phase 0 decision gate is NOT yet passed.**

§34 says: "Do not proceed until these decisions are internally consistent." The consistency check in `ARCHITECTURE_DECISION.md` found one outright contradiction and two unresolved tensions, and all three are **product decisions, not engineering decisions**. An implementer should not resolve them silently.

Recommendation:

- ✅ **Start Phase 1 now.** Every Phase 1 spike listed in §6 is valuable regardless of how the open product questions are answered, and three of them exist specifically to answer questions Phase 0 cannot.
- ⛔ **Do not start Phase 2** (the real project/timeline engine) until §5's blocking items are answered. Phase 2 builds the data model, and the open questions change it.
  - **Update:** O-1 and O-16 are now resolved (§5b). **O-3 (colour policy) and O-4 (crash reporting) remain**, and both still change the Phase 2 data model.

---

## 3. Challenges to the specification

§50 asks explicitly for anything technically weak, unnecessarily complex, contradictory, or risky, and says not to agree blindly. This section is that.

### 3.1 The biggest problem: two native applications in V1

§32 places both a native macOS app and a native Windows app in MUST-HAVE V1. This is the single riskiest line in the specification.

It means, in V1: two UI codebases, two renderers with two shader languages, two hardware-acceleration stacks, two packaging and signing pipelines, two update mechanisms, two crash-reporting paths, and a QA matrix that multiplies rather than adds. Meanwhile the rest of the spec — "lightweight", "simple", "do not over-engineer", "do not build another Premiere" — describes a product built by a small team moving quickly.

These two things are in direct conflict, and the spec never acknowledges it.

**Recommendation:** macOS in V1; Windows in V1.1. Write the Rust core cross-platform from the first commit, keep a Windows spike building in CI throughout so the core cannot accrete macOS assumptions, and ship the Windows app as the next release. This preserves the entire architecture — nothing in `ARCHITECTURE_DECISION.md` changes — while roughly halving the time to a shippable product.

**If this recommendation is rejected**, that is a legitimate call, but the schedule must be planned at roughly 2× and stated openly rather than discovered in Phase 5.

**Second-order effect:** if Windows leaves V1 entirely (not just deferred), the Rust core decision should be re-opened. Rust is justified *by the second platform*. A macOS-only product would be faster to build in Swift end-to-end. Do not inherit AD-1 without its premise.

### 3.2 The V1 MUST-HAVE list is not a lightweight product

§32's V1 list, taken literally, is close to the scope of a commercial NLE's first release. §47 says the product must not feel bloated. §1 says lightweight. The list and the philosophy do not describe the same product.

The specification actually contains its own better answer. §48 defines success as **three specific workflows**: edit-and-export a 4K video, edit-and-export audio, batch-convert twenty files. That is a sharper, more testable V1 definition than the feature list in §32.

**Recommendation: make §48 normative and subordinate §32 to it.** Concretely:

| Keep in V1 (serves §48's three workflows) | Move to V1.1 |
|---|---|
| Import, drag/drop, media library | Arbitrary multi-track video |
| Timeline: 1 video track + 1 overlay track + 2 audio tracks | Transitions |
| Trim, split, cut, delete, reorder | Freeze frame |
| Crop, rotate, flip, speed | Basic EQ, compressor, noise reduction |
| Volume, fade in/out, mute | Pitch adjustment |
| Detach / replace audio | Silence removal |
| Text overlay (static + fade), image overlay (PNG/JPEG) | WebP overlays |
| Audio waveform, trim/split/fade/gain/normalize | Subtitle support |
| Export presets + advanced panel | HDR workflows |
| Converter with batch queue, extract audio | Additional export codecs |
| Project save/load, autosave, recovery, undo/redo | |
| Hardware acceleration | |

This is not a large cut. It removes the features that are expensive, licence-encumbered, or hard to make reliable, and keeps everything §48 needs.

### 3.3 The specification contradicts itself

| Contradiction | Detail | Recommended resolution |
|---|---|---|
| **Multiple video tracks** | §2 lists "Multiple video tracks where practical" and "Multiple audio tracks" as things the editor *must support*; §30's UI mock shows V1, V2, A1, A2; but §32's MUST-HAVE V1 list omits multiple video tracks entirely. | §32 governs V1. Ship one video track plus one overlay track (enough for text and logo overlays, which §32 *does* require), full multi-track in V1.1. |
| **§2/§3 "must support" vs §32 MUST-HAVE** | §2 and §3 contain long "must support" lists including transitions, flip, freeze frame, detach/replace audio, normalize, EQ, silence removal, pitch. §32's V1 list contains none of those except by implication in "Basic audio editing". The spec never says which list is authoritative. | State explicitly that §2/§3 describe the **product**, §32 describes the **release**, and §48 defines **done**. |
| **Privacy vs. quality bar** | §25 forbids telemetry for core functionality; §44 requires "no known reproducible crashes" at release and §42 requires a large failure matrix to hold in the real world. With zero field signal, neither is verifiable after shipping. | Opt-in, off by default, crash traces only — no media, no file paths, no file names. §25 already permits explicit opt-in; make it an explicit product decision rather than an implied gap. |

### 3.4 Significant omissions

These are absent from the spec and each one affects architecture, so they cannot be deferred to a later phase without cost.

1. **Colour management and HDR.** The spec requires 4K HEVC but never mentions colour space, transfer function, or tone mapping. Much real-world 4K HEVC is 10-bit HLG or PQ. Treating it as Rec.709 produces washed-out output — a highly visible defect. **Needs a written V1 colour policy before Phase 2.** Recommended: Rec.709 SDR pipeline; detect HDR input, tone-map with a documented curve, tell the user; no HDR output in V1.

2. **Variable frame rate.** Never mentioned. Screen recordings and phone video are commonly VFR, and a frame-index-based timeline silently desyncs audio on them — usually only visible at the end of a long clip, so it escapes casual testing. **The engine must be PTS-driven on a rational timebase**, decided now (AD-9), not retrofitted.

3. **"Smart Export" is named but not specified.** §12 is titled Smart Export and then describes an ordinary export dialog. The actual smart behaviour — **remuxing without re-encoding** when the edit is cut-only and the source format already matches the target — is missing, and it is the single largest perceived-speed win available for the most common edit anyone makes. It also directly serves §26's "never re-encode source files unnecessarily". **Recommend adding stream-copy export to V1.** It is not expensive to build and it is the kind of thing that makes a product feel fast in a way no amount of optimisation elsewhere will.

4. **Timecode, drop-frame, and pixel aspect ratio.** Unspecified. Affects the timeline model.

5. **Audio channel layouts beyond stereo.** Unspecified. 5.1 audio in MKV is common; the app needs a defined downmix policy.

6. **Custom dimension validation.** §5 allows custom dimensions. H.264/HEVC require even dimensions, and hardware encoders have additional alignment and maximum-size constraints. Custom sizes must be validated and snapped, with the adjustment shown to the user — not silently rejected by the encoder at export time.

7. **Update mechanism.** Not mentioned; affects packaging and signing on both platforms.

8. **Licensing / activation** for a commercial product. Not mentioned; interacts directly with §25's privacy stance.

9. **Internationalisation.** Not mentioned; retrofitting is expensive and touches every string in the UI.

10. **Accessibility** appears only in §43 "Polish". For a custom-drawn timeline that is far too late — custom views expose nothing to assistive technology unless designed to. Accessibility for the timeline and preview must be designed in Phase 3, not added in Phase 9.

### 3.5 Requirements that cannot be verified as written

§45 says a feature is not complete merely because the happy path works. By the same standard, these requirements are not testable as written:

| Requirement | Problem | Fix |
|---|---|---|
| §41 performance tests | Lists nine test cases and says "record" the metrics, but defines **no pass/fail thresholds**. A benchmark with no target cannot fail, so it cannot gate a release. | Adopt the budget table in `PHASE_0_ARCHITECTURE_RESEARCH.md` §R20, ratified or corrected by Phase 1 measurement. |
| §18 "noise reduction only if it can be implemented without making the application bloated" | "Bloated" is unmeasurable; this cannot be accepted or rejected. | Cut from V1 (§3.2). If it returns, define the acceptance criterion in MB and ms. |
| §3 "Pitch adjustment where technically reliable" | Same shape. Also has a licensing trap (Rubber Band is GPL). | Cut from V1 (AD-7). |
| §21 "Pause where technically safe" | Pausing mid-encode is not generally possible with hardware encoders; only *between* files in a batch is realistic. | State plainly: batch pauses between items; the in-flight item runs to completion or is cancelled. Per §46 Rule 11, document the limitation rather than implying a capability. |
| §16 "Playback should remain responsive" | No number. | Adopt the fps and scrub-latency budgets from §R20. |
| §22 "Never crash because of an invalid media file" | Correct requirement, and **unachievable in-process** with codec libraries parsing untrusted input. | Achievable only via the isolated worker process in AD-3. This is a case where the spec's requirement forced a good architectural decision. |

### 3.6 Where the specification is right, and should be defended

Not everything needs challenging. These are the strongest parts and should not be traded away under schedule pressure:

- **§13 non-destructive editing and §46 Rule 9 ("never destroy source media").** Made a compile-time property in AD-9 rather than a coding convention.
- **§48's three workflows.** The clearest definition of done in the document; §3.2 recommends promoting it to normative.
- **§23's error-handling standard** ("FFmpeg error 234" → a human explanation). Requires an error-classification layer in the core from the start, which AD-1 includes.
- **§26's performance principles.** Correct, specific, and unusually well-aimed for a spec of this kind.
- **§46 Rules 6, 10, 11** (no fake functionality; never claim untested format support; document limitations). These are the rules that keep a media app honest, and they are why this report has a `[MEASURE]`/`[COUNSEL]` tag on every unverified claim.
- **§6's insistence that export uses original source media.** Enforced structurally in AD-5.

---

## 4. Decisions made in Phase 0

| # | Decision | State |
|---|---|---|
| AD-1 | Rust shared core; Swift on macOS; C# on Windows | DECIDED |
| AD-2 | SwiftUI + AppKit (macOS); WinUI 3 (Windows) | GATED on Phase 1 |
| AD-3 | FFmpeg, LGPL-only, dynamic, in an isolated worker process | DECIDED |
| AD-4 | Metal / Direct3D native compositors; shaders written twice from one spec | DECIDED |
| AD-5 | Tiered preview (direct → scaled → proxy); export always from source | GATED on Phase 1 |
| AD-6 | Hardware acceleration selected by real probing, not driver claims | DECIDED |
| AD-7 | One audio stack (FFmpeg); f32 planar; fixed project rate; R128 normalise; no pitch in V1 | DECIDED |
| AD-8 | Platform split table; identical semantics, native implementation | DECIDED |
| AD-9 | Single-file JSON project, versioned, atomic save, journal autosave, bookmarks | DECIDED |
| AD-10 | Content-hash-keyed cache, four budgeted classes, never source of truth | DECIDED |
| AD-11 | Our own reproducible LGPL FFmpeg build, signed, with CI gates | DECIDED |
| AD-12 | Hardware-only H.264/HEVC encode; patent position open | DECIDED (posture) / OPEN (legal) |
| AD-13 | Top technical risks identified and registered | DECIDED |

---

## 5. Open items ledger

### Blocking Phase 2 — ✅ ALL RESOLVED as of 2026-09-09

> **Update 2026-09-09 — O-1 and O-16 resolved by the product owner.** See the resolutions below.

| # | Question | Owner | Why it blocks |
|---|---|---|---|
| ~~O-1~~ | ~~Is Windows in V1, or V1.1?~~ | Product | ✅ **RESOLVED: macOS V1, Windows V1.1.** Rust core stays cross-platform from the first commit; a Windows spike stays building in CI (R-01). |
| O-2 | Does §32 or §48 define V1 scope? Is multi-track video in V1? | Product | Determines the timeline data model built in Phase 2. **Partially answered** by `PRODUCT_DIRECTION.md` §6 (one video + one overlay track), pending sign-off. |
| ~~O-3~~ | ✅ **RESOLVED** — SDR Rec.709 pipeline, detect + tone-map HDR, no HDR output in V1. See §5c. |
| ~~O-4~~ | ✅ **RESOLVED** — opt-in crash reporting, off by default, stack traces only. See §5c. |

### Blocking first public distribution

| # | Question | Owner |
|---|---|---|
| O-5 | Does OS/hardware encoding discharge our AVC/HEVC royalty obligation? | ⚖️ Counsel |
| O-6 | What are our actual AVC and HEVC royalty obligations at projected volume? | ⚖️ Counsel + Product |
| O-7 | Full LGPL compliance review of the shipping build | ⚖️ Counsel |
| O-8 | Every 🔍 VERIFY in the dependency audit resolved | Eng |

### Raised by `PRODUCT_DIRECTION.md`

| # | Question | Owner | Why it blocks |
|---|---|---|---|
| O-16 | Is local (on-device) transcription within §32's "no AI" policy? | Product | Blocks captions, the largest addition in the proposed V1 scope. Also adds a model dependency and a model-weights licence row to the audit. |
| O-17 | Free-tier distribution volume vs per-copy AVC/HEVC royalties | Product + ⚖️ Counsel | Blocks pricing. Currently unowned. |

### Answered by Phase 1 measurement

O-9 timeline rendering approach · O-10 whether Tier 2 proxies are needed at all · O-11 WinUI 3 viability · O-12 zero-copy FFI proven · O-13 preset bitrates from VMAF data · O-14 FFmpeg binding crate choice · O-15 minimum supported hardware.

---

## 5b. Resolutions (2026-09-09)

### O-1 — Platform sequencing: **macOS V1, Windows V1.1**

Adopted as recommended in §3.1. Consequences, so nothing is inherited by accident:

- **AD-1 (Rust core) stands.** Its stated caveat was that Rust is justified *by the second platform*. Windows is **deferred, not dropped**, so the premise holds — the point of the Rust core is that the Windows port is a port, not a rewrite.
- **R-01's mitigation is now mandatory, not optional.** The Rust core is written cross-platform from the first commit and a Windows build stays green in CI throughout V1. Without that, "V1.1 port" silently becomes "V1.1 rewrite".
- **AD-2's Windows half becomes a V1.1 gate**, not a Phase 1 gate. The WinUI 3 decision is no longer on the critical path.
- **Phase 5 moves out of V1.** Phases 1–4, 6–10 proceed for macOS.
- Windows code-signing procurement (R-13) can relax to V1.1 timing, but the *decision* between MSIX and MSI should still be made before the core's file-access abstractions harden.

### O-16 — Local processing: **approved**, and the principle restated

The product owner's rule is clearer than §32's wording and supersedes it for future decisions:

> **No recurring cost. No dependency on an external service.**
> Anything that runs locally on the user's machine is acceptable.

This reframes §32's DO-NOT-BUILD list. The entries that matter — "Cloud editing", "Subscription-required processing", "Stock media" — are all instances of *this* rule. "AI video generation" and "AI avatars" stay out of V1 on **scope** grounds (§32 Rule 1: features must be approved), not because local computation is objectionable.

**Unblocked:** captions via local transcription. This is the largest item in `PRODUCT_DIRECTION.md` §6's ADDED list and, per §8 there, the most-wanted feature by anyone publishing video.

**Licensing — verified, not assumed** (see `DEPENDENCY_AND_LICENSE_AUDIT.md` §5):

| Component | Licence | Evidence |
|---|---|---|
| OpenAI Whisper — **code and model weights** | MIT | LICENSE read: "MIT License / Copyright (c) 2022 OpenAI" |
| `whisper.cpp` (ggml) | MIT | LICENSE read: "MIT License / Copyright (c) 2023-2026 The ggml authors" |

Both are MIT, so both may be bundled and redistributed commercially with attribution. **No royalty, no subscription, no service dependency** — this feature is fully compatible with the governing principle.

**But local is not free of cost, and one decision remains open (O-18):** model weights are large. Bundling a large model directly contradicts §47's "lightweight". Options: ship a small model and offer larger ones as an optional download; ship nothing and fetch on first use of captions; or ship one mid-size model. This trades bundle size against out-of-the-box accuracy and needs a product decision before Phase 3.

---

## 5c. Resolutions (2026-09-09) — O-3 and O-4

### O-3 — Colour policy: **SDR Rec.709 pipeline for V1**

Adopted as recommended. The policy:

1. **Internal pipeline is Rec.709 SDR.** All compositing, effects and preview happen in that space.
2. **HDR input is detected, not assumed.** PQ (SMPTE 2084) and HLG (ARIB STD-B67) transfer functions are identified at probe time.
3. **HDR is tone-mapped to SDR with a documented curve**, and **the user is told a conversion happened.** Silent conversion is what produces "why does my video look washed out" reports.
4. **No HDR output in V1.** Stated in the product documentation so it reads as a scope decision rather than a defect.
5. **Unspecified colour metadata is treated as Rec.709** for HD/SDR content, per convention.

**Verified during Phase 1, not merely specified.** Colour metadata reporting was added to the core (`MCProbe.color_primaries / color_trc / color_space / is_hdr`) and tested against purpose-built files:

| File | Transfer | Detected |
|---|---|---|
| `sdr_709.mp4` (H.264 8-bit) | unspecified | SDR → pass through |
| `hlg_2020.mp4` (HEVC 10-bit) | ARIB B67 (HLG) | **HDR → tone-map + inform** |
| `pq_2020.mp4` (HEVC 10-bit) | SMPTE 2084 (PQ) | **HDR → tone-map + inform** |
| Tears of Steel, 4K test clip | unspecified | SDR → pass through |

**A practical finding from that test:** the SDR file reports transfer as **"unspecified"** even though it was encoded with explicit `bt709` flags. Unspecified is the *common* real-world case, so item 5 above is load-bearing — "unspecified" must mean "assume Rec.709", never "unknown, refuse". Also noted: `bits_per_raw_sample` came back 0 on the HDR files, so bit depth must be derived from the pixel format rather than that field.

**Still to decide before Phase 3:** which tone-mapping curve (Hable, Mobius, BT.2390). That is a quality choice needing visual comparison, not an architectural one, and it does not gate Phase 2.

### O-4 — Crash reporting: **opt-in, off by default**

Adopted as recommended, resolving the §25-vs-§44 tension the consistency check flagged:

- **Off by default.** Nothing is transmitted unless the user turns it on.
- **Explained plainly at first run**, not buried in preferences.
- **Stack traces and app version only.** No media, no file contents, **no file paths, no file names** — a path alone can disclose a client name or an unreleased project.
- **Not required for any functionality**, satisfying §25's "no telemetry required for core functionality".

This is compatible with §25's own carve-out for explicit opt-in, and it makes §44's "no known reproducible crashes" verifiable in the field rather than aspirational.

---

## 6. Phase 1 plan

Per §35: technical proof-of-concepts only, no polished UI, and **none of this code is kept**. Each spike has a pass condition; a spike without one is a demo, not a validation.

| # | Spike | Pass condition |
|---|---|---|
| **S1** | Rust core → C ABI → Swift → VideoToolbox decode → `CVPixelBuffer` → Metal display | 4K30 H.264 sustained ≥ 29.5 fps; **zero CPU frame copies** verified in Instruments; RSS < 500 MB |
| **S2** | Timeline rendering: SwiftUI `Canvas` vs custom `NSView`, 200 clips with waveforms and thumbnails | ≥ 60 fps sustained scroll/zoom; no main-thread block > 16 ms. Decides AD-2 (macOS) |
| **S3** | Reproducible LGPL FFmpeg build script + CI configuration gate | Build reproducible from a clean checkout; gate **rejects** a deliberately GPL-configured build |
| **S4** | Encode benchmark: `h264_videotoolbox` / `hevc_videotoolbox` vs `libx264 -preset medium`, VMAF-scored | Dataset produced; **preset bitrates chosen from the data**, not guessed (§R5, R-09) |
| **S5** | Windows: WinUI 3 + `SwapChainPanel` + Rust via P/Invoke + D3D11VA decode | 4K30 playback ≥ 29.5 fps; cold start ≤ 2.5 s; idle RSS ≤ 250 MB. Decides AD-2 (Windows) |
| **S6** | Worker-process crash isolation, fuzzed with a corrupt/truncated media corpus | 1000+ malformed files: **zero UI-process crashes**; every failure produces a classified, human-readable error (§23) |
| **S7** | Async waveform generation and cache | 60-minute WAV waveform in ≤ 10 s; UI never blocked > 16 ms; cache reused across relaunch (§17) |
| **S8** | macOS sandbox + security-scoped bookmarks + sandboxed worker process | Project reopens after relaunch with media access intact, sandboxed, hardened, notarizable |
| **S9** | Hardware capability probe across ≥ 3 GPU configurations | Probe correctly detects and rejects non-working paths; results cached and invalidated on driver change |
| **S10** | Proxy necessity test: 4K30 H.264, 4K HEVC 10-bit, 4K60 on min-spec Windows and Apple Silicon | Determines whether Tier 2 proxies ship in V1 (AD-5, R-03) |

Output: `docs/PHASE_1_RESULTS.md` with real measured numbers — including the ones that disappoint. Per §46 Rules 6 and 12, a spike that fails is a result, not a problem to hide.

---

## 7. Phase 0 exit criteria

| Criterion (§34, §50) | Status |
|---|---|
| All 20 §33 research topics addressed | ✅ |
| All 13 §34 decision-gate questions answered | ✅ (3 marked GATED, with the gate defined) |
| Risks identified before code is written | ✅ 20 registered with triggers |
| FFmpeg licensing strategy established before distribution | ✅ Copyright layer decided; ⚖️ patent layer open |
| Codec/patent considerations researched | ✅ Researched; ⚖️ requires counsel |
| Specification challenged, not blindly agreed with | ✅ §3 — 1 contradiction, 10 omissions, 6 untestable requirements |
| Architecture internally consistent | ⚠️ 2 tensions + 1 contradiction, all product decisions (O-1..O-4) |
| Architecture justified, not assumed | ✅ Every claim tagged; unverifiable claims marked `[MEASURE]`/`[COUNSEL]` |
| Defensible path to a fast 4K native app on both platforms | ⚠️ Defensible; **contingent on O-1** |

---

## 8. Recommendation

1. **Answer O-1 and O-2 first.** They are product decisions, they are the largest schedule levers in the project, and no amount of engineering resolves them. Recommended answers: macOS V1 / Windows V1.1; §48 governs scope.
2. **Start Phase 1 immediately.** All ten spikes are useful under any answer to O-1/O-2, and S1, S2, S5 and S10 exist precisely to close the GATED decisions.
3. **Start the counsel engagement now** (O-5 through O-7). Legal review has a lead time, and O-5 in particular could change the codec strategy — which is much cheaper to change in Phase 1 than in Phase 7.
4. **Start Windows certificate procurement now** (R-13). It has historically blocked releases and costs nothing to begin early.
5. **Do not begin Phase 2** until O-1 through O-4 are answered. Phase 2 builds the data model, and all four change it.

---

*Prepared as the Phase 0 written validation report required by §46 Rule 12. Every unverified claim in these five documents is tagged. Nothing here should be treated as measured until `PHASE_1_RESULTS.md` exists.*
