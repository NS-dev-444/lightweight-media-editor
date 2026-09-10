# Product Direction

**Project:** Lightweight cross-platform video + audio editor
**Document status:** ✅ **RATIFIED 2026-09-09.** Supersedes §31's mode structure and §32's V1 list. O-2 resolved: V1 ships one video track plus one overlay track and two audio tracks.
**Updated 2026-09-09:** O-1 resolved (macOS V1, Windows V1.1) and O-16 resolved (local processing approved). See §2.1 and §11.
**Date:** 2026-09-09
**Relationship to the spec:** This document proposes replacing §32's V1 MUST-HAVE list and §31's four-mode structure. Everywhere else, `LIGHTWEIGHT_MEDIA_EDITOR_MASTER_SPEC.md` remains the source of truth, and every engineering decision in `ARCHITECTURE_DECISION.md` still holds.

> §46 Rule 1 says do not build features that are not approved. This document is a **proposal for approval**, not an approval. Nothing here is buildable until §11's open questions are answered and the cut list in §5 is signed off.

---

## 1. The problem with the spec's positioning

The master spec is a strong engineering document that never answers one question: *why would someone open this instead of what they already have?*

Answered honestly today, they wouldn't. The spec's V1 — trim, split, text overlay, export, convert — describes a category where the free tools are strongest:

| What we'd be offering | What it competes with, free |
|---|---|
| Lightweight timeline editing | CapCut, iMovie, DaVinci Resolve |
| Video conversion | HandBrake |
| Audio editing (§3 as written) | Audacity |

"A lightweight editor with the basics" is the most crowded category in consumer software, and the basics are exactly where free products are hardest to beat. §48's three success workflows describe a *toolbox*, not a job anyone actually has.

We should not fight there.

---

## 2. Positioning

> **A fast local utility belt for people who publish video.**
> The thing you reach for between recording and posting.

The heavyweights are bad at small annoying jobs. The free web tools want your files uploaded and your email address. That gap is real, it is under-served, and it is defensible by a small team.

### 2.1 The governing principle

Stated by the product owner, and clearer than §32's wording:

> **No recurring cost. No dependency on an external service.**
> Anything that runs locally on the user's machine is acceptable.

This is the rule that decides feature questions, and it supersedes §32's "no AI" phrasing. §32's real targets — "Cloud editing", "Subscription-required processing", "Stock media" — are all instances of it. Local computation was never the objection.

Two consequences:

1. **Captions are approved** (O-16). Local transcription costs nothing recurring, calls no service, and is MIT-licensed on both the runtime and the model weights — verified, not assumed.
2. **The business model follows from this, not the other way round.** §9's "no subscription" is not a marketing preference; it is the same principle applied to our own pricing. A product that refuses to depend on subscriptions cannot credibly sell one.

### The wedge is already in the spec — it's just filed in the wrong place

§25 files "no cloud, no account, no telemetry, no upload" under **Privacy**, as if it were a compliance checkbox. It is the entire marketing position.

Anyone editing client work under NDA, anything unreleased, anything with a face in it — they *cannot* use the browser-based tools, and they know it. "Your files never leave your machine" is a stronger sales line in 2026 than any feature in §32.

**Lead with it.** It also happens to be free to deliver, because the architecture is local-first anyway.

---

## 3. The three workflows (replacing §48)

§48's workflows are "edit a video / edit audio / convert files." These replace them:

**W1 — The creator loop.**
Long recording → cut the dead air → captions → export for three platforms.
*The actual end-to-end job. Nothing lightweight does the whole thing well.*

**W2 — The one-off fix.**
Drop a file → "make this under 25 MB" or "make this 9:16" → done.
*No timeline, no project, no save dialog. Under thirty seconds.*

**W3 — The bulk job.**
Drop 20 files → pick a format → done.
*Boring, and the thing people literally type into search bars.*

If these three are excellent, the product is successful. This supersedes §48 as the definition of done.

---

## 4. Product structure: three modes, not four

§31 proposes EDIT / AUDIO / CONVERT / TOOLS. **Proposed: three modes.**

| Mode | Contains | Notes |
|---|---|---|
| **EDIT** | Video and audio together. Music as a first-class ingredient. Ducking, fit-to-length, loudness, voiceover, silence removal, captions, text, overlays. | W1 lives here |
| **CONVERT** | Batch conversion, video and audio, standalone. | W3. Free forever — see §9 |
| **TOOLS** | Extract audio, compress, resize, merge, change format. | W2. Presets over the converter engine, **not separate code** |

**The separate AUDIO mode is removed.** Not because audio is unimportant — see §5 — but because giving it its own mode is precisely what pushed §3 toward rebuilding Audacity. Its genuinely useful parts move into EDIT, where the work actually happens.

---

## 5. Audio: ingredient, not destination

This is the central correction to §3, and it comes from the intended use — music prepared *for the video you are already editing*.

| Audio as **destination** | Audio as **ingredient** |
|---|---|
| Master a podcast, repair a recording, export MP3s as the deliverable | Get this track under my video at the right length and the right volume |
| Audacity's job. **Out of scope.** | **The product's job. In scope.** |

### The conversion problem largely dissolves

"Convert music to use in videos" mostly stops being a conversion problem once the editor accepts everything natively. FFmpeg does not care whether the file is FLAC, M4A, OGG, WAV or MP3 — it decodes, you drop it on the timeline, it works. No convert step, no intermediate file, no round trip.

Audio conversion only matters when audio is an **output** — extracting a video's audio as MP3, or converting a folder of FLACs. That is CONVERT mode, which stays.

**The one-stop shop is achieved mainly by removing a step, not by adding a feature.**

### What music-under-video actually needs

The spec wrote several of these as *audio-editor* features when they are really *video-editing* features:

1. **Ducking** — music drops automatically under speech. Absent from the spec entirely, and it is the highest-value audio feature in the product. Every creator does this by hand with volume keyframes and hates it.
2. **Fit music to video length** — trim to end cleanly on the last frame, or loop-and-fade to fill. Music is never the length of your video, and nothing lightweight handles it well.
3. **Loudness targeting** — §3's "Normalize", reframed: not "normalize this file" but "make this music sit correctly under my voice." Same EBU R128 engine (AD-7), completely different framing in the UI.
4. **Voiceover recording straight to the timeline** — creators do this constantly; not in the spec. Audio device I/O already exists for playback, so the marginal cost is small.
5. **Silence removal** — belongs to **video**, not audio. It is "cut the dead air out of my 40-minute recording," which is the first half of W1.

---

## 6. V1 scope (supersedes §32)

### IN

**Editing**
Import, drag/drop, media library · timeline with **one video track + one overlay track + two audio tracks** · trim, split, cut, delete, reorder · crop, rotate, flip · speed · **delete-range as the primary gesture** · undo/redo · project save/load, autosave, recovery

**Audio (as ingredient)**
Waveform on the timeline · volume, fade in/out, mute · **ducking** · **fit-to-length** · **loudness targeting** · **voiceover recording** · **silence removal** · detach / replace audio

**Text and overlays**
**Text presets that look finished untouched** · font, size, weight, tracking, leading, alignment, position, opacity · outline and shadow · **platform safe-area guides** · image overlay (PNG with transparency, JPEG)

**Publishing**
**Captions** (see §8) · **reframe to aspect ratio** · **target-size export** · export presets **named by destination** · advanced panel for codec/bitrate/frame rate · hardware acceleration

**Utility**
CONVERT: batch video and audio, queue, progress, cancel, retry, error reporting · TOOLS: extract audio, compress, resize, merge, change format

### OUT of V1 — cut from §2, §3 and §32

| Cut | Why |
|---|---|
| Multiple video tracks | §32 already omits them; one video + one overlay covers text and logos. Resolves the §2/§32 contradiction |
| Transitions | Not in §32's V1 list. Expensive; low value for W1 |
| EQ, compressor, noise reduction | Audacity's job. §18 gated them on "without bloat", which is untestable |
| Pitch adjustment | Rubber Band is GPL; SoundTouch is lower quality. §3 already hedged |
| Separate AUDIO mode | See §4 |
| Freeze frame | Not in §32's V1 list |
| Standalone audio-editor export flows | Covered by CONVERT |

That is roughly 40% of the proposed V1 scope. Removing it makes the product **better**, not merely smaller — every item is something a mature free tool already does properly.

### ADDED — not in the spec at all

Captions · reframe to aspect ratio · target-size export · ducking · fit-to-length · voiceover recording · real typographic controls · safe-area guides · LUT support · destination-named presets

**Net effect: V1 is smaller than §32 and sharper.**

---

## 7. Design principles

### Typography is the difference between amateur and professional

§19's text tool — font, size, weight, alignment, position, rotation, opacity, "simple background" — produces the exact look that says *made in a free editor*: Arial Bold, white, dead centre, no padding. Bad text is the number one tell of an amateur edit, and it is the cheapest thing in the entire spec to get right.

- **Presets are finished looks, not fonts.** Padding, weight and contrast already solved. The default must be good enough to ship untouched.
- **Tracking and line height.** Missing from §19; they are most of what separates good type from bad.
- **Outline and shadow.** Text sits over moving footage; white-on-anything is unreadable half the time.
- **Safe-area guides.** Show where the Reels or Shorts UI will cover the caption. Costs almost nothing; no lightweight tool does it.

### The rest

- **Export presets are named by destination** — "Instagram Reel", "YouTube 4K" — never by resolution. Users think in destinations.
- **The primary timeline gesture is "delete this range"**, not "arrange these clips." Arranging is a professional metaphor; cutting the boring parts out of one long take is the actual job.
- **LUTs instead of eight sliders.** §18's brightness/contrast/saturation is the 1998 version of colour. One good LUT beats twenty minutes of slider-nudging, and creators already own LUTs. Keep the sliders, but LUTs are the headline.
- **Always show the output path before running anything.**
- **Usable with a trackpad and no manual.** This is §44's UX criterion, promoted to a design rule.

---

## 8. Implementation notes that protect the cut list

These exist so the added features do not quietly re-import the ones we cut.

**Ducking — do not build a sidechain compressor.** Analyse the speech track's envelope offline, generate a volume automation curve on the music clip, let the user drag the resulting points. Simpler to build, and better UX: visible, editable, undoable — which a live compressor is not. This is how we get the feature *without* the compressor we cut in §6.

**Ducking and silence removal share one analysis pass.** Both need a speech-envelope detector over the audio track. One piece of DSP, two headline features. Build it once, well.

**Reframe is a crop rectangle, optionally keyframed.** Not object tracking — §32 rightly forbids motion tracking, and manual reframing is fine for the 15-second clips this targets.

**Target-size export is a two-pass bitrate calculation.** Standard, well-understood, no new machinery.

**Captions run locally.** ✅ **Approved (O-16).** Whisper-class transcription on-device: no network at runtime, no account, fast on Apple Silicon. It fits §25 exactly and is the single most-wanted feature by anyone publishing video — which makes §32's deferral of subtitles to V1.1 the most costly deferral in the spec.

Licensing is verified and unusually clean: `whisper.cpp` is MIT ("Copyright (c) 2023-2026 The ggml authors") and OpenAI releases Whisper's **code and model weights** under MIT ("Copyright (c) 2022 OpenAI"). Weights being MIT is rare and worth stating — it means they can be redistributed inside the product. No royalty, no service, no subscription.

✅ **O-18 RESOLVED, 2026-09-10.** `ggml-base-q5_1` — 57 MB, multilingual, quantised — is **bundled**, not downloaded.

The size is the argument: the app plus its FFmpeg libraries is around 60 MB, so 57 MB roughly doubles it and anything larger would contradict the word "lightweight". `base-q5_1` is the largest model that stays on the right side of that line. **Multilingual rather than `base.en`**, because English-only is marginally better on English and useless on everything else.

**Bundling rather than downloading is a departure from the preferred shape above, and a deliberate one.** Shipping the model means captions work on first launch with no network at all — so the first-run network requirement this paragraph warned "must be stated, not discovered" simply does not exist. Larger models (`small`, 181 MB; `large-v3-turbo`, 547 MB) remain an explicit opt-in for people who ask for the accuracy.

Measured on the machine this was built on: 7.6 s of speech transcribed in 0.4 s, word-perfect. `RISK_REGISTER.md` R-22 is retired.

---

## 9. Business model

- **CONVERT and TOOLS: free, permanently.** They are the acquisition channel — the thing people search for — and they cost nothing we were not already building for W1.
- **EDIT and captions: one-time purchase**, in the region of $40–60.
- **No subscription.** A subscription directly contradicts the local-first pitch that is the whole wedge.

### One thread back to Phase 0

AVC and HEVC royalties attach **per copy distributed** (`DEPENDENCY_AND_LICENSE_AUDIT.md` §6). A free tier that reaches a hundred thousand people has a very different royalty shape from a paid app that sells five thousand — and the free tier is the one that decodes H.264 all day.

**Model this before pricing is set, not after.** It may argue for a download-gated free tier, or for making AV1/Opus the default in the free path. It is a business decision with an engineering input, and it is currently unowned.

---

## 10. What this changes in the architecture

**Almost nothing — and that is the point.**

| Area | Impact |
|---|---|
| AD-1 Rust core scope | No change |
| AD-3 FFmpeg, LGPL, worker process | No change |
| AD-4 Metal / D3D compositors | No change |
| AD-7 audio engine | **No change.** One FFmpeg stack, f32 planar, fixed project rate, R128 loudness — all of it holds. What changes is UI framing, not the engine |
| AD-9 project format | Minor: caption tracks and automation curves become schema entries. Cheaper to add now than after v1 of the schema ships |
| §31 mode structure | **Changed** — four modes become three |
| §32 V1 list | **Superseded** by §6 |
| Phase 2 and Phase 4 scope | **Reduced.** The cut list removes more than the added features put back |

The Phase 0 engineering work does not need redoing. This document re-aims the product, not the architecture — which is the correct order to discover things in, and the reason Phase 0 came first.

---

## 11. What this does not answer

**Resolved 2026-09-09:**

- ✅ **O-1** — **macOS V1, Windows V1.1.** The Rust core is still written cross-platform from the first commit and a Windows build stays green in CI, so the port stays a port (`RISK_REGISTER.md` R-21).
- ✅ **O-16** — **Local processing approved**, under the principle in §2.1. Captions are in.

**Still open:**

- **O-3** — What is the V1 colour policy? *(LUT support in §7 makes this more urgent, not less.)*
- **O-4** — Opt-in crash reporting: yes or no?
- **O-17** — Free-tier distribution volume versus per-copy codec royalties. Blocks pricing.
- **O-18** — Transcription model size and download strategy. Blocks caption UX design (R-22).

**Partially answered:** O-2 (scope, and multi-track video) — resolved by §6, pending sign-off.

---

## 12. Summary

Keep the architecture — it is sound, and this document does not disturb it.

Re-aim the product at the annoying jobs between recording and posting. Ship one app where music is an ingredient rather than a second application. Be known for **captions and typography** — the two things that make published video look professional, and the two things the spec either deferred or under-specified.

Cut 40% of V1 and add the six features nobody lightweight does well.
