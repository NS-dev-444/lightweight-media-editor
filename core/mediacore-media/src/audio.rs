//! Audio decoding and resampling.
//!
//! AD-7: ONE audio stack (FFmpeg), f32 planar internally, and a single project
//! sample rate with everything resampled on import. Mixing sample rates across
//! a timeline is a rich source of subtle bugs for no user benefit.
//!
//! Output here is INTERLEAVED f32 stereo at the project rate, which is what
//! both the mixer and the audio device want.

use rusty_ffmpeg::ffi;
use std::ffi::{c_char, CStr};
use std::ptr;

pub struct MCAudio {
    fmt: *mut ffi::AVFormatContext,
    dec: *mut ffi::AVCodecContext,
    swr: *mut ffi::SwrContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    stream_index: i32,
    /// Decoded-but-undelivered samples, interleaved stereo f32.
    pending: Vec<f32>,
    pending_read: usize,
    eof: bool,
    pub sample_rate: i32,
}

/// Open an audio stream, resampled to `sample_rate` stereo f32.
///
/// Returns null when the file has no audio — which is not an error, just a
/// video without sound.
#[no_mangle]
pub unsafe extern "C" fn mc_audio_open(path: *const c_char, sample_rate: i32) -> *mut MCAudio {
    if path.is_null() || sample_rate <= 0 { return ptr::null_mut(); }
    let cpath = CStr::from_ptr(path);

    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut fmt, cpath.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return ptr::null_mut();
    }
    if ffi::avformat_find_stream_info(fmt, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let mut codec: *const ffi::AVCodec = ptr::null();
    let si = ffi::av_find_best_stream(fmt, ffi::AVMEDIA_TYPE_AUDIO, -1, -1,
                                      &mut codec as *mut *const ffi::AVCodec, 0);
    if si < 0 || codec.is_null() {
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let dec = ffi::avcodec_alloc_context3(codec);
    let stream = *(*fmt).streams.offset(si as isize);
    ffi::avcodec_parameters_to_context(dec, (*stream).codecpar);
    if ffi::avcodec_open2(dec, codec, ptr::null_mut()) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    // Everything is conformed to stereo f32 at the project rate (AD-7).
    let mut out_layout: ffi::AVChannelLayout = std::mem::zeroed();
    ffi::av_channel_layout_default(&mut out_layout, 2);
    let mut swr: *mut ffi::SwrContext = ptr::null_mut();
    let rc = ffi::swr_alloc_set_opts2(
        &mut swr,
        &out_layout, ffi::AV_SAMPLE_FMT_FLT, sample_rate,
        &(*dec).ch_layout, (*dec).sample_fmt, (*dec).sample_rate,
        0, ptr::null_mut());
    if rc < 0 || swr.is_null() || ffi::swr_init(swr) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    Box::into_raw(Box::new(MCAudio {
        fmt, dec, swr,
        packet: ffi::av_packet_alloc(),
        frame: ffi::av_frame_alloc(),
        stream_index: si,
        pending: Vec::new(), pending_read: 0,
        eof: false,
        sample_rate,
    }))
}

/// Seek to a position in nanoseconds.
#[no_mangle]
pub unsafe extern "C" fn mc_audio_seek(a: *mut MCAudio, ns: i64) -> i32 {
    let Some(a) = a.as_mut() else { return -1 };
    let stream = *(*a.fmt).streams.offset(a.stream_index as isize);
    let tb = (*stream).time_base;
    if tb.num == 0 { return -2; }
    let ts = (ns as f64 / 1e9 * tb.den as f64 / tb.num as f64) as i64;
    if ffi::av_seek_frame(a.fmt, a.stream_index, ts, ffi::AVSEEK_FLAG_BACKWARD as i32) < 0 {
        return -3;
    }
    ffi::avcodec_flush_buffers(a.dec);
    // Buffered samples belong to the OLD position; keeping them would play a
    // fragment of the previous location after every seek.
    a.pending.clear();
    a.pending_read = 0;
    a.eof = false;
    0
}

/// Read up to `frames` stereo sample-frames into `out` (interleaved f32).
///
/// Returns the number of sample-frames written; 0 means end of stream.
#[no_mangle]
pub unsafe extern "C" fn mc_audio_read(a: *mut MCAudio, out: *mut f32, frames: i32) -> i32 {
    let (Some(a), false) = (a.as_mut(), out.is_null()) else { return 0 };
    if frames <= 0 { return 0; }
    let want = frames as usize * 2;
    let mut written = 0usize;

    while written < want {
        // Drain what is already decoded first.
        if a.pending_read < a.pending.len() {
            let n = (a.pending.len() - a.pending_read).min(want - written);
            ptr::copy_nonoverlapping(a.pending.as_ptr().add(a.pending_read),
                                     out.add(written), n);
            a.pending_read += n;
            written += n;
            continue;
        }
        a.pending.clear();
        a.pending_read = 0;
        if a.eof { break; }

        // Decode more.
        let mut got = false;
        while !got {
            let r = ffi::avcodec_receive_frame(a.dec, a.frame);
            if r == 0 {
                let in_samples = (*a.frame).nb_samples;
                let max_out = ffi::swr_get_out_samples(a.swr, in_samples).max(in_samples);
                let mut buf = vec![0f32; (max_out as usize) * 2];
                let dst = [buf.as_mut_ptr() as *mut u8, ptr::null_mut(),
                           ptr::null_mut(), ptr::null_mut()];
                let n = ffi::swr_convert(a.swr, dst.as_ptr(), max_out,
                                         (*a.frame).data.as_ptr() as *const *const u8,
                                         in_samples);
                if n > 0 {
                    buf.truncate((n as usize) * 2);
                    a.pending = buf;
                    got = true;
                }
                ffi::av_frame_unref(a.frame);
                if got { break; }
                continue;
            }
            if r != ffi::AVERROR(ffi::EAGAIN) { a.eof = true; break; }

            if ffi::av_read_frame(a.fmt, a.packet) < 0 {
                ffi::avcodec_send_packet(a.dec, ptr::null());
                a.eof = true;
                // Drain whatever the decoder still holds.
                if ffi::avcodec_receive_frame(a.dec, a.frame) == 0 {
                    ffi::av_frame_unref(a.frame);
                }
                break;
            }
            if (*a.packet).stream_index == a.stream_index {
                ffi::avcodec_send_packet(a.dec, a.packet);
            }
            ffi::av_packet_unref(a.packet);
        }
        if !got && a.eof { break; }
    }

    // Silence the tail so a short read never plays stale buffer contents.
    for i in written..want { *out.add(i) = 0.0; }
    (written / 2) as i32
}

#[no_mangle]
pub unsafe extern "C" fn mc_audio_close(a: *mut MCAudio) {
    if a.is_null() { return; }
    let mut a = Box::from_raw(a);
    ffi::swr_free(&mut a.swr);
    ffi::av_frame_free(&mut a.frame);
    ffi::av_packet_free(&mut a.packet);
    ffi::avcodec_free_context(&mut a.dec);
    ffi::avformat_close_input(&mut a.fmt);
}

/// Does this file contain audio? Cheaper than opening a decoder.
#[no_mangle]
pub unsafe extern "C" fn mc_has_audio(path: *const c_char) -> i32 {
    let a = mc_audio_open(path, 48_000);
    if a.is_null() { return 0; }
    mc_audio_close(a);
    1
}
