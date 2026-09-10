# State of play — 2026-09-10 (end of day)

A single page to pick up from. Details live in the per-phase documents.

## Where things stand

| Phase | Status |
|---|---|
| **0** — Architecture research | ✅ Complete. 5 documents, 20 risks, spec challenged. |
| **1** — Technical prototype | ✅ Complete for macOS. 9 spikes passed; S5/S9 need Windows. |
| **2** — Project & timeline engine | ✅ Complete. |
| **3** — macOS application | ✅ Timeline, preview, playback, autosave, export. |
| **4** — Rest of the V1 feature set | ✅ Complete (`PHASE_4_EDITING.md`). |
| **6** — Converter & tools | ✅ Complete except MP4 → WebM (`PHASE_6_PROGRESS.md`). |
| **7** — Captions | ✅ Complete. On-device, bundled model (`PHASE_7_CAPTIONS.md`). |
| **5** — Windows | 🔵 V1.1 per O-1. `WINDOWS_BRINGUP.md` is ready to work through. |

**The ratified V1 feature set is complete.** Every line of
`PRODUCT_DIRECTION.md` §6 now has code behind it.

**A correction carried forward.** An earlier version of this page said "Phase 3
— V1 feature set complete". That was wrong: Phase 3 completed the timeline and
export half, and auditing §6 line by line found thirteen ratified items with no
code behind them. Phase 4 closed that, Phase 7 closed captions.

## Where the code lives

`github.com/NS-dev-444/lightweight-media-editor` — **private**. Build output,
the app bundle, the caption model and the generated fixtures are all excluded;
what is committed is 120 files of source, scripts and documentation.

CI runs `tools/check.sh` on every push and is **green** — 13 checks on a clean
runner, building FFmpeg, whisper.cpp and the model from nothing in about eleven
minutes (`CI.md`).

## The app today

```bash
app/macos/build_app.sh && open app/macos/Editor.app
```

**Edit** — import (including from Finder), trim, split, ripple, delete-range ·
crop, rotate, flip, reframe to 9:16 / 1:1 / 4:5 · LUTs with an amount slider ·
titles with presets, image overlays, safe-area guides · volume in dB, fades,
detach audio, match loudness, remove silence, duck under voice, fit music to the
video, record a voiceover · **captions: transcribe on-device, correct them in a
list, burn in or export as SRT/WebVTT** · playback with sound · autosave and
recovery · export by destination, including a size-targeted preset that says
when it cannot keep its promise.

**Convert** — batch remux and transcode, video and audio, with a queue that
survives quitting.

**Tools** — extract audio, change format, resize, compress, extract frames,
merge.

## Health

```
150 Rust tests           0 failures
468 Swift checks         0 failures
0 compiler warnings      (§46 Rule 4, both languages, whisper on AND off)
0 unwrap/expect/panic!   in non-test library code (§22)
licence gate passes      on the shipped FFmpeg build, rejects a GPL one (R-05)
mediacore-model checks   for x86_64-pc-windows-msvc (R-21)
loudness agrees with     FFmpeg's ebur128 to 0.04 LU
transcription is         100% word-accurate on the known-speech fixture
every binary targets      macOS 14.0, asserted against the plist (R-27)
attribution ships         for all 16 components, and the build fails without it
```

Run them all with **one command**, which is also the only thing CI runs:

```bash
tools/check.sh            # stops at the first failure
tools/check.sh --all      # runs everything, reports at the end
```

Individually, when you want just one:

```bash
tools/ct.sh test            # Rust, with the FFmpeg environment set
tools/swift_selftest.sh     # the app layer's pure logic
tools/check_loudness.sh media/*.wav
python3 tools/license_gate.py --prefix build/ffmpeg-lgpl
```

First build on a fresh checkout also needs:

```bash
third_party/ffmpeg/build.sh     # LGPL FFmpeg
third_party/whisper/build.sh    # whisper.cpp (needs cmake)
tools/fetch_models.sh           # the 57 MB caption model
tools/prep_corpus.sh            # test fixtures
```

## Pick up here

1. **Windows bring-up** when the machine is ready. `WINDOWS_BRINGUP.md` is
   ordered so nothing blocks, and only `mediacore-media` needs the box.
2. **MP4 → WebM**, which needs libvpx in the FFmpeg build and an AD-12
   amendment (VP8/VP9 are royalty-free, so the hardware-only rationale does not
   reach them).
3. **Merge keeps no audio on its re-encode path** — it should resample and mix.
4. **No app icon, no installer, no first-run experience.** None started.

## Scope decision: personal use — 2026-09-10

**The product is not being distributed.** That is a decision, not a delay, and it
retires most of what was outstanding — because nearly every open legal item
attaches to *distribution*, not to building or using:

| Was blocking | Now |
|---|---|
| O-5, O-6 — AVC/HEVC royalties | **Dormant.** Royalties attach per copy distributed. None are. |
| O-7 — LGPL compliance review | **Dormant.** LGPL obligations attach on distribution. |
| O-17 — free-tier volume vs royalties | **Dormant.** It was a pricing input; there is no pricing. |
| Mac App Store vs LGPL | **Moot.** No channel, no conflict. |
| Notarization | **Optional.** Gatekeeper matters for *other people's* Macs. |

**Dormant, not deleted.** If this is ever distributed, every one of them returns
exactly as written, and the work already done is what makes that cheap: the
FFmpeg build is LGPL-clean and gated, the attributions are generated and
verified, and the app is Developer ID signed with the hardened runtime on. The
expensive part of compliance is the part that has to be designed in, and it was.

## Decisions still yours

| # | Question |
|---|---|
| — | **Nothing is blocking.** Everything above is dormant while this stays personal. |
| O-18 | ✅ Resolved — `ggml-base-q5_1`, 57 MB, bundled. |

The one thing only you can do is **test voiceover recording** (⌘R). It is the
single feature written but never exercised, because trying it means recording
from your microphone.

## What measurement changed

Each of these overturned something that had been written down as true:

- The timeline did **not** need AppKit — `Canvas` is immediate-mode, so the
  per-row cost that makes SwiftUI lists stutter never applied.
- Hardware encoders need **2.31× (H.264) / 1.74× (HEVC)** libx264's bitrate. The
  first measurement said 1.38×/1.21× and was flattering them.
- An **LGPL-only FFmpeg costs no user-visible capability** — every format in
  §2/§3/§4 is covered, and WebM→MP4 now works through the transcode path.
- Property testing found **four** real bugs no hand-written test caught, all of
  which corrupted documents silently rather than erroring.
- The "bare path stops the window appearing" bug was **AppKit, not the app**,
  and **not user-facing** — Finder, the Dock and `open` all work. Six candidate
  fixes were tried and measured; none of them work, because the decision is made
  before any of them run.
- A **Swift/Metal struct layout mismatch** had been quietly corrupting blur and
  sharpen since they were written. It only surfaced because rotation put a
  visible symptom on the same bug.
- `ripple_delete_range` **discarded the tail** of any clip it cut into — the
  operation PRODUCT_DIRECTION §7 calls the primary gesture. A comment described
  the behaviour as deliberate, which is how it survived.
- Transcription **lost a whole sentence at every chunk boundary**, and the
  word-accuracy average was fine while it did. Only a fixture of *numbered*
  sentences made the loss visible.
- The app **claimed macOS 14 and was built for macOS 26** — every binary in the
  bundle. It would have launched on nothing older than this machine. Each piece
  was individually correct; only the combination was wrong (R-27).
- Re-running the FFmpeg build with a new floor left a prefix where
  `libavcodec` was 14.0 and `libavutil` was 26.0, because **`make` relinks only
  what changed**. Half-fixed is a state a build can be in.
