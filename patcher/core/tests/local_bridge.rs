//! The app on a real installation, with no release feed: discovery adopts the game, packs show
//! their status and Play starts the game, wherever it came from, and follows its process.
//! Sessions use a fake host, so no test starts a program or depends on what runs on this
//! computer.
#![cfg(target_os = "linux")]
use patcher_core::{
    app::Layout,
    bridge::{
        Options, Session, Timing, patcher_command, patcher_free, patcher_revision, patcher_snapshot,
    },
    discovery::Probe,
    local::Run,
    process::{Activity, Detection, Host},
};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn pack(root: &Path, id: &str) {
    for f in [
        "ClientFullBuild0.package",
        "SimulationFullBuild0.package",
        "ResourceClient.cfg",
        "ResourceSimulation.cfg",
        "Version.ini",
        "thumbnails.package",
    ] {
        write(root, &format!("{id}/{f}"), &format!("{id} {f}"));
    }
    for f in [
        "ClientDeltaBuild0.package",
        "ClipHeader.package",
        "ResourceClient.cfg",
        "ResourceSimulation.cfg",
        "SimulationDeltaBuild0.package",
        "SimulationPreload.package",
        "Version.ini",
    ] {
        write(root, &format!("Delta/{id}/{f}"), &format!("delta {id} {f}"));
    }
}
fn base_game(game: &Path) {
    write(game, "Data/Client/ClientFullBuild0.package", "base");
    write(game, "Data/Client/Strings_ENG_US.package", "en");
    write(game, "Data/Client/Strings_GER_DE.package", "de");
    write(game, "Game/Bin/TS4_x64.exe", "exe");
    pack(game, "EP01");
}
fn pack_of<'a>(s: &'a Value, id: &str) -> &'a Value {
    s["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap()
}
fn has(s: &Value, action: &str) -> bool {
    s["actions"].as_array().unwrap().contains(&json!(action))
}

/// Stands in for the system: which programs run, which links open, and patcher updates.
#[derive(Default)]
struct FakeHost {
    game: AtomicBool,
    /// Opening a Steam run link starts the game, as Steam would.
    steam_starts_game: AtomicBool,
    no_handler: AtomicBool,
    opened: Mutex<Vec<String>>,
    started: Mutex<Vec<Run>>,
    /// Starting a program starts the game.
    program_starts_game: AtomicBool,
    wine: Mutex<Option<PathBuf>>,
    update_delay: Mutex<Duration>,
}

impl Host for FakeHost {
    fn detect(&self) -> Detection {
        Detection::Seen(Activity {
            game: self.game.load(Ordering::Acquire),
            launcher: false,
            installer: false,
        })
    }
    fn handler(&self, _scheme: &str) -> Result<(), String> {
        if self.no_handler.load(Ordering::Acquire) {
            Err("Steam is not set up to open Steam links on this computer. Start Steam once, then try again".into())
        } else {
            Ok(())
        }
    }
    fn open(&self, url: &str) -> anyhow::Result<()> {
        self.opened.lock().unwrap().push(url.into());
        if url.starts_with("steam://rungameid/") && self.steam_starts_game.load(Ordering::Acquire) {
            self.game.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn start(&self, run: &Run) -> anyhow::Result<()> {
        self.started.lock().unwrap().push(run.clone());
        if self.program_starts_game.load(Ordering::Acquire) {
            self.game.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn wine(&self) -> Option<PathBuf> {
        self.wine.lock().unwrap().clone()
    }
    fn stage_app_update(&self) -> anyhow::Result<Option<String>> {
        std::thread::sleep(*self.update_delay.lock().unwrap());
        Ok(Some("9.9.9".into()))
    }
}

struct World {
    _dir: tempfile::TempDir,
    home: PathBuf,
    data: PathBuf,
    host: Arc<FakeHost>,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let data = dir.path().join("data");
        fs::create_dir_all(&home).unwrap();
        Self {
            home,
            data,
            host: Arc::new(FakeHost::default()),
            _dir: dir,
        }
    }
    fn steam_game(&self) -> PathBuf {
        self.home
            .join(".local/share/Steam/steamapps/common/The Sims 4")
    }
    fn session(&self) -> App {
        self.session_with(Layout::at(self.data.clone()))
    }
    fn session_with(&self, layout: Layout) -> App {
        App(Session::start(Options {
            config: None,
            layout: Some(layout),
            probe: Probe::unix(&self.home, false),
            host: self.host.clone(),
            timing: Timing {
                launch_timeout: Duration::from_millis(400),
                poll_starting: Duration::from_millis(10),
                poll_idle: Duration::from_millis(20),
                game_recheck: Duration::from_secs(30),
            },
        }))
    }
}

struct App(Session);

impl App {
    fn snapshot(&self) -> Value {
        self.0.snapshot()
    }
    fn command(&self, name: &str, mut args: Value) -> Value {
        args["id"] = json!(name);
        args["command"] = json!(name);
        self.0.command(args.to_string().as_bytes())
    }
    fn accept(&self, name: &str, args: Value) {
        let r = self.command(name, args);
        assert_eq!(r["accepted"], true, "{name}: {r}");
    }
    fn wait(&self, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            let s = self.snapshot();
            if done(&s) {
                return s;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "never {what}: {s:#}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// Waits until the installation is shown with its packs read.
    fn idle(&self) -> Value {
        self.wait("shows the installation", |s| {
            s["state"] == "installed" && s["collection"] != "Reading your game folder..."
        })
    }
}

#[test]
fn play_follows_the_game_from_starting_to_playing_and_back() {
    let w = World::new();
    base_game(&w.steam_game());
    let app = w.session();
    let s = app.idle();
    assert_eq!(s["play"], "available", "{s:#}");
    assert_eq!(s["title"], "Ready to play");
    assert_eq!(s["primary"], "Launch");
    assert_eq!(s["labels"]["Launch"], "Play");

    w.host.steam_starts_game.store(true, Ordering::Release);
    app.accept("Launch", json!({}));
    let s = app.snapshot();
    assert!(
        s["play"] == "starting" || s["play"] == "playing",
        "Play shows progress at once: {s:#}"
    );
    let s = app.wait("sees the game", |s| s["play"] == "playing");
    assert_eq!(s["status"], "Playing");
    assert_eq!(
        w.host.opened.lock().unwrap().as_slice(),
        ["steam://rungameid/1222670"]
    );
    // The installation is not swapped, and Play is not pressed twice, while the game runs.
    for action in ["ConfirmInstall", "Discover", "Launch"] {
        assert!(!has(&s, action), "{action} offered while playing");
    }
    assert_eq!(s["unavailable"]["ConfirmInstall"], "Close the game first.");
    assert_eq!(app.command("Launch", json!({}))["accepted"], false);

    w.host.game.store(false, Ordering::Release);
    let s = app.wait("sees the game close", |s| s["play"] == "available");
    assert!(has(&s, "ConfirmInstall"));
    assert!(s["play_reason"].is_null());

    // Steam never starts the game: after the wait, one sentence and one next action.
    w.host.steam_starts_game.store(false, Ordering::Release);
    app.accept("Launch", json!({}));
    let s = app.wait("gives up", |s| {
        s["play"] == "available" && !s["play_reason"].is_null()
    });
    assert_eq!(
        s["play_reason"],
        "The game did not start. Open Steam, then try again."
    );

    // A game started elsewhere is noticed too.
    w.host.game.store(true, Ordering::Release);
    app.wait("notices the running game", |s| s["play"] == "playing");
}

/// A game outside any store - a plain folder or a Wine prefix - is started with Wine, and a
/// Steam library falls back to Wine when Steam links cannot be opened.
#[test]
fn play_starts_a_game_that_no_store_installed() {
    let w = World::new();
    *w.host.wine.lock().unwrap() = Some("/usr/bin/wine".into());
    w.host.program_starts_game.store(true, Ordering::Release);
    let prefix = w.home.join("Games/the-sims-4");
    let other = prefix.join("drive_c/Program Files/EA Games/The Sims 4");
    base_game(&other);
    let app = w.session();
    let s = app.wait("asks for the folder", |s| s["state"] == "setup");
    assert_eq!(s["play"], "hidden");
    assert_eq!(s["primary"], "ConfirmInstall");
    app.accept("ConfirmInstall", json!({"path": other}));
    let s = app.idle();
    assert_eq!(s["play"], "available", "{s:#}");
    assert_eq!(s["title"], "Ready to play");
    app.accept("Launch", json!({}));
    app.wait("sees the game", |s| s["play"] == "playing");
    let started = w.host.started.lock().unwrap().clone();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].program, PathBuf::from("/usr/bin/wine"));
    let exe = fs::canonicalize(other.join("Game/Bin/TS4_x64.exe")).unwrap();
    assert_eq!(started[0].args, [exe.into_os_string()]);
    assert_eq!(
        started[0].env,
        [(
            "WINEPREFIX".to_string(),
            fs::canonicalize(&prefix).unwrap().into_os_string()
        )]
    );
    assert!(
        w.host.opened.lock().unwrap().is_empty(),
        "Steam is not involved"
    );

    // A Steam library on a system that cannot open Steam links: Wine starts it.
    let w = World::new();
    *w.host.wine.lock().unwrap() = Some("/usr/bin/wine".into());
    w.host.no_handler.store(true, Ordering::Release);
    base_game(&w.steam_game());
    let app = w.session();
    let s = app.idle();
    assert_eq!(s["play"], "available", "{s:#}");
    app.accept("Launch", json!({}));
    app.wait("starts it", |_| w.host.started.lock().unwrap().len() == 1);
    assert!(w.host.opened.lock().unwrap().is_empty());
}

#[test]
fn play_says_why_when_nothing_here_can_start_the_game() {
    // The Windows version in a plain folder, with neither Steam nor Wine to run it.
    let w = World::new();
    let other = w.home.join("Games/The Sims 4");
    base_game(&other);
    let app = w.session();
    app.wait("asks for the folder", |s| s["state"] == "setup");
    app.accept("ConfirmInstall", json!({"path": other}));
    let s = app.idle();
    assert_eq!(s["play"], "unavailable");
    let reason = s["play_reason"].as_str().unwrap().to_string();
    assert!(reason.contains("Wine is not installed"), "{reason}");
    assert_eq!(s["unavailable"]["Launch"], reason.as_str());
    assert!(!has(&s, "Launch"));
    assert_ne!(s["title"], "Ready to play");
    let r = app.command("Launch", json!({}));
    assert_eq!(
        (r["accepted"].clone(), r["error"].clone()),
        (json!(false), json!(reason))
    );
    assert!(w.host.started.lock().unwrap().is_empty());

    // The game program went missing: the folder is no longer a valid installation, so Play
    // is not offered at all.
    let w = World::new();
    base_game(&w.steam_game());
    w.session().idle();
    fs::remove_file(w.steam_game().join("Game/Bin/TS4_x64.exe")).unwrap();
    let app = w.session();
    let s = app.wait("asks for the folder", |s| s["state"] == "setup");
    assert_eq!(s["play"], "hidden", "{s:#}");
}

#[test]
fn saved_choices_survive_a_recreated_game_folder() {
    let w = World::new();
    let game = w.steam_game();
    base_game(&game);
    let app = w.session();
    let s = app.idle();
    let revision = s["policy_revision"].as_u64().unwrap();
    // Pack choices do nothing on disk without releases, so they are not taken.
    let refused = app.command(
        "SetPolicy",
        json!({"revision": revision, "dlc": {"EP01": false}}),
    );
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("nothing to save yet"),
        "{refused}"
    );
    app.accept(
        "SetPolicy",
        json!({"revision": revision, "language": "GER_DE"}),
    );
    drop(app);

    // The folder is moved away and copied back: same path, new identity.
    let moved = w.home.join("moved");
    fs::rename(&game, &moved).unwrap();
    base_game(&game);
    let app = w.session();
    let s = app.wait("asks to confirm", |s| s["state"] == "confirm_location");
    assert_eq!(s["title"], "Confirm your game folder");
    assert_eq!(s["language"], "GER_DE", "choices kept: {s:#}");
    assert_eq!(s["primary"], "ConfirmLocation");
    assert_eq!(s["play"], "unavailable");
    assert_eq!(app.command("Verify", json!({}))["accepted"], false);
    app.accept("ConfirmLocation", json!({}));
    let s = app.idle();
    assert_eq!(s["language"], "GER_DE");
    // The next session opens without asking again.
    drop(app);
    let app = w.session();
    let s = app.idle();
    assert_eq!(s["language"], "GER_DE");
}

#[test]
fn when_the_location_cannot_be_saved_search_and_folder_choice_stay_offered() {
    let w = World::new();
    base_game(&w.steam_game());
    // The data folder is a file, so nothing can be saved there.
    fs::write(&w.data, "not a folder").unwrap();
    let app = w.session();
    let s = app.wait("fails", |s| s["state"] == "needs_attention");
    assert!(
        s["detail"]
            .as_str()
            .unwrap()
            .starts_with("Cannot save the game location")
    );
    assert!(has(&s, "Discover") && has(&s, "ConfirmInstall"), "{s:#}");
    assert_eq!(s["labels"]["Discover"], "Search again");
    assert_eq!(s["labels"]["ConfirmInstall"], "Choose game folder");
    assert!(!has(&s, "Start"), "no maintenance to start");
}

#[test]
fn a_patcher_update_check_runs_without_blocking() {
    let w = World::new();
    base_game(&w.steam_game());
    let app = w.session();
    let s = app.idle();
    assert!(!has(&s, "CheckAppUpdate"));
    assert!(
        s["unavailable"]["CheckAppUpdate"]
            .as_str()
            .unwrap()
            .contains("not installed by its installer")
    );
    assert!(s["app_update"].is_null());
    drop(app);

    write(&w.data, "bin/universal-patcher", "launcher");
    *w.host.update_delay.lock().unwrap() = Duration::from_millis(300);
    let app = w.session();
    let s = app.idle();
    assert!(has(&s, "CheckAppUpdate"), "{s:#}");
    let started = Instant::now();
    app.accept("CheckAppUpdate", json!({}));
    assert!(started.elapsed() < Duration::from_millis(200));
    assert_eq!(
        app.snapshot()["app_update_status"],
        "Checking for a patcher update..."
    );
    let s = app.wait("stages it", |s| s["app_update"] == "9.9.9");
    assert_eq!(
        s["app_update_status"],
        "Universal Patcher 9.9.9 is ready. Restart the patcher to use it."
    );
}

#[test]
fn choosing_a_folder_returns_before_the_folder_is_read() {
    let w = World::new();
    let game = w.home.join("Games/The Sims 4");
    base_game(&game);
    for id in patcher_core::local::scan(&game)
        .iter()
        .map(|p| p.id.clone())
    {
        for i in 0..150 {
            write(&game, &format!("{id}/extra{i:03}.package"), "x");
        }
    }
    let started = Instant::now();
    patcher_core::local::scan(&game);
    let scan = started.elapsed();
    let app = w.session();
    app.wait("asks for the folder", |s| s["state"] == "setup");
    let started = Instant::now();
    app.accept("ConfirmInstall", json!({"path": game}));
    let command = started.elapsed();
    assert!(
        command < scan,
        "the command took {command:?}; reading takes {scan:?}"
    );
    app.idle();
}

fn take(ptr: *mut std::ffi::c_char) -> Value {
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
    unsafe { patcher_free(ptr) };
    serde_json::from_str(&text).unwrap()
}

/// The exported functions the Flutter shell calls, on the real system's session.
#[test]
fn the_native_interface_serves_the_same_session() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let game = home.join(".local/share/Steam/steamapps/common/The Sims 4");
    base_game(&game);
    // SAFETY: set before the global session is first touched; nothing else in this test
    // binary reads these variables.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("PATCHER_HOME", t.path().join("data"));
        std::env::remove_var("PATCHER_CONFIG");
    }
    let started = Instant::now();
    let s = loop {
        let s = take(patcher_snapshot());
        if s["state"] == "installed" && s["collection"] != "Reading your game folder..." {
            break s;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "{s:#}");
        std::thread::sleep(Duration::from_millis(10));
    };
    let revision = patcher_revision();
    assert_eq!(s["revision"].as_u64(), Some(revision));
    assert_eq!(pack_of(&s, "EP01")["name"], "Get to Work");
    assert_eq!(s["languages"], json!(["ENG_US", "GER_DE"]));
    let input = CString::new(
        json!({"id": "1", "command": "SetPolicy", "revision": s["policy_revision"], "language": "GER_DE"})
            .to_string(),
    )
    .unwrap();
    let r = take(unsafe { patcher_command(input.as_ptr()) });
    assert_eq!(r["accepted"], true, "{r}");
    assert!(patcher_revision() > revision, "a change moves the revision");
    let bad = CString::new(r#"{"id":"2","command":"Verify","command":"Start"}"#).unwrap();
    assert_eq!(
        take(unsafe { patcher_command(bad.as_ptr()) })["accepted"],
        false
    );
}
