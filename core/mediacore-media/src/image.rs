//! Still-image loading for overlays (§20).
//!
//! Images take a different path from video: FFmpeg decodes PNG/JPEG/WebP to a
//! CPU-side RGBA buffer rather than a VideoToolbox surface, so there is no
//! hardware frame to hand over. The buffer is uploaded once and cached by the
//! compositor — an overlay logo is decoded a single time, not per frame.
//!
//! §20 requires PNG transparency to work correctly, so the output is RGBA and
//! the alpha channel is preserved through the conversion.

use rusty_ffmpeg::ffi;
use std::ffi::{c_char, CStr};
use std::ptr;

/// Decoded RGBA image. Free with `mc_image_free`.
#[repr(C)]
pub struct MCImage {
    pub width: i32,
    pub height: i32,
    /// Tightly packed RGBA8, `width * height * 4` bytes.
    pub rgba: *mut u8,
    pub bytes: i64,
}

/// Load a still image as RGBA.
///
/// Returns null if the file is not a readable image. §22: an unreadable file
/// must be a refusal, never a crash.
#[no_mangle]
pub unsafe extern "C" fn mc_image_load(path: *const c_char) -> *mut MCImage {
    if path.is_null() { return ptr::null_mut(); }
    let cpath = CStr::from_ptr(path);

    let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
    if ffi::avformat_open_input(&mut fmt, cpath.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
        return ptr::null_mut();
    }
    // For a single still, the image demuxers often cannot report dimensions
    // from the header alone ("unspecified size") and this returns an error.
    // That is NOT fatal: the decoder determines width and height from the
    // frame itself, which is what we read below. Bailing here rejected
    // perfectly valid PNGs.
    let _ = ffi::avformat_find_stream_info(fmt, ptr::null_mut());

    let mut codec: *const ffi::AVCodec = ptr::null();
    let mut si = ffi::av_find_best_stream(fmt, ffi::AVMEDIA_TYPE_VIDEO, -1, -1,
                                          &mut codec as *mut *const ffi::AVCodec, 0);
    // With no stream info, fall back to the first stream and its declared codec.
    if si < 0 && (*fmt).nb_streams > 0 {
        si = 0;
        let st = *(*fmt).streams;
        codec = ffi::avcodec_find_decoder((*(*st).codecpar).codec_id);
    }
    if si < 0 || codec.is_null() {
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let dec = ffi::avcodec_alloc_context3(codec);
    let stream = *(*fmt).streams.offset(si as isize);
    ffi::avcodec_parameters_to_context(dec, (*stream).codecpar);
    // Images decode in software; no hardware context is wanted here.
    if ffi::avcodec_open2(dec, codec, ptr::null_mut()) < 0 {
        let mut d = dec; ffi::avcodec_free_context(&mut d);
        ffi::avformat_close_input(&mut fmt);
        return ptr::null_mut();
    }

    let packet = ffi::av_packet_alloc();
    let frame = ffi::av_frame_alloc();
    let mut decoded = false;

    while ffi::av_read_frame(fmt, packet) >= 0 {
        if (*packet).stream_index == si && ffi::avcodec_send_packet(dec, packet) >= 0 {
            if ffi::avcodec_receive_frame(dec, frame) == 0 { decoded = true; }
        }
        ffi::av_packet_unref(packet);
        if decoded { break; }
    }
    // A still may need flushing before it emits its only frame.
    if !decoded {
        ffi::avcodec_send_packet(dec, ptr::null());
        decoded = ffi::avcodec_receive_frame(dec, frame) == 0;
    }

    let mut result = ptr::null_mut();
    if decoded && (*frame).width > 0 && (*frame).height > 0 {
        let w = (*frame).width;
        let h = (*frame).height;
        // RGBA out, so PNG alpha survives (§20 requires transparency to work).
        let sws = ffi::sws_getContext(w, h, (*frame).format,
                                      w, h, ffi::AV_PIX_FMT_RGBA,
                                      ffi::SWS_BILINEAR as i32,
                                      ptr::null_mut(), ptr::null_mut(), ptr::null());
        if !sws.is_null() {
            let stride = (w * 4) as usize;
            let bytes = stride * h as usize;
            let mut buf = vec![0u8; bytes].into_boxed_slice();
            let dst_data = [buf.as_mut_ptr(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut()];
            let dst_stride = [stride as i32, 0, 0, 0];
            ffi::sws_scale(sws, (*frame).data.as_ptr() as *const *const u8,
                           (*frame).linesize.as_ptr(), 0, h,
                           dst_data.as_ptr(), dst_stride.as_ptr());
            ffi::sws_freeContext(sws);
            let raw = Box::into_raw(buf) as *mut u8;
            result = Box::into_raw(Box::new(MCImage {
                width: w, height: h, rgba: raw, bytes: bytes as i64,
            }));
        }
    }

    let mut f = frame; ffi::av_frame_free(&mut f);
    let mut pk = packet; ffi::av_packet_free(&mut pk);
    let mut d = dec; ffi::avcodec_free_context(&mut d);
    ffi::avformat_close_input(&mut fmt);
    result
}

#[no_mangle]
pub unsafe extern "C" fn mc_image_free(img: *mut MCImage) {
    if img.is_null() { return; }
    let boxed = Box::from_raw(img);
    if !boxed.rgba.is_null() && boxed.bytes > 0 {
        drop(Vec::from_raw_parts(boxed.rgba, boxed.bytes as usize, boxed.bytes as usize));
    }
}
