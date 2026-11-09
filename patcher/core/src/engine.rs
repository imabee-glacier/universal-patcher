use crate::{
    cache::{HashCache, Stamp},
    model::{GameFile, Manifest, Marker},
    storage::{self, Applied, Binding, Event, Journal, JournalCorrupt, Lock, Policy},
    transfer::{Blocker, Control, ObjectSource},
    trust::VerifiedRelease,
};
use anyhow::{Context, Result, bail, ensure};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub phase: String,
    pub detail: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: usize,
    pub files_total: usize,
}

/// A durability boundary inside one file replacement, used to inject interruptions in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    AfterIntent,
    AfterRename,
    AfterCommit,
}

#[derive(Debug, Default)]
pub struct RunOptions {
    /// Stop with an error at this boundary of the nth replacement (1-based).
    pub interrupt: Option<(Boundary, usize)>,
    /// Hash every selected file instead of trusting cached hashes.
    pub full_verify: bool,
}

/// What a completed run changed, for the Details view.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RunReport {
    pub replaced: Vec<String>,
    pub deleted: Vec<String>,
    /// Obsolete files left in place because their content was not the expected old content.
    pub preserved: Vec<String>,
    /// The recovery journal was damaged and set aside; every selected file was hashed.
    pub journal_reset: bool,
}

struct Root {
    dir: Dir,
    path: PathBuf,
    identity: String,
}

pub fn identity(path: &Path) -> Result<String> {
    let path = fs::canonicalize(path)?;
    let m = fs::metadata(&path)?;
    ensure!(m.is_dir(), "Install root is not a directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}:{}", path.display(), m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(path.to_string_lossy().into_owned())
    }
}

fn not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

impl Root {
    fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path)?;
        let identity = identity(&path)?;
        let dir = Dir::open_ambient_dir(&path, ambient_authority())?;
        Ok(Self {
            dir,
            path,
            identity,
        })
    }

    /// Rejects a changed root, an unsafe path, and any symlink along the path.
    fn check(&self, path: &str) -> Result<()> {
        ensure!(
            identity(&self.path)? == self.identity,
            "Installation root changed"
        );
        crate::model::valid_path(path)?;
        let mut prefix = PathBuf::new();
        for component in path.split('/') {
            prefix.push(component);
            match self.dir.symlink_metadata(&prefix) {
                Ok(m) => ensure!(
                    !m.file_type().is_symlink(),
                    "Managed path contains a symlink"
                ),
                Err(e) if not_found(&e) => break,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<Option<Stamp>> {
        self.check(path)?;
        match self.dir.open(path) {
            Ok(f) => {
                let f = f.into_std();
                ensure!(
                    f.metadata()?.is_file(),
                    "Managed path is not a regular file"
                );
                Ok(Some(Stamp::of_file(&f)?))
            }
            Err(e) if not_found(&e) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Hashes a file, rejecting the result if the file changed or was replaced while reading.
    fn hash(&self, path: &str) -> Result<Option<(String, Stamp)>> {
        self.check(path)?;
        let f = match self.dir.open(path) {
            Ok(f) => f.into_std(),
            Err(e) if not_found(&e) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            f.metadata()?.is_file(),
            "Managed path is not a regular file"
        );
        let before = Stamp::of_file(&f)?;
        let (hash, size) = storage::hash_reader(&f)?;
        ensure!(
            size == before.size && Stamp::of_file(&f)? == before,
            "File changed while hashing"
        );
        ensure!(
            self.stat(path)?.as_ref() == Some(&before),
            "File replaced while hashing"
        );
        Ok(Some((hash, before)))
    }

    fn parent(&self, path: &str) -> Result<(Dir, String)> {
        self.check(path)?;
        let p = Path::new(path);
        let parent = p.parent().context("Managed path has no parent")?;
        let name = p
            .file_name()
            .context("Managed path has no file name")?
            .to_string_lossy()
            .into_owned();
        if parent.as_os_str().is_empty() {
            return Ok((self.dir.try_clone()?, name));
        }
        self.dir.create_dir_all(parent)?;
        self.check(path)?;
        Ok((self.dir.open_dir(parent)?, name))
    }
}

/// Flushes `dir`'s entries to disk. Windows flushes only through a directory handle opened
/// with write access and backup semantics.
fn sync(dir: &Dir) -> Result<()> {
    #[cfg(windows)]
    let handle = {
        use cap_std::fs::{OpenOptions, OpenOptionsExt};
        let mut options = OpenOptions::new();
        options
            .write(true)
            .custom_flags(crate::storage::FILE_FLAG_BACKUP_SEMANTICS);
        dir.open_with(".", &options)?
    };
    #[cfg(not(windows))]
    let handle = dir.open(".")?;
    handle.into_std().sync_all()?;
    Ok(())
}

fn busy(variant: &str) -> Result<()> {
    if variant == "fixture" {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        const PROCESSES: [&str; 5] = [
            "ts4_x64.exe",
            "ts4_dx9_x64.exe",
            "ts4_launcher_x64.exe",
            "eadesktop.exe",
            "eabackgroundservice.exe",
        ];
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            if let Ok(cmd) = fs::read(entry.path().join("cmdline")) {
                let cmd = String::from_utf8_lossy(&cmd).to_lowercase();
                ensure!(
                    !cmd.split('\0')
                        .any(|arg| PROCESSES.iter().any(|p| arg.ends_with(p))),
                    Blocker::GameRunning
                );
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!("Native platform coordination is not yet validated")
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn key(root_id: &str, path: &str) -> String {
    format!("{root_id}/{path}")
}

/// File inspection backed by the hash cache. Critical files and full verification always hash.
struct Inventory {
    cache: HashCache,
    critical: BTreeSet<String>,
    full: bool,
    now: i64,
}

impl Inventory {
    fn inspect(
        &mut self,
        root: &Root,
        root_id: &str,
        path: &str,
    ) -> Result<Option<(String, Stamp)>> {
        let k = key(root_id, path);
        if !self.full && !self.critical.contains(&k) {
            let Some(stamp) = root.stat(path)? else {
                self.cache.forget(&k);
                return Ok(None);
            };
            if let Some(hash) = self.cache.lookup(&k, &stamp, self.now) {
                return Ok(Some((hash.to_owned(), stamp)));
            }
        }
        let result = root.hash(path)?;
        match &result {
            Some((hash, stamp)) => self.cache.record(k, stamp.clone(), hash.clone(), self.now),
            None => self.cache.forget(&k),
        }
        Ok(result)
    }
}

fn matches(file: &GameFile, actual: Option<&(String, Stamp)>) -> bool {
    actual.is_some_and(|(hash, stamp)| {
        *hash == file.target_hash
            && stamp.size == file.target_size
            && (cfg!(not(unix)) || stamp.executable() == file.attributes.executable)
    })
}

/// A marker is acceptable if it has the fingerprinted content, or if this patcher wrote the
/// current release's target there during an unfinished run recorded in the journal.
fn marker_ok(
    manifest: &Manifest,
    written: &BTreeSet<(String, String, String)>,
    marker: &Marker,
    actual: Option<&(String, Stamp)>,
) -> bool {
    let Some((hash, stamp)) = actual else {
        return false;
    };
    if *hash == marker.hash && stamp.size == marker.size {
        return true;
    }
    written.contains(&(marker.root_id.clone(), marker.path.clone(), hash.clone()))
        && manifest.files.iter().any(|f| {
            f.root_id == marker.root_id
                && f.path == marker.path
                && f.target_hash == *hash
                && f.target_size == stamp.size
        })
}

fn open_journal(state: &Path) -> Result<(Journal, bool)> {
    let path = state.join("journal.jsonl");
    match Journal::open(&path) {
        Ok(j) => Ok((j, false)),
        Err(e) if e.downcast_ref::<JournalCorrupt>().is_some() => {
            let aside = state.join(format!("journal.damaged-{}.jsonl", now()));
            fs::rename(&path, &aside).context("Cannot set the damaged journal aside")?;
            storage::sync_dir(state)?;
            Ok((Journal::open(&path)?, true))
        }
        Err(e) => Err(e),
    }
}

struct Planned<'a> {
    file: &'a GameFile,
    before: Option<String>,
}

pub fn run(
    release: &VerifiedRelease,
    policy: &mut Policy,
    state: &Path,
    source: &dyn ObjectSource,
    control: &Control,
    progress: &dyn Fn(Progress),
    options: &RunOptions,
) -> Result<RunReport> {
    let manifest = release.manifest();
    ensure!(
        policy.schema == 1
            && policy.game_id == manifest.game_id
            && policy.install.variant == manifest.variant,
        "Installation and release do not match"
    );
    // Production variants stay closed until adapter evidence exists; fixtures run the real engine.
    ensure!(
        manifest.variant == "fixture",
        "Sims 4 file maintenance is not yet validated for this platform"
    );
    ensure!(
        policy.suspended.is_none(),
        "Run is paused or cancelled; resume explicitly"
    );
    fs::create_dir_all(state)?;
    let _lock = Lock::acquire(&state.join("install.lock"))?;
    let mut roots = BTreeMap::new();
    for (id, path) in &policy.install.roots {
        roots.insert(id.clone(), Root::open(path)?);
    }
    let primary = roots.get("game").context("Game root is missing")?;
    ensure!(
        primary.identity == policy.install.identity,
        "Confirmed installation identity changed"
    );
    let root_for = |id: &str| -> Result<&Root> {
        roots
            .get(id)
            .context("Manifest references an unmapped root")
    };
    let binding = Binding {
        install: policy.install.identity.clone(),
        manifest: release.digest().into(),
        policy_revision: policy.revision,
    };
    let (mut journal, journal_reset) = open_journal(state)?;
    let mut report = RunReport {
        journal_reset,
        ..Default::default()
    };
    let cache_path = state.join("hash-cache.json");
    let mut inventory = Inventory {
        cache: if journal_reset {
            HashCache::empty(&binding.install)
        } else {
            HashCache::load(&cache_path, &binding.install)
        },
        critical: manifest
            .compatible_bases
            .iter()
            .flat_map(|b| b.files.iter().map(|m| key(&m.root_id, &m.path)))
            .collect(),
        full: options.full_verify || journal_reset,
        now: now(),
    };

    let written: BTreeSet<_> = journal
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Intent {
                root_id,
                path,
                target: Some(t),
                ..
            } => Some((root_id.clone(), path.clone(), t.clone())),
            _ => None,
        })
        .collect();
    let mut markers = BTreeMap::new();
    for m in manifest.compatible_bases.iter().flat_map(|b| &b.files) {
        let actual = inventory.inspect(root_for(&m.root_id)?, &m.root_id, &m.path)?;
        markers.insert(key(&m.root_id, &m.path), actual);
    }
    let compatible = manifest.compatible_bases.iter().any(|b| {
        b.files.iter().all(|m| {
            let actual = markers[&key(&m.root_id, &m.path)].as_ref();
            marker_ok(manifest, &written, m, actual)
        })
    });
    ensure!(compatible, Blocker::UnknownVersion);

    let selected = manifest.selected(&policy.desired)?;
    let mut plan = Vec::new();
    for (i, file) in selected.iter().enumerate() {
        control.check()?;
        progress(Progress {
            phase: "checking".into(),
            detail: file.path.clone(),
            bytes_done: 0,
            bytes_total: 0,
            files_done: i,
            files_total: selected.len(),
        });
        let actual = inventory.inspect(root_for(&file.root_id)?, &file.root_id, &file.path)?;
        if !matches(file, actual.as_ref()) {
            plan.push(Planned {
                file,
                before: actual.map(|(hash, _)| hash),
            });
        }
    }
    inventory.cache.save(&cache_path)?;

    let total = plan.iter().try_fold(0u64, |n, p| {
        n.checked_add(p.file.full.encoded_size)
            .context("Transfer size overflow")
    })?;
    let peak = plan
        .iter()
        .map(|p| p.file.full.encoded_size.saturating_add(p.file.target_size))
        .max()
        .unwrap_or(0);
    let budget = peak
        .checked_add((peak / 20).max(512 * 1024 * 1024))
        .context("Space calculation overflow")?;
    ensure!(fs2::available_space(state)? >= budget, Blocker::DiskSpace);
    for root in roots.values() {
        ensure!(
            fs2::available_space(&root.path)? >= budget,
            Blocker::DiskSpace
        );
    }

    match journal.open_binding().cloned() {
        Some(previous) if previous == binding => {}
        Some(previous) => {
            journal.append(Event::Supersede {
                from: previous,
                to: binding.clone(),
            })?;
            journal.append(Event::Bind {
                binding: binding.clone(),
            })?;
        }
        None => journal.append(Event::Bind {
            binding: binding.clone(),
        })?,
    }

    let interrupt = |boundary: Boundary, n: usize| -> Result<()> {
        if options.interrupt == Some((boundary, n)) {
            bail!("Injected interruption at {boundary:?}");
        }
        Ok(())
    };
    let cache = state.join("downloads");
    let mut done = 0;
    for (i, item) in plan.iter().enumerate() {
        control.check()?;
        busy(&manifest.variant)?;
        let f = item.file;
        let root = root_for(&f.root_id)?;
        let object = source.fetch(&f.full, &cache, control, &|n, _| {
            progress(Progress {
                phase: "downloading".into(),
                detail: f.path.clone(),
                bytes_done: done + n,
                bytes_total: total,
                files_done: i,
                files_total: plan.len(),
            })
        })?;
        let current = |root: &Root| -> Result<Option<String>> {
            Ok(root.hash(&f.path)?.map(|(hash, _)| hash))
        };
        ensure!(
            current(root)? == item.before,
            "Destination changed after planning"
        );
        let (parent, name) = root.parent(&f.path)?;
        let tmp = format!(
            ".patcher-{}.tmp",
            blake3::hash(format!("{}:{}", release.digest(), f.id).as_bytes()).to_hex()
        );
        match parent.remove_file(&tmp) {
            Err(e) if !not_found(&e) => return Err(e.into()),
            _ => {}
        }
        let mut output = parent
            .open_with(&tmp, OpenOptions::new().write(true).create_new(true))?
            .into_std();
        progress(Progress {
            phase: "applying".into(),
            detail: f.path.clone(),
            bytes_done: done,
            bytes_total: total,
            files_done: i,
            files_total: plan.len(),
        });
        crate::transfer::decode(&object, &f.full, &mut output, control)?;
        output.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if f.attributes.executable {
                0o755
            } else {
                0o644
            };
            output.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        output.sync_all()?;
        drop(output);
        ensure!(
            storage::hash_reader(parent.open(&tmp)?)? == (f.target_hash.clone(), f.target_size),
            "Decoded file hash mismatch"
        );
        control.check()?;
        busy(&manifest.variant)?;
        ensure!(
            current(root)? == item.before,
            "Destination changed before commit"
        );
        let operation = format!("replace:{}:{}", binding.policy_revision, f.id);
        journal.append(Event::Intent {
            operation: operation.clone(),
            root_id: f.root_id.clone(),
            path: f.path.clone(),
            before: item.before.clone(),
            target: Some(f.target_hash.clone()),
            temporary: Some(tmp.clone()),
        })?;
        interrupt(Boundary::AfterIntent, i + 1)?;
        parent.rename(&tmp, &parent, &name)?;
        sync(&parent)?;
        inventory.cache.forget(&key(&f.root_id, &f.path));
        interrupt(Boundary::AfterRename, i + 1)?;
        journal.append(Event::Commit { operation })?;
        interrupt(Boundary::AfterCommit, i + 1)?;
        fs::remove_file(object)?;
        done += f.full.encoded_size;
        report.replaced.push(f.path.clone());
    }

    for d in &manifest.deletions {
        control.check()?;
        busy(&manifest.variant)?;
        let root = root_for(&d.root_id)?;
        let Some((hash, _)) = root.hash(&d.path)? else {
            continue;
        };
        if hash != d.expected_hash {
            report.preserved.push(d.path.clone());
            continue;
        }
        let operation = format!("delete:{}:{}", d.root_id, d.path);
        journal.append(Event::Intent {
            operation: operation.clone(),
            root_id: d.root_id.clone(),
            path: d.path.clone(),
            before: Some(hash),
            target: None,
            temporary: None,
        })?;
        let (parent, name) = root.parent(&d.path)?;
        parent.remove_file(name)?;
        sync(&parent)?;
        inventory.cache.forget(&key(&d.root_id, &d.path));
        journal.append(Event::Commit { operation })?;
        report.deleted.push(d.path.clone());
    }

    for (i, f) in selected.iter().enumerate() {
        control.check()?;
        progress(Progress {
            phase: "verifying".into(),
            detail: f.path.clone(),
            bytes_done: done,
            bytes_total: total,
            files_done: i,
            files_total: selected.len(),
        });
        let actual = inventory.inspect(root_for(&f.root_id)?, &f.root_id, &f.path)?;
        ensure!(matches(f, actual.as_ref()), "Final verification failed");
    }
    inventory.cache.save(&cache_path)?;
    journal.append(Event::Complete {
        binding: binding.clone(),
    })?;
    policy.applied = Some(Applied {
        revision: policy.revision,
        release: manifest.release_id.clone(),
        manifest_hash: release.digest().into(),
    });
    policy.save(state)?;
    // Compact only after the policy is durable. An interrupted compaction is harmless. The
    // journal is closed first: Windows refuses to replace a file that is still open.
    drop(journal);
    storage::atomic_write(&state.join("journal.jsonl"), b"")?;
    progress(Progress {
        phase: "ready".into(),
        detail: "All selected files verified".into(),
        bytes_done: total,
        bytes_total: total,
        files_done: selected.len(),
        files_total: selected.len(),
    });
    Ok(report)
}
