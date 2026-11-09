use crate::model::Selection;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub fn hash_reader(mut r: impl Read) -> Result<(String, u64)> {
    let mut h = blake3::Hasher::new();
    let mut n = 0;
    let mut b = [0u8; 128 * 1024];
    loop {
        let k = r.read(&mut b)?;
        if k == 0 {
            break;
        }
        h.update(&b[..k]);
        n += k as u64;
    }
    Ok((h.finalize().to_hex().to_string(), n))
}
/// Hashes bytes as they are written, without storing them.
#[derive(Default)]
pub struct HashWriter {
    hasher: blake3::Hasher,
    bytes: u64,
}
impl HashWriter {
    pub fn finish(&self) -> (String, u64) {
        (self.hasher.finalize().to_hex().to_string(), self.bytes)
    }
}
impl Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.bytes += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    hash_reader(File::open(path)?)
}
/// Opens a directory with the `FILE_FLAG_BACKUP_SEMANTICS` Windows requires for directory handles.
#[cfg(windows)]
pub(crate) const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// Flushes a directory's entries to disk, after a rename or create inside it.
/// Windows flushes only through a handle with write access.
pub fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(windows)]
    let dir = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
    };
    #[cfg(not(windows))]
    let dir = File::open(path)?;
    dir.sync_all()
        .with_context(|| format!("Flushing directory {}", path.display()))
}

/// Flushes a file's data and metadata to disk. Windows flushes only through a handle with
/// write access.
pub fn sync_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    let file = fs::OpenOptions::new().write(true).open(path)?;
    #[cfg(not(windows))]
    let file = File::open(path)?;
    file.sync_all()
        .with_context(|| format!("Flushing {}", path.display()))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    sync_dir(parent)
}
/// Like `atomic_write`, but readable by everyone: for files a web server publishes.
pub fn atomic_write_public(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    sync_dir(parent)
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    schema: u32,
    checksum: String,
    payload: T,
}
pub fn checked_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    // Hash canonicalized Value, independent of the struct's field serialization order.
    let payload = serde_json::to_value(value)?;
    let checksum = blake3::hash(&serde_json::to_vec(&payload)?)
        .to_hex()
        .to_string();
    Ok(serde_json::to_vec(&Envelope {
        schema: 1,
        checksum,
        payload,
    })?)
}
pub fn read_checked<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)?;
    ensure!(bytes.len() < 32 * 1024 * 1024, "State exceeds limit");
    let value = crate::strict_json::parse(&bytes)?;
    let e: Envelope<serde_json::Value> = serde_json::from_value(value)?;
    ensure!(e.schema == 1, "Unsupported state schema");
    ensure!(
        blake3::hash(&serde_json::to_vec(&e.payload)?)
            .to_hex()
            .as_str()
            == e.checksum,
        "State checksum mismatch"
    );
    Ok(serde_json::from_value(e.payload)?)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub identity: String,
    pub variant: String,
    pub roots: BTreeMap<String, PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    pub revision: u64,
    pub release: String,
    pub manifest_hash: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub future_packs: bool,
    pub background: bool,
    pub metered: bool,
    pub bandwidth_bytes_per_second: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: u32,
    pub game_id: String,
    pub install: Installation,
    pub revision: u64,
    pub desired: Selection,
    pub applied: Option<Applied>,
    pub preferences: Preferences,
    pub suspended: Option<String>,
}
impl Policy {
    /// Enables DLC that appeared since the user last answered, if they opted in to future packs.
    /// Returns true when the desired selection changed and a new revision was created.
    pub fn adopt_new_packs(&mut self, manifest: &crate::model::Manifest) -> bool {
        if !self.preferences.future_packs {
            return false;
        }
        let mut changed = false;
        for g in &manifest.groups {
            if g.kind == crate::model::GroupKind::Dlc
                && !self.desired.dlc.contains_key(&g.id)
                && !self.desired.deferred.contains(&g.id)
            {
                self.desired.dlc.insert(g.id.clone(), true);
                changed = true;
            }
        }
        if changed {
            self.revision += 1;
        }
        changed
    }
    pub fn load(dir: &Path) -> Result<Self> {
        let primary = read_checked::<Self>(&dir.join("policy.json"));
        match primary {
            Ok(p) => {
                ensure!(p.schema == 1, "Unsupported policy schema");
                Ok(p)
            }
            Err(e) => {
                let p: Self = read_checked(&dir.join("policy.previous.json"))
                    .with_context(|| format!("Cannot recover policy: {e}"))?;
                ensure!(p.schema == 1, "Unsupported policy schema");
                Ok(p)
            }
        }
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        ensure!(self.schema == 1, "Unsupported policy schema");
        let path = dir.join("policy.json");
        if let Ok(previous) = read_checked::<Self>(&path) {
            atomic_write(
                &dir.join("policy.previous.json"),
                &checked_bytes(&previous)?,
            )?;
        }
        atomic_write(&path, &checked_bytes(self)?)
    }
}
pub struct Lock {
    _file: File,
}
impl Lock {
    pub fn acquire(path: &Path) -> Result<Self> {
        fs::create_dir_all(path.parent().context("Lock parent missing")?)?;
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        f.try_lock_exclusive()
            .context("Another patcher run owns this installation")?;
        Ok(Self { _file: f })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub install: String,
    pub manifest: String,
    pub policy_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Bind {
        binding: Binding,
    },
    Intent {
        operation: String,
        root_id: String,
        path: String,
        before: Option<String>,
        target: Option<String>,
        temporary: Option<String>,
    },
    Commit {
        operation: String,
    },
    /// An unfinished run was replaced by a run with a different binding.
    Supersede {
        from: Binding,
        to: Binding,
    },
    Complete {
        binding: Binding,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    sequence: u64,
    previous: String,
    event: Event,
    checksum: String,
}
/// Marks a journal whose content cannot be trusted; full reconciliation is required.
#[derive(Debug)]
pub struct JournalCorrupt;
impl std::fmt::Display for JournalCorrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Recovery journal is damaged")
    }
}
pub struct Journal {
    file: File,
    sequence: u64,
    last: String,
    pub events: Vec<Event>,
}
impl Journal {
    pub fn open(path: &Path) -> Result<Self> {
        fs::create_dir_all(path.parent().context("Journal parent missing")?)?;
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 128 * 1024 * 1024, JournalCorrupt);
        let mut offset = 0;
        let mut sequence = 0;
        let mut last = String::new();
        let mut events = Vec::new();
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if !line.ends_with(b"\n") {
                file.set_len(offset as u64)?;
                file.sync_all()?;
                break;
            }
            let rec: Record = crate::strict_json::parse(&line[..line.len() - 1])
                .and_then(|v| Ok(serde_json::from_value(v)?))
                .context(JournalCorrupt)?;
            ensure!(
                rec.sequence == sequence && rec.previous == last,
                JournalCorrupt
            );
            let expected = blake3::hash(&serde_json::to_vec(&(
                rec.sequence,
                &rec.previous,
                &rec.event,
            ))?)
            .to_hex()
            .to_string();
            ensure!(rec.checksum == expected, JournalCorrupt);
            sequence += 1;
            last = rec.checksum;
            events.push(rec.event);
            offset += line.len();
        }
        sync_dir(path.parent().unwrap())?;
        Ok(Self {
            file,
            sequence,
            last,
            events,
        })
    }
    /// The binding of the last run that has not recorded completion.
    pub fn open_binding(&self) -> Option<&Binding> {
        for e in self.events.iter().rev() {
            match e {
                Event::Complete { .. } => return None,
                Event::Bind { binding } => return Some(binding),
                _ => {}
            }
        }
        None
    }
    pub fn append(&mut self, event: Event) -> Result<()> {
        let checksum = blake3::hash(&serde_json::to_vec(&(self.sequence, &self.last, &event))?)
            .to_hex()
            .to_string();
        let r = Record {
            sequence: self.sequence,
            previous: self.last.clone(),
            event: event.clone(),
            checksum: checksum.clone(),
        };
        let mut data = serde_json::to_vec(&r)?;
        data.push(b'\n');
        self.file.write_all(&data)?;
        self.file.sync_all()?;
        self.sequence += 1;
        self.last = checksum;
        self.events.push(event);
        Ok(())
    }
}
