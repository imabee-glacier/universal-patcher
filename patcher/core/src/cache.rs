//! Disposable hash cache. A stale or unreadable cache is discarded, never trusted.
use crate::storage::{atomic_write, checked_bytes, read_checked};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

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
    pub fn of(m: &std::fs::Metadata) -> Self {
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

    /// The stamp of an open file. On Windows the change time and file identity come from the
    /// handle: NTFS updates ChangeTime on every content write, even when the last-write time is
    /// set back, which plain metadata cannot show.
    #[cfg(unix)]
    pub fn of_file(f: &std::fs::File) -> std::io::Result<Self> {
        Ok(Self::of(&f.metadata()?))
    }

    #[cfg(windows)]
    pub fn of_file(f: &std::fs::File) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx,
        };
        let handle = f.as_raw_handle();
        // SAFETY: both structs are plain data; the handle is valid for the lifetime of `f`, and
        // each call writes at most the size it is given.
        let (basic, identity) = unsafe {
            let mut basic: FILE_BASIC_INFO = std::mem::zeroed();
            if GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                (&raw mut basic).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let mut identity: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(handle, &mut identity) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            (basic, identity)
        };
        Ok(Self {
            size: f.metadata()?.len(),
            modified_ns: basic.LastWriteTime,
            changed_ns: basic.ChangeTime,
            inode: (u64::from(identity.nFileIndexHigh) << 32) | u64::from(identity.nFileIndexLow),
            device: u64::from(identity.dwVolumeSerialNumber),
            mode: 0,
        })
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
            .filter(|e| {
                let age = now.saturating_sub(e.verified_at);
                &e.stamp == stamp && (0..MAX_AGE_SECONDS).contains(&age)
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> Stamp {
        Stamp {
            size: 1,
            modified_ns: 2,
            changed_ns: 3,
            inode: 4,
            device: 5,
            mode: 0o644,
        }
    }

    #[test]
    fn cached_hash_expires_after_the_maximum_age() {
        let mut cache = HashCache::empty("install");
        cache.record("game/a".into(), stamp(), "h".into(), 1_000);
        assert_eq!(cache.lookup("game/a", &stamp(), 1_000), Some("h"));
        assert_eq!(
            cache.lookup("game/a", &stamp(), 1_000 + MAX_AGE_SECONDS - 1),
            Some("h")
        );
        assert_eq!(
            cache.lookup("game/a", &stamp(), 1_000 + MAX_AGE_SECONDS),
            None
        );
    }

    #[test]
    fn hash_recorded_in_the_future_is_not_trusted() {
        // Recorded under a clock that was later set back: its age cannot be known.
        let mut cache = HashCache::empty("install");
        cache.record("game/a".into(), stamp(), "h".into(), 10 * MAX_AGE_SECONDS);
        assert_eq!(cache.lookup("game/a", &stamp(), 1_000), None);
    }
}
