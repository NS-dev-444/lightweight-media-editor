# Checks and CI

## One script

Everything that must be true before a change is good lives in
**`tools/check.sh`**, and `.github/workflows/ci.yml` runs *that script and
nothing else*.

That is deliberate. A CI file which grows its own list of steps drifts from what
developers run locally, and then "it passes on my machine" becomes a sentence
people say. One script, two callers, no possible disagreement about what passing
means.

```bash
tools/check.sh          # stops at the first failure — what you want while working
tools/check.sh --all    # runs everything and reports at the end
```

Checks are ordered cheapest-first: a formatting slip should not wait behind a
transcription run to be reported.

## What it checks, and why each one exists

| Check | Guards |
|---|---|
| Core builds with **zero warnings** | §46 Rule 4. A warning today is a bug next month. |
| Model tests | The edit engine, timebase, captions, undo. |
| Media tests | FFI, transcode, merge, loudness, transcription — against real files. |
| Media crate builds **without** whisper | Transcription is optional; the Windows port depends on adding one dependency at a time. |
| No `unwrap`/`expect`/`panic!` in library code | §22. |
| Model cross-compiles for Windows | **R-21** — stops the deferred port becoming a rewrite. |
| Shipped FFmpeg is LGPL-clean | **R-05**, the likeliest licensing failure here: a GPL FFmpeg sits on the default `PATH`. |
| whisper.cpp is still MIT | Audit §6b. |
| App builds with zero warnings | §46 Rule 4, other language. |
| App self-tests | Export arithmetic, `.cube` parsing. |
| **Every binary targets the same macOS floor** | **R-27** — see below. |
| App links nothing from Homebrew | **R-05** again, on the artefact rather than the script. |

## Skips are not passes

A check whose inputs are absent reports **skipped**, in yellow, and is counted
separately. This matters more than it sounds: an early version of the "builds
without whisper" check *passed* from a cached artefact while the headers it
needed were missing. A check that reports success when its inputs are gone is
worse than no check, because it is trusted.

CI always builds the fixtures, so nothing skips there.

## The deployment floor

`tools/deployment.sh` holds one definition of the oldest supported macOS,
sourced by all three build scripts and substituted into `Info.plist`.

It exists because the builds drifted: the plist said 14.0 while the Swift
binary, six FFmpeg dylibs, LAME, whisper.cpp and the C shim were all built for
26.0. Launch Services would have allowed the app onto a macOS 14 machine and
dyld would have refused it. **Nothing warned** — every piece was individually
correct.

The check compares the plist and every binary in the bundle against that one
value. Two traps it exists to catch, both hit while fixing the original bug:

- **`make` relinks only what changed** — a rebuild with a new floor left
  `libavcodec` at 14.0 and `libavutil` at 26.0. The FFmpeg script now stamps
  what it built for and wipes the prefix when that changes.
- **`build.rs` is cached** — the C shim's object survived from before the floor
  existed. The flag is passed explicitly now, not inherited from the
  environment.

## Every gate was verified to fail

A check suite that has never failed is theatre. Each was tested by breaking
something on purpose:

| Planted fault | Caught by |
|---|---|
| `v.unwrap()` in `time.rs` | §22 lint |
| An unused variable | zero-warnings gate |
| Floor moved to 15.0 without rebuilding | deployment-target gate |
| Homebrew's GPL FFmpeg pointed at the licence gate | rejected on `--enable-gpl` |

The licence gate also **fails closed**: a missing or unreadable target exits
non-zero rather than reporting success.

## First run

**CI has never run** — there is no git remote yet. The workflow targets
`macos-15` runners and caches FFmpeg, whisper.cpp and the model against the hash
of their build scripts, so editing a build script (including its licence flags)
busts the cache. That is the behaviour R-05 needs.

Expect the first real run to need adjustment.
