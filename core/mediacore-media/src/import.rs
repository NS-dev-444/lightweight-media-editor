//! The import boundary: a probe result becomes a project `Asset`.
//!
//! This conversion lives in the media crate, not the model crate, so that
//! `mediacore-model` stays FFmpeg-free and keeps cross-compiling for Windows
//! from any machine (R-21). The media crate knows about both sides; the model
//! crate knows nothing about FFmpeg.
//!
//! Every field populated here traces to a Phase 1 finding, not a guess.

use crate::{MCProbe, MC_OK};
use mediacore_model::asset::{Asset, AssetId, AudioInfo, ColourInfo, TransferFunction, VideoInfo};
use mediacore_model::time::{FrameRate, Ticks};

/// FFmpeg AVCOL_TRC values we care about.
const AVCOL_TRC_SMPTE2084: i32 = 16;   // PQ
const AVCOL_TRC_ARIB_STD_B67: i32 = 18; // HLG

/// Map a probed transfer characteristic to the model's colour policy (O-3).
///
/// Anything not explicitly PQ or HLG becomes Rec.709 — including
/// "unspecified", which Phase 1 measured as the COMMON real-world case even
/// for files encoded with explicit bt709 flags. Treating unspecified as
/// "unknown, refuse" would reject most ordinary video.
pub fn transfer_from_avcol(trc: i32) -> TransferFunction {
    match trc {
        AVCOL_TRC_SMPTE2084 => TransferFunction::Pq,
        AVCOL_TRC_ARIB_STD_B67 => TransferFunction::Hlg,
        _ => TransferFunction::Rec709,
    }
}

/// Bit depth, falling back when the container does not report it.
///
/// Phase 1 found `bits_per_raw_sample` comes back 0 on real HDR files, so a
/// naive read reports "0-bit video". HDR implies at least 10-bit.
pub fn bit_depth(probe: &MCProbe, transfer: TransferFunction) -> u8 {
    match probe.bits_per_raw_sample {
        b if b > 0 => b as u8,
        _ if transfer == TransferFunction::Pq || transfer == TransferFunction::Hlg => 10,
        _ => 8,
    }
}

/// Frame rate, preferring the average and falling back to a sane default.
pub fn frame_rate(probe: &MCProbe) -> FrameRate {
    if probe.avg_frame_rate_den > 0 && probe.avg_frame_rate_num > 0 {
        FrameRate::new(probe.avg_frame_rate_num as i64, probe.avg_frame_rate_den as i64)
    } else if probe.r_frame_rate_den > 0 && probe.r_frame_rate_num > 0 {
        FrameRate::new(probe.r_frame_rate_num as i64, probe.r_frame_rate_den as i64)
    } else {
        FrameRate::default()
    }
}

/// Build an `Asset` from a probe.
///
/// `duration` is passed separately because `MCProbe` reports what it inspected,
/// not the stream length.
pub fn asset_from_probe(
    id: AssetId,
    path: &str,
    duration: Ticks,
    probe: &MCProbe,
) -> Option<Asset> {
    if probe.error_class != MC_OK { return None; }

    let mut asset = Asset::new(id, path, duration);

    if probe.width > 0 && probe.height > 0 {
        let transfer = transfer_from_avcol(probe.color_trc);
        asset.video = Some(VideoInfo {
            width: probe.width as u32,
            height: probe.height as u32,
            frame_rate: frame_rate(probe),
            colour: ColourInfo { transfer, bit_depth: bit_depth(probe, transfer) },
            // Advisory only. Phase 1 showed detection is unreliable (metadata
            // misses mixed-rate files; a shallow probe misses everything), so
            // this drives user messaging and export strategy, never timing.
            is_vfr: probe.is_vfr != 0,
        });
    }
    Some(asset)
}

/// What the UI must tell the user at import time.
///
/// O-3 requires that an HDR→SDR conversion is disclosed rather than silent —
/// silent conversion is what produces "why does my video look washed out".
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportNotice {
    HdrToneMapped { transfer: &'static str },
    VariableFrameRate,
}

impl ImportNotice {
    /// User-facing wording (§23: explain, never emit codec jargon).
    pub fn message(&self) -> String {
        match self {
            ImportNotice::HdrToneMapped { transfer } => format!(
                "This video is {transfer} HDR. It will be converted to standard \
                 range for editing, so it may look different from the original."),
            ImportNotice::VariableFrameRate =>
                "This video has a variable frame rate, which is normal for screen \
                 recordings and phone footage. It will be handled correctly.".into(),
        }
    }
}

pub fn notices_for(asset: &Asset) -> Vec<ImportNotice> {
    let mut out = Vec::new();
    if let Some(v) = &asset.video {
        match v.colour.transfer {
            TransferFunction::Pq => out.push(ImportNotice::HdrToneMapped { transfer: "PQ" }),
            TransferFunction::Hlg => out.push(ImportNotice::HdrToneMapped { transfer: "HLG" }),
            TransferFunction::Rec709 => {}
        }
        if v.is_vfr { out.push(ImportNotice::VariableFrameRate); }
    }
    out
}

/// Attach audio information discovered separately (the probe is video-first).
pub fn with_audio(mut asset: Asset, sample_rate: u32, channels: u16) -> Asset {
    asset.audio = Some(AudioInfo { sample_rate, channels });
    asset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(trc: i32, bits: i32, vfr: i32) -> MCProbe {
        MCProbe {
            error_class: MC_OK, width: 3840, height: 2160, frames_decoded: 30,
            hw_accelerated: 1, av_error: 0,
            color_primaries: 9, color_trc: trc, color_space: 9,
            bits_per_raw_sample: bits, is_hdr: 0,
            avg_frame_rate_num: 30000, avg_frame_rate_den: 1001,
            r_frame_rate_num: 30000, r_frame_rate_den: 1001,
            time_base_num: 1, time_base_den: 15360,
            is_vfr: vfr, distinct_frame_durations: 1,
        }
    }

    #[test]
    fn unspecified_transfer_becomes_rec709() {
        // Phase 1: this is the common real-world case, not an error case.
        assert_eq!(transfer_from_avcol(2), TransferFunction::Rec709);
        assert_eq!(transfer_from_avcol(1), TransferFunction::Rec709);
        assert_eq!(transfer_from_avcol(AVCOL_TRC_SMPTE2084), TransferFunction::Pq);
        assert_eq!(transfer_from_avcol(AVCOL_TRC_ARIB_STD_B67), TransferFunction::Hlg);
    }

    #[test]
    fn hdr_without_reported_bit_depth_is_not_zero_bit() {
        // Phase 1 measured bits_per_raw_sample == 0 on real HDR files.
        let p = probe(AVCOL_TRC_SMPTE2084, 0, 0);
        assert_eq!(bit_depth(&p, TransferFunction::Pq), 10);
        assert_eq!(bit_depth(&probe(1, 0, 0), TransferFunction::Rec709), 8);
        assert_eq!(bit_depth(&probe(1, 12, 0), TransferFunction::Rec709), 12);
    }

    #[test]
    fn ntsc_rate_is_kept_rational() {
        let r = frame_rate(&probe(1, 8, 0));
        assert_eq!((r.num, r.den), (30000, 1001), "must not be rounded to 29.97");
        assert!(r.is_drop_frame_rate());
    }

    #[test]
    fn hdr_import_produces_a_user_notice() {
        let p = probe(AVCOL_TRC_ARIB_STD_B67, 0, 1);
        let a = asset_from_probe(AssetId(1), "/m/hdr.mov", Ticks::from_seconds(10.0), &p).unwrap();
        assert!(a.is_hdr());
        let n = notices_for(&a);
        assert!(n.contains(&ImportNotice::HdrToneMapped { transfer: "HLG" }));
        assert!(n.contains(&ImportNotice::VariableFrameRate));
        // §23: readable, no codec jargon leaking to the user.
        let msg = n[0].message();
        assert!(msg.contains("converted to standard range"));
        assert!(!msg.contains("AVCOL"));
    }

    #[test]
    fn sdr_import_is_silent() {
        let p = probe(1, 8, 0);
        let a = asset_from_probe(AssetId(2), "/m/a.mp4", Ticks::from_seconds(10.0), &p).unwrap();
        assert!(!a.is_hdr());
        assert!(notices_for(&a).is_empty(), "ordinary video must not nag the user");
    }

    #[test]
    fn a_failed_probe_produces_no_asset() {
        let mut p = probe(1, 8, 0);
        p.error_class = 7;      // MC_ERR_DECODE_FAILED
        assert!(asset_from_probe(AssetId(3), "/m/bad.mp4", Ticks::ZERO, &p).is_none());
    }
}
