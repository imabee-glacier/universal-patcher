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
    /// Download size, when the list comes from a release.
    pub bytes: u64,
    /// expansion, game, stuff, kit or free.
    pub kind: String,
    /// On the user's own installation: installed, incomplete, changed or not_installed.
    /// Empty when the list comes from a release.
    pub status: String,
    /// Bytes on disk for an installed pack.
    pub installed_bytes: u64,
    /// Why the pack needs attention: files missing, changed or added since the last check.
    pub issues: Vec<String>,
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
    /// Languages installed with the game, as codes such as ENG_US.
    pub languages: Vec<String>,
    /// Whether background maintenance can be turned on here.
    pub background_supported: bool,
    /// When the game's files were last checked, if ever.
    pub checked_at: Option<String>,
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
            languages: vec![],
            background_supported: cfg!(target_os = "linux"),
            checked_at: None,
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

/// The installation to use without asking: a saved one that is still valid, or on first run
/// the only one found. A saved installation that is gone, or several found, needs a choice.
fn adopt(d: &crate::discovery::Discovery, had_saved: bool) -> Option<&crate::discovery::Candidate> {
    match d.candidates.as_slice() {
        [only] if !had_saved || only.source == "saved installation" => Some(only),
        _ => None,
    }
}

fn found_detail(c: &crate::discovery::Candidate) -> String {
    if c.maintenance_supported {
        "This is the installation the patcher looks after.".into()
    } else {
        "Updates and repair are not yet available on this platform, so the patcher leaves the game \
         as it is."
            .into()
    }
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
    /// The user's own installation, when there is no release feed to maintain it from.
    local: Mutex<Option<LocalGame>>,
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
                local: Mutex::new(None),
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
            kind: crate::local::kind_of(&g.id).as_str().into(),
            status: String::new(),
            installed_bytes: 0,
            issues: vec![],
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
        let layout = crate::app::Layout::for_user().ok();
        let saved = layout.as_ref().and_then(crate::app::saved_installation);
        let d =
            crate::discovery::discover(&crate::discovery::Probe::this_system(), saved.as_deref());
        let adopted = adopt(&d, saved.is_some()).cloned();
        if let (Some(c), Some(layout)) = (&adopted, &layout)
            && saved.as_deref() != Some(c.path.as_path())
            && let Err(e) = crate::app::save_installation(layout, &c.path)
        {
            self.failure(&format!("Cannot save the game location: {e:#}"));
            return;
        }
        self.update(|s| {
            s.state = "setup".into();
            s.phase = "setup".into();
            s.actions = vec!["Discover".into(), "ConfirmInstall".into()];
            s.next_action = "No game files are changed until maintenance is validated for this \
                             platform."
                .into();
            match &adopted {
                Some(c) => {
                    s.path = Some(c.path.display().to_string());
                    s.title = "Game found".into();
                    s.detail = found_detail(c);
                    s.candidates = vec![];
                }
                None => {
                    s.path = None;
                    (s.title, s.detail) = if !d.candidates.is_empty() && saved.is_some() {
                        (
                            "Your saved game folder is gone".into(),
                            "Choose which installation to look after instead.".into(),
                        )
                    } else if !d.candidates.is_empty() {
                        (
                            "Choose your installation".into(),
                            "More than one copy of The Sims 4 was found.".into(),
                        )
                    } else {
                        (
                            "Choose your game folder".into(),
                            "The Sims 4 was not found in the usual places listed below.".into(),
                        )
                    };
                    s.candidates = d.candidates.clone();
                }
            }
            s.checked = d.checked.clone();
        });
        if let Some(c) = &adopted
            && self.config.is_none()
        {
            self.open_local(c);
        }
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
        if self.config.is_none() && self.local_command(command, &v)? {
            return Ok(json!({"id":id,"accepted":true}));
        }
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
                crate::app::save_installation(&crate::app::Layout::for_user()?, &c.path)?;
                self.update(|s| {
                    s.path = Some(c.path.display().to_string());
                    s.title = "Game found".into();
                    s.detail = found_detail(&c);
                    s.candidates = vec![];
                });
                if self.config.is_none() {
                    self.open_local(&c);
                }
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
/// The user's own installation, maintained in place without a release feed.
struct LocalGame {
    root: PathBuf,
    source: String,
    /// Where this installation's choices and file fingerprints are kept.
    state: PathBuf,
    /// The last check, kept until its changes are accepted.
    report: Option<crate::local::CheckReport>,
}

fn local_state_dir() -> Result<PathBuf> {
    Ok(crate::app::Layout::for_user()?.root.join("game"))
}

fn new_policy(c: &crate::discovery::Candidate) -> Policy {
    Policy {
        schema: 1,
        game_id: "sims4".into(),
        install: crate::storage::Installation {
            identity: c.identity.clone(),
            variant: c.variant.clone(),
            roots: BTreeMap::from([("game".to_string(), c.path.clone())]),
        },
        revision: 1,
        desired: crate::model::Selection::default(),
        applied: None,
        preferences: Preferences::default(),
        suspended: None,
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

impl Service {
    /// Adopts `c` as the installation to look after: its saved choices, or new ones (all
    /// unanswered) for a first run or a different installation, then shows its packs.
    fn open_local(self: &Arc<Self>, c: &crate::discovery::Candidate) {
        let opened = (|| -> Result<Policy> {
            let dir = local_state_dir()?;
            fs::create_dir_all(&dir)?;
            match Policy::load(&dir) {
                Ok(p) if p.install.identity == c.identity => Ok(p),
                _ => {
                    // A first run, another installation, or both policy copies unreadable:
                    // ask again rather than guess.
                    let p = new_policy(c);
                    p.save(&dir)?;
                    Ok(p)
                }
            }
        })();
        let policy = match opened {
            Ok(p) => p,
            Err(e) => return self.failure(&format!("Cannot save your choices: {e:#}")),
        };
        let state = match local_state_dir() {
            Ok(d) => d,
            Err(e) => return self.failure(&format!("{e:#}")),
        };
        *self.local.lock().unwrap_or_else(|e| e.into_inner()) = Some(LocalGame {
            root: c.path.clone(),
            source: c.source.clone(),
            state,
            report: None,
        });
        self.show_local(&policy);
    }

    /// Reads the installation again and publishes it: packs with their status, languages,
    /// choices, and the last check.
    fn show_local(&self, policy: &Policy) {
        let (root, state, report) = {
            let guard = self.local.lock().unwrap_or_else(|e| e.into_inner());
            let Some(g) = guard.as_ref() else { return };
            (g.root.clone(), g.state.clone(), g.report.clone())
        };
        let scanned = crate::local::scan(&root);
        let languages = crate::local::languages(&root);
        let baseline = crate::local::Baseline::load(&state.join("baseline.json"));
        let packs: Vec<Pack> = scanned
            .iter()
            .map(|p| {
                let installed = p.status != crate::local::Status::NotInstalled;
                let check = report.as_ref().and_then(|r| r.packs.get(&p.id));
                let mut issues: Vec<String> =
                    p.missing.iter().map(|m| format!("Missing: {m}")).collect();
                if let Some(c) = check {
                    issues.extend(c.changed.iter().map(|f| format!("Changed: {f}")));
                    issues.extend(
                        c.missing
                            .iter()
                            .map(|f| format!("Gone since last check: {f}")),
                    );
                    issues.extend(c.added.iter().map(|f| format!("New since last check: {f}")));
                }
                let status = match p.status {
                    crate::local::Status::NotInstalled => "not_installed",
                    crate::local::Status::Incomplete => "incomplete",
                    crate::local::Status::Installed if check.is_some_and(|c| !c.clean()) => {
                        "changed"
                    }
                    crate::local::Status::Installed => "installed",
                };
                Pack {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    // Unanswered packs are proposed as installed.
                    selected: policy.desired.dlc.get(&p.id).copied().unwrap_or(installed),
                    answered: policy.desired.dlc.contains_key(&p.id),
                    deferred: policy.desired.deferred.contains(&p.id),
                    bytes: 0,
                    kind: p.kind.as_str().into(),
                    status: status.into(),
                    installed_bytes: p.bytes,
                    issues,
                }
            })
            .collect();
        let installed = packs.iter().filter(|p| p.status != "not_installed").count();
        let attention = packs
            .iter()
            .filter(|p| matches!(p.status.as_str(), "incomplete" | "changed"))
            .count();
        let unanswered = packs
            .iter()
            .any(|p| p.status != "not_installed" && !p.answered && !p.deferred);
        let on_disk: u64 = packs.iter().map(|p| p.installed_bytes).sum();
        let base_changed = report
            .as_ref()
            .and_then(|r| r.packs.get("base"))
            .is_some_and(|c| !c.clean());
        self.update(|s| {
            s.state = "installed".into();
            s.phase = "installed".into();
            s.ready = false;
            s.path = Some(root.display().to_string());
            s.candidates = vec![];
            s.policy_revision = policy.revision;
            s.preferences = policy.preferences.clone();
            s.language = policy.desired.language.clone();
            s.languages = languages;
            s.background_supported = false;
            s.checked_at = baseline.as_ref().map(|b| b.recorded_at.clone());
            s.bytes_done = 0;
            s.bytes_total = 0;
            s.files_done = 0;
            s.files_total = 0;
            s.title = if unanswered {
                "Review your packs".into()
            } else if attention > 0 || base_changed {
                "Some packs need attention".into()
            } else {
                "Your game".into()
            };
            s.detail = format!(
                "{installed} of {} packs installed, {} on disk{}",
                packs.len(),
                gib(on_disk),
                match attention {
                    0 => String::new(),
                    1 => " · 1 needs attention".into(),
                    n => format!(" · {n} need attention"),
                }
            );
            s.next_action = match (&report, &baseline) {
                (Some(r), _) if r.packs.values().all(|c| c.clean()) => {
                    "Every file matches the last check.".into()
                }
                (Some(_), _) => {
                    "Review what changed, then accept it if you updated the game.".into()
                }
                (None, Some(b)) => format!("Files last checked {}.", b.recorded_at),
                (None, None) => "Check your files once to record how they look today.".into(),
            };
            let mut actions: Vec<String> = [
                "Verify",
                "Launch",
                "SetPolicy",
                "Discover",
                "ConfirmInstall",
            ]
            .map(String::from)
            .to_vec();
            if report
                .as_ref()
                .is_some_and(|r| !r.first && r.packs.values().any(|c| !c.clean()))
            {
                actions.push("AcceptCheck".into());
            }
            s.actions = actions;
            s.packs = packs;
            s.error = None;
        });
    }

    /// Commands for the user's own installation. Returns false when `command` is not one of
    /// them, so the caller handles it.
    fn local_command(self: &Arc<Self>, command: &str, v: &Value) -> Result<bool> {
        let have = self
            .local
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if !have {
            return Ok(false);
        }
        let state = local_state_dir()?;
        match command {
            "SetPolicy" => {
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Wait for the file check to finish"
                );
                let _lock = Lock::acquire(&state.join("session.lock"))?;
                let mut p = Policy::load(&state)?;
                ensure!(
                    v.get("revision").and_then(Value::as_u64) == Some(p.revision),
                    "Your choices changed; reopen the pack list"
                );
                let root = self
                    .local
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|g| g.root.clone())
                    .context("No installation")?;
                let known: Vec<String> = crate::local::scan(&root)
                    .into_iter()
                    .map(|p| p.id)
                    .collect();
                if let Some(dlc) = v.get("dlc") {
                    let dlc: BTreeMap<String, bool> = serde_json::from_value(dlc.clone())?;
                    for id in dlc.keys() {
                        ensure!(known.contains(id), "Unknown pack: {id}");
                    }
                    p.desired.dlc = dlc;
                }
                if let Some(deferred) = v.get("deferred") {
                    p.desired.deferred = serde_json::from_value(deferred.clone())?;
                    for id in &p.desired.deferred {
                        ensure!(known.contains(id), "Unknown pack: {id}");
                    }
                }
                let deferred = p.desired.deferred.clone();
                p.desired.dlc.retain(|id, _| !deferred.contains(id));
                if let Some(language) = v.get("language") {
                    let language: Option<String> = serde_json::from_value(language.clone())?;
                    if let Some(code) = &language {
                        ensure!(
                            crate::local::languages(&root).contains(code),
                            "{code} is not installed with this game"
                        );
                    }
                    p.desired.language = language;
                }
                if let Some(preferences) = v.get("preferences") {
                    let preferences: Preferences = serde_json::from_value(preferences.clone())?;
                    ensure!(
                        !preferences.background,
                        "Background maintenance needs a published release, so it is not available yet"
                    );
                    p.preferences = preferences;
                }
                p.revision += 1;
                p.save(&state)?;
                self.show_local(&p);
            }
            "Verify" | "Start" | "Resume" => {
                ensure!(
                    !self.running.swap(true, Ordering::AcqRel),
                    "A file check is already running"
                );
                self.cancel.store(false, Ordering::Release);
                let s = self.clone();
                std::thread::spawn(move || s.run_check());
            }
            "Pause" | "Cancel" => {
                ensure!(
                    self.running.load(Ordering::Acquire),
                    "No file check is running"
                );
                self.cancel.store(true, Ordering::Release);
            }
            "AcceptCheck" => {
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Wait for the file check to finish"
                );
                let report = self
                    .local
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                    .and_then(|g| g.report.take())
                    .context("There is no check to accept")?;
                crate::local::Baseline {
                    schema: 1,
                    recorded_at: jiff::Timestamp::now()
                        .strftime("%Y-%m-%d %H:%M UTC")
                        .to_string(),
                    files: report.current,
                }
                .save(&state.join("baseline.json"))?;
                self.show_local(&Policy::load(&state)?);
            }
            "Launch" => {
                let (root, source) = self
                    .local
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|g| (g.root.clone(), g.source.clone()))
                    .context("No installation")?;
                crate::local::launch(&root, &source)?;
                self.update(|s| s.next_action = "Starting The Sims 4.".into());
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Checks every game file against the last accepted fingerprints, publishing progress.
    fn run_check(self: &Arc<Self>) {
        let target = self
            .local
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|g| (g.root.clone(), g.state.clone()));
        let Some((root, state)) = target else {
            self.running.store(false, Ordering::Release);
            return;
        };
        let baseline = crate::local::Baseline::load(&state.join("baseline.json"));
        self.update(|s| {
            s.state = "verifying".into();
            s.phase = "verifying".into();
            s.title = "Checking your game files".into();
            s.detail = "Reading every file of the game and your packs.".into();
            s.actions = vec!["Cancel".into()];
        });
        let started = Instant::now();
        let mut last = Instant::now() - Duration::from_secs(1);
        let result = crate::local::check(&root, baseline.as_ref(), &self.cancel, |p| {
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                self.update(|s| {
                    s.files_done = p.files_done;
                    s.files_total = p.files_total;
                    s.bytes_done = p.bytes_done;
                    s.bytes_total = p.bytes_total;
                    s.bytes_per_second =
                        (p.bytes_done as f64 / started.elapsed().as_secs_f64().max(0.01)) as u64;
                    s.detail = p.current;
                });
            }
        });
        let policy = Policy::load(&state);
        match result {
            Ok(report) => {
                let lines: Vec<String> = if report.first {
                    vec![format!(
                        "Checked {} files ({}). They are the reference for future checks.",
                        report.files,
                        gib(report.bytes)
                    )]
                } else {
                    let mut lines = vec![format!(
                        "Checked {} files ({}).",
                        report.files,
                        gib(report.bytes)
                    )];
                    for (id, c) in report.packs.iter().filter(|(_, c)| !c.clean()) {
                        lines.push(format!(
                            "{}: {} changed, {} gone, {} new",
                            if id == "base" { "Base game" } else { id },
                            c.changed.len(),
                            c.missing.len(),
                            c.added.len()
                        ));
                    }
                    lines
                };
                if report.first {
                    let saved = crate::local::Baseline {
                        schema: 1,
                        recorded_at: jiff::Timestamp::now()
                            .strftime("%Y-%m-%d %H:%M UTC")
                            .to_string(),
                        files: report.current.clone(),
                    }
                    .save(&state.join("baseline.json"));
                    if let Err(e) = saved {
                        self.running.store(false, Ordering::Release);
                        return self.failure(&format!("Cannot record the check: {e:#}"));
                    }
                }
                if let Some(g) = self
                    .local
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                {
                    g.report = (!report.first).then_some(report);
                }
                self.running.store(false, Ordering::Release);
                match policy {
                    Ok(p) => self.show_local(&p),
                    Err(e) => self.failure(&format!("Cannot read your choices: {e:#}")),
                }
                self.update(|s| s.details = lines);
            }
            Err(e) => {
                self.running.store(false, Ordering::Release);
                let stopped = self.cancel.load(Ordering::Acquire);
                match policy {
                    Ok(p) => self.show_local(&p),
                    Err(e) => self.failure(&format!("Cannot read your choices: {e:#}")),
                }
                self.update(|s| {
                    s.details = vec![if stopped {
                        "The check was stopped; nothing was changed.".into()
                    } else {
                        format!("The check could not finish: {e:#}")
                    }];
                });
            }
        }
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

    fn candidate(path: &str, source: &str) -> crate::discovery::Candidate {
        crate::discovery::Candidate {
            path: path.into(),
            identity: path.into(),
            variant: "windows-native".into(),
            source: source.into(),
            maintenance_supported: false,
        }
    }

    fn found(candidates: Vec<crate::discovery::Candidate>) -> crate::discovery::Discovery {
        crate::discovery::Discovery {
            candidates,
            checked: vec![],
        }
    }

    #[test]
    fn the_only_installation_found_is_used_without_asking() {
        let d = found(vec![candidate("C:/Games/The Sims 4", "EA app")]);
        assert_eq!(adopt(&d, false).map(|c| c.source.as_str()), Some("EA app"));
    }

    #[test]
    fn a_valid_saved_installation_is_used_without_asking() {
        let d = found(vec![candidate("D:/Sims", "saved installation")]);
        assert!(adopt(&d, true).is_some());
    }

    #[test]
    fn several_installations_or_a_lost_saved_one_need_a_choice() {
        let two = found(vec![
            candidate("C:/A", "EA app"),
            candidate("D:/B", "Steam library"),
        ]);
        assert!(adopt(&two, false).is_none());
        // The saved folder is gone and something else was found: confirm the replacement.
        let replacement = found(vec![candidate("C:/A", "EA app")]);
        assert!(adopt(&replacement, true).is_none());
        assert!(adopt(&found(vec![]), false).is_none());
    }
}
