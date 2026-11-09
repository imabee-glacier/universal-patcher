//! Narrow JSON FFI boundary. Flutter renders snapshots and submits commands only.
use crate::{
    engine::{self, Progress},
    fixture::Config,
    model::GroupKind,
    storage::{Lock, Policy, Preferences},
    transfer::{Blocker, Control, HttpSource, LocalSource},
    trust,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char},
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
/// How long to wait before checking again whether the game has closed.
const GAME_RECHECK: Duration = Duration::from_secs(30);
#[derive(Clone, Serialize)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub selected: bool,
    /// False until the user enabled, disabled or deferred this pack.
    pub answered: bool,
    pub deferred: bool,
    pub bytes: u64,
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub state: String,
    pub phase: String,
    pub title: String,
    pub detail: String,
    pub path: Option<String>,
    pub fixture: bool,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub bytes_per_second: u64,
    pub files_done: usize,
    pub files_total: usize,
    pub details: Vec<String>,
    pub ready: bool,
    pub actions: Vec<String>,
    pub packs: Vec<Pack>,
    pub candidates: Vec<crate::discovery::Candidate>,
    pub checked: Vec<PathBuf>,
    pub policy_revision: u64,
    pub online: String,
    pub preferences: Preferences,
    pub language: Option<String>,
    /// A downloaded patcher update that starts next time.
    pub app_update: Option<String>,
    pub error: Option<String>,
    pub next_action: String,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            state: "checking".into(),
            phase: "discovery".into(),
            title: "Finding your game".into(),
            detail: "Checking saved locations and Steam libraries.".into(),
            path: None,
            fixture: false,
            bytes_done: 0,
            bytes_total: 0,
            bytes_per_second: 0,
            files_done: 0,
            files_total: 0,
            details: vec![],
            ready: false,
            actions: vec![],
            packs: vec![],
            candidates: vec![],
            checked: vec![],
            policy_revision: 0,
            online: "Unavailable: sign-in handoff is not validated".into(),
            preferences: Preferences::default(),
            language: None,
            app_update: crate::app::staged_version(),
            error: None,
            next_action: "Your game will appear here automatically.".into(),
        }
    }
}
/// Copies engine progress into the snapshot. The engine's final "ready" report is not
/// readiness: only the run's outcome sets the Ready state, together with `ready`, so the two
/// can never disagree in a published snapshot.
fn apply_progress(s: &mut Snapshot, p: Progress, elapsed: Duration) {
    s.bytes_per_second = if p.phase == "downloading" {
        (p.bytes_done as f64 / elapsed.as_secs_f64().max(0.01)) as u64
    } else {
        0
    };
    if p.phase != "ready" {
        s.title = phase_title(&p.phase).into();
        s.state = p.phase.clone();
        s.phase = p.phase;
        s.detail = p.detail;
    }
    s.bytes_done = p.bytes_done;
    s.bytes_total = p.bytes_total;
    s.files_done = p.files_done;
    s.files_total = p.files_total;
}

struct Service {
    snapshot: Mutex<Snapshot>,
    config: Option<Config>,
    config_path: Option<PathBuf>,
    config_error: Option<String>,
    /// The last release that passed signature verification; selections are checked against it.
    catalog: Mutex<Option<Arc<crate::model::Manifest>>>,
    running: AtomicBool,
    control: Mutex<Control>,
    pause: AtomicBool,
    cancel: AtomicBool,
}
static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();
fn service() -> Arc<Service> {
    SERVICE
        .get_or_init(|| {
            let config_path = std::env::var_os("PATCHER_CONFIG")
                .map(PathBuf::from)
                .or_else(|| {
                    crate::app::Layout::for_user()
                        .ok()
                        .map(|l| l.config())
                        .filter(|c| c.is_file())
                });
            let loaded = config_path.as_ref().map(|path| -> Result<Config> {
                let bytes = fs::read(path)?;
                Ok(serde_json::from_value(crate::strict_json::parse(&bytes)?)?)
            });
            let (config, config_error) = match loaded {
                Some(Ok(c)) => (Some(c), None),
                Some(Err(e)) => (
                    None,
                    Some(format!("Cannot read patcher configuration: {e:#}")),
                ),
                None => (None, None),
            };
            let s = Arc::new(Service {
                snapshot: Mutex::new(Snapshot::default()),
                config,
                config_path,
                config_error,
                catalog: Mutex::new(None),
                running: AtomicBool::new(false),
                control: Mutex::new(Control::default()),
                pause: AtomicBool::new(false),
                cancel: AtomicBool::new(false),
            });
            let init = s.clone();
            std::thread::spawn(move || init.discover());
            s
        })
        .clone()
}
/// The engine scheduled maintenance runs: the stable launcher when installed, so the timer
/// keeps working across updates; otherwise the engine shipped beside the interface.
fn engine_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PATCHER_ENGINE") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(layout) = crate::app::Layout::for_user()
        && layout.launcher().is_file()
    {
        return Ok(layout.launcher());
    }
    let exe = std::env::current_exe()?;
    Ok(exe
        .parent()
        .context("Executable has no directory")?
        .join("lib/patcher-engine"))
}
fn feed(cfg: &Config) -> Result<trust::VerifiedRelease> {
    tokio::runtime::Runtime::new()?.block_on(trust::load(
        &cfg.trusted_root,
        cfg.metadata_url.parse()?,
        cfg.targets_url.parse()?,
        &cfg.target,
        &cfg.state.join("tuf"),
    ))
}
fn packs(release: &trust::VerifiedRelease, policy: &Policy) -> Vec<Pack> {
    let manifest = release.manifest();
    manifest
        .groups
        .iter()
        .filter(|g| g.kind == GroupKind::Dlc)
        .map(|g| Pack {
            id: g.id.clone(),
            name: g.display_name.clone(),
            selected: policy.desired.dlc.get(&g.id) == Some(&true),
            answered: policy.desired.dlc.contains_key(&g.id),
            deferred: policy.desired.deferred.contains(&g.id),
            bytes: manifest
                .files
                .iter()
                .filter(|f| g.file_ids.contains(&f.id))
                .map(|f| f.full.encoded_size)
                .sum(),
        })
        .collect()
}
fn report_lines(report: &engine::RunReport) -> Vec<String> {
    let mut lines = Vec::new();
    if report.journal_reset {
        lines.push("The recovery record was damaged, so every file was checked again.".into());
    }
    lines.extend(report.replaced.iter().map(|p| format!("Replaced {p}")));
    lines.extend(
        report
            .deleted
            .iter()
            .map(|p| format!("Removed obsolete {p}")),
    );
    lines.extend(
        report
            .preserved
            .iter()
            .map(|p| format!("Kept {p}: it was changed outside the patcher")),
    );
    if lines.is_empty() {
        lines.push("No files needed changes.".into());
    }
    lines
}
fn phase_title(phase: &str) -> &'static str {
    match phase {
        "checking" => "Checking your files",
        "downloading" => "Downloading",
        "applying" => "Applying",
        "verifying" => "Verifying",
        "ready" => "Ready",
        _ => "Maintaining your game",
    }
}
impl Service {
    fn remember(&self, release: &trust::VerifiedRelease) {
        *self.catalog.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::new(release.manifest().clone()));
    }
    fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut s = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        s.revision += 1;
    }
    /// Presents a known blocker as one sentence and one next action.
    fn blocked(self: &Arc<Self>, blocker: Blocker) {
        let (state, title, detail, next, actions): (&str, &str, &str, &str, &[&str]) = match blocker
        {
            Blocker::GameRunning => (
                "waiting",
                "Waiting for game to close",
                "Files are not changed while The Sims 4 or its installer is running.",
                "Maintenance continues automatically after the game closes.",
                &["Cancel"],
            ),
            Blocker::UnknownVersion => (
                "needs_attention",
                "Game updated elsewhere",
                "This game version is newer or different from the published release, so nothing was changed.",
                "Maintenance resumes when a release for this version is published.",
                &["Verify"],
            ),
            Blocker::DiskSpace => (
                "needs_attention",
                "Not enough disk space",
                "Free space is needed for downloads and safe replacement of files.",
                "Free some space, then start maintenance again.",
                &["Start"],
            ),
            Blocker::Network => (
                "needs_attention",
                "Offline",
                "The download server could not be reached. Completed work is kept.",
                "Maintenance retries on the next launch, or start it again now.",
                &["Start"],
            ),
            Blocker::Stopped => (
                "paused",
                "Paused",
                "Committed files stay in place.",
                "Nothing restarts until you choose to continue.",
                &["Resume"],
            ),
        };
        self.update(|s| {
            s.state = state.into();
            s.phase = state.into();
            s.title = title.into();
            s.detail = detail.into();
            s.error = Some(blocker.to_string());
            s.ready = false;
            s.bytes_per_second = 0;
            s.actions = actions.iter().map(|a| (*a).to_string()).collect();
            s.next_action = next.into();
        });
        if blocker == Blocker::GameRunning {
            let service = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(GAME_RECHECK);
                let waiting = service
                    .snapshot
                    .lock()
                    .map(|s| s.state == "waiting")
                    .unwrap_or(false);
                if waiting {
                    service.start(false, false);
                }
            });
        }
    }
    fn failure(&self, error: &str) {
        self.update(|s| {
            s.state = "needs_attention".into();
            s.title = "Needs attention".into();
            s.error = Some(error.into());
            s.detail = error.into();
            s.ready = false;
            s.actions = vec!["Start".into(), "Discover".into()];
            s.next_action = "Review Details, then try again when the cause is resolved.".into();
        });
    }
    fn suspended(&self, reason: &str) {
        let cancelled = reason == "cancelled";
        self.update(|s| {
            s.state = reason.into();
            s.phase = reason.into();
            s.title = if cancelled { "Stopped" } else { "Paused" }.into();
            s.detail = "Committed files stay in place. Play is unavailable until maintenance \
                        finishes verification."
                .into();
            s.ready = false;
            s.bytes_per_second = 0;
            s.actions = vec![
                if cancelled { "Start" } else { "Resume" }.into(),
                "SetPolicy".into(),
            ];
            s.next_action = "Nothing restarts until you choose to continue.".into();
        });
    }
    fn discover(self: &Arc<Self>) {
        if let Some(error) = &self.config_error {
            self.failure(error);
            return;
        }
        if let Some(cfg) = &self.config {
            let policy = match Policy::load(&cfg.state) {
                Ok(policy) => policy,
                Err(e) => {
                    self.failure(&format!("Cannot restore saved selections: {e:#}"));
                    return;
                }
            };
            self.update(|s| {
                s.fixture = policy.install.variant == "fixture";
                s.path = policy
                    .install
                    .roots
                    .get("game")
                    .map(|p| p.display().to_string());
                s.policy_revision = policy.revision;
                s.preferences = policy.preferences.clone();
                s.language = policy.desired.language.clone();
            });
            match &policy.suspended {
                Some(reason) => {
                    self.suspended(reason);
                    match feed(cfg) {
                        Ok(release) => {
                            self.remember(&release);
                            self.update(|s| s.packs = packs(&release, &policy));
                        }
                        Err(_) => self.update(|s| {
                            s.next_action = "Update availability is unknown while the release \
                                             feed is unreachable."
                                .into()
                        }),
                    }
                }
                None => self.start(false, false),
            }
            return;
        }
        let d = crate::discovery::discover(&crate::discovery::Probe::this_system(), None);
        self.update(|s| {
            let found = !d.candidates.is_empty();
            s.checked = d.checked;
            s.candidates = d.candidates;
            s.state = "setup".into();
            s.phase = "setup".into();
            s.actions = vec!["Discover".into(), "ConfirmInstall".into()];
            s.title = if found {
                "Game found"
            } else {
                "Choose your game folder"
            }
            .into();
            s.detail = if found {
                "Confirm the installation this patcher should maintain."
            } else {
                "No verified installation was found in the usual locations."
            }
            .into();
            s.next_action =
                "A verified release feed is required before any game files change.".into();
        });
    }
    fn start(self: &Arc<Self>, resume: bool, full_verify: bool) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        // Install the stop control before the worker exists so an early Pause is never lost.
        let control = Control::default();
        *self.control.lock().unwrap_or_else(|e| e.into_inner()) = control.clone();
        self.pause.store(false, Ordering::Release);
        self.cancel.store(false, Ordering::Release);
        self.update(|s| {
            s.state = "checking".into();
            s.phase = "checking".into();
            s.ready = false;
            s.error = None;
            s.title = "Checking".into();
            s.detail = "Verifying the signed release and your selections.".into();
            s.actions = vec!["Pause".into(), "Cancel".into()];
            s.next_action = "Maintenance continues automatically.".into();
        });
        let service = self.clone();
        std::thread::spawn(move || {
            let result = service.maintain(&control, resume, full_verify);
            service.running.store(false, Ordering::Release);
            service.finish(result);
        });
    }
    fn maintain(
        &self,
        control: &Control,
        resume: bool,
        full_verify: bool,
    ) -> Result<engine::RunReport> {
        let cfg = self
            .config
            .as_ref()
            .context("No verified release feed is configured")?;
        let _session = Lock::acquire(&cfg.state.join("session.lock"))?;
        let mut policy = Policy::load(&cfg.state)?;
        if resume && policy.suspended.is_some() {
            policy.suspended = None;
            policy.save(&cfg.state)?;
        }
        let release = feed(cfg)?;
        self.remember(&release);
        if policy.adopt_new_packs(release.manifest()) {
            policy.save(&cfg.state)?;
        }
        let list = packs(&release, &policy);
        self.update(|s| {
            s.packs = list;
            s.policy_revision = policy.revision;
            s.preferences = policy.preferences.clone();
        });
        let started = Instant::now();
        let report = |p: Progress| self.update(|s| apply_progress(s, p, started.elapsed()));
        let options = engine::RunOptions {
            full_verify,
            ..Default::default()
        };
        match &cfg.local_objects {
            Some(directory) => {
                ensure!(
                    release.manifest().variant == "fixture",
                    "Local objects are only allowed for fixtures"
                );
                let source = LocalSource {
                    directory: directory.clone(),
                };
                engine::run(
                    &release,
                    &mut policy,
                    &cfg.state,
                    &source,
                    control,
                    &report,
                    &options,
                )
            }
            None => {
                let source = HttpSource::new(
                    cfg.origin.parse()?,
                    policy.preferences.bandwidth_bytes_per_second,
                );
                engine::run(
                    &release,
                    &mut policy,
                    &cfg.state,
                    &source,
                    control,
                    &report,
                    &options,
                )
            }
        }
    }
    fn finish(self: &Arc<Self>, result: Result<engine::RunReport>) {
        let cancelled = self.cancel.load(Ordering::Acquire);
        if cancelled || self.pause.load(Ordering::Acquire) {
            let reason = if cancelled { "cancelled" } else { "paused" };
            if let Some(cfg) = &self.config {
                let saved = Policy::load(&cfg.state).and_then(|mut p| {
                    p.suspended = Some(reason.into());
                    p.save(&cfg.state)
                });
                if let Err(e) = saved {
                    self.failure(&format!("Cannot save paused state: {e:#}"));
                    return;
                }
            }
            self.suspended(reason);
            return;
        }
        match result {
            Ok(report) => self.update(|s| {
                s.state = "ready".into();
                s.phase = "ready".into();
                s.ready = true;
                s.bytes_per_second = 0;
                s.title = "Ready".into();
                s.detail = if s.fixture {
                    "All selected test files passed verification. Your game was not touched."
                } else {
                    "Your selected content is verified."
                }
                .into();
                s.details = report_lines(&report);
                s.actions = vec!["Verify".into(), "SetPolicy".into()];
                s.next_action = if s.fixture {
                    "Test mode: game launch is unavailable."
                } else {
                    "Launch is unavailable until the platform adapter is validated."
                }
                .into();
            }),
            Err(e) => match e.downcast_ref::<Blocker>() {
                Some(blocker) => self.blocked(*blocker),
                None => self.failure(&format!("{e:#}")),
            },
        }
    }
    fn command(self: &Arc<Self>, v: Value) -> Result<Value> {
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .context("Command id is required")?;
        let command = v
            .get("command")
            .and_then(Value::as_str)
            .context("Command is required")?;
        match command {
            "Discover" => {
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Maintenance is active"
                );
                let s = self.clone();
                std::thread::spawn(move || s.discover());
            }
            "Start" | "Resume" | "Verify" => {
                ensure!(self.config.is_some(), "Release feed is not configured");
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Maintenance is already active"
                );
                self.start(true, command == "Verify");
            }
            "Pause" | "Cancel" => {
                let waiting = self
                    .snapshot
                    .lock()
                    .map(|s| s.state == "waiting")
                    .unwrap_or(false);
                if waiting && !self.running.load(Ordering::Acquire) {
                    let reason = if command == "Cancel" {
                        "cancelled"
                    } else {
                        "paused"
                    };
                    let cfg = self
                        .config
                        .as_ref()
                        .context("Release feed is not configured")?;
                    let mut p = Policy::load(&cfg.state)?;
                    p.suspended = Some(reason.into());
                    p.save(&cfg.state)?;
                    self.suspended(reason);
                    return Ok(json!({"id":id,"accepted":true}));
                }
                ensure!(
                    self.running.load(Ordering::Acquire),
                    "No maintenance is active"
                );
                let flag = if command == "Cancel" {
                    &self.cancel
                } else {
                    &self.pause
                };
                flag.store(true, Ordering::Release);
                self.control
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .stop();
            }
            "SetPolicy" => {
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Pause maintenance before changing selections"
                );
                let cfg = self
                    .config
                    .as_ref()
                    .context("Release feed is not configured")?;
                let _lock = Lock::acquire(&cfg.state.join("session.lock"))?;
                let mut p = Policy::load(&cfg.state)?;
                ensure!(
                    v.get("revision").and_then(Value::as_u64) == Some(p.revision),
                    "Selections changed; reopen the pack list"
                );
                let previous = p.clone();
                let field = |name: &str| v.get(name).cloned();
                if let Some(dlc) = field("dlc") {
                    p.desired.dlc = serde_json::from_value::<BTreeMap<String, bool>>(dlc)?;
                }
                if let Some(deferred) = field("deferred") {
                    p.desired.deferred = serde_json::from_value(deferred)?;
                }
                // A deferred pack stays unanswered.
                let deferred = p.desired.deferred.clone();
                p.desired.dlc.retain(|id, _| !deferred.contains(id));
                if let Some(language) = field("language") {
                    p.desired.language = serde_json::from_value(language)?;
                }
                let preferences = match field("preferences") {
                    Some(value) => serde_json::from_value::<Preferences>(value)?,
                    None => p.preferences.clone(),
                };
                let catalog = self
                    .catalog
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .context("The pack list has not loaded yet")?;
                for id in &p.desired.deferred {
                    ensure!(
                        catalog
                            .groups
                            .iter()
                            .any(|g| g.id == *id && g.kind == GroupKind::Dlc),
                        "Unknown pack: {id}"
                    );
                }
                catalog.selected(&p.desired)?;
                // Everything is validated; only now touch the OS scheduler, then persist.
                let schedule_changed = preferences.background != p.preferences.background;
                if schedule_changed {
                    let config = self
                        .config_path
                        .as_ref()
                        .context("Configuration path is unknown")?;
                    crate::os::set_schedule(preferences.background, &engine_path()?, config)?;
                }
                let selection_changed = p.desired != previous.desired;
                p.preferences = preferences;
                p.revision += 1;
                if let Err(e) = p.save(&cfg.state) {
                    if schedule_changed && let Some(config) = &self.config_path {
                        let restore = previous.preferences.background;
                        if let Err(undo) = crate::os::set_schedule(restore, &engine_path()?, config)
                        {
                            bail!(
                                "{e:#}; the maintenance schedule could not be restored: {undo:#}"
                            );
                        }
                    }
                    return Err(e);
                }
                let selected = p.desired.dlc.clone();
                let deferred = p.desired.deferred.clone();
                self.update(|s| {
                    s.policy_revision = p.revision;
                    s.preferences = p.preferences.clone();
                    s.language = p.desired.language.clone();
                    for pack in &mut s.packs {
                        pack.selected = selected.get(&pack.id) == Some(&true);
                        pack.answered = selected.contains_key(&pack.id);
                        pack.deferred = deferred.contains(&pack.id);
                    }
                    if selection_changed {
                        s.ready = false;
                        s.state = "planned".into();
                        s.title = "Selections saved".into();
                        s.detail = "Your changes apply the next time maintenance runs.".into();
                        s.actions = vec!["Start".into(), "SetPolicy".into()];
                        s.next_action = "Start maintenance to apply your selections.".into();
                    }
                });
            }
            "ConfirmInstall" => {
                let path = v
                    .get("path")
                    .and_then(Value::as_str)
                    .context("Choose a folder")?;
                let c = crate::discovery::validate(
                    std::path::Path::new(path),
                    "folder picker",
                    crate::discovery::Probe::this_system().variant,
                )?;
                self.update(|s| {
                    s.path = Some(c.path.display().to_string());
                    s.title = "Game found".into();
                    s.detail = "Maintenance for this platform is not yet validated.".into();
                    s.next_action = "No game files have been changed.".into();
                });
            }
            "Connect" | "Disconnect" => {
                bail!("Online sign-in is unavailable: provider handoff has not been validated")
            }
            "Launch" => bail!("Game launch is unavailable: platform launch has not been validated"),
            _ => bail!("Unknown command"),
        }
        Ok(json!({"id":id,"accepted":true}))
    }
}
fn string(value: Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap().into_raw()
}
static HEALTH: std::sync::Once = std::sync::Once::new();
#[unsafe(no_mangle)]
pub extern "C" fn patcher_snapshot() -> *mut c_char {
    std::panic::catch_unwind(|| {
        let s = service();
        // The interface is polling snapshots, so this release started correctly.
        HEALTH.call_once(crate::app::report_healthy);
        let snapshot = s.snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone();
        string(serde_json::to_value(snapshot).unwrap())
    })
    .unwrap_or_else(|_| string(json!({"error":"Native snapshot failed"})))
}
/// # Safety
/// `input` must point to a valid, NUL-terminated UTF-8 string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn patcher_command(input: *const c_char) -> *mut c_char {
    std::panic::catch_unwind(|| {
        let r = (|| -> Result<Value> {
            ensure!(!input.is_null(), "Missing command");
            let bytes = unsafe { CStr::from_ptr(input) }.to_bytes();
            ensure!(bytes.len() <= 65536, "Command is too large");
            service().command(crate::strict_json::parse(bytes)?)
        })();
        string(match r {
            Ok(v) => v,
            Err(e) => json!({"accepted":false,"error":e.to_string()}),
        })
    })
    .unwrap_or_else(|_| string(json!({"accepted":false,"error":"Native command failed"})))
}
/// # Safety
/// `value` must be a live string returned by this library, freed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn patcher_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(phase: &str) -> Progress {
        Progress {
            phase: phase.into(),
            detail: format!("{phase} detail"),
            bytes_done: 10,
            bytes_total: 10,
            files_done: 2,
            files_total: 2,
        }
    }

    #[test]
    fn engine_ready_progress_never_claims_readiness() {
        let mut s = Snapshot::default();
        apply_progress(&mut s, progress("verifying"), Duration::from_secs(1));
        assert_eq!((s.state.as_str(), s.ready), ("verifying", false));
        apply_progress(&mut s, progress("ready"), Duration::from_secs(1));
        assert_eq!((s.state.as_str(), s.ready), ("verifying", false));
        assert_eq!((s.files_done, s.files_total), (2, 2));
    }

    #[test]
    fn transfer_rate_is_reported_only_while_downloading() {
        let mut s = Snapshot::default();
        apply_progress(&mut s, progress("downloading"), Duration::from_secs(2));
        assert_eq!(s.bytes_per_second, 5);
        apply_progress(&mut s, progress("applying"), Duration::from_secs(2));
        assert_eq!(s.bytes_per_second, 0);
    }
}
