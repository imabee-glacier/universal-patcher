//! Packaging and publishing patcher builds, and the one-line install scripts.
//!
//! Repository layout under the published origin:
//! `install.sh`, `install.ps1` - generated scripts with the origin filled in
//! `app/objects/<hash>` - one tar archive per platform build
//! `app/metadata/`, `app/targets/` - TUF repository signing `app-<os>-<arch>.json`
//! `app/bootstrap/<v>/manifest.json` - SHA-256 of each platform's engine, for the scripts
//! `app/bootstrap/<v>/<os>-<arch>/patcher-engine[.exe]` - the engine the scripts run
//! `app/bootstrap/latest` - moved after everything it points to is in place
//! `app/setup/<v>/UniversalPatcherSetup-<v>.exe` - the Windows graphical installer, when supplied
//! `site/data/catalog.json` - the website's download and release listing, written last
use crate::{
    model::{AppManifest, Codec},
    publisher::{encode, install_immutable, promote},
    storage::{Lock, atomic_write_public as atomic_write, hash_file, sync_dir},
};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const INSTALL_SH: &str = include_str!("../../distribution/install.sh");
const INSTALL_PS1: &str = include_str!("../../distribution/install.ps1");

fn engine_name(os: &str) -> &'static str {
    if os == "windows" {
        "patcher-engine.exe"
    } else {
        "patcher-engine"
    }
}

/// Files a build must contain, relative to its bundle directory.
/// The second entry is always the engine the install scripts run.
fn required_files(os: &str) -> Result<&'static [&'static str]> {
    match os {
        "linux" => Ok(&["patcher", "lib/patcher-engine"]),
        "windows" => Ok(&[
            "patcher.exe",
            "patcher-engine.exe",
            "patcher-launch.exe",
            "component/version.dll",
        ]),
        _ => bail!("Packaging for {os} is not yet validated"),
    }
}

pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut ctx = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256);
    let mut file = File::open(path)?;
    let mut buf = [0u8; 128 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        size += n as u64;
    }
    let hex: String = ctx
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((hex, size))
}

fn walk(dir: &Path, rel: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let full = entry.path();
        let name = rel.join(entry.file_name());
        let kind = entry.file_type()?;
        ensure!(
            kind.is_file() || kind.is_dir() || kind.is_symlink(),
            "{} is not a regular file, directory or link",
            full.display()
        );
        out.push((full.clone(), name.clone()));
        if kind.is_dir() {
            walk(&full, &name, out)?;
        }
    }
    Ok(())
}

/// Writes a reproducible tar of `bundle`: sorted entries, zeroed owners and times, no sparse entries.
fn archive(bundle: &Path, output: &Path) -> Result<()> {
    let mut entries = Vec::new();
    walk(bundle, Path::new(""), &mut entries)?;
    let mut builder = tar::Builder::new(File::create(output)?);
    builder.mode(tar::HeaderMode::Deterministic);
    builder.follow_symlinks(false);
    // Holes are written out as zeros: the installer accepts only plain file entries.
    builder.sparse(false);
    for (full, name) in entries {
        builder.append_path_with_name(&full, &name)?;
    }
    builder.into_inner()?.sync_all()?;
    Ok(())
}

/// Packages one platform build into `staging`: its object, its signed-to-be manifest, and the
/// engine copy and checksum the install scripts use.
pub fn package_app(
    bundle: &Path,
    version: &str,
    os: &str,
    arch: &str,
    staging: &Path,
) -> Result<AppManifest> {
    ensure!(
        crate::model::game_runs_on(os, arch),
        "The Sims 4 does not run on {os}-{arch}; nothing to package"
    );
    for file in required_files(os)?.iter() {
        ensure!(
            bundle.join(file).is_file(),
            "Build is missing {file}: {}",
            bundle.display()
        );
    }
    fs::create_dir_all(staging)?;
    let tar_path = staging.join(format!(".{os}-{arch}.tar"));
    archive(bundle, &tar_path)?;
    let artifact = encode(&tar_path, &staging.join("objects"));
    fs::remove_file(&tar_path)?;
    let artifact = artifact?;
    let codec = match artifact.codec {
        Codec::Raw => "raw",
        Codec::Zstd => "zstd",
    };
    let manifest = AppManifest {
        schema: 1,
        app_version: version.into(),
        os: os.into(),
        arch: arch.into(),
        required_capabilities: vec!["app-v1".into(), codec.into()],
        artifact,
        state_schema_range: [crate::app::STATE_SCHEMA, crate::app::STATE_SCHEMA],
    };
    manifest.validate()?;
    atomic_write(
        &staging.join(format!("app-{os}-{arch}.json")),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;

    let platform = format!("{os}-{arch}");
    let engine_dir = staging.join("bootstrap").join(version).join(&platform);
    fs::create_dir_all(&engine_dir)?;
    let engine_source = bundle.join(required_files(os)?[1]);
    let engine = engine_dir.join(engine_name(os));
    fs::copy(&engine_source, &engine)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o755))?;
    }
    let (sha256, size) = sha256_file(&engine)?;
    let manifest_path = staging
        .join("bootstrap")
        .join(version)
        .join("manifest.json");
    let mut bootstrap: Value = match fs::read(&manifest_path) {
        Ok(bytes) => crate::strict_json::parse(&bytes)?,
        Err(_) => json!({ "version": version, "platforms": {} }),
    };
    ensure!(
        bootstrap["version"] == version,
        "Staging already holds a different version"
    );
    bootstrap["platforms"][&platform] = json!({ "sha256": sha256, "size": size });
    atomic_write(&manifest_path, &serde_json::to_vec_pretty(&bootstrap)?)?;
    Ok(manifest)
}

/// Accepts only characters that are inert inside both generated scripts.
pub fn check_origin(origin: &str) -> Result<url::Url> {
    ensure!(
        origin.ends_with('/')
            && origin
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b":/._~-".contains(&b)),
        "Origin must be a plain URL ending in /"
    );
    let url: url::Url = origin.parse()?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Origin requires HTTPS"
    );
    Ok(url)
}

pub fn render_scripts(origin: &str) -> Result<(String, String)> {
    check_origin(origin)?;
    Ok((
        INSTALL_SH.replace("@ORIGIN@", origin),
        INSTALL_PS1.replace("@ORIGIN@", origin),
    ))
}

fn copy_tree_immutable(staged: &Path, live: &Path) -> Result<()> {
    fs::create_dir_all(live)?;
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_tree_immutable(&entry.path(), &live.join(entry.file_name()))?;
        }
    }
    let flat = tempfile::tempdir_in(live)?;
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), flat.path().join(entry.file_name()))?;
        }
    }
    install_immutable(flat.path(), live, "")
}

/// What the website shows for a release, beyond the signed builds themselves.
#[derive(Debug, Default, Clone)]
pub struct Listing {
    /// Platforms (`<os>-<arch>`) the website offers for download, from `distribution/offer`.
    /// Other packaged platforms are listed with `offered: false`, and the website offers no
    /// download for them. Being offered records a release decision, not a test result.
    pub offered: Vec<String>,
    /// Release notes, one paragraph per entry.
    pub notes: Vec<String>,
    /// The Windows graphical installer for this version.
    pub windows_setup: Option<PathBuf>,
}

pub fn setup_name(version: &str) -> String {
    format!("UniversalPatcherSetup-{version}.exe")
}

/// The website names architectures as `uname -m` does.
fn website_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "x64" => Ok("x86_64"),
        "arm64" => Ok("aarch64"),
        _ => bail!("No website name for architecture {arch}"),
    }
}

/// Builds `site/data/catalog.json`: current downloads, and release notes accumulated across
/// releases. Pack listings are carried over unchanged; game releases own them.
fn catalog(
    previous: Option<Value>,
    origin: &str,
    version: &str,
    manifests: &[&AppManifest],
    listing: &Listing,
    now: jiff::Timestamp,
) -> Result<Value> {
    let previous = previous.unwrap_or_else(|| json!({}));
    let mut releases: Vec<Value> = previous["releases"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r["version"] != version)
        .collect();
    releases.push(json!({ "version": version, "date": now.to_string(), "notes": listing.notes }));
    let mut downloads = Vec::new();
    for m in manifests {
        let platform = format!("{}-{}", m.os, m.arch);
        let script = if m.os == "windows" {
            "install.ps1"
        } else {
            "install.sh"
        };
        let url = match (&m.os[..], &listing.windows_setup) {
            ("windows", Some(_)) => format!("{origin}app/setup/{version}/{}", setup_name(version)),
            _ => format!("{origin}{script}"),
        };
        downloads.push(json!({
            "os": m.os,
            "arch": website_arch(&m.arch)?,
            "offered": listing.offered.contains(&platform),
            "version": version,
            "url": url,
            "install_script": format!("{origin}{script}"),
        }));
    }
    Ok(json!({
        "schema": 1,
        "published_at": now.to_string(),
        "downloads": downloads,
        "packs": previous["packs"].as_array().cloned().unwrap_or_default(),
        "releases": releases,
    }))
}

/// Publishes every packaged platform in `staging` as one application release.
pub async fn publish_app(
    root: &Path,
    keys: Vec<PathBuf>,
    staging: &Path,
    repository: &Path,
    origin: &str,
    listing: &Listing,
) -> Result<(String, u64)> {
    check_origin(origin)?;
    let app = repository.join("app");
    fs::create_dir_all(&app)?;
    let _lock = Lock::acquire(&app.join("publish.lock"))?;

    let mut manifests = BTreeMap::new();
    for entry in fs::read_dir(staging)? {
        let path = entry?.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        if let Some(name) = name.filter(|n| n.starts_with("app-") && n.ends_with(".json")) {
            let m = AppManifest::parse(&fs::read(&path)?)?;
            ensure!(
                name == format!("app-{}-{}.json", m.os, m.arch),
                "{name} does not match its platform"
            );
            manifests.insert(path, m);
        }
    }
    ensure!(
        !manifests.is_empty(),
        "No packaged builds in {}",
        staging.display()
    );
    let version = manifests
        .values()
        .next()
        .map(|m| m.app_version.clone())
        .unwrap_or_default();
    ensure!(
        manifests.values().all(|m| m.app_version == version),
        "Packaged builds disagree on the version"
    );
    let platforms: Vec<String> = manifests
        .values()
        .map(|m| format!("{}-{}", m.os, m.arch))
        .collect();
    for offered in &listing.offered {
        ensure!(
            platforms.contains(offered),
            "Cannot offer {offered}: no build for it is packaged"
        );
    }
    ensure!(
        listing.notes.iter().all(|n| !n.trim().is_empty()),
        "Release notes cannot contain empty paragraphs"
    );
    if let Some(setup) = &listing.windows_setup {
        ensure!(
            manifests.values().any(|m| m.os == "windows"),
            "A Windows installer needs a packaged Windows build"
        );
        ensure!(setup.is_file(), "Missing installer {}", setup.display());
    }
    if let Ok(latest) = fs::read_to_string(app.join("bootstrap/latest")) {
        ensure!(
            crate::model::parse_version(&version)? > crate::model::parse_version(latest.trim())?,
            "Version {version} is not newer than the published {}",
            latest.trim()
        );
    }

    // Copied rather than moved: staging may be read-only or on another filesystem.
    copy_tree_immutable(&staging.join("objects"), &app.join("objects"))?;
    for m in manifests.values() {
        let served = app.join("objects").join(&m.artifact.hash);
        ensure!(
            hash_file(&served)? == (m.artifact.hash.clone(), m.artifact.encoded_size),
            "Object for {}-{} does not match its manifest",
            m.os,
            m.arch
        );
    }
    let bootstrap = staging.join("bootstrap").join(&version);
    let listed: Value = crate::strict_json::parse(&fs::read(bootstrap.join("manifest.json"))?)?;
    for m in manifests.values() {
        let platform = format!("{}-{}", m.os, m.arch);
        let entry = &listed["platforms"][&platform];
        let engine = bootstrap.join(&platform).join(engine_name(&m.os));
        let (sha256, size) = sha256_file(&engine)?;
        ensure!(
            entry["sha256"] == sha256.as_str() && entry["size"] == size,
            "Bootstrap checksum for {platform} does not match its engine"
        );
    }
    copy_tree_immutable(&bootstrap, &app.join("bootstrap").join(&version))?;
    if let Some(setup) = &listing.windows_setup {
        let flat = tempfile::tempdir_in(&app)?;
        let staged = flat.path().join(setup_name(&version));
        fs::copy(setup, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o644))?;
        }
        install_immutable(flat.path(), &app.join("setup").join(&version), "")?;
    }

    let targets: Vec<PathBuf> = manifests.keys().cloned().collect();
    let metadata_version = promote(root, keys, &targets, &app).await?;

    let (sh, ps1) = render_scripts(origin)?;
    atomic_write(&repository.join("install.sh"), sh.as_bytes())?;
    atomic_write(&repository.join("install.ps1"), ps1.as_bytes())?;
    atomic_write(
        &app.join("bootstrap/latest"),
        format!("{version}\n").as_bytes(),
    )?;
    sync_dir(&app.join("bootstrap"))?;

    let catalog_path = repository.join("site/data/catalog.json");
    let previous = match fs::read(&catalog_path) {
        Ok(bytes) => Some(crate::strict_json::parse(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let listed: Vec<&AppManifest> = manifests.values().collect();
    let site = catalog(
        previous,
        origin,
        &version,
        &listed,
        listing,
        jiff::Timestamp::now(),
    )?;
    atomic_write(&catalog_path, &serde_json::to_vec_pretty(&site)?)?;
    Ok((version, metadata_version))
}
