use patcher_core::{
    fixture,
    model::Manifest,
    os::{self, Network},
    storage::Policy,
};
use std::{path::Path, path::PathBuf, process::Command};

/// The patcher binary this suite spawns as a child process. `cargo test` on the
/// build host bakes the path in through `CARGO_BIN_EXE_patcher`; `PATCHER_BIN`
/// overrides it so the suite can also run where it was not built, such as a
/// cross-compiled Windows executable observed on a Windows machine.
fn patcher_bin() -> PathBuf {
    std::env::var_os("PATCHER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_patcher")))
}

#[test]
fn network_manager_values_map_to_network_cost() {
    assert_eq!(os::metered_from_nm(0), Network::Unknown);
    assert_eq!(os::metered_from_nm(1), Network::Metered);
    assert_eq!(os::metered_from_nm(2), Network::Unmetered);
    assert_eq!(os::metered_from_nm(3), Network::Metered);
    assert_eq!(os::metered_from_nm(4), Network::Unmetered);
    assert_eq!(os::metered_from_nm(99), Network::Unknown);
}

#[test]
fn unattended_transfers_need_an_unmetered_network_or_opt_in() {
    assert!(os::unattended_allowed(Network::Unmetered, false));
    assert!(!os::unattended_allowed(Network::Metered, false));
    assert!(!os::unattended_allowed(Network::Unknown, false));
    assert!(os::unattended_allowed(Network::Metered, true));
    assert!(os::unattended_allowed(Network::Unknown, true));
}

#[test]
fn timer_runs_at_login_and_every_six_hours() {
    let (service, timer) = os::systemd_units(
        Path::new("/opt/patcher/lib/patcher-engine"),
        Path::new("/home/u/.local/share/patcher/config.json"),
    )
    .unwrap();
    assert!(service.contains(
        "ExecStart=\"/opt/patcher/lib/patcher-engine\" run --scheduled --config \
         \"/home/u/.local/share/patcher/config.json\""
    ));
    assert!(timer.contains("OnStartupSec=2min"));
    assert!(timer.contains("OnUnitActiveSec=6h"));
    for unsafe_path in ["/tmp/a\"b", "/tmp/100%", "/tmp/$HOME", "/tmp/a\nb"] {
        assert!(os::systemd_units(Path::new(unsafe_path), Path::new("/c")).is_err());
    }
}

fn fixture(path: &Path) -> fixture::Config {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fixture::create(path))
        .unwrap()
}

#[test]
fn future_packs_are_adopted_only_with_consent_and_never_when_deferred() {
    let t = tempfile::tempdir().unwrap();
    let cfg = fixture(&t.path().join("fixture"));
    let manifest =
        Manifest::parse(&std::fs::read(t.path().join("fixture/input/release.json")).unwrap())
            .unwrap();
    let mut p = Policy::load(&cfg.state).unwrap();
    p.desired.dlc.remove("SP01");
    assert!(!p.adopt_new_packs(&manifest), "off by default");
    p.preferences.future_packs = true;
    p.desired.deferred.insert("SP01".into());
    assert!(
        !p.adopt_new_packs(&manifest),
        "deferred packs stay unanswered"
    );
    p.desired.deferred.clear();
    let revision = p.revision;
    assert!(p.adopt_new_packs(&manifest));
    assert_eq!(p.desired.dlc.get("SP01"), Some(&true));
    assert_eq!(p.revision, revision + 1);
    assert!(!p.adopt_new_packs(&manifest), "idempotent");
}

#[test]
fn scheduled_run_does_nothing_unless_background_maintenance_is_enabled() {
    let t = tempfile::tempdir().unwrap();
    let cfg = fixture(&t.path().join("fixture"));
    let config = t.path().join("fixture/config.json");
    let run = || {
        Command::new(patcher_bin())
            .args(["run", "--scheduled", "--config"])
            .arg(&config)
            .output()
            .unwrap()
    };
    let out = run();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Background maintenance is off"));
    assert!(!t.path().join("fixture/game/Data/base.package").exists());

    let mut p = Policy::load(&cfg.state).unwrap();
    p.preferences.background = true;
    p.suspended = Some("paused".into());
    p.save(&cfg.state).unwrap();
    let out = run();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("paused or stopped"));
    assert!(!t.path().join("fixture/game/Data/base.package").exists());
}

fn fake_game(dir: &Path) {
    std::fs::create_dir_all(dir.join("Data/Client")).unwrap();
    std::fs::create_dir_all(dir.join("Game/Bin")).unwrap();
    std::fs::write(dir.join("Data/Client/ClientFullBuild0.package"), "x").unwrap();
    std::fs::write(dir.join("Game/Bin/TS4_x64.exe"), "x").unwrap();
}

#[test]
fn windows_probe_finds_ea_app_and_extra_steam_libraries() {
    use patcher_core::discovery::{Probe, discover};
    let t = tempfile::tempdir().unwrap();
    let pf = t.path().join("Program Files");
    let pf86 = t.path().join("Program Files (x86)");
    fake_game(&pf.join("EA Games/The Sims 4"));
    let library = t.path().join("D/SteamLibrary");
    fake_game(&library.join("steamapps/common/The Sims 4"));
    std::fs::create_dir_all(pf86.join("Steam/steamapps")).unwrap();
    std::fs::write(
        pf86.join("Steam/steamapps/libraryfolders.vdf"),
        // Steam escapes backslashes in Windows paths, as in "D:\\SteamLibrary".
        format!(
            "\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}",
            library.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    let found = discover(&Probe::windows(Some(&pf), Some(&pf86)), None);
    assert_eq!(found.candidates.len(), 2, "{found:?}");
    assert!(
        found
            .candidates
            .iter()
            .all(|c| c.variant == "windows-native")
    );
    assert!(found.candidates.iter().all(|c| !c.maintenance_supported));
    let sources: Vec<_> = found.candidates.iter().map(|c| c.source.as_str()).collect();
    assert!(sources.contains(&"EA app") && sources.contains(&"Steam library"));
    assert!(
        found
            .checked
            .iter()
            .any(|p| p.ends_with("Origin Games/The Sims 4"))
    );
}

fn bundle(dir: &Path, name: &str, plist: &[u8]) -> std::path::PathBuf {
    let bundle = dir.join(name);
    std::fs::create_dir_all(bundle.join("Contents")).unwrap();
    std::fs::write(bundle.join("Contents/Info.plist"), plist).unwrap();
    bundle
}

fn xml_plist(id: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\
         <key>CFBundleName</key>\n<string>Game</string>\n\
         <key>CFBundleIdentifier</key>\n<string>{id}</string>\n</dict>\n</plist>\n"
    )
    .into_bytes()
}

#[test]
fn mac_probe_finds_the_game_bundle_but_does_not_maintain_it() {
    use patcher_core::discovery::{self, Probe, discover};
    let t = tempfile::tempdir().unwrap();
    let applications = t.path().join("Applications");
    let home_applications = t.path().join("home/Applications");
    let game = bundle(
        &applications,
        "The Sims 4.app",
        &xml_plist("com.ea.mac.thesims4"),
    );
    bundle(&applications, "Other.app", &xml_plist("com.example.other"));
    let binary = bundle(&home_applications, "Packed.app", b"bplist00\x00\x01binary");
    std::fs::create_dir_all(applications.join("NotABundle")).unwrap();
    let probe = Probe {
        steam_roots: Vec::new(),
        direct: Vec::new(),
        mac_applications: vec![applications.clone(), home_applications.clone()],
        variant: "macos-native",
    };
    let found = discover(&probe, None);
    assert_eq!(found.candidates.len(), 1, "{found:?}");
    let c = &found.candidates[0];
    assert_eq!(c.path, std::fs::canonicalize(&game).unwrap());
    assert_eq!(c.variant, "macos-native");
    assert!(!c.maintenance_supported);
    assert_eq!(c.unsupported_reason(), Some(discovery::MAC_UNVERIFIED));
    assert!(found.checked.contains(&applications));
    assert!(found.checked.contains(&home_applications));

    let (_, unresolved, _) = discovery::mac_bundles(&probe.mac_applications);
    assert_eq!(unresolved.len(), 1, "{unresolved:?}");
    assert_eq!(unresolved[0].path, binary);
    assert!(unresolved[0].reason.contains("binary"));

    // A saved bundle is validated by its identifier, and anything else is refused.
    let saved = discover(&probe, Some(&game));
    assert_eq!(saved.candidates.len(), 1);
    assert_eq!(saved.candidates[0].source, "saved installation");
    assert!(
        discovery::validate(&applications.join("Other.app"), "picked", "macos-native").is_err()
    );
    assert!(discovery::validate(&binary, "picked", "macos-native").is_err());
    // Mac folders are searched only on a Mac.
    assert!(Probe::unix(t.path(), false).mac_applications.is_empty());
    assert!(
        Probe::unix(t.path(), true)
            .mac_applications
            .contains(&t.path().join("Applications"))
    );
}

#[test]
fn linux_probe_reports_every_location_checked() {
    use patcher_core::discovery::{Probe, discover};
    let t = tempfile::tempdir().unwrap();
    let found = discover(&Probe::unix(t.path(), false), None);
    assert!(found.candidates.is_empty());
    for root in [
        ".local/share/Steam",
        ".var/app/com.valvesoftware.Steam",
        "snap/steam",
    ] {
        assert!(
            found
                .checked
                .iter()
                .any(|p| p.starts_with(t.path().join(root))),
            "{root}"
        );
    }
}

#[test]
fn turning_off_maintenance_never_warns_where_it_cannot_be_turned_on() {
    let t = tempfile::tempdir().unwrap();
    let (engine, config) = (t.path().join("engine"), t.path().join("config.json"));
    if cfg!(target_os = "linux") {
        return; // Linux schedules maintenance; its units are covered by the systemd tests.
    }
    assert!(patcher_core::os::set_schedule(false, &engine, &config).is_ok());
    assert!(patcher_core::os::set_schedule(true, &engine, &config).is_err());
}
