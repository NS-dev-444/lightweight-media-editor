//! Cache policy (AD-10, §14).
//!
//! Policy only — no file I/O. Keeping the rules here means they are unit
//! testable and identical on both platforms; the host owns the actual store.
//!
//! Three properties AD-10 requires, all enforced by construction below:
//!
//! 1. **Keyed by content identity, not path.** Phase 1's §42 cases — moved,
//!    renamed, restored files — break a path-keyed cache. A content key
//!    survives all of them and also distinguishes two different files that
//!    happen to share a name.
//! 2. **Version-gated.** Every key carries the format version that produced
//!    it, so a changed generator cannot read stale entries as if they were
//!    current. This is what makes "corrupt cache must not corrupt the project"
//!    achievable rather than aspirational.
//! 3. **Never the source of truth.** Deleting the entire cache may change how
//!    fast the app is. It must never change what a project contains or what it
//!    exports.

use serde::{Deserialize, Serialize};

/// Cache classes have very different rebuild costs, so they get independent
/// budgets. One global "clear cache" that discards proxies alongside
/// thumbnails is a bad experience: thumbnails regenerate in moments, proxies
/// can take minutes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum CacheClass { Thumbnail, Waveform, Proxy, Render }

impl CacheClass {
    pub fn name(self) -> &'static str {
        match self {
            CacheClass::Thumbnail => "thumbnails",
            CacheClass::Waveform => "waveforms",
            CacheClass::Proxy => "proxies",
            CacheClass::Render => "render",
        }
    }
    /// Bump when the GENERATOR changes, so old entries are ignored rather than
    /// misread. Per class, because the generators are independent.
    pub fn format_version(self) -> u32 {
        match self {
            CacheClass::Thumbnail => 1,
            CacheClass::Waveform => 1,
            CacheClass::Proxy => 1,
            CacheClass::Render => 1,
        }
    }
    /// Roughly how expensive this is to rebuild. Drives eviction order: cheap
    /// things go first when space is tight.
    pub fn rebuild_cost(self) -> u32 {
        match self {
            CacheClass::Thumbnail => 1,
            CacheClass::Waveform => 2,
            CacheClass::Render => 3,
            CacheClass::Proxy => 10,
        }
    }
}

/// Identity of one cache entry.
///
/// Deliberately contains no path. Two projects referencing the same media
/// share entries; the same file at a new path still hits.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct CacheKey {
    pub class: CacheClass,
    /// Content hash of the source. The identity that survives moves.
    pub content_hash: String,
    pub stream_index: u32,
    /// Generation parameters — thumbnail size, waveform buckets/sec, proxy
    /// resolution. Different parameters are different entries, not a
    /// conflicting one.
    pub params: String,
    pub format_version: u32,
}

impl CacheKey {
    pub fn new(class: CacheClass, content_hash: impl Into<String>,
               stream_index: u32, params: impl Into<String>) -> Self {
        CacheKey { class, content_hash: content_hash.into(), stream_index,
                   params: params.into(), format_version: class.format_version() }
    }
    /// Stable, filesystem-safe name. Class first so a class can be cleared by
    /// removing one directory.
    pub fn storage_path(&self) -> String {
        let p: String = self.params.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        format!("{}/{}_{}_{}_v{}", self.class.name(), self.content_hash,
                self.stream_index, p, self.format_version)
    }
    /// An entry written by a different generator version must be ignored.
    pub fn is_current(&self) -> bool { self.format_version == self.class.format_version() }
}

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub key: CacheKey,
    pub bytes: u64,
    /// Monotonic counter, not a wall clock: clock changes must not corrupt
    /// eviction order.
    pub last_used: u64,
}

/// Per-class byte budgets.
#[derive(Clone, Copy, Debug)]
pub struct CacheBudget {
    pub thumbnails: u64,
    pub waveforms: u64,
    pub proxies: u64,
    pub render: u64,
}

impl Default for CacheBudget {
    fn default() -> Self {
        // Proxies dominate because they are the expensive thing to rebuild;
        // thumbnails and waveforms are small and cheap.
        CacheBudget {
            thumbnails: 512 * 1024 * 1024,
            waveforms: 256 * 1024 * 1024,
            proxies: 20 * 1024 * 1024 * 1024,
            render: 4 * 1024 * 1024 * 1024,
        }
    }
}

impl CacheBudget {
    pub fn for_class(&self, c: CacheClass) -> u64 {
        match c {
            CacheClass::Thumbnail => self.thumbnails,
            CacheClass::Waveform => self.waveforms,
            CacheClass::Proxy => self.proxies,
            CacheClass::Render => self.render,
        }
    }
}

/// What to delete to get a class back inside its budget.
///
/// Least-recently-used within a class, so eviction never crosses classes —
/// filling the proxy cache must not evict every thumbnail.
pub fn evict_for_budget(entries: &[CacheEntry], class: CacheClass, budget: &CacheBudget)
    -> Vec<CacheKey>
{
    let limit = budget.for_class(class);
    let mut in_class: Vec<&CacheEntry> = entries.iter().filter(|e| e.key.class == class).collect();
    let total: u64 = in_class.iter().map(|e| e.bytes).sum();
    if total <= limit { return Vec::new(); }

    in_class.sort_by_key(|e| e.last_used);      // oldest first
    let mut freed = 0u64;
    let mut out = Vec::new();
    for e in in_class {
        if total - freed <= limit { break; }
        freed += e.bytes;
        out.push(e.key.clone());
    }
    out
}

/// Entries written by an older generator. Always safe to delete — they cannot
/// be read correctly, and keeping them only wastes space.
pub fn stale_entries(entries: &[CacheEntry]) -> Vec<CacheKey> {
    entries.iter().filter(|e| !e.key.is_current()).map(|e| e.key.clone()).collect()
}

/// Total bytes per class, for the preferences UI (§14: sizes must be shown so
/// clearing is an informed choice).
pub fn usage_by_class(entries: &[CacheEntry]) -> Vec<(CacheClass, u64, usize)> {
    let mut out = Vec::new();
    for class in [CacheClass::Thumbnail, CacheClass::Waveform,
                  CacheClass::Proxy, CacheClass::Render] {
        let matching: Vec<&CacheEntry> = entries.iter().filter(|e| e.key.class == class).collect();
        out.push((class, matching.iter().map(|e| e.bytes).sum(), matching.len()));
    }
    out
}
