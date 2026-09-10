//! §24 — autosave and crash recovery.
//!
//! §42 requires surviving termination during a save. The journal is designed
//! for exactly that: a crash mid-write leaves a truncated trailing line, which
//! recovery must skip rather than treating the whole journal as corrupt.

use mediacore_model::asset::*;
use mediacore_model::command::*;
use mediacore_model::project::*;
use mediacore_model::time::*;
use mediacore_model::timeline::*;
use std::io::Write;

fn secs(s: f64) -> Ticks { Ticks::from_seconds(s) }

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mcm_test_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn seeded() -> (Project, AssetId, TrackId) {
    let mut p = Project::default();
    let aid = p.new_asset_id();
    p.assets.push(Asset::new(aid, "/media/a.mp4", secs(60.0)));
    let track = p.timeline.tracks[0].id;
    (p, aid, track)
}

#[test]
fn edits_after_the_last_save_are_recovered() {
    let dir = tmpdir("recover");
    let path = dir.join("p.mcproj");
    let (project, aid, track) = seeded();

    {
        let mut s = Session::new(project).with_autosave(&path);
        s.autosave.as_mut().unwrap().checkpoint(&s.project).unwrap();  // full save
        // Three edits that exist ONLY in the journal.
        for i in 0..3u64 {
            let cid = s.project.new_clip_id();
            let clip = Clip::new(cid, aid, secs(i as f64 * 10.0),
                                 TimeRange::new(Ticks::ZERO, secs(5.0)));
            s.apply(Edit::AddClip { track, clip }).unwrap();
        }
        // Simulate a crash: no checkpoint, no clean exit.
    }

    let auto = Autosave::new(&path);
    assert!(auto.has_unrecovered_work(), "journal should signal unclean exit");
    let (recovered, applied, skipped) = auto.recover().unwrap();
    assert_eq!(applied, 3);
    assert_eq!(skipped, 0);
    assert_eq!(recovered.timeline.track(track).unwrap().clips.len(), 3);
    assert!(recovered.timeline.validate().is_empty());
}

#[test]
fn a_journal_truncated_mid_write_still_recovers_what_it_can() {
    let dir = tmpdir("truncated");
    let path = dir.join("p.mcproj");
    let (project, aid, track) = seeded();

    {
        let mut s = Session::new(project).with_autosave(&path);
        s.autosave.as_mut().unwrap().checkpoint(&s.project).unwrap();
        for i in 0..2u64 {
            let cid = s.project.new_clip_id();
            let clip = Clip::new(cid, aid, secs(i as f64 * 10.0),
                                 TimeRange::new(Ticks::ZERO, secs(5.0)));
            s.apply(Edit::AddClip { track, clip }).unwrap();
        }
    }

    // Power loss mid-append: a partial JSON line at the end.
    let jpath = path.with_extension("journal");
    let mut f = std::fs::OpenOptions::new().append(true).open(&jpath).unwrap();
    write!(f, "{{\"AddClip\":{{\"track\":1,\"cli").unwrap();
    drop(f);

    let auto = Autosave::new(&path);
    let (recovered, applied, skipped) = auto.recover().unwrap();
    assert_eq!(applied, 2, "complete entries should still be applied");
    assert_eq!(skipped, 1, "the torn line should be skipped, not fatal");
    assert_eq!(recovered.timeline.track(track).unwrap().clips.len(), 2);
}

#[test]
fn a_clean_save_clears_the_journal() {
    let dir = tmpdir("clean");
    let path = dir.join("p.mcproj");
    let (project, aid, track) = seeded();

    let mut s = Session::new(project).with_autosave(&path);
    s.autosave.as_mut().unwrap().checkpoint(&s.project).unwrap();
    let cid = s.project.new_clip_id();
    s.apply(Edit::AddClip { track,
        clip: Clip::new(cid, aid, Ticks::ZERO, TimeRange::new(Ticks::ZERO, secs(5.0))) }).unwrap();
    assert!(s.autosave.as_ref().unwrap().has_unrecovered_work());

    s.save(&path).unwrap();
    assert!(!s.autosave.as_ref().unwrap().has_unrecovered_work(),
            "a clean save must leave no recovery prompt behind");
    assert!(!s.is_dirty());

    let reloaded = Project::load(&path).unwrap();
    assert_eq!(reloaded.timeline.track(track).unwrap().clips.len(), 1);
}

#[test]
fn saving_is_atomic() {
    // AD-9: temp -> fsync -> rename. A reader must never see a partial file,
    // and no .tmp file may be left behind.
    let dir = tmpdir("atomic");
    let path = dir.join("p.mcproj");
    let (mut project, _aid, _t) = seeded();
    project.name = "First".into();
    project.save(&path).unwrap();

    project.name = "Second".into();
    project.save(&path).unwrap();

    assert_eq!(Project::load(&path).unwrap().name, "Second");
    assert!(!path.with_extension("tmp").exists(), "left a temp file behind");
}

#[test]
fn undo_survives_a_save_and_reload_as_a_fresh_history() {
    // Undo history is deliberately NOT persisted: §28 says undo must not
    // duplicate media, and a reopened project starting with a clean history is
    // the conventional and safer behaviour.
    let dir = tmpdir("history");
    let path = dir.join("p.mcproj");
    let (project, aid, track) = seeded();
    let mut s = Session::new(project);
    let cid = s.project.new_clip_id();
    s.apply(Edit::AddClip { track,
        clip: Clip::new(cid, aid, Ticks::ZERO, TimeRange::new(Ticks::ZERO, secs(5.0))) }).unwrap();
    s.save(&path).unwrap();

    let reopened = Session::new(Project::load(&path).unwrap());
    assert!(!reopened.history.can_undo());
    assert_eq!(reopened.project.timeline.track(track).unwrap().clips.len(), 1);
}
