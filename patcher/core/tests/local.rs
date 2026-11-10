//! The user's own installation read in place: pack status, languages, and change checks.
use patcher_core::local::{self, Baseline, Kind, Status};
use std::{fs, path::Path, sync::atomic::AtomicBool};

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

#[test]
fn checks_record_a_baseline_then_report_changes_by_pack() {
    let t = tempfile::tempdir().unwrap();
    game(t.path());
    let stop = AtomicBool::new(false);
    let mut seen = 0;
    let first = local::check(t.path(), None, &stop, |p| {
        assert!(p.files_done < p.files_total);
        seen += 1;
    })
    .unwrap();
    assert!(first.first);
    assert!(first.packs.values().all(|p| p.clean()));
    assert_eq!(first.files as usize, seen);
    assert!(
        !first.current.contains_key("__Installer/Touchup.exe"),
        "installers are out of scope"
    );
    let baseline = Baseline {
        schema: 1,
        recorded_at: "now".into(),
        files: first.current.clone(),
    };
    let path = t.path().join("baseline.json");
    baseline.save(&path).unwrap();
    let baseline = Baseline::load(&path).unwrap();

    // Same size, different bytes; a removed file; a new file.
    write(
        t.path(),
        "EP01/Version.ini",
        &"x".repeat("EP01 Version.ini".len()),
    );
    fs::remove_file(t.path().join("Delta/SP20/ClipHeader.package")).unwrap();
    write(t.path(), "Data/Client/Strings_GER_DE.package", "de");

    let second = local::check(t.path(), Some(&baseline), &stop, |_| {}).unwrap();
    assert!(!second.first);
    assert_eq!(second.packs["EP01"].changed, ["EP01/Version.ini"]);
    assert_eq!(
        second.packs["SP20"].missing,
        ["Delta/SP20/ClipHeader.package"]
    );
    assert_eq!(
        second.packs["base"].added,
        ["Data/Client/Strings_GER_DE.package"]
    );
    assert!(second.packs["SP99"].clean());
}

#[test]
fn a_stopped_check_reports_nothing() {
    let t = tempfile::tempdir().unwrap();
    game(t.path());
    let stop = AtomicBool::new(true);
    assert!(local::check(t.path(), None, &stop, |_| {}).is_err());
}

#[test]
fn files_belong_to_their_pack_or_the_base_game() {
    assert_eq!(
        local::owner("EP01/ClientFullBuild0.package").as_deref(),
        Some("EP01")
    );
    assert_eq!(
        local::owner("Delta/GP12/Version.ini").as_deref(),
        Some("GP12")
    );
    assert_eq!(local::owner("Data/Client/Strings_ENG_US.package"), None);
    assert_eq!(local::owner("Delta/Base/x"), None);
    assert!(!local::is_pack_id("EP1"));
    assert!(!local::is_pack_id("XP01"));
}
