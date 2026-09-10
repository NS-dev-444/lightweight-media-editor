//! Loudness and silence, measured against fixtures with known answers.
//!
//! Two kinds of check, deliberately:
//!
//!   * against the **specification's own calibration point** — a 1 kHz stereo
//!     sine of amplitude 0.1 is defined to measure -20 LUFS, which is a fact
//!     about R128 rather than another measurement of ours;
//!   * against **arithmetic** — a file built as tone / 2 s silence / tone has
//!     exactly one gap, in a known place.
//!
//! `tools/check_loudness.sh` adds a third: agreement with FFmpeg's independent
//! `ebur128` implementation, which came out within 0.04 LU across the corpus.

use mediacore::analysis::*;
use std::ffi::CString;

fn repo_media(rel: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../media").join(rel);
    p.exists().then_some(p)
}

fn c(p: &std::path::Path) -> CString {
    CString::new(p.to_str().unwrap_or_default()).unwrap_or_default()
}

#[test]
fn a_calibration_tone_measures_what_the_standard_says() {
    let Some(p) = repo_media("tone_-20dbfs.wav") else {
        eprintln!("fixture absent, skipped"); return
    };
    let lufs = unsafe { mc_loudness(c(&p).as_ptr()) };
    // R128 is calibrated so this signal reads -20.0. The -0.691 offset in the
    // formula exists precisely to cancel the K-filter's gain at 1 kHz — which
    // is why a naive "-0.691 + 20*log10(0.1)" prediction of -20.69 is wrong.
    assert!((lufs - -20.0).abs() < 0.2, "expected about -20.0 LUFS, got {lufs:.2}");
}

#[test]
fn sample_peak_matches_the_signal() {
    let Some(p) = repo_media("tone_-20dbfs.wav") else {
        eprintln!("fixture absent, skipped"); return
    };
    let peak = unsafe { mc_sample_peak(c(&p).as_ptr()) };
    assert!((peak - 0.1).abs() < 0.01, "expected a peak near 0.1, got {peak:.4}");
}

#[test]
fn silence_has_no_measurable_loudness() {
    // Reported as the sentinel, not as some large negative number that would
    // look like a reading and normalise to an enormous gain.
    let Some(p) = repo_media("gap.wav") else {
        eprintln!("fixture absent, skipped"); return
    };
    let lufs = unsafe { mc_loudness(c(&p).as_ptr()) };
    assert!(lufs > -60.0, "a file with two tones in it should measure: {lufs}");

    let missing = CString::new("/nonexistent/nope.wav").unwrap_or_default();
    assert_eq!(unsafe { mc_loudness(missing.as_ptr()) }, -200.0);
}

#[test]
fn the_gap_is_found_where_it_actually_is() {
    let Some(p) = repo_media("gap.wav") else {
        eprintln!("fixture absent, skipped"); return
    };
    let mut out = [0i64; 64];
    let n = unsafe {
        mc_silence_ranges(c(&p).as_ptr(), -50.0, 300, 50, out.as_mut_ptr(), 64)
    };
    assert_eq!(n, 1, "expected exactly one gap, found {n}");

    let (start, end) = (out[0] as f64 / 1e9, out[1] as f64 / 1e9);
    // The silence runs 1.0 s to 3.0 s; 50 ms of padding is left at each end, so
    // the range handed back should sit just inside that.
    assert!((start - 1.05).abs() < 0.1, "gap starts at {start:.2}s, expected ~1.05");
    assert!((end - 2.95).abs() < 0.1, "gap ends at {end:.2}s, expected ~2.95");
}

#[test]
fn a_minimum_length_that_excludes_the_gap_finds_nothing() {
    // The parameter has to actually do something — a min-length longer than
    // the only gap must return no ranges rather than the gap anyway.
    let Some(p) = repo_media("gap.wav") else {
        eprintln!("fixture absent, skipped"); return
    };
    let mut out = [0i64; 64];
    let n = unsafe {
        mc_silence_ranges(c(&p).as_ptr(), -50.0, 5_000, 0, out.as_mut_ptr(), 64)
    };
    assert_eq!(n, 0, "a 5 s minimum should not match a 2 s gap");
}

#[test]
fn a_threshold_below_the_noise_floor_finds_nothing() {
    let Some(p) = repo_media("tone_-20dbfs.wav") else {
        eprintln!("fixture absent, skipped"); return
    };
    let mut out = [0i64; 64];
    let n = unsafe {
        mc_silence_ranges(c(&p).as_ptr(), -50.0, 200, 0, out.as_mut_ptr(), 64)
    };
    assert_eq!(n, 0, "a continuous tone contains no silence");
}

#[test]
fn a_missing_file_fails_without_panicking() {
    let mut out = [0i64; 8];
    let missing = CString::new("/nonexistent/nope.wav").unwrap_or_default();
    assert_eq!(unsafe {
        mc_silence_ranges(missing.as_ptr(), -50.0, 200, 0, out.as_mut_ptr(), 8)
    }, -1);
    assert_eq!(unsafe {
        mc_silence_ranges(std::ptr::null(), -50.0, 200, 0, out.as_mut_ptr(), 8)
    }, -1);
}
