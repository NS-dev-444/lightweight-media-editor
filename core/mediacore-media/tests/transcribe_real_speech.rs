//! Transcription, measured against speech whose words are known exactly.
//!
//! The fixture is synthesised with macOS `say`, which matters twice: the
//! expected transcript is not a judgement call, so **word accuracy is a number**
//! rather than an opinion; and nothing anybody said was recorded to make it.
//!
//! Every test skips cleanly when the model or the whisper build is absent, so a
//! checkout that has not run `third_party/whisper/build.sh` still passes.

use mediacore::transcribe::*;
use std::ffi::{CStr, CString};

fn repo(rel: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel);
    p.exists().then_some(p)
}

const SPOKEN: &str = "The quick brown fox jumps over the lazy dog. \
                      Video editing should be simple and fast. \
                      This sentence is here to test the caption timing.";

struct Result_ {
    segments: Vec<(f64, f64, String)>,
}

fn transcribe(file: &str) -> Option<Result_> {
    if mc_transcribe_available() == 0 {
        eprintln!("no transcription in this build, skipped");
        return None;
    }
    let (input, model) = (repo(file)?, repo("build/models/ggml-base-q5_1.bin")?);
    let ci = CString::new(input.to_str()?).ok()?;
    let cm = CString::new(model.to_str()?).ok()?;
    let cl = CString::new("en").ok()?;
    unsafe {
        let t = mc_transcribe_open(ci.as_ptr(), cm.as_ptr(), cl.as_ptr(), 0);
        if t.is_null() { return None; }
        let mut guard = 0;
        loop {
            let r = mc_transcribe_step(t);
            if r <= 0 { break; }
            guard += 1;
            if guard > 10_000 { break; }
        }
        let n = mc_transcribe_count(t);
        let segments = (0..n).map(|i| (
            mc_transcribe_start_ns(t, i) as f64 / 1e9,
            mc_transcribe_end_ns(t, i) as f64 / 1e9,
            CStr::from_ptr(mc_transcribe_text(t, i)).to_string_lossy().into_owned(),
        )).collect();
        mc_transcribe_close(t);
        Some(Result_ { segments })
    }
}

/// Lowercase words, punctuation stripped — transcription is judged on words.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

#[test]
fn known_speech_is_transcribed_accurately() {
    let Some(r) = transcribe("media/speech.wav") else { return };
    assert!(!r.segments.is_empty(), "nothing was transcribed at all");

    let got = words(&r.segments.iter().map(|s| s.2.clone()).collect::<Vec<_>>().join(" "));
    let want = words(SPOKEN);

    // Word accuracy, not exact equality: a small model on synthesised speech is
    // allowed the odd mistake, and a test that demands perfection would fail on
    // a model upgrade that is otherwise an improvement.
    let matched = want.iter().filter(|w| got.contains(w)).count();
    let accuracy = matched as f64 / want.len() as f64;
    assert!(accuracy > 0.9,
            "word accuracy {:.0}% — expected >90%\n  wanted: {want:?}\n  got:    {got:?}",
            accuracy * 100.0);
}

#[test]
fn segments_are_in_order_and_within_the_recording() {
    let Some(r) = transcribe("media/speech.wav") else { return };
    // The fixture is about 7.6 s. Timings outside it, or going backwards, mean
    // the chunk-offset arithmetic is wrong — which is the failure mode that
    // produces captions drifting further out of sync the longer a video runs.
    let mut previous = -1.0;
    for (start, end, text) in &r.segments {
        assert!(*start >= previous, "segment starts went backwards at {text:?}");
        assert!(end > start, "segment {text:?} has no duration");
        assert!(*start >= 0.0 && *end < 12.0,
                "segment {text:?} is at {start:.2}-{end:.2}s, outside a 7.6 s file");
        previous = *start;
    }
}

#[test]
fn captions_cover_most_of_the_speech() {
    let Some(r) = transcribe("media/speech.wav") else { return };
    // Continuous speech should produce captions covering most of the file. Big
    // gaps mean whole chunks were dropped — which the overlap-trimming logic
    // could plausibly cause, and which nothing else here would catch.
    let covered: f64 = r.segments.iter().map(|(s, e, _)| e - s).sum();
    assert!(covered > 5.0,
            "only {covered:.1}s of a 7.6 s recording produced captions");
}

#[test]
fn a_missing_model_fails_cleanly() {
    if mc_transcribe_available() == 0 { return }
    let Some(input) = repo("media/speech.wav") else { return };
    let ci = CString::new(input.to_str().unwrap_or_default()).unwrap_or_default();
    let bad = CString::new("/nonexistent/model.bin").unwrap_or_default();
    let lang = CString::new("en").unwrap_or_default();
    let t = unsafe { mc_transcribe_open(ci.as_ptr(), bad.as_ptr(), lang.as_ptr(), 0) };
    assert!(t.is_null(), "a missing model must not produce a working transcription");
}

#[test]
fn a_file_with_no_speech_produces_no_captions() {
    // Silence must yield nothing, not a caption saying "[BLANK_AUDIO]" — which
    // is exactly what whisper emits by default and what suppress_nst is for.
    let Some(r) = transcribe("media/gap.wav") else { return };
    for (_, _, text) in &r.segments {
        let lower = text.to_lowercase();
        assert!(!lower.contains("blank") && !lower.contains("silence"),
                "a non-speech token reached the captions: {text:?}");
    }
}

#[test]
fn nothing_is_lost_at_a_chunk_boundary() {
    // The regression this exists for: transcription hands the model 25 seconds
    // at a time, and the first implementation overlapped chunks by a fixed two
    // seconds and dropped any segment starting inside the overlap. Whisper's
    // segment boundaries do not line up with the chunk's, so a segment that
    // began just inside the overlap and ran well past it was discarded whole —
    // and "sentence number six" of this fixture vanished without a trace.
    //
    // Numbered sentences make the loss visible: a missing number is a missing
    // sentence, and no amount of word-accuracy averaging would have shown it.
    let Some(r) = transcribe("media/speech_long.wav") else { return };
    let all = r.segments.iter().map(|s| s.2.clone()).collect::<Vec<_>>().join(" ").to_lowercase();

    let spelled = ["one", "two", "three", "four", "five", "six",
                   "seven", "eight", "nine", "ten", "eleven", "twelve"];
    let mut missing = Vec::new();
    for (i, word) in spelled.iter().enumerate() {
        let n = i + 1;
        // The model writes larger numbers as digits ("number 10"), so accept
        // either form rather than failing on a formatting choice.
        if !all.contains(&format!("number {word}")) && !all.contains(&format!("number {n}")) {
            missing.push(n);
        }
    }
    assert!(missing.is_empty(), "sentences {missing:?} were lost\n{all}");
}

#[test]
fn a_long_recording_is_covered_end_to_end() {
    let Some(r) = transcribe("media/speech_long.wav") else { return };
    let last_end = r.segments.iter().map(|s| s.1).fold(0.0, f64::max);
    // The fixture is about 60 s. Stopping well short means a chunk failed to
    // advance, or the loop exited early.
    assert!(last_end > 55.0, "captions stop at {last_end:.1}s of a 60 s recording");
    assert!(last_end < 65.0, "captions run past the end of the recording: {last_end:.1}s");

    // And no gap larger than a few seconds, since the speech is continuous.
    let mut sorted: Vec<_> = r.segments.iter().map(|(s, e, _)| (*s, *e)).collect();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut covered_to = 0.0f64;
    for (s, e) in sorted {
        assert!(s - covered_to < 4.0,
                "a {:.1}s hole before {s:.1}s — a chunk was dropped", s - covered_to);
        covered_to = covered_to.max(e);
    }
}
