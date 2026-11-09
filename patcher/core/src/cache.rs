//! Disposable hash cache. A stale or unreadable cache is discarded, never trusted.
use crate::storage::{atomic_write, checked_bytes, read_checked};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::Metadata, path::Path};

/// Maximum age of a cached hash before the file must be read again.
pub const MAX_AGE_SECONDS: i64 = 30 * 24 * 60 * 60;

/// File metadata that must be unchanged for a cached hash to be reused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub size: u64,
    pub modified_ns: i64,
    pub changed_ns: i64,
    pub inode: u64,
    pub device: u64,
    pub mode: u32,
}

impl Stamp {
    #[cfg(unix)]
    pub fn of(m: &Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        let ns = |s: i64, n: i64| s.saturating_mul(1_000_000_000).saturating_add(n);
        Self {
            size: m.len(),
            modified_ns: ns(m.mtime(), m.mtime_nsec()),
            changed_ns: ns(m.ctime(), m.ctime_nsec()),
            inode: m.ino(),
            device: m.dev(),
            mode: m.mode(),
        }
    }

    #[cfg(not(unix))]
    pub fn of(m: &Metadata) -> Self {
        let ns = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(-1, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
        Self {
            size: m.len(),
            modified_ns: ns,
            changed_ns: ns,
            inode: 0,
            device: 0,
            mode: 0,
        }
    }

    pub fn executable(&self) -> bool {
        self.mode & 0o111 != 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    stamp: Stamp,
    hash: String,
    verified_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashCache {
    install: String,
    entries: BTreeMap<String, Entry>,
}

impl HashCache {
    pub fn empty(install: &str) -> Self {
        Self {
            install: install.into(),
            entries: BTreeMap::new(),
        }
    }

    /// Loads the cache for one installation. Any failure yields an empty cache.
    pub fn load(path: &Path, install: &str) -> Self {
        match read_checked::<Self>(path) {
            Ok(cache) if cache.install == install => cache,
            _ => Self::empty(install),
        }
    }

    pub fn lookup(&self, key: &str, stamp: &Stamp, now: i64) -> Option<&str> {
        self.entries
            .get(key)
            .filter(|e| &e.stamp == stamp && now.saturating_sub(e.verified_at) < MAX_AGE_SECONDS)
            .map(|e| e.hash.as_str())
    }

    pub fn record(&mut self, key: String, stamp: Stamp, hash: String, now: i64) {
        self.entries.insert(
            key,
            Entry {
                stamp,
                hash,
                verified_at: now,
            },
        );
    }

    pub fn forget(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        atomic_write(path, &checked_bytes(self)?)
    }
}
