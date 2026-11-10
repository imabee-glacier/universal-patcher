//! Drives the FFI boundary exactly as the Flutter shell does. The service is process-global,
//! so this file holds a single test.
use patcher_core::{
    bridge::{patcher_command, patcher_free, patcher_snapshot},
    fixture,
};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString},
    time::{Duration, Instant},
};

fn take(pointer: *mut std::ffi::c_char) -> Value {
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap()
        .to_owned();
    unsafe { patcher_free(pointer) };
    serde_json::from_str(&text).unwrap()
}

fn snapshot() -> Value {
    take(patcher_snapshot())
}

fn raw(text: &str) -> Value {
    let input = CString::new(text).unwrap();
    take(unsafe { patcher_command(input.as_ptr()) })
}

fn command(value: Value) -> Value {
    raw(&value.to_string())
}

fn wait_for(state: &str) -> Value {
    let started = Instant::now();
    loop {
        let s = snapshot();
        // Every snapshot is coherent: the Ready state and play readiness never disagree.
        assert_eq!(
            s["state"] == "ready",
            s["ready"] == true,
            "incoherent snapshot: {s}"
        );
        if s["state"] == state {
            return s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "never reached {state}: {s}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn ui_commands_follow_the_engine_contract() {
    let t = tempfile::tempdir().unwrap();
    let cfg = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fixture::create(&t.path().join("fixture")))
        .unwrap();
    // SAFETY: set before the service is first touched; no other thread reads the environment yet.
    unsafe { std::env::set_var("PATCHER_CONFIG", t.path().join("fixture/config.json")) };

    // A saved installation is maintained automatically on launch.
    let ready = wait_for("ready");
    assert_eq!(ready["ready"], true);
    assert_eq!(ready["packs"].as_array().unwrap().len(), 2);
    assert!(
        ready["details"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "Replaced EP01/content.package")
    );
    // Play is decided here, not in the interface: a test installation has no game to start.
    assert_eq!(ready["play"], "unavailable");
    assert_eq!(
        ready["play_reason"],
        "Game launch is unavailable for a test installation."
    );
    assert_eq!(ready["labels"]["Verify"], "Verify files");
    assert_eq!(ready["status"], "Ready");
    assert!(
        ready["collection"]
            .as_str()
            .unwrap()
            .contains("packs selected")
    );
    let revision = ready["policy_revision"].as_u64().unwrap();

    let stale = command(json!({"id":"1","command":"SetPolicy","revision":revision - 1,"dlc":{}}));
    assert_eq!(stale["accepted"], false);

    let missing_id = command(json!({"command":"Verify"}));
    assert_eq!(missing_id["accepted"], false);

    let duplicate = raw(r#"{"id":"2","command":"Verify","command":"Start"}"#);
    assert_eq!(duplicate["accepted"], false, "duplicate keys are rejected");

    let changed = command(json!({
        "id": "3",
        "command": "SetPolicy",
        "revision": revision,
        "dlc": {"EP01": true, "SP01": true}
    }));
    assert_eq!(changed["accepted"], true, "{changed}");
    let planned = wait_for("planned");
    assert_eq!(
        planned["ready"], false,
        "a changed selection is not Ready until applied"
    );
    assert_eq!(planned["policy_revision"].as_u64(), Some(revision + 1));

    assert_eq!(
        command(json!({"id":"4","command":"Start"}))["accepted"],
        true
    );
    wait_for("ready");
    let game = t.path().join("fixture/game");
    assert!(game.join("SP01/content.package").exists());

    assert_eq!(
        command(json!({"id":"5","command":"Verify"}))["accepted"],
        true
    );
    let verified = wait_for("ready");
    assert!(
        verified["details"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "No files needed changes.")
    );

    let revision = verified["policy_revision"].as_u64().unwrap();
    for (case, body) in [
        ("unknown pack", json!({"dlc": {"EP99": true}})),
        ("unknown deferred pack", json!({"deferred": ["EP99"]})),
        ("unknown language", json!({"language": "xx_XX"})),
        (
            "unknown preference field",
            json!({"preferences": {"turbo": true}}),
        ),
    ] {
        let mut request = json!({"id": "8", "command": "SetPolicy", "revision": revision});
        request
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        let r = command(request);
        assert_eq!(r["accepted"], false, "{case}: {r}");
    }
    assert_eq!(
        snapshot()["policy_revision"].as_u64(),
        Some(revision),
        "rejected changes are not saved"
    );

    let deferred = command(json!({
        "id": "9",
        "command": "SetPolicy",
        "revision": revision,
        "dlc": {"EP01": true, "SP01": true},
        "deferred": ["SP01"],
        "preferences": {
            "future_packs": false,
            "background": false,
            "metered": true,
            "bandwidth_bytes_per_second": 1048576
        }
    }));
    assert_eq!(deferred["accepted"], true, "{deferred}");
    let s = snapshot();
    let sp01 = s["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "SP01")
        .unwrap()
        .clone();
    assert_eq!(sp01["deferred"], true);
    assert_eq!(sp01["answered"], false, "a deferred pack is unanswered");
    assert_eq!(s["preferences"]["bandwidth_bytes_per_second"], 1048576);
    assert_eq!(s["preferences"]["metered"], true);

    let revision = s["policy_revision"].as_u64().unwrap();
    let preferences_only = command(json!({
        "id": "10",
        "command": "SetPolicy",
        "revision": revision,
        "preferences": {
            "future_packs": true,
            "background": false,
            "metered": false,
            "bandwidth_bytes_per_second": 0
        }
    }));
    assert_eq!(preferences_only["accepted"], true, "{preferences_only}");
    assert_eq!(snapshot()["preferences"]["future_packs"], true);

    for unsupported in ["Launch", "Connect", "Disconnect"] {
        let r = command(json!({"id":"6","command":unsupported}));
        assert_eq!(r["accepted"], false);
        assert!(r["error"].as_str().unwrap().contains("unavailable"), "{r}");
    }
    let pause = command(json!({"id":"7","command":"Pause"}));
    assert_eq!(pause["accepted"], false, "nothing to pause while idle");
    drop(cfg);
}
