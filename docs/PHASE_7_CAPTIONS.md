# Phase 7 — Captions

**Status:** COMPLETE. 2026-09-10.
**Scope:** §8, and the last unbuilt item on `PRODUCT_DIRECTION.md` §6's ratified
V1 list.

PRODUCT_DIRECTION calls captions "the single most-wanted feature by anyone
publishing video", and §32's deferral of them "the most costly deferral in the
spec". This closes it.

---

## O-18, resolved

The open question was model size and download strategy. The answer:

> **`ggml-base-q5_1` — 57 MB, multilingual, quantised — bundled in the app.**

**The size is the argument.** The app plus its FFmpeg libraries is around 60 MB,
so 57 MB roughly doubles it, and anything larger would contradict the word
"lightweight" that is the product's whole pitch. `base-q5_1` is the largest
model that stays on the right side of that line.

**Multilingual, not `base.en`.** English-only is marginally more accurate on
English and completely useless on everything else. A caption feature that works
in one language is not a caption feature.

**Bundled rather than downloaded** — a deliberate departure from §8's "preferred
shape". Shipping the model means captions work on first launch with no network
at all, so the first-run network requirement that §8 said "must be stated, not
discovered" simply does not exist. Larger models (`small`, 181 MB;
`large-v3-turbo`, 547 MB) stay an explicit opt-in.

`RISK_REGISTER.md` R-22 is retired; its trigger stays live in case a larger
model is ever made the default.

---

## Why whisper.cpp and not the system recogniser

macOS has `SFSpeechRecognizer` and it needs no model shipped, which is a real
advantage. It was rejected for three reasons, in order of weight:

1. **It can silently go to Apple's servers.** On-device recognition is a flag
   (`requiresOnDeviceRecognition`) that has to be set and can be forgotten. With
   whisper.cpp there is no network code in the path at all, so "the audio never
   leaves this machine" is a property of the architecture rather than a promise
   about a boolean somebody has to remember to set.
2. **It is macOS-only.** Windows is V1.1 (O-1), and this is one implementation
   rather than two.
3. **It needs a permission prompt** to transcribe a file the user already has.

Reason 1 is the one that decided it. The user's question was whether offline
transcription has a privacy problem; it does not, and the reason it does not is
that there is nothing to switch off.

---

## What was built

**Model** (`mediacore-model/src/captions.rs`). A caption is a span of timeline
time with words on it — deliberately **not** a clip:

- a talk carries hundreds, and hundreds of clips would make the timeline
  unusable for the editing the timeline is for;
- they are exported as a **sidecar** as often as they are burned in, and a
  sidecar is a property of the document, not of a track;
- they are edited as *text*, in a list, which is how anyone who has corrected a
  transcript expects to work.

**They live on the `Timeline`, not the `Project`** — and the reason is the edit
funnel. Every edit goes through `&mut Timeline`, and so do undo, redo and
crash-recovery replay. Captions anywhere else would need a second path through
all three, and a transcript is expensive enough to regenerate that it must be
journalled like everything else.

**Burning in reuses the text pipeline exactly.** A burned-in caption becomes an
ordinary text layer in the render plan, drawn with a `TextPreset` — `Subtitle`
exists for precisely this and is outlined for legibility over any footage. There
is no second way to draw words on a frame.

**SRT and WebVTT, both directions.** Writing is how captions leave the product.
**Reading matters just as much**: plenty of people already have a transcript,
and making them re-transcribe it would be absurd. It also means the entire
caption feature is testable and usable without the speech engine at all.

---

## Two bugs the tests caught

### Content lost at every chunk boundary

Transcription hands the model 25 seconds at a time. The first implementation
overlapped chunks by a fixed two seconds and dropped any segment starting inside
the overlap.

Whisper's segment boundaries do not line up with the chunk's. A segment that
began just inside the overlap and ran well past it was **discarded whole** — a
60-second recording lost "sentence number six" entirely, silently.

It now **resumes at the last complete sentence** instead: the next chunk starts
on a clean boundary and nothing is dropped. Forward progress is enforced
explicitly, because a chunk whose last segment ends where the chunk began would
loop for ever.

The fixture that caught it is 60 seconds of *numbered* sentences. A missing
number is a missing sentence, and no amount of word-accuracy averaging would
have shown it — the average was fine.

### The next cue's sequence number eaten as text

A caption file missing its blank line between cues parsed "One" as `"One\n2"`.
Real files omit that separator constantly. Fixed by dropping a trailing
all-digits line when a cue is ended by the *next cue's timing line* rather than
by a blank one — a heuristic, and documented as one.

---

## Verification

The fixture is synthesised with macOS `say`, which matters twice: the expected
transcript is not a judgement call, so **word accuracy is a number** rather than
an opinion; and nothing anybody said was recorded to make it.

| Check | Result |
|---|---|
| Word accuracy on known speech | 100 % (threshold: 90 %) |
| Speed | 7.6 s of audio in 0.4 s — about 19× real time, Metal |
| 60 s recording, three chunks | all twelve sentences present, no gaps |
| Timings | continuous to 60.28 s of a 60.33 s file |
| Silence | produces no captions, and no `[BLANK_AUDIO]` token |
| Burn-in | verified in exported **pixels** at three timestamps |
| Sidecar | SRT and WebVTT round-trip, including hour-long timestamps |
| Undo | a whole transcript is one undo |
| Save/reopen | captions, burn-in flag and preset all survive |

22 caption tests, 7 transcription tests.

---

## What is deliberately not here

- **Per-word timing.** Whisper can produce it with DTW, at a cost in speed and
  complexity. Caption-level timing is what SRT and WebVTT carry and what
  platforms consume; karaoke-style highlighting is a V1.1 question.
- **Transcribing the whole timeline.** One clip at a time. Transcription is
  expensive, the thing people caption is a take rather than an assembly, and
  doing the timeline would mean rendering a mixdown first — a different feature.
- **Translation.** The plumbing is there (`translate` in the C ABI) but no UI
  exposes it. Translating and transcribing are different promises and should not
  share a button.
