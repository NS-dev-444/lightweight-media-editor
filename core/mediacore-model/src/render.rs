//! Render plans: what to decode and composite at a given instant.
//!
//! AD-4 draws the line here — **the core decides WHAT to draw, the platform
//! decides HOW**. A plan is small, serialisable, platform-free data. Metal and
//! Direct3D each consume the same plan and produce pixels their own way, so
//! edit semantics stay shared while the rendering stays native.
//!
//! Nothing in this module touches pixels. A 4K frame is ~33 MB; per S1 those
//! never cross the FFI as data, only as platform buffer handles.

use crate::asset::AssetId;
use crate::time::{Ticks, TimeRange};
use crate::timeline::{ClipId, Effects, Geometry, Timeline, TrackKind};

/// One video source to draw, bottom-most first.
#[derive(Clone, PartialEq, Debug)]
pub struct VideoLayer {
    pub clip: ClipId,
    pub asset: AssetId,
    /// Where to read from the SOURCE — this is what the decoder seeks to.
    pub source_time: Ticks,
    /// 0.0–1.0 after fades are applied.
    pub opacity: f64,
    /// True when this clip is an overlay (text, logo) rather than the base.
    pub is_overlay: bool,
    /// §18 effects to apply. The platform compositor consumes these; the core
    /// never touches pixels (AD-4).
    pub effects: Effects,
    /// True when this layer draws TEXT rather than decoded media. The
    /// compositor fetches the spec separately — text changes rarely, so it does
    /// not belong in a per-frame bulk read.
    pub is_text: bool,
    /// True when this layer draws an image overlay (§20).
    pub is_image: bool,
    /// True when this layer draws a CAPTION. The compositor fetches the words
    /// with `mcs_caption_at` — captions change every few seconds, so they do
    /// not belong in a per-frame bulk read any more than a title does.
    pub is_caption: bool,
    /// How much of the clip's LUT to apply, 0 when it has none. The TABLE is
    /// fetched separately by path — it is megabytes of data and has no place in
    /// a per-frame bulk read (AD-1).
    pub lut_amount: f32,
    /// How the source is framed: crop, quarter turns, flips.
    pub geometry: Geometry,
}

/// One audio source to mix.
#[derive(Clone, PartialEq, Debug)]
pub struct AudioLayer {
    pub clip: ClipId,
    pub asset: AssetId,
    pub source_time: Ticks,
    /// Clip gain with fades applied.
    pub gain: f64,
    /// Playback rate; the resampler needs this, not just the seek position.
    pub speed: f64,
}

/// Everything needed to produce one frame.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RenderPlan {
    pub time: Ticks,
    /// Bottom-most first, so the platform composites in order.
    pub video: Vec<VideoLayer>,
    pub audio: Vec<AudioLayer>,
}

impl RenderPlan {
    /// Nothing to show. The preview draws black rather than the previous frame.
    pub fn is_empty(&self) -> bool { self.video.is_empty() && self.audio.is_empty() }
    /// Distinct assets the decoder must have open. Drives decoder pooling —
    /// S1 measured ~337 MB per open 4K stream (R-23), so this number matters.
    pub fn required_assets(&self) -> Vec<AssetId> {
        let mut v: Vec<AssetId> = self.video.iter().map(|l| l.asset)
            .chain(self.audio.iter().map(|l| l.asset)).collect();
        v.sort(); v.dedup(); v
    }
}

/// Fade multiplier for a clip at a timeline instant.
///
/// Linear, and clamped so overlapping fades on a short clip cannot produce a
/// negative or >1 multiplier.
fn fade_multiplier(t: Ticks, range: TimeRange, fade_in: Ticks, fade_out: Ticks) -> f64 {
    let mut m = 1.0f64;
    if fade_in.0 > 0 {
        let into = (t - range.start).0 as f64;
        if into < fade_in.0 as f64 { m = m.min(into / fade_in.0 as f64); }
    }
    if fade_out.0 > 0 {
        let remaining = (range.end() - t).0 as f64;
        if remaining < fade_out.0 as f64 { m = m.min(remaining / fade_out.0 as f64); }
    }
    m.clamp(0.0, 1.0)
}

/// Build the plan for one instant.
///
/// Track order defines compositing order: the video track is the base and
/// overlays sit above it, matching the V1 layout in PRODUCT_DIRECTION.md §6.
pub fn plan_at(tl: &Timeline, t: Ticks) -> RenderPlan {
    plan_at_with_captions(tl, t, Some(&tl.captions))
}

/// The plan for an instant, with an explicit caption source.
///
/// `plan_at` uses the timeline's own captions; this exists so a caller can pass
/// `None` to plan a frame deliberately without them — which the caption editor
/// wants, so its preview is not showing a burned-in copy of the line being
/// edited underneath the editable one.
///
/// Captions are added as an ordinary TEXT layer, on top of everything else.
/// That is the whole point of the design: burning in reuses the text renderer,
/// the text presets and the export path exactly as they are, instead of a
/// second way to draw words on a frame.
pub fn plan_at_with_captions(tl: &Timeline, t: Ticks,
                             captions: Option<&crate::captions::Captions>) -> RenderPlan {
    let mut plan = plan_without_captions(tl, t);
    if let Some(c) = captions {
        if c.burn_in && c.at(t).is_some() {
            plan.video.push(VideoLayer {
                // A caption is not a clip and has no id of its own to give,
                // so the sentinel says "ask for the caption at this instant"
                // rather than "look up clip 0", which does not exist.
                clip: ClipId(0),
                asset: AssetId(0),
                source_time: t,
                opacity: 1.0,
                is_overlay: true,
                effects: Effects::default(),
                is_text: true,
                is_image: false,
                is_caption: true,
                geometry: Geometry::default(),
                lut_amount: 0.0,
            });
        }
    }
    plan
}

fn plan_without_captions(tl: &Timeline, t: Ticks) -> RenderPlan {
    let mut plan = RenderPlan { time: t, ..Default::default() };

    for track in &tl.tracks {
        if track.muted { continue; }
        let Some(clip) = track.clip_at(t) else { continue };
        let Some(source_time) = clip.source_time_at(t) else { continue };
        let range = clip.timeline_range();
        let fade = fade_multiplier(t, range, clip.fade_in, clip.fade_out);

        match track.kind {
            TrackKind::Video | TrackKind::Overlay => {
                plan.video.push(VideoLayer {
                    clip: clip.id, asset: clip.asset, source_time,
                    opacity: fade,
                    is_overlay: track.kind == TrackKind::Overlay,
                    effects: clip.effects,
                    is_text: clip.is_text(),
                    is_image: clip.is_image(),
                    is_caption: false,
                    geometry: clip.geometry,
                    lut_amount: clip.lut.as_ref()
                        .filter(|l| !l.path.is_empty())
                        .map(|l| l.amount.clamp(0.0, 1.0)).unwrap_or(0.0),
                });
            }
            TrackKind::Audio => {
                plan.audio.push(AudioLayer {
                    clip: clip.id, asset: clip.asset, source_time,
                    // Three multipliers, deliberately kept separate in the
                    // model and combined only here: the clip's own level, its
                    // fades, and its automation curve. The mixer applies what
                    // it is told rather than re-deriving any of it, so preview,
                    // export and the model cannot disagree.
                    gain: clip.gain * fade * clip.automation_at(t - range.start),
                    speed: clip.speed,
                });
            }
        }
    }
    plan
}

/// Time ranges a set of edits invalidated.
///
/// §26: never re-render everything because one item changed. The render cache
/// is invalidated by time RANGE, and only the affected span is recomputed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirtyRanges { ranges: Vec<TimeRange> }

impl DirtyRanges {
    pub fn new() -> Self { Self::default() }
    pub fn is_empty(&self) -> bool { self.ranges.is_empty() }
    pub fn as_slice(&self) -> &[TimeRange] { &self.ranges }

    /// Add a range, merging with anything it touches so the list stays minimal.
    pub fn add(&mut self, r: TimeRange) {
        if r.is_empty() { return; }
        let mut merged = r;
        let mut keep: Vec<TimeRange> = Vec::with_capacity(self.ranges.len() + 1);
        for existing in self.ranges.drain(..) {
            // Adjacent counts as touching: two abutting dirty spans are one.
            if existing.overlaps(merged)
                || existing.end() == merged.start
                || merged.end() == existing.start
            {
                let s = existing.start.min(merged.start);
                let e = existing.end().max(merged.end());
                merged = TimeRange::new(s, e - s);
            } else {
                keep.push(existing);
            }
        }
        keep.push(merged);
        keep.sort_by_key(|x| x.start);
        self.ranges = keep;
    }

    pub fn contains(&self, t: Ticks) -> bool { self.ranges.iter().any(|r| r.contains(t)) }
    pub fn intersects(&self, r: TimeRange) -> bool { self.ranges.iter().any(|x| x.overlaps(r)) }
    pub fn clear(&mut self) { self.ranges.clear(); }
    pub fn total(&self) -> Ticks {
        Ticks(self.ranges.iter().map(|r| r.duration.0).sum())
    }
}

/// The span a clip occupies, which is what its edits dirty.
pub fn dirty_for_clip(tl: &Timeline, clip: ClipId) -> Option<TimeRange> {
    tl.find_clip(clip).map(|(_, c)| c.timeline_range())
}
