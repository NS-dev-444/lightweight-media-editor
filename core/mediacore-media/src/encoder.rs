//! Video encoding for export.
//!
//! AD-12: **hardware encoders only**. No software H.264/HEVC encoder ships, so
//! the patent exposure documented in DEPENDENCY_AND_LICENSE_AUDIT.md §6 stays
//! bounded, and the LGPL build never needs libx264/libx265.
//!
//! HEVC is the default wherever the target permits it — S4b measured hardware
//! HEVC needing ~1.74x libx264's bitrate versus ~2.31x for hardware H.264, so
//! choosing HEVC is the single most effective quality lever available.
//!
//! Frames arrive as `CVPixelBuffer`s the compositor already rendered, so the
//! export path is zero-copy on the way IN as well as on the way out.

use crate::MCInfo;
use rusty_ffmpeg::ffi;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;

pub struct MCEncoder {
    fmt: *mut ffi::AVFormatContext,
    enc: *mut ffi::AVCodecContext,
    stream_index: i32,
    frame: *mut ffi::AVFrame,
    packet: *mut ffi::AVPacket,
    frames_written: i64,
    /// The ENCODER's timebase (1/fps).
    time_base: ffi::AVRational,
    /// The STREAM's timebase, read back AFTER `avformat_write_header`.
    ///
    /// Muxers rewrite it — MP4 typically imposes 1/90000 — so rescaling
    /// timestamps to the value set beforehand produces a file whose duration is
    /// wrong by whatever ratio the muxer chose. A 1-second clip reported
    /// 0.0019 s before this was fixed.
    stream_time_base: ffi::AVRational,

    // --- audio (§4: an export without sound is not an export) -------------
    aenc: *mut ffi::AVCodecContext,
    aframe: *mut ffi::AVFrame,
    apacket: *mut ffi::AVPacket,
    audio_stream_index: i32,
    audio_time_base: ffi::AVRational,
    audio_samples_written: i64,
    /// Interleaved stereo f32 awaiting a full encoder frame.
    audio_buffer: Vec<f32>,
}

/// Codec selection. Values match `MCExportCodec` in the header.
pub const MC_CODEC_HEVC: i32 = 0;
pub const MC_CODEC_H264: i32 = 1;

/// Open an encoder writing to `path`.
///
/// Returns null on failure; the reason is available from `mc_encoder_error`.
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_open(path: *const c_char,
                                         width: i32, height: i32,
                                         fps_num: i32, fps_den: i32,
                                         codec_kind: i32, bitrate_kbps: i32,
                                         audio_sample_rate: i32)
    -> *mut MCEncoder
{
    if path.is_null() || width <= 0 || height <= 0 || fps_num <= 0 || fps_den <= 0 {
        return ptr::null_mut();
    }
    // H.264 and HEVC require even dimensions; hardware encoders also dislike
    // odd sizes. §5 allows custom dimensions, so this must be handled rather
    // than left to fail deep inside the encoder.
    let width = width & !1;
    let height = height & !1;

    let cpath = CStr::from_ptr(path);
    let Some((codec, _)) = crate::platform::find_encoder(
        crate::platform::video_encoders(codec_kind != MC_CODEC_H264))
    else { return ptr::null_mut() };

    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_alloc_output_context2(&mut fmt, ptr::null_mut(),
                                           ptr::null(), cpath.as_ptr()) < 0 {
        return ptr::null_mut();
    }
    let stream = ffi::avformat_new_stream(fmt, ptr::null());
    if stream.is_null() { ffi::avformat_free_context(fmt); return ptr::null_mut(); }

    let enc = ffi::avcodec_alloc_context3(codec);
    (*enc).width = width;
    (*enc).height = height;

    // Accepting CVPixelBuffers directly requires a VideoToolbox HW FRAMES
    // CONTEXT. Without it the encoder expects CPU-side NV12 and a hardware
    // frame is rejected — the export path would have to read every frame back
    // from the GPU, which is exactly the copy S1 established we must avoid.
    let mut hw_device: *mut ffi::AVBufferRef = ptr::null_mut();
    let hw_ok = ffi::av_hwdevice_ctx_create(&mut hw_device,
                    crate::platform::HW_DEVICE_TYPE,
                    ptr::null(), ptr::null_mut(), 0) >= 0;
    if hw_ok {
        let frames_ref = ffi::av_hwframe_ctx_alloc(hw_device);
        if !frames_ref.is_null() {
            let frames_ctx = (*frames_ref).data as *mut ffi::AVHWFramesContext;
            (*frames_ctx).format = crate::platform::HW_PIX_FMT;
            (*frames_ctx).sw_format = ffi::AV_PIX_FMT_BGRA;
            (*frames_ctx).width = width;
            (*frames_ctx).height = height;
            (*frames_ctx).initial_pool_size = 8;
            if ffi::av_hwframe_ctx_init(frames_ref) >= 0 {
                (*enc).pix_fmt = crate::platform::HW_PIX_FMT;
                (*enc).hw_frames_ctx = ffi::av_buffer_ref(frames_ref);
            }
            let mut fr = frames_ref;
            ffi::av_buffer_unref(&mut fr);
        }
        let mut hd = hw_device;
        ffi::av_buffer_unref(&mut hd);
    }
    // Fall back to CPU frames if the hardware context could not be built —
    // AD-6: never assume the hardware path exists.
    if (*enc).hw_frames_ctx.is_null() {
        (*enc).pix_fmt = ffi::AV_PIX_FMT_NV12;
    }
    (*enc).time_base = ffi::AVRational { num: fps_den, den: fps_num };
    (*enc).framerate = ffi::AVRational { num: fps_num, den: fps_den };
    (*enc).bit_rate = (bitrate_kbps.max(1) as i64) * 1000;
    // O-3: V1 is an SDR Rec.709 pipeline, so the output is tagged 709. Tagging
    // matters — an untagged file is guessed at by every player differently.
    (*enc).color_primaries = ffi::AVCOL_PRI_BT709;
    (*enc).color_trc = ffi::AVCOL_TRC_BT709;
    (*enc).colorspace = ffi::AVCOL_SPC_BT709;
    if (*(*fmt).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
        (*enc).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
    }

    if ffi::avcodec_open2(enc, codec, ptr::null_mut()) < 0 {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(fmt);
        return ptr::null_mut();
    }
    ffi::avcodec_parameters_from_context((*stream).codecpar, enc);
    (*stream).time_base = (*enc).time_base;

    // The audio stream must exist BEFORE the header is written.
    //
    // Prefer AAC via AudioToolbox where it exists (see platform.rs): Phase 1
    // found it present in our LGPL
    // build, and Apple's encoder is generally better than FFmpeg's native AAC.
    // Falls back to the native one, which is equally LGPL-clean.
    let mut aenc: *mut ffi::AVCodecContext = ptr::null_mut();
    let mut audio_stream_index = -1;
    let mut audio_sample_fmt = ffi::AV_SAMPLE_FMT_FLTP;
    if audio_sample_rate > 0 {
        let acodec = crate::platform::find_encoder(crate::platform::aac_encoders())
            .map(|(c, _)| c).unwrap_or(ptr::null());
        if !acodec.is_null() {
            // Ask the encoder which sample formats it accepts rather than
            // assuming. AudioToolbox's AAC takes s16 ONLY, so hard-coding FLTP
            // made `avcodec_open2` fail — and FFmpeg 8 replaced the old
            // `codec->sample_fmts` array with this query (noted in Phase 1).
            let mut cfg: *const std::ffi::c_void = ptr::null();
            let mut count: i32 = 0;
            let chosen = if ffi::avcodec_get_supported_config(
                    ptr::null(), acodec, ffi::AV_CODEC_CONFIG_SAMPLE_FORMAT, 0,
                    &mut cfg as *mut _ as *mut *const std::ffi::c_void, &mut count) >= 0
                && !cfg.is_null() && count > 0
            {
                *(cfg as *const i32)
            } else {
                ffi::AV_SAMPLE_FMT_FLTP
            };
            audio_sample_fmt = chosen;

            // Open the CODEC FIRST, and only create the stream if it succeeds.
            //
            // Creating the stream up front left a "codec none" stream behind
            // when the codec failed to open, and the muxer then refused to
            // write the file at all — a failed audio codec took the whole
            // export down with it.
            let ac = ffi::avcodec_alloc_context3(acodec);
            (*ac).sample_fmt = chosen;
            (*ac).sample_rate = audio_sample_rate;
            (*ac).bit_rate = 192_000;
            ffi::av_channel_layout_default(&mut (*ac).ch_layout, 2);
            (*ac).time_base = ffi::AVRational { num: 1, den: audio_sample_rate };
            if (*(*fmt).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
                (*ac).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
            if ffi::avcodec_open2(ac, acodec, ptr::null_mut()) >= 0 {
                let astream = ffi::avformat_new_stream(fmt, ptr::null());
                if !astream.is_null() {
                    ffi::avcodec_parameters_from_context((*astream).codecpar, ac);
                    (*astream).time_base = (*ac).time_base;
                    aenc = ac;
                    audio_stream_index = (*astream).index;
                } else {
                    let mut a = ac; ffi::avcodec_free_context(&mut a);
                }
            } else {
                // No audio rather than a broken file. Video still exports.
                let mut a = ac; ffi::avcodec_free_context(&mut a);
            }
        }
    }

    if (*(*fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0
        && ffi::avio_open(&mut (*fmt).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE as i32) < 0
    {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(fmt);
        return ptr::null_mut();
    }
    if ffi::avformat_write_header(fmt, ptr::null_mut()) < 0 {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(fmt);
        return ptr::null_mut();
    }

    let result = Box::into_raw(Box::new(MCEncoder {
        fmt, enc, stream_index: (*stream).index,
        frame: ffi::av_frame_alloc(),
        packet: ffi::av_packet_alloc(),
        frames_written: 0,
        time_base: (*enc).time_base,
        // Read back AFTER the header is written; the muxer may have changed it.
        stream_time_base: (*stream).time_base,
        aenc,
        aframe: if aenc.is_null() { ptr::null_mut() } else { ffi::av_frame_alloc() },
        apacket: if aenc.is_null() { ptr::null_mut() } else { ffi::av_packet_alloc() },
        audio_stream_index,
        audio_time_base: if aenc.is_null() {
            ffi::AVRational { num: 1, den: 48_000 }
        } else {
            (**(*fmt).streams.offset(audio_stream_index as isize)).time_base
        },
        audio_samples_written: 0,
        audio_buffer: Vec::new(),
    }));
    let _ = audio_sample_fmt;
    result
}

/// True when the encoder is taking hardware surfaces (zero-copy).
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_is_hardware(e: *const MCEncoder) -> i32 {
    e.as_ref().map(|e| (!(*e.enc).hw_frames_ctx.is_null()) as i32).unwrap_or(0)
}

/// Borrow a writable surface from the ENCODER'S pool.
///
/// The compositor renders into this and then calls `mc_encoder_submit`.
///
/// This is the API shape the hardware path requires, and it was arrived at by
/// measurement rather than assumption: pushing a `CVPixelBuffer` from a
/// caller-owned pool is rejected outright (`avcodec_send_frame` returns an
/// error), because FFmpeg's hardware frames context can only encode frames it
/// allocated. Rendering into ITS buffer keeps the path genuinely zero-copy.
///
/// The returned buffer is owned by the encoder — do not release it.
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_acquire(e: *mut MCEncoder,
                                            out_pixel_buffer: *mut *mut c_void) -> i32 {
    let Some(e) = e.as_mut() else { return -1 };
    if out_pixel_buffer.is_null() { return -1; }
    *out_pixel_buffer = ptr::null_mut();

    ffi::av_frame_unref(e.frame);
    if (*e.enc).hw_frames_ctx.is_null() { return -2; }

    (*e.frame).format = crate::platform::HW_PIX_FMT;
    (*e.frame).width = (*e.enc).width;
    (*e.frame).height = (*e.enc).height;
    if ffi::av_hwframe_get_buffer((*e.enc).hw_frames_ctx, e.frame, 0) < 0 { return -3; }
    if (*e.frame).data[3].is_null() { return -4; }

    *out_pixel_buffer = (*e.frame).data[3] as *mut c_void;
    0
}

/// Encode the surface most recently returned by `mc_encoder_acquire`.
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_submit(e: *mut MCEncoder) -> i32 {
    let Some(e) = e.as_mut() else { return -1 };
    if (*e.frame).data[3].is_null() { return -2; }
    (*e.frame).pts = e.frames_written;
    if ffi::avcodec_send_frame(e.enc, e.frame) < 0 { return -3; }
    drain(e);
    e.frames_written += 1;
    0
}

unsafe fn drain(e: &mut MCEncoder) {
    while ffi::avcodec_receive_packet(e.enc, e.packet) == 0 {
        (*e.packet).stream_index = e.stream_index;
        ffi::av_packet_rescale_ts(e.packet, e.time_base, e.stream_time_base);
        ffi::av_interleaved_write_frame(e.fmt, e.packet);
        ffi::av_packet_unref(e.packet);
    }
}

/// Push interleaved stereo f32 audio.
///
/// The AAC encoder needs fixed-size frames, so samples are buffered until a
/// full frame is available. Feeding partial frames produces gaps and drift.
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_push_audio(e: *mut MCEncoder,
                                               samples: *const f32,
                                               frame_count: i32) -> i32 {
    let Some(e) = e.as_mut() else { return -1 };
    if e.aenc.is_null() { return 0; }              // no audio stream: ignore
    if samples.is_null() || frame_count <= 0 { return 0; }

    e.audio_buffer.extend_from_slice(
        std::slice::from_raw_parts(samples, frame_count as usize * 2));

    let frame_size = if (*e.aenc).frame_size > 0 { (*e.aenc).frame_size as usize } else { 1024 };
    while e.audio_buffer.len() >= frame_size * 2 {
        ffi::av_frame_unref(e.aframe);
        (*e.aframe).nb_samples = frame_size as i32;
        (*e.aframe).format = (*e.aenc).sample_fmt;
        ffi::av_channel_layout_copy(&mut (*e.aframe).ch_layout, &(*e.aenc).ch_layout);
        (*e.aframe).sample_rate = (*e.aenc).sample_rate;
        if ffi::av_frame_get_buffer(e.aframe, 0) < 0 { return -2; }

        // Convert from interleaved f32 into whatever layout the codec wants.
        // AudioToolbox's AAC takes interleaved s16; FFmpeg's native AAC takes
        // planar f32. Writing one layout and declaring the other produces
        // noise, so this follows `sample_fmt`.
        match (*e.aenc).sample_fmt {
            f if f == ffi::AV_SAMPLE_FMT_FLTP => {
                let left = (*e.aframe).data[0] as *mut f32;
                let right = (*e.aframe).data[1] as *mut f32;
                for i in 0..frame_size {
                    *left.add(i) = e.audio_buffer[i * 2];
                    *right.add(i) = e.audio_buffer[i * 2 + 1];
                }
            }
            f if f == ffi::AV_SAMPLE_FMT_S16 => {
                let dst = (*e.aframe).data[0] as *mut i16;
                for i in 0..frame_size * 2 {
                    let v = e.audio_buffer[i].clamp(-1.0, 1.0);
                    *dst.add(i) = (v * 32767.0) as i16;
                }
            }
            f if f == ffi::AV_SAMPLE_FMT_S16P => {
                let left = (*e.aframe).data[0] as *mut i16;
                let right = (*e.aframe).data[1] as *mut i16;
                for i in 0..frame_size {
                    *left.add(i) = (e.audio_buffer[i * 2].clamp(-1.0, 1.0) * 32767.0) as i16;
                    *right.add(i) = (e.audio_buffer[i * 2 + 1].clamp(-1.0, 1.0) * 32767.0) as i16;
                }
            }
            f if f == ffi::AV_SAMPLE_FMT_FLT => {
                let dst = (*e.aframe).data[0] as *mut f32;
                for i in 0..frame_size * 2 { *dst.add(i) = e.audio_buffer[i]; }
            }
            _ => { return -4; }   // unsupported layout: refuse rather than emit noise
        }
        (*e.aframe).pts = e.audio_samples_written;
        e.audio_samples_written += frame_size as i64;
        e.audio_buffer.drain(0..frame_size * 2);

        if ffi::avcodec_send_frame(e.aenc, e.aframe) < 0 { return -3; }
        drain_audio(e);
    }
    0
}

unsafe fn drain_audio(e: &mut MCEncoder) {
    if e.aenc.is_null() { return; }
    while ffi::avcodec_receive_packet(e.aenc, e.apacket) == 0 {
        (*e.apacket).stream_index = e.audio_stream_index;
        ffi::av_packet_rescale_ts(e.apacket, (*e.aenc).time_base, e.audio_time_base);
        ffi::av_interleaved_write_frame(e.fmt, e.apacket);
        ffi::av_packet_unref(e.apacket);
    }
}

/// Does this encoder have an audio stream?
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_has_audio(e: *const MCEncoder) -> i32 {
    e.as_ref().map(|e| (!e.aenc.is_null()) as i32).unwrap_or(0)
}

/// Flush, write the trailer, and close. The encoder is consumed.
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_finish(e: *mut MCEncoder) -> i32 {
    if e.is_null() { return -1; }
    let mut e = Box::from_raw(e);
    // A trailer that is never written leaves an unplayable file, so the drain
    // must happen even if frames failed.
    ffi::avcodec_send_frame(e.enc, ptr::null());
    drain(&mut e);
    if !e.aenc.is_null() {
        ffi::avcodec_send_frame(e.aenc, ptr::null());
        drain_audio(&mut e);
    }
    ffi::av_write_trailer(e.fmt);
    if (*(*e.fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0 {
        ffi::avio_closep(&mut (*e.fmt).pb);
    }
    ffi::av_frame_free(&mut e.frame);
    ffi::av_packet_free(&mut e.packet);
    ffi::avcodec_free_context(&mut e.enc);
    if !e.aframe.is_null() { ffi::av_frame_free(&mut e.aframe); }
    if !e.apacket.is_null() { ffi::av_packet_free(&mut e.apacket); }
    if !e.aenc.is_null() { ffi::avcodec_free_context(&mut e.aenc); }
    ffi::avformat_free_context(e.fmt);
    0
}

/// Frames written so far, for progress reporting.
#[no_mangle]
pub unsafe extern "C" fn mc_encoder_frames(e: *const MCEncoder) -> i64 {
    e.as_ref().map(|e| e.frames_written).unwrap_or(0)
}

/// Is a codec available on this machine? AD-6: probe by ACTUALLY looking, never
/// by assuming a GPU or a codec exists.
#[no_mangle]
pub extern "C" fn mc_encoder_available(codec_kind: i32) -> i32 {
    crate::platform::find_encoder(
        crate::platform::video_encoders(codec_kind != MC_CODEC_H264)
    ).is_some() as i32
}

/// Unused today, but keeps `MCInfo` referenced so the header stays stable.
#[allow(dead_code)]
fn _keep(_: MCInfo, _: CString) {}
