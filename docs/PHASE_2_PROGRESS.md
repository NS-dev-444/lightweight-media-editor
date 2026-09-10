# Phase 2 — Project and Timeline Engine

**Status:** ✅ **COMPLETE** for the model layer — 2026-09-10
**Outstanding:** Windows CI only (hardware arriving; see `WINDOWS_BRINGUP.md`)
**Scope:** §36 — project model, media assets, timeline, clips, tracks, timecode, trim, split, move, delete, undo/redo, save/load, autosave. No effects.
**Code:** `core/mediacore-model/` — kept, not a spike.

---

## What exists

| Module | Contents | Tests |
|---|---|---|
| `time` | `Ticks`, `FrameRate`, `TimeRange`, `Timecode` (drop-frame) | 8 |
| `asset` | `Asset`, colour, VFR, bookmark, content hash | — |
| `timeline` | `Track`, `Clip`, `Timeline`, validation | — |
| `command` | `Edit`, `History` (bounded undo/redo) | 15 |
| `project` | `Project`, JSON + migration, atomic save, `Autosave`, `Session` | 5 |
| `ops` | ripple delete/trim, delete range, close gap, split-at, snapping, gaps | 6 + 19 |
| `editor` | playhead, in/out points, navigation, selection | 10 |
| `relink` | missing-media matching by content hash and name | (in 19) |
| `cache` | AD-10 policy: content keys, four budgets, LRU eviction | (in 19) |
| `render` | render plans, fades, dirty ranges | (in 19) |
| *(properties)* | random-sequence invariants | 3 |

**`core/mediacore-media`** (FFmpeg FFI) is now in the same workspace:

| Module | Contents | Tests |
|---|---|---|
| `import` | probe → `Asset`, colour policy, import notices | 6 unit + 5 end-to-end |

**77 tests, 0 failures, 0 warnings, and zero `unwrap`/`expect`/`panic!` in non-test library code.** Cross-compiles for `x86_64-pc-windows-msvc` on every check (R-21).

## Decisions carried in from Phase 1

Each of these is implemented because a spike or an open question settled it — not assumed:

| Carried from | Implementation |
|---|---|
| §R11 + VFR findings | `TICKS_PER_SECOND = 705_600_000`; 24/25/30/48/50/60, the 1001-family and 44.1/48/96/192 kHz all divide **exactly**. Timeline is PTS-driven unconditionally; `is_vfr` is advisory only. |
| S8 (sandbox) | `Asset.bookmark` in schema v1, alongside path, relative path and content hash. |
| O-3 (colour) | `TransferFunction` defaults to `Rec709`, which also absorbs "unspecified" — Phase 1 found that is the common real-world case. `Project::hdr_assets()` exists so the UI can tell the user a conversion happened. |
| AD-12 / S4b | `ExportSettings` defaults to HEVC. |
| PRODUCT_DIRECTION §6 | `Timeline::v1_default()` — one video, one overlay, two audio. |
| §28 / R-16 | `History` stores only ids and small values; bounded by both entry count and byte budget. |
| §23 | `EditError` and `ProjectError` render human sentences, never internal jargon. Asserted by test. |

## Design notes

**Applying a command returns its own inverse.** Undo is "apply the inverse"; redo is "apply the inverse of that". One code path, so a forward operation and its inverse cannot silently disagree.

**Batches are atomic.** A failing step rolls back everything already applied in that batch — tested.

**Failed edits change nothing.** Every refusal path (locked track, overlap, bad split point, missing destination track) is tested for leaving the timeline byte-identical. The move path specifically restores the clip if the destination is invalid, so a clip cannot vanish into a failed move.

**Autosave journals commands, not documents.** §24 requires autosave that never blocks the UI; rewriting a large document per edit would. Recovery replays the journal onto the last full save and **skips a torn trailing line**, which is what a real crash mid-append produces — tested explicitly.

## A bug the tests caught

`History::undo()` initially returned the label of the *stored inverse*, so undoing an "Add Clip" reported **"Undo Delete Clip"**. The history now carries the original action's label separately. Small, user-visible, and exactly the kind of thing that survives to release without a test asserting the menu text.

## §15 operations (`ops`)

`command::Edit` holds *primitives*; `ops` composes them into what the UI offers. Composites are built as `Edit::Batch`, deliberately — a batch is atomic and inverts by reversing its parts, so **ripple delete gets correct undo for free** instead of needing a hand-written inverse, which is where this class of bug normally lives.

- `ripple_delete` — remove a clip, pull the rest of the track back
- `ripple_delete_range` — cut a time range out and close the hole (workflow **W1**, "cut the dead air out")
- `close_gap`, `gaps` — gap detection and closing
- `split_at` — split whatever sits under the playhead (§29's `S`)
- `snap` / `snap_targets` — edge and playhead snapping, excluding the dragged clip

## Property tests — and the two bugs they found

Hand-written tests check the cases we thought of. These check the ones we did not:

> apply N random edits → undo all N → **byte-identical to the start**
> then redo all N → **byte-identical to the edited state**

Invariants asserted after *every* step: the document validates, and a **rejected edit mutates nothing**. The generator is a small deterministic xorshift PRNG — no new dependency, and a failing seed reproduces exactly.

They immediately found two real bugs that the hand-written tests missed:

**1. `SetSpeed` did not validate geometry.** `timeline_duration` is `source / speed`, so slowing a clip *lengthens* it on the timeline and it can grow into its neighbour. Every other geometry-changing edit (add, move, trim) checked for overlap; this one reported success while producing an invalid document.

**2. `SplitClip` had a double-rounding bug at non-unit speed.** Converting timeline → source → timeline rounds twice, and at speeds other than 1.0 the result can land one tick past the requested point. Placing the right half at the requested time then left the two halves overlapping by a tick. Fixed by deriving the timeline split point *back* from the source point, so the halves are exactly adjacent at any speed.

Neither was reachable by any test I would have written by hand, and both produce corrupt projects rather than visible errors. This is the strongest argument for keeping property tests in CI from here on.

## Editing state (`editor`) — deliberately not serialised

Playhead, in/out points and selection are **transient session state, not part of `Project`**. Where the cursor sits is not a property of the document, and persisting it would mark the project dirty on every click and trigger an autosave.

- Frame stepping is **quantised**, so stepping forward and back 10,000 times returns to exactly the starting tick — tested.
- The playhead cannot go negative.
- In/out form a range only when both are set and ordered.
- `reconcile()` **prunes deleted clips from the selection**. Without it the selection silently references deleted material and the next operation fails for no visible reason.
- Snapping honours the §15 toggle and excludes the clip being dragged.

## Import boundary (`mediacore-media::import`)

The probe → `Asset` conversion lives in the **media** crate, so `mediacore-model` stays FFmpeg-free and keeps cross-compiling for Windows (R-21). Every rule traces to a Phase 1 measurement:

| Rule | Why |
|---|---|
| Unspecified transfer → **Rec.709** | Phase 1 found "unspecified" is the *common* real-world case, even in files encoded with explicit bt709 flags. Treating it as "unknown, refuse" would reject most ordinary video. |
| HDR without reported bit depth → **10-bit** | Real HDR files report `bits_per_raw_sample == 0`; a naive read yields "0-bit video". |
| Frame rate stays **rational** | 30000/1001, never 29.97. |
| HDR import raises a **user-visible notice** | O-3: the conversion must be disclosed, never silent. SDR imports stay silent — ordinary video must not nag. |
| A failed probe yields **no asset** | A partial asset is worse than none. |

End-to-end tests run against real files — the PQ, HLG, CFR, VFR and corrupt fixtures generated during Phase 1 — and skip cleanly when the fixtures are absent.

## Findings from wiring the crates together

**FFmpeg logs straight to stderr**, bypassing our error classification entirely — visible the first time a corrupt file was probed ("moov atom not found"). §23 requires errors the user can understand, so `mc_set_log_quiet()` was added; the host app silences FFmpeg at startup and relies on `mc_probe`'s error classes.

**Our dylibs carry `@rpath` install names** (required for `.app` bundling), so anything linking them needs a matching rpath. Without it `cargo test` builds fine and then aborts at load. `build.rs` now emits the rpath from `FFMPEG_LIBS_DIR`.

## Ripple trim, relink, cache, render

**Ripple trim** (`ops::ripple_trim_in/out`) anchors the clip's left edge on the timeline and shifts everything downstream — the gap *before* the clip is unchanged, which is what makes it a ripple rather than a slip. Ordering is load-bearing: growing moves neighbours out **first** (rightmost first) and then trims; shrinking trims first. Backwards, a legitimate edit fails with "would overlap".

**Relink** (`relink`) is now the *fallback*, not the primary mechanism — S8 showed bookmarks already survive a rename plus a directory move. What remains is what bookmarks cannot cover: cross-volume moves, restores from backup, a file replaced by a different copy, a project opened on another machine. Matching is ordered by confidence, and **only content-hash equality applies unattended**. A same-name, same-duration file is *not* proof — that is how the wrong take silently ends up in someone's edit. Relinking also **invalidates the stored bookmark**, since a stale one that still resolves would keep opening the old file.

**Cache** (`cache`) is policy only, no I/O, so the rules are unit-testable and identical on both platforms. Keys carry content hash + stream + parameters + **format version**, so a changed generator cannot misread stale entries. Eviction is LRU **within a class** — filling the proxy cache must never evict every thumbnail.

**Render plans** (`render`) are the AD-4 contract: the core decides *what* to draw, the platform decides *how*. A plan is small, serialisable, platform-free data that Metal and Direct3D both consume. No pixels cross it. `DirtyRanges` merges touching spans so §26's "never re-render everything" is enforceable by range.

## Two more bugs the property tests found

Adding ripple trim to the fuzzer immediately surfaced both:

**3. Ripple trim ordering.** Growing a clip trimmed it *before* moving its neighbours, so it collided mid-batch and a valid edit was rejected.

**4. `SplitClip` still overlapped by a tick.** The deeper cause: `timeline_duration` is *derived* as `source / speed`, so rounding two halves independently can make their sum one tick longer than the original — pushing the right half into the next clip. Fixed by fitting the right half to the room actually available, and by having `MergeClips` carry the original source window so undo restores it **exactly** rather than recomputing it.

That is **four real bugs** found by property testing across Phase 2, none of which any hand-written test caught, and all of which produce corrupt projects rather than visible errors.

Fuzzing now runs 250 seeds × 80 steps plus five 1,500-step sequences on every test run.

## Phase 2 exit

§36 asked for: project model, media assets, timeline, clips, tracks, timecode, trim, split, move, delete, undo/redo, save/load, autosave. **All present and tested.**

| Carried into Phase 3 | |
|---|---|
| `render::plan_at` | the contract the Metal compositor consumes |
| `cache` | policy for the host's store |
| `editor` | playhead/selection the UI drives |
| `import` | probe → asset, with HDR/VFR notices to surface |

**The only outstanding Phase 2 item is Windows CI**, which needs the machine.
