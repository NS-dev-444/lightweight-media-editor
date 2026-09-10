//! End-to-end: probe a real file with FFmpeg, produce a model `Asset`.
//!
//! The unit tests in `import` use synthetic `MCProbe` values. This exercises
//! the whole path against files on disk, which is where the mismatches between
//! "what FFmpeg reports" and "what the model expects" actually surface.
//!
//! Skips cleanly when the media fixtures are absent, so a fresh checkout does
//! not fail (the fixtures are generated, not committed).

use mediacore::import::*;
use mediacore::{mc_probe, MCProbe, MC_OK};
use mediacore_model::asset::{AssetId, TransferFunction};
use mediacore_model::time::Ticks;
use std::ffi::CString;

fn repo_media(rel: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../media").join(rel);
    p.exists().then_some(p)
}

fn probe_file(path: &std::path::Path) -> MCProbe {
    let c = CString::new(path.to_str().unwrap()).unwrap();
    let mut p = MCProbe::default();
    unsafe { mc_probe(c.as_ptr(), 150, &mut p) };
    p
}

#[test]
fn sdr_file_imports_without_notices() {
    let Some(path) = repo_media("vfr/cfr_30.mp4") else { eprintln!("fixture absent, skipped"); return };
    let p = probe_file(&path);
    assert_eq!(p.error_class, MC_OK, "probe failed on a valid file");

    let a = asset_from_probe(AssetId(1), path.to_str().unwrap(),
                             Ticks::from_seconds(4.0), &p).unwrap();
    let v = a.video.as_ref().expect("video info");
    assert_eq!((v.width, v.height), (640, 480));
    assert_eq!(v.colour.transfer, TransferFunction::Rec709);
    assert_eq!(v.colour.bit_depth, 8);
    assert!(!v.is_vfr);
    assert!(notices_for(&a).is_empty(), "ordinary SDR video must import silently");
}

#[test]
fn hdr_files_are_detected_and_disclosed() {
    // O-3: HDR must be detected and the conversion disclosed, never silent.
    for (rel, expect) in [("hdr/pq_2020.mp4", TransferFunction::Pq),
                          ("hdr/hlg_2020.mp4", TransferFunction::Hlg)] {
        let Some(path) = repo_media(rel) else { eprintln!("fixture absent, skipped"); return };
        let p = probe_file(&path);
        assert_eq!(p.error_class, MC_OK, "probe failed on {rel}");

        let a = asset_from_probe(AssetId(2), path.to_str().unwrap(),
                                 Ticks::from_seconds(2.0), &p).unwrap();
        let v = a.video.as_ref().expect("video info");
        assert_eq!(v.colour.transfer, expect, "{rel} transfer misdetected");
        assert!(a.is_hdr());
        // Phase 1: real HDR files report bits_per_raw_sample == 0. The import
        // must not conclude "0-bit video".
        assert!(v.colour.bit_depth >= 10, "{rel} reported {}-bit", v.colour.bit_depth);

        let n = notices_for(&a);
        assert!(n.iter().any(|x| matches!(x, ImportNotice::HdrToneMapped { .. })),
                "{rel} imported without telling the user it will be tone-mapped");
    }
}

#[test]
fn variable_frame_rate_is_flagged() {
    for rel in ["vfr/vfr_dropped.mp4", "vfr/vfr_mixed.mkv"] {
        let Some(path) = repo_media(rel) else { eprintln!("fixture absent, skipped"); return };
        let p = probe_file(&path);
        assert_eq!(p.error_class, MC_OK, "probe failed on {rel}");
        let a = asset_from_probe(AssetId(3), path.to_str().unwrap(),
                                 Ticks::from_seconds(4.0), &p).unwrap();
        assert!(a.video.as_ref().unwrap().is_vfr,
                "{rel} not flagged VFR (needs a deep enough probe)");
        assert!(notices_for(&a).contains(&ImportNotice::VariableFrameRate));
    }
}

#[test]
fn frame_rate_stays_rational() {
    let Some(path) = repo_media("hdr/sdr_709.mp4") else { eprintln!("fixture absent, skipped"); return };
    let p = probe_file(&path);
    let a = asset_from_probe(AssetId(4), path.to_str().unwrap(),
                             Ticks::from_seconds(2.0), &p).unwrap();
    let r = a.video.as_ref().unwrap().frame_rate;
    assert!(r.den > 0 && r.num > 0, "frame rate must never be zero-denominator");
    // 30/1 exactly, never a float approximation.
    assert_eq!((r.num, r.den), (30, 1));
}

#[test]
fn a_corrupt_file_yields_no_asset_and_a_readable_class() {
    let Some(dir) = repo_media("../build/s6-corpus") else { eprintln!("corpus absent, skipped"); return };
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap().flatten().take(60) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("garbage_") && !name.starts_with("trunc_0") { continue; }
        let p = probe_file(&entry.path());
        if p.error_class != MC_OK {
            assert!(asset_from_probe(AssetId(9), "x", Ticks::ZERO, &p).is_none(),
                    "a failed probe must not produce an asset");
            checked += 1;
        }
    }
    assert!(checked > 0, "expected at least one rejected file in the corpus");
}
