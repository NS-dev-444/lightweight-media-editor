//! The C ABI the macOS and Windows apps drive.
//!
//! `mediacore-model` is pure Rust with no FFI, deliberately (R-21). This module
//! is the single place it is exposed, and it lives here because this crate
//! already depends on both the model and FFmpeg and already generates the
//! header with cbindgen.
//!
//! Two rules shape the surface:
//!
//! 1. **Bulk reads, not per-item calls.** The timeline draws every frame. S2
//!    measured the draw itself at 0.74–3.3 ms, so a per-clip FFI call would
//!    dominate. `mcs_clips` fills a caller-owned flat array in ONE call.
//! 2. **Pixels never cross.** Frames stay platform buffer handles (S1). This
//!    module carries ids, times and small PODs only.

use mediacore_model::asset::Asset;
use mediacore_model::command::Edit;
use mediacore_model::editor::EditorState;
use mediacore_model::ops;
use mediacore_model::project::{Project, Session};
use mediacore_model::render;
use mediacore_model::time::{FrameRate, Ticks, TimeRange};
use mediacore_model::timeline::{Clip, ClipId, TrackId};
use std::ffi::{c_char, CStr, CString};
use std::ptr;

pub struct MCSession {
    session: Session,
    editor: EditorState,
    /// Kept alive so `mcs_last_error` can hand out a stable pointer.
    last_error: CString,
}

/// One clip, flattened for drawing. Matches what the S2 timeline renderer needs.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MCClipView {
    pub clip_id: u64,
    pub track_id: u64,
    pub asset_id: u64,
    pub track_index: i32,
    /// 0 = video, 1 = overlay, 2 = audio.
    pub track_kind: i32,
    pub start_ticks: i64,
    pub duration_ticks: i64,
    pub source_start_ticks: i64,
    pub gain: f64,
    pub speed: f64,
    pub selected: u8,
    pub muted: u8,
    pub locked: u8,
    pub _pad: u8,
}

/// One layer of a render plan (AD-4: what to draw, not how).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MCPlanLayer {
    pub clip_id: u64,
    pub asset_id: u64,
    pub source_time_ticks: i64,
    pub opacity: f64,
    pub gain: f64,
    pub speed: f64,
    /// 0 = video, 1 = audio.
    pub kind: i32,
    pub is_overlay: i32,
    // --- §18 effects, applied by the platform compositor (AD-4).
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub grayscale: i32,
    pub blur: f32,
    pub sharpen: f32,
    /// 0 = Rec.709, 1 = PQ, 2 = HLG. O-3: HDR must be tone-mapped to SDR
    /// deliberately, and the compositor needs to know which curve to undo.
    pub transfer: i32,
    /// 1 when this layer draws text rather than media; fetch the spec with
    /// `mcs_clip_text`.
    pub is_text: i32,
    /// 1 when this layer draws an image overlay; fetch it with `mcs_clip_image`.
    pub is_image: i32,
    // --- framing: crop, quarter turns, flips. Normalised source coordinates,
    // so the same values are right at any resolution (see `Geometry`).
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_w: f32,
    pub crop_h: f32,
    /// Quarter turns clockwise, 0-3.
    pub rotation: i32,
    pub flip_h: i32,
    pub flip_v: i32,
    /// How much of the clip's colour lookup table to apply, 0 = none. The
    /// table itself is fetched by path with `mcs_clip_lut`; it is megabytes of
    /// data and has no place in a per-frame bulk read.
    pub lut_amount: f32,
    /// 1 when this layer draws a CAPTION rather than a clip's own text. The
    /// compositor asks `mcs_caption_at` for the words.
    pub is_caption: i32,
}

fn ok(_s: &mut MCSession) -> i32 { 0 }

fn fail(s: &mut MCSession, msg: impl std::fmt::Display) -> i32 {
    s.last_error = CString::new(msg.to_string()).unwrap_or_default();
    -1
}

// ---------------------------------------------------------------- lifecycle

#[no_mangle]
pub extern "C" fn mcs_new() -> *mut MCSession {
    Box::into_raw(Box::new(MCSession {
        session: Session::new(Project::default()),
        editor: EditorState::new(FrameRate::FILM),
        last_error: CString::default(),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn mcs_free(s: *mut MCSession) {
    if !s.is_null() { drop(Box::from_raw(s)); }
}

/// Human-readable text for the last failure (§23). Valid until the next call.
#[no_mangle]
pub unsafe extern "C" fn mcs_last_error(s: *const MCSession) -> *const c_char {
    if s.is_null() { return ptr::null(); }
    (*s).last_error.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn mcs_open(s: *mut MCSession, path: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(p) = cstr(path) else { return fail(s, "No file was given.") };
    match Project::load(std::path::Path::new(&p)) {
        Ok(project) => {
            s.session = Session::new(project);
            s.editor = EditorState::new(s.session.project.frame_rate);
            ok(s)
        }
        Err(e) => fail(s, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_save(s: *mut MCSession, path: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(p) = cstr(path) else { return fail(s, "No file was given.") };
    match s.session.save(std::path::Path::new(&p)) {
        Ok(()) => ok(s),
        Err(e) => fail(s, e),
    }
}

/// Turn on crash-safe autosave for a project path (§24).
///
/// Each applied edit is appended to a journal; a full save happens
/// periodically. Recovery replays the journal onto the last full save. This is
/// why autosave never blocks the UI — appending a few hundred bytes is cheap,
/// rewriting a large document per edit is not.
#[no_mangle]
pub unsafe extern "C" fn mcs_enable_autosave(s: *mut MCSession, path: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(p) = cstr(path) else { return fail(s, "No file was given.") };
    let project = std::mem::take(&mut s.session.project);
    let mut session = Session::new(project).with_autosave(&p);
    std::mem::swap(&mut session.history, &mut s.session.history);
    s.session = session;
    ok(s)
}

/// Is there a journal from a session that did not exit cleanly?
#[no_mangle]
pub unsafe extern "C" fn mcs_has_unrecovered_work(path: *const c_char) -> i32 {
    let Some(p) = cstr(path) else { return 0 };
    mediacore_model::project::Autosave::new(&p).has_unrecovered_work() as i32
}

/// Replay a journal onto the last full save. Returns edits recovered, or -1.
///
/// A journal can end mid-write after a crash, so a torn trailing line is
/// expected and is skipped rather than aborting recovery (§24, §42).
#[no_mangle]
pub unsafe extern "C" fn mcs_recover(s: *mut MCSession, path: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(p) = cstr(path) else { return fail(s, "No file was given.") };
    match mediacore_model::project::Autosave::new(&p).recover() {
        Ok((project, applied, _skipped)) => {
            s.session = Session::new(project).with_autosave(&p);
            s.editor = EditorState::new(s.session.project.frame_rate);
            applied as i32
        }
        Err(e) => fail(s, e),
    }
}

/// Discard recovery data and keep the last full save.
#[no_mangle]
pub unsafe extern "C" fn mcs_discard_recovery(path: *const c_char) -> i32 {
    let Some(p) = cstr(path) else { return -1 };
    let journal = std::path::Path::new(&p).with_extension("journal");
    let _ = std::fs::remove_file(journal);
    0
}

#[no_mangle]
pub unsafe extern "C" fn mcs_is_dirty(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.is_dirty() as i32).unwrap_or(0)
}

fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() { return None; }
    unsafe { CStr::from_ptr(p) }.to_str().ok().map(|s| s.to_owned())
}

// -------------------------------------------------------------------- reads

#[no_mangle]
pub unsafe extern "C" fn mcs_duration(s: *const MCSession) -> i64 {
    s.as_ref().map(|s| s.session.project.duration().0).unwrap_or(0)
}

/// The project's frame rate as a rational. Export must use this rather than
/// assuming 30 — a 24 fps project exported at 30 changes every clip's duration.
#[no_mangle]
pub unsafe extern "C" fn mcs_frame_rate(s: *const MCSession, num: *mut i32, den: *mut i32) {
    let Some(s) = s.as_ref() else { return };
    if num.is_null() || den.is_null() { return }
    let r = s.session.project.frame_rate;
    *num = r.num as i32;
    *den = r.den as i32;
}

/// Set the project frame rate. Conforming clips to it is the timeline's job.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_frame_rate(s: *mut MCSession, num: i32, den: i32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    if num <= 0 || den <= 0 { return fail(s, "That frame rate is not valid.") }
    s.session.project.frame_rate =
        mediacore_model::time::FrameRate::new(num as i64, den as i64);
    s.editor = EditorState::new(s.session.project.frame_rate);
    ok(s)
}

#[no_mangle]
pub unsafe extern "C" fn mcs_track_count(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.project.timeline.tracks.len() as i32).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn mcs_track_id_at(s: *const MCSession, index: i32) -> u64 {
    s.as_ref()
        .and_then(|s| s.session.project.timeline.tracks.get(index.max(0) as usize))
        .map(|t| t.id.0).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn mcs_clip_count(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.project.timeline.tracks.iter()
        .map(|t| t.clips.len()).sum::<usize>() as i32).unwrap_or(0)
}

/// Fill `out` with up to `cap` clips. Returns how many were written.
///
/// ONE call per frame. Bulk reads are the whole point of this surface.
#[no_mangle]
pub unsafe extern "C" fn mcs_clips(s: *const MCSession, out: *mut MCClipView, cap: i32) -> i32 {
    let (Some(s), false) = (s.as_ref(), out.is_null()) else { return 0 };
    if cap <= 0 { return 0; }
    let mut n = 0usize;
    let cap = cap as usize;
    for (ti, track) in s.session.project.timeline.tracks.iter().enumerate() {
        for c in &track.clips {
            if n >= cap { return n as i32; }
            *out.add(n) = MCClipView {
                clip_id: c.id.0, track_id: track.id.0, asset_id: c.asset.0,
                track_index: ti as i32,
                track_kind: match track.kind {
                    mediacore_model::timeline::TrackKind::Video => 0,
                    mediacore_model::timeline::TrackKind::Overlay => 1,
                    mediacore_model::timeline::TrackKind::Audio => 2,
                },
                start_ticks: c.timeline_start.0,
                duration_ticks: c.timeline_duration().0,
                source_start_ticks: c.source.start.0,
                gain: c.gain, speed: c.speed,
                selected: s.editor.selection.contains(c.id) as u8,
                muted: track.muted as u8, locked: track.locked as u8, _pad: 0,
            };
            n += 1;
        }
    }
    n as i32
}

/// Render plan at a time (AD-4). Also one bulk call.
#[no_mangle]
pub unsafe extern "C" fn mcs_plan_at(s: *const MCSession, t: i64,
                                     out: *mut MCPlanLayer, cap: i32) -> i32 {
    let (Some(s), false) = (s.as_ref(), out.is_null()) else { return 0 };
    if cap <= 0 { return 0; }
    let plan = render::plan_at(&s.session.project.timeline, Ticks(t));
    let mut n = 0usize;
    let cap = cap as usize;
    for l in &plan.video {
        if n >= cap { return n as i32; }
        let transfer = s.session.project.asset(l.asset)
            .and_then(|a| a.video.as_ref())
            .map(|v| match v.colour.transfer {
                mediacore_model::asset::TransferFunction::Rec709 => 0,
                mediacore_model::asset::TransferFunction::Pq => 1,
                mediacore_model::asset::TransferFunction::Hlg => 2,
            }).unwrap_or(0);
        let e = l.effects;
        let g = l.geometry;
        *out.add(n) = MCPlanLayer {
            clip_id: l.clip.0, asset_id: l.asset.0, source_time_ticks: l.source_time.0,
            opacity: l.opacity, gain: 1.0, speed: 1.0, kind: 0,
            is_overlay: l.is_overlay as i32,
            brightness: e.brightness, contrast: e.contrast, saturation: e.saturation,
            temperature: e.temperature, tint: e.tint,
            grayscale: e.grayscale as i32, blur: e.blur, sharpen: e.sharpen,
            transfer,
            is_text: l.is_text as i32,
            is_image: l.is_image as i32,
            crop_x: g.crop_x, crop_y: g.crop_y, crop_w: g.crop_w, crop_h: g.crop_h,
            rotation: g.rotation as i32,
            flip_h: g.flip_h as i32, flip_v: g.flip_v as i32,
            lut_amount: l.lut_amount,
            is_caption: l.is_caption as i32,
        };
        n += 1;
    }
    for l in &plan.audio {
        if n >= cap { return n as i32; }
        *out.add(n) = MCPlanLayer {
            clip_id: l.clip.0, asset_id: l.asset.0, source_time_ticks: l.source_time.0,
            opacity: 1.0, gain: l.gain, speed: l.speed, kind: 1, is_overlay: 0,
            ..Default::default()
        };
        n += 1;
    }
    n as i32
}

// -------------------------------------------------------------------- edits

#[no_mangle]
pub unsafe extern "C" fn mcs_add_clip(s: *mut MCSession, track: u64, asset: u64,
                                      start: i64, src_start: i64, src_dur: i64) -> u64 {
    let Some(s) = s.as_mut() else { return 0 };
    let id = s.session.project.new_clip_id();
    let clip = Clip::new(id, mediacore_model::asset::AssetId(asset), Ticks(start),
                         TimeRange::new(Ticks(src_start), Ticks(src_dur)));
    match s.session.apply(Edit::AddClip { track: TrackId(track), clip }) {
        Ok(()) => id.0,
        Err(e) => { fail(s, e); 0 }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_split_at(s: *mut MCSession, track: u64, at: i64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let new_id = s.session.project.new_clip_id();
    match ops::split_at(&s.session.project.timeline, TrackId(track), Ticks(at), new_id) {
        Ok(e) => match s.session.apply(e) { Ok(()) => ok(s), Err(e) => fail(s, e) },
        Err(e) => fail(s, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_ripple_delete(s: *mut MCSession, track: u64, clip: u64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    match ops::ripple_delete(&s.session.project.timeline, TrackId(track), ClipId(clip)) {
        Ok(e) => match s.session.apply(e) {
            Ok(()) => { s.editor.reconcile(&s.session.project.timeline); ok(s) }
            Err(e) => fail(s, e),
        },
        Err(e) => fail(s, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_delete_selected(s: *mut MCSession) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let ids = s.editor.selection.as_vec();
    if ids.is_empty() { return fail(s, "Nothing is selected."); }
    let mut edits = Vec::new();
    for id in ids {
        if let Some((track, c)) = s.session.project.timeline.find_clip(id) {
            edits.push(Edit::RemoveClip { track, clip: c.clone() });
        }
    }
    match s.session.apply(Edit::Batch(edits)) {
        Ok(()) => { s.editor.reconcile(&s.session.project.timeline); ok(s) }
        Err(e) => fail(s, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_move_clip(s: *mut MCSession, clip: u64,
                                       to_track: u64, to_start: i64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((from_track, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from_start = c.timeline_start;
    match s.session.apply(Edit::MoveClip {
        clip: ClipId(clip), from_track, to_track: TrackId(to_track),
        from_start, to_start: Ticks(to_start),
    }) { Ok(()) => ok(s), Err(e) => fail(s, e) }
}

/// Add an image overlay on the overlay track (§20).
///
/// PNG transparency must work, so the loader keeps the alpha channel.
#[no_mangle]
pub unsafe extern "C" fn mcs_add_image(s: *mut MCSession, track: u64,
                                       start: i64, duration: i64,
                                       path: *const c_char) -> u64 {
    let Some(s) = s.as_mut() else { return 0 };
    let Some(p) = cstr(path) else { return 0 };

    // Existence is checked here; DECODING is the platform's job.
    //
    // FFmpeg's still-image demuxers are awkward for single files (a valid PNG
    // reports "unspecified size"), and every target platform has a first-class
    // still decoder — ImageIO on macOS, WIC on Windows. Those handle PNG alpha,
    // JPEG, WebP and HEIC correctly. FFmpeg stays for video.
    if !std::path::Path::new(&p).exists() {
        fail(s, "That image file could not be found.");
        return 0;
    }

    let id = s.session.project.new_clip_id();
    let mut clip = Clip::new(id, mediacore_model::asset::AssetId(0), Ticks(start),
                             TimeRange::new(Ticks::ZERO, Ticks(duration)));
    clip.image = Some(mediacore_model::timeline::ImageSpec {
        path: p, ..Default::default()
    });
    match s.session.apply(Edit::AddClip { track: TrackId(track), clip }) {
        Ok(()) => id.0,
        Err(e) => { fail(s, e); 0 }
    }
}

/// The image spec for a clip as JSON, or empty.
#[no_mangle]
pub unsafe extern "C" fn mcs_clip_image(s: *mut MCSession, clip: u64) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return ptr::null() };
    let json = c.image.as_ref()
        .and_then(|i| serde_json::to_string(i).ok())
        .unwrap_or_default();
    s.last_error = CString::new(json).unwrap_or_default();
    s.last_error.as_ptr()
}

/// Replace a clip's image spec from JSON. Undoable like any other edit.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_clip_image(s: *mut MCSession, clip: u64,
                                            json: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(j) = cstr(json) else { return fail(s, "No image settings were given.") };
    let Ok(spec) = serde_json::from_str::<mediacore_model::timeline::ImageSpec>(&j)
        else { return fail(s, "Those image settings could not be read.") };
    let Some((track, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let old = c.clone();
    let mut updated = c.clone();
    updated.image = Some(spec);
    match s.session.apply(Edit::Batch(vec![
        Edit::RemoveClip { track, clip: old },
        Edit::AddClip { track, clip: updated },
    ])) { Ok(()) => ok(s), Err(e) => fail(s, e) }
}

/// The LUT spec for a clip as JSON, or empty.
#[no_mangle]
pub unsafe extern "C" fn mcs_clip_lut(s: *mut MCSession, clip: u64) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return ptr::null() };
    let json = c.lut.as_ref()
        .and_then(|l| serde_json::to_string(l).ok())
        .unwrap_or_default();
    s.last_error = CString::new(json).unwrap_or_default();
    s.last_error.as_ptr()
}

/// Apply a colour lookup table to a clip, or remove it with an empty path.
/// Undoable like any other edit.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_clip_lut(s: *mut MCSession, clip: u64,
                                          path: *const c_char, amount: f32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let p = cstr(path).unwrap_or_default();
    let Some((track, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let old = c.clone();
    let mut updated = c.clone();
    updated.lut = if p.is_empty() { None } else {
        Some(mediacore_model::timeline::LutSpec { path: p, amount: amount.clamp(0.0, 1.0) })
    };
    match s.session.apply(Edit::Batch(vec![
        Edit::RemoveClip { track, clip: old },
        Edit::AddClip { track, clip: updated },
    ])) { Ok(()) => ok(s), Err(e) => fail(s, e) }
}

/// Add a text overlay on the overlay track (§19).
///
/// `preset` indexes `TextPreset::all()`. Presets are finished LOOKS, not fonts:
/// padding, weight and contrast are already solved so the default ships
/// untouched (PRODUCT_DIRECTION §7).
#[no_mangle]
pub unsafe extern "C" fn mcs_add_text(s: *mut MCSession, track: u64,
                                      start: i64, duration: i64,
                                      preset: i32, text: *const c_char) -> u64 {
    let Some(s) = s.as_mut() else { return 0 };
    let body = cstr(text).unwrap_or_else(|| "Text".to_string());
    let presets = mediacore_model::timeline::TextPreset::all();
    let p = presets[(preset.max(0) as usize).min(presets.len() - 1)];

    let id = s.session.project.new_clip_id();
    let mut clip = Clip::new(id, mediacore_model::asset::AssetId(0), Ticks(start),
                             TimeRange::new(Ticks::ZERO, Ticks(duration)));
    clip.text = Some(p.spec(body));
    match s.session.apply(Edit::AddClip { track: TrackId(track), clip }) {
        Ok(()) => id.0,
        Err(e) => { fail(s, e); 0 }
    }
}

/// The text spec for a clip as JSON, or empty. Text changes rarely, so JSON is
/// fine here — unlike the per-frame reads, which are flat arrays.
#[no_mangle]
pub unsafe extern "C" fn mcs_clip_text(s: *mut MCSession, clip: u64) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return ptr::null() };
    let json = c.text.as_ref()
        .and_then(|t| serde_json::to_string(t).ok())
        .unwrap_or_default();
    s.last_error = CString::new(json).unwrap_or_default();
    s.last_error.as_ptr()
}

/// Replace a clip's text spec from JSON. Undoable like any other edit.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_clip_text(s: *mut MCSession, clip: u64,
                                           json: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(j) = cstr(json) else { return fail(s, "No text was given.") };
    let Ok(spec) = serde_json::from_str::<mediacore_model::timeline::TextSpec>(&j)
        else { return fail(s, "That text could not be read.") };
    let Some((track, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let mut updated = c.clone();
    updated.text = Some(spec);
    // Replace as remove+add so it goes through the undo stack unchanged.
    let old = c.clone();
    match s.session.apply(Edit::Batch(vec![
        Edit::RemoveClip { track, clip: old },
        Edit::AddClip { track, clip: updated },
    ])) { Ok(()) => ok(s), Err(e) => fail(s, e) }
}

/// Number of built-in text presets.
#[no_mangle]
pub extern "C" fn mcs_text_preset_count() -> i32 {
    mediacore_model::timeline::TextPreset::all().len() as i32
}

/// Name of a text preset.
#[no_mangle]
pub unsafe extern "C" fn mcs_text_preset_name(s: *mut MCSession, index: i32) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let presets = mediacore_model::timeline::TextPreset::all();
    let name = presets.get(index.max(0) as usize).map(|p| p.name()).unwrap_or("");
    s.last_error = CString::new(name).unwrap_or_default();
    s.last_error.as_ptr()
}

/// Set §18 effects on a clip. Undoable like any other edit.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_effects(s: *mut MCSession, clip: u64,
                                         brightness: f32, contrast: f32, saturation: f32,
                                         temperature: f32, tint: f32,
                                         grayscale: i32, blur: f32, sharpen: f32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from = c.effects;
    let to = mediacore_model::timeline::Effects {
        brightness, contrast, saturation, temperature, tint,
        grayscale: grayscale != 0, blur, sharpen,
    };
    match s.session.apply(Edit::SetEffects { clip: ClipId(clip), from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Set crop, rotation and flip on a clip. Undoable like any other edit.
///
/// The crop is in normalised source coordinates (0..1) rather than pixels, so
/// the value keeps its meaning if the clip is relinked to a different
/// resolution or exported at a size other than the preview's.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_geometry(s: *mut MCSession, clip: u64,
                                          crop_x: f32, crop_y: f32,
                                          crop_w: f32, crop_h: f32,
                                          rotation: i32, flip_h: i32, flip_v: i32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from = c.geometry;
    let to = mediacore_model::timeline::Geometry {
        crop_x, crop_y, crop_w, crop_h,
        rotation: rotation.rem_euclid(4) as u8,
        flip_h: flip_h != 0, flip_v: flip_v != 0,
    };
    match s.session.apply(Edit::SetGeometry { clip: ClipId(clip), from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Current geometry for a clip, into a 7-float array
/// (crop x, y, w, h, rotation, flip h, flip v).
#[no_mangle]
pub unsafe extern "C" fn mcs_get_geometry(s: *const MCSession, clip: u64,
                                          out: *mut f32, cap: i32) -> i32 {
    let (Some(s), false) = (s.as_ref(), out.is_null()) else { return 0 };
    if cap < 7 { return 0; }
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip)) else { return 0 };
    let g = c.geometry;
    let vals = [g.crop_x, g.crop_y, g.crop_w, g.crop_h, g.rotation as f32,
                if g.flip_h { 1.0 } else { 0.0 }, if g.flip_v { 1.0 } else { 0.0 }];
    for (i, v) in vals.iter().enumerate() { *out.add(i) = *v; }
    7
}

/// Duck a music clip under a voice clip (§3, PRODUCT_DIRECTION.md §6).
///
/// Ducking is a *changing* gain by definition, so it writes a volume curve
/// rather than one number. The curve is derived from where the voice actually
/// speaks — the complement of the silence the analysis finds — so it follows
/// the performance rather than a fixed pattern.
///
/// `duck_db` is how far the music steps back (a negative number; -12 dB is a
/// good default and leaves the music clearly audible). `ramp_ms` is how long it
/// takes to get there and back — long enough not to pump, short enough not to
/// swallow the first word.
///
/// Returns the number of ducked passages, or -1.
#[no_mangle]
pub unsafe extern "C" fn mcs_duck(s: *mut MCSession, music: u64, voice: u64,
                                  duck_db: f64, ramp_ms: i32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, mclip)) = s.session.project.timeline.find_clip(ClipId(music)) else {
        fail(s, "That music clip no longer exists.");
        return -1;
    };
    let Some((_, vclip)) = s.session.project.timeline.find_clip(ClipId(voice)) else {
        fail(s, "That voice clip no longer exists.");
        return -1;
    };
    let Some(path) = s.session.project.asset(vclip.asset).map(|a| a.path.clone()) else {
        fail(s, "The voice clip's file could not be found.");
        return -1;
    };
    let Ok(cpath) = CString::new(path) else {
        fail(s, "The voice clip's file could not be read.");
        return -1;
    };

    let music_range = mclip.timeline_range();
    let voice_range = vclip.timeline_range();
    let voice_start = vclip.timeline_start;
    let voice_source_start = vclip.source.start;
    let voice_speed = vclip.speed;
    let music_start = mclip.timeline_start;
    let music_len = music_range.duration;
    let from_points = mclip.gain_points.clone();

    // Silence in the voice recording, in nanoseconds of SOURCE time.
    let mut buf = vec![0i64; 4096];
    let n = crate::analysis::mc_silence_ranges(
        cpath.as_ptr(), -40.0, 400, 80, buf.as_mut_ptr(), buf.len() as i32);
    if n < 0 {
        fail(s, "The voice clip could not be analysed.");
        return -1;
    }

    // Silence -> speech, on the TIMELINE, clipped to where the voice clip
    // actually sits. The complement is what we want: ducking follows speech.
    let ns_to_ticks = |ns: i64| Ticks((ns as i128 * mediacore_model::time::TICKS_PER_SECOND as i128
                                      / 1_000_000_000) as i64);
    let to_timeline = |src: Ticks| -> Ticks {
        voice_start + Ticks(((src.0 - voice_source_start.0) as f64 / voice_speed) as i64)
    };
    let mut speech: Vec<(Ticks, Ticks)> = Vec::new();
    let mut cursor = voice_range.start;
    for i in 0..n as usize {
        let s0 = to_timeline(ns_to_ticks(buf[i * 2]) + voice_source_start);
        let s1 = to_timeline(ns_to_ticks(buf[i * 2 + 1]) + voice_source_start);
        if s0 > cursor { speech.push((cursor, s0.min(voice_range.end()))); }
        cursor = cursor.max(s1);
    }
    if cursor < voice_range.end() { speech.push((cursor, voice_range.end())); }

    // Only the parts that overlap the music are worth a curve.
    let ramp = Ticks((ramp_ms.max(10) as i64 * mediacore_model::time::TICKS_PER_SECOND) / 1000);
    let level = 10f64.powf(duck_db.min(0.0) / 20.0) as f32;
    let mut points: Vec<mediacore_model::timeline::GainPoint> = Vec::new();
    let mut passages = 0i32;
    for (a, b) in speech {
        let a = a.max(music_range.start);
        let b = b.min(music_range.end());
        if b <= a { continue; }
        passages += 1;
        // Positions are relative to the MUSIC clip's start.
        let rel = |t: Ticks| Ticks((t.0 - music_start.0).clamp(0, music_len.0));
        let push = |v: &mut Vec<mediacore_model::timeline::GainPoint>, at: Ticks, gain: f32| {
            v.push(mediacore_model::timeline::GainPoint { at, gain });
        };
        push(&mut points, rel(a - ramp), 1.0);
        push(&mut points, rel(a), level);
        push(&mut points, rel(b), level);
        push(&mut points, rel(b + ramp), 1.0);
    }
    if passages == 0 {
        fail(s, "The voice does not overlap the music, so there is nothing to duck.");
        return -1;
    }

    match s.session.apply(Edit::SetGainPoints {
        clip: ClipId(music), from: from_points, to: points,
    }) {
        Ok(()) => { ok(s); passages }
        Err(e) => { fail(s, e); -1 }
    }
}

/// Remove a clip's volume curve, leaving its plain level.
#[no_mangle]
pub unsafe extern "C" fn mcs_clear_gain_points(s: *mut MCSession, clip: u64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from = c.gain_points.clone();
    if from.is_empty() { return ok(s); }
    match s.session.apply(Edit::SetGainPoints { clip: ClipId(clip), from, to: Vec::new() }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// How many points are on a clip's volume curve. 0 means it has none.
#[no_mangle]
pub unsafe extern "C" fn mcs_gain_point_count(s: *const MCSession, clip: u64) -> i32 {
    let Some(s) = s.as_ref() else { return 0 };
    s.session.project.timeline.find_clip(ClipId(clip))
        .map(|(_, c)| c.gain_points.len() as i32).unwrap_or(0)
}

/// A text preset's spec as JSON, without creating a clip.
///
/// Burned-in captions need the LOOK of a preset applied to words that change
/// every few seconds. Going through `mcs_add_text` would mean creating and
/// destroying a clip per caption, which is absurd; this hands over the spec so
/// captions reuse the presets exactly as titles do.
#[no_mangle]
pub unsafe extern "C" fn mcs_text_preset_spec(s: *mut MCSession, preset: i32,
                                              text: *const c_char) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let words = cstr(text).unwrap_or_default();
    let presets = mediacore_model::timeline::TextPreset::all();
    let p = presets[(preset.max(0) as usize).min(presets.len() - 1)];
    let json = serde_json::to_string(&p.spec(words)).unwrap_or_default();
    s.last_error = CString::new(json).unwrap_or_default();
    s.last_error.as_ptr()
}

// ===========================================================================
// Captions (§8)
// ===========================================================================

/// How many captions the project has.
#[no_mangle]
pub unsafe extern "C" fn mcs_caption_count(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.project.timeline.captions.len() as i32).unwrap_or(0)
}

/// One caption, as JSON: `{"start":ticks,"end":ticks,"text":"..."}`.
#[no_mangle]
pub unsafe extern "C" fn mcs_caption_at_index(s: *mut MCSession, index: i32) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let json = s.session.project.timeline.captions.as_slice().get(index.max(0) as usize)
        .and_then(|c| serde_json::to_string(c).ok())
        .unwrap_or_default();
    s.last_error = CString::new(json).unwrap_or_default();
    s.last_error.as_ptr()
}

/// The caption showing at an instant, or empty. Used by the compositor when a
/// plan layer is marked `is_caption`.
#[no_mangle]
pub unsafe extern "C" fn mcs_caption_at(s: *mut MCSession, t: i64) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let text = s.session.project.timeline.captions.at(Ticks(t))
        .map(|c| c.text.clone()).unwrap_or_default();
    s.last_error = CString::new(text).unwrap_or_default();
    s.last_error.as_ptr()
}

/// Replace every caption from a JSON array. Undoable.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_captions(s: *mut MCSession, json: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(j) = cstr(json) else { return fail(s, "No captions were given.") };
    let Ok(items) = serde_json::from_str::<Vec<mediacore_model::captions::Caption>>(&j)
        else { return fail(s, "Those captions could not be read.") };
    let from = s.session.project.timeline.captions.clone();
    let mut to = from.clone();
    to.set(items);
    match s.session.apply(Edit::SetCaptions { from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Import an SRT or WebVTT file. Returns the number of captions read, or -1.
#[no_mangle]
pub unsafe extern "C" fn mcs_import_subtitles(s: *mut MCSession, path: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(p) = cstr(path) else { return fail(s, "No file was given.") };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return fail(s, "That subtitle file could not be read.");
    };
    let items = mediacore_model::captions::Captions::parse_subtitles(&text);
    if items.is_empty() {
        return fail(s, "No captions could be read from that file. It may not be \
                        an SRT or WebVTT file.");
    }
    let n = items.len() as i32;
    let from = s.session.project.timeline.captions.clone();
    let mut to = from.clone();
    to.set(items);
    match s.session.apply(Edit::SetCaptions { from, to }) {
        Ok(()) => n, Err(e) => fail(s, e),
    }
}

/// Write the captions as SRT or WebVTT, chosen by the file's extension.
#[no_mangle]
pub unsafe extern "C" fn mcs_export_subtitles(s: *mut MCSession, path: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(p) = cstr(path) else { return fail(s, "No file was given.") };
    if s.session.project.timeline.captions.is_empty() {
        return fail(s, "There are no captions to export.");
    }
    let text = if p.to_ascii_lowercase().ends_with(".vtt") {
        s.session.project.timeline.captions.to_vtt()
    } else {
        s.session.project.timeline.captions.to_srt()
    };
    match std::fs::write(&p, text) {
        Ok(()) => ok(s),
        Err(_) => fail(s, "The subtitle file could not be written. Check the folder \
                           and free space."),
    }
}

/// Edit one caption's words. Undoable.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_caption_text(s: *mut MCSession, index: i32,
                                              text: *const c_char) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some(t) = cstr(text) else { return fail(s, "No text was given.") };
    let from = s.session.project.timeline.captions.clone();
    let mut to = from.clone();
    if !to.set_text(index.max(0) as usize, t) {
        return fail(s, "That caption no longer exists.");
    }
    match s.session.apply(Edit::SetCaptions { from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Remove one caption. Undoable.
#[no_mangle]
pub unsafe extern "C" fn mcs_remove_caption(s: *mut MCSession, index: i32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let from = s.session.project.timeline.captions.clone();
    let mut to = from.clone();
    if !to.remove(index.max(0) as usize) {
        return fail(s, "That caption no longer exists.");
    }
    match s.session.apply(Edit::SetCaptions { from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Shift every caption in time — the fix for a transcript that is uniformly
/// early or late. Undoable.
#[no_mangle]
pub unsafe extern "C" fn mcs_shift_captions(s: *mut MCSession, by: i64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let from = s.session.project.timeline.captions.clone();
    let mut to = from.clone();
    to.shift(Ticks(by));
    match s.session.apply(Edit::SetCaptions { from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Burn captions into the exported picture, or not. Undoable, because it
/// changes what an export looks like.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_caption_burn_in(s: *mut MCSession, on: i32,
                                                 preset: i32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let from = s.session.project.timeline.captions.clone();
    let mut to = from.clone();
    to.burn_in = on != 0;
    to.preset = preset.clamp(0, 3) as u8;
    match s.session.apply(Edit::SetCaptions { from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// 1 when captions are burned into the picture.
#[no_mangle]
pub unsafe extern "C" fn mcs_caption_burn_in(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.project.timeline.captions.burn_in as i32).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn mcs_caption_preset(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.project.timeline.captions.preset as i32).unwrap_or(0)
}

/// Ripple-delete a span of the timeline (PRODUCT_DIRECTION.md §7's primary
/// gesture, and what silence removal is built from).
///
/// Everything inside the span goes and everything after it moves back, so the
/// gap closes rather than becoming a hole.
#[no_mangle]
pub unsafe extern "C" fn mcs_ripple_delete_range(s: *mut MCSession, track: u64,
                                                 start: i64, end: i64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    if end <= start { return fail(s, "That is not a range of time."); }
    let range = TimeRange::new(Ticks(start), Ticks(end - start));
    let new_id = ClipId(s.session.project.new_id());
    match mediacore_model::ops::ripple_delete_range(
              &s.session.project.timeline, TrackId(track), range, new_id) {
        Ok(edit) => match s.session.apply(edit) {
            Ok(()) => { s.editor.reconcile(&s.session.project.timeline); ok(s) }
            Err(e) => fail(s, e),
        },
        Err(e) => fail(s, e),
    }
}

/// Clip volume, 0.0–4.0 (0 dB = 1.0). Undoable like any other edit.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_gain(s: *mut MCSession, clip: u64, gain: f64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from = c.gain;
    let to = gain.clamp(0.0, 4.0);
    match s.session.apply(Edit::SetGain { clip: ClipId(clip), from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Fade in and out, in ticks. Undoable like any other edit.
#[no_mangle]
pub unsafe extern "C" fn mcs_set_fades(s: *mut MCSession, clip: u64,
                                       fade_in: i64, fade_out: i64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from = (c.fade_in, c.fade_out);
    // A fade cannot be longer than the clip, and the two together cannot
    // exceed it either — the render would produce a negative multiplier.
    let len = c.timeline_range().duration.0;
    let mut fi = fade_in.clamp(0, len);
    let mut fo = fade_out.clamp(0, len);
    if fi + fo > len {
        let scale = len as f64 / (fi + fo) as f64;
        fi = (fi as f64 * scale) as i64;
        fo = (fo as f64 * scale) as i64;
    }
    let to = (Ticks(fi), Ticks(fo));
    match s.session.apply(Edit::SetFades { clip: ClipId(clip), from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Read a clip's volume and fades: [gain, fade_in_ticks, fade_out_ticks].
#[no_mangle]
pub unsafe extern "C" fn mcs_get_levels(s: *const MCSession, clip: u64,
                                        out: *mut f64, cap: i32) -> i32 {
    let (Some(s), false) = (s.as_ref(), out.is_null()) else { return 0 };
    if cap < 3 { return 0; }
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip)) else { return 0 };
    *out.add(0) = c.gain;
    *out.add(1) = c.fade_in.0 as f64;
    *out.add(2) = c.fade_out.0 as f64;
    3
}

/// Move a clip's sound onto its own audio track (§3's "detach audio").
///
/// The picture clip stays where it is and is silenced; a new clip covering the
/// same span appears on the first audio track with room. Both halves are ONE
/// undoable edit, because "detach" is one action to the person doing it —
/// undoing it should not leave a silenced video behind.
///
/// Returns the new clip's id, or 0.
#[no_mangle]
pub unsafe extern "C" fn mcs_detach_audio(s: *mut MCSession, clip: u64) -> u64 {
    let Some(s) = s.as_mut() else { return 0 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip)) else {
        fail(s, "That clip no longer exists.");
        return 0;
    };
    if c.gain <= 0.0 {
        fail(s, "That clip's sound has already been detached or silenced.");
        return 0;
    }
    let source = c.source;
    let start = c.timeline_start;
    let asset = c.asset;
    let gain = c.gain;
    let speed = c.speed;
    let range = c.timeline_range();

    let Some(track) = s.session.project.timeline.tracks.iter()
        .find(|t| t.kind == mediacore_model::timeline::TrackKind::Audio
                  && !t.locked
                  && t.clips.iter().all(|o| !o.timeline_range().overlaps(range)))
        .map(|t| t.id)
    else {
        fail(s, "There is no free audio track for this clip's sound.");
        return 0;
    };

    let new_id = ClipId(s.session.project.new_id());
    let mut new_clip = mediacore_model::timeline::Clip::new(new_id, asset, start, source);
    new_clip.gain = gain;
    new_clip.speed = speed;

    match s.session.apply(Edit::Batch(vec![
        Edit::AddClip { track, clip: new_clip },
        Edit::SetGain { clip: ClipId(clip), from: gain, to: 0.0 },
    ])) {
        Ok(()) => { ok(s); new_id.0 }
        Err(e) => { fail(s, e); 0 }
    }
}

/// Trim a clip to a target length and fade it out at the end (§3's
/// "fit to length").
///
/// The operation music needs: a three-minute track under a ninety-second video
/// should stop cleanly rather than being cut off mid-bar. Trimming with a fade
/// is what an editor does by hand, and speeding the music up instead would
/// change its pitch and tempo.
#[no_mangle]
pub unsafe extern "C" fn mcs_fit_to_length(s: *mut MCSession, clip: u64,
                                           target: i64, fade: i64) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    if target <= 0 { return fail(s, "That length is too short to fit anything into."); }
    let current = c.timeline_range().duration.0;
    if current <= target {
        return fail(s, "This clip is already shorter than the video.");
    }
    let from_source = c.source;
    let start = c.timeline_start;
    // Trimming from the END keeps the start where the user put it.
    let keep = (target as f64 * c.speed) as i64;
    let to_source = TimeRange::new(from_source.start, Ticks(keep.max(1)));
    let fade_out = Ticks(fade.clamp(0, target / 2));
    let from_fades = (c.fade_in, c.fade_out);

    match s.session.apply(Edit::Batch(vec![
        Edit::TrimClip { clip: ClipId(clip),
                         from_source, to_source,
                         from_start: start, to_start: start },
        Edit::SetFades { clip: ClipId(clip), from: from_fades,
                         to: (from_fades.0, fade_out) },
    ])) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// The visible aspect ratio of a clip as it will be DRAWN — source shape with
/// its crop and rotation applied. 0 when the clip has no picture.
///
/// The preview's overlays need this: a safe-area guide drawn on the letterbox
/// instead of on the picture is worse than no guide.
#[no_mangle]
pub unsafe extern "C" fn mcs_clip_aspect(s: *const MCSession, clip: u64) -> f32 {
    let Some(s) = s.as_ref() else { return 0.0 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip)) else { return 0.0 };
    let Some(v) = s.session.project.asset(c.asset).and_then(|a| a.video.as_ref())
        else { return 0.0 };
    let source = v.width as f64 / v.height.max(1) as f64;
    c.geometry.aspect(source) as f32
}

/// Reframe a clip to an aspect ratio by taking the largest centred crop that
/// fits — "make this landscape shot vertical", the operation itself.
///
/// The source's own aspect comes from the asset rather than from the caller, so
/// the UI does not have to know the footage's shape to ask for this.
#[no_mangle]
pub unsafe extern "C" fn mcs_reframe(s: *mut MCSession, clip: u64,
                                     target_aspect: f32) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip))
        else { return fail(s, "That clip no longer exists.") };
    let from = c.geometry;
    let asset = c.asset;
    let Some(source_aspect) = s.session.project.asset(asset)
        .and_then(|a| a.video.as_ref())
        .map(|v| v.width as f64 / v.height.max(1) as f64)
        else { return fail(s, "This clip has no picture to reframe.") };
    let to = mediacore_model::timeline::Geometry::fill_aspect(
        source_aspect, target_aspect as f64);
    match s.session.apply(Edit::SetGeometry { clip: ClipId(clip), from, to }) {
        Ok(()) => ok(s), Err(e) => fail(s, e),
    }
}

/// Current effects for a clip, into a 8-float array + grayscale flag.
#[no_mangle]
pub unsafe extern "C" fn mcs_get_effects(s: *const MCSession, clip: u64,
                                         out: *mut f32, cap: i32) -> i32 {
    let (Some(s), false) = (s.as_ref(), out.is_null()) else { return 0 };
    if cap < 8 { return 0; }
    let Some((_, c)) = s.session.project.timeline.find_clip(ClipId(clip)) else { return 0 };
    let e = c.effects;
    let vals = [e.brightness, e.contrast, e.saturation, e.temperature, e.tint,
                if e.grayscale { 1.0 } else { 0.0 }, e.blur, e.sharpen];
    for (i, v) in vals.iter().enumerate() { *out.add(i) = *v; }
    8
}

#[no_mangle]
pub unsafe extern "C" fn mcs_undo(s: *mut MCSession) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    match s.session.undo() {
        Ok(Some(_)) => { s.editor.reconcile(&s.session.project.timeline); ok(s) }
        Ok(None) => fail(s, "There is nothing to undo."),
        Err(e) => fail(s, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_redo(s: *mut MCSession) -> i32 {
    let Some(s) = s.as_mut() else { return -1 };
    match s.session.redo() {
        Ok(Some(_)) => { s.editor.reconcile(&s.session.project.timeline); ok(s) }
        Ok(None) => fail(s, "There is nothing to redo."),
        Err(e) => fail(s, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_can_undo(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.history.can_undo() as i32).unwrap_or(0)
}
#[no_mangle]
pub unsafe extern "C" fn mcs_can_redo(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.history.can_redo() as i32).unwrap_or(0)
}

// ------------------------------------------------------- playhead/selection

#[no_mangle]
pub unsafe extern "C" fn mcs_playhead(s: *const MCSession) -> i64 {
    s.as_ref().map(|s| s.editor.playhead.position().0).unwrap_or(0)
}
#[no_mangle]
pub unsafe extern "C" fn mcs_set_playhead(s: *mut MCSession, t: i64) {
    if let Some(s) = s.as_mut() { s.editor.playhead.set(Ticks(t)); }
}
#[no_mangle]
pub unsafe extern "C" fn mcs_step_frames(s: *mut MCSession, frames: i64) {
    if let Some(s) = s.as_mut() {
        let rate = s.session.project.frame_rate;
        s.editor.playhead.step_frames(frames, rate);
    }
}
#[no_mangle]
pub unsafe extern "C" fn mcs_select_only(s: *mut MCSession, clip: u64) {
    if let Some(s) = s.as_mut() { s.editor.selection.select_only(ClipId(clip)); }
}
#[no_mangle]
pub unsafe extern "C" fn mcs_select_toggle(s: *mut MCSession, clip: u64) {
    if let Some(s) = s.as_mut() { s.editor.selection.toggle(ClipId(clip)); }
}
#[no_mangle]
pub unsafe extern "C" fn mcs_select_clear(s: *mut MCSession) {
    if let Some(s) = s.as_mut() { s.editor.selection.clear(); }
}
#[no_mangle]
pub unsafe extern "C" fn mcs_selection_count(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.editor.selection.len() as i32).unwrap_or(0)
}

/// Timecode string at the playhead. Caller must NOT free; valid until the next call.
#[no_mangle]
pub unsafe extern "C" fn mcs_timecode(s: *mut MCSession) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let tc = s.editor.playhead.timecode(s.session.project.frame_rate);
    s.last_error = CString::new(tc.to_string()).unwrap_or_default();
    s.last_error.as_ptr()
}

// ------------------------------------------------------------------- import

/// Probe a file and add it to the project. Returns the new asset id, or 0.
#[no_mangle]
pub unsafe extern "C" fn mcs_import(s: *mut MCSession, path: *const c_char) -> u64 {
    let Some(s) = s.as_mut() else { return 0 };
    let Some(p) = cstr(path) else { return 0 };

    let mut probe = crate::MCProbe::default();
    crate::mc_probe(path, 150, &mut probe);

    // NO_VIDEO_STREAM is not a failure — it is an audio file, and §3 makes
    // audio a first-class import. Only a genuinely unreadable file fails.
    let audio_only = probe.error_class == crate::MC_ERR_NO_VIDEO_STREAM;
    if probe.error_class != crate::MC_OK && !audio_only {
        let msg = CStr::from_ptr(crate::mc_class_message(probe.error_class))
            .to_string_lossy().into_owned();
        fail(s, msg);
        return 0;
    }

    let id = s.session.project.new_asset_id();
    let duration = duration_of(&p).unwrap_or(Ticks::ZERO);
    if duration.0 <= 0 {
        fail(s, "This file has no readable duration, so it cannot be placed on a timeline.");
        return 0;
    }

    let asset = if audio_only {
        // Audio carries no video info; the timeline places it on an audio track.
        mediacore_model::asset::Asset::new(id, &p, duration)
    } else {
        match crate::import::asset_from_probe(id, &p, duration, &probe) {
            Some(a) => a,
            None => { fail(s, "This file could not be imported."); return 0; }
        }
    };
    s.session.project.assets.push(asset);
    id.0
}

/// Stream duration, which `mc_probe` does not report (it describes what it
/// inspected, not the whole file).
fn duration_of(path: &str) -> Option<Ticks> {
    use rusty_ffmpeg::ffi;
    unsafe {
        let c = CString::new(path).ok()?;
        let mut fmt: *mut ffi::AVFormatContext = ptr::null_mut();
        if ffi::avformat_open_input(&mut fmt, c.as_ptr(), ptr::null(), ptr::null_mut()) < 0 {
            return None;
        }
        let d = if ffi::avformat_find_stream_info(fmt, ptr::null_mut()) >= 0
            && (*fmt).duration != ffi::AV_NOPTS_VALUE
        {
            Some(Ticks::from_seconds((*fmt).duration as f64 / ffi::AV_TIME_BASE as f64))
        } else { None };
        ffi::avformat_close_input(&mut fmt);
        d
    }
}

#[no_mangle]
pub unsafe extern "C" fn mcs_asset_count(s: *const MCSession) -> i32 {
    s.as_ref().map(|s| s.session.project.assets.len() as i32).unwrap_or(0)
}

/// Fill `out` with min/max peak pairs for an asset's audio.
///
/// Returns the number of PAIRS written. S7 measured 60 minutes of FLAC in
/// 3.18 s, so this is fast enough to call on a background thread at import and
/// cache; it is not something to call per frame.
#[no_mangle]
pub unsafe extern "C" fn mcs_waveform(s: *mut MCSession, asset: u64,
                                      buckets_per_second: i32,
                                      out: *mut f32, cap: i32) -> i32 {
    let (Some(s), false) = (s.as_mut(), out.is_null()) else { return 0 };
    if cap <= 0 { return 0; }
    let Some(a) = s.session.project.asset(mediacore_model::asset::AssetId(asset))
        else { return 0 };
    let Ok(path) = CString::new(a.path.clone()) else { return 0 };

    struct Sink { out: *mut f32, cap: usize, n: usize }
    let mut sink = Sink { out, cap: cap as usize, n: 0 };

    unsafe extern "C" fn collect(user: *mut std::ffi::c_void, peaks: *const f32,
                                 pairs: i32, _progress: f32) -> i32 {
        let sink = &mut *(user as *mut Sink);
        for i in 0..(pairs as usize * 2) {
            if sink.n >= sink.cap * 2 { return 0; }   // full: stop early
            *sink.out.add(sink.n) = *peaks.add(i);
            sink.n += 1;
        }
        1
    }

    let mut duration = 0.0f64;
    crate::mc_waveform(path.as_ptr(), buckets_per_second, collect,
                       &mut sink as *mut Sink as *mut std::ffi::c_void, &mut duration);
    (sink.n / 2) as i32
}

/// Whether an asset has a video track. Audio-only files belong on an audio
/// track, not the video track.
#[no_mangle]
pub unsafe extern "C" fn mcs_asset_has_video(s: *const MCSession, asset: u64) -> i32 {
    s.as_ref()
        .and_then(|s| s.session.project.asset(mediacore_model::asset::AssetId(asset)))
        .map(|a| a.video.is_some() as i32).unwrap_or(0)
}

/// Import notices for an asset — HDR tone-mapping, variable frame rate — as
/// one newline-separated string, empty when there is nothing to say.
///
/// O-3 requires an HDR→SDR conversion to be DISCLOSED rather than silent;
/// silent conversion is what produces "why does my video look washed out".
/// Ordinary SDR video returns nothing, because it must not nag.
#[no_mangle]
pub unsafe extern "C" fn mcs_asset_notices(s: *mut MCSession, asset: u64) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let Some(a) = s.session.project.asset(mediacore_model::asset::AssetId(asset))
        else { return ptr::null() };
    let text = crate::import::notices_for(a).iter()
        .map(|n| n.message()).collect::<Vec<_>>().join("\n");
    s.last_error = CString::new(text).unwrap_or_default();
    s.last_error.as_ptr()
}

/// File path of an asset. Valid until the next call; the caller must not free.
#[no_mangle]
pub unsafe extern "C" fn mcs_asset_path(s: *mut MCSession, asset: u64) -> *const c_char {
    let Some(s) = s.as_mut() else { return ptr::null() };
    let Some(a) = s.session.project.asset(mediacore_model::asset::AssetId(asset))
        else { return ptr::null() };
    s.last_error = CString::new(a.path.clone()).unwrap_or_default();
    s.last_error.as_ptr()
}

/// Duration of an asset, or 0.
#[no_mangle]
pub unsafe extern "C" fn mcs_asset_duration(s: *const MCSession, asset: u64) -> i64 {
    s.as_ref()
        .and_then(|s| s.session.project.asset(mediacore_model::asset::AssetId(asset)))
        .map(|a: &Asset| a.duration.0).unwrap_or(0)
}
