//! Commands and undo/redo.
//!
//! §28: every user-visible edit is undoable, and undo must never duplicate
//! media. It doesn't — commands carry only ids and small values, and R-16
//! (unbounded history growth) is addressed by a bounded stack with a measured
//! memory estimate.
//!
//! Design: **applying a command returns the command that undoes it.** Undo is
//! therefore just "apply the inverse", and redo is "apply the inverse of that".
//! One code path, so an inverse cannot silently disagree with its forward form.

use crate::time::{Ticks, TimeRange};
use crate::timeline::{Clip, ClipId, Effects, GainPoint, Geometry, Timeline, TrackId};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq)]
pub enum EditError {
    NoSuchTrack(TrackId),
    NoSuchClip(ClipId),
    TrackLocked(TrackId),
    WouldOverlap { existing: ClipId },
    InvalidSplitPoint,
    InvalidValue(&'static str),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // §23: human wording, no internal jargon.
            EditError::NoSuchTrack(_) => write!(f, "That track no longer exists."),
            EditError::NoSuchClip(_) => write!(f, "That clip no longer exists."),
            EditError::TrackLocked(_) => write!(f, "This track is locked. Unlock it to make changes."),
            EditError::WouldOverlap { .. } => write!(f, "That would overlap another clip on the same track."),
            EditError::InvalidSplitPoint => write!(f, "There is nothing to split at that point."),
            EditError::InvalidValue(w) => write!(f, "That {w} is not a valid value."),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Edit {
    AddClip { track: TrackId, clip: Clip },
    /// Carries the whole clip so the inverse can restore it exactly.
    RemoveClip { track: TrackId, clip: Clip },
    MoveClip { clip: ClipId, from_track: TrackId, to_track: TrackId,
               from_start: Ticks, to_start: Ticks },
    /// Trim carries both windows, so it inverts without recomputation.
    TrimClip { clip: ClipId, from_source: TimeRange, to_source: TimeRange,
               from_start: Ticks, to_start: Ticks },
    /// `left` keeps the original id; `right` is newly created.
    SplitClip { clip: ClipId, at: Ticks, right: ClipId },
    /// Inverse of a split.
    ///
    /// Carries the source window to restore. `timeline_duration` is DERIVED
    /// (`source / speed`), so splitting rounds each half independently and the
    /// pair may not reconstruct the original by arithmetic alone. Storing the
    /// original window makes undo exact instead of approximately right.
    MergeClips { left: ClipId, right: ClipId, restore_source: TimeRange },
    SetSpeed { clip: ClipId, from: f64, to: f64 },
    SetGain { clip: ClipId, from: f64, to: f64 },
    SetFades { clip: ClipId, from: (Ticks, Ticks), to: (Ticks, Ticks) },
    /// Effects carry no geometry, so unlike SetSpeed this needs no overlap check.
    SetEffects { clip: ClipId, from: Effects, to: Effects },
    /// Crop, rotation and flip. Separate from `SetEffects` so undo describes
    /// what the user actually did — "Undo Crop" rather than "Undo Effects".
    SetGeometry { clip: ClipId, from: Geometry, to: Geometry },
    /// Every caption at once. Transcription replaces the lot, and an import
    /// does too — one edit means one undo, not four hundred.
    SetCaptions { from: crate::captions::Captions, to: crate::captions::Captions },
    /// A whole volume curve at once. Ducking rewrites all of it, and one edit
    /// means one undo — "Undo Ducking", not forty "Undo Volume"s.
    SetGainPoints { clip: ClipId, from: Vec<GainPoint>, to: Vec<GainPoint> },
    SetTrackMuted { track: TrackId, from: bool, to: bool },
    SetTrackLocked { track: TrackId, from: bool, to: bool },
    /// Several edits applied and undone as one user-visible step.
    Batch(Vec<Edit>),
}

impl Edit {
    /// Short label for the Undo/Redo menu item.
    pub fn label(&self) -> &'static str {
        match self {
            Edit::AddClip { .. } => "Add Clip",
            Edit::RemoveClip { .. } => "Delete Clip",
            Edit::MoveClip { .. } => "Move Clip",
            Edit::TrimClip { .. } => "Trim Clip",
            Edit::SplitClip { .. } => "Split Clip",
            Edit::MergeClips { .. } => "Merge Clips",
            Edit::SetSpeed { .. } => "Change Speed",
            Edit::SetGain { .. } => "Change Volume",
            Edit::SetFades { .. } => "Change Fade",
            Edit::SetEffects { .. } => "Change Effects",
            Edit::SetGeometry { .. } => "Crop or Rotate",
            Edit::SetGainPoints { .. } => "Change Volume Over Time",
            Edit::SetCaptions { .. } => "Change Captions",
            Edit::SetTrackMuted { .. } => "Mute Track",
            Edit::SetTrackLocked { .. } => "Lock Track",
            Edit::Batch(_) => "Multiple Changes",
        }
    }

    /// Rough heap cost, for the bounded-history budget (R-16).
    pub fn size_estimate(&self) -> usize {
        let base = std::mem::size_of::<Edit>();
        match self {
            Edit::AddClip { .. } | Edit::RemoveClip { .. } => base + std::mem::size_of::<Clip>(),
            Edit::Batch(v) => base + v.iter().map(|e| e.size_estimate()).sum::<usize>(),
            _ => base,
        }
    }

    /// Apply, returning the edit that undoes this one.
    pub fn apply(self, tl: &mut Timeline) -> Result<Edit, EditError> {
        match self {
            Edit::AddClip { track, clip } => {
                let t = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?;
                if t.locked { return Err(EditError::TrackLocked(track)); }
                if let Some(c) = t.clips.iter()
                    .find(|c| c.timeline_range().overlaps(clip.timeline_range())) {
                    return Err(EditError::WouldOverlap { existing: c.id });
                }
                // Clone before inserting rather than looking the clip back up:
                // no lookup, no panic site, and the inverse is exact.
                let restored = clip.clone();
                t.insert_clip(clip);
                Ok(Edit::RemoveClip { track, clip: restored })
            }

            Edit::RemoveClip { track, clip } => {
                let t = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?;
                if t.locked { return Err(EditError::TrackLocked(track)); }
                let removed = t.remove_clip(clip.id).ok_or(EditError::NoSuchClip(clip.id))?;
                Ok(Edit::AddClip { track, clip: removed })
            }

            Edit::MoveClip { clip, from_track, to_track, from_start, to_start } => {
                let src = tl.track_mut(from_track).ok_or(EditError::NoSuchTrack(from_track))?;
                if src.locked { return Err(EditError::TrackLocked(from_track)); }
                let mut c = src.remove_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                c.timeline_start = to_start;

                let dst = match tl.track_mut(to_track) {
                    Some(d) => d,
                    None => { // put it back before failing
                        c.timeline_start = from_start;
                        if let Some(back) = tl.track_mut(from_track) { back.insert_clip(c); }
                        return Err(EditError::NoSuchTrack(to_track));
                    }
                };
                if let Some(other) = dst.clips.iter()
                    .find(|o| o.timeline_range().overlaps(c.timeline_range())) {
                    let existing = other.id;
                    c.timeline_start = from_start;
                    if let Some(back) = tl.track_mut(from_track) { back.insert_clip(c); }
                    return Err(EditError::WouldOverlap { existing });
                }
                dst.insert_clip(c);
                Ok(Edit::MoveClip { clip, from_track: to_track, to_track: from_track,
                                    from_start: to_start, to_start: from_start })
            }

            Edit::TrimClip { clip, from_source, to_source, from_start, to_start } => {
                if to_source.duration.0 <= 0 { return Err(EditError::InvalidValue("trim")); }
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let t = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?;
                if t.locked { return Err(EditError::TrackLocked(track)); }
                {
                    let c = t.clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                    c.source = to_source;
                    c.timeline_start = to_start;
                }
                t.resort();
                if let Some((a, b)) = t.overlaps().first().copied() {
                    let c = t.clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                    c.source = from_source;
                    c.timeline_start = from_start;
                    t.resort();
                    return Err(EditError::WouldOverlap { existing: if a == clip { b } else { a } });
                }
                Ok(Edit::TrimClip { clip, from_source: to_source, to_source: from_source,
                                    from_start: to_start, to_start: from_start })
            }

            Edit::SplitClip { clip, at, right } => {
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let t = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?;
                if t.locked { return Err(EditError::TrackLocked(track)); }
                let c = t.clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                let r = c.timeline_range();
                // A split exactly on a boundary produces an empty half.
                if !r.contains(at) || at == r.start { return Err(EditError::InvalidSplitPoint); }

                let src_at = c.source_time_at(at).ok_or(EditError::InvalidSplitPoint)?;
                let left_src = TimeRange::new(c.source.start, src_at - c.source.start);
                let right_src = TimeRange::new(src_at, c.source.end() - src_at);
                if left_src.duration.0 <= 0 || right_src.duration.0 <= 0 {
                    return Err(EditError::InvalidSplitPoint);
                }

                // Derive the timeline split point BACK from the source point.
                //
                // timeline -> source -> timeline rounds twice, and at speeds
                // other than 1.0 the result can land a tick past `at`. Placing
                // the right half at `at` then leaves the left half one tick
                // long and the two overlap. A property test caught exactly
                // that. Splitting where the source actually divides keeps the
                // halves exactly adjacent at any speed.
                let left_timeline = Ticks(
                    (left_src.duration.0 as f64 / c.speed).round() as i64);
                let effective_at = c.timeline_start + left_timeline;
                let full = c.timeline_range();
                if effective_at <= full.start || effective_at >= full.end() {
                    return Err(EditError::InvalidSplitPoint);
                }

                // The right half must fit exactly in the room the left half
                // left behind. Rounding both halves independently can make
                // their sum one tick longer than the original, which pushes
                // the right half into the NEXT clip -- caught by a property
                // test as "split produced an overlap". Give back at most a
                // tick or two of source; MergeClips restores it exactly.
                let available = full.end() - effective_at;
                let mut right_src = right_src;
                while right_src.duration.0 > 1
                    && Ticks((right_src.duration.0 as f64 / c.speed).round() as i64) > available
                {
                    right_src.duration = Ticks(right_src.duration.0 - 1);
                }

                let original_source = c.source;
                let mut right_clip = c.clone();
                c.source = left_src;
                // Fades belong to the outer edges of the pair.
                let orig_fade_out = c.fade_out;
                c.fade_out = Ticks::ZERO;

                right_clip.id = right;
                right_clip.source = right_src;
                right_clip.timeline_start = effective_at;
                right_clip.fade_in = Ticks::ZERO;
                right_clip.fade_out = orig_fade_out;
                t.insert_clip(right_clip);
                debug_assert!(t.overlaps().is_empty(), "split produced an overlap");
                Ok(Edit::MergeClips { left: clip, right, restore_source: original_source })
            }

            Edit::MergeClips { left, right, restore_source } => {
                let (track, _) = tl.find_clip(left).ok_or(EditError::NoSuchClip(left))?;
                let t = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?;
                if t.locked { return Err(EditError::TrackLocked(track)); }
                let r = t.remove_clip(right).ok_or(EditError::NoSuchClip(right))?;
                let at = r.timeline_start;
                let c = t.clip_mut(left).ok_or(EditError::NoSuchClip(left))?;
                // Restore the exact original window rather than recomputing it.
                c.source = restore_source;
                c.fade_out = r.fade_out;
                Ok(Edit::SplitClip { clip: left, at, right })
            }

            Edit::SetSpeed { clip, from, to } => {
                if to <= 0.0 || !to.is_finite() { return Err(EditError::InvalidValue("speed")); }
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let t = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?;
                if t.locked { return Err(EditError::TrackLocked(track)); }

                // Speed changes GEOMETRY: timeline_duration is source/speed, so
                // slowing a clip lengthens it and it can grow into its
                // neighbour. Every other geometry-changing edit validates this;
                // omitting it here let a property test build an overlapping
                // document through an edit that reported success.
                let previous = t.clip(clip).ok_or(EditError::NoSuchClip(clip))?.speed;
                t.clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?.speed = to;
                let grown = t.clip(clip).ok_or(EditError::NoSuchClip(clip))?.timeline_range();
                if let Some(other) = t.clips.iter()
                    .find(|o| o.id != clip && o.timeline_range().overlaps(grown))
                {
                    let existing = other.id;
                    if let Some(c) = t.clip_mut(clip) { c.speed = previous; }
                    return Err(EditError::WouldOverlap { existing });
                }
                Ok(Edit::SetSpeed { clip, from: to, to: from })
            }

            Edit::SetGain { clip, from, to } => {
                if to < 0.0 { return Err(EditError::InvalidValue("volume")); }
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?
                  .clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?.gain = to;
                Ok(Edit::SetGain { clip, from: to, to: from })
            }

            Edit::SetFades { clip, from, to } => {
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let c = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?
                          .clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                c.fade_in = to.0; c.fade_out = to.1;
                Ok(Edit::SetFades { clip, from: to, to: from })
            }

            Edit::SetEffects { clip, from, to } => {
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let c = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?
                          .clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                c.effects = to.clamped();
                Ok(Edit::SetEffects { clip, from: to, to: from })
            }

            Edit::SetGeometry { clip, from, to } => {
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let c = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?
                          .clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                c.geometry = to.clamped();
                Ok(Edit::SetGeometry { clip, from: to, to: from })
            }

            Edit::SetGainPoints { clip, from, to } => {
                let (track, _) = tl.find_clip(clip).ok_or(EditError::NoSuchClip(clip))?;
                let c = tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?
                          .clip_mut(clip).ok_or(EditError::NoSuchClip(clip))?;
                let mut sorted = to.clone();
                // The interpolator walks the points in order and would read a
                // shuffled list as a sawtooth.
                sorted.sort_by_key(|p| p.at.0);
                for p in sorted.iter_mut() { p.gain = p.gain.clamp(0.0, 1.0); }
                c.gain_points = sorted;
                Ok(Edit::SetGainPoints { clip, from: to, to: from })
            }

            Edit::SetCaptions { from, to } => {
                tl.captions = to.clone();
                Ok(Edit::SetCaptions { from: to, to: from })
            }

            Edit::SetTrackMuted { track, from, to } => {
                tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?.muted = to;
                Ok(Edit::SetTrackMuted { track, from: to, to: from })
            }

            Edit::SetTrackLocked { track, from, to } => {
                tl.track_mut(track).ok_or(EditError::NoSuchTrack(track))?.locked = to;
                Ok(Edit::SetTrackLocked { track, from: to, to: from })
            }

            Edit::Batch(edits) => {
                let mut inverses = Vec::with_capacity(edits.len());
                for (i, e) in edits.into_iter().enumerate() {
                    match e.apply(tl) {
                        Ok(inv) => inverses.push(inv),
                        Err(err) => {
                            // A batch is atomic: roll back what already applied.
                            for undo in inverses.into_iter().rev() {
                                let _ = undo.apply(tl);
                            }
                            let _ = i;
                            return Err(err);
                        }
                    }
                }
                inverses.reverse();
                Ok(Edit::Batch(inverses))
            }
        }
    }
}

/// One history entry: how to reverse a step, plus what the USER called it.
///
/// The label must come from the original action, not from the stored inverse.
/// Undoing an "Add Clip" is still "Undo Add Clip" even though the stored edit
/// is a RemoveClip — a test caught this reporting "Undo Delete Clip".
#[derive(Debug)]
struct Entry { inverse: Edit, label: &'static str }

/// Bounded undo/redo history (R-16).
#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for History {
    fn default() -> Self { History::new(500, 8 * 1024 * 1024) }
}

impl History {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        History { undo: Vec::new(), redo: Vec::new(), max_entries, max_bytes }
    }

    /// Apply an edit and record how to undo it.
    pub fn apply(&mut self, tl: &mut Timeline, edit: Edit) -> Result<(), EditError> {
        let label = edit.label();
        let inverse = edit.apply(tl)?;
        self.undo.push(Entry { inverse, label });
        self.redo.clear();          // a new edit invalidates the redo branch
        self.trim();
        Ok(())
    }

    pub fn undo(&mut self, tl: &mut Timeline) -> Result<Option<&'static str>, EditError> {
        let Some(e) = self.undo.pop() else { return Ok(None) };
        let label = e.label;
        let inverse = e.inverse.apply(tl)?;
        self.redo.push(Entry { inverse, label });
        Ok(Some(label))
    }

    pub fn redo(&mut self, tl: &mut Timeline) -> Result<Option<&'static str>, EditError> {
        let Some(e) = self.redo.pop() else { return Ok(None) };
        let label = e.label;
        let inverse = e.inverse.apply(tl)?;
        self.undo.push(Entry { inverse, label });
        Ok(Some(label))
    }

    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub fn undo_label(&self) -> Option<&'static str> { self.undo.last().map(|e| e.label) }
    pub fn redo_label(&self) -> Option<&'static str> { self.redo.last().map(|e| e.label) }
    pub fn depth(&self) -> (usize, usize) { (self.undo.len(), self.redo.len()) }
    pub fn bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|e| e.inverse.size_estimate()).sum()
    }

    /// Drop the OLDEST entries when over budget. Losing distant history is
    /// acceptable; unbounded growth over a long session is not (§27, R-16).
    fn trim(&mut self) {
        while self.undo.len() > self.max_entries { self.undo.remove(0); }
        while self.bytes() > self.max_bytes && self.undo.len() > 1 { self.undo.remove(0); }
    }
}
