//! Rational time.
//!
//! §R11 and the Phase 1 findings: the timeline is PTS-driven on a fixed
//! high-resolution integer timebase. Never float seconds, never frame indices.
//!
//! Phase 1 measured why this matters: variable-frame-rate sources are common
//! and are NOT reliably detectable (the metadata heuristic misses mixed-rate
//! files, and a shallow probe misses everything). Correctness therefore cannot
//! depend on detecting VFR — the model is PTS-driven unconditionally.

use serde::{Deserialize, Serialize};

/// Ticks per second.
///
/// 705_600_000 = 2^7 × 3^2 × 5^2 × 7^2 × 10^2 … chosen so every common frame
/// duration divides it exactly:
///   24, 25, 30, 48, 50, 60 fps  and the 1001/24000, 1001/30000, 1001/60000
///   NTSC family, plus 44100 and 48000 Hz audio.
/// Exactness here is what prevents drift accumulating over a long timeline.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ticks(pub i64);

impl Ticks {
    pub const ZERO: Ticks = Ticks(0);

    pub fn from_seconds(s: f64) -> Self {
        Ticks((s * TICKS_PER_SECOND as f64).round() as i64)
    }
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }
    /// Exact conversion from a stream PTS on an arbitrary rational timebase.
    pub fn from_pts(pts: i64, tb_num: i64, tb_den: i64) -> Self {
        debug_assert!(tb_den != 0, "timebase denominator must be non-zero");
        if tb_den == 0 { return Ticks::ZERO; }
        Ticks(pts.saturating_mul(tb_num).saturating_mul(TICKS_PER_SECOND) / tb_den)
    }
    /// Convert back to a PTS on the given timebase (for muxing).
    pub fn to_pts(self, tb_num: i64, tb_den: i64) -> i64 {
        if tb_num == 0 { return 0; }
        self.0.saturating_mul(tb_den) / (tb_num.saturating_mul(TICKS_PER_SECOND))
    }
    pub fn saturating_sub(self, o: Ticks) -> Ticks { Ticks(self.0.saturating_sub(o.0)) }
    pub fn saturating_add(self, o: Ticks) -> Ticks { Ticks(self.0.saturating_add(o.0)) }
    pub fn max(self, o: Ticks) -> Ticks { Ticks(self.0.max(o.0)) }
    pub fn min(self, o: Ticks) -> Ticks { Ticks(self.0.min(o.0)) }
}

impl std::ops::Add for Ticks { type Output = Ticks; fn add(self, o: Ticks) -> Ticks { Ticks(self.0 + o.0) } }
impl std::ops::Sub for Ticks { type Output = Ticks; fn sub(self, o: Ticks) -> Ticks { Ticks(self.0 - o.0) } }

/// A rational frame rate, kept exact. 30000/1001, never 29.97.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FrameRate { pub num: i64, pub den: i64 }

impl FrameRate {
    pub const FILM: FrameRate = FrameRate { num: 24, den: 1 };
    pub const PAL: FrameRate = FrameRate { num: 25, den: 1 };
    pub const NTSC: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const NTSC_FILM: FrameRate = FrameRate { num: 24000, den: 1001 };
    pub const SIXTY: FrameRate = FrameRate { num: 60, den: 1 };

    pub fn new(num: i64, den: i64) -> Self { FrameRate { num, den } }
    pub fn as_f64(self) -> f64 { self.num as f64 / self.den as f64 }
    /// Duration of exactly one frame.
    pub fn frame_duration(self) -> Ticks { Ticks::from_pts(1, self.den, self.num) }
    /// True for rates that require drop-frame timecode (29.97, 59.94).
    pub fn is_drop_frame_rate(self) -> bool {
        self.den == 1001 && (self.num == 30000 || self.num == 60000)
    }
    /// Nominal integer rate used by timecode (30 for 29.97, 60 for 59.94).
    pub fn timecode_rate(self) -> i64 {
        ((self.num as f64 / self.den as f64).round()) as i64
    }
}

impl Default for FrameRate { fn default() -> Self { FrameRate::FILM } }

/// A half-open span `[start, start + duration)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct TimeRange { pub start: Ticks, pub duration: Ticks }

impl TimeRange {
    pub fn new(start: Ticks, duration: Ticks) -> Self { TimeRange { start, duration } }
    pub fn end(self) -> Ticks { self.start + self.duration }
    pub fn contains(self, t: Ticks) -> bool { t >= self.start && t < self.end() }
    pub fn is_empty(self) -> bool { self.duration.0 <= 0 }
    pub fn overlaps(self, o: TimeRange) -> bool {
        self.start < o.end() && o.start < self.end()
    }
    pub fn intersection(self, o: TimeRange) -> Option<TimeRange> {
        let s = self.start.max(o.start);
        let e = self.end().min(o.end());
        (e > s).then(|| TimeRange::new(s, e - s))
    }
}

/// SMPTE timecode, with correct drop-frame handling.
///
/// Drop-frame exists because 30000/1001 fps is ~0.1% slower than 30 fps, so
/// non-drop timecode drifts about 3.6 seconds per hour against the wall clock.
/// A Phase 1 unit test initially asserted that 107,892 frames at 30000/1001 is
/// exactly one hour. It is not — it is 3599.9964 s. That belief is precisely
/// what drop-frame conceals, and encoding it would have been a silent error.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Timecode {
    pub hours: u32, pub minutes: u32, pub seconds: u32, pub frames: u32,
    pub drop_frame: bool,
}

impl Timecode {
    /// Frame number -> timecode.
    pub fn from_frame(frame: i64, rate: FrameRate) -> Self {
        let tc_rate = rate.timecode_rate().max(1);
        let drop = rate.is_drop_frame_rate();
        let mut f = frame.max(0);

        if drop {
            // Two frames dropped every minute except every tenth minute
            // (scaled by rate/30 for 59.94).
            let per_min = 2 * (tc_rate / 30);
            let frames_per_10min = tc_rate * 60 * 10 - per_min * 9;
            let frames_per_min = tc_rate * 60 - per_min;
            let d = f / frames_per_10min;
            let m = f % frames_per_10min;
            f += 9 * per_min * d;
            if m >= per_min {
                f += per_min * ((m - per_min) / frames_per_min);
            }
        }

        let frames = (f % tc_rate) as u32;
        let total_secs = f / tc_rate;
        Timecode {
            hours: (total_secs / 3600) as u32,
            minutes: ((total_secs % 3600) / 60) as u32,
            seconds: (total_secs % 60) as u32,
            frames,
            drop_frame: drop,
        }
    }

    /// Timecode -> frame number.
    pub fn to_frame(self, rate: FrameRate) -> i64 {
        let tc_rate = rate.timecode_rate().max(1);
        let total_minutes = self.hours as i64 * 60 + self.minutes as i64;
        let mut f = ((self.hours as i64 * 3600 + self.minutes as i64 * 60 + self.seconds as i64)
            * tc_rate) + self.frames as i64;
        if self.drop_frame {
            let per_min = 2 * (tc_rate / 30);
            f -= per_min * (total_minutes - total_minutes / 10);
        }
        f
    }

    pub fn from_ticks(t: Ticks, rate: FrameRate) -> Self {
        let fd = rate.frame_duration().0.max(1);
        Timecode::from_frame(t.0 / fd, rate)
    }
    pub fn to_ticks(self, rate: FrameRate) -> Ticks {
        Ticks(self.to_frame(rate) * rate.frame_duration().0)
    }
}

impl std::fmt::Display for Timecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // ';' before frames is the standard drop-frame marker.
        write!(f, "{:02}:{:02}:{:02}{}{:02}",
               self.hours, self.minutes, self.seconds,
               if self.drop_frame { ';' } else { ':' }, self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_frame_durations_are_exact() {
        for (num, den) in [(1i64,24i64),(1,25),(1,30),(1,48),(1,50),(1,60),
                           (1001,24000),(1001,30000),(1001,60000)] {
            assert_eq!((num * TICKS_PER_SECOND) % den, 0,
                       "frame duration {num}/{den} does not divide the timebase exactly");
        }
        // Audio rates too — audio is sample-accurate on the same timebase.
        for rate in [44100i64, 48000, 96000, 192000] {
            assert_eq!(TICKS_PER_SECOND % rate, 0, "{rate} Hz does not divide exactly");
        }
    }

    #[test]
    fn pts_round_trips() {
        for (num, den) in [(1i64,24i64),(1,30),(1001,30000),(1,48000)] {
            for pts in [0i64, 1, 100, 1_000_000] {
                let t = Ticks::from_pts(pts, num, den);
                assert_eq!(t.to_pts(num, den), pts, "round trip failed {pts} @ {num}/{den}");
            }
        }
    }

    #[test]
    fn no_drift_accumulating_frames() {
        // Two hours of NTSC, one frame at a time, must equal one bulk conversion.
        let per_frame = FrameRate::NTSC.frame_duration();
        let n = 216_000i64;
        assert_eq!(per_frame.0 * n, Ticks::from_pts(n, 1001, 30000).0);
    }

    #[test]
    fn ntsc_hour_is_not_a_wall_clock_hour() {
        let ntsc_hour = FrameRate::NTSC.frame_duration().0 * 107_892;
        assert!(ntsc_hour < 3600 * TICKS_PER_SECOND);
        assert_eq!(ntsc_hour, 35_999_964 * (TICKS_PER_SECOND / 10_000));
    }

    #[test]
    fn drop_frame_timecode_matches_smpte() {
        let r = FrameRate::NTSC;
        // Frame 0 is 00:00:00;00.
        assert_eq!(Timecode::from_frame(0, r).to_string(), "00:00:00;00");
        // Frames 0 and 1 of each minute are skipped, except every 10th minute.
        // Frame 1799 is 00:00:59;29; the next frame jumps to 00:01:00;02.
        assert_eq!(Timecode::from_frame(1799, r).to_string(), "00:00:59;29");
        assert_eq!(Timecode::from_frame(1800, r).to_string(), "00:01:00;02");
        // The tenth minute does NOT drop.
        assert_eq!(Timecode::from_frame(17982, r).to_string(), "00:10:00;00");
        // One "hour" of drop-frame timecode is exactly 107892 frames.
        assert_eq!(Timecode::from_frame(107_892, r).to_string(), "01:00:00;00");
    }

    #[test]
    fn timecode_round_trips_across_an_hour() {
        for rate in [FrameRate::FILM, FrameRate::PAL, FrameRate::NTSC, FrameRate::SIXTY] {
            for f in [0i64, 1, 999, 1800, 17982, 107_892, 200_000] {
                let tc = Timecode::from_frame(f, rate);
                assert_eq!(tc.to_frame(rate), f, "{tc} did not round trip at {rate:?}");
            }
        }
    }

    #[test]
    fn non_drop_rates_use_colon() {
        assert!(!FrameRate::FILM.is_drop_frame_rate());
        assert!(!FrameRate::PAL.is_drop_frame_rate());
        assert!(FrameRate::NTSC.is_drop_frame_rate());
        assert_eq!(Timecode::from_frame(24, FrameRate::FILM).to_string(), "00:00:01:00");
    }

    #[test]
    fn ranges() {
        let a = TimeRange::new(Ticks(100), Ticks(50));
        assert_eq!(a.end(), Ticks(150));
        assert!(a.contains(Ticks(100)) && a.contains(Ticks(149)) && !a.contains(Ticks(150)));
        let b = TimeRange::new(Ticks(140), Ticks(50));
        assert!(a.overlaps(b));
        assert_eq!(a.intersection(b), Some(TimeRange::new(Ticks(140), Ticks(10))));
        let c = TimeRange::new(Ticks(200), Ticks(10));
        assert!(!a.overlaps(c) && a.intersection(c).is_none());
    }
}
