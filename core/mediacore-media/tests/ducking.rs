//! Ducking, checked by reading the gain the render plan actually produces.
//!
//! Asserting on the plan rather than on the curve is the point: the plan is
//! what the mixer and the exporter consume, so a curve that is right but is not
//! reaching them would pass a test on `gain_points` and still be silent in the
//! product.
//!
//! `media/gap.wav` stands in for a voice take: tone from 0-1 s, two seconds of
//! silence, tone from 3-4 s. Ducking should therefore pull the music down over
//! the two tones and let it back up in the middle.

use mediacore::session::*;
use std::ffi::CString;

fn repo_media(rel: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../media").join(rel);
    p.exists().then_some(p)
}

const TICKS_PER_SECOND: i64 = 705_600_000;
fn secs(s: f64) -> i64 { (s * TICKS_PER_SECOND as f64) as i64 }

/// The audio gain the plan reports at a time, for a given clip.
unsafe fn gain_at(s: *const MCSession, clip: u64, t: f64) -> Option<f64> {
    let mut out = vec![MCPlanLayer::default(); 16];
    let n = mcs_plan_at(s, secs(t), out.as_mut_ptr(), 16);
    (0..n as usize).find(|&i| out[i].clip_id == clip && out[i].kind == 1)
        .map(|i| out[i].gain)
}

struct Fixture {
    s: *mut MCSession,
    music: u64,
    voice: u64,
}

unsafe fn build() -> Option<Fixture> {
    let music_path = repo_media("test_60min.mp3")?;
    let voice_path = repo_media("gap.wav")?;

    let s = mcs_new();
    if s.is_null() { return None; }

    let cm = CString::new(music_path.to_str()?).ok()?;
    let cv = CString::new(voice_path.to_str()?).ok()?;
    let music_asset = mcs_import(s, cm.as_ptr());
    let voice_asset = mcs_import(s, cv.as_ptr());
    assert!(music_asset != 0 && voice_asset != 0, "the fixtures did not import");

    // Two audio tracks, so the clips overlap instead of queueing up.
    let a1 = mcs_track_id_at(s, 3);
    let a2 = mcs_track_id_at(s, 4);
    assert!(a1 != 0 && a2 != 0, "V1 should have two audio tracks");

    let music = mcs_add_clip(s, a1, music_asset, 0, 0, secs(10.0));
    let voice = mcs_add_clip(s, a2, voice_asset, 0, 0, secs(4.0));
    assert!(music != 0 && voice != 0, "clips were not added");
    Some(Fixture { s, music, voice })
}

#[test]
fn music_steps_back_while_the_voice_speaks() {
    unsafe {
        let Some(f) = build() else { eprintln!("fixtures absent, skipped"); return };
        let n = mcs_duck(f.s, f.music, f.voice, -12.0, 200);
        assert!(n >= 2, "expected at least two spoken passages, found {n}");

        let quiet_1 = gain_at(f.s, f.music, 0.5).expect("music playing at 0.5s");
        let loud    = gain_at(f.s, f.music, 2.0).expect("music playing at 2.0s");
        let quiet_2 = gain_at(f.s, f.music, 3.5).expect("music playing at 3.5s");

        // -12 dB is a quarter of the amplitude.
        let expected = 10f64.powf(-12.0 / 20.0);
        assert!((quiet_1 - expected).abs() < 0.05,
                "over the first tone the music should be ducked, got {quiet_1:.3}");
        assert!((quiet_2 - expected).abs() < 0.05,
                "over the second tone the music should be ducked, got {quiet_2:.3}");
        assert!(loud > 0.9,
                "in the silence between them the music should come back, got {loud:.3}");

        mcs_free(f.s);
    }
}

#[test]
fn the_music_comes_back_gradually_rather_than_jumping() {
    // A step change pumps audibly. The ramp is the difference between ducking
    // that sounds professional and ducking that sounds broken.
    unsafe {
        let Some(f) = build() else { eprintln!("fixtures absent, skipped"); return };
        assert!(mcs_duck(f.s, f.music, f.voice, -12.0, 400) >= 2);

        // 1.0 s is where the first tone ends; with a 400 ms ramp the music
        // should be on its way up but not yet back.
        let at_end = gain_at(f.s, f.music, 1.0).expect("music at 1.0s");
        let mid_ramp = gain_at(f.s, f.music, 1.2).expect("music at 1.2s");
        let after = gain_at(f.s, f.music, 1.5).expect("music at 1.5s");
        assert!(mid_ramp > at_end, "the ramp does not rise: {at_end:.3} -> {mid_ramp:.3}");
        assert!(after > mid_ramp, "the ramp does not finish: {mid_ramp:.3} -> {after:.3}");
        mcs_free(f.s);
    }
}

#[test]
fn ducking_is_one_undo() {
    unsafe {
        let Some(f) = build() else { eprintln!("fixtures absent, skipped"); return };
        let before = gain_at(f.s, f.music, 0.5).expect("music at 0.5s");
        assert!(mcs_duck(f.s, f.music, f.voice, -12.0, 200) >= 2);
        assert!(gain_at(f.s, f.music, 0.5).expect("ducked") < before);

        // §28: forty curve points are still ONE thing the user did.
        assert_eq!(mcs_undo(f.s), 0);
        let after = gain_at(f.s, f.music, 0.5).expect("music at 0.5s");
        assert!((after - before).abs() < 1e-9, "undo did not restore the level");
        mcs_free(f.s);
    }
}

#[test]
fn ducking_against_nothing_fails_cleanly() {
    unsafe {
        let Some(f) = build() else { eprintln!("fixtures absent, skipped"); return };
        // A clip id that does not exist must not panic or corrupt the session.
        assert_eq!(mcs_duck(f.s, f.music, 999_999, -12.0, 200), -1);
        assert_eq!(mcs_gain_point_count(f.s, f.music), 0,
                   "a failed duck should leave no curve behind");
        mcs_free(f.s);
    }
}
