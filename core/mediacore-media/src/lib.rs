//! S1 — media core spike.
//!
//! Demonstrates the CONTROL PLANE / DATA PLANE split from ARCHITECTURE_DECISION.md AD-1:
//!
//!   CONTROL PLANE  small POD structs cross the FFI freely (open, info, errors)
//!   DATA PLANE     frames NEVER cross as data. A 4K frame is ~33 MB; copying one
//!                  per frame destroys both the performance and the memory budget.
//!                  Instead we hand back a RETAINED CVPixelBufferRef as an opaque
//!                  pointer — the exact IOSurface-backed buffer VideoToolbox
//!                  produced. Swift binds it to a Metal texture with no copy.

pub mod import;
pub mod session;
pub mod encoder;
pub mod image;
pub mod audio;
pub mod convert;
pub mod transcode;
pub mod merge;
pub mod analysis;
pub mod transcribe;

use rusty_ffmpeg::ffi;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;

/// Silence FFmpeg's own stderr logging.
///
/// §23 requires errors the user can understand. FFmpeg writes things like
/// "moov atom not found" straight to stderr, bypassing our error
/// classification entirely — visible in test output the first time a corrupt
/// file was probed. The host app should call this at startup and rely on
/// `mc_probe`'s error classes instead. Pass 0 to restore FFmpeg's default,
/// which is useful when diagnosing a decode problem.
#[no_mangle]
pub unsafe extern "C" fn mc_set_log_quiet(quiet: i32) {
    // AV_LOG_QUIET = -8, AV_LOG_INFO = 32.
    ffi::av_log_set_level(if quiet != 0 { -8 } else { 32 });
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRetain(cf: *const c_void) -> *const c_void;
}

// ---------------------------------------------------------------- control plane

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MCInfo {
    pub width: i32,
    pub height: i32,
    pub fps: f64,
    pub duration_sec: f64,
    /// 1 when frames arrive on the VideoToolbox hardware path (no CPU copy).
    pub hw_accelerated: i32,
    pub pix_fmt: i32,
}

pub struct MCDecoder {
    fmt: *mut ffi::AVFormatContext,
    dec: *mut ffi::AVCodecContext,
    hw_device: *mut ffi::AVBufferRef,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    stream_index: c_int,
    info: MCInfo,
    eof: bool,
    last_error: CString,
    /// Counts frames that came back on the SOFTWARE path. Must stay 0.
    pub sw_frames: i64,
}

unsafe extern "C" fn get_hw_format(
    _ctx: *mut ffi::AVCodecContext,
    mut fmts: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    while *fmts != ffi::AV_PIX_FMT_NONE {
        if *fmts == ffi::AV_PIX_FMT_VIDEOTOOLBOX {
            return ffi::AV_PIX_FMT_VIDEOTOOLBOX;
        }
        fmts = fmts.add(1);
    }
    ffi::AV_PIX_FMT_NONE
}

#[no_mangle]
pub unsafe extern "C" fn mc_open(path: *const c_char) -> *mut MCDecoder {
    if path.is_null() {
        return ptr::null_mut();
    }
    let path = CStr::from_ptr(path);

    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut fmt, path.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return ptr::null_mut();
    }
    if ffi::avformat_find_stream_info(fmt, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let mut codec: *const ffi::AVCodec = ptr::null();
    let si = ffi::av_find_best_stream(
        fmt,
        ffi::AVMEDIA_TYPE_VIDEO,
        -1,
        -1,
        &mut codec as *mut *const ffi::AVCodec,
        0,
    );
    if si < 0 || codec.is_null() {
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let dec = ffi::avcodec_alloc_context3(codec);
    let stream = *(*fmt).streams.offset(si as isize);
    ffi::avcodec_parameters_to_context(dec, (*stream).codecpar);

    // --- hardware decode via VideoToolbox -------------------------------
    let mut hw_device: *mut ffi::AVBufferRef = ptr::null_mut();
    let hw_ok = ffi::av_hwdevice_ctx_create(
        &mut hw_device,
        ffi::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
        ptr::null(),
        ptr::null_mut(),
        0,
    ) >= 0;
    if hw_ok {
        (*dec).hw_device_ctx = ffi::av_buffer_ref(hw_device);
        (*dec).get_format = Some(get_hw_format);
    }

    (*dec).thread_count = 0; // let FFmpeg choose

    if ffi::avcodec_open2(dec, codec, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let fps_r = (*stream).avg_frame_rate;
    let fps = if fps_r.den != 0 { fps_r.num as f64 / fps_r.den as f64 } else { 0.0 };
    let duration = if (*fmt).duration != ffi::AV_NOPTS_VALUE {
        (*fmt).duration as f64 / ffi::AV_TIME_BASE as f64
    } else {
        0.0
    };

    let d = Box::new(MCDecoder {
        fmt,
        dec,
        hw_device,
        packet: ffi::av_packet_alloc(),
        frame: ffi::av_frame_alloc(),
        stream_index: si,
        info: MCInfo {
            width: (*dec).width,
            height: (*dec).height,
            fps,
            duration_sec: duration,
            hw_accelerated: hw_ok as i32,
            pix_fmt: (*dec).pix_fmt,
        },
        eof: false,
        last_error: CString::default(),
        sw_frames: 0,
    });
    Box::into_raw(d)
}

#[no_mangle]
pub unsafe extern "C" fn mc_info(d: *const MCDecoder, out: *mut MCInfo) -> i32 {
    if d.is_null() || out.is_null() {
        return -1;
    }
    *out = (*d).info;
    0
}

#[no_mangle]
pub unsafe extern "C" fn mc_sw_frame_count(d: *const MCDecoder) -> i64 {
    if d.is_null() { -1 } else { (*d).sw_frames }
}

/// DATA PLANE.
///
/// Returns 1 on a frame, 0 at end of stream, negative on error.
/// `out_pb` receives a **retained** `CVPixelBufferRef`; the caller must `CFRelease` it.
/// No pixel data crosses this boundary — only the buffer handle.
#[no_mangle]
pub unsafe extern "C" fn mc_next_frame(
    d: *mut MCDecoder,
    out_pb: *mut *const c_void,
    out_pts_ns: *mut i64,
) -> i32 {
    if d.is_null() || out_pb.is_null() {
        return -1;
    }
    let d = &mut *d;
    *out_pb = ptr::null();

    loop {
        let r = ffi::avcodec_receive_frame(d.dec, d.frame);
        if r == 0 {
            let f = &*d.frame;

            // The hardware path: data[3] IS the CVPixelBufferRef VideoToolbox made.
            if f.format == ffi::AV_PIX_FMT_VIDEOTOOLBOX && !f.data[3].is_null() {
                let pb = f.data[3] as *const c_void;
                *out_pb = CFRetain(pb);
            } else {
                // Software fallback. Correct, but it means a CPU-side frame —
                // which is exactly what the zero-copy design must avoid.
                d.sw_frames += 1;
            }

            if !out_pts_ns.is_null() {
                let tb = (**(*d.fmt).streams.offset(d.stream_index as isize)).time_base;
                *out_pts_ns = if f.pts != ffi::AV_NOPTS_VALUE && tb.den != 0 {
                    (f.pts as f64 * tb.num as f64 / tb.den as f64 * 1e9) as i64
                } else {
                    0
                };
            }
            ffi::av_frame_unref(d.frame);
            return 1;
        }

        if r == ffi::AVERROR_EOF {
            return 0;
        }
        if r != ffi::AVERROR(ffi::EAGAIN) {
            return -2;
        }

        // Need more input.
        if d.eof {
            ffi::avcodec_send_packet(d.dec, ptr::null());
            // Next receive_frame will drain or report EOF.
            let r2 = ffi::avcodec_receive_frame(d.dec, d.frame);
            if r2 == ffi::AVERROR_EOF || r2 < 0 {
                return 0;
            }
            continue;
        }

        let rr = ffi::av_read_frame(d.fmt, d.packet);
        if rr < 0 {
            d.eof = true;
            ffi::avcodec_send_packet(d.dec, ptr::null());
            continue;
        }
        if (*d.packet).stream_index == d.stream_index {
            ffi::avcodec_send_packet(d.dec, d.packet);
        }
        ffi::av_packet_unref(d.packet);
    }
}

/// Seek to (at or before) a time in nanoseconds and flush the decoder.
///
/// Seeks land on the nearest preceding KEYFRAME, not the exact time — that is
/// how inter-frame codecs work. The caller decodes forward from there to reach
/// the requested frame. Returning the actual landing point lets the caller know
/// how far it must still decode.
///
/// Returns 0 on success, negative on failure.
#[no_mangle]
pub unsafe extern "C" fn mc_seek(d: *mut MCDecoder, ns: i64) -> i32 {
    let Some(d) = d.as_mut() else { return -1 };
    let stream = *(*d.fmt).streams.offset(d.stream_index as isize);
    let tb = (*stream).time_base;
    if tb.num == 0 { return -2; }
    let ts = (ns as f64 / 1e9 * tb.den as f64 / tb.num as f64) as i64;

    if ffi::av_seek_frame(d.fmt, d.stream_index, ts, ffi::AVSEEK_FLAG_BACKWARD as i32) < 0 {
        return -3;
    }
    // A decoder holds reference frames from before the seek; without a flush it
    // would emit corrupt frames from the old position.
    ffi::avcodec_flush_buffers(d.dec);
    d.eof = false;
    0
}

#[no_mangle]
pub unsafe extern "C" fn mc_close(d: *mut MCDecoder) {
    if d.is_null() {
        return;
    }
    let mut d = Box::from_raw(d);
    ffi::av_frame_free(&mut d.frame);
    ffi::av_packet_free(&mut d.packet);
    ffi::avcodec_free_context(&mut d.dec);
    if !d.hw_device.is_null() {
        ffi::av_buffer_unref(&mut d.hw_device);
    }
    ffi::avformat_close_input(&mut d.fmt);
    let _ = &d.last_error;
}

// ===========================================================================
// S6 — probing and error classification.
//
// Spec §23: errors must be understandable. "FFmpeg error 234" is unacceptable;
// "This video could not be decoded. The file may be corrupted..." is the bar.
// Classification belongs in the core so both platforms get identical wording
// and identical codes (AD-8: identical semantics, native implementation).
// ===========================================================================

pub const MC_OK: i32 = 0;
pub const MC_ERR_FILE_UNREADABLE: i32 = 1;
pub const MC_ERR_NOT_MEDIA: i32 = 2;
pub const MC_ERR_NO_VIDEO_STREAM: i32 = 3;
pub const MC_ERR_UNSUPPORTED_CODEC: i32 = 4;
pub const MC_ERR_CORRUPT_HEADER: i32 = 5;
pub const MC_ERR_TRUNCATED: i32 = 6;
pub const MC_ERR_DECODE_FAILED: i32 = 7;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MCProbe {
    pub error_class: i32,
    pub width: i32,
    pub height: i32,
    pub frames_decoded: i32,
    pub hw_accelerated: i32,
    /// The raw AVERROR, preserved for the "technical details" disclosure (§23).
    pub av_error: i32,
    // --- colour (O-3). The V1 policy is an SDR Rec.709 pipeline, so HDR input
    // must be DETECTED and tone-mapped deliberately rather than treated as 709,
    // which is what produces washed-out exports.
    pub color_primaries: i32,
    pub color_trc: i32,
    pub color_space: i32,
    pub bits_per_raw_sample: i32,
    /// 1 when the transfer function is PQ (SMPTE 2084) or HLG — i.e. HDR.
    pub is_hdr: i32,
    // --- timing. The timeline is PTS-driven on a rational timebase (§R11).
    // Variable frame rate is absent from the master spec but common in phone
    // and screen-recorded footage, and a frame-index timeline silently
    // desyncs audio on it — usually only visibly at the END of a long clip,
    // so it escapes short-clip testing and reaches users.
    pub avg_frame_rate_num: i32,
    pub avg_frame_rate_den: i32,
    pub r_frame_rate_num: i32,
    pub r_frame_rate_den: i32,
    pub time_base_num: i32,
    pub time_base_den: i32,
    /// 1 when the stream is likely variable frame rate.
    /// Set from metadata AND from observed packet timestamps — metadata alone
    /// misses mixed-rate files whose headers were not recomputed.
    pub is_vfr: i32,
    /// Distinct frame durations observed while probing (1 = perfectly regular).
    pub distinct_frame_durations: i32,
}

/// Human-readable message for an error class. Never returns a raw codec error.
#[no_mangle]
pub extern "C" fn mc_class_message(class: i32) -> *const c_char {
    let s: &'static [u8] = match class {
        MC_OK => b"The file opened successfully.\0",
        MC_ERR_FILE_UNREADABLE =>
            b"This file could not be opened. It may have been moved, renamed, or be on a disconnected drive.\0",
        MC_ERR_NOT_MEDIA =>
            b"This file does not appear to be a video or audio file. Its contents could not be recognised.\0",
        MC_ERR_NO_VIDEO_STREAM =>
            b"This file contains no video track. If you only need the audio, try importing it as audio.\0",
        MC_ERR_UNSUPPORTED_CODEC =>
            b"This file uses a video format that is not supported on this system.\0",
        MC_ERR_CORRUPT_HEADER =>
            b"This file appears to be damaged. Its header could not be read, so the format is unknown.\0",
        MC_ERR_TRUNCATED =>
            b"This file appears to be incomplete, as if a copy or download did not finish.\0",
        MC_ERR_DECODE_FAILED =>
            b"This video could not be decoded. The file may be corrupted or use a codec that is not supported on this system.\0",
        _ => b"An unexpected problem occurred while reading this file.\0",
    };
    s.as_ptr() as *const c_char
}

/// Open a file, inspect it, and attempt to decode up to `max_frames`.
/// This is the call that runs INSIDE THE WORKER PROCESS, where a codec bug
/// parsing hostile input can only take down the worker (AD-3, R-15).
#[no_mangle]
pub unsafe extern "C" fn mc_probe(
    path: *const c_char,
    max_frames: i32,
    out: *mut MCProbe,
) -> i32 {
    if out.is_null() { return -1; }
    let mut p = MCProbe::default();

    if path.is_null() {
        p.error_class = MC_ERR_FILE_UNREADABLE;
        *out = p;
        return p.error_class;
    }
    let cpath = CStr::from_ptr(path);

    // Distinguish "cannot read the file at all" from "contents unrecognised".
    if std::fs::metadata(cpath.to_string_lossy().as_ref()).is_err() {
        p.error_class = MC_ERR_FILE_UNREADABLE;
        *out = p;
        return p.error_class;
    }

    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    let r = ffi::avformat_open_input(&mut fmt, cpath.as_ptr(), ptr::null(), ptr::null_mut());
    if r < 0 {
        p.av_error = r;
        p.error_class = if r == ffi::AVERROR_INVALIDDATA { MC_ERR_NOT_MEDIA }
                        else { MC_ERR_CORRUPT_HEADER };
        *out = p;
        return p.error_class;
    }

    let si_r = ffi::avformat_find_stream_info(fmt, ptr::null_mut());
    if si_r < 0 {
        p.av_error = si_r;
        p.error_class = MC_ERR_CORRUPT_HEADER;
        ffi::avformat_close_input(&mut fmt);
        *out = p;
        return p.error_class;
    }

    let mut codec: *const ffi::AVCodec = ptr::null();
    let si = ffi::av_find_best_stream(fmt, ffi::AVMEDIA_TYPE_VIDEO, -1, -1,
                                      &mut codec as *mut *const ffi::AVCodec, 0);
    if si < 0 {
        p.av_error = si;
        p.error_class = MC_ERR_NO_VIDEO_STREAM;
        ffi::avformat_close_input(&mut fmt);
        *out = p;
        return p.error_class;
    }
    if codec.is_null() {
        p.error_class = MC_ERR_UNSUPPORTED_CODEC;
        ffi::avformat_close_input(&mut fmt);
        *out = p;
        return p.error_class;
    }

    let dec = ffi::avcodec_alloc_context3(codec);
    let stream = *(*fmt).streams.offset(si as isize);
    ffi::avcodec_parameters_to_context(dec, (*stream).codecpar);
    p.width = (*dec).width;
    p.height = (*dec).height;
    p.color_primaries = (*dec).color_primaries as i32;
    p.color_trc = (*dec).color_trc as i32;
    p.color_space = (*dec).colorspace as i32;
    p.bits_per_raw_sample = (*dec).bits_per_raw_sample;
    // AVCOL_TRC_SMPTE2084 = PQ, AVCOL_TRC_ARIB_STD_B67 = HLG.
    p.is_hdr = ((*dec).color_trc == ffi::AVCOL_TRC_SMPTE2084
             || (*dec).color_trc == ffi::AVCOL_TRC_ARIB_STD_B67) as i32;

    // Timing. r_frame_rate is FFmpeg's guess at the lowest frame rate that can
    // express every timestamp exactly; avg_frame_rate is the mean. For CFR they
    // agree. A meaningful divergence means the stream carries irregular frame
    // durations -- i.e. VFR -- and the timeline must be PTS-driven for it.
    let afr = (*stream).avg_frame_rate;
    let rfr = (*stream).r_frame_rate;
    let tb  = (*stream).time_base;
    p.avg_frame_rate_num = afr.num; p.avg_frame_rate_den = afr.den;
    p.r_frame_rate_num   = rfr.num; p.r_frame_rate_den   = rfr.den;
    p.time_base_num      = tb.num;  p.time_base_den      = tb.den;
    if afr.den > 0 && rfr.den > 0 {
        let a = afr.num as f64 / afr.den as f64;
        let r = rfr.num as f64 / rfr.den as f64;
        // 1% tolerance: 30000/1001 vs 30 is CFR, not VFR.
        p.is_vfr = ((a - r).abs() / a.max(1e-9) > 0.01) as i32;
    }

    let mut hw_device: *mut ffi::AVBufferRef = ptr::null_mut();
    if ffi::av_hwdevice_ctx_create(&mut hw_device, ffi::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                                   ptr::null(), ptr::null_mut(), 0) >= 0 {
        (*dec).hw_device_ctx = ffi::av_buffer_ref(hw_device);
        (*dec).get_format = Some(get_hw_format);
        p.hw_accelerated = 1;
    }

    let open_r = ffi::avcodec_open2(dec, codec, ptr::null_mut());
    if open_r < 0 {
        p.av_error = open_r;
        p.error_class = MC_ERR_UNSUPPORTED_CODEC;
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut fmt);
        *out = p;
        return p.error_class;
    }

    let packet = ffi::av_packet_alloc();
    let frame = ffi::av_frame_alloc();
    let mut decoded = 0i32;
    let mut saw_packet = false;
    // Observed frame durations, for VFR detection that does not trust metadata.
    let mut last_pts: i64 = ffi::AV_NOPTS_VALUE;
    let mut durations: Vec<i64> = Vec::new();

    while decoded < max_frames {
        let rr = ffi::av_read_frame(fmt, packet);
        if rr < 0 { break; }
        if (*packet).stream_index == si {
            saw_packet = true;
            if ffi::avcodec_send_packet(dec, packet) >= 0 {
                while ffi::avcodec_receive_frame(dec, frame) == 0 {
                    let pts = (*frame).pts;
                    if pts != ffi::AV_NOPTS_VALUE && last_pts != ffi::AV_NOPTS_VALUE {
                        let d = pts - last_pts;
                        if d > 0 && !durations.iter().any(|&x| (x - d).abs() <= 1) {
                            durations.push(d);
                        }
                    }
                    if pts != ffi::AV_NOPTS_VALUE { last_pts = pts; }
                    decoded += 1;
                    ffi::av_frame_unref(frame);
                    if decoded >= max_frames { break; }
                }
            }
        }
        ffi::av_packet_unref(packet);
    }

    p.frames_decoded = decoded;
    p.distinct_frame_durations = durations.len() as i32;
    // Two or more distinct frame durations means irregular timing regardless of
    // what the container header claims. (A tolerance of +-1 tick absorbs
    // rounding on timebases like 1/15360.)
    if durations.len() > 1 { p.is_vfr = 1; }
    if decoded == 0 {
        p.error_class = if saw_packet { MC_ERR_DECODE_FAILED } else { MC_ERR_TRUNCATED };
    } else {
        p.error_class = MC_OK;
    }

    let mut f = frame; ffi::av_frame_free(&mut f);
    let mut pk = packet; ffi::av_packet_free(&mut pk);
    let mut d = dec; ffi::avcodec_free_context(&mut d);
    if !hw_device.is_null() { ffi::av_buffer_unref(&mut hw_device); }
    ffi::avformat_close_input(&mut fmt);
    *out = p;
    p.error_class
}

// ===========================================================================
// S7 — waveform generation.
//
// Spec §17: waveforms are generated asynchronously and must never block the UI;
// the cache is reused between launches. The core streams peaks through a
// callback rather than returning one big buffer, so the caller can draw
// progressively and cancel — a 60-minute file should show a partial waveform
// immediately, not nothing for ten seconds.
// ===========================================================================

/// Called with a batch of (min,max) pairs. Return 0 to cancel.
pub type MCPeakCallback = unsafe extern "C" fn(
    user: *mut c_void,
    peaks: *const f32,   // interleaved min,max
    pair_count: i32,
    progress: f32,
) -> i32;

#[no_mangle]
pub unsafe extern "C" fn mc_waveform(
    path: *const c_char,
    buckets_per_second: i32,
    cb: MCPeakCallback,
    user: *mut c_void,
    out_duration: *mut f64,
) -> i32 {
    if path.is_null() { return MC_ERR_FILE_UNREADABLE; }
    let cpath = CStr::from_ptr(path);

    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut fmt, cpath.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return MC_ERR_NOT_MEDIA;
    }
    if ffi::avformat_find_stream_info(fmt, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut fmt);
        return MC_ERR_CORRUPT_HEADER;
    }

    let mut codec: *const ffi::AVCodec = ptr::null();
    let si = ffi::av_find_best_stream(fmt, ffi::AVMEDIA_TYPE_AUDIO, -1, -1,
                                      &mut codec as *mut *const ffi::AVCodec, 0);
    if si < 0 || codec.is_null() {
        ffi::avformat_close_input(&mut fmt);
        return MC_ERR_NO_VIDEO_STREAM;   // reused: "no usable stream of that type"
    }

    let dec = ffi::avcodec_alloc_context3(codec);
    let stream = *(*fmt).streams.offset(si as isize);
    ffi::avcodec_parameters_to_context(dec, (*stream).codecpar);
    if ffi::avcodec_open2(dec, codec, ptr::null_mut()) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut fmt);
        return MC_ERR_UNSUPPORTED_CODEC;
    }

    let sample_rate = (*dec).sample_rate.max(1);
    let total_secs = if (*fmt).duration != ffi::AV_NOPTS_VALUE {
        (*fmt).duration as f64 / ffi::AV_TIME_BASE as f64
    } else { 0.0 };
    if !out_duration.is_null() { *out_duration = total_secs; }

    let bps = buckets_per_second.max(1);
    let samples_per_bucket = (sample_rate / bps).max(1) as usize;

    let packet = ffi::av_packet_alloc();
    let frame = ffi::av_frame_alloc();

    let mut batch: Vec<f32> = Vec::with_capacity(2048);
    let mut cur_min = f32::MAX;
    let mut cur_max = f32::MIN;
    let mut in_bucket = 0usize;
    let mut samples_seen: u64 = 0;
    let total_samples = (total_secs * sample_rate as f64) as u64;
    let mut cancelled = false;

    'outer: while ffi::av_read_frame(fmt, packet) >= 0 {
        if (*packet).stream_index == si && ffi::avcodec_send_packet(dec, packet) >= 0 {
            while ffi::avcodec_receive_frame(dec, frame) == 0 {
                let f = &*frame;
                let n = f.nb_samples as usize;
                let fmt_id = f.format;
                let planar = ffi::av_sample_fmt_is_planar(fmt_id) == 1;
                let ch = f.ch_layout.nb_channels.max(1) as usize;

                for i in 0..n {
                    // Peak across channels; mono-sum is wrong for a peak display.
                    let mut v: f32 = 0.0;
                    for c in 0..ch {
                        let s = read_sample(f, fmt_id, planar, c, i);
                        if s.abs() > v.abs() { v = s; }
                    }
                    if v < cur_min { cur_min = v; }
                    if v > cur_max { cur_max = v; }
                    in_bucket += 1;
                    if in_bucket >= samples_per_bucket {
                        batch.push(cur_min);
                        batch.push(cur_max);
                        cur_min = f32::MAX; cur_max = f32::MIN; in_bucket = 0;
                        if batch.len() >= 2048 {
                            let prog = if total_samples > 0 {
                                (samples_seen as f64 / total_samples as f64) as f32
                            } else { 0.0 };
                            if cb(user, batch.as_ptr(), (batch.len()/2) as i32, prog) == 0 {
                                cancelled = true;
                            }
                            batch.clear();
                            if cancelled { ffi::av_frame_unref(frame); break 'outer; }
                        }
                    }
                }
                samples_seen += n as u64;
                ffi::av_frame_unref(frame);
            }
        }
        ffi::av_packet_unref(packet);
    }

    if !batch.is_empty() && !cancelled {
        cb(user, batch.as_ptr(), (batch.len()/2) as i32, 1.0);
    }

    let mut f = frame; ffi::av_frame_free(&mut f);
    let mut pk = packet; ffi::av_packet_free(&mut pk);
    let mut d = dec; ffi::avcodec_free_context(&mut d);
    ffi::avformat_close_input(&mut fmt);
    if cancelled { -1 } else { MC_OK }
}

#[inline]
unsafe fn read_sample(f: &ffi::AVFrame, fmt_id: i32, planar: bool, ch: usize, i: usize) -> f32 {
    let (buf, idx) = if planar { (f.data[ch], i) } else { (f.data[0], i * f.ch_layout.nb_channels.max(1) as usize + ch) };
    if buf.is_null() { return 0.0; }
    match fmt_id {
        x if x == ffi::AV_SAMPLE_FMT_FLTP || x == ffi::AV_SAMPLE_FMT_FLT =>
            *(buf as *const f32).add(idx),
        x if x == ffi::AV_SAMPLE_FMT_S16P || x == ffi::AV_SAMPLE_FMT_S16 =>
            *(buf as *const i16).add(idx) as f32 / 32768.0,
        x if x == ffi::AV_SAMPLE_FMT_S32P || x == ffi::AV_SAMPLE_FMT_S32 =>
            *(buf as *const i32).add(idx) as f32 / 2147483648.0,
        x if x == ffi::AV_SAMPLE_FMT_DBLP || x == ffi::AV_SAMPLE_FMT_DBL =>
            *(buf as *const f64).add(idx) as f32,
        _ => 0.0,
    }
}
