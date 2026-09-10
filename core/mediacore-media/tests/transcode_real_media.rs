//! End-to-end: transcode real files and check the result is what was asked for.
//!
//! These assert on the OUTPUT FILE, not on return codes. A conversion that
//! reports success and writes an unplayable file is the failure mode that
//! matters, and only reading the result back catches it.
//!
//! Skips cleanly when the media fixtures are absent, so a fresh checkout does
//! not fail (the fixtures are generated, not committed).

use mediacore::audio::{mc_audio_close, mc_audio_open, mc_audio_read, mc_has_audio};
use mediacore::transcode::*;
use mediacore::{mc_close, mc_info, mc_open, MCInfo};
use std::ffi::CString;

fn repo_media(rel: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../media").join(rel);
    p.exists().then_some(p)
}

fn out_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
}

/// Read a file back the way the app would: open it, ask what it is.
/// `None` means the file is not decodable, which is the failure that matters.
fn info(path: &std::path::Path) -> Option<MCInfo> {
    let c = CString::new(path.to_str().unwrap_or_default()).unwrap_or_default();
    unsafe {
        let d = mc_open(c.as_ptr());
        if d.is_null() { return None; }
        let mut i = MCInfo::default();
        let ok = mc_info(d, &mut i) == 0;
        mc_close(d);
        ok.then_some(i)
    }
}

fn has_sound(path: &std::path::Path) -> bool {
    let c = CString::new(path.to_str().unwrap_or_default()).unwrap_or_default();
    unsafe { mc_has_audio(c.as_ptr()) != 0 }
}

/// RMS of the first second of audio.
///
/// "Has an audio stream" is a weaker claim than it looks: a broken resampler
/// writes a full-length track of digital silence and every stream check still
/// passes. Reading the samples back is the only thing that catches it.
fn audio_rms(path: &std::path::Path) -> f32 {
    let c = CString::new(path.to_str().unwrap_or_default()).unwrap_or_default();
    unsafe {
        let a = mc_audio_open(c.as_ptr(), 48_000);
        if a.is_null() { return 0.0; }
        let mut buf = vec![0f32; 48_000 * 2];
        let got = mc_audio_read(a, buf.as_mut_ptr(), 48_000);
        mc_audio_close(a);
        if got <= 0 { return 0.0; }
        let n = got as usize * 2;
        (buf[..n].iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt()
    }
}

/// Run a whole conversion. Returns the number of steps, or an error string.
fn run(input: &std::path::Path, output: &std::path::Path,
       codec: i32, kbps: i32, w: i32, h: i32) -> Result<i64, String> {
    let _ = std::fs::remove_file(output);
    let ci = CString::new(input.to_str().unwrap_or_default()).unwrap_or_default();
    let co = CString::new(output.to_str().unwrap_or_default()).unwrap_or_default();
    unsafe {
        let c = mc_video_convert_open(ci.as_ptr(), co.as_ptr(), codec, kbps, w, h);
        if c.is_null() { return Err("could not open the conversion".into()); }
        let mut steps = 0i64;
        loop {
            let r = mc_video_convert_step(c);
            if r == 0 { break; }
            if r < 0 {
                let e = mc_video_convert_error(c);
                let msg = if e.is_null() { "unknown".to_string() }
                          else { std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned() };
                mc_video_convert_close(c);
                return Err(msg);
            }
            steps += 1;
            // A runaway loop should fail the test, not hang CI.
            if steps > 2_000_000 { mc_video_convert_close(c); return Err("did not terminate".into()); }
        }
        let frames = mc_video_convert_frames(c);
        mc_video_convert_close(c);
        Ok(frames)
    }
}

#[test]
fn transcode_preserves_duration_and_sound() {
    let Some(src) = repo_media("av_sync.mp4") else {
        eprintln!("fixture absent, skipped"); return
    };
    let before = info(&src).expect("the fixture is not readable");
    assert!(has_sound(&src), "the fixture has no sound to preserve");

    let out = out_path("mc_transcode_same_size.mp4");
    let frames = run(&src, &out, 0, 0, 0, 0).expect("transcode failed");
    assert!(frames > 0, "no frames were encoded");

    let after = info(&out).expect("the output is not readable");
    assert_eq!((after.width, after.height), (before.width, before.height),
               "size changed when it was not asked to");
    assert!(has_sound(&out), "the sound was lost");

    // Durations are compared loosely: an encoder may hold a few frames, and a
    // fraction of a second either way is not a defect.
    let (d0, d1) = (before.duration_sec, after.duration_sec);
    assert!((d0 - d1).abs() < 0.5, "duration drifted: {d0} -> {d1}");
}

#[test]
fn resize_scales_and_keeps_aspect() {
    let Some(src) = repo_media("corpus/screencast_1080.mp4") else {
        eprintln!("fixture absent, skipped"); return
    };
    let before = info(&src).expect("the fixture is not readable");
    let out = out_path("mc_transcode_720.mp4");
    run(&src, &out, 0, 0, 0, 720).expect("resize failed");

    let after = info(&out).expect("the output is not readable");
    assert_eq!(after.height, 720, "did not scale to the requested height");
    // Width follows the source aspect, rounded down to an even number.
    let expect_w = ((720i64 * before.width as i64 / before.height as i64) as i32) & !1;
    assert_eq!(after.width, expect_w, "aspect ratio was not preserved");
}

#[test]
fn compress_produces_a_smaller_file() {
    let Some(src) = repo_media("corpus/screencast_1080.mp4") else {
        eprintln!("fixture absent, skipped"); return
    };
    let original = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
    assert!(original > 0);

    let out = out_path("mc_transcode_small.mp4");
    run(&src, &out, 0, 800, 0, 0).expect("compress failed");

    info(&out).expect("the compressed file is not readable");
    let smaller = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(u64::MAX);
    assert!(smaller < original,
            "asking for 800 kbps produced {smaller} bytes against {original}");
}

#[test]
fn a_missing_file_fails_without_panicking() {
    let out = out_path("mc_transcode_never.mp4");
    let r = run(std::path::Path::new("/nonexistent/nope.mp4"), &out, 0, 0, 0, 0);
    assert!(r.is_err(), "a missing input should not succeed");
    assert!(!out.exists(), "a failed conversion left a file behind");
}


#[test]
fn sound_survives_a_resize() {
    // Resizing takes the video through decode/scale/encode while the audio is
    // copied alongside it. The two paths run in one interleaved loop, and
    // getting that wrong loses the sound rather than erroring.
    let Some(src) = repo_media("av_sync.mp4") else {
        eprintln!("fixture absent, skipped"); return
    };
    let out = out_path("mc_transcode_resized_sound.mp4");
    run(&src, &out, 0, 0, 640, 0).expect("resize failed");

    let after = info(&out).expect("the output is not readable");
    assert_eq!(after.width, 640, "did not scale to the requested width");
    assert_eq!(after.height, 360, "aspect ratio was not preserved");
    assert!(has_sound(&out), "resizing lost the sound");
}

#[test]
fn h264_is_selectable() {
    // AD-12 ships both hardware encoders. HEVC is the default for quality, but
    // H.264 is what older players and phones actually accept, so the choice has
    // to work rather than merely exist.
    let Some(src) = repo_media("av_sync.mp4") else {
        eprintln!("fixture absent, skipped"); return
    };
    let out = out_path("mc_transcode_h264.mp4");
    let frames = run(&src, &out, 1, 0, 0, 0).expect("h264 transcode failed");
    assert!(frames > 0, "no frames were encoded");
    info(&out).expect("the H.264 output is not readable");
}


#[test]
fn audio_is_re_encoded_when_the_container_will_not_carry_it() {
    // MP4 cannot hold PCM, so this source cannot take the copy path — it has to
    // decode, resample and re-encode to AAC. That is the half of the audio code
    // av_sync.mp4 never reaches.
    let Some(src) = repo_media("av_pcm.mkv") else {
        eprintln!("fixture absent, skipped"); return
    };
    let loudness_in = audio_rms(&src);
    assert!(loudness_in > 0.01, "the fixture is silent: {loudness_in}");

    let out = out_path("mc_transcode_reencoded_audio.mp4");
    run(&src, &out, 0, 0, 0, 0).expect("transcode failed");

    let after = info(&out).expect("the output is not readable");
    assert_eq!((after.width, after.height), (640, 480));
    assert!(has_sound(&out), "the re-encoded sound is missing");

    // A 440 Hz tone through AAC keeps its level; anything near zero means the
    // resampler wrote silence and every stream-level check would still pass.
    let loudness_out = audio_rms(&out);
    let ratio = loudness_out / loudness_in;
    assert!((0.5..2.0).contains(&ratio),
            "loudness changed by {ratio:.2}x ({loudness_in} -> {loudness_out})");
}

#[test]
fn copied_audio_keeps_its_level() {
    let Some(src) = repo_media("av_sync.mp4") else {
        eprintln!("fixture absent, skipped"); return
    };
    let out = out_path("mc_transcode_copied_audio.mp4");
    run(&src, &out, 0, 0, 0, 0).expect("transcode failed");
    let ratio = audio_rms(&out) / audio_rms(&src).max(f32::MIN_POSITIVE);
    assert!((0.5..2.0).contains(&ratio), "copied audio changed level by {ratio:.2}x");
}

#[test]
fn this_build_has_no_still_image_encoder() {
    // Recorded deliberately, because it decided where Extract Frames lives.
    //
    // The LGPL build disables autodetect and does not include PNG or MJPEG, so
    // writing stills through FFmpeg is not available here. That matches what
    // Phase 3 already found from the other direction — FFmpeg reported
    // "unspecified size" for a perfectly good PNG, and ImageIO replaced it for
    // reading. Stills go through the platform's imaging framework in both
    // directions, and this test fails loudly if a future FFmpeg build silently
    // changes the premise.
    unsafe {
        let png = rusty_ffmpeg::ffi::avcodec_find_encoder(rusty_ffmpeg::ffi::AV_CODEC_ID_PNG);
        assert!(png.is_null(),
                "this build now has a PNG encoder; revisit where Extract Frames lives");
    }
}

// ---------------------------------------------------------------- merge

mod merge_tests {
    use super::*;
    use mediacore::merge::*;

    fn run_merge(inputs: &[std::path::PathBuf], out: &std::path::Path)
        -> Result<i32, String>
    {
        let _ = std::fs::remove_file(out);
        let cs: Vec<CString> = inputs.iter()
            .map(|p| CString::new(p.to_str().unwrap_or_default()).unwrap_or_default())
            .collect();
        let ptrs: Vec<*const std::os::raw::c_char> = cs.iter().map(|c| c.as_ptr()).collect();
        let co = CString::new(out.to_str().unwrap_or_default()).unwrap_or_default();
        unsafe {
            let m = mc_merge_open(ptrs.as_ptr(), ptrs.len() as i32, co.as_ptr());
            if m.is_null() { return Err("could not open the merge".into()); }
            let mode = mc_merge_mode(m);
            let mut steps = 0i64;
            loop {
                let r = mc_merge_step(m);
                if r == 0 { break; }
                if r < 0 {
                    let e = mc_merge_error(m);
                    let msg = if e.is_null() { "unknown".to_string() }
                              else { std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned() };
                    mc_merge_close(m);
                    return Err(msg);
                }
                steps += 1;
                if steps > 2_000_000 { mc_merge_close(m); return Err("did not terminate".into()); }
            }
            mc_merge_close(m);
            Ok(mode)
        }
    }

    #[test]
    fn identical_files_are_joined_by_copying() {
        let Some(a) = repo_media("av_sync.mp4") else {
            eprintln!("fixture absent, skipped"); return
        };
        let one = info(&a).expect("fixture readable");
        let out = out_path("mc_merge_copy.mp4");
        let mode = run_merge(&[a.clone(), a.clone()], &out).expect("merge failed");
        assert_eq!(mode, MC_MERGE_COPY, "identical files should not be re-encoded");

        let joined = info(&out).expect("the merged file is not readable");
        assert_eq!((joined.width, joined.height), (one.width, one.height));
        // Two copies of a 5 s clip make a 10 s one. Getting the timestamp
        // offset wrong stacks them and the duration stays at 5.
        assert!((joined.duration_sec - one.duration_sec * 2.0).abs() < 0.5,
                "expected ~{:.1}s, got {:.1}s", one.duration_sec * 2.0, joined.duration_sec);
        assert!(has_sound(&out), "the merged file lost its sound");
    }

    #[test]
    fn differently_shaped_files_are_re_encoded() {
        // 1280x720 and 640x480: a copy would produce a file that changes size
        // halfway through, which most players simply refuse.
        let (Some(a), Some(b)) = (repo_media("av_sync.mp4"), repo_media("av_pcm.mkv")) else {
            eprintln!("fixtures absent, skipped"); return
        };
        let copyable = unsafe {
            let ca = CString::new(a.to_str().unwrap_or_default()).unwrap_or_default();
            let cb = CString::new(b.to_str().unwrap_or_default()).unwrap_or_default();
            let ptrs = [ca.as_ptr(), cb.as_ptr()];
            mc_merge_would_copy(ptrs.as_ptr(), 2)
        };
        assert_eq!(copyable, 0, "these two should not be copyable");

        let out = out_path("mc_merge_encode.mp4");
        let mode = run_merge(&[a.clone(), b.clone()], &out).expect("merge failed");
        assert_eq!(mode, MC_MERGE_ENCODE);

        let joined = info(&out).expect("the merged file is not readable");
        let first = info(&a).expect("fixture readable");
        assert_eq!((joined.width, joined.height), (first.width, first.height),
                   "everything should come to the first file's size");
        // Tight on purpose. A loose tolerance hid a real defect: the second
        // clip was 25 fps and came out 20% short, because output frames were
        // counted rather than placed by their source timestamps.
        let expected = first.duration_sec + info(&b).expect("fixture").duration_sec;
        assert!((joined.duration_sec - expected).abs() < 0.25,
                "expected ~{expected:.2}s, got {:.2}s — segments joined at the \
                 wrong speed", joined.duration_sec);
    }

    #[test]
    fn a_single_file_still_merges() {
        let Some(a) = repo_media("av_sync.mp4") else {
            eprintln!("fixture absent, skipped"); return
        };
        let out = out_path("mc_merge_one.mp4");
        run_merge(&[a.clone()], &out).expect("merging one file should work");
        let one = info(&a).expect("fixture");
        let joined = info(&out).expect("output readable");
        assert!((joined.duration_sec - one.duration_sec).abs() < 0.5);
    }

    #[test]
    fn an_unreadable_file_fails_before_anything_is_written() {
        // Discovering the second file is broken halfway through would leave a
        // half-made video behind; every input is checked up front.
        let Some(a) = repo_media("av_sync.mp4") else {
            eprintln!("fixture absent, skipped"); return
        };
        let out = out_path("mc_merge_never.mp4");
        let missing = std::path::PathBuf::from("/nonexistent/nope.mp4");
        assert!(run_merge(&[a, missing], &out).is_err());
        assert!(!out.exists(), "a failed merge left a file behind");
    }
}
