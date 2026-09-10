//! Joining several files into one (§31's merge tool).
//!
//! Two paths, chosen by inspecting the inputs.
//!
//! **Copy** when every file already agrees — same codecs, same size, same
//! audio layout. Clips off one phone or one camera almost always do, and then
//! merging is a timestamp-shifting exercise: instant, and bit-for-bit the
//! original footage. This is the same instinct as `convert.rs`'s remux, and for
//! the same reason: the fastest work is the work not done.
//!
//! **Re-encode** when they do not. Mixed sources are the case that makes merge
//! hard rather than the case to refuse — footage from a phone and a camera in
//! one video is exactly what someone is trying to make. Everything is brought
//! to the FIRST file's geometry and frame rate, which is the one the user
//! chose first and the one they are most likely to have meant.
//!
//! Stepwise, like every other long operation here.

use rusty_ffmpeg::ffi;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

pub const MC_MERGE_COPY: i32 = 0;
pub const MC_MERGE_ENCODE: i32 = 1;

pub struct MCMerge {
    inputs: Vec<CString>,
    /// Which input is being read now.
    index: usize,
    in_fmt: *mut ffi::AVFormatContext,
    out_fmt: *mut ffi::AVFormatContext,
    mode: i32,

    /// input stream index -> output stream index for the current input.
    stream_map: Vec<i32>,
    /// Where the current input starts on the output's clock, in microseconds.
    ///
    /// ONE offset for every stream, not one per stream. Per-stream offsets let
    /// audio and video start each segment at slightly different points — the
    /// two rarely end on the same timestamp — and that difference accumulates
    /// into visible lip-sync drift by the third or fourth clip.
    offset_us: i64,
    /// Furthest point written so far, in microseconds, across all streams.
    max_end_us: i64,
    /// Where the current input's own clock starts, in microseconds.
    ///
    /// Not always zero, and not always positive. AAC in MP4 carries encoder
    /// priming as a NEGATIVE first timestamp, so a file that plays from 0
    /// reports its audio starting at -1024 samples. Adding an offset without
    /// subtracting this puts the next segment's first audio packet *before*
    /// the previous segment's last one, and the muxer rejects the file.
    /// Taken from the container rather than per stream, so the relative
    /// alignment of picture and sound is preserved exactly.
    start_us: i64,
    /// Last dts written per output stream, as a backstop for sources whose
    /// timestamps are not monotonic to begin with.
    last_dts: Vec<i64>,

    // Re-encode path.
    dec: *mut ffi::AVCodecContext,
    enc: *mut ffi::AVCodecContext,
    sws: *mut ffi::SwsContext,
    scaled: *mut ffi::AVFrame,
    v_in: i32,
    frames_encoded: i64,
    /// Last pts written on the encoded path, so output stays strictly
    /// increasing across a join.
    last_enc_pts: i64,
    enc_tb: ffi::AVRational,
    v_stream_tb: ffi::AVRational,

    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    out_pkt: *mut ffi::AVPacket,

    total_us: i64,
    done_us: i64,
    finished: bool,
    last_error: CString,
}

/// What has to match for the streams to be copied rather than re-encoded.
#[derive(PartialEq)]
struct Shape {
    video: (i32, i32, i32),      // codec id, width, height
    audio: Option<(i32, i32, i32)>, // codec id, sample rate, channels
}

unsafe fn shape_of(path: &CStr) -> Option<Shape> {
    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut fmt, path.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return None;
    }
    if ffi::avformat_find_stream_info(fmt, ptr::null_mut()) < 0 {
        ffi::avformat_close_input(&mut fmt);
        return None;
    }
    let mut video = None;
    let mut audio = None;
    for i in 0..(*fmt).nb_streams {
        let par = (*(*(*fmt).streams.offset(i as isize))).codecpar;
        match (*par).codec_type {
            t if t == ffi::AVMEDIA_TYPE_VIDEO && video.is_none() =>
                video = Some(((*par).codec_id as i32, (*par).width, (*par).height)),
            t if t == ffi::AVMEDIA_TYPE_AUDIO && audio.is_none() =>
                audio = Some(((*par).codec_id as i32, (*par).sample_rate,
                              (*par).ch_layout.nb_channels)),
            _ => {}
        }
    }
    ffi::avformat_close_input(&mut fmt);
    video.map(|v| Shape { video: v, audio })
}

fn err(m: &mut MCMerge, msg: &str) -> i32 {
    m.last_error = CString::new(msg).unwrap_or_default();
    -1
}

/// Open a merge over `count` input paths, in the order given.
///
/// The order is the caller's: files are joined as listed, never re-sorted, so
/// "part 1, part 2, part 3" means what it says.
#[no_mangle]
pub unsafe extern "C" fn mc_merge_open(inputs: *const *const c_char, count: i32,
                                       output: *const c_char) -> *mut MCMerge {
    if inputs.is_null() || output.is_null() || count < 1 { return ptr::null_mut(); }
    let mut paths: Vec<CString> = Vec::with_capacity(count as usize);
    for i in 0..count as isize {
        let p = *inputs.offset(i);
        if p.is_null() { return ptr::null_mut(); }
        paths.push(CStr::from_ptr(p).to_owned());
    }

    // Every file must be readable before anything is written. Discovering the
    // third file is unreadable halfway through leaves a half-made video.
    let shapes: Vec<Shape> = match paths.iter().map(|p| shape_of(p)).collect::<Option<Vec<_>>>() {
        Some(s) => s,
        None => return ptr::null_mut(),
    };
    let can_copy = shapes.windows(2).all(|w| w[0] == w[1]);

    let cout = CStr::from_ptr(output);
    let mut out_fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_alloc_output_context2(&mut out_fmt, ptr::null_mut(),
                                           ptr::null(), cout.as_ptr()) < 0 {
        return ptr::null_mut();
    }

    let mut total_us = 0i64;
    for p in &paths {
        let mut f: *mut ffi::AVFormatContext = ptr::null_mut();
        if ffi::avformat_open_input(&mut f, p.as_ptr(), ptr::null(), ptr::null_mut()) >= 0 {
            if ffi::avformat_find_stream_info(f, ptr::null_mut()) >= 0
                && (*f).duration != ffi::AV_NOPTS_VALUE {
                total_us += (*f).duration;
            }
            ffi::avformat_close_input(&mut f);
        }
    }

    let mut m = Box::new(MCMerge {
        inputs: paths,
        index: 0,
        in_fmt: ptr::null_mut(),
        out_fmt,
        mode: if can_copy { MC_MERGE_COPY } else { MC_MERGE_ENCODE },
        stream_map: Vec::new(),
        offset_us: 0,
        max_end_us: 0,
        start_us: 0,
        last_dts: Vec::new(),
        dec: ptr::null_mut(),
        enc: ptr::null_mut(),
        sws: ptr::null_mut(),
        scaled: ptr::null_mut(),
        v_in: -1,
        frames_encoded: 0,
        last_enc_pts: i64::MIN,
        enc_tb: ffi::AVRational { num: 1, den: 30 },
        v_stream_tb: ffi::AVRational { num: 1, den: 30 },
        packet: ffi::av_packet_alloc(),
        frame: ffi::av_frame_alloc(),
        out_pkt: ffi::av_packet_alloc(),
        total_us,
        done_us: 0,
        finished: false,
        last_error: CString::default(),
    });

    // The output streams are built from the FIRST input, then every later input
    // is mapped onto them.
    if open_input(&mut m, 0) < 0 { return ptr::null_mut(); }

    if can_copy {
        for i in 0..(*m.in_fmt).nb_streams {
            let st = *(*m.in_fmt).streams.offset(i as isize);
            let t = (*(*st).codecpar).codec_type;
            if t != ffi::AVMEDIA_TYPE_VIDEO && t != ffi::AVMEDIA_TYPE_AUDIO { continue; }
            let out_st = ffi::avformat_new_stream(m.out_fmt, ptr::null());
            if out_st.is_null() { continue; }
            ffi::avcodec_parameters_copy((*out_st).codecpar, (*st).codecpar);
            (*(*out_st).codecpar).codec_tag = 0;
        }
    } else if build_encoder(&mut m) < 0 {
        return ptr::null_mut();
    }

    if (*(*m.out_fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0
        && ffi::avio_open(&mut (*m.out_fmt).pb, cout.as_ptr(), ffi::AVIO_FLAG_WRITE as i32) < 0 {
        return ptr::null_mut();
    }
    if ffi::avformat_write_header(m.out_fmt, ptr::null_mut()) < 0 {
        return ptr::null_mut();
    }

    if m.mode == MC_MERGE_ENCODE {
        m.v_stream_tb = (*(*(*m.out_fmt).streams.offset(0))).time_base;
    }
    m.last_dts = vec![i64::MIN; (*m.out_fmt).nb_streams as usize];
    map_streams(&mut m);
    Box::into_raw(m)
}

/// Open input `i` and make it current.
unsafe fn open_input(m: &mut MCMerge, i: usize) -> i32 {
    if !m.in_fmt.is_null() { ffi::avformat_close_input(&mut m.in_fmt); }
    let Some(path) = m.inputs.get(i) else { return -1 };
    if ffi::avformat_open_input(&mut m.in_fmt, path.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return -1;
    }
    if ffi::avformat_find_stream_info(m.in_fmt, ptr::null_mut()) < 0 { return -1; }
    m.index = i;
    m.start_us = if (*m.in_fmt).start_time != ffi::AV_NOPTS_VALUE {
        (*m.in_fmt).start_time
    } else { 0 };
    m.v_in = ffi::av_find_best_stream(m.in_fmt, ffi::AVMEDIA_TYPE_VIDEO, -1, -1,
                                      ptr::null_mut(), 0);
    0
}

/// Map the current input's streams onto the output's, by media type and order.
unsafe fn map_streams(m: &mut MCMerge) {
    let n_in = (*m.in_fmt).nb_streams as usize;
    m.stream_map = vec![-1; n_in];
    if m.mode == MC_MERGE_ENCODE { return; }
    let mut next_video = 0usize;
    let mut next_audio = 0usize;
    let out_n = (*m.out_fmt).nb_streams as usize;
    for i in 0..n_in {
        let t = (*(*(*(*m.in_fmt).streams.offset(i as isize))).codecpar).codec_type;
        let want = if t == ffi::AVMEDIA_TYPE_VIDEO { &mut next_video }
                       else if t == ffi::AVMEDIA_TYPE_AUDIO { &mut next_audio }
                       else { continue };
        let mut seen = 0usize;
        for o in 0..out_n {
            let ot = (*(*(*(*m.out_fmt).streams.offset(o as isize))).codecpar).codec_type;
            if ot != t { continue; }
            if seen == *want { m.stream_map[i] = o as i32; break; }
            seen += 1;
        }
        *want += 1;
    }
}

unsafe fn build_encoder(m: &mut MCMerge) -> i32 {
    if m.v_in < 0 { return -1; }
    let st = *(*m.in_fmt).streams.offset(m.v_in as isize);
    let par = (*st).codecpar;
    // HEVC by default: S4b measured it needing ~1.74x libx264's bitrate against
    // ~2.31x for hardware H.264, which makes it the better default here too.
    let Some((codec, _)) = crate::platform::find_encoder(
        crate::platform::video_encoders(true)) else { return -1 };
    let fr = (*st).avg_frame_rate;
    let rate = if fr.num > 0 && fr.den > 0 { fr } else { ffi::AVRational { num: 30, den: 1 } };

    let enc = ffi::avcodec_alloc_context3(codec);
    (*enc).width = (*par).width & !1;
    (*enc).height = (*par).height & !1;
    (*enc).pix_fmt = ffi::AV_PIX_FMT_NV12;
    // A FIXED rate for the merged file. Inputs may disagree about frame rate,
    // and a container carrying two timebases is a file that plays wrong
    // somewhere; the first input's rate is the one the user chose first.
    (*enc).time_base = ffi::AVRational { num: rate.den, den: rate.num };
    (*enc).framerate = rate;
    (*enc).bit_rate = crate::transcode::default_bitrate_kbps(
        (*enc).width, (*enc).height,
        rate.num as f64 / rate.den.max(1) as f64, true) as i64 * 1000;
    (*enc).color_range = ffi::AVCOL_RANGE_MPEG;
    (*enc).color_primaries = ffi::AVCOL_PRI_BT709;
    (*enc).color_trc = ffi::AVCOL_TRC_BT709;
    (*enc).colorspace = ffi::AVCOL_SPC_BT709;
    if (*(*m.out_fmt).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
        (*enc).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
    }
    if ffi::avcodec_open2(enc, codec, ptr::null_mut()) < 0 { return -1; }
    let out_st = ffi::avformat_new_stream(m.out_fmt, ptr::null());
    if out_st.is_null() { return -1; }
    ffi::avcodec_parameters_from_context((*out_st).codecpar, enc);
    (*out_st).time_base = (*enc).time_base;
    m.enc = enc;
    m.enc_tb = (*enc).time_base;
    0
}

/// Open a decoder for the current input's video stream.
unsafe fn open_decoder(m: &mut MCMerge) -> i32 {
    if !m.dec.is_null() { ffi::avcodec_free_context(&mut m.dec); }
    if m.v_in < 0 { return -1; }
    let st = *(*m.in_fmt).streams.offset(m.v_in as isize);
    let par = (*st).codecpar;
    let codec = ffi::avcodec_find_decoder((*par).codec_id);
    if codec.is_null() { return -1; }
    let dec = ffi::avcodec_alloc_context3(codec);
    ffi::avcodec_parameters_to_context(dec, par);
    (*dec).pkt_timebase = (*st).time_base;
    if ffi::avcodec_open2(dec, codec, ptr::null_mut()) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        return -1;
    }
    m.dec = dec;
    // Each input's frames are re-timed onto the output's own clock, so the join
    // is seamless whatever the sources' timestamps were.
    if !m.sws.is_null() { ffi::sws_freeContext(m.sws); m.sws = ptr::null_mut(); }
    0
}

unsafe fn drain_encoder(m: &mut MCMerge) {
    while ffi::avcodec_receive_packet(m.enc, m.out_pkt) == 0 {
        m.frames_encoded += 1;
        (*m.out_pkt).stream_index = 0;
        ffi::av_packet_rescale_ts(m.out_pkt, m.enc_tb, m.v_stream_tb);
        (*m.out_pkt).pos = -1;
        ffi::av_interleaved_write_frame(m.out_fmt, m.out_pkt);
        ffi::av_packet_unref(m.out_pkt);
    }
}

unsafe fn encode_available_frames(m: &mut MCMerge) -> i32 {
    while ffi::avcodec_receive_frame(m.dec, m.frame) == 0 {
        let src_fmt = (*m.frame).format;
        let needs_convert = src_fmt != ffi::AV_PIX_FMT_NV12
            || (*m.frame).width != (*m.enc).width
            || (*m.frame).height != (*m.enc).height;

        let f = if needs_convert {
            if m.sws.is_null() {
                m.sws = ffi::sws_getContext(
                    (*m.frame).width, (*m.frame).height, src_fmt,
                    (*m.enc).width, (*m.enc).height, ffi::AV_PIX_FMT_NV12,
                    ffi::SWS_BILINEAR as i32, ptr::null_mut(), ptr::null_mut(), ptr::null());
                if m.sws.is_null() { return -1; }
            }
            if m.scaled.is_null() {
                m.scaled = ffi::av_frame_alloc();
                (*m.scaled).format = ffi::AV_PIX_FMT_NV12;
                (*m.scaled).width = (*m.enc).width;
                (*m.scaled).height = (*m.enc).height;
                if ffi::av_frame_get_buffer(m.scaled, 0) < 0 { return -1; }
            }
            ffi::sws_scale(m.sws,
                           (*m.frame).data.as_ptr() as *const *const u8,
                           (*m.frame).linesize.as_ptr(),
                           0, (*m.frame).height,
                           (*m.scaled).data.as_ptr() as *const *mut u8,
                           (*m.scaled).linesize.as_ptr());
            m.scaled
        } else { m.frame };

        // Each frame is placed by WHEN IT HAPPENS in its own file, shifted onto
        // the output's clock — not by counting output frames.
        //
        // Counting is the obvious implementation and it is wrong: a 25 fps
        // clip merged after a 30 fps one comes out playing 20% fast, because
        // its 75 frames get 75 slots on a 30 fps clock. Measured — a 3 s clip
        // became 2.5 s. Following the source timestamps keeps every segment at
        // its own real speed.
        let micro = ffi::AVRational { num: 1, den: 1_000_000 };
        let in_tb = (*(*(*m.in_fmt).streams.offset(m.v_in as isize))).time_base;
        let ts = if (*m.frame).best_effort_timestamp != ffi::AV_NOPTS_VALUE {
            (*m.frame).best_effort_timestamp
        } else { (*m.frame).pts };
        let t_us = if ts == ffi::AV_NOPTS_VALUE { 0 }
                   else { ffi::av_rescale_q(ts, in_tb, micro) - m.start_us };
        let mut pts = ffi::av_rescale_q(m.offset_us + t_us.max(0), micro, m.enc_tb);
        if m.last_enc_pts != i64::MIN && pts <= m.last_enc_pts { pts = m.last_enc_pts + 1; }
        m.last_enc_pts = pts;
        m.max_end_us = m.max_end_us
            .max(ffi::av_rescale_q(pts + 1, m.enc_tb, micro));

        (*f).pts = pts;
        if ffi::avcodec_send_frame(m.enc, f) < 0 { return -1; }
        drain_encoder(m);
        ffi::av_frame_unref(m.frame);
    }
    0
}

/// Move to the next input, or finish. 1 = more work, 0 = done.
unsafe fn advance(m: &mut MCMerge) -> i32 {
    // ORDER MATTERS HERE.
    //
    // The decoder still holds the tail of the input that just ended, and those
    // frames belong at the END of the current segment. Advancing the offset
    // first placed them a whole segment late — a five-second gap at the join,
    // measured — and the monotonic guard then dragged everything after them
    // along too. Flush first, shift second.
    if m.mode == MC_MERGE_ENCODE && !m.dec.is_null() {
        ffi::avcodec_send_packet(m.dec, ptr::null());
        if encode_available_frames(m) < 0 {
            return err(m, "The merged video could not be encoded.");
        }
    }

    // Advance by whichever is longer: what the container claimed, or what was
    // actually written. A container that under-reports its duration would
    // otherwise make the next segment start on top of this one's tail, and the
    // muxer rejects the resulting backwards timestamp outright.
    let claimed = current_duration(m).unwrap_or(0).max(0);
    let observed = (m.max_end_us - m.offset_us).max(0);
    let advance_by = claimed.max(observed);
    m.done_us += advance_by;
    m.offset_us += advance_by;

    if m.index + 1 >= m.inputs.len() {
        if m.mode == MC_MERGE_ENCODE {
            ffi::avcodec_send_frame(m.enc, ptr::null());
            drain_encoder(m);
        }
        m.finished = true;
        return 0;
    }
    if open_input(m, m.index + 1) < 0 {
        return err(m, "One of the files could not be read.");
    }
    map_streams(m);
    if m.mode == MC_MERGE_ENCODE && open_decoder(m) < 0 {
        return err(m, "One of the files could not be decoded.");
    }
    1
}

unsafe fn current_duration(m: &MCMerge) -> Option<i64> {
    if m.in_fmt.is_null() { return None; }
    let d = (*m.in_fmt).duration;
    (d != ffi::AV_NOPTS_VALUE).then_some(d)
}

/// Process one packet. 1 = more work, 0 = finished, negative = error.
#[no_mangle]
pub unsafe extern "C" fn mc_merge_step(m: *mut MCMerge) -> i32 {
    let Some(m) = m.as_mut() else { return -1 };
    if m.finished { return 0; }

    // The decoder is opened lazily so the first input goes through exactly the
    // same code path as every later one.
    if m.mode == MC_MERGE_ENCODE && m.dec.is_null() && open_decoder(m) < 0 {
        return err(m, "One of the files could not be decoded.");
    }

    if ffi::av_read_frame(m.in_fmt, m.packet) < 0 {
        return advance(m);
    }
    let idx = (*m.packet).stream_index;

    if m.mode == MC_MERGE_ENCODE {
        if idx == m.v_in {
            if ffi::avcodec_send_packet(m.dec, m.packet) < 0 {
                ffi::av_packet_unref(m.packet);
                return err(m, "One of the files could not be decoded.");
            }
            ffi::av_packet_unref(m.packet);
            if encode_available_frames(m) < 0 {
                return err(m, "The merged video could not be encoded.");
            }
            return 1;
        }
        // Audio in the re-encode path is dropped rather than mangled: mixed
        // sources rarely share an audio format, and a merged file with the
        // wrong sound is worse than one the user adds sound to afterwards.
        ffi::av_packet_unref(m.packet);
        return 1;
    }

    let out_index = m.stream_map.get(idx as usize).copied().unwrap_or(-1);
    if out_index < 0 {
        ffi::av_packet_unref(m.packet);
        return 1;
    }
    let in_st = *(*m.in_fmt).streams.offset(idx as isize);
    let out_st = *(*m.out_fmt).streams.offset(out_index as isize);
    ffi::av_packet_rescale_ts(m.packet, (*in_st).time_base, (*out_st).time_base);

    let micro = ffi::AVRational { num: 1, den: 1_000_000 };
    let shift = ffi::av_rescale_q(m.offset_us - m.start_us, micro, (*out_st).time_base);
    if (*m.packet).pts != ffi::AV_NOPTS_VALUE { (*m.packet).pts += shift; }
    if (*m.packet).dts != ffi::AV_NOPTS_VALUE { (*m.packet).dts += shift; }

    // Backstop. Normalising each input's start handles the ordinary causes;
    // this catches a source whose own timestamps are not monotonic, where the
    // alternative is refusing to write the file at all. One tick is inaudible
    // and invisible; a failed merge is neither.
    let o = out_index as usize;
    if let Some(last) = m.last_dts.get_mut(o) {
        if (*m.packet).dts != ffi::AV_NOPTS_VALUE && *last != i64::MIN
            && (*m.packet).dts <= *last {
            let bump = *last + 1 - (*m.packet).dts;
            (*m.packet).dts += bump;
            if (*m.packet).pts != ffi::AV_NOPTS_VALUE { (*m.packet).pts += bump; }
        }
        if (*m.packet).dts != ffi::AV_NOPTS_VALUE { *last = (*m.packet).dts; }
    }

    let end = (*m.packet).pts.max((*m.packet).dts) + (*m.packet).duration.max(0);
    m.max_end_us = m.max_end_us.max(ffi::av_rescale_q(end, (*out_st).time_base, micro));

    (*m.packet).stream_index = out_index;
    (*m.packet).pos = -1;
    let rc = ffi::av_interleaved_write_frame(m.out_fmt, m.packet);
    ffi::av_packet_unref(m.packet);
    if rc < 0 { return err(m, "The merged file could not be written."); }
    1
}

/// 0 = streams were copied (instant, lossless), 1 = re-encoded.
#[no_mangle]
pub unsafe extern "C" fn mc_merge_mode(m: *const MCMerge) -> i32 {
    m.as_ref().map(|m| m.mode).unwrap_or(-1)
}

#[no_mangle]
pub unsafe extern "C" fn mc_merge_progress(m: *const MCMerge) -> f32 {
    let Some(m) = m.as_ref() else { return 0.0 };
    if m.finished { return 1.0; }
    if m.total_us <= 0 { return 0.0; }
    (m.done_us as f32 / m.total_us as f32).clamp(0.0, 1.0)
}

#[no_mangle]
pub unsafe extern "C" fn mc_merge_error(m: *const MCMerge) -> *const c_char {
    match m.as_ref() { Some(m) => m.last_error.as_ptr(), None => ptr::null() }
}

/// Would merging these files copy the streams, or re-encode them?
///
/// Lets the UI promise "instant" honestly before anything starts.
#[no_mangle]
pub unsafe extern "C" fn mc_merge_would_copy(inputs: *const *const c_char, count: i32) -> i32 {
    if inputs.is_null() || count < 1 { return 0; }
    let mut first: Option<Shape> = None;
    for i in 0..count as isize {
        let p = *inputs.offset(i);
        if p.is_null() { return 0; }
        let Some(s) = shape_of(CStr::from_ptr(p)) else { return 0 };
        match &first {
            None => first = Some(s),
            Some(f) => if *f != s { return 0; },
        }
    }
    1
}

#[no_mangle]
pub unsafe extern "C" fn mc_merge_close(m: *mut MCMerge) -> i32 {
    if m.is_null() { return -1; }
    let mut m = Box::from_raw(m);
    ffi::av_write_trailer(m.out_fmt);
    if (*(*m.out_fmt).oformat).flags & ffi::AVFMT_NOFILE as i32 == 0 {
        ffi::avio_closep(&mut (*m.out_fmt).pb);
    }
    ffi::av_packet_free(&mut m.packet);
    ffi::av_packet_free(&mut m.out_pkt);
    ffi::av_frame_free(&mut m.frame);
    if !m.scaled.is_null() { ffi::av_frame_free(&mut m.scaled); }
    if !m.sws.is_null() { ffi::sws_freeContext(m.sws); }
    if !m.dec.is_null() { ffi::avcodec_free_context(&mut m.dec); }
    if !m.enc.is_null() { ffi::avcodec_free_context(&mut m.enc); }
    ffi::avformat_free_context(m.out_fmt);
    if !m.in_fmt.is_null() { ffi::avformat_close_input(&mut m.in_fmt); }
    0
}
