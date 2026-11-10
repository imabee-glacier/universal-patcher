//! The user's own installation read in place: pack status, languages, and how it is started.
use patcher_core::local::{self, Kind, Launch, Runners, Status};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACK_FILES: [&str; 6] = [
    "ClientFullBuild0.package",
    "SimulationFullBuild0.package",
    "ResourceClient.cfg",
    "ResourceSimulation.cfg",
    "Version.ini",
    "thumbnails.package",
];
const DELTA_FILES: [&str; 7] = [
    "ClientDeltaBuild0.package",
    "ClipHeader.package",
    "ResourceClient.cfg",
    "ResourceSimulation.cfg",
    "SimulationDeltaBuild0.package",
    "SimulationPreload.package",
    "Version.ini",
];

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn pack(root: &Path, id: &str) {
    for f in PACK_FILES {
        write(root, &format!("{id}/{f}"), &format!("{id} {f}"));
    }
    for f in DELTA_FILES {
        write(root, &format!("Delta/{id}/{f}"), &format!("delta {id} {f}"));
    }
}

fn game(root: &Path) {
    write(root, "Data/Client/ClientFullBuild0.package", "base");
    write(root, "Data/Client/Strings_ENG_US.package", "en");
    write(root, "Data/Client/Strings_POR_BR.package", "pt");
    write(root, "Data/Client/Strings_bad.package", "ignored");
    write(root, "Game/Bin/TS4_x64.exe", "exe");
    write(root, "__Installer/Touchup.exe", "out of scope");
    pack(root, "EP01");
    pack(root, "SP20");
    pack(root, "SP99");
}

#[test]
fn packs_are_installed_incomplete_or_missing() {
    let t = tempfile::tempdir().unwrap();
    game(t.path());
    // An incomplete pack: one core file empty, one Delta file gone.
    pack(t.path(), "GP05");
    fs::write(t.path().join("GP05/ClientFullBuild0.package"), "").unwrap();
    fs::remove_file(t.path().join("Delta/GP05/ClipHeader.package")).unwrap();

    let packs = local::scan(t.path());
    let get = |id: &str| packs.iter().find(|p| p.id == id).unwrap();
    assert_eq!(get("EP01").status, Status::Installed);
    assert_eq!(get("EP01").name, "Get to Work");
    assert_eq!(get("EP01").kind, Kind::Expansion);
    assert!(get("EP01").bytes > 0);
    assert_eq!(get("SP20").kind, Kind::Kit);
    assert_eq!(get("GP05").status, Status::Incomplete);
    assert_eq!(
        get("GP05").missing,
        [
            "GP05/ClientFullBuild0.package",
            "Delta/GP05/ClipHeader.package"
        ]
    );
    assert_eq!(get("EP02").status, Status::NotInstalled);
    assert_eq!(get("EP02").bytes, 0);
    // A pack folder the catalog does not name is still listed, under its id.
    assert_eq!(get("SP99").name, "SP99");
    assert_eq!(get("SP99").status, Status::Installed);
    assert!(packs.windows(2).all(|w| w[0].id < w[1].id), "sorted by id");
    // Every pack of a full installation is listed (106 ids), plus SP99 found on disk.
    assert_eq!(packs.len(), 107);
    assert_eq!(get("SP81").status, Status::NotInstalled);
    assert_eq!(get("SP81").kind, Kind::Kit);
}

#[test]
fn installed_languages_come_from_the_string_files() {
    let t = tempfile::tempdir().unwrap();
    game(t.path());
    assert_eq!(local::languages(t.path()), ["ENG_US", "POR_BR"]);
}

/// The game starts wherever it came from: directly on Windows, through Steam for a Steam
/// library that needs its compatibility layer, through Wine for any other folder, and as an
/// application on a Mac.
#[test]
fn the_game_is_started_whatever_installed_it() {
    let t = tempfile::tempdir().unwrap();
    let steam = t.path().join("Steam/steamapps/common/The Sims 4");
    let plain = t.path().join("Games/The Sims 4");
    let prefix = t.path().join("prefix");
    let in_prefix = prefix.join("drive_c/Program Files/EA Games/The Sims 4");
    for root in [&steam, &plain, &in_prefix] {
        write(root, "Game/Bin/TS4_x64.exe", "exe");
    }
    let exe = |root: &Path| root.join("Game").join("Bin").join("TS4_x64.exe");
    let wine = PathBuf::from("/usr/bin/wine");
    let none = Runners::default();
    let both = Runners {
        steam_links: true,
        wine: Some(wine.clone()),
    };
    let wine_only = Runners {
        steam_links: false,
        wine: Some(wine.clone()),
    };

    // Windows: the program itself, for every source.
    for root in [&steam, &plain, &in_prefix] {
        let Launch::Program(run) =
            local::how_to_launch_on(root, "folder picker", true, false, &none)
        else {
            panic!("Windows starts the program")
        };
        assert_eq!(run.program, exe(root));
        assert!(run.args.is_empty() && run.env.is_empty());
        assert_eq!(run.dir.as_deref(), exe(root).parent());
    }

    // Linux: Steam for its own library, Wine for everything else and when Steam is absent.
    assert_eq!(
        local::how_to_launch_on(&steam, "Steam library", false, false, &both),
        Launch::Steam
    );
    for (root, runners, expected_prefix) in [
        (&steam, &wine_only, None),
        (&plain, &both, None),
        (&in_prefix, &both, Some(prefix.clone())),
    ] {
        let Launch::Program(run) =
            local::how_to_launch_on(root, "folder picker", false, false, runners)
        else {
            panic!("Wine starts {}", root.display())
        };
        assert_eq!(run.program, wine);
        assert_eq!(run.args, [exe(root).into_os_string()]);
        let env: Vec<(String, PathBuf)> = run
            .env
            .into_iter()
            .map(|(k, v)| (k, PathBuf::from(v)))
            .collect();
        assert_eq!(
            env,
            expected_prefix
                .map(|p| vec![("WINEPREFIX".to_string(), p)])
                .unwrap_or_default()
        );
    }
    assert!(matches!(
        local::how_to_launch_on(&plain, "folder picker", false, false, &none),
        Launch::Unsupported(reason) if reason.contains("Wine is not installed")
    ));

    // A Mac application is opened as one.
    let bundle = t.path().join("Applications/The Sims 4.app");
    fs::create_dir_all(&bundle).unwrap();
    let Launch::Program(run) =
        local::how_to_launch_on(&bundle, "Mac application", false, true, &none)
    else {
        panic!("a Mac application is opened")
    };
    assert_eq!(run.program, PathBuf::from("/usr/bin/open"));
    assert_eq!(run.args, [bundle.clone().into_os_string()]);

    fs::remove_file(exe(&plain)).unwrap();
    assert!(matches!(
        local::how_to_launch_on(&plain, "folder picker", true, false, &none),
        Launch::Unsupported(reason) if reason.contains("missing")
    ));
}

/// A tree shaped like a full installation: every pack folder with its Delta folder, a base
/// game with thousands of files, and a few large sparse files.
#[test]
fn a_full_size_layout_is_scanned() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let ids: Vec<String> = local::scan(root).into_iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), 106);
    for id in &ids {
        pack(root, id);
    }
    for i in 0..2000 {
        write(
            root,
            &format!("Data/Simulation/Gameplay/file{i:04}.package"),
            &format!("{i}"),
        );
    }
    write(root, "Game/Bin/TS4_x64.exe", "exe");
    let f = fs::File::create(root.join("EP01/Big.package")).unwrap();
    f.set_len(48 << 20).unwrap();

    let started = Instant::now();
    let packs = local::scan(root);
    assert!(packs.iter().all(|p| p.status == Status::Installed));
    assert!(packs.iter().find(|p| p.id == "EP01").unwrap().bytes > 48 << 20);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "scan took {:?}",
        started.elapsed()
    );
}

#[test]
fn pack_ids_have_a_known_prefix_and_two_digits() {
    assert!(local::is_pack_id("EP01"));
    assert!(!local::is_pack_id("EP1"));
    assert!(!local::is_pack_id("XP01"));
}

/// A real process whose program is called TS4_x64.exe, as Proton runs the game, is seen as the
/// game while it runs and not after it exits.
#[cfg(target_os = "linux")]
#[test]
fn a_running_game_process_is_detected() {
    use patcher_core::process::{Detection, detect};
    let t = tempfile::tempdir().unwrap();
    let fake = t.path().join("TS4_x64.exe");
    let sleep = ["/usr/bin/sleep", "/bin/sleep"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .expect("sleep is installed");
    fs::copy(sleep, &fake).unwrap();
    let game = |d: Detection| match d {
        Detection::Seen(a) => a.game,
        Detection::Unsupported(r) => panic!("{r}"),
    };
    assert!(!game(detect()), "no game runs before the test");
    let mut child = std::process::Command::new(&fake).arg("30").spawn().unwrap();
    let started = Instant::now();
    while !game(detect()) {
        assert!(started.elapsed() < Duration::from_secs(5), "never seen");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!game(detect()), "seen after it exited");
}
