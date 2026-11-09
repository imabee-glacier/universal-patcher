use patcher_core::{
    fixture,
    model::Manifest,
    os::{self, Network},
    storage::Policy,
};
use std::{path::Path, process::Command};

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
        Command::new(env!("CARGO_BIN_EXE_patcher"))
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
        format!(
            "\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}",
            library.display()
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
