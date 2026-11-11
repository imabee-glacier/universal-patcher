//! Narrow JSON FFI boundary. Flutter renders snapshots and submits commands only: which actions
//! exist, which one leads, what each button says and why an action is unavailable are all
//! decided here.
use crate::{
    app::Layout,
    discovery::{Candidate, Probe},
    engine::{self, Progress},
    fixture::Config,
    local::{self, Launch, LocalPack},
    model::GroupKind,
    process::{Detection, Host, SystemHost},
    storage::{Lock, Policy, Preferences},
    transfer::{Blocker, Control, HttpSource, LocalSource},
    trust,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub selected: bool,
    /// False until the user enabled, disabled or deferred this pack.
    pub answered: bool,
    pub deferred: bool,
    /// Neither answered nor deferred: a new pack waiting for a decision.
    pub undecided: bool,
    /// Download size, when the list comes from a release.
    pub bytes: u64,
    /// expansion, game, stuff, kit or free.
    pub kind: String,
    /// On the user's own installation: installed, incomplete, changed or not_installed.
    /// Empty when the list comes from a release.
    pub status: String,
    /// Bytes on disk for an installed pack.
    pub installed_bytes: u64,
    /// Why the pack needs attention: files missing, changed or added since the reference.
    pub issues: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct Snapshot {
    /// Advances with every change; the interface fetches a snapshot only when it moved.
    pub revision: u64,
    pub state: String,
    pub phase: String,
    pub title: String,
    pub detail: String,
    /// Short status beside the game name, readable without colour.
    pub status: String,
    pub path: Option<String>,
    pub fixture: bool,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub bytes_per_second: u64,
    pub files_done: usize,
    pub files_total: usize,
    /// Whether progress belongs on screen.
    pub show_progress: bool,
    pub details: Vec<String>,
    pub ready: bool,
    pub actions: Vec<String>,
    /// The action that leads in this state. When it is `Launch`, the Play button leads.
    pub primary: Option<String>,
    /// Button text for every offered action.
    pub labels: BTreeMap<String, String>,
    /// Actions that exist here but cannot be used now, with the reason shown to the user.
    pub unavailable: BTreeMap<String, String>,
    /// The Play button: hidden, available, starting, playing or unavailable.
    pub play: String,
    /// Shown under the Play button: why it cannot be used, or what happened last time.
    pub play_reason: Option<String>,
    pub packs: Vec<Pack>,
    /// One line about the packs, for the main screen.
    pub collection: String,
    pub candidates: Vec<Candidate>,
    pub checked: Vec<PathBuf>,
    pub policy_revision: u64,
    pub online: String,
    pub preferences: Preferences,
    pub language: Option<String>,
    /// A downloaded patcher update that starts next time.
    pub app_update: Option<String>,
    /// The outcome of the last patcher update check, or that one is running.
    pub app_update_status: Option<String>,
    pub error: Option<String>,
    pub next_action: String,
    /// Languages installed with the game, as codes such as ENG_US.
    pub languages: Vec<String>,
    /// Whether download settings mean anything here.
    pub downloads_supported: bool,
    /// Whether background maintenance can be turned on here.
    pub background_supported: bool,
    /// What background maintenance does, when it can be turned on.
    pub background_note: Option<String>,
    /// What the patcher does to game files in this mode.
    pub mode_note: String,
    /// When the reference fingerprints were recorded, if ever.
    pub checked_at: Option<String>,
    /// Why Play cannot be used in this state, beyond what the installation itself allows.
    #[serde(skip)]
    launch_blocked: Option<String>,
    /// enabled, disabled, unsupported, or failed. Enabled only after a verified rewrite.
    pub unlock: String,
    /// What that status means. It does not say the game is playable.
    pub unlock_detail: String,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            state: "checking".into(),
            phase: "discovery".into(),
            title: "Finding your game".into(),
            detail: "Checking saved locations and Steam libraries.".into(),
            status: "Finding your game".into(),
            path: None,
            fixture: false,
            bytes_done: 0,
            bytes_total: 0,
            bytes_per_second: 0,
            files_done: 0,
            files_total: 0,
            show_progress: false,
            details: vec![],
            ready: false,
            actions: vec![],
            primary: None,
            labels: BTreeMap::new(),
            unavailable: BTreeMap::new(),
            play: "hidden".into(),
            play_reason: None,
            packs: vec![],
            collection: "Shown once your game is found".into(),
            candidates: vec![],
            checked: vec![],
            policy_revision: 0,
            online: "Unavailable: sign-in handoff is not validated".into(),
            preferences: Preferences::default(),
            language: None,
            app_update: None,
            app_update_status: None,
            error: None,
            next_action: "Your game will appear here automatically.".into(),
            languages: vec![],
            downloads_supported: false,
            background_supported: false,
            background_note: None,
            mode_note: String::new(),
            checked_at: None,
            launch_blocked: None,
            unlock: "unsupported".into(),
            unlock_detail: "Unlock is not available on this system.".into(),
        }
    }
}

/// Sets the offered actions and the one that leads, dropping labels chosen for earlier actions.
fn set_actions(s: &mut Snapshot, actions: &[&str], primary: Option<&str>) {
    s.actions = actions.iter().map(|a| (*a).to_string()).collect();
    s.primary = primary.map(String::from);
    s.labels.clear();
}

fn label(s: &mut Snapshot, action: &str, text: &str) {
    s.labels.insert(action.into(), text.into());
}

fn default_label(action: &str) -> &'static str {
    match action {
        "Launch" => "Play",
        "Verify" => "Verify files",
        "Start" => "Start maintenance",
        "Resume" => "Resume",
        "Pause" => "Pause",
        "Cancel" => "Stop for now",
        "Discover" => "Search again",
        "ConfirmInstall" => "Choose game folder",
        "ConfirmLocation" => "Keep my choices for this folder",
        "SetPolicy" => "Save preferences",
        "ViewPacks" => "Your packs",
        "CheckAppUpdate" => "Check for patcher update",
        _ => "",
    }
}

/// Actions that read or change game files, or change which installation is looked after. None
/// of them runs while the game is starting or running.
const GAME_FILE_ACTIONS: [&str; 6] = [
    "Verify",
    "Start",
    "Resume",
    "ConfirmLocation",
    "ConfirmInstall",
    "Discover",
];

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
        s.status = phase_status(&p.phase).into();
        s.state = p.phase.clone();
        s.phase = p.phase;
        s.detail = p.detail;
    }
    s.show_progress = true;
    s.bytes_done = p.bytes_done;
    s.bytes_total = p.bytes_total;
    s.files_done = p.files_done;
    s.files_total = p.files_total;
}

/// The installation to use without asking: a saved one that is still valid, or on first run
/// the only one found. A saved installation that is gone, or several found, needs a choice.
fn adopt(d: &crate::discovery::Discovery, had_saved: bool) -> Option<&Candidate> {
    match d.candidates.as_slice() {
        [only] if !had_saved || only.source == "saved installation" => Some(only),
        _ => None,
    }
}

/// How the interface's timers behave. Tests shorten them.
#[derive(Debug, Clone)]
pub struct Timing {
    /// How long Play waits for the game to appear before saying it did not start.
    pub launch_timeout: Duration,
    /// How often running programs are checked while the game is starting.
    pub poll_starting: Duration,
    /// How often running programs are checked otherwise.
    pub poll_idle: Duration,
    /// How long maintenance waits before checking again whether the game has closed.
    pub game_recheck: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            // Steam may need to start, update itself and sign in first.
            launch_timeout: Duration::from_secs(90),
            poll_starting: Duration::from_millis(500),
            poll_idle: Duration::from_secs(3),
            game_recheck: Duration::from_secs(30),
        }
    }
}

/// Everything a session depends on outside itself.
pub struct Options {
    /// A release feed configuration; without one the user's own installation is shown.
    pub config: Option<PathBuf>,
    /// Where the patcher keeps its data; `None` when the system has no such place.
    pub layout: Option<Layout>,
    pub probe: Probe,
    pub host: Arc<dyn Host>,
    pub timing: Timing,
}

impl Options {
    /// The real system, configured from the environment as the installed app is.
    pub fn from_env() -> Self {
        let layout = Layout::for_user().ok();
        let config = std::env::var_os("PATCHER_CONFIG")
            .map(PathBuf::from)
            .or_else(|| layout.as_ref().map(Layout::config).filter(|c| c.is_file()));
        Self {
            config,
            layout,
            probe: Probe::this_system(),
            host: Arc::new(SystemHost),
            timing: Timing::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum PlayPhase {
    #[default]
    Idle,
    Starting(Instant),
    Playing,
    /// The last attempt failed; the message says what to do.
    Failed(String),
}

struct Target {
    how: Launch,
}

#[derive(Default)]
struct LaunchCtl {
    /// The installation Play starts; `None` hides Play.
    target: Option<Target>,
    phase: PlayPhase,
    detection: Option<Detection>,
}

/// Why Play cannot work for this installation on this system, whatever the state.
fn installation_launch_reason(ctl: &LaunchCtl) -> Option<String> {
    match &ctl.target.as_ref()?.how {
        Launch::Unsupported(reason) => Some(reason.clone()),
        _ => None,
    }
}

fn did_not_start(how: &Launch) -> String {
    match how {
        Launch::Steam => "The game did not start. Open Steam, then try again.".into(),
        _ => "The game did not start.".into(),
    }
}

/// The user's own installation, read in place without a release feed.
struct LocalGame {
    candidate: Candidate,
    /// Where this installation's choices are kept.
    state: PathBuf,
    policy: Policy,
    /// Set when the saved choices belong to an earlier folder identity; they are kept, and the
    /// user confirms before they are moved to this folder.
    location_from: Option<String>,
    /// `None` while the folder is still being read.
    packs: Option<Vec<LocalPack>>,
    languages: Vec<String>,
    lines: Vec<String>,
}

impl LocalGame {
    fn root(&self) -> &Path {
        &self.candidate.path
    }
    fn file(&self, name: &str) -> PathBuf {
        self.state.join(name)
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

struct Service {
    snapshot: Mutex<Snapshot>,
    config: Option<Config>,
    config_path: Option<PathBuf>,
    config_error: Option<String>,
    layout: Option<Layout>,
    probe: Probe,
    host: Arc<dyn Host>,
    timing: Timing,
    /// The last release that passed signature verification; selections are checked against it.
    catalog: Mutex<Option<Arc<crate::model::Manifest>>>,
    running: AtomicBool,
    control: Mutex<Control>,
    pause: AtomicBool,
    cancel: AtomicBool,
    local: Mutex<Option<LocalGame>>,
    launch: Mutex<LaunchCtl>,
    updating: AtomicBool,
    closed: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One running interface session: discovery, maintenance or the local view, and Play.
pub struct Session(Arc<Service>);

impl Session {
    /// Starts discovery and the game monitor in the background, and returns at once.
    pub fn start(options: Options) -> Self {
        let loaded = options.config.as_ref().map(|path| -> Result<Config> {
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
        let staged = options
            .layout
            .as_ref()
            .and_then(|l| crate::app::InstallState::load(l).ok())
            .and_then(|s| s.staged);
        let s = Arc::new(Service {
            snapshot: Mutex::new(Snapshot {
                app_update: staged,
                ..Snapshot::default()
            }),
            config,
            config_path: options.config,
            config_error,
            layout: options.layout,
            probe: options.probe,
            host: options.host,
            timing: options.timing,
            catalog: Mutex::new(None),
            running: AtomicBool::new(false),
            control: Mutex::new(Control::default()),
            pause: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            local: Mutex::new(None),
            launch: Mutex::new(LaunchCtl::default()),
            updating: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let init = s.clone();
        std::thread::spawn(move || init.discover());
        let monitor = s.clone();
        std::thread::spawn(move || monitor.monitor());
        Self(s)
    }

    /// The current snapshot, as the interface renders it.
    pub fn snapshot(&self) -> Value {
        serde_json::to_value(self.0.published()).unwrap_or_else(|_| json!({}))
    }

    /// The snapshot's revision, without building the snapshot.
    pub fn revision(&self) -> u64 {
        lock(&self.0.snapshot).revision
    }

    /// Runs one command. Returns `{"id", "accepted": true}` or `{"accepted": false, "error"}`.
    pub fn command(&self, input: &[u8]) -> Value {
        let r = (|| -> Result<Value> {
            ensure!(input.len() <= 65536, "Command is too large");
            self.0.command(crate::strict_json::parse(input)?)
        })();
        match r {
            Ok(v) => v,
            Err(e) => json!({"accepted":false,"error":e.to_string()}),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.cancel.store(true, Ordering::Release);
    }
}

/// The engine scheduled maintenance runs: the stable launcher when installed, so the timer
/// keeps working across updates; otherwise the engine shipped beside the interface.
fn engine_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PATCHER_ENGINE") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(layout) = Layout::for_user()
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
        .map(|g| {
            let answered = policy.desired.dlc.contains_key(&g.id);
            let deferred = policy.desired.deferred.contains(&g.id);
            Pack {
                id: g.id.clone(),
                name: g.display_name.clone(),
                selected: policy.desired.dlc.get(&g.id) == Some(&true),
                answered,
                deferred,
                undecided: !answered && !deferred,
                kind: local::kind_of(&g.id).as_str().into(),
                status: String::new(),
                installed_bytes: 0,
                issues: vec![],
                bytes: manifest
                    .files
                    .iter()
                    .filter(|f| g.file_ids.contains(&f.id))
                    .map(|f| f.full.encoded_size)
                    .sum(),
            }
        })
        .collect()
}
/// "3 of 5 packs selected - 1 new pack to review", for packs from a release.
fn feed_summary(packs: &[Pack]) -> String {
    if packs.is_empty() {
        return "Shown once the release list loads".into();
    }
    let selected = packs.iter().filter(|p| p.selected).count();
    let fresh = packs.iter().filter(|p| p.undecided).count();
    let summary = format!("{selected} of {} packs selected", packs.len());
    match fresh {
        0 => summary,
        1 => format!("{summary} - 1 new pack to review"),
        n => format!("{summary} - {n} new packs to review"),
    }
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
fn phase_status(phase: &str) -> &'static str {
    match phase {
        "checking" => "Checking",
        "downloading" => "Downloading",
        "applying" => "Applying",
        "verifying" => "Verifying",
        "ready" => "Ready",
        _ => "Working",
    }
}

/// What the run's ending means. A run that completed reports completion, even when a pause
/// arrived after its last stop check; only a run the stop actually ended is suspended.
#[derive(Debug, PartialEq, Eq)]
enum Ending {
    Completed,
    Suspended(&'static str),
    Failed,
}

fn ending(result: &Result<engine::RunReport>, pause: bool, cancel: bool) -> Ending {
    match result {
        Ok(_) => Ending::Completed,
        Err(_) if cancel => Ending::Suspended("cancelled"),
        Err(_) if pause => Ending::Suspended("paused"),
        Err(_) => Ending::Failed,
    }
}

impl Service {
    fn local_mode(&self) -> bool {
        self.config.is_none()
    }
    fn remember(&self, release: &trust::VerifiedRelease) {
        *lock(&self.catalog) = Some(Arc::new(release.manifest().clone()));
    }
    fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut s = lock(&self.snapshot);
        f(&mut s);
        s.revision += 1;
    }
    /// Bumps the revision so the interface fetches the snapshot again.
    fn touch(&self) {
        self.update(|_| {});
    }

    /// The snapshot as the interface sees it: with Play, patcher updates and every label.
    fn published(&self) -> Snapshot {
        let mut s = lock(&self.snapshot).clone();
        let dir = lock(&self.local)
            .as_ref()
            .map(|g| g.state.clone())
            .or_else(|| self.config.as_ref().map(|c| c.state.clone()));
        let (state, detail) = crate::entitlement::unlock_report(
            cfg!(all(windows, target_arch = "x86_64")),
            dir.as_deref(),
        );
        s.unlock = state.into();
        s.unlock_detail = detail;
        {
            let ctl = lock(&self.launch);
            self.overlay_launch(&mut s, &ctl);
        }
        let installed = self.layout.as_ref().is_some_and(|l| l.launcher().is_file());
        if self.updating.load(Ordering::Acquire) {
            s.unavailable.insert(
                "CheckAppUpdate".into(),
                "Checking for a patcher update...".into(),
            );
        } else if installed {
            s.actions.push("CheckAppUpdate".into());
        } else {
            s.unavailable.insert(
                "CheckAppUpdate".into(),
                "This copy of the patcher was not installed by its installer, so it cannot \
                 update itself."
                    .into(),
            );
        }
        let actions = s.actions.clone();
        s.labels.retain(|a, _| actions.contains(a));
        for a in &actions {
            s.labels
                .entry(a.clone())
                .or_insert_with(|| default_label(a).into());
        }
        s
    }

    fn overlay_launch(&self, s: &mut Snapshot, ctl: &LaunchCtl) {
        s.actions.retain(|a| a != "Launch");
        let Some(target) = &ctl.target else {
            s.play = "hidden".into();
            s.play_reason = None;
            if s.primary.as_deref() == Some("Launch") {
                s.primary = None;
            }
            return;
        };
        let busy = match &ctl.phase {
            PlayPhase::Playing => Some((
                "playing",
                "Playing",
                "Close the game first.",
                "The patcher waits until you close the game.",
            )),
            PlayPhase::Starting(_) => Some((
                "starting",
                "Starting",
                "Wait until the game has started.",
                "The game may need a moment to open.",
            )),
            _ => None,
        };
        if let Some((play, status, reason, note)) = busy {
            s.play = play.into();
            s.status = status.into();
            s.play_reason = Some(note.into());
            for a in GAME_FILE_ACTIONS {
                if let Some(i) = s.actions.iter().position(|x| x == a) {
                    s.actions.remove(i);
                    s.unavailable.insert(a.into(), reason.into());
                }
            }
            s.primary = Some("Launch".into());
            return;
        }
        let reason = installation_launch_reason(ctl)
            .or_else(|| s.launch_blocked.clone())
            .or_else(|| {
                (!self.local_mode() && !s.ready)
                    .then(|| "Play is available once maintenance finishes.".to_string())
            });
        match reason {
            Some(reason) => {
                s.play = "unavailable".into();
                s.unavailable.insert("Launch".into(), reason.clone());
                s.play_reason = Some(reason);
                if s.primary.as_deref() == Some("Launch") {
                    s.primary = None;
                }
            }
            None => {
                s.play = "available".into();
                s.actions.insert(0, "Launch".into());
                s.play_reason = match (&ctl.phase, &target.how) {
                    (PlayPhase::Failed(message), _) => Some(message.clone()),
                    _ => None,
                };
            }
        }
    }

    /// Follows the game's process: Starting until it appears (or the wait runs out), Playing
    /// while it runs, then back to idle.
    fn monitor(self: Arc<Self>) {
        while !self.closed.load(Ordering::Acquire) {
            let wait = match lock(&self.launch).phase {
                PlayPhase::Starting(_) => self.timing.poll_starting,
                _ => self.timing.poll_idle,
            };
            std::thread::sleep(wait);
            if lock(&self.launch).target.is_none() {
                continue;
            }
            let detection = self.host.detect();
            let changed = {
                let mut ctl = lock(&self.launch);
                let before = (ctl.phase.clone(), ctl.detection.clone());
                if let Detection::Seen(a) = &detection {
                    let how = ctl.target.as_ref().map(|t| t.how.clone());
                    ctl.phase = match &ctl.phase {
                        _ if a.game => PlayPhase::Playing,
                        PlayPhase::Starting(since)
                            if since.elapsed() >= self.timing.launch_timeout =>
                        {
                            PlayPhase::Failed(did_not_start(&how.unwrap_or(Launch::Steam)))
                        }
                        PlayPhase::Playing => PlayPhase::Idle,
                        other => other.clone(),
                    };
                }
                ctl.detection = Some(detection);
                before != (ctl.phase.clone(), ctl.detection.clone())
            };
            if changed {
                if self.local_mode() {
                    self.publish_local();
                } else {
                    self.touch();
                }
            }
        }
    }

    /// Sets what Play starts, and checks once what this system allows.
    fn set_target(&self, root: Option<(&Path, &str)>, how: Option<Launch>) {
        let target = root.map(|(root, source)| {
            let how = how.unwrap_or_else(|| {
                let runners = local::Runners {
                    steam_links: self.host.handler("steam").is_ok(),
                    wine: self.host.wine(),
                };
                local::how_to_launch(root, source, &runners)
            });
            Target { how }
        });
        let detection = target.as_ref().map(|_| self.host.detect());
        let mut ctl = lock(&self.launch);
        ctl.target = target;
        ctl.detection = detection;
        if ctl.phase != PlayPhase::Playing {
            ctl.phase = PlayPhase::Idle;
        }
        if let Some(Detection::Seen(a)) = &ctl.detection
            && a.game
        {
            ctl.phase = PlayPhase::Playing;
        }
    }

    fn play_phase(&self) -> PlayPhase {
        lock(&self.launch).phase.clone()
    }

    fn launch(self: &Arc<Self>) -> Result<()> {
        let published = self.published();
        if !published.actions.iter().any(|a| a == "Launch") {
            bail!(
                "{}",
                published
                    .unavailable
                    .get("Launch")
                    .cloned()
                    .unwrap_or_else(|| "Play is not available right now".into())
            );
        }
        let how = {
            let mut ctl = lock(&self.launch);
            let how = ctl
                .target
                .as_ref()
                .map(|t| t.how.clone())
                .context("No installation")?;
            ctl.phase = PlayPhase::Starting(Instant::now());
            how
        };
        self.after_launch_change();
        let service = self.clone();
        std::thread::spawn(move || {
            let started = match &how {
                Launch::Steam => service
                    .host
                    .handler("steam")
                    .map_err(|e| anyhow!(e))
                    .and_then(|()| {
                        service
                            .host
                            .open(&format!("steam://rungameid/{}", local::STEAM_APP_ID))
                    }),
                Launch::Program(run) => service.host.start(run),
                Launch::Unsupported(reason) => Err(anyhow!("{reason}")),
            };
            let untracked = matches!(
                lock(&service.launch).detection,
                Some(Detection::Unsupported(_))
            );
            match started {
                Err(e) => {
                    lock(&service.launch).phase =
                        PlayPhase::Failed(format!("The game could not be started: {e:#}"));
                    service.after_launch_change();
                }
                // Nothing here can see the game's process, so there is no Starting to follow.
                Ok(()) if untracked => {
                    lock(&service.launch).phase = PlayPhase::Idle;
                    service.after_launch_change();
                }
                Ok(()) => {}
            }
        });
        Ok(())
    }

    fn after_launch_change(&self) {
        if self.local_mode() {
            self.publish_local();
        } else {
            self.touch();
        }
    }

    /// Presents a known blocker as one sentence and one next action.
    fn blocked(self: &Arc<Self>, blocker: Blocker) {
        let (state, title, detail, next, actions, primary): (
            &str,
            &str,
            &str,
            &str,
            &[&str],
            Option<&str>,
        ) = match blocker {
            Blocker::GameRunning => (
                "waiting",
                "Waiting for game to close",
                "Files are not changed while The Sims 4 or its installer is running.",
                "Maintenance continues automatically after the game closes.",
                &["Cancel"],
                None,
            ),
            Blocker::UnknownVersion => (
                "needs_attention",
                "Game updated elsewhere",
                "This game version is newer or different from the published release, so nothing was changed.",
                "Maintenance resumes when a release for this version is published.",
                &["Verify"],
                None,
            ),
            Blocker::DiskSpace => (
                "needs_attention",
                "Not enough disk space",
                "Free space is needed for downloads and safe replacement of files.",
                "Free some space, then try again.",
                &["Start"],
                Some("Start"),
            ),
            Blocker::Network => (
                "needs_attention",
                "Offline",
                "The download server could not be reached. Completed work is kept.",
                "Maintenance retries on the next launch, or try again now.",
                &["Start"],
                Some("Start"),
            ),
            Blocker::Stopped => (
                "paused",
                "Paused",
                "Committed files stay in place.",
                "Nothing restarts until you choose to continue.",
                &["Resume"],
                Some("Resume"),
            ),
        };
        self.update(|s| {
            s.state = state.into();
            s.phase = state.into();
            s.title = title.into();
            s.status = title.into();
            s.detail = detail.into();
            s.error = Some(blocker.to_string());
            s.ready = false;
            s.bytes_per_second = 0;
            s.show_progress = state != "needs_attention";
            set_actions(s, actions, primary);
            if actions.contains(&"Start") {
                label(s, "Start", "Try again");
            }
            s.next_action = next.into();
        });
        if blocker == Blocker::GameRunning {
            let service = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(service.timing.game_recheck);
                let waiting = lock(&service.snapshot).state == "waiting";
                if waiting {
                    service.start(false, false);
                }
            });
        }
    }
    fn failure(&self, error: &str) {
        let local = self.local_mode();
        self.update(|s| {
            s.state = "needs_attention".into();
            s.phase = "needs_attention".into();
            s.title = "Needs attention".into();
            s.status = "Needs attention".into();
            s.error = Some(error.into());
            s.detail = error.into();
            s.ready = false;
            s.show_progress = false;
            if local {
                set_actions(s, &["Discover", "ConfirmInstall"], Some("Discover"));
                s.next_action = "Search again, or choose the game folder yourself.".into();
            } else {
                set_actions(s, &["Start", "Discover"], Some("Start"));
                label(s, "Start", "Try again");
                s.next_action = "Review Details, then try again when the cause is resolved.".into();
            }
        });
    }
    fn suspended(&self, reason: &str) {
        let cancelled = reason == "cancelled";
        self.update(|s| {
            s.state = reason.into();
            s.phase = reason.into();
            s.title = if cancelled { "Stopped" } else { "Paused" }.into();
            s.status = s.title.clone();
            s.detail = "Committed files stay in place. Play is unavailable until maintenance \
                        finishes verification."
                .into();
            s.ready = false;
            s.bytes_per_second = 0;
            s.show_progress = true;
            let lead = if cancelled { "Start" } else { "Resume" };
            set_actions(s, &[lead, "SetPolicy"], Some(lead));
            s.next_action = "Nothing restarts until you choose to continue.".into();
        });
    }
    fn discover(self: &Arc<Self>) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let _ = crate::entitlement::install_windows_component();
        }
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
            let fixture = policy.install.variant == "fixture";
            let root = policy.install.roots.get("game").cloned();
            match &root {
                Some(root) if fixture => self.set_target(
                    Some((root, "")),
                    Some(Launch::Unsupported(
                        "Game launch is unavailable for a test installation.".into(),
                    )),
                ),
                Some(root) => self.set_target(Some((root, "")), None),
                None => self.set_target(None, None),
            }
            self.update(|s| {
                s.fixture = fixture;
                s.path = root.as_ref().map(|p| p.display().to_string());
                s.policy_revision = policy.revision;
                s.preferences = policy.preferences.clone();
                s.language = policy.desired.language.clone();
                s.downloads_supported = true;
                s.background_supported = cfg!(target_os = "linux");
                s.background_note = cfg!(target_os = "linux").then(|| {
                    "Checks at login and every 6 hours. Files never change while the game is \
                     running."
                        .into()
                });
                s.mode_note = if fixture {
                    "Test mode uses an independent local signing key and synthetic content."
                } else {
                    "The patcher replaces damaged or outdated game files from verified releases. \
                     Saves and mods are never changed."
                }
                .into();
            });
            match &policy.suspended {
                Some(reason) => {
                    self.suspended(reason);
                    match feed(cfg) {
                        Ok(release) => {
                            self.remember(&release);
                            let list = packs(&release, &policy);
                            self.update(|s| {
                                s.collection = feed_summary(&list);
                                s.packs = list;
                            });
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
        self.discover_local();
    }

    fn discover_local(self: &Arc<Self>) {
        let saved = self
            .layout
            .as_ref()
            .and_then(crate::app::saved_installation);
        let d = crate::discovery::discover(&self.probe, saved.as_deref());
        let adopted = adopt(&d, saved.is_some()).cloned();
        if adopted.is_none() {
            *lock(&self.local) = None;
            self.set_target(None, None);
        }
        if let Some(c) = &adopted
            && saved.as_deref() != Some(c.path.as_path())
        {
            let saved = self
                .layout
                .as_ref()
                .context("The patcher has no place to keep its data on this system")
                .and_then(|layout| crate::app::save_installation(layout, &c.path));
            if let Err(e) = saved {
                self.failure(&format!("Cannot save the game location: {e:#}"));
                self.update(|s| {
                    s.path = Some(c.path.display().to_string());
                    s.checked = d.checked.clone();
                });
                return;
            }
        }
        self.update(|s| {
            s.state = "setup".into();
            s.phase = "setup".into();
            s.status = "Setup".into();
            s.ready = false;
            s.error = None;
            s.show_progress = false;
            s.mode_note = "The patcher reads your game folder and never changes game files.".into();
            match &adopted {
                Some(c) => {
                    s.path = Some(c.path.display().to_string());
                    s.title = "Game found".into();
                    s.detail = "Reading your game folder.".into();
                    s.candidates = vec![];
                    set_actions(s, &["Discover", "ConfirmInstall"], None);
                    label(s, "ConfirmInstall", "Use a different folder");
                    s.next_action = "Your packs appear in a moment.".into();
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
                            "The Sims 4 was not found in the usual places listed in Details."
                                .into(),
                        )
                    };
                    s.candidates = d.candidates.clone();
                    if d.candidates.is_empty() {
                        set_actions(s, &["ConfirmInstall", "Discover"], Some("ConfirmInstall"));
                    } else {
                        set_actions(s, &["Discover", "ConfirmInstall"], None);
                        label(s, "ConfirmInstall", "Use a different folder");
                    }
                    s.next_action = "Pick the folder that contains Game and Data.".into();
                    s.collection = "Shown once your game is found".into();
                    s.packs = vec![];
                }
            }
            s.checked = d.checked.clone();
        });
        if let Some(c) = &adopted {
            self.open_local(c);
        }
    }

    fn start(self: &Arc<Self>, resume: bool, full_verify: bool) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        // Install the stop control before the worker exists so an early Pause is never lost.
        let control = Control::default();
        *lock(&self.control) = control.clone();
        self.pause.store(false, Ordering::Release);
        self.cancel.store(false, Ordering::Release);
        self.update(|s| {
            s.state = "checking".into();
            s.phase = "checking".into();
            s.status = "Checking".into();
            s.ready = false;
            s.error = None;
            s.show_progress = true;
            s.title = "Checking".into();
            s.detail = "Verifying the signed release and your selections.".into();
            set_actions(s, &["Pause", "Cancel"], None);
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
            s.collection = feed_summary(&list);
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
        let pause = self.pause.load(Ordering::Acquire);
        let cancel = self.cancel.load(Ordering::Acquire);
        match ending(&result, pause, cancel) {
            Ending::Suspended(reason) => {
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
            }
            Ending::Completed => {
                let Ok(report) = result else { return };
                self.update(|s| {
                    s.state = "ready".into();
                    s.phase = "ready".into();
                    s.status = "Ready".into();
                    s.ready = true;
                    s.bytes_per_second = 0;
                    s.show_progress = false;
                    s.title = "Ready".into();
                    s.detail = if s.fixture {
                        "All selected test files passed verification. Your game was not touched."
                    } else {
                        "Every selected file matches the signed release."
                    }
                    .into();
                    s.details = report_lines(&report);
                    set_actions(s, &["Verify", "SetPolicy"], Some("Launch"));
                    s.next_action =
                        "Maintenance runs again the next time the patcher starts.".into();
                });
            }
            Ending::Failed => {
                if let Err(e) = result {
                    match e.downcast_ref::<Blocker>() {
                        Some(blocker) => self.blocked(*blocker),
                        None => self.failure(&format!("{e:#}")),
                    }
                }
            }
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
        match self.play_phase() {
            PlayPhase::Playing if GAME_FILE_ACTIONS.contains(&command) => {
                bail!("Close the game first")
            }
            PlayPhase::Starting(_) if GAME_FILE_ACTIONS.contains(&command) => {
                bail!("Wait until the game has started")
            }
            _ => {}
        }
        match command {
            "Launch" => {
                self.launch()?;
                return Ok(json!({"id":id,"accepted":true}));
            }
            "CheckAppUpdate" => {
                self.check_app_update()?;
                return Ok(json!({"id":id,"accepted":true}));
            }
            _ => {}
        }
        if self.local_mode() && self.local_command(command, &v)? {
            return Ok(json!({"id":id,"accepted":true}));
        }
        match command {
            "Discover" => {
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Wait for the current work to finish"
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
                let waiting = lock(&self.snapshot).state == "waiting";
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
                lock(&self.control).stop();
            }
            "SetPolicy" => self.set_feed_policy(&v)?,
            "ConfirmInstall" => {
                ensure!(
                    !self.running.load(Ordering::Acquire),
                    "Wait for the current work to finish"
                );
                let path = v
                    .get("path")
                    .and_then(Value::as_str)
                    .context("Choose a folder")?;
                let c = crate::discovery::validate(
                    Path::new(path),
                    "folder picker",
                    self.probe.variant,
                )?;
                let layout = self
                    .layout
                    .as_ref()
                    .context("The patcher has no place to keep its data on this system")?;
                crate::app::save_installation(layout, &c.path)?;
                self.update(|s| {
                    s.path = Some(c.path.display().to_string());
                    s.title = "Game found".into();
                    s.detail = "Reading your game folder.".into();
                    s.candidates = vec![];
                });
                if self.local_mode() {
                    let s = self.clone();
                    std::thread::spawn(move || s.open_local(&c));
                }
            }
            "Connect" | "Disconnect" => {
                bail!("Online sign-in is unavailable: provider handoff has not been validated")
            }
            _ => bail!("Unknown command"),
        }
        Ok(json!({"id":id,"accepted":true}))
    }

    fn set_feed_policy(&self, v: &Value) -> Result<()> {
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
        let catalog = lock(&self.catalog)
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
                if let Err(undo) = crate::os::set_schedule(restore, &engine_path()?, config) {
                    bail!("{e:#}; the maintenance schedule could not be restored: {undo:#}");
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
                pack.undecided = !pack.answered && !pack.deferred;
            }
            s.collection = feed_summary(&s.packs);
            if selection_changed {
                s.ready = false;
                s.state = "planned".into();
                s.phase = "planned".into();
                s.status = "Selections saved".into();
                s.show_progress = false;
                s.title = "Selections saved".into();
                s.detail = "Your changes apply the next time maintenance runs.".into();
                set_actions(s, &["Start", "SetPolicy"], Some("Start"));
                s.next_action = "Start maintenance to apply your selections.".into();
            }
        });
        Ok(())
    }

    fn check_app_update(self: &Arc<Self>) -> Result<()> {
        let published = self.published();
        if !published.actions.iter().any(|a| a == "CheckAppUpdate") {
            bail!(
                "{}",
                published
                    .unavailable
                    .get("CheckAppUpdate")
                    .cloned()
                    .unwrap_or_else(|| "A patcher update check is not available".into())
            );
        }
        ensure!(
            !self.updating.swap(true, Ordering::AcqRel),
            "A patcher update check is already running"
        );
        self.update(|s| s.app_update_status = Some("Checking for a patcher update...".into()));
        let service = self.clone();
        std::thread::spawn(move || {
            let result = service.host.stage_app_update();
            service.updating.store(false, Ordering::Release);
            service.update(|s| match result {
                Ok(Some(v)) => {
                    s.app_update_status = Some(format!(
                        "Universal Patcher {v} is ready. Restart the patcher to use it."
                    ));
                    s.app_update = Some(v);
                }
                Ok(None) => {
                    s.app_update_status = Some(format!(
                        "You have the newest patcher ({}).",
                        crate::app::APP_VERSION
                    ))
                }
                Err(e) => {
                    s.app_update_status =
                        Some(format!("Could not check for a patcher update: {e:#}"))
                }
            });
        });
        Ok(())
    }
}

impl Service {
    /// Adopts `c` as the installation to look after: its saved choices, or new ones for a
    /// first run. Choices saved for an earlier identity of a folder are kept and the user is
    /// asked to confirm the move. The folder is read on this (worker) thread.
    fn open_local(self: &Arc<Self>, c: &Candidate) {
        let opened = (|| -> Result<(PathBuf, Policy, Option<String>)> {
            let dir = self
                .layout
                .as_ref()
                .context("The patcher has no place to keep its data on this system")?
                .root
                .join("game");
            fs::create_dir_all(&dir)?;
            match Policy::load(&dir) {
                Ok(p) if p.install.identity == c.identity => Ok((dir, p, None)),
                Ok(p) if p.game_id == "sims4" => {
                    let from = p
                        .install
                        .roots
                        .get("game")
                        .map(|r| r.display().to_string())
                        .unwrap_or_default();
                    Ok((dir, p, Some(from)))
                }
                _ => {
                    // A first run, or both policy copies unreadable: ask again rather than guess.
                    let p = new_policy(c);
                    p.save(&dir)?;
                    Ok((dir, p, None))
                }
            }
        })();
        let (state, policy, location_from) = match opened {
            Ok(v) => v,
            Err(e) => return self.failure(&format!("Cannot save your choices: {e:#}")),
        };
        #[cfg(all(windows, target_arch = "x86_64"))]
        let component_note = match crate::entitlement::install_windows_component() {
            Ok(path) => format!("Entitlement component installed at {}.", path.display()),
            Err("component-missing") => {
                "The entitlement component is not in this patcher release.".into()
            }
            Err("ea-desktop-missing") => {
                "EA Desktop was not found, so the entitlement component was not installed.".into()
            }
            Err("component-replace") => "EA Desktop is using the entitlement component. Close \
                 EA Desktop and open the patcher again."
                .into(),
            Err("component-access") => "The entitlement component needs Windows permission to be \
                 placed beside EA Desktop. Open the patcher again and allow the prompt."
                .into(),
            Err(_) => "The entitlement component could not be installed.".into(),
        };
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        let component_note = String::new();
        *lock(&self.local) = Some(LocalGame {
            candidate: c.clone(),
            state,
            policy,
            location_from,
            packs: None,
            languages: vec![],
            lines: if component_note.is_empty() {
                vec![]
            } else {
                vec![component_note]
            },
        });
        self.set_target(Some((&c.path, &c.source)), None);
        self.publish_local();
        self.rescan();
    }

    /// Reads the folder for packs and languages, then publishes. Runs on a worker thread.
    fn rescan(&self) {
        let Some(root) = lock(&self.local).as_ref().map(|g| g.root().to_path_buf()) else {
            return;
        };
        let packs = local::scan(&root);
        let languages = local::languages(&root);
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            if let Some(state) = lock(&self.local).as_ref().map(|g| g.state.clone()) {
                let installed: std::collections::BTreeSet<String> = packs
                    .iter()
                    .filter(|p| p.status != local::Status::NotInstalled)
                    .map(|p| p.id.clone())
                    .collect();
                let _ = crate::entitlement::record_installed_selection(&state, &installed);
            }
        }
        if let Some(g) = lock(&self.local).as_mut()
            && g.root() == root
        {
            if let Ok(policy) = Policy::load(&g.state) {
                g.policy = policy;
            }
            g.packs = Some(packs);
            g.languages = languages;
        }
        self.publish_local();
    }

    /// Builds the snapshot for the user's own installation from what is known about it.
    fn publish_local(&self) {
        let guard = lock(&self.local);
        let Some(g) = guard.as_ref() else { return };
        let launch_reason = installation_launch_reason(&lock(&self.launch));
        let packs: Vec<Pack> = g
            .packs
            .iter()
            .flatten()
            .map(|p| Pack {
                id: p.id.clone(),
                name: p.name.clone(),
                selected: p.status != local::Status::NotInstalled,
                answered: g.policy.desired.dlc.contains_key(&p.id),
                deferred: false,
                undecided: false,
                bytes: 0,
                kind: p.kind.as_str().into(),
                status: match p.status {
                    local::Status::NotInstalled => "not_installed",
                    local::Status::Incomplete => "incomplete",
                    local::Status::Installed => "installed",
                }
                .into(),
                installed_bytes: p.bytes,
                issues: p.missing.iter().map(|m| format!("Missing: {m}")).collect(),
            })
            .collect();
        let scanned = g.packs.is_some();
        let installed = packs.iter().filter(|p| p.status != "not_installed").count();
        let incomplete = packs.iter().filter(|p| p.status == "incomplete").count();
        let on_disk: u64 = packs.iter().map(|p| p.installed_bytes).sum();
        let collection = if !scanned {
            "Reading your game folder...".to_string()
        } else {
            let mut line = format!("{installed} of {} packs installed", packs.len());
            match incomplete {
                0 => {}
                1 => line.push_str(" - 1 is missing files"),
                n => line.push_str(&format!(" - {n} are missing files")),
            }
            line
        };
        let lines = g.lines.clone();
        let languages = g.languages.clone();
        let policy = g.policy.clone();
        let location_from = g.location_from.clone();
        let root = g.root().display().to_string();
        drop(guard);

        self.update(|s| {
            s.fixture = false;
            s.ready = false;
            s.error = None;
            s.path = Some(root.clone());
            s.candidates = vec![];
            s.policy_revision = policy.revision;
            s.preferences = policy.preferences.clone();
            s.language = policy.desired.language.clone();
            s.languages = languages;
            s.downloads_supported = false;
            s.background_supported = false;
            s.background_note = None;
            s.mode_note =
                "The patcher reads your game folder and does not change game files.".into();
            s.checked_at = None;
            s.collection = collection;
            s.packs = packs;
            s.details = lines;
            s.launch_blocked = None;
            s.bytes_per_second = 0;
            s.show_progress = false;
            s.files_done = 0;
            s.files_total = 0;
            s.bytes_done = 0;
            s.bytes_total = 0;
            s.next_action = String::new();
            let mut extra: Vec<&str> = vec![];
            if scanned {
                extra.push("ViewPacks");
            }
            if let Some(from) = &location_from {
                s.state = "confirm_location".into();
                s.phase = s.state.clone();
                s.status = "Needs attention".into();
                s.title = "Confirm your game folder".into();
                s.detail = format!(
                    "Your choices were saved for the game folder at {from}, which has since been \
                     replaced or moved. They are kept."
                );
                s.next_action = "Keep your choices for this folder, or choose another one.".into();
                s.launch_blocked = Some("Confirm your game folder first.".into());
                set_actions(
                    s,
                    &["ConfirmLocation", "Discover", "ConfirmInstall"],
                    Some("ConfirmLocation"),
                );
                label(s, "ConfirmInstall", "Use a different folder");
                return;
            }
            s.state = "installed".into();
            s.phase = "installed".into();
            let playable = launch_reason.is_none();
            s.status = if playable {
                "Ready to play"
            } else {
                "Installed"
            }
            .into();
            s.title = s.status.clone();
            s.detail = if scanned {
                format!(
                    "{installed} of {} packs installed, {} on disk.",
                    s.packs.len(),
                    gib(on_disk)
                )
            } else {
                "Reading your game folder...".into()
            };
            let mut actions = vec!["SetPolicy"];
            actions.extend(extra);
            actions.extend(["Discover", "ConfirmInstall"]);
            set_actions(s, &actions, Some("Launch"));
            label(s, "ConfirmInstall", "Choose another installation");
        });
    }

    /// Commands for the user's own installation. Returns false when `command` is not one of
    /// them, so the caller handles it.
    fn local_command(self: &Arc<Self>, command: &str, v: &Value) -> Result<bool> {
        if lock(&self.local).is_none() {
            return Ok(false);
        }
        match command {
            "SetPolicy" => self.set_local_policy(v)?,
            "ConfirmLocation" => {
                {
                    let mut guard = lock(&self.local);
                    let g = guard.as_mut().context("No installation")?;
                    ensure!(
                        g.location_from.is_some(),
                        "There is no folder change to confirm"
                    );
                    let _lock = Lock::acquire(&g.file("session.lock"))?;
                    let mut p = g.policy.clone();
                    p.install = crate::storage::Installation {
                        identity: g.candidate.identity.clone(),
                        variant: g.candidate.variant.clone(),
                        roots: BTreeMap::from([("game".to_string(), g.candidate.path.clone())]),
                    };
                    p.revision += 1;
                    p.save(&g.state)?;
                    g.policy = p;
                    g.location_from = None;
                    g.lines = vec!["Your choices now belong to this folder.".into()];
                }
                self.publish_local();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn set_local_policy(&self, v: &Value) -> Result<()> {
        ensure!(
            v.get("dlc").is_none() && v.get("deferred").is_none(),
            "Pack choices take effect only once releases are published, so there is nothing to \
             save yet"
        );
        {
            let mut guard = lock(&self.local);
            let g = guard.as_mut().context("No installation")?;
            ensure!(g.location_from.is_none(), "Confirm your game folder first");
            let _lock = Lock::acquire(&g.file("session.lock"))?;
            let mut p = g.policy.clone();
            ensure!(
                v.get("revision").and_then(Value::as_u64) == Some(p.revision),
                "Your choices changed; reopen Settings"
            );
            if let Some(language) = v.get("language") {
                let language: Option<String> = serde_json::from_value(language.clone())?;
                if let Some(code) = &language {
                    ensure!(
                        g.languages.contains(code),
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
            p.save(&g.state)?;
            g.policy = p;
        }
        self.publish_local();
        Ok(())
    }
}

fn new_policy(c: &Candidate) -> Policy {
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

fn string(value: Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap().into_raw()
}

static SESSION: OnceLock<Session> = OnceLock::new();
fn session() -> &'static Session {
    SESSION.get_or_init(|| Session::start(Options::from_env()))
}
static HEALTH: std::sync::Once = std::sync::Once::new();

#[unsafe(no_mangle)]
pub extern "C" fn patcher_snapshot() -> *mut c_char {
    std::panic::catch_unwind(|| {
        let s = session();
        // The interface is reading snapshots, so this release started correctly.
        HEALTH.call_once(crate::app::report_healthy);
        string(s.snapshot())
    })
    .unwrap_or_else(|_| string(json!({"error":"Native snapshot failed"})))
}

/// The snapshot's revision; the interface reads it every frame and fetches the snapshot only
/// when it changed.
#[unsafe(no_mangle)]
pub extern "C" fn patcher_revision() -> u64 {
    std::panic::catch_unwind(|| session().revision()).unwrap_or(u64::MAX)
}

/// # Safety
/// `input` must point to a valid, NUL-terminated UTF-8 string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn patcher_command(input: *const c_char) -> *mut c_char {
    std::panic::catch_unwind(|| {
        if input.is_null() {
            return string(json!({"accepted":false,"error":"Missing command"}));
        }
        let bytes = unsafe { CStr::from_ptr(input) }.to_bytes();
        string(session().command(bytes))
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

    #[test]
    fn a_run_that_completed_reports_completion_even_if_a_pause_came_late() {
        let done: Result<engine::RunReport> = Ok(engine::RunReport::default());
        assert_eq!(ending(&done, true, false), Ending::Completed);
        assert_eq!(ending(&done, false, true), Ending::Completed);
        let stopped: Result<engine::RunReport> = Err(anyhow::Error::msg(Blocker::Stopped));
        assert_eq!(ending(&stopped, true, false), Ending::Suspended("paused"));
        assert_eq!(ending(&stopped, true, true), Ending::Suspended("cancelled"));
        let failed: Result<engine::RunReport> = Err(anyhow!("disk"));
        assert_eq!(ending(&failed, false, false), Ending::Failed);
    }

    fn candidate(path: &str, source: &str) -> Candidate {
        Candidate {
            path: path.into(),
            identity: path.into(),
            variant: "windows-native".into(),
            source: source.into(),
            maintenance_supported: false,
        }
    }

    fn found(candidates: Vec<Candidate>) -> crate::discovery::Discovery {
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
