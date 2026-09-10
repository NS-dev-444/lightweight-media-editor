//! Transient editing state: playhead, in/out points, selection.
//!
//! Deliberately NOT part of `Project` and never serialised. Where the cursor
//! sits is not a property of the document, and persisting it would mean every
//! click marks the project dirty and triggers an autosave.
//!
//! §15 (playhead, timecode, multi-select) and §29 (I/O points, arrow-key
//! navigation) both live here.

use crate::ops;
use crate::time::{FrameRate, Ticks, TimeRange, Timecode};
use crate::timeline::{ClipId, Timeline, TrackId};
use std::collections::BTreeSet;

/// Where playback and edits are anchored.
#[derive(Clone, Debug, Default)]
pub struct Playhead {
    position: Ticks,
    /// §29: `I` and `O`. Either may be set without the other.
    pub in_point: Option<Ticks>,
    pub out_point: Option<Ticks>,
}

impl Playhead {
    pub fn position(&self) -> Ticks { self.position }

    /// Never negative — a playhead before zero is not a meaningful state.
    pub fn set(&mut self, t: Ticks) { self.position = t.max(Ticks::ZERO); }

    /// Step by whole frames (§29 arrow keys). Frame-quantised so repeated
    /// stepping cannot accumulate sub-frame drift.
    pub fn step_frames(&mut self, frames: i64, rate: FrameRate) {
        let fd = rate.frame_duration().0.max(1);
        let current = self.position.0 / fd;
        self.set(Ticks((current + frames).max(0) * fd));
    }

    /// Snap to the exact frame boundary at or before the current position.
    pub fn quantise(&mut self, rate: FrameRate) {
        let fd = rate.frame_duration().0.max(1);
        self.position = Ticks(self.position.0 / fd * fd);
    }

    pub fn timecode(&self, rate: FrameRate) -> Timecode { Timecode::from_ticks(self.position, rate) }

    /// The in/out span, if both are set and ordered. This is what "export
    /// range" and "delete range" operate on.
    pub fn marked_range(&self) -> Option<TimeRange> {
        match (self.in_point, self.out_point) {
            (Some(i), Some(o)) if o > i => Some(TimeRange::new(i, o - i)),
            _ => None,
        }
    }
    pub fn clear_marks(&mut self) { self.in_point = None; self.out_point = None; }
}

/// Clip selection (§15 multi-select).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    clips: BTreeSet<ClipId>,
}

impl Selection {
    pub fn is_empty(&self) -> bool { self.clips.is_empty() }
    pub fn len(&self) -> usize { self.clips.len() }
    pub fn contains(&self, id: ClipId) -> bool { self.clips.contains(&id) }
    pub fn iter(&self) -> impl Iterator<Item = ClipId> + '_ { self.clips.iter().copied() }
    pub fn as_vec(&self) -> Vec<ClipId> { self.clips.iter().copied().collect() }

    pub fn clear(&mut self) { self.clips.clear(); }
    /// Plain click: replaces the selection.
    pub fn select_only(&mut self, id: ClipId) { self.clips.clear(); self.clips.insert(id); }
    /// Cmd/Ctrl-click: add without disturbing the rest.
    pub fn add(&mut self, id: ClipId) { self.clips.insert(id); }
    pub fn remove(&mut self, id: ClipId) { self.clips.remove(&id); }
    pub fn toggle(&mut self, id: ClipId) {
        if !self.clips.remove(&id) { self.clips.insert(id); }
    }

    /// Everything intersecting a time range — a marquee drag, or Select All
    /// within the marked range.
    pub fn select_in_range(&mut self, tl: &Timeline, range: TimeRange, tracks: Option<&[TrackId]>) {
        self.clips.clear();
        for t in &tl.tracks {
            if tracks.is_some_and(|ids| !ids.contains(&t.id)) { continue; }
            for c in &t.clips {
                if c.timeline_range().overlaps(range) { self.clips.insert(c.id); }
            }
        }
    }

    pub fn select_all(&mut self, tl: &Timeline) {
        self.clips = tl.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id)).collect();
    }

    /// Drop ids that no longer exist. Must be called after any edit that can
    /// remove clips, or the selection silently references deleted material —
    /// which then makes the next operation fail for no visible reason.
    pub fn prune(&mut self, tl: &Timeline) {
        self.clips.retain(|id| tl.find_clip(*id).is_some());
    }
}

/// Timeline positions worth jumping to: clip starts and ends, plus zero.
///
/// §29 navigation. Distinct from snapping — this ignores the playhead itself.
pub fn edit_points(tl: &Timeline, track: Option<TrackId>) -> Vec<Ticks> {
    let mut v = vec![Ticks::ZERO];
    for t in &tl.tracks {
        if track.is_some_and(|id| id != t.id) { continue; }
        for c in &t.clips {
            let r = c.timeline_range();
            v.push(r.start);
            v.push(r.end());
        }
    }
    v.sort();
    v.dedup();
    v
}

pub fn next_edit_point(tl: &Timeline, from: Ticks, track: Option<TrackId>) -> Option<Ticks> {
    edit_points(tl, track).into_iter().find(|&t| t > from)
}

pub fn prev_edit_point(tl: &Timeline, from: Ticks, track: Option<TrackId>) -> Option<Ticks> {
    edit_points(tl, track).into_iter().rev().find(|&t| t < from)
}

/// The editing surface the UI drives: a timeline view plus where the user is.
#[derive(Clone, Debug, Default)]
pub struct EditorState {
    pub playhead: Playhead,
    pub selection: Selection,
    pub active_track: Option<TrackId>,
    /// Snap threshold in ticks; zero disables snapping (§15 snap toggle).
    pub snap_threshold: Ticks,
}

impl EditorState {
    pub fn new(rate: FrameRate) -> Self {
        EditorState {
            playhead: Playhead::default(),
            selection: Selection::default(),
            active_track: None,
            // Half a frame: close enough to feel magnetic, small enough that a
            // deliberate placement is never overridden.
            snap_threshold: Ticks(rate.frame_duration().0 / 2),
        }
    }

    /// Snap a candidate time to nearby edges, honouring the snap toggle and
    /// ignoring the clip currently being dragged.
    pub fn snap(&self, tl: &Timeline, t: Ticks, dragging: Option<ClipId>) -> Ticks {
        if self.snap_threshold.0 <= 0 { return t; }
        let targets = ops::snap_targets(tl, dragging, Some(self.playhead.position()));
        ops::snap(t, &targets, self.snap_threshold)
    }

    /// Keep transient state coherent after the document changes.
    pub fn reconcile(&mut self, tl: &Timeline) {
        self.selection.prune(tl);
        if self.active_track.is_some_and(|id| tl.track(id).is_none()) {
            self.active_track = None;
        }
    }
}
