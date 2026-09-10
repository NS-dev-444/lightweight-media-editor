//! §15/§29 — playhead, in/out points, navigation, selection.

use mediacore_model::asset::*;
use mediacore_model::command::*;
use mediacore_model::editor::*;
use mediacore_model::project::*;
use mediacore_model::time::*;
use mediacore_model::timeline::*;

fn secs(s: f64) -> Ticks { Ticks::from_seconds(s) }

fn three_clips() -> (Session, TrackId, Vec<ClipId>) {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/a.mp4", secs(600.0)));
    let track = p.timeline.tracks[0].id;
    let mut s = Session::new(p);
    let mut ids = Vec::new();
    for i in 0..3u64 {
        let cid = s.project.new_clip_id();
        ids.push(cid);
        s.apply(Edit::AddClip { track, clip: Clip::new(cid, aid, secs(i as f64 * 10.0),
            TimeRange::new(Ticks::ZERO, secs(10.0))) }).unwrap();
    }
    (s, track, ids)
}

#[test]
fn stepping_frames_does_not_accumulate_drift() {
    // The failure this guards against: stepping forward and back N times
    // leaving the playhead a fraction of a frame off where it started.
    let mut ph = Playhead::default();
    let rate = FrameRate::NTSC;
    for _ in 0..10_000 { ph.step_frames(1, rate); }
    assert_eq!(ph.position(), Ticks(rate.frame_duration().0 * 10_000));
    for _ in 0..10_000 { ph.step_frames(-1, rate); }
    assert_eq!(ph.position(), Ticks::ZERO, "stepping back returned to a different place");
}

#[test]
fn the_playhead_never_goes_negative() {
    let mut ph = Playhead::default();
    ph.step_frames(-100, FrameRate::FILM);
    assert_eq!(ph.position(), Ticks::ZERO);
    ph.set(Ticks(-5000));
    assert_eq!(ph.position(), Ticks::ZERO);
}

#[test]
fn quantising_lands_on_a_frame_boundary() {
    let mut ph = Playhead::default();
    let rate = FrameRate::FILM;
    ph.set(secs(1.031));                 // between frames
    ph.quantise(rate);
    assert_eq!(ph.position().0 % rate.frame_duration().0, 0);
    assert!(ph.position() <= secs(1.031), "quantise must not move forward");
}

#[test]
fn in_and_out_points_form_a_range_only_when_ordered() {
    let mut ph = Playhead::default();
    assert!(ph.marked_range().is_none());
    ph.in_point = Some(secs(5.0));
    assert!(ph.marked_range().is_none(), "an in point alone is not a range");
    ph.out_point = Some(secs(2.0));
    assert!(ph.marked_range().is_none(), "out before in is not a range");
    ph.out_point = Some(secs(12.0));
    assert_eq!(ph.marked_range(), Some(TimeRange::new(secs(5.0), secs(7.0))));
    ph.clear_marks();
    assert!(ph.marked_range().is_none());
}

#[test]
fn navigation_walks_clip_boundaries() {
    let (s, track, _ids) = three_clips();
    let tl = &s.project.timeline;
    assert_eq!(next_edit_point(tl, Ticks::ZERO, Some(track)), Some(secs(10.0)));
    assert_eq!(next_edit_point(tl, secs(10.0), Some(track)), Some(secs(20.0)));
    assert_eq!(next_edit_point(tl, secs(30.0), Some(track)), None, "nothing past the end");
    assert_eq!(prev_edit_point(tl, secs(25.0), Some(track)), Some(secs(20.0)));
    assert_eq!(prev_edit_point(tl, Ticks::ZERO, Some(track)), None);
}

#[test]
fn selection_click_versus_modifier_click() {
    let (_s, _t, ids) = three_clips();
    let mut sel = Selection::default();
    sel.select_only(ids[0]);
    sel.select_only(ids[1]);
    assert_eq!(sel.as_vec(), vec![ids[1]], "a plain click replaces the selection");
    sel.add(ids[2]);
    assert_eq!(sel.len(), 2, "modifier-click adds");
    sel.toggle(ids[2]);
    assert_eq!(sel.len(), 1, "toggling an included clip removes it");
}

#[test]
fn marquee_selects_by_overlap() {
    let (s, _track, ids) = three_clips();
    let mut sel = Selection::default();
    // 5s..15s touches clip 0 and clip 1 but not clip 2.
    sel.select_in_range(&s.project.timeline, TimeRange::new(secs(5.0), secs(10.0)), None);
    assert!(sel.contains(ids[0]) && sel.contains(ids[1]) && !sel.contains(ids[2]));
}

#[test]
fn a_deleted_clip_leaves_the_selection() {
    // Otherwise the selection silently references deleted material and the
    // next operation fails for no visible reason.
    let (mut s, track, ids) = three_clips();
    let mut ed = EditorState::new(FrameRate::FILM);
    ed.selection.select_all(&s.project.timeline);
    assert_eq!(ed.selection.len(), 3);

    let clip = s.project.timeline.track(track).unwrap().clip(ids[1]).cloned().unwrap();
    s.apply(Edit::RemoveClip { track, clip }).unwrap();
    ed.reconcile(&s.project.timeline);

    assert_eq!(ed.selection.len(), 2);
    assert!(!ed.selection.contains(ids[1]));
}

#[test]
fn snapping_respects_the_toggle() {
    let (s, _track, _ids) = three_clips();
    let mut ed = EditorState::new(FrameRate::FILM);
    let near = secs(9.99);
    assert_eq!(ed.snap(&s.project.timeline, near, None), secs(10.0));
    ed.snap_threshold = Ticks::ZERO;      // snapping off
    assert_eq!(ed.snap(&s.project.timeline, near, None), near);
}

#[test]
fn timecode_tracks_the_playhead() {
    let mut ph = Playhead::default();
    ph.step_frames(107_892, FrameRate::NTSC);
    assert_eq!(ph.timecode(FrameRate::NTSC).to_string(), "01:00:00;00");
    let mut ph2 = Playhead::default();
    ph2.step_frames(24, FrameRate::FILM);
    assert_eq!(ph2.timecode(FrameRate::FILM).to_string(), "00:00:01:00");
}
