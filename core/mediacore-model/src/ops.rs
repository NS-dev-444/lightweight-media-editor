//! Higher-level edit operations.
//!
//! `command::Edit` holds *primitives* — each one small, invertible and tested.
//! This module composes them into the operations the UI actually offers
//! (§15: ripple delete, ripple trim, split at playhead, close gap).
//!
//! Composites are built as `Edit::Batch`, deliberately. A batch is atomic and
//! inverts by inverting its parts in reverse, so a ripple delete gets correct
//! undo for free rather than needing its own hand-written inverse — which is
//! where this class of bug normally lives.

use crate::command::{Edit, EditError};
use crate::time::{Ticks, TimeRange};
use crate::timeline::{Clip, ClipId, Timeline, TrackId};

/// Times a dragged edge should prefer to land on (§15 snapping).
///
/// Ordered and de-duplicated so the nearest-target search is predictable.
pub fn snap_targets(tl: &Timeline, exclude: Option<ClipId>, playhead: Option<Ticks>) -> Vec<Ticks> {
    let mut t = vec![Ticks::ZERO];
    for track in &tl.tracks {
        for c in &track.clips {
            if Some(c.id) == exclude { continue; }
            let r = c.timeline_range();
            t.push(r.start);
            t.push(r.end());
        }
    }
    if let Some(p) = playhead { t.push(p); }
    t.sort();
    t.dedup();
    t
}

/// Snap `t` to the nearest target within `threshold`, else leave it alone.
pub fn snap(t: Ticks, targets: &[Ticks], threshold: Ticks) -> Ticks {
    let mut best: Option<(i64, Ticks)> = None;
    for &c in targets {
        let d = (c.0 - t.0).abs();
        if d <= threshold.0 && best.is_none_or(|(bd, _)| d < bd) { best = Some((d, c)); }
    }
    best.map(|(_, c)| c).unwrap_or(t)
}

/// Delete a clip and pull everything after it on that track backwards, so no
/// gap is left (§2 "ripple delete where practical").
pub fn ripple_delete(tl: &Timeline, track: TrackId, clip: ClipId) -> Result<Edit, EditError> {
    let t = tl.track(track).ok_or(EditError::NoSuchTrack(track))?;
    let target = t.clip(clip).ok_or(EditError::NoSuchClip(clip))?;
    let gap = target.timeline_duration();
    let after = target.timeline_range().end();

    let mut edits = vec![Edit::RemoveClip { track, clip: target.clone() }];
    // Later clips shift left by exactly the removed duration. Applying in
    // ascending order keeps each intermediate state overlap-free, which the
    // primitives check on every step.
    for c in t.clips.iter().filter(|c| c.timeline_start >= after) {
        edits.push(Edit::MoveClip {
            clip: c.id, from_track: track, to_track: track,
            from_start: c.timeline_start,
            to_start: c.timeline_start.saturating_sub(gap),
        });
    }
    Ok(Edit::Batch(edits))
}

/// Remove a time RANGE from a track: trim or delete whatever it covers, then
/// close the hole. This is the "cut the dead air out" operation that
/// PRODUCT_DIRECTION.md workflow W1 is built around.
/// `new_id` is used only when the range lands strictly inside a clip, which is
/// the case that has to SPLIT it. Allocating an id that goes unused otherwise
/// is cheaper than teaching this function about the project's id counter.
pub fn ripple_delete_range(tl: &Timeline, track: TrackId, range: TimeRange,
                           new_id: ClipId)
    -> Result<Edit, EditError>
{
    if range.is_empty() { return Err(EditError::InvalidValue("range")); }
    let t = tl.track(track).ok_or(EditError::NoSuchTrack(track))?;
    let mut edits = Vec::new();

    for c in &t.clips {
        let cr = c.timeline_range();
        if !cr.overlaps(range) { continue; }
        if range.start <= cr.start && range.end() >= cr.end() {
            // Fully covered — delete it.
            edits.push(Edit::RemoveClip { track, clip: c.clone() });
        } else if range.start > cr.start && range.end() < cr.end() {
            // Range sits strictly inside: keep the head AND the tail, with the
            // tail pulled back to where the range began.
            //
            // This used to keep only the head and throw the tail away, which is
            // wrong for the operation this function exists to serve: cutting a
            // gap out of the middle of one long take must close the gap, not
            // truncate the take. Found by removing a two-second silence from a
            // four-second file and getting a 1.1-second one back.
            let keep = range.start - cr.start;
            let src_keep = Ticks((keep.0 as f64 * c.speed).round() as i64);
            edits.push(Edit::TrimClip {
                clip: c.id,
                from_source: c.source,
                to_source: TimeRange::new(c.source.start, src_keep),
                from_start: c.timeline_start, to_start: c.timeline_start,
            });

            let drop = range.end() - cr.start;
            let src_drop = Ticks((drop.0 as f64 * c.speed).round() as i64);
            let mut tail = c.clone();
            tail.id = new_id;
            tail.timeline_start = range.start;
            tail.source = TimeRange::new(c.source.start + src_drop,
                                         c.source.duration.saturating_sub(src_drop));
            // A fade belongs to the end it was applied to: the head keeps the
            // fade in, the tail keeps the fade out. Copying both to both would
            // add two fades in the middle of a continuous take.
            tail.fade_in = Ticks(0);
            if !tail.source.is_empty() { edits.push(Edit::AddClip { track, clip: tail }); }
            // The head's fade out would now land at the cut, so it goes.
            if c.fade_out.0 > 0 {
                edits.push(Edit::SetFades { clip: c.id,
                                            from: (c.fade_in, c.fade_out),
                                            to: (c.fade_in, Ticks(0)) });
            }
        } else if range.start <= cr.start {
            // Overlaps the head: drop the covered front.
            //
            // The remainder used to begin at range.end(); once the range is
            // removed, that content sits at range.start. Leaving it at
            // range.end() would keep the hole open AND collide with the clips
            // being pulled left below — which is exactly how this first failed.
            let drop = range.end() - cr.start;
            let src_drop = Ticks((drop.0 as f64 * c.speed).round() as i64);
            edits.push(Edit::TrimClip {
                clip: c.id,
                from_source: c.source,
                to_source: TimeRange::new(c.source.start + src_drop,
                                          c.source.duration.saturating_sub(src_drop)),
                from_start: c.timeline_start, to_start: range.start,
            });
        } else {
            // Overlaps the tail: drop the covered back.
            let keep = range.start - cr.start;
            let src_keep = Ticks((keep.0 as f64 * c.speed).round() as i64);
            edits.push(Edit::TrimClip {
                clip: c.id,
                from_source: c.source,
                to_source: TimeRange::new(c.source.start, src_keep),
                from_start: c.timeline_start, to_start: c.timeline_start,
            });
        }
    }

    // Close the hole: everything starting at or after the range moves left.
    for c in t.clips.iter().filter(|c| c.timeline_start >= range.end()) {
        edits.push(Edit::MoveClip {
            clip: c.id, from_track: track, to_track: track,
            from_start: c.timeline_start,
            to_start: c.timeline_start.saturating_sub(range.duration),
        });
    }

    if edits.is_empty() { return Err(EditError::InvalidValue("range")); }
    Ok(Edit::Batch(edits))
}

/// Drag a clip's RIGHT edge, shifting everything downstream (§15 ripple trim).
///
/// The clip's timeline START stays anchored; its duration changes and later
/// clips move by the same delta, so the gap after the clip is preserved.
pub fn ripple_trim_out(tl: &Timeline, track: TrackId, clip: ClipId, new_end: Ticks)
    -> Result<Edit, EditError>
{
    let t = tl.track(track).ok_or(EditError::NoSuchTrack(track))?;
    let c = t.clip(clip).ok_or(EditError::NoSuchClip(clip))?;
    let r = c.timeline_range();
    let delta = new_end - r.end();
    if delta.0 == 0 { return Err(EditError::InvalidValue("trim")); }

    let new_timeline_dur = new_end - r.start;
    if new_timeline_dur.0 <= 0 { return Err(EditError::InvalidValue("trim")); }
    let new_src_dur = Ticks((new_timeline_dur.0 as f64 * c.speed).round() as i64);
    if new_src_dur.0 <= 0 { return Err(EditError::InvalidValue("trim")); }

    let trim = Edit::TrimClip {
        clip, from_source: c.source,
        to_source: TimeRange::new(c.source.start, new_src_dur),
        from_start: c.timeline_start, to_start: c.timeline_start,
    };
    Ok(Edit::Batch(sequence(t, r.end(), delta, trim, track)))
}

/// Drag a clip's LEFT edge, shifting everything downstream.
///
/// A ripple trim anchors the clip's left edge on the TIMELINE: dragging the
/// in-point removes material from the head, the clip gets shorter in place,
/// and everything after it moves left. The gap BEFORE the clip is unchanged,
/// which is what makes it a ripple rather than a slip.
pub fn ripple_trim_in(tl: &Timeline, track: TrackId, clip: ClipId, new_start: Ticks)
    -> Result<Edit, EditError>
{
    let t = tl.track(track).ok_or(EditError::NoSuchTrack(track))?;
    let c = t.clip(clip).ok_or(EditError::NoSuchClip(clip))?;
    let r = c.timeline_range();
    let head = new_start - r.start;          // positive = remove from the head
    if head.0 == 0 { return Err(EditError::InvalidValue("trim")); }

    let new_timeline_dur = r.duration.saturating_sub(head);
    if new_timeline_dur.0 <= 0 { return Err(EditError::InvalidValue("trim")); }
    let src_shift = Ticks((head.0 as f64 * c.speed).round() as i64);
    let new_src_start = c.source.start + src_shift;
    let new_src_dur = c.source.duration.saturating_sub(src_shift);
    // Cannot trim past the beginning of the source material.
    if new_src_start < Ticks::ZERO || new_src_dur.0 <= 0 {
        return Err(EditError::InvalidValue("trim"));
    }

    let trim = Edit::TrimClip {
        clip, from_source: c.source,
        to_source: TimeRange::new(new_src_start, new_src_dur),
        from_start: c.timeline_start, to_start: c.timeline_start,
    };
    // Removing `head` shortens the clip, so downstream moves by -head.
    Ok(Edit::Batch(sequence(t, r.end(), Ticks(-head.0), trim, track)))
}

/// Order a trim and the downstream moves so no INTERMEDIATE state overlaps.
///
/// The primitives validate on every step, so ordering is not cosmetic:
///   * growing  -> move neighbours out FIRST (rightmost first), then trim
///   * shrinking -> trim first, then pull neighbours in (leftmost first)
/// Getting this backwards makes a legitimate edit fail with "would overlap",
/// which is exactly how this first failed.
fn sequence(t: &crate::timeline::Track, after: Ticks, delta: Ticks,
            trim: Edit, track: TrackId) -> Vec<Edit>
{
    let mut later: Vec<&Clip> = t.clips.iter().filter(|o| o.timeline_start >= after).collect();
    let growing = delta.0 > 0;
    if growing { later.reverse(); }
    let moves = later.into_iter().map(|o| Edit::MoveClip {
        clip: o.id, from_track: track, to_track: track,
        from_start: o.timeline_start, to_start: o.timeline_start + delta,
    });

    let mut edits = Vec::new();
    if growing {
        edits.extend(moves);
        edits.push(trim);
    } else {
        edits.push(trim);
        edits.extend(moves);
    }
    edits
}

/// Split whatever clip sits under the playhead (§29: the `S` shortcut).
pub fn split_at(tl: &Timeline, track: TrackId, at: Ticks, new_id: ClipId)
    -> Result<Edit, EditError>
{
    let t = tl.track(track).ok_or(EditError::NoSuchTrack(track))?;
    let c = t.clip_at(at).ok_or(EditError::InvalidSplitPoint)?;
    Ok(Edit::SplitClip { clip: c.id, at, right: new_id })
}

/// Pull later clips back to close a gap starting at `from`.
pub fn close_gap(tl: &Timeline, track: TrackId, from: Ticks) -> Result<Edit, EditError> {
    let t = tl.track(track).ok_or(EditError::NoSuchTrack(track))?;
    let next = t.clips.iter()
        .filter(|c| c.timeline_start > from)
        .min_by_key(|c| c.timeline_start)
        .ok_or(EditError::InvalidValue("gap"))?;
    let gap = next.timeline_start - from;
    if gap.0 <= 0 { return Err(EditError::InvalidValue("gap")); }

    let edits: Vec<Edit> = t.clips.iter()
        .filter(|c| c.timeline_start >= next.timeline_start)
        .map(|c| Edit::MoveClip {
            clip: c.id, from_track: track, to_track: track,
            from_start: c.timeline_start,
            to_start: c.timeline_start.saturating_sub(gap),
        })
        .collect();
    Ok(Edit::Batch(edits))
}

/// Gaps on a track, in order. Used for "close all gaps" and for UI hinting.
pub fn gaps(tl: &Timeline, track: TrackId) -> Vec<TimeRange> {
    let Some(t) = tl.track(track) else { return Vec::new() };
    let mut out = Vec::new();
    let mut cursor = Ticks::ZERO;
    for c in &t.clips {
        let r = c.timeline_range();
        if r.start > cursor { out.push(TimeRange::new(cursor, r.start - cursor)); }
        cursor = cursor.max(r.end());
    }
    out
}
