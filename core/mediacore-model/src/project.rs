//! The project document: save, load, migrate, autosave.
//!
//! AD-9: a single JSON file with a `schema_version` from the first commit, an
//! atomic save (temp → fsync → rename), and an autosave JOURNAL of commands
//! rather than repeated full rewrites — §24 requires autosave that never
//! blocks the UI, and rewriting a large document on every keystroke would.

use crate::asset::{Asset, AssetId};
use crate::command::{Edit, EditError, History};
use crate::time::{FrameRate, Ticks};
use crate::timeline::{ClipId, Timeline};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Bump ONLY together with a migration in `migrate`. AD-9 requires a migration
/// path to exist before any format change ships.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ExportSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// AD-12: HEVC is the default wherever compatibility permits. S4b measured
    /// hardware HEVC at 1.74x libx264's bitrate versus 2.31x for hardware H.264.
    pub codec: String,
    pub bitrate_kbps: u32,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings { width: 1920, height: 1080, frame_rate: FrameRate::FILM,
                         codec: "hevc".into(), bitrate_kbps: 12_000 }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub name: String,
    /// The project's own rate. Source clips at other rates are conformed to it.
    pub frame_rate: FrameRate,
    /// AD-7: one project sample rate, everything resampled on import.
    pub sample_rate: u32,
    pub assets: Vec<Asset>,
    pub timeline: Timeline,
    pub export: ExportSettings,
    #[serde(default)]
    next_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            schema_version: SCHEMA_VERSION,
            name: "Untitled".into(),
            frame_rate: FrameRate::FILM,
            sample_rate: 48_000,
            assets: Vec::new(),
            timeline: Timeline::v1_default(),
            export: ExportSettings::default(),
            next_id: 100,
        }
    }
}

#[derive(Debug)]
pub enum ProjectError {
    Io(std::io::Error),
    Parse(String),
    UnsupportedVersion(u32),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // §23: explain, then offer the technical detail separately.
            ProjectError::Io(e) => write!(f, "This project file could not be read. {e}"),
            ProjectError::Parse(_) => write!(f, "This project file appears to be damaged and could not be opened."),
            ProjectError::UnsupportedVersion(v) =>
                write!(f, "This project was created by a newer version of the app (format {v}). Please update to open it."),
        }
    }
}
impl From<std::io::Error> for ProjectError { fn from(e: std::io::Error) -> Self { ProjectError::Io(e) } }

impl Project {
    pub fn new_id(&mut self) -> u64 { self.next_id += 1; self.next_id }
    pub fn new_clip_id(&mut self) -> ClipId { ClipId(self.new_id()) }
    pub fn new_asset_id(&mut self) -> AssetId { AssetId(self.new_id()) }
    pub fn asset(&self, id: AssetId) -> Option<&Asset> { self.assets.iter().find(|a| a.id == id) }
    pub fn duration(&self) -> Ticks { self.timeline.duration() }

    /// Any HDR source present. O-3: V1 tone-maps to SDR and must TELL the user.
    pub fn hdr_assets(&self) -> Vec<AssetId> {
        self.assets.iter().filter(|a| a.is_hdr()).map(|a| a.id).collect()
    }
    /// Any VFR source present. Advisory only — timing is PTS-driven regardless.
    pub fn vfr_assets(&self) -> Vec<AssetId> {
        self.assets.iter()
            .filter(|a| a.video.as_ref().is_some_and(|v| v.is_vfr))
            .map(|a| a.id).collect()
    }

    pub fn to_json(&self) -> Result<String, ProjectError> {
        serde_json::to_string_pretty(self).map_err(|e| ProjectError::Parse(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, ProjectError> {
        let mut v: serde_json::Value =
            serde_json::from_str(s).map_err(|e| ProjectError::Parse(e.to_string()))?;
        let version = v.get("schema_version").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        if version > SCHEMA_VERSION { return Err(ProjectError::UnsupportedVersion(version)); }
        migrate(&mut v, version)?;
        serde_json::from_value(v).map_err(|e| ProjectError::Parse(e.to_string()))
    }

    /// Atomic save: temp file → fsync → rename. A crash or power loss leaves
    /// either the old file or the new one, never a truncated hybrid (AD-9).
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let json = self.to_json()?;
        let tmp = path.with_extension("tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;                      // the fsync is the point
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        Project::from_json(&std::fs::read_to_string(path)?)
    }
}

/// Migrations run oldest-first. Adding a new SCHEMA_VERSION without a matching
/// arm here is a bug, and the test below enforces that.
fn migrate(v: &mut serde_json::Value, from: u32) -> Result<(), ProjectError> {
    let mut version = from;
    // v0 predates versioning: files written before the field existed.
    if version == 0 {
        v.as_object_mut()
            .ok_or_else(|| ProjectError::Parse("project is not an object".into()))?
            .insert("schema_version".into(), serde_json::json!(1));
        version = 1;
    }
    debug_assert_eq!(version, SCHEMA_VERSION, "missing migration arm");
    Ok(())
}

/// Crash-safe autosave (§24).
///
/// Full saves are periodic; between them each applied command is appended to a
/// journal. Recovery replays the journal onto the last full save. Appending a
/// few hundred bytes never blocks the UI; rewriting a large document would.
pub struct Autosave {
    project_path: PathBuf,
    journal_path: PathBuf,
    pending: usize,
    full_save_every: usize,
}

impl Autosave {
    pub fn new(project_path: impl Into<PathBuf>) -> Self {
        let p: PathBuf = project_path.into();
        let j = p.with_extension("journal");
        Autosave { project_path: p, journal_path: j, pending: 0, full_save_every: 50 }
    }

    pub fn journal_path(&self) -> &Path { &self.journal_path }

    /// Record one applied edit. Cheap: an append and an fsync of a small file.
    pub fn record(&mut self, edit: &Edit, project: &Project) -> Result<(), ProjectError> {
        let line = serde_json::to_string(edit).map_err(|e| ProjectError::Parse(e.to_string()))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true).append(true).open(&self.journal_path)?;
        writeln!(f, "{line}")?;
        f.sync_data()?;
        self.pending += 1;
        if self.pending >= self.full_save_every { self.checkpoint(project)?; }
        Ok(())
    }

    /// Write a full save and clear the journal.
    pub fn checkpoint(&mut self, project: &Project) -> Result<(), ProjectError> {
        project.save(&self.project_path)?;
        let _ = std::fs::remove_file(&self.journal_path);
        self.pending = 0;
        Ok(())
    }

    /// True when a journal survives — the app did not exit cleanly.
    pub fn has_unrecovered_work(&self) -> bool { self.journal_path.exists() }

    /// Replay the journal onto the last full save.
    ///
    /// Returns how many edits were recovered and how many were skipped. A
    /// journal can end mid-write after a crash, so a trailing partial line is
    /// expected and must not abort recovery (§24, §42).
    pub fn recover(&self) -> Result<(Project, usize, usize), ProjectError> {
        let mut project = Project::load(&self.project_path)?;
        let mut applied = 0usize;
        let mut skipped = 0usize;
        if let Ok(text) = std::fs::read_to_string(&self.journal_path) {
            for line in text.lines() {
                if line.trim().is_empty() { continue; }
                match serde_json::from_str::<Edit>(line) {
                    Ok(edit) => match edit.apply(&mut project.timeline) {
                        Ok(_) => applied += 1,
                        Err(_) => skipped += 1,
                    },
                    Err(_) => skipped += 1,   // truncated trailing line
                }
            }
        }
        Ok((project, applied, skipped))
    }
}

/// A project plus its history, which is what the UI actually drives.
pub struct Session {
    pub project: Project,
    pub history: History,
    pub autosave: Option<Autosave>,
    dirty: bool,
}

impl Session {
    pub fn new(project: Project) -> Self {
        Session { project, history: History::default(), autosave: None, dirty: false }
    }
    pub fn with_autosave(mut self, path: impl Into<PathBuf>) -> Self {
        self.autosave = Some(Autosave::new(path)); self
    }
    pub fn is_dirty(&self) -> bool { self.dirty }

    /// The single funnel every edit goes through: apply, record for undo,
    /// journal for crash recovery. Nothing else may mutate the timeline.
    pub fn apply(&mut self, edit: Edit) -> Result<(), EditError> {
        let journal_copy = self.autosave.as_ref().map(|_| edit.clone());
        self.history.apply(&mut self.project.timeline, edit)?;
        self.dirty = true;
        if let (Some(a), Some(e)) = (self.autosave.as_mut(), journal_copy) {
            let _ = a.record(&e, &self.project);   // never fail an edit on I/O
        }
        Ok(())
    }

    pub fn undo(&mut self) -> Result<Option<&'static str>, EditError> {
        let r = self.history.undo(&mut self.project.timeline)?;
        if r.is_some() { self.dirty = true; }
        Ok(r)
    }
    pub fn redo(&mut self) -> Result<Option<&'static str>, EditError> {
        let r = self.history.redo(&mut self.project.timeline)?;
        if r.is_some() { self.dirty = true; }
        Ok(r)
    }
    pub fn save(&mut self, path: &Path) -> Result<(), ProjectError> {
        self.project.save(path)?;
        if let Some(a) = self.autosave.as_mut() { a.checkpoint(&self.project)?; }
        self.dirty = false;
        Ok(())
    }
}
