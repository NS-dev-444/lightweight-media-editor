//! Measuring sound: loudness and silence.
//!
//! Both feed one-click operations that are otherwise tedious by hand —
//! "make this as loud as everything else" and "cut the gaps out of this take".
//!
//! ## Why loudness is implemented here rather than borrowed
//!
//! EBU R128 is a precisely specified algorithm, not a matter of taste: a
//! K-weighting filter, 400 ms blocks overlapping by 75 %, an absolute gate at
//! -70 LUFS and a relative gate 10 LU below the ungated mean. Writing it out is
//! about a hundred lines, it has no dependency question attached, and — the
//! deciding point — it can be checked against an independent implementation,
//! which is exactly what `tools/check_loudness.sh` does.
//!
//! Sample-peak is measured alongside it because loudness alone does not tell
//! you whether normalising will clip.

use std::ffi::{c_char, CStr};

/// R128 works on a fixed rate, and the K-weighting coefficients below are the
/// 48 kHz ones from the specification. Reading at 48 kHz means they are exact
/// rather than approximated.
const RATE: i32 = 48_000;

/// A biquad in direct form I.
#[derive(Clone, Copy)]
struct Biquad { b0: f64, b1: f64, b2: f64, a1: f64, a2: f64 }

#[derive(Clone, Copy, Default)]
struct BiquadState { x1: f64, x2: f64, y1: f64, y2: f64 }

impl Biquad {
    #[inline]
    fn step(&self, s: &mut BiquadState, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * s.x1 + self.b2 * s.x2
                - self.a1 * s.y1 - self.a2 * s.y2;
        s.x2 = s.x1; s.x1 = x;
        s.y2 = s.y1; s.y1 = y;
        y
    }
}

/// Stage 1: the shelving filter approximating the head's acoustic effect.
const SHELF: Biquad = Biquad {
    b0: 1.53512485958697, b1: -2.69169618940638, b2: 1.19839281085285,
    a1: -1.69065929318241, a2: 0.73248077421585,
};
/// Stage 2: the RLB high-pass.
const HIGHPASS: Biquad = Biquad {
    b0: 1.0, b1: -2.0, b2: 1.0,
    a1: -1.99004745483398, a2: 0.99007225036621,
};

/// Integrated loudness of a file, in LUFS, and its sample peak.
///
/// Returns `None` when the file has no readable audio. A silent file has no
/// meaningful loudness at all, and is reported as `f64::NEG_INFINITY` rather
/// than as some large negative number that looks like a measurement.
pub fn measure(path: &CStr) -> Option<(f64, f64)> {
    unsafe {
        let reader = crate::audio::mc_audio_open(path.as_ptr(), RATE);
        if reader.is_null() { return None; }

        let block = (RATE as usize * 4) / 10;      // 400 ms
        let hop = block / 4;                        // 75 % overlap
        let mut states = [BiquadState::default(); 4];   // shelf/hp per channel
        let mut ring = vec![0f64; block * 2];
        let mut filled = 0usize;
        let mut write = 0usize;
        let mut since_hop = 0usize;
        let mut blocks: Vec<f64> = Vec::new();
        let mut peak = 0f64;

        let chunk = 4096usize;
        let mut scratch = vec![0f32; chunk * 2];
        loop {
            let got = crate::audio::mc_audio_read(reader, scratch.as_mut_ptr(), chunk as i32);
            if got <= 0 { break; }
            for i in 0..got as usize {
                for ch in 0..2 {
                    let x = scratch[i * 2 + ch] as f64;
                    if x.abs() > peak { peak = x.abs(); }
                    let shelved = SHELF.step(&mut states[ch * 2], x);
                    let k = HIGHPASS.step(&mut states[ch * 2 + 1], shelved);
                    ring[write * 2 + ch] = k * k;
                }
                write = (write + 1) % block;
                if filled < block { filled += 1; }
                since_hop += 1;
                if filled == block && since_hop >= hop {
                    since_hop = 0;
                    let mut sum = 0f64;
                    for s in ring.iter() { sum += *s; }
                    let mean = sum / block as f64;   // both channels summed
                    blocks.push(-0.691 + 10.0 * (mean).log10());
                }
            }
        }
        crate::audio::mc_audio_close(reader);
        if blocks.is_empty() { return Some((f64::NEG_INFINITY, peak)); }

        // Absolute gate, then a relative gate 10 LU below what survives it.
        let gated = |threshold: f64, blocks: &[f64]| -> Option<f64> {
            let kept: Vec<f64> = blocks.iter().copied()
                .filter(|l| l.is_finite() && *l > threshold).collect();
            if kept.is_empty() { return None; }
            // Averaging must happen in the ENERGY domain, not in decibels:
            // the mean of two loudness figures is not the loudness of their sum.
            let energy: f64 = kept.iter().map(|l| 10f64.powf((l + 0.691) / 10.0)).sum();
            Some(-0.691 + 10.0 * (energy / kept.len() as f64).log10())
        };
        let Some(ungated) = gated(-70.0, &blocks) else {
            return Some((f64::NEG_INFINITY, peak));
        };
        let relative = ungated - 10.0;
        let integrated = gated(relative.max(-70.0), &blocks).unwrap_or(ungated);
        Some((integrated, peak))
    }
}

/// Integrated loudness in LUFS. -200 means "no measurable sound".
#[no_mangle]
pub unsafe extern "C" fn mc_loudness(path: *const c_char) -> f64 {
    if path.is_null() { return -200.0; }
    match measure(CStr::from_ptr(path)) {
        Some((l, _)) if l.is_finite() => l,
        _ => -200.0,
    }
}

/// Highest absolute sample value, 0.0–1.0+. Tells you whether a gain change
/// will clip before you make it.
#[no_mangle]
pub unsafe extern "C" fn mc_sample_peak(path: *const c_char) -> f64 {
    if path.is_null() { return 0.0; }
    measure(CStr::from_ptr(path)).map(|(_, p)| p).unwrap_or(0.0)
}

/// Stretches of near-silence, as start/end pairs in NANOSECONDS.
///
/// Written into `out` as consecutive start, end values; returns the number of
/// RANGES found (so `2 * n` values were written), or -1 on failure.
///
/// The parameters exist because silence is not one thing. `threshold_db` is
/// what counts as quiet — room tone in a bedroom sits far higher than in a
/// booth. `min_ms` stops every breath from becoming a cut. `pad_ms` leaves a
/// little of the silence at each end, because a cut that lands exactly on the
/// first syllable sounds clipped even when the timing is right.
#[no_mangle]
pub unsafe extern "C" fn mc_silence_ranges(path: *const c_char,
                                           threshold_db: f64,
                                           min_ms: i32,
                                           pad_ms: i32,
                                           out: *mut i64,
                                           cap: i32) -> i32 {
    if path.is_null() || out.is_null() || cap < 2 { return -1; }
    let reader = crate::audio::mc_audio_open(path, RATE);
    if reader.is_null() { return -1; }

    let threshold = 10f64.powf(threshold_db / 20.0);
    let min_samples = (min_ms.max(0) as i64 * RATE as i64) / 1000;
    let pad_samples = (pad_ms.max(0) as i64 * RATE as i64) / 1000;

    // A short window rather than per-sample: a waveform crosses zero constantly,
    // so a single quiet SAMPLE means nothing. 20 ms is short enough to place a
    // cut accurately and long enough to be about the sound rather than the wave.
    let window = (RATE as usize) / 50;
    let chunk = 4096usize;
    let mut scratch = vec![0f32; chunk * 2];

    let mut n = 0i32;
    let mut sample: i64 = 0;
    let mut quiet_from: Option<i64> = None;
    let mut acc = 0f64;
    let mut acc_n = 0usize;

    let emit = |from: i64, to: i64, n: &mut i32| {
        let from = (from + pad_samples).max(0);
        let to = to - pad_samples;
        if to - from < min_samples { return; }
        if *n as i64 * 2 + 1 >= cap as i64 { return; }
        let ns = |s: i64| s * 1_000_000_000 / RATE as i64;
        *out.add(*n as usize * 2) = ns(from);
        *out.add(*n as usize * 2 + 1) = ns(to);
        *n += 1;
    };

    loop {
        let got = crate::audio::mc_audio_read(reader, scratch.as_mut_ptr(), chunk as i32);
        if got <= 0 { break; }
        for i in 0..got as usize {
            let l = scratch[i * 2] as f64;
            let r = scratch[i * 2 + 1] as f64;
            acc += (l * l + r * r) / 2.0;
            acc_n += 1;
            sample += 1;
            if acc_n >= window {
                let rms = (acc / acc_n as f64).sqrt();
                acc = 0.0; acc_n = 0;
                let quiet = rms < threshold;
                match (quiet, quiet_from) {
                    (true, None) => quiet_from = Some(sample - window as i64),
                    (false, Some(from)) => { emit(from, sample - window as i64, &mut n);
                                             quiet_from = None; }
                    _ => {}
                }
            }
        }
    }
    // Trailing silence counts: a take usually ends with several seconds of it.
    if let Some(from) = quiet_from { emit(from, sample, &mut n); }
    crate::audio::mc_audio_close(reader);
    n
}
