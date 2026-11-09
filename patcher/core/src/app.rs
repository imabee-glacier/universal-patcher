//! Installation and self-update of the patcher itself.
//!
//! Layout under the per-user application-data directory:
//! `versions/<v>/` one unpacked release each, `install.json` the pointers (current, previous,
//! staged, trial), `bin/` the stable launcher, `health/<v>` written by a release once its
//! interface has started.
use crate::{
    model::{AppManifest, Object, parse_version},
    storage::{Lock, atomic_write, checked_bytes, read_checked, sync_dir},
    transfer::{Control, HttpSource, ObjectSource},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use url::Url;

/// Local state schema this build reads and writes.
pub const STATE_SCHEMA: u32 = 1;
const DIST_ORIGIN: &str = include_str!(concat!(env!("OUT_DIR"), "/dist_origin.txt"));
const DIST_ROOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dist_root.json"));
/// How long a newly activated release has to show its interface.
pub const STARTUP_CHECK: Duration = Duration::from_secs(60);

pub fn platform() -> Result<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => bail!("Unsupported operating system: {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => bail!("Unsupported architecture: {other}"),
    };
    Ok((os, arch))
}

/// Where releases come from and which root of trust verifies them.
pub struct Source {
    pub origin: Url,
    pub root: Vec<u8>,
}

impl Source {
    /// Uses the values baked into this build unless overridden.
    pub fn configured(origin: Option<&str>, root: Option<&Path>) -> Result<Self> {
        let origin = origin.unwrap_or(DIST_ORIGIN.trim());
        ensure!(
            !origin.is_empty(),
            "This build has no download origin; pass --origin"
        );
        let mut origin: Url = origin.parse().context("Invalid download origin")?;
        if !origin.path().ends_with('/') {
            let path = format!("{}/", origin.path());
            origin.set_path(&path);
        }
        let root = match root {
            Some(path) => fs::read(path)?,
            None => DIST_ROOT.to_vec(),
        };
        ensure!(
            !root.is_empty(),
            "This build has no trusted root; pass --root"
        );
        Ok(Self { origin, root })
    }

    fn app(&self) -> Result<Url> {
        Ok(self.origin.join("app/")?)
    }
}

/// Paths of one per-user installation.
#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .context("Home directory is unavailable")
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
fn data_home() -> Result<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => Ok(home()?.join(".local/share")),
    }
}

impl Layout {
    /// The installation for the current user. `PATCHER_HOME` overrides the location.
    pub fn for_user() -> Result<Self> {
        if let Some(dir) = std::env::var_os("PATCHER_HOME").filter(|d| !d.is_empty()) {
            return Ok(Self::at(PathBuf::from(dir)));
        }
        let root = match platform()?.0 {
            "linux" => data_home()?.join("universal-patcher"),
            "macos" => home()?.join("Library/Application Support/Universal Patcher"),
            _ => PathBuf::from(
                std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?,
            )
            .join("UniversalPatcher"),
        };
        Ok(Self::at(root))
    }
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn version(&self, v: &str) -> PathBuf {
        self.root.join("versions").join(v)
    }
    pub fn launcher(&self) -> PathBuf {
        let name = if cfg!(windows) {
            "universal-patcher.exe"
        } else {
            "universal-patcher"
        };
        self.root.join("bin").join(name)
    }
    /// Game maintenance configuration written when an installation is confirmed.
    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }
    pub fn health(&self, v: &str) -> PathBuf {
        self.root.join("health").join(v)
    }
    fn state_path(&self) -> PathBuf {
        self.root.join("install.json")
    }
}

/// Executable paths inside an unpacked release.
fn app_executable(version_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        version_dir.join("patcher.exe")
    } else {
        version_dir.join("patcher")
    }
}
fn engine_executable(version_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        version_dir.join("patcher-engine.exe")
    } else {
        version_dir.join("lib/patcher-engine")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallState {
    pub current: Option<String>,
    pub previous: Option<String>,
    /// Downloaded and verified, activated at the next launch.
    pub staged: Option<String>,
    /// Activated but not yet confirmed by a startup check.
    pub trial: Option<String>,
    /// Releases that failed their startup check; never activated again.
    pub rejected: Vec<String>,
}

impl InstallState {
    /// Loads the pointers. A damaged file falls back to the newest complete release on disk.
    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.state_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        if let Ok(state) = read_checked::<Self>(&path) {
            return Ok(state);
        }
        let mut versions = installed_versions(layout)?;
        versions.sort_by_key(|v| parse_version(v).unwrap_or_default());
        Ok(Self {
            current: versions.pop(),
            ..Default::default()
        })
    }
    pub fn save(&self, layout: &Layout) -> Result<()> {
        atomic_write(&layout.state_path(), &checked_bytes(self)?)
    }
}

fn installed_versions(layout: &Layout) -> Result<Vec<String>> {
    let dir = layout.root.join("versions");
    let mut found = Vec::new();
    match fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if parse_version(&name).is_ok() {
                    found.push(name);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(found)
}

fn newer(candidate: &str, than: Option<&str>) -> Result<bool> {
    Ok(match than {
        None => true,
        Some(current) => parse_version(candidate)? > parse_version(current)?,
    })
}

/// Fetches and verifies this platform's application manifest through TUF.
pub fn fetch_manifest(source: &Source, layout: &Layout) -> Result<AppManifest> {
    let (os, arch) = platform()?;
    fs::create_dir_all(&layout.root)?;
    let root_path = layout.root.join("app-root.json");
    atomic_write(&root_path, &source.root)?;
    let app = source.app()?;
    let bytes = tokio::runtime::Runtime::new()?.block_on(crate::trust::load_bytes(
        &root_path,
        app.join("metadata/")?,
        app.join("targets/")?,
        &format!("app-{os}-{arch}.json"),
        &layout.root.join("tuf-app"),
    ))?;
    let m = AppManifest::parse(&bytes)?;
    ensure!(
        m.os == os && m.arch == arch,
        "Release is for {}-{}, not {os}-{arch}",
        m.os,
        m.arch
    );
    ensure!(
        (m.state_schema_range[0]..=m.state_schema_range[1]).contains(&STATE_SCHEMA),
        "Release {} cannot read this installation's saved state",
        m.app_version
    );
    Ok(m)
}

/// Counts and hashes decoded bytes, failing as soon as the declared size is exceeded.
struct Verified<R> {
    inner: R,
    hasher: blake3::Hasher,
    bytes: u64,
    limit: u64,
}
impl<R: Read> Read for Verified<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes += n as u64;
        if self.bytes > self.limit {
            return Err(std::io::Error::other("Archive exceeds its declared size"));
        }
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// Rejects any entry that could land outside `dest`.
fn safe_entry_path(path: &Path) -> Result<()> {
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_)))
            && path.components().next().is_some(),
        "Archive entry has an unsafe path: {}",
        path.display()
    );
    Ok(())
}

fn safe_link_target(entry: &Path, target: &Path) -> Result<()> {
    ensure!(
        target.is_relative(),
        "Archive link {} is absolute",
        entry.display()
    );
    let mut depth: i64 = entry.components().count() as i64 - 1;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => depth -= 1,
            _ => bail!("Archive link {} is unsafe", entry.display()),
        }
        ensure!(
            depth >= 0,
            "Archive link {} leaves the release",
            entry.display()
        );
    }
    Ok(())
}

/// Streams an object through decoding, hashing and safe extraction into `dest`.
pub fn unpack(object_path: &Path, o: &Object, dest: &Path) -> Result<()> {
    let file = File::open(object_path)?;
    let reader: Box<dyn Read> = match o.codec {
        crate::model::Codec::Raw => Box::new(file),
        crate::model::Codec::Zstd => {
            let mut d = zstd::stream::read::Decoder::new(file)?;
            d.window_log_max(26)?;
            Box::new(d)
        }
    };
    let mut verified = Verified {
        inner: reader,
        hasher: blake3::Hasher::new(),
        bytes: 0,
        limit: o.decoded_size,
    };
    fs::create_dir_all(dest)?;
    {
        let mut archive = tar::Archive::new(&mut verified);
        archive.set_overwrite(false);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            safe_entry_path(&path)?;
            match entry.header().entry_type() {
                tar::EntryType::Regular | tar::EntryType::Directory => {}
                tar::EntryType::Symlink if cfg!(unix) => {
                    let target = entry
                        .link_name()?
                        .context("Archive link has no target")?
                        .into_owned();
                    safe_link_target(&path, &target)?;
                }
                other => bail!(
                    "Archive entry {} has unsupported type {other:?}",
                    path.display()
                ),
            }
            ensure!(
                entry.unpack_in(dest)?,
                "Archive entry {} was refused",
                path.display()
            );
        }
    }
    std::io::copy(&mut verified, &mut std::io::sink())?;
    ensure!(
        verified.bytes == o.decoded_size
            && verified.hasher.finalize().to_hex().as_str() == o.decoded_hash,
        "Release archive does not match its signed hash"
    );
    Ok(())
}

fn sync_tree(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            sync_tree(&entry.path())?;
        } else if kind.is_file() {
            crate::storage::sync_file(&entry.path())?;
        }
    }
    sync_dir(dir)
}

fn remove_dir_if_present(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Downloads and unpacks a release into `versions/<v>/` unless it is already there.
pub fn stage(source: &Source, layout: &Layout, m: &AppManifest, control: &Control) -> Result<()> {
    let dest = layout.version(&m.app_version);
    if dest.is_dir() {
        return Ok(());
    }
    let versions = layout.root.join("versions");
    fs::create_dir_all(&versions)?;
    let needed = m
        .artifact
        .encoded_size
        .saturating_add(m.artifact.decoded_size)
        .saturating_add(64 * 1024 * 1024);
    ensure!(
        fs2::available_space(&versions)? >= needed,
        crate::transfer::Blocker::DiskSpace
    );
    let downloads = layout.root.join("downloads");
    let object =
        HttpSource::new(source.app()?, 0).fetch(&m.artifact, &downloads, control, &|_, _| {})?;
    let partial = versions.join(format!(".partial-{}", m.app_version));
    remove_dir_if_present(&partial)?;
    if let Err(e) = unpack(&object, &m.artifact, &partial) {
        remove_dir_if_present(&partial)?;
        return Err(e);
    }
    ensure!(
        app_executable(&partial).is_file() && engine_executable(&partial).is_file(),
        "Release {} is missing its program files",
        m.app_version
    );
    sync_tree(&partial)?;
    fs::rename(&partial, &dest)?;
    sync_dir(&versions)?;
    fs::remove_file(object)?;
    Ok(())
}

/// Replaces the stable launcher with the engine from release `v`.
fn install_launcher(layout: &Layout, v: &str) -> Result<()> {
    let launcher = layout.launcher();
    let bin = launcher.parent().context("Launcher has no directory")?;
    fs::create_dir_all(bin)?;
    let fresh = bin.join(".launcher.new");
    fs::copy(engine_executable(&layout.version(v)), &fresh)?;
    crate::storage::sync_file(&fresh)?;
    if cfg!(windows) && launcher.exists() {
        // A running executable cannot be replaced on Windows, but it can be renamed.
        let old = bin.join("universal-patcher.old.exe");
        let _ = fs::remove_file(&old);
        fs::rename(&launcher, &old)?;
    }
    fs::rename(&fresh, &launcher)?;
    sync_dir(bin)
}

/// Rejects characters that desktop entries and shells would reinterpret.
fn plain(path: &Path) -> Result<String> {
    let text = path.to_str().context("Path is not valid UTF-8")?;
    ensure!(
        !text.contains(['"', '`', '$', '\\', '\n', '%']),
        "Installation path contains characters that cannot be quoted safely: {text}"
    );
    Ok(text.to_owned())
}

/// The desktop entry text for the Linux applications menu.
pub fn desktop_entry(launcher: &Path, icon: Option<&Path>) -> Result<String> {
    let mut text = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Universal Patcher\n\
         Comment=Keeps The Sims 4 updated and repaired\n\
         Exec=\"{}\" launch\n\
         Terminal=false\n\
         Categories=Game;Utility;\n",
        plain(launcher)?
    );
    if let Some(icon) = icon {
        text.push_str(&format!("Icon={}\n", plain(icon)?));
    }
    Ok(text)
}

fn linux_paths() -> Result<(PathBuf, PathBuf)> {
    Ok((
        home()?.join(".local/bin/universal-patcher"),
        data_home()?.join("applications/universal-patcher.desktop"),
    ))
}

fn powershell(script: &str, env: &[(&str, &Path)]) -> Result<()> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status().context("PowerShell is unavailable")?;
    ensure!(status.success(), "PowerShell integration step failed");
    Ok(())
}

const WINDOWS_INTEGRATE: &str = r#"
$ErrorActionPreference = 'Stop'
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($env:UP_SHORTCUT)
$link.TargetPath = $env:UP_LAUNCHER
$link.Arguments = 'launch'
$link.WorkingDirectory = $env:UP_ROOT
$link.Save()
$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($path -split ';') -contains $env:UP_BIN)) {
    $next = (@($path.TrimEnd(';'), $env:UP_BIN) | Where-Object { $_ }) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $next, 'User')
}
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\UniversalPatcher'
New-Item -Path $key -Force | Out-Null
Set-ItemProperty -Path $key -Name DisplayName -Value 'Universal Patcher'
Set-ItemProperty -Path $key -Name DisplayVersion -Value $env:UP_VERSION
Set-ItemProperty -Path $key -Name InstallLocation -Value $env:UP_ROOT
Set-ItemProperty -Path $key -Name UninstallString -Value ('"' + $env:UP_LAUNCHER + '" uninstall')
Set-ItemProperty -Path $key -Name NoModify -Value 1 -Type DWord
Set-ItemProperty -Path $key -Name NoRepair -Value 1 -Type DWord
"#;

const WINDOWS_REMOVE: &str = r#"
$ErrorActionPreference = 'Stop'
Remove-Item -LiteralPath $env:UP_SHORTCUT -ErrorAction SilentlyContinue
$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($path) {
    $next = (($path -split ';') | Where-Object { $_ -and $_ -ne $env:UP_BIN }) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $next, 'User')
}
Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\UniversalPatcher' -Recurse -ErrorAction SilentlyContinue
"#;

fn windows_shortcut() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").context("APPDATA is unavailable")?)
            .join("Microsoft/Windows/Start Menu/Programs/Universal Patcher.lnk"),
    )
}

/// Adds the menu entry and the command on the user's path. Returns notes for the user.
fn integrate(layout: &Layout, v: &str) -> Result<Vec<String>> {
    let launcher = layout.launcher();
    let mut notes = Vec::new();
    match platform()?.0 {
        "linux" => {
            let (link, desktop) = linux_paths()?;
            fs::create_dir_all(link.parent().context("No bin directory")?)?;
            match fs::symlink_metadata(&link) {
                Ok(_) if fs::read_link(&link).ok().as_deref() == Some(launcher.as_path()) => {}
                Ok(_) => notes.push(format!(
                    "{} already exists and was left unchanged.",
                    link.display()
                )),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&launcher, &link)?;
                }
                Err(e) => return Err(e.into()),
            }
            let icon_source = layout.version(v).join("data/icon.svg");
            let icon = layout.root.join("share/icon.svg");
            let icon = if icon_source.is_file() {
                fs::create_dir_all(icon.parent().context("No share directory")?)?;
                fs::copy(&icon_source, &icon)?;
                Some(icon)
            } else {
                None
            };
            let entry = desktop_entry(&launcher, icon.as_deref())?;
            atomic_write(&desktop, entry.as_bytes())?;
            let on_path = std::env::var_os("PATH").is_some_and(|p| {
                std::env::split_paths(&p).any(|d| Some(d.as_path()) == link.parent())
            });
            if !on_path {
                notes.push(format!(
                    "Add {} to your PATH to run universal-patcher from a terminal.",
                    link.parent()
                        .map(Path::display)
                        .map(|d| d.to_string())
                        .unwrap_or_default()
                ));
            }
        }
        "windows" => {
            let bin = launcher.parent().context("Launcher has no directory")?;
            let version = PathBuf::from(v);
            powershell(
                WINDOWS_INTEGRATE,
                &[
                    ("UP_SHORTCUT", &windows_shortcut()?),
                    ("UP_LAUNCHER", &launcher),
                    ("UP_ROOT", &layout.root),
                    ("UP_BIN", bin),
                    ("UP_VERSION", &version),
                ],
            )?;
            notes.push("Open a new terminal to use the universal-patcher command.".into());
        }
        _ => bail!("Desktop integration for this platform is not yet validated"),
    }
    Ok(notes)
}

fn disintegrate(layout: &Layout) -> Result<()> {
    let launcher = layout.launcher();
    match platform()?.0 {
        "linux" => {
            let (link, desktop) = linux_paths()?;
            if fs::read_link(&link).ok().as_deref() == Some(launcher.as_path()) {
                fs::remove_file(&link)?;
            }
            match fs::remove_file(&desktop) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        "windows" => {
            let bin = launcher.parent().context("Launcher has no directory")?;
            powershell(
                WINDOWS_REMOVE,
                &[("UP_SHORTCUT", &windows_shortcut()?), ("UP_BIN", bin)],
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// Removes releases other than the current, previous and staged ones.
fn prune(layout: &Layout, state: &InstallState) -> Result<()> {
    let keep = [&state.current, &state.previous, &state.staged];
    for v in installed_versions(layout)? {
        if !keep.iter().any(|k| k.as_deref() == Some(v.as_str())) {
            remove_dir_if_present(&layout.version(&v))?;
        }
    }
    Ok(())
}

/// First installation, or reinstallation of the latest release.
pub fn install(source: &Source, layout: &Layout) -> Result<(String, Vec<String>)> {
    let (os, _) = platform()?;
    ensure!(
        os != "macos",
        "macOS installation is not yet available: no validated macOS build has been published"
    );
    fs::create_dir_all(&layout.root)?;
    let _lock = Lock::acquire(&layout.root.join("install.lock"))?;
    let m = fetch_manifest(source, layout)?;
    stage(source, layout, &m, &Control::default())?;
    let v = m.app_version.clone();
    let mut state = InstallState::load(layout)?;
    if state.current.as_deref() != Some(v.as_str()) {
        state.previous = state.current.take();
        state.current = Some(v.clone());
    }
    state.staged = None;
    state.trial = None;
    state.rejected.retain(|r| r != &v);
    install_launcher(layout, &v)?;
    state.save(layout)?;
    let notes = integrate(layout, &v)?;
    prune(layout, &state)?;
    Ok((v, notes))
}

/// Downloads a newer release for activation at the next launch.
pub fn update(source: &Source, layout: &Layout, control: &Control) -> Result<Option<String>> {
    let _lock = Lock::acquire(&layout.root.join("install.lock"))?;
    let mut state = InstallState::load(layout)?;
    ensure!(
        state.current.is_some(),
        "Universal Patcher is not installed"
    );
    let m = fetch_manifest(source, layout)?;
    let v = m.app_version.clone();
    let current = state.current.as_deref();
    if !newer(&v, current)?
        || state.rejected.contains(&v)
        || state.staged.as_deref() == Some(v.as_str())
    {
        return Ok(None);
    }
    stage(source, layout, &m, control)?;
    state.staged = Some(v.clone());
    state.save(layout)?;
    prune(layout, &state)?;
    Ok(Some(v))
}

/// True while game maintenance holds its session lock.
fn maintenance_running(layout: &Layout) -> bool {
    let Ok(bytes) = fs::read(layout.config()) else {
        return false;
    };
    let Ok(cfg) = crate::strict_json::parse(&bytes)
        .and_then(|v| Ok(serde_json::from_value::<crate::fixture::Config>(v)?))
    else {
        return false;
    };
    Lock::acquire(&cfg.state.join("session.lock")).is_err()
}

fn spawn(layout: &Layout, v: &str, args: &[String]) -> Result<std::process::Child> {
    let dir = layout.version(v);
    let health = layout.health(v);
    fs::create_dir_all(health.parent().context("No health directory")?)?;
    match fs::remove_file(&health) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let mut cmd = Command::new(app_executable(&dir));
    cmd.args(args)
        .current_dir(&dir)
        .env("PATCHER_HOME", &layout.root)
        .env("PATCHER_APP_VERSION", v)
        .env("PATCHER_HEALTH_FILE", &health);
    if std::env::var_os("PATCHER_CONFIG").is_none() && layout.config().is_file() {
        cmd.env("PATCHER_CONFIG", layout.config());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
        .with_context(|| format!("Cannot start Universal Patcher {v}"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Launched {
    Started(String),
    /// A new release failed its startup check; the previous one was started instead.
    RolledBack {
        failed: String,
        running: String,
    },
}

/// Activates a staged release when safe, starts the current release, and returns a failed
/// release to the previous one.
pub fn launch(layout: &Layout, args: &[String], startup_check: Duration) -> Result<Launched> {
    let mut state = {
        let _lock = Lock::acquire(&layout.root.join("install.lock"))?;
        let mut state = InstallState::load(layout)?;
        if let Some(v) = state.staged.clone()
            && !state.rejected.contains(&v)
            && layout.version(&v).is_dir()
            && !maintenance_running(layout)
        {
            state.previous = state.current.replace(v.clone());
            state.staged = None;
            state.trial = Some(v.clone());
            install_launcher(layout, &v)?;
            state.save(layout)?;
        }
        state
    };
    let current = state
        .current
        .clone()
        .context("Universal Patcher is not installed")?;
    let mut child = spawn(layout, &current, args)?;
    if state.trial.as_deref() != Some(current.as_str()) {
        return Ok(Launched::Started(current));
    }
    let started = Instant::now();
    let healthy = loop {
        if layout.health(&current).is_file() {
            break true;
        }
        if child.try_wait()?.is_some() || started.elapsed() >= startup_check {
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _lock = Lock::acquire(&layout.root.join("install.lock"))?;
    if healthy {
        state.trial = None;
        state.save(layout)?;
        prune(layout, &state)?;
        return Ok(Launched::Started(current));
    }
    let _ = child.kill();
    let _ = child.wait();
    let previous = state
        .previous
        .take()
        .with_context(|| format!("Universal Patcher {current} failed to start"))?;
    state.rejected.push(current.clone());
    state.current = Some(previous.clone());
    state.trial = None;
    install_launcher(layout, &previous)?;
    state.save(layout)?;
    spawn(layout, &previous, args)?;
    Ok(Launched::RolledBack {
        failed: current,
        running: previous,
    })
}

/// Removes the program, launcher and menu entries. Game choices are kept unless asked.
pub fn uninstall(layout: &Layout, remove_data: bool) -> Result<()> {
    let _lock = Lock::acquire(&layout.root.join("install.lock"))?;
    disintegrate(layout)?;
    for dir in ["versions", "downloads", "tuf-app", "health", "share"] {
        remove_dir_if_present(&layout.root.join(dir))?;
    }
    for file in ["install.json", "app-root.json"] {
        match fs::remove_file(layout.root.join(file)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    // The running launcher removes itself last; Windows keeps it until the process exits.
    let bin = layout.root.join("bin");
    if cfg!(windows) {
        let _ = fs::remove_dir_all(&bin);
    } else {
        remove_dir_if_present(&bin)?;
    }
    if remove_data {
        let _ = fs::remove_file(layout.root.join("install.lock"));
        remove_dir_if_present(&layout.root)?;
    }
    Ok(())
}

/// Called by a running release once its interface is up.
pub fn report_healthy() {
    if let Some(path) = std::env::var_os("PATCHER_HEALTH_FILE") {
        let path = PathBuf::from(path);
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = atomic_write(&path, b"ok\n");
    }
}

/// A staged release waiting for restart, for display.
pub fn staged_version() -> Option<String> {
    let layout = Layout::for_user().ok()?;
    InstallState::load(&layout).ok()?.staged
}
