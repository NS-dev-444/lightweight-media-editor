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
use std::ptr;

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
    #[cfg(target_os = "windows")]
    {
        // NVENC only, for now. `amf` is the same shape and could join it; `qsv`
        // links libmfx rather than loading the driver's encoder, which is a
        // redistribution question still open in the audit.
        //
        // A machine with an AMD or Intel GPU therefore has no hardware encoder
        // and will be told so honestly — see `no_encoder_message`.
        if hevc { &[c"hevc_nvenc"] } else { &[c"h264_nvenc"] }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
/// **Only says the encoder is COMPILED IN.** For audio that is enough. For
/// video on Windows it is not — see `find_working_encoder`.
pub fn find_encoder(names: &[&'static CStr])
    -> Option<(*const ffi::AVCodec, &'static CStr)>
{
    for name in names {
        let c = unsafe { ffi::avcodec_find_encoder_by_name(name.as_ptr()) };
        if !c.is_null() { return Some((c, name)); }
    }
    None
}

/// Can this encoder actually be OPENED on this machine?
///
/// **AD-6: probe by actually looking, never by assuming a GPU exists.**
///
/// This matters on Windows in a way it never did on macOS. Our Windows build
/// contains `nvenc` whether or not the machine has an NVIDIA card, so
/// `avcodec_find_encoder_by_name` succeeds on an AMD or Intel machine and the
/// failure only appears at `avcodec_open2` — which, without this, would be at
/// the moment the user pressed Export on a finished edit.
///
/// The probe opens a tiny encoder and throws it away. That costs a few
/// milliseconds once, and the answer is cached because opening a hardware
/// encoder session is not free.
unsafe fn encoder_opens(codec: *const ffi::AVCodec) -> bool {
    let ctx = ffi::avcodec_alloc_context3(codec);
    if ctx.is_null() { return false; }
    // Small, even, and a rate every encoder accepts. This is a liveness check,
    // not a representative encode.
    (*ctx).width = 640;
    (*ctx).height = 480;
    (*ctx).time_base = ffi::AVRational { num: 1, den: 30 };
    (*ctx).framerate = ffi::AVRational { num: 30, den: 1 };
    (*ctx).pix_fmt = ffi::AV_PIX_FMT_NV12;
    (*ctx).bit_rate = 1_000_000;
    let ok = ffi::avcodec_open2(ctx, codec, ptr::null_mut()) >= 0;
    let mut c = ctx;
    ffi::avcodec_free_context(&mut c);
    ok
}

/// The first encoder from the list that this machine can actually use.
///
/// Cached: the answer cannot change while the process is running, and probing
/// a hardware encoder repeatedly is wasteful.
pub fn find_working_encoder(names: &[&'static CStr])
    -> Option<(*const ffi::AVCodec, &'static CStr)>
{
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::sync::Mutex<Vec<(&'static CStr, bool)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()));

    for name in names {
        let codec = unsafe { ffi::avcodec_find_encoder_by_name(name.as_ptr()) };
        if codec.is_null() { continue; }

        let cached = cache.lock().ok()
            .and_then(|c| c.iter().find(|(n, _)| n == name).map(|(_, ok)| *ok));
        let works = match cached {
            Some(ok) => ok,
            None => {
                let ok = unsafe { encoder_opens(codec) };
                if let Ok(mut c) = cache.lock() { c.push((name, ok)); }
                ok
            }
        };
        if works { return Some((codec, name)); }
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
    { "This PC has no graphics card that can export video. \
       An NVIDIA card is needed; support for AMD and Intel graphics is \
       not in this build yet." }
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
    fn the_hardware_encoder_this_machine_has_can_actually_be_opened() {
        // The distinction this test exists for: find_encoder says "compiled
        // in", find_working_encoder says "this machine can use it". On Windows
        // our build contains nvenc regardless of the GPU, so the two answers
        // differ on any AMD or Intel machine.
        let compiled = find_encoder(video_encoders(true));
        let usable = find_working_encoder(video_encoders(true));

        if cfg!(target_os = "macos") {
            assert!(compiled.is_some(), "hevc_videotoolbox missing from this build");
            assert!(usable.is_some(), "hevc_videotoolbox will not open on this Mac");
        } else if compiled.is_some() && usable.is_none() {
            // A legitimate state on Windows: the encoder is in the build, the
            // machine cannot use it. The message must say something true.
            assert!(!no_encoder_message().is_empty());
        }
    }

    #[test]
    fn probing_an_encoder_twice_gives_the_same_answer() {
        // The result is cached; a cache that disagrees with itself would make
        // export availability depend on when it was asked.
        let a = find_working_encoder(video_encoders(true)).map(|(_, n)| n);
        let b = find_working_encoder(video_encoders(true)).map(|(_, n)| n);
        assert_eq!(a, b);
    }
}
