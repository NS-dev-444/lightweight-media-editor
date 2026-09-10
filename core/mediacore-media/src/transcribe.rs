//! On-device transcription (§8's captions, O-16/O-18).
//!
//! ## Why whisper.cpp and not the platform's own recogniser
//!
//! macOS has `SFSpeechRecognizer`, which needs no model shipped. It was
//! rejected for three reasons, in order of weight:
//!
//! 1. **It can silently go to Apple's servers.** On-device recognition is a
//!    flag (`requiresOnDeviceRecognition`) that must be set and can be
//!    forgotten. whisper.cpp contains no network code at all, so "the audio
//!    never leaves this machine" is a property of the architecture rather than
//!    a promise about a boolean somebody has to remember.
//! 2. **It is macOS-only.** Windows is V1.1 (O-1), and this is one
//!    implementation for both rather than two.
//! 3. **It needs a permission prompt** to transcribe a file the user already
//!    has and already owns.
//!
//! Licensing was the precondition and is unusually clean: whisper.cpp is MIT,
//! and OpenAI released Whisper's code **and weights** under MIT. Weights being
//! MIT is what makes bundling one legal at all.
//!
//! ## Shape
//!
//! Stepwise, like conversion and export: the caller drives the loop, so
//! progress and cancellation stay on its side. whisper's own `full()` is not
//! incremental, so a step is one **chunk** of audio — which is necessary
//! regardless, because an hour of 16 kHz f32 is 230 MB and a long recording
//! should not have to fit in memory at once.

use std::ffi::{c_char, CString};
use std::ptr;
#[cfg(whisper)]
use std::ffi::{c_float, c_int, CStr};

/// Whisper wants 16 kHz mono f32. Not negotiable: the model was trained on it,
/// and feeding it anything else produces confident nonsense rather than an
/// error. `mc_audio_open` resamples, so this is free.
const WHISPER_RATE: i32 = 16_000;

/// How much audio to hand the model at once. Whisper's own window is 30 s.
#[cfg(whisper)]
const CHUNK_SECONDS: f64 = 25.0;

#[cfg(whisper)]
extern "C" {
    fn mcw_open(model_path: *const c_char, use_gpu: c_int) -> *mut std::ffi::c_void;
    fn mcw_close(w: *mut std::ffi::c_void);
    fn mcw_run(w: *mut std::ffi::c_void, samples: *const c_float, n: c_int,
               threads: c_int, lang: *const c_char, translate: c_int) -> c_int;
    fn mcw_n_segments(w: *mut std::ffi::c_void) -> c_int;
    fn mcw_seg_t0(w: *mut std::ffi::c_void, i: c_int) -> i64;
    fn mcw_seg_t1(w: *mut std::ffi::c_void, i: c_int) -> i64;
    fn mcw_seg_text(w: *mut std::ffi::c_void, i: c_int) -> *const c_char;
}

/// Fields the non-whisper build does not read: this type still exists there so
/// the C ABI is identical in both builds and the app needs no conditional code.
#[allow(dead_code)]
pub struct MCTranscribe {
    ctx: *mut std::ffi::c_void,
    reader: *mut crate::audio::MCAudio,
    /// Samples consumed so far, so a chunk knows where it sits in the file.
    consumed: i64,
    total_samples: i64,
    finished: bool,
    language: CString,
    translate: i32,
    /// Results, as (start ns, end ns, text).
    segments: Vec<(i64, i64, CString)>,
    last_error: CString,
}

/// 1 = more work, 0 = finished, negative = failed.
#[cfg(whisper)]
const MORE: i32 = 1;
const DONE: i32 = 0;

/// Open a transcription of `input` using the model at `model_path`.
///
/// `language` may be empty or "auto". `translate` asks for English output from
/// non-English speech, which is a different feature from transcribing and is
/// off unless asked for.
#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_open(input: *const c_char,
                                            model_path: *const c_char,
                                            language: *const c_char,
                                            translate: i32) -> *mut MCTranscribe {
    if input.is_null() || model_path.is_null() { return ptr::null_mut(); }

    #[cfg(not(whisper))]
    {
        let _ = (language, translate);
        return ptr::null_mut();
    }

    #[cfg(whisper)]
    {
        // Mono is what the model wants; the reader gives stereo, and the two
        // channels are averaged as they are read.
        let reader = crate::audio::mc_audio_open(input, WHISPER_RATE);
        if reader.is_null() { return ptr::null_mut(); }

        let ctx = mcw_open(model_path, 1);
        if ctx.is_null() {
            crate::audio::mc_audio_close(reader);
            return ptr::null_mut();
        }

        let lang = if language.is_null() { CString::default() }
                   else { CStr::from_ptr(language).to_owned() };

        Box::into_raw(Box::new(MCTranscribe {
            ctx, reader,
            consumed: 0,
            total_samples: 0,
            finished: false,
            language: lang,
            translate,
            segments: Vec::new(),
            last_error: CString::default(),
        }))
    }
}

/// Transcribe one chunk. 1 = more work, 0 = finished, negative = failed.
#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_step(t: *mut MCTranscribe) -> i32 {
    let Some(t) = t.as_mut() else { return -1 };
    if t.finished { return DONE; }

    #[cfg(not(whisper))]
    { t.finished = true; DONE }

    #[cfg(whisper)]
    {
        let want = (CHUNK_SECONDS * WHISPER_RATE as f64) as usize;
        let start_sample = t.consumed;

        let block = 4096usize;
        let mut scratch = vec![0f32; block * 2];
        let mut mono: Vec<f32> = Vec::with_capacity(want);
        while mono.len() < want {
            let got = crate::audio::mc_audio_read(t.reader, scratch.as_mut_ptr(), block as i32);
            if got <= 0 { break; }
            for i in 0..got as usize {
                mono.push((scratch[i * 2] + scratch[i * 2 + 1]) * 0.5);
            }
        }
        if mono.is_empty() { t.finished = true; return DONE; }
        let at_end = mono.len() < want;

        let rc = mcw_run(t.ctx, mono.as_ptr(), mono.len() as c_int,
                         num_threads(), t.language.as_ptr(), t.translate);
        if rc != 0 {
            t.last_error = CString::new("This audio could not be transcribed.")
                .unwrap_or_default();
            return -1;
        }

        // Whisper reports times in centiseconds, relative to the chunk.
        let base_ns = start_sample * 1_000_000_000 / WHISPER_RATE as i64;
        let mut found: Vec<(i64, i64, String)> = Vec::new();
        for i in 0..mcw_n_segments(t.ctx) {
            let text = CStr::from_ptr(mcw_seg_text(t.ctx, i)).to_string_lossy();
            let text = text.trim().to_string();
            if text.is_empty() { continue; }
            found.push((base_ns + mcw_seg_t0(t.ctx, i) * 10_000_000,
                        base_ns + mcw_seg_t1(t.ctx, i) * 10_000_000,
                        text));
        }

        // RESUME AT THE LAST COMPLETE SENTENCE, rather than overlapping by a
        // fixed amount and trimming.
        //
        // The fixed-overlap version dropped any segment starting inside the
        // overlap — and whisper's segment boundaries do not line up with the
        // chunk's, so a segment that began just inside the overlap and ran well
        // past it was discarded whole. A 60-second recording lost "sentence
        // number six" entirely, silently, at the seam. Resuming where the last
        // finished sentence ended means the next chunk starts on a clean
        // boundary and nothing is dropped at all.
        let mut resume_sample = start_sample + mono.len() as i64;
        if !at_end {
            // The final segment of a chunk is usually cut off mid-sentence, so
            // it is left for the next chunk to transcribe properly.
            if found.len() > 1 { found.pop(); }
            if let Some(last) = found.last() {
                let candidate = last.1 * WHISPER_RATE as i64 / 1_000_000_000;
                // Forward progress is not optional: a chunk whose last segment
                // ends where the chunk began would loop for ever.
                if candidate > start_sample + WHISPER_RATE as i64 {
                    resume_sample = candidate;
                }
            }
        }

        for (s, e, text) in found {
            if let Ok(c) = CString::new(text) { t.segments.push((s, e, c)); }
        }

        if at_end {
            t.consumed = start_sample + mono.len() as i64;
            t.finished = true;
            return DONE;
        }

        let resume_ns = resume_sample * 1_000_000_000 / WHISPER_RATE as i64;
        if crate::audio::mc_audio_seek(t.reader, resume_ns) != 0 {
            // Seeking back failed; carry on from where the read ended rather
            // than transcribing the same audio for ever.
            t.consumed = start_sample + mono.len() as i64;
        } else {
            t.consumed = resume_sample;
        }
        MORE
    }
}

#[cfg(whisper)]
fn num_threads() -> c_int {
    // Leave a core for the UI: a transcription that pins every core makes the
    // rest of the app stutter, and it is a background job.
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (n.saturating_sub(1).max(1)).min(8) as c_int
}

/// Seconds of audio transcribed so far, for progress against the file's length.
#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_seconds_done(t: *const MCTranscribe) -> f64 {
    t.as_ref().map(|t| t.consumed as f64 / WHISPER_RATE as f64).unwrap_or(0.0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_count(t: *const MCTranscribe) -> i32 {
    t.as_ref().map(|t| t.segments.len() as i32).unwrap_or(0)
}

/// Start of segment `i`, in nanoseconds.
#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_start_ns(t: *const MCTranscribe, i: i32) -> i64 {
    t.as_ref().and_then(|t| t.segments.get(i as usize)).map(|s| s.0).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_end_ns(t: *const MCTranscribe, i: i32) -> i64 {
    t.as_ref().and_then(|t| t.segments.get(i as usize)).map(|s| s.1).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_text(t: *const MCTranscribe, i: i32) -> *const c_char {
    match t.as_ref().and_then(|t| t.segments.get(i as usize)) {
        Some(s) => s.2.as_ptr(),
        None => c"".as_ptr(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_error(t: *const MCTranscribe) -> *const c_char {
    match t.as_ref() { Some(t) => t.last_error.as_ptr(), None => ptr::null() }
}

#[no_mangle]
pub unsafe extern "C" fn mc_transcribe_close(t: *mut MCTranscribe) -> i32 {
    if t.is_null() { return -1; }
    let t = Box::from_raw(t);
    #[cfg(whisper)]
    {
        if !t.ctx.is_null() { mcw_close(t.ctx); }
    }
    crate::audio::mc_audio_close(t.reader);
    0
}

/// Is on-device transcription available in this build?
///
/// Reported rather than assumed, so the UI can say "this build has no
/// transcription" instead of offering a button that silently does nothing.
#[no_mangle]
pub extern "C" fn mc_transcribe_available() -> i32 {
    if cfg!(whisper) { 1 } else { 0 }
}
