//! Source media references.
//!
//! §13 and §46 Rule 9: a project stores references plus edit instructions and
//! never modifies source media. That is enforced structurally here — nothing in
//! this module can express a write to a source file.
//!
//! Fields below are not speculative; each was established by a Phase 1 spike:
//!   * `bookmark`      S8 — the sandbox denies arbitrary paths outright, so a
//!                     bookmark is the only thing that survives relaunch.
//!   * `colour`        O-3 — HDR must be detected, never assumed.
//!   * `is_vfr`        Phase 1 — detection is unreliable, so this is advisory
//!                     only; timing correctness never depends on it.

use crate::time::{FrameRate, Ticks};
use serde::{Deserialize, Serialize};

/// Stable identifier for a source within a project.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum TransferFunction {
    #[default]
    /// Includes "unspecified", which Phase 1 found is the COMMON real-world
    /// case even for files tagged bt709 at encode time. O-3: treat as Rec.709.
    Rec709,
    Pq,
    Hlg,
}

impl TransferFunction {
    pub fn is_hdr(self) -> bool { matches!(self, TransferFunction::Pq | TransferFunction::Hlg) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct ColourInfo {
    pub transfer: TransferFunction,
    pub bit_depth: u8,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub colour: ColourInfo,
    /// Advisory only. Phase 1 showed VFR detection is unreliable (metadata
    /// misses mixed-rate files; shallow probes miss everything), so this
    /// drives user messaging and export strategy — never timing correctness.
    pub is_vfr: bool,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

/// A reference to a file on disk. Never a copy of it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    /// Absolute path at the time of import. A hint, not the source of truth.
    pub path: String,
    /// Path relative to the project file, so a project plus its media can be
    /// moved together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    /// Content hash, for relinking when the bookmark cannot resolve
    /// (cross-volume move, restore from backup, replaced copy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// macOS security-scoped bookmark, base64. REQUIRED for sandboxed relaunch.
    /// S8 proved this is load-bearing, not precautionary — and that it also
    /// survives a rename plus a directory move, which a path does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
    pub duration: Ticks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioInfo>,
}

impl Asset {
    pub fn new(id: AssetId, path: impl Into<String>, duration: Ticks) -> Self {
        Asset { id, path: path.into(), relative_path: None, content_hash: None,
                bookmark: None, duration, video: None, audio: None }
    }
    pub fn is_hdr(&self) -> bool {
        self.video.as_ref().is_some_and(|v| v.colour.transfer.is_hdr())
    }
    pub fn frame_rate(&self) -> FrameRate {
        self.video.as_ref().map(|v| v.frame_rate).unwrap_or_default()
    }
}
