//! Timeline: tracks and clips.
//!
//! V1 scope per PRODUCT_DIRECTION.md §6 (ratified): ONE video track, ONE
//! overlay track, TWO audio tracks. That resolves the §2-vs-§32 contradiction
//! Phase 0 found. The model permits more tracks so V1.1 needs no migration;
//! the UI is what constrains V1.

use crate::asset::AssetId;
use crate::time::{Ticks, TimeRange};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClipId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TrackKind { Video, Overlay, Audio }

/// A text overlay (§19, and PRODUCT_DIRECTION.md §7).
///
/// §19 lists font/size/weight/alignment/position/opacity. That set produces the
/// exact look that says "made in a free editor": Arial Bold, white, dead
/// centre, no padding. PRODUCT_DIRECTION §7 argues bad text is the number one
/// tell of an amateur edit and the cheapest thing in the spec to get right, so
/// this adds the parts that actually separate good type from bad:
///
///   * **tracking and line height** — most of the difference, absent from §19
///   * **outline and shadow** — text sits over moving footage; white-on-anything
///     is unreadable half the time
///   * **presets** — finished looks with padding and weight already solved, so
///     the default is good enough to ship untouched
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSpec {
    pub text: String,
    pub font: String,
    pub size: f32,
    /// 100–900, CSS-style.
    pub weight: u16,
    /// Letter spacing as a fraction of size. Negative tightens.
    pub tracking: f32,
    /// Line spacing as a multiple of size.
    pub line_height: f32,
    /// 0 = left, 1 = centre, 2 = right.
    pub align: u8,
    /// Normalised 0..1 within the frame; (0.5, 0.85) is the lower-third default.
    pub x: f32,
    pub y: f32,
    pub opacity: f32,
    /// RGBA 0..1.
    pub colour: [f32; 4],
    pub outline_width: f32,
    pub outline_colour: [f32; 4],
    pub shadow_radius: f32,
    pub shadow_opacity: f32,
    /// Padded background behind the text; alpha 0 disables it.
    pub background: [f32; 4],
    pub background_padding: f32,
}

impl Default for TextSpec {
    fn default() -> Self { TextPreset::Clean.spec("Text") }
}

/// Finished looks, not fonts.
///
/// The point of a preset is that padding, weight and contrast are already
/// solved — the default must be good enough to ship without adjustment.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TextPreset { Clean, Subtitle, Bold, Card }

impl TextPreset {
    pub fn all() -> [TextPreset; 4] {
        [TextPreset::Clean, TextPreset::Subtitle, TextPreset::Bold, TextPreset::Card]
    }
    pub fn name(self) -> &'static str {
        match self {
            TextPreset::Clean => "Clean",
            TextPreset::Subtitle => "Subtitle",
            TextPreset::Bold => "Bold",
            TextPreset::Card => "Card",
        }
    }
    pub fn spec(self, text: impl Into<String>) -> TextSpec {
        let base = TextSpec {
            text: text.into(),
            font: "SF Pro Display".into(),
            size: 0.06, weight: 600, tracking: -0.01, line_height: 1.2,
            align: 1, x: 0.5, y: 0.85, opacity: 1.0,
            colour: [1.0, 1.0, 1.0, 1.0],
            outline_width: 0.0, outline_colour: [0.0, 0.0, 0.0, 1.0],
            shadow_radius: 0.004, shadow_opacity: 0.55,
            background: [0.0, 0.0, 0.0, 0.0], background_padding: 0.02,
        };
        match self {
            // Shadow rather than outline: cleaner over most footage.
            TextPreset::Clean => base,
            // Readable over anything, which is what subtitles must be.
            TextPreset::Subtitle => TextSpec {
                size: 0.045, weight: 500, y: 0.88,
                outline_width: 0.0035, shadow_opacity: 0.7, ..base
            },
            TextPreset::Bold => TextSpec {
                size: 0.10, weight: 800, tracking: -0.02, y: 0.5,
                shadow_radius: 0.006, shadow_opacity: 0.6, ..base
            },
            // A padded plate — the most legible option over busy footage.
            TextPreset::Card => TextSpec {
                size: 0.05, weight: 600, y: 0.82,
                background: [0.0, 0.0, 0.0, 0.62], background_padding: 0.022,
                shadow_opacity: 0.0, ..base
            },
        }
    }
}

/// An image overlay (§20): PNG, JPEG, WebP where practical.
///
/// Shares the clip machinery with text and media, so trim, split, move, fades
/// and undo all work on it unchanged.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSpec {
    pub path: String,
    /// Normalised centre within the frame.
    pub x: f32,
    pub y: f32,
    /// Height as a fraction of the frame; width follows the image's aspect, so
    /// a logo never stretches.
    pub scale: f32,
    /// Degrees, clockwise.
    pub rotation: f32,
    pub opacity: f32,
}

impl Default for ImageSpec {
    fn default() -> Self {
        ImageSpec { path: String::new(), x: 0.5, y: 0.5, scale: 0.25,
                    rotation: 0.0, opacity: 1.0 }
    }
}

/// A colour lookup table applied to a clip (PRODUCT_DIRECTION.md §7).
///
/// "LUTs instead of eight sliders." §18's brightness/contrast/saturation is the
/// 1998 version of colour grading; one good LUT beats twenty minutes of
/// slider-nudging, and creators already own LUTs. The sliders stay — this is
/// the headline, not the replacement.
///
/// Only the PATH is stored, never the table itself. A .cube file is often
/// several megabytes, and a project file that swallowed one would be a project
/// file nobody could email. It relinks like any other missing media (§25).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LutSpec {
    pub path: String,
    /// 0..1 — how much of the graded result to mix back in. Being able to dial
    /// a LUT back to 60 % is most of what makes them usable rather than a
    /// preset that is either on or wrong.
    pub amount: f32,
}

impl Default for LutSpec {
    fn default() -> Self { LutSpec { path: String::new(), amount: 1.0 } }
}

/// One clip: a window onto a source, placed on the timeline.
///
/// `source` is the span taken FROM the asset; `timeline_start` is where it
/// begins on the timeline. Trimming changes the window, never the file.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub asset: AssetId,
    pub timeline_start: Ticks,
    pub source: TimeRange,
    /// 1.0 = normal. Affects how much source is consumed per timeline tick.
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default = "one")]
    pub gain: f64,
    #[serde(default)]
    pub fade_in: Ticks,
    #[serde(default)]
    pub fade_out: Ticks,
    #[serde(default, skip_serializing_if = "Effects::is_neutral")]
    pub effects: Effects,
    /// Crop, rotation and flip (PRODUCT_DIRECTION.md §6). Separate from
    /// `Effects` because framing and colour are different operations with
    /// different UI, and only framing changes the shape of what is drawn.
    #[serde(default, skip_serializing_if = "Geometry::is_neutral")]
    pub geometry: Geometry,
    /// A volume curve over the clip, in order of `at`.
    ///
    /// Empty for almost every clip, which is why it is skipped when writing.
    /// It exists for **ducking** — music stepping aside under a voice — which
    /// is a changing gain by definition and cannot be expressed as one number.
    /// Fades stay separate because they are anchored to the clip's ENDS and
    /// survive trimming differently.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gain_points: Vec<GainPoint>,
    /// When set, this clip draws TEXT rather than media (§19). Kept as an
    /// option on Clip rather than a separate clip type so trim, split, move,
    /// fades and undo all work on it unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextSpec>,
    /// When set, this clip draws an IMAGE overlay (§20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSpec>,
    /// A colour lookup table applied to this clip's picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<LutSpec>,
}

fn one() -> f64 { 1.0 }

/// §18's initial effect set — and nothing more.
///
/// "Do not build a giant effects marketplace." Each of these is a few lines of
/// shader maths, which is why AD-4 can afford to write them twice (MSL and
/// HLSL) from one written specification rather than adopting a cross-platform
/// shader abstraction. If this list ever passes ~20 entries, revisit that.
///
/// All values are neutral at their `Default`, so an untouched clip costs
/// nothing and serialises to almost nothing.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    /// -1..1, 0 = unchanged.
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// -1..1, negative = cooler, positive = warmer.
    pub temperature: f32,
    pub tint: f32,
    pub grayscale: bool,
    /// 0..1.
    pub blur: f32,
    pub sharpen: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Effects { brightness: 0.0, contrast: 0.0, saturation: 0.0,
                  temperature: 0.0, tint: 0.0, grayscale: false,
                  blur: 0.0, sharpen: 0.0 }
    }
}

impl Effects {
    /// True when this clip needs no effect pass at all — the common case, and
    /// worth checking so an untouched timeline skips the work entirely.
    pub fn is_neutral(&self) -> bool { *self == Effects::default() }

    /// Clamp to sane ranges. Values arrive from a UI and from project files,
    /// neither of which can be trusted to stay in range.
    pub fn clamped(mut self) -> Self {
        for v in [&mut self.brightness, &mut self.contrast, &mut self.saturation,
                  &mut self.temperature, &mut self.tint] {
            *v = v.clamp(-1.0, 1.0);
        }
        self.blur = self.blur.clamp(0.0, 1.0);
        self.sharpen = self.sharpen.clamp(0.0, 1.0);
        self
    }
}

/// One point on a clip's volume curve.
///
/// `at` is measured from the CLIP's start, not the timeline's, so trimming or
/// moving a clip carries its automation with it instead of leaving it behind.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct GainPoint {
    pub at: Ticks,
    /// Multiplier, 0..1. Applied on top of the clip's own gain and its fades.
    pub gain: f32,
}

/// Crop, rotation and flip — how the source is FRAMED, as opposed to how it is
/// coloured (`Effects`).
///
/// The crop is in **normalised** source coordinates, 0..1, not pixels. That is
/// the difference between a crop that survives everything and one that does
/// not: the same clip may be re-linked to a different resolution (§25), swapped
/// for a proxy, or exported at a size other than the preview's, and a crop in
/// pixels silently means something different in each case. Normalised, "the
/// middle half" stays the middle half.
///
/// Rotation is in quarter turns because that is what the operation actually is.
/// Arbitrary-angle rotation needs a resampling filter, a decision about what
/// fills the corners, and a UI for the angle — none of which V1 has, and a
/// quarter turn is what a sideways phone video needs.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Geometry {
    /// Left edge of the visible region, 0..1 of the source width.
    pub crop_x: f32,
    pub crop_y: f32,
    /// Width of the visible region, 0..1. 1.0 = no horizontal crop.
    pub crop_w: f32,
    pub crop_h: f32,
    /// Quarter turns clockwise, 0-3.
    pub rotation: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Geometry { crop_x: 0.0, crop_y: 0.0, crop_w: 1.0, crop_h: 1.0,
                   rotation: 0, flip_h: false, flip_v: false }
    }
}

impl Geometry {
    /// True when this clip is drawn exactly as it was shot.
    pub fn is_neutral(&self) -> bool { *self == Geometry::default() }

    /// True when the frame's width and height swap — an odd number of quarter
    /// turns. The compositor needs this to work out the drawn aspect ratio.
    pub fn swaps_axes(&self) -> bool { self.rotation % 2 == 1 }

    /// The visible aspect ratio, given the source's.
    pub fn aspect(&self, source_aspect: f64) -> f64 {
        let a = source_aspect * (self.crop_w as f64 / self.crop_h.max(f32::EPSILON) as f64);
        if self.swaps_axes() { 1.0 / a } else { a }
    }

    /// Clamp to a rectangle that actually exists inside the source.
    ///
    /// Values arrive from a UI and from project files, neither of which can be
    /// trusted. A zero or negative width would divide by zero downstream; a
    /// rectangle hanging off the edge would sample outside the texture.
    pub fn clamped(mut self) -> Self {
        self.crop_w = self.crop_w.clamp(0.01, 1.0);
        self.crop_h = self.crop_h.clamp(0.01, 1.0);
        self.crop_x = self.crop_x.clamp(0.0, 1.0 - self.crop_w);
        self.crop_y = self.crop_y.clamp(0.0, 1.0 - self.crop_h);
        self.rotation %= 4;
        self
    }

    /// Centre the largest rectangle of `target` aspect that fits in the source.
    ///
    /// This is "reframe to 9:16" — the operation a creator actually performs
    /// when a landscape shot has to become a vertical one. Centred because a
    /// centre crop is right often enough to be the default, and the crop
    /// controls are there when it is not.
    pub fn fill_aspect(source_aspect: f64, target_aspect: f64) -> Self {
        let mut g = Geometry::default();
        if !(source_aspect.is_finite() && target_aspect.is_finite())
            || source_aspect <= 0.0 || target_aspect <= 0.0 { return g; }
        if target_aspect < source_aspect {
            // Taller than the source: keep full height, take a slice of width.
            g.crop_w = (target_aspect / source_aspect) as f32;
            g.crop_x = (1.0 - g.crop_w) / 2.0;
        } else if target_aspect > source_aspect {
            g.crop_h = (source_aspect / target_aspect) as f32;
            g.crop_y = (1.0 - g.crop_h) / 2.0;
        }
        g.clamped()
    }
}

impl Clip {
    /// The automation multiplier at a point measured from the clip's start.
    ///
    /// Linear between points, flat outside them. Flat rather than extrapolated:
    /// a curve that keeps falling past its last point would silence the tail of
    /// a clip for no reason the user could see.
    pub fn automation_at(&self, from_clip_start: Ticks) -> f64 {
        if self.gain_points.is_empty() { return 1.0; }
        let t = from_clip_start.0;
        let first = self.gain_points[0];
        if t <= first.at.0 { return first.gain as f64; }
        let last = self.gain_points[self.gain_points.len() - 1];
        if t >= last.at.0 { return last.gain as f64; }
        for w in self.gain_points.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t >= a.at.0 && t <= b.at.0 {
                let span = (b.at.0 - a.at.0) as f64;
                if span <= 0.0 { return b.gain as f64; }
                let k = (t - a.at.0) as f64 / span;
                return a.gain as f64 + (b.gain as f64 - a.gain as f64) * k;
            }
        }
        1.0
    }

    pub fn new(id: ClipId, asset: AssetId, timeline_start: Ticks, source: TimeRange) -> Self {
        Clip { id, asset, timeline_start, source, speed: 1.0, gain: 1.0,
               fade_in: Ticks::ZERO, fade_out: Ticks::ZERO,
               effects: Effects::default(), geometry: Geometry::default(),
               gain_points: Vec::new(), text: None, image: None, lut: None }
    }
    /// Duration on the timeline, which differs from source duration when speed != 1.
    pub fn timeline_duration(&self) -> Ticks {
        if self.speed <= 0.0 { return self.source.duration; }
        Ticks((self.source.duration.0 as f64 / self.speed).round() as i64)
    }
    pub fn timeline_range(&self) -> TimeRange {
        TimeRange::new(self.timeline_start, self.timeline_duration())
    }
    pub fn is_text(&self) -> bool { self.text.is_some() }
    pub fn is_image(&self) -> bool { self.image.is_some() }

    /// Map a timeline instant to the corresponding instant in the source.
    pub fn source_time_at(&self, t: Ticks) -> Option<Ticks> {
        let r = self.timeline_range();
        if !r.contains(t) { return None; }
        let into = (t - r.start).0 as f64 * self.speed;
        Some(self.source.start + Ticks(into.round() as i64))
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    /// Ordered by `timeline_start`; the invariant is maintained by the methods
    /// here and checked by `validate`.
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind, name: impl Into<String>) -> Self {
        Track { id, kind, name: name.into(), muted: false, locked: false, clips: Vec::new() }
    }
    pub fn duration(&self) -> Ticks {
        self.clips.iter().map(|c| c.timeline_range().end()).max().unwrap_or(Ticks::ZERO)
    }
    pub fn clip(&self, id: ClipId) -> Option<&Clip> { self.clips.iter().find(|c| c.id == id) }
    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == id)
    }
    pub fn clip_at(&self, t: Ticks) -> Option<&Clip> {
        self.clips.iter().find(|c| c.timeline_range().contains(t))
    }
    /// Insert keeping clips ordered by start time.
    pub fn insert_clip(&mut self, clip: Clip) {
        let pos = self.clips.iter()
            .position(|c| c.timeline_start > clip.timeline_start)
            .unwrap_or(self.clips.len());
        self.clips.insert(pos, clip);
    }
    pub fn remove_clip(&mut self, id: ClipId) -> Option<Clip> {
        let i = self.clips.iter().position(|c| c.id == id)?;
        Some(self.clips.remove(i))
    }
    pub fn resort(&mut self) { self.clips.sort_by_key(|c| c.timeline_start); }
    /// Clips that overlap each other on the same track — always a bug.
    pub fn overlaps(&self) -> Vec<(ClipId, ClipId)> {
        let mut out = Vec::new();
        for (i, a) in self.clips.iter().enumerate() {
            for b in &self.clips[i + 1..] {
                if a.timeline_range().overlaps(b.timeline_range()) { out.push((a.id, b.id)); }
            }
        }
        out
    }
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Timeline {
    pub tracks: Vec<Track>,
    /// §8's captions.
    ///
    /// On the timeline rather than on the Project, and the reason is the edit
    /// funnel: every edit is applied through `&mut Timeline`, and that is also
    /// what undo, redo and crash-recovery replay operate on. Captions living
    /// anywhere else would need a second path through all three — and a
    /// transcript is expensive enough to regenerate that it must be journalled
    /// like everything else.
    ///
    /// They are still not a *track*: a talk carries hundreds of them, they are
    /// exported as a sidecar as often as they are burned in, and they are
    /// edited as text in a list. See `captions.rs`.
    #[serde(default, skip_serializing_if = "crate::captions::Captions::is_empty")]
    pub captions: crate::captions::Captions,
}

impl Timeline {
    /// The V1 track layout (PRODUCT_DIRECTION.md §6).
    ///
    /// TWO overlay tracks, not one. Clips on a track may not overlap, so a
    /// single overlay track cannot hold a lower-third title AND a corner logo
    /// at the same time — which is an entirely ordinary thing to want, and was
    /// caught the moment both were tried together. The second track is the
    /// smallest fix that makes §19 and §20 usable simultaneously.
    pub fn v1_default() -> Self {
        Timeline {
            captions: crate::captions::Captions::default(),
            tracks: vec![
                Track::new(TrackId(1), TrackKind::Video,   "V1"),
                Track::new(TrackId(2), TrackKind::Overlay, "Overlay 1"),
                Track::new(TrackId(3), TrackKind::Overlay, "Overlay 2"),
                Track::new(TrackId(4), TrackKind::Audio,   "A1"),
                Track::new(TrackId(5), TrackKind::Audio,   "A2"),
            ],
        }
    }
    pub fn duration(&self) -> Ticks {
        self.tracks.iter().map(|t| t.duration()).max().unwrap_or(Ticks::ZERO)
    }
    pub fn track(&self, id: TrackId) -> Option<&Track> { self.tracks.iter().find(|t| t.id == id) }
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }
    pub fn find_clip(&self, id: ClipId) -> Option<(TrackId, &Clip)> {
        self.tracks.iter().find_map(|t| t.clip(id).map(|c| (t.id, c)))
    }
    /// Structural problems that should never be reachable through commands.
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for t in &self.tracks {
            for c in &t.clips {
                if !seen.insert(c.id) { errs.push(format!("duplicate clip id {:?}", c.id)); }
                if c.source.duration.0 <= 0 { errs.push(format!("clip {:?} has empty source", c.id)); }
                if c.timeline_start.0 < 0 { errs.push(format!("clip {:?} starts before zero", c.id)); }
                if c.speed <= 0.0 { errs.push(format!("clip {:?} has non-positive speed", c.id)); }
            }
            if !t.clips.windows(2).all(|w| w[0].timeline_start <= w[1].timeline_start) {
                errs.push(format!("track {:?} clips are not ordered", t.id));
            }
            for (a, b) in t.overlaps() {
                errs.push(format!("clips {a:?} and {b:?} overlap on track {:?}", t.id));
            }
        }
        errs
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn a_fresh_clip_is_drawn_as_shot() {
        assert!(Geometry::default().is_neutral());
        assert_eq!(Geometry::default().aspect(16.0 / 9.0), 16.0 / 9.0);
    }

    #[test]
    fn a_crop_outside_the_source_is_pulled_back_inside() {
        // Project files and UIs both produce out-of-range values; sampling
        // outside the texture is the bug this prevents.
        let g = Geometry { crop_x: 0.9, crop_y: -0.4, crop_w: 0.5, crop_h: 2.0,
                           rotation: 7, flip_h: false, flip_v: false }.clamped();
        assert!(g.crop_x >= 0.0 && g.crop_x + g.crop_w <= 1.0 + f32::EPSILON);
        assert!(g.crop_y >= 0.0 && g.crop_y + g.crop_h <= 1.0 + f32::EPSILON);
        assert_eq!(g.rotation, 3, "rotation should wrap, not clamp");
    }

    #[test]
    fn a_zero_width_crop_is_refused() {
        // Not cosmetic: crop_w reaches a division downstream.
        let g = Geometry { crop_w: 0.0, crop_h: 0.0, ..Default::default() }.clamped();
        assert!(g.crop_w > 0.0 && g.crop_h > 0.0);
    }

    #[test]
    fn a_quarter_turn_swaps_the_aspect_ratio() {
        let g = Geometry { rotation: 1, ..Default::default() };
        assert!(g.swaps_axes());
        assert!((g.aspect(16.0 / 9.0) - 9.0 / 16.0).abs() < 1e-9);
        let half = Geometry { rotation: 2, ..Default::default() };
        assert!(!half.swaps_axes());
        assert!((half.aspect(16.0 / 9.0) - 16.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn reframing_landscape_to_vertical_takes_a_centred_slice() {
        // 16:9 -> 9:16 is the reframe people actually do.
        let g = Geometry::fill_aspect(16.0 / 9.0, 9.0 / 16.0);
        assert!((g.crop_h - 1.0).abs() < 1e-6, "full height should be kept");
        assert!(g.crop_w < 1.0, "width must be narrowed");
        assert!((g.crop_x - (1.0 - g.crop_w) / 2.0).abs() < 1e-6, "not centred");
        assert!((g.aspect(16.0 / 9.0) - 9.0 / 16.0).abs() < 1e-6,
                "the result is not 9:16");
    }

    #[test]
    fn reframing_vertical_to_landscape_takes_a_centred_band() {
        let g = Geometry::fill_aspect(9.0 / 16.0, 16.0 / 9.0);
        assert!((g.crop_w - 1.0).abs() < 1e-6);
        assert!(g.crop_h < 1.0);
        assert!((g.aspect(9.0 / 16.0) - 16.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn reframing_to_the_same_shape_changes_nothing() {
        assert!(Geometry::fill_aspect(16.0 / 9.0, 16.0 / 9.0).is_neutral());
    }

    #[test]
    fn nonsense_aspects_do_not_produce_a_nonsense_crop() {
        for (s, t) in [(0.0, 1.0), (1.0, 0.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)] {
            let g = Geometry::fill_aspect(s, t);
            assert!(g.is_neutral(), "fill_aspect({s}, {t}) should fall back to no crop");
        }
    }

    #[test]
    fn geometry_survives_a_round_trip_through_a_project_file() {
        let g = Geometry { crop_x: 0.1, crop_y: 0.2, crop_w: 0.5, crop_h: 0.6,
                           rotation: 3, flip_h: true, flip_v: false };
        let json = serde_json::to_string(&g).expect("serialise");
        let back: Geometry = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(g, back);
    }

    #[test]
    fn an_untouched_clip_writes_no_geometry_at_all() {
        // skip_serializing_if keeps ordinary projects small and their files
        // readable — the same reason Effects does it.
        let c = Clip::new(ClipId(1), AssetId(1), Ticks(0),
                          TimeRange::new(Ticks(0), Ticks(100)));
        let json = serde_json::to_string(&c).expect("serialise");
        assert!(!json.contains("crop_x"), "neutral geometry should not be written");
    }
}

#[cfg(test)]
mod automation_tests {
    use super::*;

    fn clip_with(points: Vec<GainPoint>) -> Clip {
        let mut c = Clip::new(ClipId(1), AssetId(1), Ticks(0),
                              TimeRange::new(Ticks(0), Ticks(1000)));
        c.gain_points = points;
        c
    }

    #[test]
    fn a_clip_without_a_curve_is_unaffected() {
        assert_eq!(clip_with(vec![]).automation_at(Ticks(500)), 1.0);
    }

    #[test]
    fn the_curve_is_linear_between_points() {
        let c = clip_with(vec![
            GainPoint { at: Ticks(0), gain: 1.0 },
            GainPoint { at: Ticks(100), gain: 0.0 },
        ]);
        assert!((c.automation_at(Ticks(50)) - 0.5).abs() < 1e-9);
        assert!((c.automation_at(Ticks(25)) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn the_curve_is_flat_outside_its_points() {
        // Extrapolating would carry a falling curve past its last point and
        // silence the rest of the clip for no visible reason.
        let c = clip_with(vec![
            GainPoint { at: Ticks(100), gain: 0.3 },
            GainPoint { at: Ticks(200), gain: 0.9 },
        ]);
        // f32 points widened to f64: compare with a tolerance, not exactly.
        assert!((c.automation_at(Ticks(0)) - 0.3).abs() < 1e-6);
        assert!((c.automation_at(Ticks(50)) - 0.3).abs() < 1e-6);
        assert!((c.automation_at(Ticks(9999)) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn two_points_at_the_same_instant_do_not_divide_by_zero() {
        // A step change is a legitimate thing to ask for, and the obvious
        // interpolation divides by the span between the points.
        let c = clip_with(vec![
            GainPoint { at: Ticks(100), gain: 1.0 },
            GainPoint { at: Ticks(100), gain: 0.2 },
        ]);
        let g = c.automation_at(Ticks(100));
        assert!(g.is_finite(), "got {g}");
    }

    #[test]
    fn a_curve_survives_a_round_trip_through_a_project_file() {
        let c = clip_with(vec![
            GainPoint { at: Ticks(0), gain: 1.0 },
            GainPoint { at: Ticks(500), gain: 0.25 },
        ]);
        let json = serde_json::to_string(&c).expect("serialise");
        let back: Clip = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.gain_points, c.gain_points);
    }

    #[test]
    fn an_untouched_clip_writes_no_curve() {
        let c = clip_with(vec![]);
        let json = serde_json::to_string(&c).expect("serialise");
        assert!(!json.contains("gain_points"));
    }
}
