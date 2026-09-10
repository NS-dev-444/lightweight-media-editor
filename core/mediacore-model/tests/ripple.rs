//! §15 timeline operations: ripple delete, range delete, gaps, snapping.

use mediacore_model::asset::*;
use mediacore_model::command::*;
use mediacore_model::ops;
use mediacore_model::project::*;
use mediacore_model::time::*;
use mediacore_model::timeline::*;

fn secs(s: f64) -> Ticks { Ticks::from_seconds(s) }


/// One 10s clip on V1, reading the first 10s of its source.
fn session_with_one_clip(len: Ticks) -> (Session, TrackId, ClipId) {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/take.mp4", secs(600.0)));
    let track = p.timeline.tracks[0].id;
    let mut s = Session::new(p);
    let cid = s.project.new_clip_id();
    let clip = Clip::new(cid, aid, secs(0.0), TimeRange::new(secs(20.0), len));
    s.apply(Edit::AddClip { track, clip }).unwrap();
    (s, track, cid)
}

/// Three back-to-back 10s clips on V1: [0-10) [10-20) [20-30)
fn three_in_a_row() -> (Session, TrackId, Vec<ClipId>) {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/a.mp4", secs(600.0)));
    let track = p.timeline.tracks[0].id;
    let mut s = Session::new(p);
    let mut ids = Vec::new();
    for i in 0..3u64 {
        let cid = s.project.new_clip_id();
        ids.push(cid);
        let clip = Clip::new(cid, aid, secs(i as f64 * 10.0),
                             TimeRange::new(secs(i as f64 * 10.0), secs(10.0)));
        s.apply(Edit::AddClip { track, clip }).unwrap();
    }
    (s, track, ids)
}

#[test]
fn ripple_delete_closes_the_gap_and_undoes_cleanly() {
    let (mut s, track, ids) = three_in_a_row();
    let before = s.project.timeline.clone();

    let edit = ops::ripple_delete(&s.project.timeline, track, ids[1]).unwrap();
    s.apply(edit).unwrap();

    let t = s.project.timeline.track(track).unwrap();
    assert_eq!(t.clips.len(), 2);
    assert_eq!(t.clips[0].timeline_start, secs(0.0));
    assert_eq!(t.clips[1].timeline_start, secs(10.0), "third clip should have moved left");
    assert!(ops::gaps(&s.project.timeline, track).is_empty(), "ripple left a gap");

    s.undo().unwrap();
    assert_eq!(s.project.timeline, before, "ripple delete did not undo exactly");
}

#[test]
fn deleting_a_range_removes_dead_air_and_closes_up() {
    // PRODUCT_DIRECTION.md W1: "cut the dead air out of a long recording".
    let (mut s, track, _ids) = three_in_a_row();
    let before = s.project.timeline.clone();
    let total_before = s.project.timeline.duration();

    // Remove 5s straddling the boundary between clip 1 and clip 2.
    let new_id = ClipId(s.project.new_id());
    let edit = ops::ripple_delete_range(&s.project.timeline, track,
                                        TimeRange::new(secs(7.5), secs(5.0)), new_id).unwrap();
    s.apply(edit).unwrap();

    assert_eq!(s.project.timeline.duration(), total_before - secs(5.0),
               "timeline should shorten by exactly the removed range");
    assert!(ops::gaps(&s.project.timeline, track).is_empty());
    assert!(s.project.timeline.validate().is_empty());

    s.undo().unwrap();
    assert_eq!(s.project.timeline, before);
}

#[test]
fn close_gap_pulls_the_next_clip_back() {
    let (mut s, track, ids) = three_in_a_row();
    // Plain delete leaves a hole.
    let middle = s.project.timeline.track(track).unwrap().clip(ids[1]).cloned().unwrap();
    s.apply(Edit::RemoveClip { track, clip: middle }).unwrap();
    assert_eq!(ops::gaps(&s.project.timeline, track),
               vec![TimeRange::new(secs(10.0), secs(10.0))]);

    let edit = ops::close_gap(&s.project.timeline, track, secs(10.0)).unwrap();
    s.apply(edit).unwrap();
    assert!(ops::gaps(&s.project.timeline, track).is_empty());
    assert_eq!(s.project.timeline.track(track).unwrap().clips[1].timeline_start, secs(10.0));
}

#[test]
fn split_at_playhead_finds_the_clip_under_it() {
    let (mut s, track, ids) = three_in_a_row();
    let right = s.project.new_clip_id();
    let edit = ops::split_at(&s.project.timeline, track, secs(15.0), right).unwrap();
    s.apply(edit).unwrap();
    assert_eq!(s.project.timeline.track(track).unwrap().clips.len(), 4);
    let l = s.project.timeline.find_clip(ids[1]).unwrap().1;
    assert_eq!(l.timeline_range().end(), secs(15.0));

    // Nothing under the playhead is a refusal, not a panic.
    let another = s.project.new_clip_id();
    assert_eq!(ops::split_at(&s.project.timeline, track, secs(500.0), another),
               Err(EditError::InvalidSplitPoint));
}

#[test]
fn snapping_prefers_the_nearest_edge_within_threshold() {
    let (s, track, _ids) = three_in_a_row();
    let targets = ops::snap_targets(&s.project.timeline, None, Some(secs(25.0)));
    let thresh = secs(0.5);

    // Just shy of a clip boundary snaps to it.
    assert_eq!(ops::snap(secs(9.8), &targets, thresh), secs(10.0));
    assert_eq!(ops::snap(secs(20.2), &targets, thresh), secs(20.0));
    // The playhead is a snap target too.
    assert_eq!(ops::snap(secs(24.9), &targets, thresh), secs(25.0));
    // Far from anything, leave it alone.
    assert_eq!(ops::snap(secs(14.0), &targets, thresh), secs(14.0));
    // Zero is always a target.
    assert_eq!(ops::snap(secs(0.1), &targets, thresh), Ticks::ZERO);
    let _ = track;
}

#[test]
fn a_dragged_clip_does_not_snap_to_itself() {
    // Back-to-back clips share edges, so excluding one removes nothing --
    // its neighbours contribute the same times. Use a SEPARATED clip so the
    // excluded edges are its alone. (The first version of this test used the
    // adjacent fixture and asserted a reduction that cannot happen.)
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/a.mp4", secs(600.0)));
    let track = p.timeline.tracks[0].id;
    let mut s = Session::new(p);

    let anchor = s.project.new_clip_id();
    s.apply(Edit::AddClip { track, clip: Clip::new(anchor, aid, secs(0.0),
        TimeRange::new(Ticks::ZERO, secs(10.0))) }).unwrap();
    let dragged = s.project.new_clip_id();
    s.apply(Edit::AddClip { track, clip: Clip::new(dragged, aid, secs(40.0),
        TimeRange::new(Ticks::ZERO, secs(10.0))) }).unwrap();

    let with_self = ops::snap_targets(&s.project.timeline, None, None);
    let without = ops::snap_targets(&s.project.timeline, Some(dragged), None);
    assert!(with_self.contains(&secs(40.0)) && with_self.contains(&secs(50.0)));
    assert!(!without.contains(&secs(40.0)), "a dragged clip must not snap to its own start");
    assert!(!without.contains(&secs(50.0)), "a dragged clip must not snap to its own end");
    // The other clip's edges remain available.
    assert!(without.contains(&secs(10.0)));
}

#[test]
fn cutting_a_gap_out_of_the_middle_keeps_both_sides() {
    // The operation the product is built around: remove the dead air from the
    // middle of one long take and close the gap.
    //
    // This used to keep only the head and silently discard everything after
    // the cut — a four-second take with a two-second gap came back as one
    // second. It looks like a working feature until you play the result.
    let (mut s, track, clip) = session_with_one_clip(secs(10.0));
    let before = s.project.timeline.tracks[0].clips[0].source;

    let new_id = ClipId(s.project.new_id());
    let edit = ops::ripple_delete_range(&s.project.timeline, track,
                                        TimeRange::new(secs(3.0), secs(2.0)), new_id).unwrap();
    s.apply(edit).unwrap();

    let clips = &s.project.timeline.tracks[0].clips;
    assert_eq!(clips.len(), 2, "the take should be in two pieces, not one");

    let head = clips.iter().find(|c| c.id == clip).expect("the head survives");
    let tail = clips.iter().find(|c| c.id == new_id).expect("the tail survives");

    assert_eq!(head.timeline_start, secs(0.0));
    assert_eq!(head.timeline_range().duration, secs(3.0));
    // The tail is pulled back to where the cut began: no hole is left.
    assert_eq!(tail.timeline_start, secs(3.0), "the gap was not closed");

    let total: i64 = clips.iter().map(|c| c.timeline_range().duration.0).sum();
    assert_eq!(total, secs(8.0).0, "ten seconds minus a two-second cut is eight");

    // And the tail reads from AFTER the cut in the source, not from the start.
    assert_eq!(tail.source.start, before.start + secs(5.0),
               "the tail is reading the wrong part of the file");
}

#[test]
fn cutting_a_middle_gap_is_exactly_undoable() {
    // §28: the split is two edits plus a fade change, and undo has to put all
    // of it back — a partial undo would leave a stray clip behind.
    let (mut s, track, _clip) = session_with_one_clip(secs(10.0));
    let before = s.project.timeline.clone();

    let new_id = ClipId(s.project.new_id());
    let edit = ops::ripple_delete_range(&s.project.timeline, track,
                                        TimeRange::new(secs(3.0), secs(2.0)), new_id).unwrap();
    s.apply(edit).unwrap();
    s.undo().unwrap();
    assert_eq!(s.project.timeline, before, "undo did not restore the take");
}
