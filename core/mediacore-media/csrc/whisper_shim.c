// A minimal, stable C surface over whisper.cpp.
//
// The alternative was declaring `whisper_full_params` in Rust by hand. That
// struct has thirty-odd fields, its layout changes between releases, and a
// mismatch would not fail to compile — it would silently feed the model the
// wrong parameters. RISK_REGISTER.md R-24 is the same failure mode in a shader,
// and it cost weeks of quietly wrong blur.
//
// Here the layout is the C compiler's problem, which is where it belongs, and
// this file is the only thing that has to be checked when the pin moves.

#include "whisper.h"
#include <stdlib.h>
#include <string.h>

struct mcw {
    struct whisper_context *ctx;
};

// Whisper logs its model architecture and timings to stderr. §23: the user's
// error surface is sentences, and FFmpeg's chatter was silenced for the same
// reason (mc_set_log_quiet).
static void mcw_silent(enum ggml_log_level level, const char *text, void *user) {
    (void)level; (void)text; (void)user;
}

struct mcw *mcw_open(const char *model_path, int use_gpu) {
    whisper_log_set(mcw_silent, NULL);
    struct whisper_context_params cp = whisper_context_default_params();
    cp.use_gpu = use_gpu != 0;
    struct whisper_context *ctx = whisper_init_from_file_with_params(model_path, cp);
    if (!ctx) return NULL;
    struct mcw *w = (struct mcw *)calloc(1, sizeof(struct mcw));
    if (!w) { whisper_free(ctx); return NULL; }
    w->ctx = ctx;
    return w;
}

void mcw_close(struct mcw *w) {
    if (!w) return;
    if (w->ctx) whisper_free(w->ctx);
    free(w);
}

// `lang` may be NULL or "auto" to let the model detect it.
int mcw_run(struct mcw *w, const float *samples, int n_samples,
            int threads, const char *lang, int translate) {
    if (!w || !w->ctx || !samples || n_samples <= 0) return -1;
    struct whisper_full_params p = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    p.print_progress   = false;
    p.print_realtime   = false;
    p.print_timestamps = false;
    p.print_special    = false;
    p.translate        = translate != 0;
    p.language         = (lang && *lang) ? lang : "auto";
    p.n_threads        = threads > 0 ? threads : 4;
    // Suppress the "[BLANK_AUDIO]"-style non-speech tokens: they are not words
    // anybody wants burned into their video.
    p.suppress_nst     = true;
    // Single segment off: we want per-utterance timings, which is the whole
    // point of generating captions rather than a transcript.
    p.single_segment   = false;
    p.no_timestamps    = false;
    return whisper_full(w->ctx, p, samples, n_samples);
}

int mcw_n_segments(struct mcw *w) {
    return (w && w->ctx) ? whisper_full_n_segments(w->ctx) : 0;
}

// Times are in whisper's centisecond units; the caller converts.
long long mcw_seg_t0(struct mcw *w, int i) {
    return (w && w->ctx) ? (long long)whisper_full_get_segment_t0(w->ctx, i) : 0;
}
long long mcw_seg_t1(struct mcw *w, int i) {
    return (w && w->ctx) ? (long long)whisper_full_get_segment_t1(w->ctx, i) : 0;
}
const char *mcw_seg_text(struct mcw *w, int i) {
    if (!w || !w->ctx) return "";
    const char *t = whisper_full_get_segment_text(w->ctx, i);
    return t ? t : "";
}
