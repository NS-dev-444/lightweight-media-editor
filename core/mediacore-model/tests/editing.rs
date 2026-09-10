//! Phase 2 integration tests — the §36 operations, exercised end to end.
//!
//! §45: a feature is not complete because the happy path works. Each operation
//! here is tested for its result, its INVERSE, and its failure modes.

use mediacore_model::asset::*;
use mediacore_model::command::*;
use mediacore_model::project::*;
use mediacore_model::time::*;
use mediacore_model::timeline::*;

fn secs(s: f64) -> Ticks { Ticks::from_seconds(s) }

/// A project with one 60-second asset and one clip on V1.
fn fixture() -> (Session, TrackId, ClipId) {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    let mut asset = Asset::new(aid, "/media/a.mp4", secs(60.0));
    asset.video = Some(VideoInfo { width: 1920, height: 1080,
        frame_rate: FrameRate::FILM, colour: ColourInfo::default(), is_vfr: false });
    p.assets.push(asset);

    let track = p.timeline.tracks[0].id;
    let cid = p.new_clip_id();
    let clip = Clip::new(cid, aid, Ticks::ZERO, TimeRange::new(secs(5.0), secs(10.0)));
    let mut s = Session::new(p);
    s.apply(Edit::AddClip { track, clip }).unwrap();
    (s, track, cid)
}

#[test]
fn v1_track_layout_is_ratified_shape() {
    // PRODUCT_DIRECTION.md §6: one video, one overlay, two audio.
    let p = Project::default();
    let kinds: Vec<_> = p.timeline.tracks.iter().map(|t| t.kind).collect();
    // Two overlay tracks: a title and a logo must coexist.
    assert_eq!(kinds, vec![TrackKind::Video, TrackKind::Overlay, TrackKind::Overlay,
                           TrackKind::Audio, TrackKind::Audio]);
}

#[test]
fn add_then_undo_leaves_no_trace() {
    let (mut s, _t, _c) = fixture();
    assert_eq!(s.project.timeline.tracks[0].clips.len(), 1);
    assert_eq!(s.undo().unwrap(), Some("Add Clip"));
    assert_eq!(s.project.timeline.tracks[0].clips.len(), 0);
    assert_eq!(s.redo().unwrap(), Some("Add Clip"));
    assert_eq!(s.project.timeline.tracks[0].clips.len(), 1);
    assert!(s.project.timeline.validate().is_empty());
}

#[test]
fn split_then_undo_restores_exactly() {
    let (mut s, _t, cid) = fixture();
    let before = s.project.timeline.clone();
    let right = s.project.new_clip_id();
    s.apply(Edit::SplitClip { clip: cid, at: secs(4.0), right }).unwrap();

    let tl = &s.project.timeline;
    assert_eq!(tl.tracks[0].clips.len(), 2);
    let l = tl.tracks[0].clip(cid).unwrap();
    let r = tl.tracks[0].clip(right).unwrap();
    assert_eq!(l.timeline_duration(), secs(4.0));
    assert_eq!(r.timeline_start, secs(4.0));
    assert_eq!(r.timeline_duration(), secs(6.0));
    // The split must not invent or lose source material.
    assert_eq!(l.source.duration + r.source.duration, secs(10.0));
    assert_eq!(l.source.end(), r.source.start);

    s.undo().unwrap();
    assert_eq!(s.project.timeline, before, "undo of split did not restore exactly");
}

#[test]
fn split_at_a_boundary_is_refused() {
    let (mut s, _t, cid) = fixture();
    let r = s.project.new_clip_id();
    assert_eq!(s.apply(Edit::SplitClip { clip: cid, at: Ticks::ZERO, right: r }),
               Err(EditError::InvalidSplitPoint));
    assert_eq!(s.apply(Edit::SplitClip { clip: cid, at: secs(10.0), right: r }),
               Err(EditError::InvalidSplitPoint));
    // A refused edit must leave the timeline untouched.
    assert_eq!(s.project.timeline.tracks[0].clips.len(), 1);
}

#[test]
fn overlapping_add_is_refused_and_changes_nothing() {
    let (mut s, track, _c) = fixture();
    let before = s.project.timeline.clone();
    let aid = s.project.assets[0].id;
    let cid = s.project.new_clip_id();
    let overlapping = Clip::new(cid, aid, secs(5.0), TimeRange::new(Ticks::ZERO, secs(10.0)));
    assert!(matches!(s.apply(Edit::AddClip { track, clip: overlapping }),
                     Err(EditError::WouldOverlap { .. })));
    assert_eq!(s.project.timeline, before);
}

#[test]
fn move_between_tracks_and_back() {
    let (mut s, from, cid) = fixture();
    let to = s.project.timeline.tracks[1].id;
    let before = s.project.timeline.clone();
    s.apply(Edit::MoveClip { clip: cid, from_track: from, to_track: to,
                             from_start: Ticks::ZERO, to_start: secs(20.0) }).unwrap();
    assert!(s.project.timeline.track(from).unwrap().clips.is_empty());
    assert_eq!(s.project.timeline.track(to).unwrap().clips[0].timeline_start, secs(20.0));
    s.undo().unwrap();
    assert_eq!(s.project.timeline, before);
}

#[test]
fn a_failed_move_puts_the_clip_back() {
    let (mut s, from, cid) = fixture();
    let before = s.project.timeline.clone();
    let missing = TrackId(9999);
    assert_eq!(s.apply(Edit::MoveClip { clip: cid, from_track: from, to_track: missing,
                                        from_start: Ticks::ZERO, to_start: secs(20.0) }),
               Err(EditError::NoSuchTrack(missing)));
    // The clip must not vanish because the destination was invalid.
    assert_eq!(s.project.timeline, before, "failed move lost the clip");
}

#[test]
fn locked_tracks_refuse_edits() {
    let (mut s, track, cid) = fixture();
    s.apply(Edit::SetTrackLocked { track, from: false, to: true }).unwrap();
    let aid = s.project.assets[0].id;
    let new = s.project.new_clip_id();
    assert_eq!(s.apply(Edit::AddClip { track,
                   clip: Clip::new(new, aid, secs(30.0), TimeRange::new(Ticks::ZERO, secs(1.0))) }),
               Err(EditError::TrackLocked(track)));
    let existing = s.project.timeline.track(track).unwrap().clip(cid).cloned().unwrap();
    assert_eq!(s.apply(Edit::RemoveClip { track, clip: existing }),
               Err(EditError::TrackLocked(track)));
}

#[test]
fn a_failed_batch_rolls_back_completely() {
    let (mut s, track, _c) = fixture();
    let before = s.project.timeline.clone();
    let aid = s.project.assets[0].id;
    let ok = Clip::new(s.project.new_clip_id(), aid, secs(20.0), TimeRange::new(Ticks::ZERO, secs(5.0)));
    let bad = Clip::new(s.project.new_clip_id(), aid, secs(21.0), TimeRange::new(Ticks::ZERO, secs(5.0)));
    // Second clip overlaps the first, so the batch must fail atomically.
    assert!(s.apply(Edit::Batch(vec![
        Edit::AddClip { track, clip: ok },
        Edit::AddClip { track, clip: bad },
    ])).is_err());
    assert_eq!(s.project.timeline, before, "batch left a partial edit behind");
}

#[test]
fn speed_changes_timeline_duration_not_source() {
    let (mut s, _t, cid) = fixture();
    let src_before = s.project.timeline.find_clip(cid).unwrap().1.source;
    s.apply(Edit::SetSpeed { clip: cid, from: 1.0, to: 2.0 }).unwrap();
    let c = s.project.timeline.find_clip(cid).unwrap().1;
    assert_eq!(c.timeline_duration(), secs(5.0), "2x speed should halve timeline duration");
    assert_eq!(c.source, src_before, "speed must not alter the source window");
    assert_eq!(s.apply(Edit::SetSpeed { clip: cid, from: 2.0, to: 0.0 }),
               Err(EditError::InvalidValue("speed")));
}

#[test]
fn a_long_session_does_not_grow_history_without_bound() {
    // R-16: undo must not accumulate forever.
    let (mut s, track, _c) = fixture();
    s.history = History::new(20, 1 << 20);
    let aid = s.project.assets[0].id;
    for i in 0..200u64 {
        let cid = s.project.new_clip_id();
        let start = secs(20.0 + i as f64 * 2.0);
        let clip = Clip::new(cid, aid, start, TimeRange::new(Ticks::ZERO, secs(1.0)));
        s.apply(Edit::AddClip { track, clip }).unwrap();
    }
    let (undo, _redo) = s.history.depth();
    assert!(undo <= 20, "history grew to {undo}, past its bound");
    assert!(s.project.timeline.validate().is_empty());
}

#[test]
fn round_trips_through_json() {
    let (s, _t, _c) = fixture();
    let json = s.project.to_json().unwrap();
    let back = Project::from_json(&json).unwrap();
    assert_eq!(back, s.project);
    assert!(json.contains("\"schema_version\": 1"));
}

#[test]
fn a_versionless_file_migrates() {
    // Files written before schema_version existed must still open.
    let mut v: serde_json::Value = serde_json::from_str(&Project::default().to_json().unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("schema_version");
    let p = Project::from_json(&serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(p.schema_version, SCHEMA_VERSION);
}

#[test]
fn a_newer_file_is_refused_with_a_readable_message() {
    let mut v: serde_json::Value = serde_json::from_str(&Project::default().to_json().unwrap()).unwrap();
    v.as_object_mut().unwrap().insert("schema_version".into(), serde_json::json!(999));
    let err = Project::from_json(&serde_json::to_string(&v).unwrap()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("newer version"), "unhelpful message: {msg}");
    assert!(!msg.contains("serde"), "leaked internals: {msg}");
}

#[test]
fn hdr_and_vfr_sources_are_reported() {
    // O-3 requires telling the user a conversion happened.
    let mut p = Project::default();
    let id = p.new_asset_id();
    let mut a = Asset::new(id, "/media/hdr.mov", secs(10.0));
    a.video = Some(VideoInfo { width: 3840, height: 2160, frame_rate: FrameRate::NTSC,
        colour: ColourInfo { transfer: TransferFunction::Hlg, bit_depth: 10 }, is_vfr: true });
    p.assets.push(a);
    assert_eq!(p.hdr_assets(), vec![id]);
    assert_eq!(p.vfr_assets(), vec![id]);
}

#[test]
fn everything_phase_4_added_survives_a_save_and_reopen() {
    // A new field that does not round-trip is silent data loss: the grade, the
    // crop and the ducking curve simply are not there when the project is
    // reopened, and nothing errors to say so. Each of these was added after the
    // original round-trip test was written, and each needs asserting by name —
    // `assert_eq!(back, project)` passes happily when a field is missing from
    // BOTH sides because neither was serialised.
    let (mut s, track, clip) = fixture();

    let geometry = Geometry { crop_x: 0.1, crop_y: 0.2, crop_w: 0.5, crop_h: 0.6,
                              rotation: 3, flip_h: true, flip_v: false };
    s.apply(Edit::SetGeometry { clip, from: Geometry::default(), to: geometry }).unwrap();

    let points = vec![
        GainPoint { at: Ticks(0), gain: 1.0 },
        GainPoint { at: Ticks(1000), gain: 0.25 },
        GainPoint { at: Ticks(2000), gain: 1.0 },
    ];
    s.apply(Edit::SetGainPoints { clip, from: Vec::new(), to: points.clone() }).unwrap();

    // LutSpec has no dedicated edit; it is set on the clip like the image spec.
    {
        let c = s.project.timeline.track_mut(track).unwrap().clip_mut(clip).unwrap();
        c.lut = Some(LutSpec { path: "/looks/teal.cube".into(), amount: 0.6 });
    }

    let json = s.project.to_json().unwrap();
    let back = Project::from_json(&json).unwrap();
    let c = back.timeline.track(track).unwrap().clips.iter()
        .find(|c| c.id == clip).expect("the clip survived");

    assert_eq!(c.geometry, geometry, "the crop and rotation were lost");
    assert_eq!(c.gain_points, points, "the volume curve was lost");
    assert_eq!(c.lut.as_ref().map(|l| l.path.as_str()), Some("/looks/teal.cube"),
               "the look was lost");
    assert_eq!(c.lut.as_ref().map(|l| l.amount), Some(0.6),
               "the look's amount was lost");

    // And the whole project still compares equal, which catches anything else.
    assert_eq!(back, s.project);
}

#[test]
fn an_ordinary_project_does_not_carry_phase_4_fields_at_all() {
    // The other half of the bargain: these are skipped when unused, so an
    // ordinary project file stays small and readable.
    let (s, _t, _c) = fixture();
    let json = s.project.to_json().unwrap();
    for field in ["crop_x", "gain_points", "lut"] {
        assert!(!json.contains(field), "an untouched project wrote {field}");
    }
}
