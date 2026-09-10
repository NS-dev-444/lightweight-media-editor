# Phase 6 — Converter and Tools

**Status:** IN PROGRESS — started 2026-09-10
**Scope:** §4 (media converter), §21 (conversion queue), §31 (CONVERT and TOOLS modes).
**Why it matters:** this is §48's third success workflow, and PRODUCT_DIRECTION.md §9 makes CONVERT and TOOLS **free forever** — they are the acquisition channel and what people actually search for, so they have to be genuinely good rather than a token feature.

---

## What works

### Remux — the fast path

MKV→MP4, MP4→MOV and similar are done by **copying streams, not re-encoding**: near-instant and lossless. This is the "smart export" idea PRODUCT_DIRECTION §3.4 flagged as missing from §12, and it is the most common conversion people actually ask for.

The UI labels these **"instant"** before you start, because "this will be immediate and lossless" is a materially different promise from "this will take a while".

Verified: MKV→MP4 (221 packets, video + audio preserved, 3.0 s), MP4→MOV (150 packets, 6.25 s).

`can_remux` is deliberately conservative — a wrong "yes" produces a file that looks fine and will not play, which is far worse than an unnecessary transcode.

### Audio transcoding — §4's listed targets

All verified end to end, with content preserved:

| Conversion | Result | Mean volume |
|---|---|---|
| WAV → MP3 | mp3, 48 kHz stereo, 12.02 s | −24.6 dB |
| WAV → M4A | aac, 12.03 s | −24.3 dB |
| WAV → FLAC | flac, 12.00 s | −24.3 dB |
| MP3 → WAV | pcm_s16le, 12.01 s | — |
| FLAC → MP3 | mp3, 12.00 s | — |
| M4A → WAV | pcm_s16le, 12.03 s | — |

Source was −24.3 dB: FLAC and AAC match exactly, MP3 differs by 0.3 dB as a lossy codec should. **MP3 output is why LAME is in the build** — FFmpeg has no native MP3 encoder and macOS no longer exposes an AudioToolbox one (S3).

### The queue (§21)

- **Pause happens BETWEEN items**, and the button says so ("Pause after current"). Pausing mid-encode is not generally possible, and §46 Rule 11 says to document the limitation rather than imply a capability.
- **Cancel** per item and queue-wide; a cancelled job still writes its trailer, so the partial file is playable rather than corrupt.
- **Retry** requeues a failed item without re-adding it.
- **Overwrite protection** refuses to clobber an existing file unless asked (§4).
- **Errors are sentences** (§23), never codec numbers.

### TOOLS (§31)

Six tools — extract audio as MP3/M4A/WAV, change format to MP4/MOV, convert to FLAC. Each is a **one-click configuration of the same queue**, per PRODUCT_DIRECTION §4's "presets over the converter engine, not separate code". They inherit progress, cancel, retry and overwrite protection for free, and there is no second implementation to keep in step.

### Three modes, not four

EDIT / CONVERT / TOOLS, as ratified in PRODUCT_DIRECTION §4. The separate AUDIO mode is gone; its useful parts live in EDIT.

---

## Resolved: the bare-path launch bug was AppKit, not the app

**It is not user-facing, and it is not fixable.** Both halves of that were
measured rather than assumed, and both were surprising.

### What it actually is

AppKit treats every bare (non-`-flag`) command-line argument as a file to open,
by exactly the rule NSUserDefaults' argument domain uses: the token after a
`-flag` is that flag's value, anything else is a file. Having decided the app
was launched to open a document, it **skips the "open untitled window" step** —
and a SwiftUI `Window` scene has no other trigger, so nothing is ever created.

The giveaway was that `Editor hello` fails identically to `Editor /tmp/a.wav`.
It was never about paths, and never about the file existing.

A stack sample settled the mechanism: the process is **idle in its run loop**,
not blocked. Nothing is waiting for a reply; the window was simply never asked
for.

```
DBG willFinishLaunching
DBG openFiles ["hello"]
DBG didFinishLaunching windows=0        <- no window, app otherwise healthy
```

### Every escape was tried, and none works

| Attempt | Result |
|---|---|
| Implement `application(_:openFile:)`, return `true` | windows=0 |
| Return `false` instead | windows=0 |
| `application(_:openFiles:)` + `reply(toOpenOrPrint: .cancel)` | windows=0 |
| …+ `.failure` | windows=0 |
| Declare `applicationShouldOpenUntitledFile` | never called |
| `.defaultLaunchBehavior(.presented)` on the scene | windows=0 |

The decision is made inside `finishLaunching`, before any of those run. It is
not something the app can answer its way out of.

### Why it costs nothing

**The path users actually take works.** Finder, the Dock, and
`open -a Editor.app file.mp4` deliver through an Apple event, which does *not*
suppress the window. Verified end to end by screenshot: opening a video from
Finder shows "Imported 1 file" with the clip on the timeline.

Only a direct `exec` of the binary with a bare path is affected — a test
harness, which we control. So the harness says `--import <path>`, repeated per
file, and the problem disappears.

The rule is stricter than it looks and is worth stating exactly: **every
argument must be a `-flag` followed by exactly one value.** `--import=<path>`
is *not* equivalent and re-breaks it — AppKit reads it as a key with no value,
pairs it with whatever flag comes next, and orphans that flag's value into a
bare argument. Measured, after shipping it briefly by mistake.

### What did change

Implementing the delegate was still the right move, for the reason it was
originally proposed: **without it, a file opened from Finder was silently
dropped on the floor.** `AppDelegate` now receives it, and `Info.plist` declares
what the app opens, so "Open With" and drag-onto-the-icon work. Media routes by
mode — into the timeline while editing, into the queue while converting — and a
`.mcproj` opens as a project.

## Video transcoding — the other half of §4

Conversions that cannot remux now **re-encode** rather than failing with an
honest message. Three decisions shape it:

- **Hardware encoders only** (AD-12), so the patent posture in
  `DEPENDENCY_AND_LICENSE_AUDIT.md` §6 stays bounded. HEVC by default: S4b
  measured hardware HEVC needing ~1.74x libx264's bitrate against ~2.31x for
  hardware H.264, which makes the codec choice the largest quality lever here.
- **Audio is copied when it can be.** Re-encoding audio that is already AAC or
  MP3 costs quality for nothing; it is only decoded and re-encoded when the
  target container will not carry the source codec.
- **CPU frames between decode and encode.** The zero-copy path S1 established
  matters where a frame's journey is GPU-to-GPU. Here the frame usually has to
  be *scaled*, which means swscale regardless — so the simple path is the right
  one. Conversion is a background batch job, not a 60 fps preview.

**WebM → MP4 now works**, which is one of the two conversions §4 names. Verified
end to end through the app: VP9 + Opus in, HEVC + AAC out. (MP4 → WebM still
needs VP8/VP9 *encoding*, i.e. libvpx in the build — BSD-licensed and
LGPL-compatible, so a build change rather than a licensing question.)

Nine tests assert on the **output file**, not on return codes: a conversion that
reports success and writes an unplayable file is the failure mode that matters.

## Resize, compress and extract frames (§31)

- **Resize to 1080p / 720p** and **Compress for Sharing** are one-click
  configurations of the transcode path, so they inherit progress, cancel, retry
  and overwrite protection.
- Output is **suffixed** (`-720p`, `-small`), and `add()` refuses to write over
  its own input — converting an MP4 to MP4 in its own folder lands on exactly
  the input path, and with overwrite on that destroys the source while reading
  it.
- **Extract Frames** goes through **ImageIO, not FFmpeg.** The shipped LGPL
  build has no PNG or MJPEG encoder — `--disable-autodetect` leaves them out,
  and a test now pins that fact so a future build change cannot quietly
  invalidate this reasoning. It also matches what Phase 3 found reading stills,
  where FFmpeg reported "unspecified size" for a valid PNG.

## Merge (§31)

Two paths, chosen by inspecting the inputs. **Copy** when every file agrees on
codecs, size and audio layout — clips off one phone almost always do, and then
merging is timestamp arithmetic: instant and lossless. **Re-encode** otherwise,
bringing everything to the first file's geometry and frame rate.

Two bugs worth recording, both found by measuring the output rather than
trusting the code:

- **Negative start timestamps.** AAC in MP4 carries encoder priming as a
  negative first timestamp, so a file that plays from zero reports its audio
  starting at -1024 samples. Adding an offset without subtracting that put the
  next segment's first packet *before* the previous segment's last, and the
  muxer refused the file outright. Each input's own start is now normalised
  away, taken from the container so picture and sound keep their alignment.
- **Counting frames instead of following timestamps.** A 25 fps clip merged
  after a 30 fps one came out playing 20 % fast — its 75 frames got 75 slots on
  a 30 fps clock. Placing each frame by *when it happens in its own file* fixed
  it. The test's tolerance was tightened from 1.0 s to 0.25 s precisely because
  the loose one had hidden this.

Ordering the decode flush also mattered: `advance()` bumped the offset before
flushing the previous input's decoder, so its last frames landed a whole segment
late — a five-second gap at the join.

## Queue persistence (§21)

A batch of conversions is exactly the kind of work that outlives a session: long,
unattended, and closing the window while it runs is normal. The unfinished part
of the queue is written to Application Support and restored at launch.

**Restored as waiting, never restarted.** Resuming an encode on launch would
write files the user did not ask for at that moment, possibly over something
they have since put there. The list comes back; pressing Convert stays their
decision. Sources that have since moved are dropped rather than restored as jobs
that can only fail.

Verified by killing the app mid-batch and relaunching: the jobs came back with
their error messages intact, and nothing started on its own.

## Still outstanding in Phase 6

- **MP4 → WebM**, which needs libvpx in the FFmpeg build. A build change and an
  AD-12 amendment (VP8/VP9 are royalty-free, so the patent rationale for
  hardware-only encoding does not apply to them), not a licensing question.
- **Merge drops audio on the re-encode path.** Mixed sources rarely share an
  audio format, and a merged file with the wrong sound is worse than one the
  user adds sound to afterwards — but it should eventually resample and mix.
