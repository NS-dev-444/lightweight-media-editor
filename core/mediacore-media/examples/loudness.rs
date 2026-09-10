// Prints a file's integrated loudness, for tools/check_loudness.sh.
fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: loudness <file>");
        std::process::exit(2);
    };
    let c = std::ffi::CString::new(path).unwrap_or_default();
    let lufs = unsafe { mediacore::analysis::mc_loudness(c.as_ptr()) };
    println!("{lufs:.2}");
}
