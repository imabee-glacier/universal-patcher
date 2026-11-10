//! The app on a real installation, with no release feed: discovery adopts the game, packs show
//! their status, choices are saved, and file checks record and compare fingerprints. Runs in
//! its own process, so the bridge starts fresh.
#![cfg(target_os = "linux")]
use patcher_core::bridge::{patcher_command, patcher_free, patcher_snapshot};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString},
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn take(ptr: *mut std::ffi::c_char) -> Value {
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
    unsafe { patcher_free(ptr) };
    serde_json::from_str(&text).unwrap()
}
fn snapshot() -> Value {
    take(patcher_snapshot())
}
fn command(name: &str, mut args: Value) -> Value {
    args["id"] = json!(name);
    args["command"] = json!(name);
    let input = CString::new(args.to_string()).unwrap();
    take(unsafe { patcher_command(input.as_ptr()) })
}
fn wait(what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = snapshot();
        if done(&s) {
            return s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "never {what}: {s}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
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
fn pack_of<'a>(s: &'a Value, id: &str) -> &'a Value {
    s["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap()
}

#[test]
fn a_real_installation_can_be_reviewed_chosen_and_checked() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let game = home.join(".local/share/Steam/steamapps/common/The Sims 4");
    write(&game, "Data/Client/ClientFullBuild0.package", "base");
    write(&game, "Data/Client/Strings_ENG_US.package", "en");
    write(&game, "Data/Client/Strings_GER_DE.package", "de");
    write(&game, "Game/Bin/TS4_x64.exe", "exe");
    pack(&game, "EP01");
    pack(&game, "GP05");
    fs::remove_file(game.join("GP05/Version.ini")).unwrap();
    // SAFETY: set before the bridge is first touched; no other thread reads the environment yet.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("PATCHER_HOME", t.path().join("data"));
        std::env::remove_var("PATCHER_CONFIG");
    }

    // Found and adopted without asking; packs show what is on disk.
    let s = wait("shows the installation", |s| s["state"] == "installed");
    assert_eq!(s["title"], "Review your packs");
    assert_eq!(s["languages"], json!(["ENG_US", "GER_DE"]));
    assert_eq!(s["background_supported"], false);
    assert_eq!(pack_of(&s, "EP01")["status"], "installed");
    assert_eq!(pack_of(&s, "EP01")["name"], "Get to Work");
    assert_eq!(pack_of(&s, "EP01")["kind"], "expansion");
    assert_eq!(
        pack_of(&s, "EP01")["selected"],
        true,
        "installed packs are proposed"
    );
    assert_eq!(pack_of(&s, "EP01")["answered"], false);
    assert_eq!(pack_of(&s, "GP05")["status"], "incomplete");
    assert_eq!(
        pack_of(&s, "GP05")["issues"],
        json!(["Missing: GP05/Version.ini"])
    );
    assert_eq!(pack_of(&s, "EP02")["status"], "not_installed");
    for action in ["Verify", "Launch", "SetPolicy"] {
        assert!(
            s["actions"].as_array().unwrap().contains(&json!(action)),
            "{action}"
        );
    }

    // Choices are saved; impossible ones are refused with a reason.
    let revision = s["policy_revision"].as_u64().unwrap();
    let refused = command(
        "SetPolicy",
        json!({"revision": revision, "language": "JPN_JP"}),
    );
    assert_eq!(refused["accepted"], false);
    let refused = command(
        "SetPolicy",
        json!({"revision": revision, "preferences":
        {"future_packs": false, "background": true, "metered": false, "bandwidth_bytes_per_second": 0}}),
    );
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("published release"),
        "{refused}"
    );
    let ok = command(
        "SetPolicy",
        json!({"revision": revision, "dlc": {"EP01": true, "GP05": false}, "language": "GER_DE"}),
    );
    assert_eq!(ok["accepted"], true, "{ok}");
    let s = snapshot();
    assert_eq!(pack_of(&s, "EP01")["answered"], true);
    assert_eq!(pack_of(&s, "GP05")["selected"], false);
    assert_eq!(s["language"], "GER_DE");
    assert_eq!(
        s["title"], "Some packs need attention",
        "GP05 is incomplete"
    );
    let stale = command("SetPolicy", json!({"revision": revision, "dlc": {}}));
    assert_eq!(stale["accepted"], false, "an old revision is refused");

    // The first check becomes the reference.
    assert_eq!(command("Verify", json!({}))["accepted"], true);
    let s = wait("finishes the first check", |s| {
        s["state"] == "installed" && !s["checked_at"].is_null()
    });
    assert!(
        s["details"][0]
            .as_str()
            .unwrap()
            .contains("reference for future checks"),
        "{s}"
    );
    assert!(
        !s["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("AcceptCheck"))
    );

    // A changed file is found and attributed to its pack; accepting it clears the change.
    write(
        &game,
        "EP01/ClientFullBuild0.package",
        "EP01 ClientFullBuild0.packagX",
    );
    assert_eq!(command("Verify", json!({}))["accepted"], true);
    let s = wait("reports the change", |s| {
        pack_of(s, "EP01")["status"] == "changed"
    });
    assert_eq!(
        pack_of(&s, "EP01")["issues"],
        json!(["Changed: EP01/ClientFullBuild0.package"])
    );
    assert!(
        s["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("AcceptCheck"))
    );
    assert_eq!(command("AcceptCheck", json!({}))["accepted"], true);
    let s = snapshot();
    assert_eq!(pack_of(&s, "EP01")["status"], "installed");
    assert!(
        !s["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("AcceptCheck"))
    );
    assert!(t.path().join("data/game/baseline.json").is_file());
    assert!(t.path().join("data/game/policy.json").is_file());
    // Nothing was written inside the game folder.
    assert!(!game.join("baseline.json").exists() && !game.join("policy.json").exists());
}
