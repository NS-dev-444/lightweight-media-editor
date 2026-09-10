// Regenerate the C ABI header on every build, so it can never drift from the
// Rust definitions. See docs/PHASE_1_RESULTS.md — a hand-maintained header
// already fell out of sync once during Phase 1.
fn main() {
    // Watch the WHOLE src directory, not just lib.rs. Watching one file meant
    // adding a function in session.rs left the generated header stale, and the
    // failure looked like "cannot find mcs_asset_path in scope" -- a confusing
    // symptom for a build-cache cause.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    // Our FFmpeg dylibs carry @rpath install names (required for .app
    // bundling), so anything that links them needs a matching rpath. Without
    // this, `cargo test` builds fine and then aborts at load time.
    if let Ok(libs) = std::env::var("FFMPEG_LIBS_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{libs}");
    }

    // --- whisper.cpp, for on-device transcription (§8) ---------------------
    //
    // OPTIONAL. When WHISPER_DIR is absent the crate still builds and
    // transcription reports itself unavailable, which keeps `cargo test`
    // working on a machine that has not run third_party/whisper/build.sh — and
    // keeps the Windows bring-up (WINDOWS_BRINGUP.md) able to proceed one
    // dependency at a time instead of all at once.
    // Declare the cfg, or every use of it is a warning (§46 Rule 4: zero).
    println!("cargo:rustc-check-cfg=cfg(whisper)");
    println!("cargo:rerun-if-env-changed=WHISPER_DIR");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo:rerun-if-changed=csrc/whisper_shim.c");
    if let Ok(dir) = std::env::var("WHISPER_DIR") {
        let inc = format!("{dir}/include");
        let lib = format!("{dir}/lib");
        if std::path::Path::new(&inc).join("whisper.h").exists() {
            // `cc` is invoked directly rather than through the `cc` crate: it
            // is three lines here against a new build dependency to audit, and
            // this project keeps its dependency list short on purpose.
            let out = std::env::var("OUT_DIR").unwrap_or_default();
            let obj = format!("{out}/whisper_shim.o");
            // The macOS floor, explicitly. Relying on MACOSX_DEPLOYMENT_TARGET
            // in the environment is not enough: build.rs is cached, so an
            // object compiled before the floor existed survives and the linker
            // warns that one object is newer than everything else.
            let min = std::env::var("MACOSX_DEPLOYMENT_TARGET")
                .unwrap_or_else(|_| "14.0".into());
            let min_flag = format!("-mmacosx-version-min={min}");
            let status = std::process::Command::new("cc")
                .args(["-c", "csrc/whisper_shim.c", "-O2", "-fPIC", &min_flag,
                       "-I", &inc, "-o", &obj])
                .status();
            match status {
                Ok(s) if s.success() => {
                    std::process::Command::new("ar")
                        .args(["crs", &format!("{out}/libwhisper_shim.a"), &obj])
                        .status().ok();
                    println!("cargo:rustc-link-search=native={out}");
                    println!("cargo:rustc-link-lib=static=whisper_shim");
                    println!("cargo:rustc-link-search=native={lib}");
                    for l in ["whisper", "ggml", "ggml-base", "ggml-cpu",
                              "ggml-metal", "ggml-blas"] {
                        println!("cargo:rustc-link-lib=static={l}");
                    }
                    for f in ["Metal", "Foundation", "Accelerate", "CoreFoundation"] {
                        println!("cargo:rustc-link-lib=framework={f}");
                    }
                    println!("cargo:rustc-link-lib=c++");
                    println!("cargo:rustc-cfg=whisper");
                }
                _ => println!("cargo:warning=whisper shim did not compile; \
                               transcription will be unavailable"),
            }
        }
    }

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out = std::path::Path::new(&crate_dir)
        .join("include/mediacore.h");

    match cbindgen::generate(&crate_dir) {
        Ok(bindings) => { bindings.write_to_file(&out); }
        Err(e) => {
            // Do not fail the build in a spike; do make the problem visible.
            println!("cargo:warning=cbindgen failed: {e}");
        }
    }
}
