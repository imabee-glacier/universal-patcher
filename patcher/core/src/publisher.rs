use crate::{
    model::*,
    storage::{Lock, atomic_write, hash_file},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Seek, Write},
    num::NonZeroU64,
    path::{Path, PathBuf},
};
use tough::{
    editor::{
        RepositoryEditor,
        signed::{PathExists, SignedRole},
    },
    key_source::{KeySource, LocalKeySource},
    schema::{KeyHolder, Root},
};

#[derive(Serialize)]
pub struct ArchiveEntry {
    pub path: String,
    pub bytes: u64,
    pub crc32: u32,
    pub compression: String,
    pub classification: String,
    pub pack: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    pub source: String,
    pub source_bytes: u64,
    pub files: Vec<ArchiveEntry>,
    pub payload_bytes: u64,
    pub pack_ids: BTreeSet<String>,
    pub production_ready: bool,
}
fn pack_id(s: &str) -> bool {
    s.is_ascii()
        && s.len() >= 4
        && ["EP", "GP", "SP", "FP"].contains(&&s[..2])
        && s[2..].bytes().all(|b| b.is_ascii_digit())
}
pub fn inventory(path: &Path) -> Result<Inventory> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let mut packs = BTreeSet::new();
    let mut bytes = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let entry_name = entry.name()?;
        let name = entry_name.trim_end_matches('/');
        valid_path(name)?;
        ensure!(
            names.insert(path_key("archive", name)),
            "Archive path collision"
        );
        ensure!(
            entry
                .unix_mode()
                .map(|m| m & 0o170000 != 0o120000)
                .unwrap_or(true),
            "Archive contains a symlink"
        );
        if entry.is_dir() {
            continue;
        }
        let parts: Vec<_> = name.split('/').collect();
        let first = parts.get(1).copied().unwrap_or("");
        let (class, pack) = if pack_id(first) {
            packs.insert(first.into());
            ("candidate_dlc", Some(first.to_string()))
        } else if first == "Delta" {
            (
                "unresolved_game_delta",
                parts.get(2).filter(|p| pack_id(p)).map(|p| p.to_string()),
            )
        } else if first == "Data" {
            ("unresolved_shared_base", None)
        } else {
            ("excluded_from_dlc_payload", None)
        };
        bytes = bytes
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        files.push(ArchiveEntry {
            path: name.into(),
            bytes: entry.size(),
            crc32: entry.crc32(),
            compression: format!("{:?}", entry.compression()),
            classification: class.into(),
            pack,
        });
    }
    Ok(Inventory {
        source: path.display().to_string(),
        source_bytes: fs::metadata(path)?.len(),
        files,
        payload_bytes: bytes,
        pack_ids: packs,
        production_ready: false,
    })
}
pub fn encode(input: &Path, objects: &Path) -> Result<Object> {
    fs::create_dir_all(objects)?;
    let (decoded_hash, decoded_size) = hash_file(input)?;
    let mut tmp = tempfile::NamedTempFile::new_in(objects)?;
    {
        let mut encoder = zstd::stream::write::Encoder::new(tmp.as_file_mut(), 3)?;
        std::io::copy(&mut File::open(input)?, &mut encoder)?;
        encoder.finish()?;
    }
    let codec = if tmp.as_file().metadata()?.len() < decoded_size {
        Codec::Zstd
    } else {
        tmp.as_file_mut().set_len(0)?;
        tmp.as_file_mut().rewind()?;
        std::io::copy(&mut File::open(input)?, tmp.as_file_mut())?;
        Codec::Raw
    };
    // Published objects are read by an unprivileged web server.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    tmp.as_file().sync_all()?;
    let (hash, encoded_size) = hash_file(tmp.path())?;
    let object = Object {
        hash: hash.clone(),
        encoded_size,
        codec,
        decoded_hash,
        decoded_size,
    };
    let mut verify = crate::storage::HashWriter::default();
    crate::transfer::decode(tmp.path(), &object, &mut verify, &Default::default())?;
    ensure!(
        verify.finish() == (object.decoded_hash.clone(), object.decoded_size),
        "Object round trip failed"
    );
    let dest = objects.join(hash);
    if dest.exists() {
        ensure!(
            hash_file(&dest)? == (object.hash.clone(), object.encoded_size),
            "Existing immutable object is corrupt"
        );
    } else {
        tmp.persist_noclobber(&dest).map_err(|e| e.error)?;
        crate::storage::sync_dir(objects)?;
    }
    Ok(object)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub manifest: Manifest,
    pub entries: BTreeMap<String, EntryDecision>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntryDecision {
    Include { file_id: String },
    Exclude { reason: String },
}
/// A recipe must account for every archive file. No executable from the archive is run.
pub fn prepare(archive: &Path, recipe: &Path, output: &Path) -> Result<Manifest> {
    let _lock = Lock::acquire(&output.join("prepare.lock"))?;
    let mut recipe: Recipe =
        serde_json::from_value(crate::strict_json::parse(&fs::read(recipe)?)?)?;
    let audit = inventory(archive)?;
    ensure!(
        audit.files.len() == recipe.entries.len(),
        "Recipe must account for every source file"
    );
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut included = BTreeSet::new();
    for entry in &audit.files {
        match recipe
            .entries
            .get(&entry.path)
            .context("Unclassified source file")?
        {
            EntryDecision::Exclude { reason } => {
                ensure!(!reason.trim().is_empty(), "Exclusion needs a reason")
            }
            EntryDecision::Include { file_id } => {
                ensure!(
                    included.insert(file_id.clone()),
                    "Multiple source files map to one destination"
                );
                ensure!(
                    !entry.path.to_lowercase().contains("anadius"),
                    "Third-party unlock components cannot be published"
                );
                let target = recipe
                    .manifest
                    .files
                    .iter_mut()
                    .find(|f| f.id == *file_id)
                    .context("Unknown recipe target")?;
                let mut source = zip.by_name(&entry.path)?;
                let mut temp = tempfile::NamedTempFile::new_in(output)?;
                let copied = std::io::copy(&mut source, &mut temp)?;
                ensure!(copied == entry.bytes, "Source size mismatch");
                temp.flush()?;
                let object = encode(temp.path(), &output.join("objects"))?;
                target.target_hash = object.decoded_hash.clone();
                target.target_size = object.decoded_size;
                target.full = object;
            }
        }
    }
    ensure!(
        included.len() == recipe.manifest.files.len(),
        "Recipe does not supply every target"
    );
    recipe.manifest.validate()?;
    atomic_write(
        &output.join("release.json"),
        &serde_json::to_vec_pretty(&recipe.manifest)?,
    )?;
    atomic_write(
        &output.join("inventory.json"),
        &serde_json::to_vec_pretty(&audit)?,
    )?;
    Ok(recipe.manifest)
}
pub(crate) fn timestamp_version(metadata: &Path) -> Result<Option<u64>> {
    let path = metadata.join("timestamp.json");
    match fs::read(&path) {
        Ok(bytes) => {
            let v = crate::strict_json::parse(&bytes)?;
            let version = v
                .pointer("/signed/version")
                .and_then(serde_json::Value::as_u64)
                .context("Live timestamp has no version")?;
            Ok(Some(version))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Every object a release references must already be served, with the declared bytes.
fn verify_objects(m: &Manifest, objects: &Path) -> Result<()> {
    let referenced = m
        .files
        .iter()
        .flat_map(|f| std::iter::once(&f.full).chain(f.deltas.iter().map(|d| &d.object)));
    for o in referenced {
        let path = objects.join(&o.hash);
        let actual =
            hash_file(&path).with_context(|| format!("Object {} is not uploaded", o.hash))?;
        ensure!(
            actual == (o.hash.clone(), o.encoded_size),
            "Object {} does not match its declared hash and size",
            o.hash
        );
    }
    Ok(())
}

/// Moves staged files into a live directory. Existing names are immutable: identical bytes are
/// accepted, different bytes stop promotion.
pub(crate) fn install_immutable(staged: &Path, live: &Path, skip: &str) -> Result<()> {
    fs::create_dir_all(live)?;
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy() == skip {
            continue;
        }
        let dst = live.join(&name);
        if dst.exists() {
            ensure!(
                hash_file(&entry.path())? == hash_file(&dst)?,
                "Refusing to replace immutable published file {}",
                dst.display()
            );
            continue;
        }
        fs::rename(entry.path(), &dst)?;
    }
    crate::storage::sync_dir(live)
}

/// Expiry of each live top-level metadata role, read from the promoted repository.
pub fn expiry(repository: &Path) -> Result<Vec<(String, jiff::Timestamp)>> {
    let metadata = repository.join("metadata");
    let version = timestamp_version(&metadata)?.context("No release has been promoted")?;
    let mut roles = Vec::new();
    for (role, file) in [
        ("timestamp", "timestamp.json".to_string()),
        ("snapshot", format!("{version}.snapshot.json")),
        ("targets", format!("{version}.targets.json")),
    ] {
        let v = crate::strict_json::parse(&fs::read(metadata.join(&file))?)?;
        let expires = v
            .pointer("/signed/expires")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{file} has no expiry"))?
            .parse()?;
        roles.push((role.to_string(), expires));
    }
    Ok(roles)
}

/// Signs `manifest` as the next release and promotes it into `repository`.
///
/// Order: verify objects, stage signed metadata, install targets and versioned metadata, then
/// replace `timestamp.json` last. Clients keep seeing the previous release until that final
/// rename, so a failure at any earlier point leaves the current release intact.
pub async fn publish(
    root: &Path,
    keys: Vec<PathBuf>,
    manifest: &Path,
    repository: &Path,
) -> Result<u64> {
    fs::create_dir_all(repository)?;
    let _lock = Lock::acquire(&repository.join("publish.lock"))?;
    let m = Manifest::parse(&fs::read(manifest)?)?;
    ensure!(
        m.variant == "fixture",
        "Production signing requires completed adapter validation; this build only signs fixtures"
    );
    verify_objects(&m, &repository.join("objects"))?;
    promote(root, keys, &[manifest.to_path_buf()], repository).await
}

/// Signs `targets` (files in one directory) as the next metadata version of `repository` and
/// promotes them, replacing `timestamp.json` last. The caller holds the publish lock and has
/// verified every object the targets reference.
pub(crate) async fn promote(
    root: &Path,
    keys: Vec<PathBuf>,
    targets: &[PathBuf],
    repository: &Path,
) -> Result<u64> {
    let source_dir = targets
        .first()
        .and_then(|t| t.parent())
        .context("Nothing to sign")?;
    ensure!(
        targets.iter().all(|t| t.parent() == Some(source_dir)),
        "Targets must share one directory"
    );
    let metadata = repository.join("metadata");
    let version = timestamp_version(&metadata)?.map_or(Ok(1), |v| {
        v.checked_add(1).context("Metadata version overflow")
    })?;
    let v = NonZeroU64::new(version).context("Version must be positive")?;
    let mut e = RepositoryEditor::new(root).await?;
    let now = jiff::Timestamp::now();
    let days = |n: u64| std::time::Duration::from_secs(n * 86400);
    e.targets_version(v)?
        .targets_expires(now + days(30))?
        .snapshot_version(v)
        .snapshot_expires(now + days(7))
        .timestamp_version(v)
        .timestamp_expires(now + days(1));
    for target in targets {
        e.add_target_path(target).await?;
    }
    let sources: Vec<Box<dyn KeySource>> = keys
        .into_iter()
        .map(|path| Box::new(LocalKeySource { path }) as Box<dyn KeySource>)
        .collect();
    let signed = e.sign(&sources).await?;
    let staging = tempfile::tempdir_in(repository)?;
    signed
        .copy_targets(source_dir, staging.path().join("targets"), PathExists::Fail)
        .await?;
    signed.write(staging.path().join("metadata")).await?;
    install_immutable(
        &staging.path().join("targets"),
        &repository.join("targets"),
        "",
    )?;
    install_immutable(
        &staging.path().join("metadata"),
        &metadata,
        "timestamp.json",
    )?;
    fs::rename(
        staging.path().join("metadata/timestamp.json"),
        metadata.join("timestamp.json"),
    )?;
    crate::storage::sync_dir(&metadata)?;
    Ok(version)
}

/// Re-signs every target of the current release with fresh expiry times. Each target is
/// verified against the trusted root first, so renewal can never sign bytes that were not
/// already published.
pub async fn renew(root: &Path, keys: Vec<PathBuf>, repository: &Path) -> Result<u64> {
    let _lock = Lock::acquire(&repository.join("publish.lock"))?;
    let metadata_dir = repository.join("metadata");
    let version = timestamp_version(&metadata_dir)?.context("No release has been promoted")?;
    let listed = crate::strict_json::parse(&fs::read(
        metadata_dir.join(format!("{version}.targets.json")),
    )?)?;
    let names: Vec<String> = listed
        .pointer("/signed/targets")
        .and_then(serde_json::Value::as_object)
        .context("Live targets metadata lists no targets")?
        .keys()
        .cloned()
        .collect();
    ensure!(!names.is_empty(), "Live release has no targets");
    let work = tempfile::tempdir()?;
    let url = |dir: &str| -> Result<url::Url> {
        url::Url::from_directory_path(fs::canonicalize(repository.join(dir))?)
            .map_err(|()| anyhow::anyhow!("Invalid {dir} path"))
    };
    let staged = work.path().join("targets");
    fs::create_dir_all(&staged)?;
    let mut files = Vec::new();
    for name in &names {
        crate::model::valid_id(name)?;
        let bytes = crate::trust::load_bytes(
            root,
            url("metadata")?,
            url("targets")?,
            name,
            &work.path().join("tuf"),
        )
        .await
        .with_context(|| format!("{name} does not verify; renewal refused"))?;
        let path = staged.join(name);
        fs::write(&path, bytes)?;
        files.push(path);
    }
    promote(root, keys, &files, repository).await
}
/// Generates independent local test keys. Never used as a production trust bootstrap.
pub async fn fixture_root(dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let key_path = dir.join("fixture-key.pem");
    ensure!(!key_path.exists(), "Fixture key already exists");
    let document =
        aws_lc_rs::signature::Ed25519KeyPair::generate_pkcs8(&aws_lc_rs::rand::SystemRandom::new())
            .map_err(|e| anyhow::anyhow!("Key generation failed: {e}"))?;
    fs::write(&key_path, document.as_ref())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    }
    let key = LocalKeySource {
        path: key_path.clone(),
    };
    let signer = key
        .as_sign()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let public = signer.tuf_key();
    let id = public.key_id()?;
    let role = serde_json::json!({"keyids":[id],"threshold":1});
    let expires =
        (jiff::Timestamp::now() + std::time::Duration::from_secs(365 * 86400)).to_string();
    let key_id = serde_json::to_value(&id)?
        .as_str()
        .context("Invalid key id")?
        .to_owned();
    let root: Root = serde_json::from_value(serde_json::json!({
        "_type": "root",
        "spec_version": "1.0.0",
        "version": 1,
        "expires": expires,
        "consistent_snapshot": true,
        "keys": { key_id: public },
        "roles": {
            "root": role,
            "targets": role,
            "snapshot": role,
            "timestamp": role
        }
    }))?;
    let signed = SignedRole::new(
        root.clone(),
        &KeyHolder::Root(root),
        &[Box::new(key)],
        &aws_lc_rs::rand::SystemRandom::new(),
    )
    .await?;
    let path = dir.join("root.json");
    atomic_write(&path, signed.buffer())?;
    Ok(path)
}
