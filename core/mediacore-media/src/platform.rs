//! What hardware and codecs THIS platform actually has.
//!
//! Every platform assumption in the crate is collected here. It was scattered
//! across six files as bare literals — `c"hevc_videotoolbox"`, a `CFRetain`
//! declaration, `AV_HWDEVICE_TYPE_VIDEOTOOLBOX` — with no `cfg` anywhere, which
//! is fine while one platform exists and becomes six separate ports the moment
//! a second one does.
//!
//! Two things are deliberate.
//!
//! **Encoders are a preference LIST, not a name.** The old code hard-coded
//! `hevc_videotoolbox`; on any machine without it that is a null pointer and a
//! dead path with nothing to say. A list lets the first available one win and
//! lets the failure message name what was looked for.
//!
//! **The constants compile everywhere.** `AV_HWDEVICE_TYPE_VIDEOTOOLBOX` is an
//! FFmpeg enum value, present in the headers on every platform — it is only
//! *useless* off macOS, not absent. So the `cfg` here is about which value to
//! prefer, not about whether the code builds.

use rusty_ffmpeg::ffi;
use std::ffi::CStr;

/// The hardware decode path to ask FFmpeg for.
///
/// Decode is the half that works on both platforms today: VideoToolbox on
/// macOS, D3D11VA on Windows, both LGPL-clean and both royalty-free to *use*
/// (the patent question in DEPENDENCY_AND_LICENSE_AUDIT.md §6 is about
/// distributing an encoder, not about calling the OS's decoder).
#[cfg(target_os = "macos")]
pub const HW_DEVICE_TYPE: ffi::AVHWDeviceType = ffi::AV_HWDEVICE_TYPE_VIDEOTOOLBOX;
#[cfg(target_os = "windows")]
pub const HW_DEVICE_TYPE: ffi::AVHWDeviceType = ffi::AV_HWDEVICE_TYPE_D3D11VA;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const HW_DEVICE_TYPE: ffi::AVHWDeviceType = ffi::AV_HWDEVICE_TYPE_NONE;

/// The pixel format hardware frames arrive in.
#[cfg(target_os = "macos")]
pub const HW_PIX_FMT: ffi::AVPixelFormat = ffi::AV_PIX_FMT_VIDEOTOOLBOX;
#[cfg(target_os = "windows")]
pub const HW_PIX_FMT: ffi::AVPixelFormat = ffi::AV_PIX_FMT_D3D11;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const HW_PIX_FMT: ffi::AVPixelFormat = ffi::AV_PIX_FMT_NONE;

/// Hardware video encoders to try, best first.
///
/// **AD-12: hardware only.** No software H.264/HEVC encoder is ever listed —
/// that is what bounds the patent exposure in the licence audit, and it is a
/// policy rather than a performance choice.
///
/// **Windows is empty for now, but not for the reason first assumed.** The
/// audit's §3.3a verified `nvenc` and `amf` against our pinned FFmpeg: neither
/// is in the GPL or nonfree list, both take MIT headers at build time, and both
/// load the actual encoder from the **user's own graphics driver** at runtime.
/// That is structurally identical to VideoToolbox, which macOS already ships.
///
/// So this is a **build task, not a licensing blocker**: the Windows FFmpeg
/// build needs `nv-codec-headers` before these names resolve to anything. They
/// go in the list when that lands, and `find_encoder` will pick whichever the
/// machine actually has.
///
/// `qsv` stays out regardless for now — it *links* `libmfx` rather than loading
/// the driver's, which is a genuine redistribution question and the one thing
/// here that is materially different.
pub fn video_encoders(hevc: bool) -> &'static [&'static CStr] {
    #[cfg(target_os = "macos")]
    {
        if hevc { &[c"hevc_videotoolbox"] } else { &[c"h264_videotoolbox"] }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = hevc;
        &[]
    }
}

/// AAC encoders to try, best first.
///
/// macOS prefers AudioToolbox's, which is generally better than FFmpeg's own.
/// Both platforms fall back to the native encoder, which is LGPL-clean.
pub fn aac_encoders() -> &'static [&'static CStr] {
    #[cfg(target_os = "macos")]
    { &[c"aac_at", c"aac"] }
    #[cfg(not(target_os = "macos"))]
    { &[c"aac"] }
}

/// The first available encoder from a preference list.
///
/// Returns the codec and the name that matched, so a caller can say which one
/// it got — "encoded with aac" versus "encoded with aac_at" is the kind of
/// thing worth being able to answer without guessing.
pub fn find_encoder(names: &[&'static CStr])
    -> Option<(*const ffi::AVCodec, &'static CStr)>
{
    for name in names {
        let c = unsafe { ffi::avcodec_find_encoder_by_name(name.as_ptr()) };
        if !c.is_null() { return Some((c, name)); }
    }
    None
}

/// Why no encoder was found, as a sentence (§23).
///
/// The honest message differs by platform, and saying "no encoder" on Windows
/// when the real answer is "we have not cleared the licence for the ones this
/// machine has" would be misleading.
pub fn no_encoder_message() -> &'static str {
    #[cfg(target_os = "macos")]
    { "This Mac cannot encode that format." }
    #[cfg(target_os = "windows")]
    { "Video export is not available on Windows in this build yet. \
       Support for this machine's graphics hardware is still being added." }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "No supported hardware video encoder was found on this system." }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn CFRetain(cf: *const std::ffi::c_void) -> *const std::ffi::c_void;
}

/// Take a reference on a platform frame handle so it outlives the AVFrame.
///
/// macOS hands out a `CVPixelBuffer` that must be `CFRetain`ed before the frame
/// is unreferenced; Windows hands out an `ID3D11Texture2D` whose lifetime is
/// managed by the decoder's frames context, so there is nothing to do.
///
/// `CFRetain` above is the only platform symbol this crate calls directly, and
/// the only thing in it that would fail to LINK on Windows rather than merely
/// fail to work at runtime.
#[cfg(target_os = "macos")]
pub unsafe fn retain_frame_handle(handle: *const std::ffi::c_void) -> *const std::ffi::c_void {
    CFRetain(handle)
}

#[cfg(not(target_os = "macos"))]
pub unsafe fn retain_frame_handle(handle: *const std::ffi::c_void) -> *const std::ffi::c_void {
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_platform_has_a_hardware_decode_path() {
        // A build for a platform with no hardware decoding is a build that
        // would silently fall back to software everywhere — worth failing on.
        assert_ne!(HW_DEVICE_TYPE, ffi::AV_HWDEVICE_TYPE_NONE,
                   "no hardware decode path is configured for this platform");
        assert_ne!(HW_PIX_FMT, ffi::AV_PIX_FMT_NONE);
    }

    #[test]
    fn an_aac_encoder_is_always_available() {
        // FFmpeg's native AAC is built in on every platform, so this list can
        // never be empty. If it is, the build lost a codec it was meant to have.
        assert!(find_encoder(aac_encoders()).is_some(),
                "no AAC encoder — check the FFmpeg build's configure flags");
    }

    #[test]
    fn macos_has_hardware_video_encoders_and_windows_deliberately_does_not() {
        let found = find_encoder(video_encoders(true));
        if cfg!(target_os = "macos") {
            assert!(found.is_some(), "hevc_videotoolbox missing from this build");
        } else {
            // Not an oversight: §3.3's VERIFY items gate nvenc/qsv/amf.
            assert!(found.is_none());
            assert!(!no_encoder_message().is_empty());
        }
    }
}
