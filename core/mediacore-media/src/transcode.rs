//! Video transcoding — the conversions a remux cannot do (§4, §31).
//!
//! Most video conversions people ask for are remuxes and never come here:
//! `convert.rs` handles those, losslessly and near-instantly. This path exists
//! for the rest — a source whose codec the target container will not carry, and
//! the **resize** and **compress** tools, which by definition re-encode.
//!
//! Three decisions shape it.
//!
//! **Hardware encoders only** (AD-12), so the patent posture in
//! DEPENDENCY_AND_LICENSE_AUDIT.md §6 stays bounded and no software H.264/HEVC
//! encoder ever ships. HEVC is preferred where the container allows it: S4b
//! measured hardware HEVC needing ~1.74x libx264's bitrate against ~2.31x for
//! hardware H.264, which makes the codec choice the largest quality lever here.
//!
//! **Audio is copied when it can be.** Re-encoding audio that is already AAC or
//! MP3 costs quality for nothing. It is only decoded and re-encoded when the
//! target container will not carry the source codec.
//!
//! **CPU frames between decode and encode.** The zero-copy hardware path S1
//! established matters for preview and export, where a frame's journey is
//! GPU-to-GPU. Here the frame usually has to be *scaled*, and a scale means a
//! trip through swscale regardless. Taking the simple path keeps this one
//! readable and correct; conversion is a background batch job, not a 60 fps
//! preview, and it is I/O and encoder bound in practice.
//!
//! Stepwise, like every other long operation here: the caller drives the loop,
//! so progress, pause and cancel stay on the caller's side and no FFmpeg
//! context is touched from two threads.

use rusty_ffmpeg::ffi;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;

pub struct MCVideoConvert {
    in_fmt: *mut ffi::AVFormatContext,
    out_fmt: *mut ffi::AVFormatContext,

    v_in: i32,
    v_out: i32,
    dec: *mut ffi::AVCodecContext,
    enc: *mut ffi::AVCodecContext,
    sws: *mut ffi::SwsContext,
    /// NV12 frame the encoder is fed, when scaling or format conversion is
    /// needed. Null when the decoder already produces exactly what it wants.
    scaled: *mut ffi::AVFrame,

    a_in: i32,
    a_out: i32,
    /// Audio is passed through untouched when the container accepts it.
    audio_copy: bool,
    adec: *mut ffi::AVCodecContext,
    aenc: *mut ffi::AVCodecContext,
    swr: *mut ffi::SwrContext,
    aframe: *mut ffi::AVFrame,
    /// Interleaved f32 awaiting a full encoder frame.
    abuf: Vec<f32>,
    asamples: i64,

    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    out_pkt: *mut ffi::AVPacket,

    v_stream_tb: ffi::AVRational,
    a_stream_tb: ffi::AVRational,
    enc_tb: ffi::AVRational,
    aenc_tb: ffi::AVRational,

    duration_us: i64,
    progress_us: i64,
    frames_encoded: i64,
    draining: bool,
    finished: bool,
    last_error: CString,
}

fn err(c: &mut MCVideoConvert, msg: &str) -> i32 {
    c.last_error = CString::new(msg).unwrap_or_default();
    -1
}

/// Does this container carry this audio codec as-is?
fn audio_copyable(container: &str, codec_id: ffi::AVCodecID) -> bool {
    match container {
        "mp4" | "mov" | "ipod" => matches!(codec_id,
            ffi::AV_CODEC_ID_AAC | ffi::AV_CODEC_ID_MP3 | ffi::AV_CODEC_ID_ALAC),
        "matroska" | "webm" => matches!(codec_id,
            ffi::AV_CODEC_ID_AAC | ffi::AV_CODEC_ID_MP3 | ffi::AV_CODEC_ID_OPUS
            | ffi::AV_CODEC_ID_VORBIS | ffi::AV_CODEC_ID_FLAC),
        _ => false,
    }
}

fn container_name(path: &str) -> &'static str {
    let l = path.to_ascii_lowercase();
    if l.ends_with(".mov") { "mov" }
    else if l.ends_with(".mkv") { "matroska" }
    else if l.ends_with(".m4a") { "ipod" }
    else { "mp4" }
}

/// A bitrate that will not visibly hurt, when the caller has no opinion.
///
/// S4b is the reason this is not a round number pulled from the air: hardware
/// encoders need materially more bitrate than libx264 for the same look, so a
/// figure copied from an x264 guide would quietly under-run. Scaled by pixel
/// count and frame rate, then floored — a small clip still deserves enough bits
/// to look like its source.
pub(crate) fn default_bitrate_kbps(width: i32, height: i32, fps: f64, hevc: bool) -> i32 {
    let pixels = (width as f64) * (height as f64);
    let per_pixel_per_frame = if hevc { 0.075 } else { 0.10 };
    let kbps = pixels * fps.clamp(1.0, 120.0) * per_pixel_per_frame / 1000.0;
    (kbps.round() as i32).clamp(400, 80_000)
}

/// Open a video transcode.
///
/// `width`/`height` of 0 keep the source size; giving one of them scales to fit
/// while preserving the aspect ratio, which is what the resize tool wants.
/// `bitrate_kbps` of 0 asks for the default above.
#[no_mangle]
pub unsafe extern "C" fn mc_video_convert_open(input: *const c_char,
                                               output: *const c_char,
                                               codec_kind: i32,
                                               bitrate_kbps: i32,
                                               width: i32,
                                               height: i32) -> *mut MCVideoConvert {
    if input.is_null() || output.is_null() { return ptr::null_mut(); }
    let cin = CStr::from_ptr(input);
    let cout = CStr::from_ptr(output);
    let out_path = cout.to_string_lossy().into_owned();
    let container = container_name(&out_path);

    let mut in_fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut in_fmt, cin.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return ptr::null_mut();
    }
    if ffi::avformat_find_stream_info(in_fmt, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    let v_in = ffi::av_find_best_stream(in_fmt, ffi::AVMEDIA_TYPE_VIDEO, -1, -1,
                                        ptr::null_mut(), 0);
    if v_in < 0 {
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }
    let a_in = ffi::av_find_best_stream(in_fmt, ffi::AVMEDIA_TYPE_AUDIO, -1, -1,
                                        ptr::null_mut(), 0);

    // ---- decoder ---------------------------------------------------------
    let v_st = *(*in_fmt).streams.offset(v_in as isize);
    let v_par = (*v_st).codecpar;
    let dec_codec = ffi::avcodec_find_decoder((*v_par).codec_id);
    if dec_codec.is_null() {
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }
    let dec = ffi::avcodec_alloc_context3(dec_codec);
    ffi::avcodec_parameters_to_context(dec, v_par);
    (*dec).pkt_timebase = (*v_st).time_base;
    if ffi::avcodec_open2(dec, dec_codec, ptr::null_mut()) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    // ---- target geometry -------------------------------------------------
    let src_w = (*dec).width;
    let src_h = (*dec).height;
    let (mut out_w, mut out_h) = match (width, height) {
        (0, 0) => (src_w, src_h),
        (w, 0) if w > 0 => (w, (w as i64 * src_h as i64 / src_w.max(1) as i64) as i32),
        (0, h) if h > 0 => ((h as i64 * src_w as i64 / src_h.max(1) as i64) as i32, h),
        (w, h) => (w, h),
    };
    // H.264 and HEVC need even dimensions, and hardware encoders reject odd
    // ones outright rather than rounding for you.
    out_w &= !1;
    out_h &= !1;
    if out_w < 2 || out_h < 2 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    // ---- output container ------------------------------------------------
    let cname = CString::new(container).unwrap_or_default();
    let mut out_fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_alloc_output_context2(&mut out_fmt, ptr::null_mut(),
                                           cname.as_ptr(), cout.as_ptr()) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    // ---- video encoder ---------------------------------------------------
    let hevc = codec_kind != 1;
    let enc_codec = crate::platform::find_encoder(crate::platform::video_encoders(hevc))
        .map(|(c, _)| c).unwrap_or(ptr::null());
    if enc_codec.is_null() {
        ffi::avformat_free_context(out_fmt);
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    // Keep the SOURCE's timebase rather than imposing 1/fps. A variable frame
    // rate source (O-3's VFR case: screen recordings especially) keeps its
    // original timing this way instead of being resampled to a nominal rate.
    let src_tb = (*v_st).time_base;
    let fr = (*v_st).avg_frame_rate;
    let fps = if fr.den > 0 && fr.num > 0 { fr.num as f64 / fr.den as f64 } else { 30.0 };

    let enc = ffi::avcodec_alloc_context3(enc_codec);
    (*enc).width = out_w;
    (*enc).height = out_h;
    (*enc).pix_fmt = ffi::AV_PIX_FMT_NV12;
    (*enc).time_base = src_tb;
    (*enc).framerate = fr;
    (*enc).sample_aspect_ratio = (*dec).sample_aspect_ratio;
    let kbps = if bitrate_kbps > 0 { bitrate_kbps }
               else { default_bitrate_kbps(out_w, out_h, fps, hevc) };
    (*enc).bit_rate = kbps as i64 * 1000;
    // O-3: V1 is an SDR Rec.709 pipeline, and an untagged file is guessed at
    // differently by every player.
    // Without an explicit range the encoder warns and guesses. Limited range
    // is what NV12 from a normal source carries.
    (*enc).color_range = ffi::AVCOL_RANGE_MPEG;
    (*enc).color_primaries = ffi::AVCOL_PRI_BT709;
    (*enc).color_trc = ffi::AVCOL_TRC_BT709;
    (*enc).colorspace = ffi::AVCOL_SPC_BT709;
    if (*(*out_fmt).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
        (*enc).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
    }
    if ffi::avcodec_open2(enc, enc_codec, ptr::null_mut()) < 0 {
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_free_context(out_fmt);
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }
    let v_stream = ffi::avformat_new_stream(out_fmt, ptr::null());
    ffi::avcodec_parameters_from_context((*v_stream).codecpar, enc);
    (*v_stream).time_base = (*enc).time_base;
    let v_out = (*v_stream).index;

    // ---- audio -----------------------------------------------------------
    let mut a_out = -1i32;
    let mut audio_copy = false;
    let mut adec: *mut ffi::AVCodecContext = ptr::null_mut();
    let mut aenc: *mut ffi::AVCodecContext = ptr::null_mut();
    let mut swr: *mut ffi::SwrContext = ptr::null_mut();
    let mut aenc_tb = ffi::AVRational { num: 1, den: 48_000 };

    if a_in >= 0 {
        let a_st = *(*in_fmt).streams.offset(a_in as isize);
        let a_par = (*a_st).codecpar;
        if audio_copyable(container, (*a_par).codec_id) {
            let st = ffi::avformat_new_stream(out_fmt, ptr::null());
            if !st.is_null() {
                ffi::avcodec_parameters_copy((*st).codecpar, a_par);
                // codec_tag belongs to the SOURCE container; carrying it over
                // produces a file some players reject.
                (*(*st).codecpar).codec_tag = 0;
                a_out = (*st).index;
                audio_copy = true;
            }
        }
        if !audio_copy {
            // Decode and re-encode to AAC. Same lesson as the export path:
            // ask the encoder which sample format it takes, and open the codec
            // BEFORE creating the stream, so a failure leaves no half-formed
            // stream behind for the muxer to choke on.
            let d_codec = ffi::avcodec_find_decoder((*a_par).codec_id);
            let e_codec = crate::platform::find_encoder(crate::platform::aac_encoders())
                .map(|(c, _)| c).unwrap_or(ptr::null());
            if !d_codec.is_null() && !e_codec.is_null() {
                let d = ffi::avcodec_alloc_context3(d_codec);
                ffi::avcodec_parameters_to_context(d, a_par);
                (*d).pkt_timebase = (*a_st).time_base;
                if ffi::avcodec_open2(d, d_codec, ptr::null_mut()) >= 0 {
                    let rate = if (*d).sample_rate > 0 { (*d).sample_rate } else { 48_000 };
                    let mut cfg: *const c_void = ptr::null();
                    let mut count: i32 = 0;
                    let fmt_id = if ffi::avcodec_get_supported_config(
                            ptr::null(), e_codec, ffi::AV_CODEC_CONFIG_SAMPLE_FORMAT, 0,
                            &mut cfg as *mut _ as *mut *const c_void, &mut count) >= 0
                        && !cfg.is_null() && count > 0
                    { *(cfg as *const i32) } else { ffi::AV_SAMPLE_FMT_FLTP };

                    let e = ffi::avcodec_alloc_context3(e_codec);
                    (*e).sample_fmt = fmt_id;
                    (*e).sample_rate = rate;
                    (*e).bit_rate = 192_000;
                    ffi::av_channel_layout_default(&mut (*e).ch_layout, 2);
                    (*e).time_base = ffi::AVRational { num: 1, den: rate };
                    if (*(*out_fmt).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
                        (*e).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                    }
                    if ffi::avcodec_open2(e, e_codec, ptr::null_mut()) >= 0 {
                        let st = ffi::avformat_new_stream(out_fmt, ptr::null());
                        if !st.is_null() {
                            ffi::avcodec_parameters_from_context((*st).codecpar, e);
                            (*st).time_base = (*e).time_base;
                            a_out = (*st).index;
                            aenc = e;
                            aenc_tb = (*e).time_base;
                            adec = d;
                            let mut layout: ffi::AVChannelLayout = std::mem::zeroed();
                            ffi::av_channel_layout_default(&mut layout, 2);
                            ffi::swr_alloc_set_opts2(
                                &mut swr,
                                &layout, ffi::AV_SAMPLE_FMT_FLT, rate,
                                &(*d).ch_layout, (*d).sample_fmt, (*d).sample_rate,
                                0, ptr::null_mut());
                            if !swr.is_null() && ffi::swr_init(swr) < 0 {
                                ffi::swr_free(&mut swr);
                            }
                        }
                    }
                    if a_out < 0 {
                        let mut e2 = e; ffi::avcodec_free_context(&mut e2);
                        let mut d2 = d; ffi::avcodec_free_context(&mut d2);
                    }
                } else {
                    let mut d2 = d; ffi::avcodec_free_context(&mut d2);
                }
            }
            // No audio stream is better than a broken one: a source whose audio
            // cannot be handled still converts, silently rather than not at all.
        }
    }

    if (*(*out_fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0
        && ffi::avio_open(&mut (*out_fmt).pb, cout.as_ptr(), ffi::AVIO_FLAG_WRITE as i32) < 0
    {
        ffi::avformat_free_context(out_fmt);
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }
    if ffi::avformat_write_header(out_fmt, ptr::null_mut()) < 0 {
        ffi::avio_closep(&mut (*out_fmt).pb);
        ffi::avformat_free_context(out_fmt);
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        let mut e = enc; ffi::avcodec_free_context(&mut e);
        ffi::avformat_close_input(&mut in_fmt);
        return ptr::null_mut();
    }

    // Read timebases back AFTER the header: muxers rewrite them (MP4 imposes
    // 1/90000), and rescaling to the value set beforehand produces a file whose
    // duration is wrong by whatever ratio the muxer chose.
    let v_stream_tb = (*(*(*out_fmt).streams.offset(v_out as isize))).time_base;
    let a_stream_tb = if a_out >= 0 {
        (*(*(*out_fmt).streams.offset(a_out as isize))).time_base
    } else { ffi::AVRational { num: 1, den: 1 } };

    let duration_us = if (*in_fmt).duration != ffi::AV_NOPTS_VALUE { (*in_fmt).duration } else { 0 };

    Box::into_raw(Box::new(MCVideoConvert {
        in_fmt, out_fmt,
        v_in, v_out, dec, enc,
        sws: ptr::null_mut(),
        scaled: ptr::null_mut(),
        a_in, a_out, audio_copy, adec, aenc, swr,
        aframe: ffi::av_frame_alloc(),
        abuf: Vec::new(),
        asamples: 0,
        packet: ffi::av_packet_alloc(),
        frame: ffi::av_frame_alloc(),
        out_pkt: ffi::av_packet_alloc(),
        v_stream_tb, a_stream_tb,
        enc_tb: src_tb,
        aenc_tb,
        duration_us,
        progress_us: 0,
        frames_encoded: 0,
        draining: false,
        finished: false,
        last_error: CString::default(),
    }))
}

/// Push one encoded video/audio packet to the muxer.
unsafe fn write_packet(out_fmt: *mut ffi::AVFormatContext, pkt: *mut ffi::AVPacket,
                       stream: i32, from: ffi::AVRational, to: ffi::AVRational) {
    (*pkt).stream_index = stream;
    ffi::av_packet_rescale_ts(pkt, from, to);
    (*pkt).pos = -1;
    ffi::av_interleaved_write_frame(out_fmt, pkt);
    ffi::av_packet_unref(pkt);
}

unsafe fn drain_video(c: &mut MCVideoConvert) {
    while ffi::avcodec_receive_packet(c.enc, c.out_pkt) == 0 {
        c.frames_encoded += 1;
        write_packet(c.out_fmt, c.out_pkt, c.v_out, c.enc_tb, c.v_stream_tb);
    }
}

unsafe fn drain_audio(c: &mut MCVideoConvert) {
    while ffi::avcodec_receive_packet(c.aenc, c.out_pkt) == 0 {
        write_packet(c.out_fmt, c.out_pkt, c.a_out, c.aenc_tb, c.a_stream_tb);
    }
}

/// Scale and/or convert a decoded frame into the NV12 the encoder wants.
///
/// Built on first use rather than up front, because the decoder's real pixel
/// format is not reliably known until a frame comes out of it.
unsafe fn to_encoder_frame(c: &mut MCVideoConvert) -> Result<*mut ffi::AVFrame, i32> {
    let src_fmt = (*c.frame).format;
    let same = src_fmt == ffi::AV_PIX_FMT_NV12
        && (*c.frame).width == (*c.enc).width
        && (*c.frame).height == (*c.enc).height;
    if same { return Ok(c.frame); }

    if c.sws.is_null() {
        c.sws = ffi::sws_getContext(
            (*c.frame).width, (*c.frame).height, src_fmt,
            (*c.enc).width, (*c.enc).height, ffi::AV_PIX_FMT_NV12,
            ffi::SWS_BILINEAR as i32, ptr::null_mut(), ptr::null_mut(), ptr::null());
        if c.sws.is_null() { return Err(-1); }
    }
    if c.scaled.is_null() {
        c.scaled = ffi::av_frame_alloc();
        (*c.scaled).format = ffi::AV_PIX_FMT_NV12;
        (*c.scaled).width = (*c.enc).width;
        (*c.scaled).height = (*c.enc).height;
        if ffi::av_frame_get_buffer(c.scaled, 0) < 0 { return Err(-2); }
    }
    ffi::sws_scale(c.sws,
                   (*c.frame).data.as_ptr() as *const *const u8,
                   (*c.frame).linesize.as_ptr(),
                   0, (*c.frame).height,
                   (*c.scaled).data.as_ptr() as *const *mut u8,
                   (*c.scaled).linesize.as_ptr());
    (*c.scaled).pts = (*c.frame).pts;
    Ok(c.scaled)
}

unsafe fn encode_decoded_video(c: &mut MCVideoConvert) -> i32 {
    while ffi::avcodec_receive_frame(c.dec, c.frame) == 0 {
        // best_effort_timestamp copes with sources whose packets carry only a
        // dts, which is common in the containers that end up here.
        let ts = if (*c.frame).best_effort_timestamp != ffi::AV_NOPTS_VALUE {
            (*c.frame).best_effort_timestamp
        } else { (*c.frame).pts };
        (*c.frame).pts = ts;
        if ts != ffi::AV_NOPTS_VALUE {
            c.progress_us = ffi::av_rescale_q(ts, c.enc_tb,
                                              ffi::AVRational { num: 1, den: 1_000_000 });
        }
        let f = match to_encoder_frame(c) {
            Ok(f) => f,
            Err(_) => return err(c, "This video could not be resized."),
        };
        (*f).pts = ts;
        if ffi::avcodec_send_frame(c.enc, f) < 0 {
            return err(c, "The video could not be encoded.");
        }
        drain_video(c);
        ffi::av_frame_unref(c.frame);
    }
    0
}

unsafe fn encode_decoded_audio(c: &mut MCVideoConvert) -> i32 {
    if c.aenc.is_null() || c.swr.is_null() { return 0; }
    while ffi::avcodec_receive_frame(c.adec, c.aframe) == 0 {
        let max_out = ffi::swr_get_out_samples(c.swr, (*c.aframe).nb_samples);
        if max_out <= 0 { ffi::av_frame_unref(c.aframe); continue; }
        let mut scratch = vec![0f32; max_out as usize * 2];
        let mut out_ptr = scratch.as_mut_ptr() as *mut u8;
        let got = ffi::swr_convert(c.swr, &mut out_ptr, max_out,
                                   (*c.aframe).data.as_ptr() as *const *const u8,
                                   (*c.aframe).nb_samples);
        ffi::av_frame_unref(c.aframe);
        if got <= 0 { continue; }
        c.abuf.extend_from_slice(&scratch[..got as usize * 2]);

        let n = if (*c.aenc).frame_size > 0 { (*c.aenc).frame_size as usize } else { 1024 };
        while c.abuf.len() >= n * 2 {
            ffi::av_frame_unref(c.aframe);
            (*c.aframe).nb_samples = n as i32;
            (*c.aframe).format = (*c.aenc).sample_fmt;
            ffi::av_channel_layout_copy(&mut (*c.aframe).ch_layout, &(*c.aenc).ch_layout);
            (*c.aframe).sample_rate = (*c.aenc).sample_rate;
            if ffi::av_frame_get_buffer(c.aframe, 0) < 0 { return -1; }
            if crate::convert::write_samples(c.aframe, (*c.aenc).sample_fmt, &c.abuf, n) < 0 {
                return -1;
            }
            (*c.aframe).pts = c.asamples;
            c.asamples += n as i64;
            c.abuf.drain(0..n * 2);
            if ffi::avcodec_send_frame(c.aenc, c.aframe) < 0 { return -1; }
            drain_audio(c);
        }
    }
    0
}

/// Process one packet. 1 = more work, 0 = finished, negative = error.
#[no_mangle]
pub unsafe extern "C" fn mc_video_convert_step(c: *mut MCVideoConvert) -> i32 {
    let Some(c) = c.as_mut() else { return -1 };
    if c.finished { return 0; }

    if !c.draining {
        if ffi::av_read_frame(c.in_fmt, c.packet) < 0 {
            // Flush both decoders, then both encoders, in that order — frames
            // still inside a decoder have to reach the encoder before it is
            // told there is nothing more coming.
            c.draining = true;
            ffi::avcodec_send_packet(c.dec, ptr::null());
            if encode_decoded_video(c) < 0 { return -1; }
            if !c.adec.is_null() {
                ffi::avcodec_send_packet(c.adec, ptr::null());
                if encode_decoded_audio(c) < 0 { return -1; }
            }
            ffi::avcodec_send_frame(c.enc, ptr::null());
            drain_video(c);
            if !c.aenc.is_null() {
                ffi::avcodec_send_frame(c.aenc, ptr::null());
                drain_audio(c);
            }
            c.finished = true;
            return 0;
        }

        let idx = (*c.packet).stream_index;
        if idx == c.v_in {
            if ffi::avcodec_send_packet(c.dec, c.packet) < 0 {
                ffi::av_packet_unref(c.packet);
                return err(c, "This video could not be decoded.");
            }
            ffi::av_packet_unref(c.packet);
            if encode_decoded_video(c) < 0 { return -1; }
            return 1;
        }
        if idx == c.a_in && c.a_out >= 0 {
            if c.audio_copy {
                let in_st = *(*c.in_fmt).streams.offset(idx as isize);
                write_packet(c.out_fmt, c.packet, c.a_out,
                             (*in_st).time_base, c.a_stream_tb);
                return 1;
            }
            if !c.adec.is_null() {
                if ffi::avcodec_send_packet(c.adec, c.packet) >= 0 {
                    ffi::av_packet_unref(c.packet);
                    if encode_decoded_audio(c) < 0 { return -1; }
                    return 1;
                }
            }
        }
        ffi::av_packet_unref(c.packet);
        return 1;                       // a stream we are not carrying
    }
    c.finished = true;
    0
}

/// 0.0–1.0. Falls back to 0 when the source reports no duration.
#[no_mangle]
pub unsafe extern "C" fn mc_video_convert_progress(c: *const MCVideoConvert) -> f32 {
    let Some(c) = c.as_ref() else { return 0.0 };
    if c.finished { return 1.0; }
    if c.duration_us <= 0 { return 0.0; }
    (c.progress_us as f32 / c.duration_us as f32).clamp(0.0, 1.0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_video_convert_frames(c: *const MCVideoConvert) -> i64 {
    c.as_ref().map(|c| c.frames_encoded).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_video_convert_error(c: *const MCVideoConvert) -> *const c_char {
    match c.as_ref() { Some(c) => c.last_error.as_ptr(), None => ptr::null() }
}

/// Finish and close. Writes the trailer, so a cancelled job leaves a playable
/// (if short) file rather than a corrupt one.
#[no_mangle]
pub unsafe extern "C" fn mc_video_convert_close(c: *mut MCVideoConvert) -> i32 {
    if c.is_null() { return -1; }
    let mut c = Box::from_raw(c);
    ffi::av_write_trailer(c.out_fmt);
    if (*(*c.out_fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0 {
        ffi::avio_closep(&mut (*c.out_fmt).pb);
    }
    ffi::av_packet_free(&mut c.packet);
    ffi::av_packet_free(&mut c.out_pkt);
    ffi::av_frame_free(&mut c.frame);
    ffi::av_frame_free(&mut c.aframe);
    if !c.scaled.is_null() { ffi::av_frame_free(&mut c.scaled); }
    if !c.sws.is_null() { ffi::sws_freeContext(c.sws); }
    if !c.swr.is_null() { ffi::swr_free(&mut c.swr); }
    if !c.adec.is_null() { ffi::avcodec_free_context(&mut c.adec); }
    if !c.aenc.is_null() { ffi::avcodec_free_context(&mut c.aenc); }
    ffi::avcodec_free_context(&mut c.dec);
    ffi::avcodec_free_context(&mut c.enc);
    ffi::avformat_free_context(c.out_fmt);
    ffi::avformat_close_input(&mut c.in_fmt);
    0
}
