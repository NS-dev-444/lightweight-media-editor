//! Reconnecting a project to media that moved (§42 "missing media").
//!
//! Spike S8 changed the shape of this problem. Security-scoped bookmarks
//! already survive a rename AND a directory move, so the platform resolves the
//! common case before relink is ever consulted. What remains is what bookmarks
//! cannot cover:
//!
//!   * a cross-volume move
//!   * a restore from backup (new inode, same content)
//!   * a file replaced by a different copy of the same material
//!   * a project opened on another machine entirely
//!
//! So this is the FALLBACK path, and it is ordered by confidence: identical
//! content first, then a filename match the user must confirm.

use crate::asset::{Asset, AssetId};
use crate::project::Project;
use std::path::{Path, PathBuf};

/// How a candidate was matched, in descending confidence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchQuality {
    /// Same content hash. Safe to apply without asking.
    ContentHash,
    /// Same filename and same duration. Very likely right, still confirm.
    NameAndDuration,
    /// Same filename only. Must be confirmed — this is how the wrong take
    /// silently ends up in someone's edit.
    NameOnly,
}

impl MatchQuality {
    /// Whether relink may be applied without asking the user.
    pub fn is_automatic(self) -> bool { self == MatchQuality::ContentHash }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub content_hash: Option<String>,
    pub duration_ticks: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct RelinkMatch {
    pub asset: AssetId,
    pub path: PathBuf,
    pub quality: MatchQuality,
}

fn file_name(p: &Path) -> Option<String> {
    p.file_name().map(|s| s.to_string_lossy().to_ascii_lowercase())
}

/// Assets whose recorded path no longer resolves.
///
/// A missing file is not an error state for the PROJECT — §13 keeps edit
/// instructions independent of media, so the timeline stays intact and only
/// playback and export are blocked.
pub fn missing_assets(project: &Project) -> Vec<AssetId> {
    project.assets.iter()
        .filter(|a| !Path::new(&a.path).exists())
        .map(|a| a.id)
        .collect()
}

/// Best candidate for one asset, or none.
pub fn best_match(asset: &Asset, candidates: &[Candidate]) -> Option<RelinkMatch> {
    let want_name = file_name(Path::new(&asset.path));

    let mut best: Option<RelinkMatch> = None;
    for c in candidates {
        let quality = if asset.content_hash.is_some()
            && c.content_hash.is_some()
            && asset.content_hash == c.content_hash
        {
            MatchQuality::ContentHash
        } else if want_name.is_some() && file_name(&c.path) == want_name {
            // Duration agreement raises confidence but never to automatic:
            // two takes of the same shot can share both name and length.
            if c.duration_ticks == Some(asset.duration.0) {
                MatchQuality::NameAndDuration
            } else {
                MatchQuality::NameOnly
            }
        } else {
            continue;
        };

        let better = match &best {
            None => true,
            Some(b) => quality_rank(quality) > quality_rank(b.quality),
        };
        if better {
            best = Some(RelinkMatch { asset: asset.id, path: c.path.clone(), quality });
            if quality == MatchQuality::ContentHash { break; }   // cannot improve
        }
    }
    best
}

fn quality_rank(q: MatchQuality) -> u8 {
    match q {
        MatchQuality::ContentHash => 3,
        MatchQuality::NameAndDuration => 2,
        MatchQuality::NameOnly => 1,
    }
}

/// Propose relinks for every missing asset.
pub fn propose(project: &Project, candidates: &[Candidate]) -> Vec<RelinkMatch> {
    missing_assets(project).into_iter()
        .filter_map(|id| project.asset(id).and_then(|a| best_match(a, candidates)))
        .collect()
}

/// Apply a relink.
///
/// The stored bookmark is INVALIDATED: it pointed at the old file, and a stale
/// bookmark that still resolves would silently keep opening the wrong one. The
/// host re-creates it after the user grants access to the new path (S8).
pub fn apply(project: &mut Project, m: &RelinkMatch) -> bool {
    let Some(a) = project.assets.iter_mut().find(|a| a.id == m.asset) else { return false };
    a.path = m.path.to_string_lossy().into_owned();
    a.bookmark = None;
    a.relative_path = None;
    true
}

/// Apply only the matches that are safe without asking (content-hash equality).
/// Everything else is returned for the user to confirm.
pub fn apply_automatic(project: &mut Project, matches: &[RelinkMatch]) -> Vec<RelinkMatch> {
    let mut needs_confirmation = Vec::new();
    for m in matches {
        if m.quality.is_automatic() { apply(project, m); }
        else { needs_confirmation.push(m.clone()); }
    }
    needs_confirmation
}
