//! Captions (§8, and PRODUCT_DIRECTION.md §6's "Publishing").
//!
//! PRODUCT_DIRECTION calls subtitles "the single most-wanted feature by anyone
//! publishing video", and §32's deferral of them "the most costly deferral in
//! the spec". They are in V1.
//!
//! ## Shape
//!
//! A caption is a **span of timeline time with words on it** — not a clip. It
//! deliberately does not live on the overlay track:
//!
//!   * a talk can carry hundreds of them, and hundreds of clips would make the
//!     timeline unusable for the editing the timeline is actually for;
//!   * they are exported as a **sidecar file** as often as they are burned in,
//!     and a sidecar is a property of the document, not of a track;
//!   * they are edited as *text*, in a list, which is how everyone who has ever
//!     corrected a transcript expects to work.
//!
//! They still become ordinary text layers at render time, so burning them in
//! reuses the whole existing text pipeline rather than adding a second one.
//!
//! ## Timing
//!
//! Ticks, like everything else (§R11). SRT's millisecond resolution and WebVTT's
//! are both *coarser* than the tick grid, so round-tripping through a file
//! loses precision — which is a property of those formats, not a defect here,
//! and is why the project keeps its own copy rather than treating an exported
//! .srt as the truth.

use crate::time::{Ticks, TimeRange};
use serde::{Deserialize, Serialize};

/// One caption: a span of time and the words shown over it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Caption {
    pub start: Ticks,
    pub end: Ticks,
    pub text: String,
}

impl Caption {
    pub fn new(start: Ticks, end: Ticks, text: impl Into<String>) -> Self {
        Caption { start, end, text: text.into() }
    }

    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, Ticks((self.end.0 - self.start.0).max(0)))
    }

    pub fn contains(&self, t: Ticks) -> bool { t >= self.start && t < self.end }
}

/// All of a project's captions, kept in order of `start`.
///
/// Ordering is an invariant rather than a convention: lookup during playback is
/// a binary search, and an out-of-order list would silently show the wrong line.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Captions {
    items: Vec<Caption>,
    /// Whether captions are drawn into the exported picture. Off by default: a
    /// sidecar file can be turned off by the viewer and burned-in text cannot,
    /// so burning in is a choice the user should make deliberately.
    pub burn_in: bool,
    /// Which text preset the burned-in captions use. `Subtitle` exists for
    /// exactly this and is outlined for legibility over any footage.
    pub preset: u8,
}

impl Captions {
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn as_slice(&self) -> &[Caption] { &self.items }

    /// Replace every caption. Used by transcription and by importing a file.
    pub fn set(&mut self, mut items: Vec<Caption>) {
        items.retain(|c| c.end > c.start && !c.text.trim().is_empty());
        items.sort_by_key(|c| (c.start.0, c.end.0));
        self.items = items;
    }

    /// The caption to show at an instant, if any.
    ///
    /// Binary search on start, then walk back over any that share it. Captions
    /// can overlap when a speaker is interrupted, and the LAST one that
    /// contains the instant wins — that is the most recently spoken line, which
    /// is the one a viewer expects to be reading.
    pub fn at(&self, t: Ticks) -> Option<&Caption> {
        let i = self.items.partition_point(|c| c.start <= t);
        self.items[..i].iter().rev().find(|c| c.contains(t))
    }

    /// Shift every caption in time — the fix for a transcript that is
    /// consistently early or late, which is the usual complaint.
    pub fn shift(&mut self, by: Ticks) {
        for c in self.items.iter_mut() {
            c.start = Ticks((c.start.0 + by.0).max(0));
            c.end = Ticks((c.end.0 + by.0).max(0));
        }
        self.items.sort_by_key(|c| (c.start.0, c.end.0));
    }

    /// Edit one caption's words in place.
    pub fn set_text(&mut self, index: usize, text: impl Into<String>) -> bool {
        match self.items.get_mut(index) {
            Some(c) => { c.text = text.into(); true }
            None => false,
        }
    }

    /// Retime one caption. Returns false when the span is empty, which would
    /// otherwise create a caption that can never be shown.
    pub fn set_range(&mut self, index: usize, start: Ticks, end: Ticks) -> bool {
        if end <= start || index >= self.items.len() { return false; }
        self.items[index].start = start;
        self.items[index].end = end;
        self.items.sort_by_key(|c| (c.start.0, c.end.0));
        true
    }

    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.items.len() { return false; }
        self.items.remove(index);
        true
    }

    /// Total time covered, ignoring any overlap.
    pub fn spoken_duration(&self) -> Ticks {
        Ticks(self.items.iter().map(|c| (c.end.0 - c.start.0).max(0)).sum())
    }
}

// ===========================================================================
// SRT and WebVTT
//
// Both directions matter, and for different reasons.
//
// **Writing** is how captions leave the product: a sidecar file is what YouTube,
// Vimeo and every player want, and it is the form the viewer can switch off.
//
// **Reading** is how captions get in without transcription at all. Plenty of
// people already have a transcript — from a client, from a previous tool, from
// a platform's own auto-captions — and making them re-transcribe it would be
// absurd. It also means the whole caption feature is testable and usable
// without the speech engine.
// ===========================================================================

/// Render a timestamp as `HH:MM:SS,mmm` (SRT) or `HH:MM:SS.mmm` (WebVTT).
fn format_timestamp(t: Ticks, decimal: char) -> String {
    let total_ms = (t.0.max(0) as i128 * 1000 / crate::time::TICKS_PER_SECOND as i128) as i64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    format!("{:02}:{:02}:{:02}{}{:03}",
            total_s / 3600, (total_s % 3600) / 60, total_s % 60, decimal, ms)
}

/// Parse `HH:MM:SS,mmm`, `HH:MM:SS.mmm` or `MM:SS.mmm`.
///
/// WebVTT allows the hour to be omitted and both formats appear with either
/// separator in the wild, so this accepts all of them rather than being strict
/// about a distinction no producer respects.
fn parse_timestamp(s: &str) -> Option<Ticks> {
    let s = s.trim();
    let (time, frac) = match s.rsplit_once([',', '.']) {
        Some((t, f)) if f.chars().all(|c| c.is_ascii_digit()) && !f.is_empty() => (t, f),
        _ => (s, ""),
    };
    let parts: Vec<&str> = time.split(':').collect();
    let (h, m, sec) = match parts.len() {
        3 => (parts[0].trim().parse::<i64>().ok()?, parts[1].parse::<i64>().ok()?,
              parts[2].parse::<i64>().ok()?),
        2 => (0, parts[0].trim().parse::<i64>().ok()?, parts[1].parse::<i64>().ok()?),
        _ => return None,
    };
    if m > 59 || sec > 59 { return None; }
    // Milliseconds, but tolerate 1-3 digits: "1.5" means 500 ms, not 5.
    let ms = if frac.is_empty() { 0 } else {
        let v: i64 = frac.parse().ok()?;
        match frac.len() { 1 => v * 100, 2 => v * 10, 3 => v, _ => v / 10i64.pow(frac.len() as u32 - 3) }
    };
    let total_ms = ((h * 60 + m) * 60 + sec) * 1000 + ms;
    Some(Ticks((total_ms as i128 * crate::time::TICKS_PER_SECOND as i128 / 1000) as i64))
}

impl Captions {
    /// SubRip (.srt).
    pub fn to_srt(&self) -> String {
        let mut out = String::new();
        for (i, c) in self.items.iter().enumerate() {
            out.push_str(&format!("{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_timestamp(c.start, ','),
                format_timestamp(c.end, ','),
                c.text.trim()));
        }
        out
    }

    /// WebVTT (.vtt). Same content, different header and separator.
    pub fn to_vtt(&self) -> String {
        let mut out = String::from("WEBVTT\n\n");
        for c in self.items.iter() {
            out.push_str(&format!("{} --> {}\n{}\n\n",
                format_timestamp(c.start, '.'),
                format_timestamp(c.end, '.'),
                c.text.trim()));
        }
        out
    }

    /// Read SRT or WebVTT — the same parser handles both.
    ///
    /// Forgiving on purpose. Caption files are produced by an enormous variety
    /// of tools and hand-edited by people; refusing a file over a stray blank
    /// line or a missing sequence number would fail on real input constantly.
    /// Cues that cannot be understood are **skipped, not fatal**: recovering
    /// ninety-eight lines of a transcript beats rejecting all hundred.
    pub fn parse_subtitles(text: &str) -> Vec<Caption> {
        let mut items = Vec::new();
        let mut pending: Option<(Ticks, Ticks)> = None;
        let mut lines: Vec<String> = Vec::new();

        // A BOM in front of "WEBVTT" or "1" is common enough to be worth
        // handling; without this the first cue is lost every time.
        let text = text.trim_start_matches('\u{feff}');

        // `at_timing` says the cue was ended by the NEXT cue's timing line
        // rather than by a blank line, which means the file omitted the
        // separator — and that the last line collected is almost certainly the
        // next cue's sequence NUMBER rather than words. Without this, "One"
        // comes back as "One\n2".
        //
        // It is a heuristic, and it can be wrong: a caption whose entire body
        // is a bare number, immediately followed by a timing line with no blank
        // between, is read as a sequence number and dropped. That input is far
        // rarer than the missing separator it fixes, and a blank line — which
        // any conforming file has — makes it unambiguous.
        let flush = |items: &mut Vec<Caption>,
                     pending: &mut Option<(Ticks, Ticks)>,
                     lines: &mut Vec<String>,
                     at_timing: bool| {
            if at_timing {
                if let Some(last) = lines.last() {
                    if last.trim().chars().all(|c| c.is_ascii_digit()) && !last.trim().is_empty() {
                        lines.pop();
                    }
                }
            }
            if let Some((start, end)) = pending.take() {
                let body = lines.join("\n").trim().to_string();
                if !body.is_empty() && end > start {
                    items.push(Caption { start, end, text: body });
                }
            }
            lines.clear();
        };

        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            if let Some((a, b)) = line.split_once("-->") {
                // A new cue's timing ends the previous one, even when the file
                // forgot the blank line between them.
                flush(&mut items, &mut pending, &mut lines, true);
                // WebVTT allows settings after the end time ("... align:start").
                let b = b.split_whitespace().next().unwrap_or("");
                if let (Some(s), Some(e)) = (parse_timestamp(a), parse_timestamp(b)) {
                    pending = Some((s, e));
                }
                continue;
            }
            if line.trim().is_empty() {
                flush(&mut items, &mut pending, &mut lines, false);
                continue;
            }
            if pending.is_none() {
                // Headers, sequence numbers, NOTE/STYLE/REGION blocks and cue
                // identifiers all land here and are all correctly ignored.
                continue;
            }
            lines.push(line.to_string());
        }
        flush(&mut items, &mut pending, &mut lines, false);
        items.sort_by_key(|c| (c.start.0, c.end.0));
        items
    }
}
