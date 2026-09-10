//! Ripple trim, relink, cache policy, render plans.

use mediacore_model::asset::*;
use mediacore_model::cache::*;
use mediacore_model::command::*;
use mediacore_model::ops;
use mediacore_model::project::*;
use mediacore_model::relink::{self, Candidate, MatchQuality};
use mediacore_model::render::{self, DirtyRanges};
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
            TimeRange::new(secs(100.0), secs(10.0))) }).unwrap();
    }
    (s, track, ids)
}

// ---------------------------------------------------------------- ripple trim

#[test]
fn shortening_a_clip_pulls_the_rest_back() {
    let (mut s, track, ids) = three_clips();
    let before = s.project.timeline.clone();
    let e = ops::ripple_trim_out(&s.project.timeline, track, ids[0], secs(6.0)).unwrap();
    s.apply(e).unwrap();

    let t = s.project.timeline.track(track).unwrap();
    assert_eq!(t.clip(ids[0]).unwrap().timeline_range().end(), secs(6.0));
    assert_eq!(t.clip(ids[1]).unwrap().timeline_start, secs(6.0), "gap left open");
    assert_eq!(t.clip(ids[2]).unwrap().timeline_start, secs(16.0));
    assert!(ops::gaps(&s.project.timeline, track).is_empty());

    s.undo().unwrap();
    assert_eq!(s.project.timeline, before);
}

#[test]
fn lengthening_a_clip_pushes_the_rest_out() {
    let (mut s, track, ids) = three_clips();
    let e = ops::ripple_trim_out(&s.project.timeline, track, ids[0], secs(14.0)).unwrap();
    s.apply(e).unwrap();
    let t = s.project.timeline.track(track).unwrap();
    assert_eq!(t.clip(ids[1]).unwrap().timeline_start, secs(14.0));
    assert_eq!(t.clip(ids[2]).unwrap().timeline_start, secs(24.0));
    assert!(s.project.timeline.validate().is_empty(), "push produced an invalid document");
}

#[test]
fn trimming_the_left_edge_moves_the_source_window_too() {
    let (mut s, track, ids) = three_clips();
    let src_before = s.project.timeline.find_clip(ids[1]).unwrap().1.source;
    let e = ops::ripple_trim_in(&s.project.timeline, track, ids[1], secs(13.0)).unwrap();
    s.apply(e).unwrap();
    let c = s.project.timeline.find_clip(ids[1]).unwrap().1;
    // A ripple trim anchors the LEFT EDGE on the timeline: the clip shortens
    // in place and everything after moves left. The gap before it is unchanged.
    assert_eq!(c.timeline_start, secs(10.0));
    assert_eq!(c.timeline_duration(), secs(7.0));
    // Trimming 3s off the head must consume 3s of source, not just move the clip.
    assert_eq!(c.source.start, src_before.start + secs(3.0));
    assert_eq!(c.source.duration, src_before.duration - secs(3.0));
    // Downstream followed.
    assert_eq!(s.project.timeline.find_clip(ids[2]).unwrap().1.timeline_start, secs(17.0));
    assert!(ops::gaps(&s.project.timeline, track).is_empty());
}

#[test]
fn a_trim_cannot_run_past_the_start_of_the_source() {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/a.mp4", secs(60.0)));
    let track = p.timeline.tracks[0].id;
    let mut s = Session::new(p);
    let cid = s.project.new_clip_id();
    // Source window starts at 1s, so only 1s of head-room exists.
    s.apply(Edit::AddClip { track, clip: Clip::new(cid, aid, secs(10.0),
        TimeRange::new(secs(1.0), secs(10.0))) }).unwrap();
    // Extending the head backwards by 5s needs 5s of source before the window,
    // and only 1s exists.
    assert!(ops::ripple_trim_in(&s.project.timeline, track, cid, secs(5.0)).is_err(),
            "extending 5s past a 1s head-room must be refused");
    assert!(ops::ripple_trim_in(&s.project.timeline, track, cid, secs(9.5)).is_ok());
}

// -------------------------------------------------------------------- relink

fn asset_with_hash(id: u64, path: &str, hash: Option<&str>, dur: Ticks) -> Asset {
    let mut a = Asset::new(AssetId(id), path, dur);
    a.content_hash = hash.map(|h| h.to_string());
    a
}

#[test]
fn identical_content_relinks_automatically() {
    let a = asset_with_hash(1, "/old/clip.mp4", Some("abc123"), secs(10.0));
    let cands = vec![
        Candidate { path: "/new/renamed.mp4".into(), content_hash: Some("abc123".into()),
                    duration_ticks: Some(secs(10.0).0) },
        Candidate { path: "/new/other.mp4".into(), content_hash: Some("zzz".into()),
                    duration_ticks: None },
    ];
    let m = relink::best_match(&a, &cands).unwrap();
    assert_eq!(m.quality, MatchQuality::ContentHash);
    assert!(m.quality.is_automatic());
    assert_eq!(m.path, std::path::PathBuf::from("/new/renamed.mp4"));
}

#[test]
fn a_name_match_always_needs_confirmation() {
    // This is how the wrong take silently ends up in someone's edit.
    let a = asset_with_hash(1, "/old/take.mp4", Some("abc"), secs(10.0));
    let cands = vec![Candidate { path: "/new/take.mp4".into(),
                                 content_hash: Some("different".into()),
                                 duration_ticks: Some(secs(10.0).0) }];
    let m = relink::best_match(&a, &cands).unwrap();
    assert_eq!(m.quality, MatchQuality::NameAndDuration);
    assert!(!m.quality.is_automatic(), "a same-name, same-length file is not proof");
}

#[test]
fn content_hash_beats_a_name_match() {
    let a = asset_with_hash(1, "/old/take.mp4", Some("abc"), secs(10.0));
    let cands = vec![
        Candidate { path: "/decoy/take.mp4".into(), content_hash: Some("nope".into()),
                    duration_ticks: Some(secs(10.0).0) },
        Candidate { path: "/real/unrelated_name.mp4".into(), content_hash: Some("abc".into()),
                    duration_ticks: Some(secs(10.0).0) },
    ];
    let m = relink::best_match(&a, &cands).unwrap();
    assert_eq!(m.path, std::path::PathBuf::from("/real/unrelated_name.mp4"));
}

#[test]
fn relinking_invalidates_the_stale_bookmark() {
    // A bookmark that still resolves would keep opening the OLD file.
    let mut p = Project::default();
    let id = p.new_asset_id();
    let mut a = Asset::new(id, "/old/x.mp4", secs(10.0));
    a.bookmark = Some("stale-bookmark-data".into());
    a.content_hash = Some("h".into());
    p.assets.push(a);

    let m = relink::RelinkMatch { asset: id, path: "/new/x.mp4".into(),
                                  quality: MatchQuality::ContentHash };
    assert!(relink::apply(&mut p, &m));
    let a = p.asset(id).unwrap();
    assert_eq!(a.path, "/new/x.mp4");
    assert!(a.bookmark.is_none(), "a stale bookmark would reopen the old file");
}

#[test]
fn only_automatic_matches_are_applied_unattended() {
    let mut p = Project::default();
    let a1 = p.new_asset_id();
    let a2 = p.new_asset_id();
    p.assets.push(asset_with_hash(a1.0, "/o/a.mp4", Some("h1"), secs(1.0)));
    p.assets.push(asset_with_hash(a2.0, "/o/b.mp4", Some("h2"), secs(1.0)));
    let matches = vec![
        relink::RelinkMatch { asset: a1, path: "/n/a.mp4".into(), quality: MatchQuality::ContentHash },
        relink::RelinkMatch { asset: a2, path: "/n/b.mp4".into(), quality: MatchQuality::NameOnly },
    ];
    let pending = relink::apply_automatic(&mut p, &matches);
    assert_eq!(p.asset(a1).unwrap().path, "/n/a.mp4");
    assert_eq!(p.asset(a2).unwrap().path, "/o/b.mp4", "a name-only match must wait for the user");
    assert_eq!(pending.len(), 1);
}

// --------------------------------------------------------------------- cache

#[test]
fn cache_keys_ignore_paths_and_separate_parameters() {
    let a = CacheKey::new(CacheClass::Proxy, "hash123", 0, "1280x720");
    let b = CacheKey::new(CacheClass::Proxy, "hash123", 0, "640x360");
    assert_ne!(a, b, "different parameters must be different entries");
    assert_eq!(a, CacheKey::new(CacheClass::Proxy, "hash123", 0, "1280x720"));
    assert!(a.storage_path().starts_with("proxies/"), "clearing a class must be one directory");
    assert!(!a.storage_path().contains('/') || a.storage_path().matches('/').count() == 1);
}

#[test]
fn entries_from_an_older_generator_are_stale() {
    let mut k = CacheKey::new(CacheClass::Waveform, "h", 0, "100bps");
    assert!(k.is_current());
    k.format_version = 0;
    assert!(!k.is_current(), "a changed generator must not read old entries");
    let entries = vec![CacheEntry { key: k, bytes: 10, last_used: 1 }];
    assert_eq!(stale_entries(&entries).len(), 1);
}

#[test]
fn eviction_is_lru_and_never_crosses_classes() {
    let budget = CacheBudget { thumbnails: 100, waveforms: 100, proxies: 100, render: 100 };
    let entries = vec![
        CacheEntry { key: CacheKey::new(CacheClass::Proxy, "a", 0, "p"), bytes: 60, last_used: 1 },
        CacheEntry { key: CacheKey::new(CacheClass::Proxy, "b", 0, "p"), bytes: 60, last_used: 5 },
        CacheEntry { key: CacheKey::new(CacheClass::Thumbnail, "c", 0, "t"), bytes: 90, last_used: 2 },
    ];
    let evict = evict_for_budget(&entries, CacheClass::Proxy, &budget);
    assert_eq!(evict.len(), 1);
    assert_eq!(evict[0].content_hash, "a", "least recently used should go first");
    // Thumbnails are inside their own budget and must be untouched.
    assert!(evict_for_budget(&entries, CacheClass::Thumbnail, &budget).is_empty());
}

#[test]
fn usage_is_reported_per_class() {
    let entries = vec![
        CacheEntry { key: CacheKey::new(CacheClass::Proxy, "a", 0, "p"), bytes: 1000, last_used: 1 },
        CacheEntry { key: CacheKey::new(CacheClass::Thumbnail, "b", 0, "t"), bytes: 10, last_used: 1 },
    ];
    let usage = usage_by_class(&entries);
    let proxies = usage.iter().find(|(c, _, _)| *c == CacheClass::Proxy).unwrap();
    assert_eq!((proxies.1, proxies.2), (1000, 1));
    // Proxies are the expensive thing to rebuild — eviction order reflects it.
    assert!(CacheClass::Proxy.rebuild_cost() > CacheClass::Thumbnail.rebuild_cost());
}

// -------------------------------------------------------------- render plans

#[test]
fn a_plan_names_what_to_decode_at_an_instant() {
    let (s, _track, ids) = three_clips();
    let plan = render::plan_at(&s.project.timeline, secs(15.0));
    assert_eq!(plan.video.len(), 1);
    assert_eq!(plan.video[0].clip, ids[1]);
    // 5s into a clip whose source window starts at 100s.
    assert_eq!(plan.video[0].source_time, secs(105.0));
    assert_eq!(plan.required_assets().len(), 1);
}

#[test]
fn overlays_composite_above_the_base_track() {
    let (mut s, _track, _ids) = three_clips();
    let overlay_track = s.project.timeline.tracks[1].id;
    let aid = s.project.assets[0].id;
    let ov = s.project.new_clip_id();
    s.apply(Edit::AddClip { track: overlay_track, clip: Clip::new(ov, aid, secs(12.0),
        TimeRange::new(Ticks::ZERO, secs(5.0))) }).unwrap();

    let plan = render::plan_at(&s.project.timeline, secs(14.0));
    assert_eq!(plan.video.len(), 2);
    assert!(!plan.video[0].is_overlay, "base track must be drawn first");
    assert!(plan.video[1].is_overlay);
}

#[test]
fn a_muted_track_contributes_nothing() {
    let (mut s, track, _ids) = three_clips();
    assert_eq!(render::plan_at(&s.project.timeline, secs(5.0)).video.len(), 1);
    s.apply(Edit::SetTrackMuted { track, from: false, to: true }).unwrap();
    assert!(render::plan_at(&s.project.timeline, secs(5.0)).is_empty());
}

#[test]
fn fades_produce_a_ramp_that_stays_in_range() {
    let (mut s, _track, ids) = three_clips();
    s.apply(Edit::SetFades { clip: ids[0], from: (Ticks::ZERO, Ticks::ZERO),
                             to: (secs(2.0), secs(2.0)) }).unwrap();
    let tl = &s.project.timeline;
    assert_eq!(render::plan_at(tl, Ticks::ZERO).video[0].opacity, 0.0);
    assert!((render::plan_at(tl, secs(1.0)).video[0].opacity - 0.5).abs() < 0.01);
    assert_eq!(render::plan_at(tl, secs(5.0)).video[0].opacity, 1.0);
    assert!((render::plan_at(tl, secs(9.0)).video[0].opacity - 0.5).abs() < 0.01);
    // Overlapping fades on a short clip must never go negative or exceed 1.
    for t in 0..100 {
        let o = render::plan_at(tl, secs(t as f64 * 0.1)).video.first().map(|l| l.opacity);
        if let Some(o) = o { assert!((0.0..=1.0).contains(&o), "opacity {o} out of range"); }
    }
}

#[test]
fn dirty_ranges_merge_and_stay_minimal() {
    let mut d = DirtyRanges::new();
    d.add(TimeRange::new(secs(0.0), secs(5.0)));
    d.add(TimeRange::new(secs(10.0), secs(5.0)));
    assert_eq!(d.as_slice().len(), 2, "disjoint edits stay separate");
    // Abutting spans are one span, not two.
    d.add(TimeRange::new(secs(5.0), secs(5.0)));
    assert_eq!(d.as_slice().len(), 1);
    assert_eq!(d.total(), secs(15.0));
    assert!(d.contains(secs(7.0)));
    assert!(!d.contains(secs(20.0)));
}

#[test]
fn editing_one_clip_dirties_only_its_span() {
    // §26: never re-render everything because one item changed.
    let (s, _track, ids) = three_clips();
    let r = render::dirty_for_clip(&s.project.timeline, ids[1]).unwrap();
    let mut d = DirtyRanges::new();
    d.add(r);
    assert!(d.intersects(TimeRange::new(secs(12.0), secs(1.0))));
    assert!(!d.intersects(TimeRange::new(secs(25.0), secs(1.0))),
            "an unrelated part of the timeline must stay clean");
}
