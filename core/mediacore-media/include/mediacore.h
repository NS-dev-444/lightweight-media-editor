#ifndef MEDIACORE_H
#define MEDIACORE_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define MC_OK 0

#define MC_ERR_FILE_UNREADABLE 1

#define MC_ERR_NOT_MEDIA 2

#define MC_ERR_NO_VIDEO_STREAM 3

#define MC_ERR_UNSUPPORTED_CODEC 4

#define MC_ERR_CORRUPT_HEADER 5

#define MC_ERR_TRUNCATED 6

#define MC_ERR_DECODE_FAILED 7

// Codec selection. Values match `MCExportCodec` in the header.
#define MC_CODEC_HEVC 0

#define MC_CODEC_H264 1

#define MC_CONVERT_REMUX 0

#define MC_CONVERT_TRANSCODE 1

#define MC_AUDIO_MP3 0

#define MC_AUDIO_AAC 1

#define MC_AUDIO_WAV 2

#define MC_AUDIO_FLAC 3

#define MC_AUDIO_ALAC 4

#define MC_MERGE_COPY 0

#define MC_MERGE_ENCODE 1

typedef struct MCAudio MCAudio;

typedef struct MCAudioConvert MCAudioConvert;

typedef struct MCConvert MCConvert;

typedef struct MCDecoder MCDecoder;

typedef struct MCEncoder MCEncoder;

typedef struct MCMerge MCMerge;

typedef struct MCSession MCSession;

// Fields the non-whisper build does not read: this type still exists there so
// the C ABI is identical in both builds and the app needs no conditional code.
typedef struct MCTranscribe MCTranscribe;

typedef struct MCVideoConvert MCVideoConvert;

typedef struct MCInfo {
    int32_t width;
    int32_t height;
    double fps;
    double duration_sec;
    // 1 when frames arrive on the VideoToolbox hardware path (no CPU copy).
    int32_t hw_accelerated;
    int32_t pix_fmt;
} MCInfo;

typedef struct MCProbe {
    int32_t error_class;
    int32_t width;
    int32_t height;
    int32_t frames_decoded;
    int32_t hw_accelerated;
    // The raw AVERROR, preserved for the "technical details" disclosure (§23).
    int32_t av_error;
    int32_t color_primaries;
    int32_t color_trc;
    int32_t color_space;
    int32_t bits_per_raw_sample;
    // 1 when the transfer function is PQ (SMPTE 2084) or HLG — i.e. HDR.
    int32_t is_hdr;
    int32_t avg_frame_rate_num;
    int32_t avg_frame_rate_den;
    int32_t r_frame_rate_num;
    int32_t r_frame_rate_den;
    int32_t time_base_num;
    int32_t time_base_den;
    // 1 when the stream is likely variable frame rate.
    // Set from metadata AND from observed packet timestamps — metadata alone
    // misses mixed-rate files whose headers were not recomputed.
    int32_t is_vfr;
    // Distinct frame durations observed while probing (1 = perfectly regular).
    int32_t distinct_frame_durations;
} MCProbe;

// Called with a batch of (min,max) pairs. Return 0 to cancel.
typedef int32_t (*MCPeakCallback)(void *user, const float *peaks, int32_t pair_count, float progress);

// One clip, flattened for drawing. Matches what the S2 timeline renderer needs.
typedef struct MCClipView {
    uint64_t clip_id;
    uint64_t track_id;
    uint64_t asset_id;
    int32_t track_index;
    // 0 = video, 1 = overlay, 2 = audio.
    int32_t track_kind;
    int64_t start_ticks;
    int64_t duration_ticks;
    int64_t source_start_ticks;
    double gain;
    double speed;
    uint8_t selected;
    uint8_t muted;
    uint8_t locked;
    uint8_t _pad;
} MCClipView;

// One layer of a render plan (AD-4: what to draw, not how).
typedef struct MCPlanLayer {
    uint64_t clip_id;
    uint64_t asset_id;
    int64_t source_time_ticks;
    double opacity;
    double gain;
    double speed;
    // 0 = video, 1 = audio.
    int32_t kind;
    int32_t is_overlay;
    float brightness;
    float contrast;
    float saturation;
    float temperature;
    float tint;
    int32_t grayscale;
    float blur;
    float sharpen;
    // 0 = Rec.709, 1 = PQ, 2 = HLG. O-3: HDR must be tone-mapped to SDR
    // deliberately, and the compositor needs to know which curve to undo.
    int32_t transfer;
    // 1 when this layer draws text rather than media; fetch the spec with
    // `mcs_clip_text`.
    int32_t is_text;
    // 1 when this layer draws an image overlay; fetch it with `mcs_clip_image`.
    int32_t is_image;
    float crop_x;
    float crop_y;
    float crop_w;
    float crop_h;
    // Quarter turns clockwise, 0-3.
    int32_t rotation;
    int32_t flip_h;
    int32_t flip_v;
    // How much of the clip's colour lookup table to apply, 0 = none. The
    // table itself is fetched by path with `mcs_clip_lut`; it is megabytes of
    // data and has no place in a per-frame bulk read.
    float lut_amount;
    // 1 when this layer draws a CAPTION rather than a clip's own text. The
    // compositor asks `mcs_caption_at` for the words.
    int32_t is_caption;
} MCPlanLayer;

// Decoded RGBA image. Free with `mc_image_free`.
typedef struct MCImage {
    int32_t width;
    int32_t height;
    // Tightly packed RGBA8, `width * height * 4` bytes.
    uint8_t *rgba;
    int64_t bytes;
} MCImage;













#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

// Silence FFmpeg's own stderr logging.
//
// §23 requires errors the user can understand. FFmpeg writes things like
// "moov atom not found" straight to stderr, bypassing our error
// classification entirely — visible in test output the first time a corrupt
// file was probed. The host app should call this at startup and rely on
// `mc_probe`'s error classes instead. Pass 0 to restore FFmpeg's default,
// which is useful when diagnosing a decode problem.
void mc_set_log_quiet(int32_t quiet);

struct MCDecoder *mc_open(const char *path);

int32_t mc_info(const struct MCDecoder *d, struct MCInfo *out);

int64_t mc_sw_frame_count(const struct MCDecoder *d);

// DATA PLANE.
//
// Returns 1 on a frame, 0 at end of stream, negative on error.
// `out_pb` receives a **retained** `CVPixelBufferRef`; the caller must `CFRelease` it.
// No pixel data crosses this boundary — only the buffer handle.
int32_t mc_next_frame(struct MCDecoder *d, const void **out_pb, int64_t *out_pts_ns);

// Seek to (at or before) a time in nanoseconds and flush the decoder.
//
// Seeks land on the nearest preceding KEYFRAME, not the exact time — that is
// how inter-frame codecs work. The caller decodes forward from there to reach
// the requested frame. Returning the actual landing point lets the caller know
// how far it must still decode.
//
// Returns 0 on success, negative on failure.
int32_t mc_seek(struct MCDecoder *d, int64_t ns);

void mc_close(struct MCDecoder *d);

// Human-readable message for an error class. Never returns a raw codec error.
const char *mc_class_message(int32_t class_);

// Open a file, inspect it, and attempt to decode up to `max_frames`.
// This is the call that runs INSIDE THE WORKER PROCESS, where a codec bug
// parsing hostile input can only take down the worker (AD-3, R-15).
int32_t mc_probe(const char *path, int32_t max_frames, struct MCProbe *out);

int32_t mc_waveform(const char *path,
                    int32_t buckets_per_second,
                    MCPeakCallback cb,
                    void *user,
                    double *out_duration);

struct MCSession *mcs_new(void);

void mcs_free(struct MCSession *s);

// Human-readable text for the last failure (§23). Valid until the next call.
const char *mcs_last_error(const struct MCSession *s);

int32_t mcs_open(struct MCSession *s, const char *path);

int32_t mcs_save(struct MCSession *s, const char *path);

// Turn on crash-safe autosave for a project path (§24).
//
// Each applied edit is appended to a journal; a full save happens
// periodically. Recovery replays the journal onto the last full save. This is
// why autosave never blocks the UI — appending a few hundred bytes is cheap,
// rewriting a large document per edit is not.
int32_t mcs_enable_autosave(struct MCSession *s, const char *path);

// Is there a journal from a session that did not exit cleanly?
int32_t mcs_has_unrecovered_work(const char *path);

// Replay a journal onto the last full save. Returns edits recovered, or -1.
//
// A journal can end mid-write after a crash, so a torn trailing line is
// expected and is skipped rather than aborting recovery (§24, §42).
int32_t mcs_recover(struct MCSession *s, const char *path);

// Discard recovery data and keep the last full save.
int32_t mcs_discard_recovery(const char *path);

int32_t mcs_is_dirty(const struct MCSession *s);

int64_t mcs_duration(const struct MCSession *s);

// The project's frame rate as a rational. Export must use this rather than
// assuming 30 — a 24 fps project exported at 30 changes every clip's duration.
void mcs_frame_rate(const struct MCSession *s, int32_t *num, int32_t *den);

// Set the project frame rate. Conforming clips to it is the timeline's job.
int32_t mcs_set_frame_rate(struct MCSession *s, int32_t num, int32_t den);

int32_t mcs_track_count(const struct MCSession *s);

uint64_t mcs_track_id_at(const struct MCSession *s, int32_t index);

int32_t mcs_clip_count(const struct MCSession *s);

// Fill `out` with up to `cap` clips. Returns how many were written.
//
// ONE call per frame. Bulk reads are the whole point of this surface.
int32_t mcs_clips(const struct MCSession *s, struct MCClipView *out, int32_t cap);

// Render plan at a time (AD-4). Also one bulk call.
int32_t mcs_plan_at(const struct MCSession *s, int64_t t, struct MCPlanLayer *out, int32_t cap);

uint64_t mcs_add_clip(struct MCSession *s,
                      uint64_t track,
                      uint64_t asset,
                      int64_t start,
                      int64_t src_start,
                      int64_t src_dur);

int32_t mcs_split_at(struct MCSession *s, uint64_t track, int64_t at);

int32_t mcs_ripple_delete(struct MCSession *s, uint64_t track, uint64_t clip);

int32_t mcs_delete_selected(struct MCSession *s);

int32_t mcs_move_clip(struct MCSession *s, uint64_t clip, uint64_t to_track, int64_t to_start);

// Add an image overlay on the overlay track (§20).
//
// PNG transparency must work, so the loader keeps the alpha channel.
uint64_t mcs_add_image(struct MCSession *s,
                       uint64_t track,
                       int64_t start,
                       int64_t duration,
                       const char *path);

// The image spec for a clip as JSON, or empty.
const char *mcs_clip_image(struct MCSession *s, uint64_t clip);

// Replace a clip's image spec from JSON. Undoable like any other edit.
int32_t mcs_set_clip_image(struct MCSession *s, uint64_t clip, const char *json);

// The LUT spec for a clip as JSON, or empty.
const char *mcs_clip_lut(struct MCSession *s, uint64_t clip);

// Apply a colour lookup table to a clip, or remove it with an empty path.
// Undoable like any other edit.
int32_t mcs_set_clip_lut(struct MCSession *s, uint64_t clip, const char *path, float amount);

// Add a text overlay on the overlay track (§19).
//
// `preset` indexes `TextPreset::all()`. Presets are finished LOOKS, not fonts:
// padding, weight and contrast are already solved so the default ships
// untouched (PRODUCT_DIRECTION §7).
uint64_t mcs_add_text(struct MCSession *s,
                      uint64_t track,
                      int64_t start,
                      int64_t duration,
                      int32_t preset,
                      const char *text);

// The text spec for a clip as JSON, or empty. Text changes rarely, so JSON is
// fine here — unlike the per-frame reads, which are flat arrays.
const char *mcs_clip_text(struct MCSession *s, uint64_t clip);

// Replace a clip's text spec from JSON. Undoable like any other edit.
int32_t mcs_set_clip_text(struct MCSession *s, uint64_t clip, const char *json);

// Number of built-in text presets.
int32_t mcs_text_preset_count(void);

// Name of a text preset.
const char *mcs_text_preset_name(struct MCSession *s, int32_t index);

// Set §18 effects on a clip. Undoable like any other edit.
int32_t mcs_set_effects(struct MCSession *s,
                        uint64_t clip,
                        float brightness,
                        float contrast,
                        float saturation,
                        float temperature,
                        float tint,
                        int32_t grayscale,
                        float blur,
                        float sharpen);

// Set crop, rotation and flip on a clip. Undoable like any other edit.
//
// The crop is in normalised source coordinates (0..1) rather than pixels, so
// the value keeps its meaning if the clip is relinked to a different
// resolution or exported at a size other than the preview's.
int32_t mcs_set_geometry(struct MCSession *s,
                         uint64_t clip,
                         float crop_x,
                         float crop_y,
                         float crop_w,
                         float crop_h,
                         int32_t rotation,
                         int32_t flip_h,
                         int32_t flip_v);

// Current geometry for a clip, into a 7-float array
// (crop x, y, w, h, rotation, flip h, flip v).
int32_t mcs_get_geometry(const struct MCSession *s, uint64_t clip, float *out, int32_t cap);

// Duck a music clip under a voice clip (§3, PRODUCT_DIRECTION.md §6).
//
// Ducking is a *changing* gain by definition, so it writes a volume curve
// rather than one number. The curve is derived from where the voice actually
// speaks — the complement of the silence the analysis finds — so it follows
// the performance rather than a fixed pattern.
//
// `duck_db` is how far the music steps back (a negative number; -12 dB is a
// good default and leaves the music clearly audible). `ramp_ms` is how long it
// takes to get there and back — long enough not to pump, short enough not to
// swallow the first word.
//
// Returns the number of ducked passages, or -1.
int32_t mcs_duck(struct MCSession *s,
                 uint64_t music,
                 uint64_t voice,
                 double duck_db,
                 int32_t ramp_ms);

// Remove a clip's volume curve, leaving its plain level.
int32_t mcs_clear_gain_points(struct MCSession *s, uint64_t clip);

// How many points are on a clip's volume curve. 0 means it has none.
int32_t mcs_gain_point_count(const struct MCSession *s, uint64_t clip);

// A text preset's spec as JSON, without creating a clip.
//
// Burned-in captions need the LOOK of a preset applied to words that change
// every few seconds. Going through `mcs_add_text` would mean creating and
// destroying a clip per caption, which is absurd; this hands over the spec so
// captions reuse the presets exactly as titles do.
const char *mcs_text_preset_spec(struct MCSession *s, int32_t preset, const char *text);

// How many captions the project has.
int32_t mcs_caption_count(const struct MCSession *s);

// One caption, as JSON: `{"start":ticks,"end":ticks,"text":"..."}`.
const char *mcs_caption_at_index(struct MCSession *s, int32_t index);

// The caption showing at an instant, or empty. Used by the compositor when a
// plan layer is marked `is_caption`.
const char *mcs_caption_at(struct MCSession *s, int64_t t);

// Replace every caption from a JSON array. Undoable.
int32_t mcs_set_captions(struct MCSession *s, const char *json);

// Import an SRT or WebVTT file. Returns the number of captions read, or -1.
int32_t mcs_import_subtitles(struct MCSession *s, const char *path);

// Write the captions as SRT or WebVTT, chosen by the file's extension.
int32_t mcs_export_subtitles(struct MCSession *s, const char *path);

// Edit one caption's words. Undoable.
int32_t mcs_set_caption_text(struct MCSession *s, int32_t index, const char *text);

// Remove one caption. Undoable.
int32_t mcs_remove_caption(struct MCSession *s, int32_t index);

// Shift every caption in time — the fix for a transcript that is uniformly
// early or late. Undoable.
int32_t mcs_shift_captions(struct MCSession *s, int64_t by);

// Burn captions into the exported picture, or not. Undoable, because it
// changes what an export looks like.
int32_t mcs_set_caption_burn_in(struct MCSession *s, int32_t on, int32_t preset);

// 1 when captions are burned into the picture.
int32_t mcs_caption_burn_in(const struct MCSession *s);

int32_t mcs_caption_preset(const struct MCSession *s);

// Ripple-delete a span of the timeline (PRODUCT_DIRECTION.md §7's primary
// gesture, and what silence removal is built from).
//
// Everything inside the span goes and everything after it moves back, so the
// gap closes rather than becoming a hole.
int32_t mcs_ripple_delete_range(struct MCSession *s, uint64_t track, int64_t start, int64_t end);

// Clip volume, 0.0–4.0 (0 dB = 1.0). Undoable like any other edit.
int32_t mcs_set_gain(struct MCSession *s, uint64_t clip, double gain);

// Fade in and out, in ticks. Undoable like any other edit.
int32_t mcs_set_fades(struct MCSession *s, uint64_t clip, int64_t fade_in, int64_t fade_out);

// Read a clip's volume and fades: [gain, fade_in_ticks, fade_out_ticks].
int32_t mcs_get_levels(const struct MCSession *s, uint64_t clip, double *out, int32_t cap);

// Move a clip's sound onto its own audio track (§3's "detach audio").
//
// The picture clip stays where it is and is silenced; a new clip covering the
// same span appears on the first audio track with room. Both halves are ONE
// undoable edit, because "detach" is one action to the person doing it —
// undoing it should not leave a silenced video behind.
//
// Returns the new clip's id, or 0.
uint64_t mcs_detach_audio(struct MCSession *s, uint64_t clip);

// Trim a clip to a target length and fade it out at the end (§3's
// "fit to length").
//
// The operation music needs: a three-minute track under a ninety-second video
// should stop cleanly rather than being cut off mid-bar. Trimming with a fade
// is what an editor does by hand, and speeding the music up instead would
// change its pitch and tempo.
int32_t mcs_fit_to_length(struct MCSession *s, uint64_t clip, int64_t target, int64_t fade);

// The visible aspect ratio of a clip as it will be DRAWN — source shape with
// its crop and rotation applied. 0 when the clip has no picture.
//
// The preview's overlays need this: a safe-area guide drawn on the letterbox
// instead of on the picture is worse than no guide.
float mcs_clip_aspect(const struct MCSession *s, uint64_t clip);

// Reframe a clip to an aspect ratio by taking the largest centred crop that
// fits — "make this landscape shot vertical", the operation itself.
//
// The source's own aspect comes from the asset rather than from the caller, so
// the UI does not have to know the footage's shape to ask for this.
int32_t mcs_reframe(struct MCSession *s, uint64_t clip, float target_aspect);

// Current effects for a clip, into a 8-float array + grayscale flag.
int32_t mcs_get_effects(const struct MCSession *s, uint64_t clip, float *out, int32_t cap);

int32_t mcs_undo(struct MCSession *s);

int32_t mcs_redo(struct MCSession *s);

int32_t mcs_can_undo(const struct MCSession *s);

int32_t mcs_can_redo(const struct MCSession *s);

int64_t mcs_playhead(const struct MCSession *s);

void mcs_set_playhead(struct MCSession *s, int64_t t);

void mcs_step_frames(struct MCSession *s, int64_t frames);

void mcs_select_only(struct MCSession *s, uint64_t clip);

void mcs_select_toggle(struct MCSession *s, uint64_t clip);

void mcs_select_clear(struct MCSession *s);

int32_t mcs_selection_count(const struct MCSession *s);

// Timecode string at the playhead. Caller must NOT free; valid until the next call.
const char *mcs_timecode(struct MCSession *s);

// Probe a file and add it to the project. Returns the new asset id, or 0.
uint64_t mcs_import(struct MCSession *s, const char *path);

int32_t mcs_asset_count(const struct MCSession *s);

// Fill `out` with min/max peak pairs for an asset's audio.
//
// Returns the number of PAIRS written. S7 measured 60 minutes of FLAC in
// 3.18 s, so this is fast enough to call on a background thread at import and
// cache; it is not something to call per frame.
int32_t mcs_waveform(struct MCSession *s,
                     uint64_t asset,
                     int32_t buckets_per_second,
                     float *out,
                     int32_t cap);

// Whether an asset has a video track. Audio-only files belong on an audio
// track, not the video track.
int32_t mcs_asset_has_video(const struct MCSession *s, uint64_t asset);

// Import notices for an asset — HDR tone-mapping, variable frame rate — as
// one newline-separated string, empty when there is nothing to say.
//
// O-3 requires an HDR→SDR conversion to be DISCLOSED rather than silent;
// silent conversion is what produces "why does my video look washed out".
// Ordinary SDR video returns nothing, because it must not nag.
const char *mcs_asset_notices(struct MCSession *s, uint64_t asset);

// File path of an asset. Valid until the next call; the caller must not free.
const char *mcs_asset_path(struct MCSession *s, uint64_t asset);

// Duration of an asset, or 0.
int64_t mcs_asset_duration(const struct MCSession *s, uint64_t asset);

// Open an encoder writing to `path`.
//
// Returns null on failure; the reason is available from `mc_encoder_error`.
struct MCEncoder *mc_encoder_open(const char *path,
                                  int32_t width,
                                  int32_t height,
                                  int32_t fps_num,
                                  int32_t fps_den,
                                  int32_t codec_kind,
                                  int32_t bitrate_kbps,
                                  int32_t audio_sample_rate);

// True when the encoder is taking hardware surfaces (zero-copy).
int32_t mc_encoder_is_hardware(const struct MCEncoder *e);

// Borrow a writable surface from the ENCODER'S pool.
//
// The compositor renders into this and then calls `mc_encoder_submit`.
//
// This is the API shape the hardware path requires, and it was arrived at by
// measurement rather than assumption: pushing a `CVPixelBuffer` from a
// caller-owned pool is rejected outright (`avcodec_send_frame` returns an
// error), because FFmpeg's hardware frames context can only encode frames it
// allocated. Rendering into ITS buffer keeps the path genuinely zero-copy.
//
// The returned buffer is owned by the encoder — do not release it.
int32_t mc_encoder_acquire(struct MCEncoder *e, void **out_pixel_buffer);

// Encode the surface most recently returned by `mc_encoder_acquire`.
int32_t mc_encoder_submit(struct MCEncoder *e);

// Push interleaved stereo f32 audio.
//
// The AAC encoder needs fixed-size frames, so samples are buffered until a
// full frame is available. Feeding partial frames produces gaps and drift.
int32_t mc_encoder_push_audio(struct MCEncoder *e, const float *samples, int32_t frame_count);

// Does this encoder have an audio stream?
int32_t mc_encoder_has_audio(const struct MCEncoder *e);

// Flush, write the trailer, and close. The encoder is consumed.
int32_t mc_encoder_finish(struct MCEncoder *e);

// Frames written so far, for progress reporting.
int64_t mc_encoder_frames(const struct MCEncoder *e);

// Is a codec available on this machine? AD-6: probe by ACTUALLY looking, never
// by assuming a GPU or a codec exists.
int32_t mc_encoder_available(int32_t codec_kind);

// Load a still image as RGBA.
//
// Returns null if the file is not a readable image. §22: an unreadable file
// must be a refusal, never a crash.
struct MCImage *mc_image_load(const char *path);

void mc_image_free(struct MCImage *img);

// Open an audio stream, resampled to `sample_rate` stereo f32.
//
// Returns null when the file has no audio — which is not an error, just a
// video without sound.
struct MCAudio *mc_audio_open(const char *path, int32_t sample_rate);

// Seek to a position in nanoseconds.
int32_t mc_audio_seek(struct MCAudio *a, int64_t ns);

// Read up to `frames` stereo sample-frames into `out` (interleaved f32).
//
// Returns the number of sample-frames written; 0 means end of stream.
int32_t mc_audio_read(struct MCAudio *a, float *out, int32_t frames);

void mc_audio_close(struct MCAudio *a);

// Does this file contain audio? Cheaper than opening a decoder.
int32_t mc_has_audio(const char *path);

// Open a conversion. `allow_remux == 0` forces a transcode.
struct MCConvert *mc_convert_open(const char *input, const char *output, int32_t allow_remux);

// Process one packet. 1 = more work, 0 = finished, negative = error.
//
// Stepwise so the caller owns the loop: progress, cancellation and pausing
// belong to whoever is driving, and no context is shared across threads.
int32_t mc_convert_step(struct MCConvert *c);

// 0.0–1.0. Falls back to 0 when the source reports no duration.
float mc_convert_progress(const struct MCConvert *c);

int32_t mc_convert_mode(const struct MCConvert *c);

const char *mc_convert_error(const struct MCConvert *c);

// Finish and close. Writes the trailer, so a cancelled job still leaves a
// playable (if short) file rather than a corrupt one.
int32_t mc_convert_close(struct MCConvert *c);

// Would this conversion be a remux (fast, lossless) or need re-encoding?
//
// Lets the UI tell the user which they are about to get — "this will be
// instant" versus "this will take a while" is worth knowing before starting.
int32_t mc_convert_would_remux(const char *input, const char *output);

// Open an audio conversion.
struct MCAudioConvert *mc_audio_convert_open(const char *input,
                                             const char *output,
                                             int32_t codec_kind,
                                             int32_t bitrate_kbps,
                                             int32_t sample_rate);

// Convert one chunk. 1 = more, 0 = done, negative = error.
int32_t mc_audio_convert_step(struct MCAudioConvert *c);

int32_t mc_audio_convert_close(struct MCAudioConvert *c);

// Sample-frames converted so far, for progress.
int64_t mc_audio_convert_frames(const struct MCAudioConvert *c);

// Open a video transcode.
//
// `width`/`height` of 0 keep the source size; giving one of them scales to fit
// while preserving the aspect ratio, which is what the resize tool wants.
// `bitrate_kbps` of 0 asks for the default above.
struct MCVideoConvert *mc_video_convert_open(const char *input,
                                             const char *output,
                                             int32_t codec_kind,
                                             int32_t bitrate_kbps,
                                             int32_t width,
                                             int32_t height);

// Process one packet. 1 = more work, 0 = finished, negative = error.
int32_t mc_video_convert_step(struct MCVideoConvert *c);

// 0.0–1.0. Falls back to 0 when the source reports no duration.
float mc_video_convert_progress(const struct MCVideoConvert *c);

int64_t mc_video_convert_frames(const struct MCVideoConvert *c);

const char *mc_video_convert_error(const struct MCVideoConvert *c);

// Finish and close. Writes the trailer, so a cancelled job leaves a playable
// (if short) file rather than a corrupt one.
int32_t mc_video_convert_close(struct MCVideoConvert *c);

// Open a merge over `count` input paths, in the order given.
//
// The order is the caller's: files are joined as listed, never re-sorted, so
// "part 1, part 2, part 3" means what it says.
struct MCMerge *mc_merge_open(const char *const *inputs, int32_t count, const char *output);

// Process one packet. 1 = more work, 0 = finished, negative = error.
int32_t mc_merge_step(struct MCMerge *m);

// 0 = streams were copied (instant, lossless), 1 = re-encoded.
int32_t mc_merge_mode(const struct MCMerge *m);

float mc_merge_progress(const struct MCMerge *m);

const char *mc_merge_error(const struct MCMerge *m);

// Would merging these files copy the streams, or re-encode them?
//
// Lets the UI promise "instant" honestly before anything starts.
int32_t mc_merge_would_copy(const char *const *inputs, int32_t count);

int32_t mc_merge_close(struct MCMerge *m);

// Integrated loudness in LUFS. -200 means "no measurable sound".
double mc_loudness(const char *path);

// Highest absolute sample value, 0.0–1.0+. Tells you whether a gain change
// will clip before you make it.
double mc_sample_peak(const char *path);

// Stretches of near-silence, as start/end pairs in NANOSECONDS.
//
// Written into `out` as consecutive start, end values; returns the number of
// RANGES found (so `2 * n` values were written), or -1 on failure.
//
// The parameters exist because silence is not one thing. `threshold_db` is
// what counts as quiet — room tone in a bedroom sits far higher than in a
// booth. `min_ms` stops every breath from becoming a cut. `pad_ms` leaves a
// little of the silence at each end, because a cut that lands exactly on the
// first syllable sounds clipped even when the timing is right.
int32_t mc_silence_ranges(const char *path,
                          double threshold_db,
                          int32_t min_ms,
                          int32_t pad_ms,
                          int64_t *out,
                          int32_t cap);

extern void *mcw_open(const char *model_path, int use_gpu);

extern void mcw_close(void *w);

extern int mcw_run(void *w,
                   const float *samples,
                   int n,
                   int threads,
                   const char *lang,
                   int translate);

extern int mcw_n_segments(void *w);

extern int64_t mcw_seg_t0(void *w, int i);

extern int64_t mcw_seg_t1(void *w, int i);

extern const char *mcw_seg_text(void *w, int i);

// Open a transcription of `input` using the model at `model_path`.
//
// `language` may be empty or "auto". `translate` asks for English output from
// non-English speech, which is a different feature from transcribing and is
// off unless asked for.
struct MCTranscribe *mc_transcribe_open(const char *input,
                                        const char *model_path,
                                        const char *language,
                                        int32_t translate);

// Transcribe one chunk. 1 = more work, 0 = finished, negative = failed.
int32_t mc_transcribe_step(struct MCTranscribe *t);

// Seconds of audio transcribed so far, for progress against the file's length.
double mc_transcribe_seconds_done(const struct MCTranscribe *t);

int32_t mc_transcribe_count(const struct MCTranscribe *t);

// Start of segment `i`, in nanoseconds.
int64_t mc_transcribe_start_ns(const struct MCTranscribe *t, int32_t i);

int64_t mc_transcribe_end_ns(const struct MCTranscribe *t, int32_t i);

const char *mc_transcribe_text(const struct MCTranscribe *t, int32_t i);

const char *mc_transcribe_error(const struct MCTranscribe *t);

int32_t mc_transcribe_close(struct MCTranscribe *t);

// Is on-device transcription available in this build?
//
// Reported rather than assumed, so the UI can say "this build has no
// transcription" instead of offering a button that silently does nothing.
int32_t mc_transcribe_available(void);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* MEDIACORE_H */
