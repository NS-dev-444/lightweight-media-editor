//! Property tests: invariants that must hold for ANY sequence of edits.
//!
//! Hand-written tests check the cases we thought of. These check the ones we
//! did not. The core property for an editor is round-tripping:
//!
//!   apply N random edits, undo all N  ->  byte-identical to where we started
//!   then redo all N                   ->  byte-identical to the edited state
//!
//! A single asymmetric inverse anywhere breaks this, which is precisely the
//! bug class that otherwise reaches users as "undo corrupted my project".
//!
//! No external dependency: the generator is a small deterministic PRNG, so a
//! failure is reproducible from its seed alone.

use mediacore_model::asset::*;
use mediacore_model::command::*;
use mediacore_model::ops;
use mediacore_model::project::*;
use mediacore_model::time::*;
use mediacore_model::timeline::*;

/// xorshift64*. Deterministic and seedable — a failing seed reproduces exactly.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12; x ^= x << 25; x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 { if n == 0 { 0 } else { self.next() % n } }
    fn ticks_to(&mut self, max_secs: u64) -> Ticks {
        Ticks::from_seconds(self.below(max_secs * 10) as f64 / 10.0)
    }
}

fn seeded_session() -> (Session, AssetId) {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/a.mp4", Ticks::from_seconds(3600.0)));
    let mut s = Session::new(p);
    s.history = History::new(100_000, 256 * 1024 * 1024);   // no trimming mid-test
    (s, aid)
}

/// Build a plausible edit. Many will be rejected; that is part of the test —
/// a rejected edit must leave the document untouched.
fn random_edit(rng: &mut Rng, s: &mut Session, aid: AssetId) -> Option<Edit> {
    let tracks: Vec<TrackId> = s.project.timeline.tracks.iter().map(|t| t.id).collect();
    let track = tracks[rng.below(tracks.len() as u64) as usize];
    let clips: Vec<(TrackId, ClipId)> = s.project.timeline.tracks.iter()
        .flat_map(|t| t.clips.iter().map(move |c| (t.id, c.id))).collect();

    match rng.below(11) {
        0 | 1 => {
            let cid = s.project.new_clip_id();
            let start = rng.ticks_to(120);
            let dur = Ticks::from_seconds(1.0 + rng.below(80) as f64 / 10.0);
            Some(Edit::AddClip { track,
                clip: Clip::new(cid, aid, start, TimeRange::new(rng.ticks_to(600), dur)) })
        }
        2 => {
            let (t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let clip = s.project.timeline.track(t)?.clip(c)?.clone();
            Some(Edit::RemoveClip { track: t, clip })
        }
        3 => {
            let (t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let from = s.project.timeline.track(t)?.clip(c)?.timeline_start;
            Some(Edit::MoveClip { clip: c, from_track: t, to_track: track,
                                  from_start: from, to_start: rng.ticks_to(120) })
        }
        4 => {
            let (_t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let (_, clip) = s.project.timeline.find_clip(c)?;
            let r = clip.timeline_range();
            let at = r.start + Ticks(rng.below(r.duration.0.max(1) as u64) as i64);
            Some(Edit::SplitClip { clip: c, at, right: s.project.new_clip_id() })
        }
        5 => {
            let (_t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let from = s.project.timeline.find_clip(c)?.1.speed;
            Some(Edit::SetSpeed { clip: c, from, to: 0.5 + rng.below(30) as f64 / 10.0 })
        }
        6 => {
            let (_t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let from = s.project.timeline.find_clip(c)?.1.gain;
            Some(Edit::SetGain { clip: c, from, to: rng.below(20) as f64 / 10.0 })
        }
        7 => {
            let (t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            ops::ripple_delete(&s.project.timeline, t, c).ok()
        }
        8 => {
            let (t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let end = s.project.timeline.find_clip(c)?.1.timeline_range().end();
            let target = Ticks(end.0 + (rng.below(20_000_000_000) as i64) - 10_000_000_000);
            ops::ripple_trim_out(&s.project.timeline, t, c, target.max(Ticks(1))).ok()
        }
        9 => {
            let (t, c) = *clips.get(rng.below(clips.len() as u64) as usize)?;
            let start = s.project.timeline.find_clip(c)?.1.timeline_start;
            let target = Ticks(start.0 + (rng.below(10_000_000_000) as i64) - 5_000_000_000);
            ops::ripple_trim_in(&s.project.timeline, t, c, target.max(Ticks::ZERO)).ok()
        }
        _ => {
            let start = rng.ticks_to(100);
            let dur = Ticks::from_seconds(0.5 + rng.below(50) as f64 / 10.0);
            let new_id = ClipId(s.project.new_id());
            ops::ripple_delete_range(&s.project.timeline, track,
                                     TimeRange::new(start, dur), new_id).ok()
        }
    }
}

fn run_seed(seed: u64, steps: usize) {
    let (mut s, aid) = seeded_session();
    let mut rng = Rng(seed);
    let start_state = s.project.timeline.clone();
    let mut applied = 0usize;

    for step in 0..steps {
        let Some(edit) = random_edit(&mut rng, &mut s, aid) else { continue };
        let before = s.project.timeline.clone();
        let edit_dbg = format!("{edit:?}");
        match s.apply(edit) {
            Ok(()) => {
                applied += 1;
                let errs = s.project.timeline.validate();
                // Name the offending edit: a bare "document is invalid" makes
                // a random-sequence failure very hard to diagnose.
                assert!(errs.is_empty(),
                        "seed {seed} step {step}: document became invalid: {errs:?}\n\
                         offending edit: {edit_dbg}");
            }
            Err(_) => {
                // A rejected edit must be a no-op. This is where partial
                // application bugs hide.
                assert_eq!(s.project.timeline, before,
                           "seed {seed} step {step}: a REJECTED edit mutated the document");
            }
        }
    }

    let edited_state = s.project.timeline.clone();

    // Undo everything.
    let mut undone = 0usize;
    while s.history.can_undo() {
        s.undo().unwrap_or_else(|e| panic!("seed {seed}: undo failed: {e:?}"));
        undone += 1;
        let errs = s.project.timeline.validate();
        assert!(errs.is_empty(), "seed {seed}: invalid mid-undo: {errs:?}");
    }
    assert_eq!(undone, applied, "seed {seed}: undo count did not match applied count");
    assert_eq!(s.project.timeline, start_state,
               "seed {seed}: undoing everything did not restore the original document");

    // Redo everything.
    while s.history.can_redo() {
        s.redo().unwrap_or_else(|e| panic!("seed {seed}: redo failed: {e:?}"));
        let errs = s.project.timeline.validate();
        assert!(errs.is_empty(), "seed {seed}: invalid mid-redo: {errs:?}");
    }
    assert_eq!(s.project.timeline, edited_state,
               "seed {seed}: redoing everything did not restore the edited document");
}

#[test]
fn undo_round_trips_across_many_random_sequences() {
    for seed in 1..=250u64 { run_seed(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), 80); }
}

#[test]
fn long_sequences_stay_consistent() {
    for seed in [0xDEAD_BEEFu64, 0x1234_5678_9ABC_DEF0, 0xCAFE_BABE,
                 0x0BAD_C0DE, 0xFEED_FACE_DEAD_BEEF] {
        run_seed(seed, 1_500);
    }
}

#[test]
fn a_saved_and_reloaded_project_is_identical() {
    // Serialisation must be lossless for any reachable document.
    let (mut s, aid) = seeded_session();
    let mut rng = Rng(0xFEED_FACE);
    for _ in 0..200 {
        if let Some(e) = random_edit(&mut rng, &mut s, aid) { let _ = s.apply(e); }
    }
    let json = s.project.to_json().unwrap();
    let back = Project::from_json(&json).unwrap();
    assert_eq!(back, s.project, "project did not survive a JSON round trip");
}
