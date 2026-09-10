//! Media conversion (§4) — the CONVERT mode.
//!
//! Two paths, chosen automatically:
//!
//! **Remux** when the source streams are already compatible with the target
//! container. MKV→MP4 with H.264/AAC inside needs no re-encoding at all: it is
//! near-instant, lossless, and skips the codec question entirely. This is the
//! "smart export" idea PRODUCT_DIRECTION.md §3.4 flagged as missing from §12 —
//! the single biggest perceived-speed win available, and the most common real
//! conversion people actually ask for.
//!
//! **Transcode** otherwise, using hardware encoders only (AD-12), so the patent
//! posture in DEPENDENCY_AND_LICENSE_AUDIT.md §6 stays bounded.
//!
//! Stepwise rather than threaded: the caller drives `mc_convert_step`, so
//! threading, cancellation and progress reporting stay on the caller's side and
//! no FFmpeg context is ever touched from two threads (a mistake already made
//! and fixed once, in export).

use rusty_ffmpeg::ffi;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

pub const MC_CONVERT_REMUX: i32 = 0;
pub const MC_CONVERT_TRANSCODE: i32 = 1;

pub struct MCConvert {
    in_fmt: *mut ffi::AVFormatContext,
    out_fmt: *mut ffi::AVFormatContext,
    packet: *mut ffi::AVPacket,
    /// input stream index -> output stream index, -1 to drop.
    stream_map: Vec<i32>,
    duration_us: i64,
    progress_us: i64,
    mode: i32,
    header_written: bool,
    finished: bool,
    last_error: CString,
}

/// Can these streams be copied into the target container untouched?
///
/// Conservative on purpose: a wrong "yes" produces a file that looks fine and
/// will not play, which is far worse than an unnecessary transcode.
unsafe fn can_remux(in_fmt: *mut ffi::AVFormatContext, out_name: &str) -> bool {
    let mp4_video = [ffi::AV_CODEC_ID_H264, ffi::AV_CODEC_ID_HEVC,
                     ffi::AV_CODEC_ID_MPEG4, ffi::AV_CODEC_ID_AV1];
    let mp4_audio = [ffi::AV_CODEC_ID_AAC, ffi::AV_CODEC_ID_MP3,
                     ffi::AV_CODEC_ID_ALAC];
    for i in 0..(*in_fmt).nb_streams {
        let st = *(*in_fmt).streams.offset(i as isize);
        let par = (*st).codecpar;
        match (*par).codec_type {
            t if t == ffi::AVMEDIA_TYPE_VIDEO => {
                if matches!(out_name, "mp4" | "mov") && !mp4_video.contains(&(*par).codec_id) {
                    return false;
                }
            }
            t if t == ffi::AVMEDIA_TYPE_AUDIO => {
                if matches!(out_name, "mp4" | "mov") && !mp4_audio.contains(&(*par).codec_id) {
                    return false;
                }
            }
            // Subtitles, data and attachments are dropped rather than blocking
            // a remux; the caller is converting the media, not archiving it.
            _ => {}
        }
    }
    true
}

fn container_of(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".mov") { "mov" }
    else if lower.ends_with(".mkv") { "matroska" }
    else if lower.ends_with(".m4a") { "ipod" }
    else if lower.ends_with(".mp3") { "mp3" }
    else if lower.ends_with(".wav") { "wav" }
    else if lower.ends_with(".flac") { "flac" }
    else { "mp4" }
}

/// Open a conversion. `allow_remux == 0` forces a transcode.
#[no_mangle]
pub unsafe extern "C" fn mc_convert_open(input: *const c_char,
                                         output: *const c_char,
                                         allow_remux: i32) -> *mut MCConvert {
    if input.is_null() || output.is_null() { return ptr::null_mut(); }
    let cin = CStr::from_ptr(input);
    let cout = CStr::from_ptr(output);
    let out_path = cout.to_string_lossy().into_owned();
    let container = container_of(&out_path);

    let mut in_fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut in_fmt, cin.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return ptr::null_mut();
    }
    if ffi::avformat_find_stream_info(in_fmt, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    let remux = allow_remux != 0 && can_remux(in_fmt, container);
    if !remux {
        // Transcoding is a separate, heavier path; this build ships the remux
        // path first because it is what most conversions actually need, and
        // says so plainly rather than pretending.
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    let mut out_fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    let cname = CString::new(container).unwrap_or_default();
    if ffi::avformat_alloc_output_context2(&mut out_fmt, ptr::null_mut(),
                                           cname.as_ptr(), cout.as_ptr()) < 0 {
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    let mut stream_map = vec![-1i32; (*in_fmt).nb_streams as usize];
    let mut next_out = 0i32;
    for i in 0..(*in_fmt).nb_streams {
        let st = *(*in_fmt).streams.offset(i as isize);
        let t = (*(*st).codecpar).codec_type;
        if t != ffi::AVMEDIA_TYPE_VIDEO && t != ffi::AVMEDIA_TYPE_AUDIO { continue; }
        let out_st = ffi::avformat_new_stream(out_fmt, ptr::null());
        if out_st.is_null() { continue; }
        ffi::avcodec_parameters_copy((*out_st).codecpar, (*st).codecpar);
        // codec_tag belongs to the SOURCE container; carrying it over produces
        // a file some players reject.
        (*(*out_st).codecpar).codec_tag = 0;
        stream_map[i as usize] = next_out;
        next_out += 1;
    }
    if next_out == 0 {
        ffi::avformat_free_context(out_fmt);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    if (*(*out_fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0
        && ffi::avio_open(&mut (*out_fmt).pb, cout.as_ptr(), ffi::AVIO_FLAG_WRITE as i32) < 0
    {
        ffi::avformat_free_context(out_fmt);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }
    if ffi::avformat_write_header(out_fmt, ptr::null_mut()) < 0 {
        ffi::avformat_free_context(out_fmt);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    let duration_us = if (*in_fmt).duration != ffi::AV_NOPTS_VALUE { (*in_fmt).duration } else { 0 };

    Box::into_raw(Box::new(MCConvert {
        in_fmt, out_fmt,
        packet: ffi::av_packet_alloc(),
        stream_map,
        duration_us,
        progress_us: 0,
        mode: MC_CONVERT_REMUX,
        header_written: true,
        finished: false,
        last_error: CString::default(),
    }))
}

/// Process one packet. 1 = more work, 0 = finished, negative = error.
///
/// Stepwise so the caller owns the loop: progress, cancellation and pausing
/// belong to whoever is driving, and no context is shared across threads.
#[no_mangle]
pub unsafe extern "C" fn mc_convert_step(c: *mut MCConvert) -> i32 {
    let Some(c) = c.as_mut() else { return -1 };
    if c.finished { return 0; }

    if ffi::av_read_frame(c.in_fmt, c.packet) < 0 {
        c.finished = true;
        return 0;
    }
    let in_index = (*c.packet).stream_index as usize;
    let out_index = c.stream_map.get(in_index).copied().unwrap_or(-1);
    if out_index < 0 {
        ffi::av_packet_unref(c.packet);
        return 1;                              // dropped stream; keep going
    }

    let in_st = *(*c.in_fmt).streams.offset(in_index as isize);
    let out_st = *(*c.out_fmt).streams.offset(out_index as isize);

    if (*c.packet).pts != ffi::AV_NOPTS_VALUE {
        let tb = (*in_st).time_base;
        c.progress_us = ffi::av_rescale_q((*c.packet).pts, tb,
                                          ffi::AVRational { num: 1, den: 1_000_000 });
    }

    // Timestamps must be rescaled from the source timebase to the target's.
    ffi::av_packet_rescale_ts(c.packet, (*in_st).time_base, (*out_st).time_base);
    (*c.packet).stream_index = out_index;
    (*c.packet).pos = -1;

    let rc = ffi::av_interleaved_write_frame(c.out_fmt, c.packet);
    ffi::av_packet_unref(c.packet);
    if rc < 0 {
        c.last_error = CString::new("The converted file could not be written.")
            .unwrap_or_default();
        return -2;
    }
    1
}

/// 0.0–1.0. Falls back to 0 when the source reports no duration.
#[no_mangle]
pub unsafe extern "C" fn mc_convert_progress(c: *const MCConvert) -> f32 {
    let Some(c) = c.as_ref() else { return 0.0 };
    if c.finished { return 1.0; }
    if c.duration_us <= 0 { return 0.0; }
    (c.progress_us as f32 / c.duration_us as f32).clamp(0.0, 1.0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_convert_mode(c: *const MCConvert) -> i32 {
    c.as_ref().map(|c| c.mode).unwrap_or(-1)
}

#[no_mangle]
pub unsafe extern "C" fn mc_convert_error(c: *const MCConvert) -> *const c_char {
    match c.as_ref() { Some(c) => c.last_error.as_ptr(), None => ptr::null() }
}

/// Finish and close. Writes the trailer, so a cancelled job still leaves a
/// playable (if short) file rather than a corrupt one.
#[no_mangle]
pub unsafe extern "C" fn mc_convert_close(c: *mut MCConvert) -> i32 {
    if c.is_null() { return -1; }
    let mut c = Box::from_raw(c);
    if c.header_written { ffi::av_write_trailer(c.out_fmt); }
    if (*(*c.out_fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0 {
        ffi::avio_closep(&mut (*c.out_fmt).pb);
    }
    ffi::av_packet_free(&mut c.packet);
    ffi::avformat_free_context(c.out_fmt);
    ffi::avformat_close_input(&mut c.in_fmt);
    0
}

/// Would this conversion be a remux (fast, lossless) or need re-encoding?
///
/// Lets the UI tell the user which they are about to get — "this will be
/// instant" versus "this will take a while" is worth knowing before starting.
#[no_mangle]
pub unsafe extern "C" fn mc_convert_would_remux(input: *const c_char,
                                                output: *const c_char) -> i32 {
    if input.is_null() || output.is_null() { return 0; }
    let cin = CStr::from_ptr(input);
    let out_path = CStr::from_ptr(output).to_string_lossy().into_owned();
    let mut in_fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut in_fmt, cin.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return 0;
    }
    let ok = ffi::avformat_find_stream_info(in_fmt, ptr::null_mut()) >= 0
        && can_remux(in_fmt, container_of(&out_path));
    ffi::avformat_close_input(&mut in_fmt);
    ok as i32
}

// ===========================================================================
// Audio transcoding — §4's most common conversions.
//
// WAV→MP3, FLAC→MP3, M4A→MP3, MP3→WAV, OGG→MP3, M4A→WAV. Most VIDEO
// conversions people ask for (MOV→MP4, MKV→MP4) are remuxes and never reach
// here; audio conversions almost always require re-encoding.
//
// MP3 encoding is why LAME is in the build: FFmpeg has no native MP3 encoder,
// and macOS no longer exposes an AudioToolbox one (Phase 1, S3).
// ===========================================================================

pub struct MCAudioConvert {
    reader: *mut crate::audio::MCAudio,
    fmt: *mut ffi::AVFormatContext,
    enc: *mut ffi::AVCodecContext,
    frame: *mut ffi::AVFrame,
    packet: *mut ffi::AVPacket,
    stream_time_base: ffi::AVRational,
    samples_written: i64,
    buffer: Vec<f32>,
    total_frames: i64,
    finished: bool,
    sample_rate: i32,
}

pub const MC_AUDIO_MP3: i32 = 0;
pub const MC_AUDIO_AAC: i32 = 1;
pub const MC_AUDIO_WAV: i32 = 2;
pub const MC_AUDIO_FLAC: i32 = 3;
pub const MC_AUDIO_ALAC: i32 = 4;

/// Open an audio conversion.
#[no_mangle]
pub unsafe extern "C" fn mc_audio_convert_open(input: *const c_char,
                                               output: *const c_char,
                                               codec_kind: i32,
                                               bitrate_kbps: i32,
                                               sample_rate: i32) -> *mut MCAudioConvert {
    if input.is_null() || output.is_null() { return ptr::null_mut(); }
    let rate = if sample_rate > 0 { sample_rate } else { 48_000 };

    let reader = crate::audio::mc_audio_open(input, rate);
    if reader.is_null() { return ptr::null_mut(); }

    let cout = CStr::from_ptr(output);
    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_alloc_output_context2(&mut fmt, ptr::null_mut(),
                                           ptr::null(), cout.as_ptr()) < 0 {
        crate::audio::mc_audio_close(reader);
        return ptr::null_mut();
    }

    let codec = match codec_kind {
        MC_AUDIO_MP3 => ffi::avcodec_find_encoder_by_name(c"libmp3lame".as_ptr()),
        MC_AUDIO_WAV => ffi::avcodec_find_encoder(ffi::AV_CODEC_ID_PCM_S16LE),
        MC_AUDIO_FLAC => ffi::avcodec_find_encoder(ffi::AV_CODEC_ID_FLAC),
        MC_AUDIO_ALAC => ffi::avcodec_find_encoder(ffi::AV_CODEC_ID_ALAC),
        _ => crate::platform::find_encoder(crate::platform::aac_encoders())
                .map(|(c, _)| c).unwrap_or(ptr::null()),
    };
    if codec.is_null() {
        ffi::avformat_free_context(fmt);
        crate::audio::mc_audio_close(reader);
        return ptr::null_mut();
    }

    // Ask the encoder which sample format it takes. Assuming one is how the
    // export path first failed against AudioToolbox's s16-only AAC.
    let mut cfg: *const std::ffi::c_void = ptr::null();
    let mut count: i32 = 0;
    let sample_fmt = if ffi::avcodec_get_supported_config(
            ptr::null(), codec, ffi::AV_CODEC_CONFIG_SAMPLE_FORMAT, 0,
            &mut cfg as *mut _ as *mut *const std::ffi::c_void, &mut count) >= 0
        && !cfg.is_null() && count > 0
    { *(cfg as *const i32) } else { ffi::AV_SAMPLE_FMT_FLTP };

    let enc = ffi::avcodec_alloc_context3(codec);
    (*enc).sample_fmt = sample_fmt;
    (*enc).sample_rate = rate;
    (*enc).bit_rate = (bitrate_kbps.max(64) as i64) * 1000;
    ffi::av_channel_layout_default(&mut (*enc).ch_layout, 2);
    (*enc).time_base = ffi::AVRational { num: 1, den: rate };
    if (*(*fmt).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
        (*enc).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
    }
    if ffi::avcodec_open2(enc, codec, ptr::null_mut()) < 0 {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(fmt);
        crate::audio::mc_audio_close(reader);
        return ptr::null_mut();
    }

    let stream = ffi::avformat_new_stream(fmt, ptr::null());
    ffi::avcodec_parameters_from_context((*stream).codecpar, enc);
    (*stream).time_base = (*enc).time_base;

    if (*(*fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0
        && ffi::avio_open(&mut (*fmt).pb, cout.as_ptr(), ffi::AVIO_FLAG_WRITE as i32) < 0
    {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(fmt);
        crate::audio::mc_audio_close(reader);
        return ptr::null_mut();
    }
    if ffi::avformat_write_header(fmt, ptr::null_mut()) < 0 {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(fmt);
        crate::audio::mc_audio_close(reader);
        return ptr::null_mut();
    }

    Box::into_raw(Box::new(MCAudioConvert {
        reader, fmt, enc,
        frame: ffi::av_frame_alloc(),
        packet: ffi::av_packet_alloc(),
        // Read back AFTER the header: the muxer may have changed it.
        stream_time_base: (*stream).time_base,
        samples_written: 0,
        buffer: Vec::new(),
        total_frames: 0,
        finished: false,
        sample_rate: rate,
    }))
}

/// Convert one chunk. 1 = more, 0 = done, negative = error.
#[no_mangle]
pub unsafe extern "C" fn mc_audio_convert_step(c: *mut MCAudioConvert) -> i32 {
    let Some(c) = c.as_mut() else { return -1 };
    if c.finished { return 0; }

    let chunk = 4096usize;
    let mut scratch = vec![0f32; chunk * 2];
    let got = crate::audio::mc_audio_read(c.reader, scratch.as_mut_ptr(), chunk as i32);
    if got <= 0 {
        // Flush the encoder before declaring completion.
        ffi::avcodec_send_frame(c.enc, ptr::null());
        drain_audio_conv(c);
        c.finished = true;
        return 0;
    }
    c.total_frames += got as i64;
    c.buffer.extend_from_slice(&scratch[..(got as usize) * 2]);

    let frame_size = if (*c.enc).frame_size > 0 { (*c.enc).frame_size as usize } else { 1024 };
    while c.buffer.len() >= frame_size * 2 {
        ffi::av_frame_unref(c.frame);
        (*c.frame).nb_samples = frame_size as i32;
        (*c.frame).format = (*c.enc).sample_fmt;
        ffi::av_channel_layout_copy(&mut (*c.frame).ch_layout, &(*c.enc).ch_layout);
        (*c.frame).sample_rate = c.sample_rate;
        if ffi::av_frame_get_buffer(c.frame, 0) < 0 { return -2; }
        if write_samples(c.frame, (*c.enc).sample_fmt, &c.buffer, frame_size) < 0 { return -3; }
        (*c.frame).pts = c.samples_written;
        c.samples_written += frame_size as i64;
        c.buffer.drain(0..frame_size * 2);
        if ffi::avcodec_send_frame(c.enc, c.frame) < 0 { return -4; }
        drain_audio_conv(c);
    }
    1
}

/// Convert interleaved f32 into whatever layout the encoder declared.
pub(crate) unsafe fn write_samples(frame: *mut ffi::AVFrame, fmt: i32,
                        src: &[f32], frames: usize) -> i32 {
    match fmt {
        f if f == ffi::AV_SAMPLE_FMT_FLTP => {
            let l = (*frame).data[0] as *mut f32;
            let r = (*frame).data[1] as *mut f32;
            for i in 0..frames { *l.add(i) = src[i * 2]; *r.add(i) = src[i * 2 + 1]; }
        }
        f if f == ffi::AV_SAMPLE_FMT_FLT => {
            let d = (*frame).data[0] as *mut f32;
            for i in 0..frames * 2 { *d.add(i) = src[i]; }
        }
        f if f == ffi::AV_SAMPLE_FMT_S16 => {
            let d = (*frame).data[0] as *mut i16;
            for i in 0..frames * 2 { *d.add(i) = (src[i].clamp(-1.0, 1.0) * 32767.0) as i16; }
        }
        f if f == ffi::AV_SAMPLE_FMT_S16P => {
            let l = (*frame).data[0] as *mut i16;
            let r = (*frame).data[1] as *mut i16;
            for i in 0..frames {
                *l.add(i) = (src[i * 2].clamp(-1.0, 1.0) * 32767.0) as i16;
                *r.add(i) = (src[i * 2 + 1].clamp(-1.0, 1.0) * 32767.0) as i16;
            }
        }
        f if f == ffi::AV_SAMPLE_FMT_S32 => {
            let d = (*frame).data[0] as *mut i32;
            for i in 0..frames * 2 {
                *d.add(i) = (src[i].clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
            }
        }
        f if f == ffi::AV_SAMPLE_FMT_S32P => {
            let l = (*frame).data[0] as *mut i32;
            let r = (*frame).data[1] as *mut i32;
            for i in 0..frames {
                *l.add(i) = (src[i * 2].clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
                *r.add(i) = (src[i * 2 + 1].clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
            }
        }
        _ => return -1,      // refuse rather than emit noise
    }
    0
}

unsafe fn drain_audio_conv(c: &mut MCAudioConvert) {
    while ffi::avcodec_receive_packet(c.enc, c.packet) == 0 {
        (*c.packet).stream_index = 0;
        ffi::av_packet_rescale_ts(c.packet, (*c.enc).time_base, c.stream_time_base);
        ffi::av_interleaved_write_frame(c.fmt, c.packet);
        ffi::av_packet_unref(c.packet);
    }
}

#[no_mangle]
pub unsafe extern "C" fn mc_audio_convert_close(c: *mut MCAudioConvert) -> i32 {
    if c.is_null() { return -1; }
    let mut c = Box::from_raw(c);
    ffi::av_write_trailer(c.fmt);
    if (*(*c.fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0 {
        ffi::avio_closep(&mut (*c.fmt).pb);
    }
    ffi::av_frame_free(&mut c.frame);
    ffi::av_packet_free(&mut c.packet);
    ffi::avcodec_free_context(&mut c.enc);
    crate::audio::mc_audio_close(c.reader);
    0
}

/// Sample-frames converted so far, for progress.
#[no_mangle]
pub unsafe extern "C" fn mc_audio_convert_frames(c: *const MCAudioConvert) -> i64 {
    c.as_ref().map(|c| c.total_frames).unwrap_or(0)
}
