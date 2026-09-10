# Phase 4 — the rest of the V1 editing feature set

**Status:** COMPLETE except captions. Started and finished 2026-09-10.
**Scope:** the parts of `PRODUCT_DIRECTION.md` §6's ratified V1 list that Phase 3
left unbuilt — framing, colour looks, and audio as an ingredient.

`STATE_OF_PLAY.md` used to say "Phase 3 — V1 feature set complete". That was
wrong, and worth recording as wrong: Phase 3 completed the *timeline and export*
half of V1. Auditing §6 line by line found thirteen ratified items with no code
behind them. This is what closed that gap.

---

## 1. Framing: crop, rotate, flip, reframe

**Model** (`Geometry` in `timeline.rs`). The crop is stored in **normalised
source coordinates, 0..1, not pixels.** That is the difference between a crop
that survives everything and one that does not: the same clip may be relinked to
a different resolution (§25), swapped for a proxy, or exported at a size other
than the preview's, and a crop in pixels means something different in each case.
Normalised, "the middle half" stays the middle half.

Rotation is in **quarter turns**, because that is what the operation actually
is. Arbitrary angles need a resampling filter, a decision about what fills the
corners, and a UI for the angle — none of which V1 has, and a quarter turn is
what a sideways phone video needs.

**Reframe** (`Geometry::fill_aspect`) takes the largest centred rectangle of a
target shape. Centred because a centre crop is right often enough to be the
default, and the crop sliders are there when it is not. Timeline ▸ Reframe All
Clips applies it to everything at once, which is the version people want: "make
the whole thing vertical", not clip by clip.

### The bug this uncovered

The first rotated export came back **pillarboxed to 9:16 with the picture still
horizontal** — the right shape, the wrong content, squeezed to fit.

The cause was not the rotation maths. It was a **struct layout mismatch between
Swift and Metal**: Metal aligns `float2` to 8 bytes and `float4` to 16 inside a
constant buffer, while a Swift struct of `Float`s packs at 4. A `float2` after
an odd number of floats reads from four bytes further along than Swift wrote.

It was already happening before this change. The existing `float2 texel` sat
after thirteen floats, so **blur and sharpen had been sampling with the wrong
step the whole time** — silently, because every value involved is plausible.

Both shader structs are now **scalars only**, in the same order as their Swift
counterparts, which makes the two layouts identical by construction rather than
by inspection.

---

## 2. Colour: LUTs

PRODUCT_DIRECTION §7 calls this the headline: "one good LUT beats twenty minutes
of slider-nudging, and creators already own LUTs." So the Look section sits
**above** the sliders in the inspector — the order of the inspector is an
argument about how to grade, and this makes it.

- `.cube` files are parsed in Swift (`CubeLUT`), forgiving about whitespace,
  comments, key case and encoding — real files come from a dozen tools — and
  strict about the data, because a table with the wrong number of entries would
  sample garbage rather than fail.
- Uploaded as a **3D texture** so the hardware's own trilinear filter does the
  interpolation. A 33-cube interpolated by hand is far more code and slower.
- Applied **last**, on the finished picture. A LUT is a look, and a look goes on
  a graded image; sampling before the sliders would have them fighting it.
- Only the **path** is stored in the project. A .cube is often several megabytes
  and a project file that swallowed one would be a project nobody could email.
- The **amount** slider is what makes a LUT usable rather than a switch. Most
  looks are too strong at 100 %, and sitting at 60 % is the difference between a
  grade and a filter.

Verified by exporting the same clip with and without a deliberately extreme test
LUT and comparing extracted frames: warmer highlights, cooler shadows, lifted
blacks, exactly as the table specifies.

---

## 3. Sound as an ingredient

### Loudness — written out rather than borrowed

EBU R128 is precisely specified, so `analysis.rs` implements it directly: the
K-weighting filter, 400 ms blocks overlapping by 75 %, an absolute gate at
-70 LUFS and a relative gate 10 LU below the ungated mean. About a hundred
lines, no dependency question, and — the deciding point — **checkable against an
independent implementation.**

`tools/check_loudness.sh` does exactly that, against FFmpeg's `ebur128`:

| File | Ours | FFmpeg | Difference |
|---|---|---|---|
| av_sync.mp4 | -21.76 | -21.8 | 0.04 |
| test_60min.mp3 | -22.34 | -22.3 | 0.04 |
| av_pcm.mkv | -21.76 | -21.8 | 0.04 |
| tone_-20dbfs.wav | -19.99 | -20.0 | 0.01 |

The tone is also the standard's own calibration point — a 1 kHz stereo sine of
amplitude 0.1 is *defined* to read -20 LUFS. Worth noting that the naive
prediction, `-0.691 + 20*log10(0.1) = -20.69`, is **wrong**: the -0.691 offset
exists precisely to cancel the K-filter's gain at 1 kHz. Both implementations
agreeing on -20.0 is the arithmetic being right, not two bugs agreeing.

"Match Loudness" targets **-14 LUFS**, which is what the streaming platforms
normalise to, so hitting it means they leave the audio alone. It **refuses to
clip**: raising a quiet-but-peaky recording to -14 can push samples past full
scale, and distortion is a worse outcome than being slightly quiet, so it stops
at the peak and says so.

### Silence removal — and the bug it found

Detection is windowed RMS over 20 ms, with a threshold, a minimum length and
padding — three parameters because silence is not one thing. Room tone in a
bedroom sits far higher than in a booth; a minimum length stops every breath
becoming a cut; padding keeps a cut from landing exactly on the first syllable.

Running it end to end turned a four-second file with a two-second gap into a
**1.1-second** one. The cause was in `ripple_delete_range`, the operation
PRODUCT_DIRECTION §7 calls the primary gesture: when a range fell **strictly
inside** a clip it kept the head and **threw the tail away**. A comment even
described this as intended — "splitting into head+tail is a separate, explicit
operation" — which made it a design decision that was simply wrong for the
operation the function exists to serve. Cutting the dead air out of one long
take must close the gap, not truncate the take.

It now splits properly, carries the fades to the right ends, and undoes as one
action. Two tests pin it: one on the geometry, one on exact undo.

### Ducking — and why the model grew a curve

Ducking is a *changing* gain by definition, so it cannot be one number. Clips
gained a **gain envelope** — the automation-curve schema entry AD-9 anticipated
— evaluated in `plan_at` and combined there with the clip's level and its fades.

Because the mixer consumes the plan's gain per block, **ducking needed no mixer
change at all.** That is AD-4 paying for itself: the core decided what the level
is, the platform applied it.

The passages are derived from where the voice *actually speaks* — the complement
of the silence the analysis finds — so the curve follows the performance rather
than a fixed pattern, and ramps in and out so it does not pump.

Verified by reading the gain the **render plan** produces, not the curve:
the plan is what the mixer and exporter consume, so a curve that is right but is
not reaching them would pass a test on `gain_points` and still be silent.

### Detach, fit-to-length, fades, voiceover

- **Detach audio** moves a clip's sound to its own track and silences the
  picture clip, as **one** undoable edit — undoing "detach" must not leave a
  silenced video behind.
- **Fit to length** trims music to the video and fades it out. Trimming rather
  than time-stretching: speeding music up changes its pitch and tempo.
- **Volume in decibels, not percent.** Doubling a percentage does not double
  what you hear.
- **Voiceover** records the default input to a WAV beside the project, asks for
  permission the first time you record rather than at launch, and shows a level
  meter — a take with no level showing is a take you find out was silent
  afterwards.

---

## 4. Publishing

**Export presets are named by destination now** — "YouTube 4K", "Reels, TikTok,
Shorts", "Email or Messaging" — never by resolution, per PRODUCT_DIRECTION §7.
The numbers are still there, in the tooltip.

**Target-size export.** "Email or Messaging" solves for a bitrate that lands
under 25 MB, with 6 % headroom for container overhead and encoder overshoot: a
file that promised under 25 MB and came out at 25.4 has failed at the only thing
it promised.

It also **says when it cannot keep the promise.** Past about seven minutes the
solved bitrate hits the quality floor and the file will exceed the target; an
alert says so, with the estimate, *before* the export runs rather than after.
The exact boundary is measured by the Swift self-test, not asserted: it fits up
to ~425 s and warns from ~430 s.

**Safe-area guides** show where Reels, TikTok and Shorts put their own
interface. Dimmed rather than outlined, because the question is "what gets
covered" and a dimmed region answers it at a glance. The numbers are
deliberately generous and the guide says which shape it assumes — every platform
moves its UI between releases, so a guide claiming pixel accuracy would be
lying.

It earned its place immediately: the first screenshot showed the **default title
position sitting inside the region TikTok's caption covers.**

---

## 5. A Swift test target, and why there was none

The macOS app is built by a direct `swiftc` invocation rather than SwiftPM (see
`build_app.sh`), so there was no XCTest bundle to hang tests on — and therefore
no tests at all on the app layer.

`tools/swift_selftest/` is the smallest thing that works: a plain executable
compiling the dependency-free Swift sources alongside assertions. Only files
importing nothing beyond Foundation can be tested this way, and **that
constraint is a feature** — it is what moved export bitrate arithmetic out of a
Metal-importing file and into `ExportPreset.swift`.

468 checks today, across export presets and the .cube parser.

They are now run by `tools/check.sh` along with everything else — see
`CI.md`.

---

## What is left

**Captions** are the only ratified V1 item still unbuilt, and they are blocked
on **O-17/O-18** — which transcription model, and how it is downloaded. That is
a decision with cost and privacy consequences, and it is the user's.
