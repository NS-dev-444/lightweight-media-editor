// Transcribe a file from the command line, for tools/check_captions.sh.
fn main() {
    let mut a = std::env::args().skip(1);
    let (Some(input), Some(model)) = (a.next(), a.next()) else {
        eprintln!("usage: transcribe <audio-or-video> <model.bin> [lang]");
        std::process::exit(2);
    };
    let lang = a.next().unwrap_or_else(|| "auto".into());
    if mediacore::transcribe::mc_transcribe_available() == 0 {
        eprintln!("this build has no transcription (WHISPER_DIR was not set)");
        std::process::exit(3);
    }
    let ci = std::ffi::CString::new(input).unwrap_or_default();
    let cm = std::ffi::CString::new(model).unwrap_or_default();
    let cl = std::ffi::CString::new(lang).unwrap_or_default();
    unsafe {
        let t = mediacore::transcribe::mc_transcribe_open(ci.as_ptr(), cm.as_ptr(), cl.as_ptr(), 0);
        if t.is_null() { eprintln!("could not open"); std::process::exit(1); }
        let started = std::time::Instant::now();
        loop {
            let r = mediacore::transcribe::mc_transcribe_step(t);
            if r == 0 { break; }
            if r < 0 { eprintln!("failed"); break; }
        }
        let n = mediacore::transcribe::mc_transcribe_count(t);
        eprintln!("{n} segments in {:.1}s", started.elapsed().as_secs_f64());
        for i in 0..n {
            let s = mediacore::transcribe::mc_transcribe_start_ns(t, i) as f64 / 1e9;
            let e = mediacore::transcribe::mc_transcribe_end_ns(t, i) as f64 / 1e9;
            let txt = std::ffi::CStr::from_ptr(mediacore::transcribe::mc_transcribe_text(t, i));
            println!("[{s:7.2} -> {e:7.2}] {}", txt.to_string_lossy());
        }
        mediacore::transcribe::mc_transcribe_close(t);
    }
}
