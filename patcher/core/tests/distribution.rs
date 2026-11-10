//! Publishes patcher builds to a local HTTP origin and drives install, update, launch,
//! rollback and uninstall through the real CLI with an isolated home directory.
#![cfg(target_os = "linux")]
use patcher_core::{app, distribution, model::Object, publisher};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tiny_http::{Response, Server};

/// Serves files under `dir`, like the production download service.
fn serve(dir: PathBuf) -> String {
    let server = Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let path = request.url().trim_start_matches('/').to_owned();
            let file = dir.join(&path);
            let ok = !path.split('/').any(|p| p == ".." || p.starts_with('.')) && file.is_file();
            let response = if ok {
                Response::from_data(fs::read(file).unwrap())
            } else {
                Response::from_data(Vec::new()).with_status_code(404)
            };
            let _ = request.respond(response);
        }
    });
    format!("http://127.0.0.1:{port}/")
}

/// A stand-in build whose interface either reports a healthy start or exits immediately.
fn bundle(dir: &Path, healthy: bool) {
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::create_dir_all(dir.join("data")).unwrap();
    let script = if healthy {
        "#!/bin/sh\nprintf ok > \"$PATCHER_HEALTH_FILE\"\n"
    } else {
        "#!/bin/sh\nexit 3\n"
    };
    for (name, text) in [
        ("patcher", script),
        ("lib/patcher-engine", "#!/bin/sh\nexit 0\n"),
    ] {
        fs::write(dir.join(name), text).unwrap();
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(dir.join("data/icon.svg"), "<svg/>").unwrap();
    // Release builds can leave holes in linked libraries; they must package as ordinary files.
    let mut sparse = fs::File::create(dir.join("lib/libsparse.so")).unwrap();
    std::io::Seek::seek(&mut sparse, std::io::SeekFrom::Start(SPARSE_HOLE)).unwrap();
    std::io::Write::write_all(&mut sparse, b"end").unwrap();
}

const SPARSE_HOLE: u64 = 1 << 20;

/// Sets the mode of every directory under `dir`. Publishing reads staging from a read-only
/// volume in the cluster.
fn set_mode(dir: &Path, mode: u32) {
    fs::set_permissions(dir, fs::Permissions::from_mode(mode)).unwrap();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            set_mode(&path, mode);
        }
    }
}

struct Env {
    t: tempfile::TempDir,
    origin: String,
    root: PathBuf,
    key: PathBuf,
}

impl Env {
    fn new() -> Self {
        let t = tempfile::tempdir().unwrap();
        let root = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(publisher::fixture_root(&t.path().join("keys")))
            .unwrap();
        let key = t.path().join("keys/fixture-key.pem");
        Self::with(t, root, key)
    }
    fn with(t: tempfile::TempDir, root: PathBuf, key: PathBuf) -> Self {
        let origin = serve(t.path().join("repo"));
        Self {
            t,
            origin,
            root,
            key,
        }
    }
    fn home(&self) -> PathBuf {
        self.t.path().join("home")
    }
    fn layout(&self) -> app::Layout {
        app::Layout::at(self.home().join(".local/share/universal-patcher"))
    }
    fn release(&self, version: &str, healthy: bool) {
        let (os, arch) = app::platform().unwrap();
        let build = self.t.path().join(format!("build-{version}"));
        bundle(&build, healthy);
        let staging = self.t.path().join(format!("staging-{version}"));
        distribution::package_app(&build, version, os, arch, &staging).unwrap();
        let listing = distribution::Listing {
            offered: vec![format!("{os}-{arch}")],
            notes: vec![format!("Release {version}.")],
            windows_setup: None,
        };
        set_mode(&staging, 0o555);
        self.publish(&staging, &listing).unwrap();
        set_mode(&staging, 0o755);
    }
    fn publish(
        &self,
        staging: &Path,
        listing: &distribution::Listing,
    ) -> anyhow::Result<(String, u64)> {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(distribution::publish_app(
                &self.root,
                vec![self.key.clone()],
                staging,
                &self.t.path().join("repo"),
                &self.origin,
                listing,
            ))
    }
    fn cli(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_patcher"));
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", "/usr/bin:/bin");
        if matches!(args.first(), Some(&"install" | &"update")) {
            cmd.args(["--origin", &self.origin, "--root"])
                .arg(&self.root);
        }
        cmd.output().unwrap()
    }
    fn state(&self) -> app::InstallState {
        app::InstallState::load(&self.layout()).unwrap()
    }
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn install_update_rollback_and_uninstall() {
    let env = Env::new();
    env.release("1.0.0", true);
    let repo = env.t.path().join("repo");
    for script in ["install.sh", "install.ps1"] {
        let text = fs::read_to_string(repo.join(script)).unwrap();
        assert!(text.contains(&env.origin) && !text.contains("@ORIGIN@"));
    }
    assert_eq!(
        fs::read_to_string(repo.join("app/bootstrap/latest")).unwrap(),
        "1.0.0\n"
    );

    let out = ok(&env.cli(&["install"]));
    let sparse = fs::read(env.layout().version("1.0.0").join("lib/libsparse.so")).unwrap();
    assert_eq!(sparse.len() as u64, SPARSE_HOLE + 3);
    assert!(sparse[..SPARSE_HOLE as usize].iter().all(|b| *b == 0) && sparse.ends_with(b"end"));
    assert!(
        out.contains("Universal Patcher 1.0.0 is installed"),
        "{out}"
    );
    let layout = env.layout();
    assert!(layout.version("1.0.0").join("patcher").is_file());
    let mode = fs::metadata(layout.version("1.0.0").join("patcher"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "executables stay executable");
    assert!(layout.launcher().is_file());
    let link = env.home().join(".local/bin/universal-patcher");
    assert_eq!(fs::read_link(&link).unwrap(), layout.launcher());
    let desktop = fs::read_to_string(
        env.home()
            .join(".local/share/applications/universal-patcher.desktop"),
    )
    .unwrap();
    assert!(desktop.contains(&format!("Exec=\"{}\" launch", layout.launcher().display())));
    assert!(desktop.contains("Icon="));
    assert_eq!(env.state().current.as_deref(), Some("1.0.0"));

    ok(&env.cli(&["launch"]));
    assert!(layout.health("1.0.0").is_file());

    // A broken release is activated, fails its startup check, and is never tried again.
    env.release("2.0.0", false);
    assert!(ok(&env.cli(&["update"])).contains("2.0.0 is ready"));
    assert_eq!(env.state().staged.as_deref(), Some("2.0.0"));
    let out = env.cli(&["launch"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("2.0.0 did not start correctly"));
    let state = env.state();
    assert_eq!(state.current.as_deref(), Some("1.0.0"));
    assert_eq!(state.rejected, vec!["2.0.0".to_string()]);
    assert!(ok(&env.cli(&["update"])).contains("up to date"));

    // A good release activates, and releases other than current and previous are removed.
    env.release("3.0.0", true);
    ok(&env.cli(&["update"]));
    ok(&env.cli(&["launch"]));
    let state = env.state();
    assert_eq!(state.current.as_deref(), Some("3.0.0"));
    assert_eq!(state.previous.as_deref(), Some("1.0.0"));
    assert_eq!(state.trial, None);
    assert!(!layout.version("2.0.0").exists());

    // Older releases cannot be published over newer ones.
    let (os, arch) = app::platform().unwrap();
    let old = env.t.path().join("staging-old");
    bundle(&env.t.path().join("build-old"), true);
    distribution::package_app(&env.t.path().join("build-old"), "2.5.0", os, arch, &old).unwrap();
    assert!(
        env.publish(&old, &distribution::Listing::default())
            .is_err()
    );

    // Renewal re-signs every platform manifest of the app repository without changing bytes.
    let before = env.state();
    let renewed = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(publisher::renew(
            &env.root,
            vec![env.key.clone()],
            &repo.join("app"),
        ))
        .unwrap();
    assert!(renewed > 1);
    assert!(ok(&env.cli(&["update"])).contains("up to date"));
    assert_eq!(env.state(), before);

    // Tampered bytes on the server are refused before anything is unpacked.
    env.release("4.0.0", true);
    for entry in fs::read_dir(repo.join("app/objects")).unwrap() {
        let path = entry.unwrap().path();
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
    }
    assert!(!env.cli(&["update"]).status.success());
    assert!(!layout.version("4.0.0").exists());

    fs::write(layout.config(), "{}").unwrap();
    ok(&env.cli(&["uninstall"]));
    assert!(!layout.launcher().exists());
    assert!(!layout.root.join("versions").exists());
    assert!(fs::symlink_metadata(&link).is_err());
    assert!(
        layout.config().is_file(),
        "choices are kept without --remove-data"
    );
    ok(&env.cli(&["uninstall", "--remove-data"]));
    assert!(!layout.root.exists());
}

fn raw_tar(entries: &[(&str, tar::EntryType, Option<&str>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, kind, link) in entries {
        let mut header = tar::Header::new_old();
        let bytes = name.as_bytes();
        header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
        header.set_entry_type(*kind);
        header.set_mode(if *kind == tar::EntryType::Directory {
            0o755
        } else {
            0o644
        });
        header.set_size(0);
        if let Some(link) = link {
            let l = link.as_bytes();
            header.as_old_mut().linkname[..l.len()].copy_from_slice(l);
        }
        header.set_cksum();
        out.extend_from_slice(header.as_bytes());
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

fn raw_object(dir: &Path, bytes: &[u8]) -> (PathBuf, Object) {
    let hash = blake3::hash(bytes).to_hex().to_string();
    let path = dir.join(&hash);
    fs::write(&path, bytes).unwrap();
    let o = Object {
        hash: hash.clone(),
        encoded_size: bytes.len() as u64,
        codec: patcher_core::model::Codec::Raw,
        decoded_hash: hash,
        decoded_size: bytes.len() as u64,
    };
    (path, o)
}

#[test]
fn archives_cannot_escape_the_release_directory() {
    let t = tempfile::tempdir().unwrap();
    let cases = [
        (
            "parent",
            raw_tar(&[("../evil", tar::EntryType::Regular, None)]),
        ),
        (
            "absolute",
            raw_tar(&[("/tmp/evil", tar::EntryType::Regular, None)]),
        ),
        (
            "link out",
            raw_tar(&[("lib/x", tar::EntryType::Symlink, Some("../../outside"))]),
        ),
        (
            "absolute link",
            raw_tar(&[("x", tar::EntryType::Symlink, Some("/etc/passwd"))]),
        ),
        (
            "hard link",
            raw_tar(&[("x", tar::EntryType::Link, Some("y"))]),
        ),
        ("device", raw_tar(&[("x", tar::EntryType::Char, None)])),
    ];
    for (case, bytes) in cases {
        let (path, o) = raw_object(t.path(), &bytes);
        let dest = t.path().join(format!("dest-{case}"));
        assert!(app::unpack(&path, &o, &dest).is_err(), "{case}");
    }
    assert!(!t.path().parent().unwrap().join("evil").exists());

    let good = raw_tar(&[
        ("lib", tar::EntryType::Directory, None),
        ("lib/a", tar::EntryType::Regular, None),
        ("lib/link", tar::EntryType::Symlink, Some("a")),
    ]);
    let (path, o) = raw_object(t.path(), &good);
    app::unpack(&path, &o, &t.path().join("ok")).unwrap();
    let (path, mut o) = raw_object(t.path(), &good);
    o.decoded_hash = blake3::hash(b"other").to_hex().to_string();
    assert!(app::unpack(&path, &o, &t.path().join("mismatch")).is_err());
}

#[test]
fn desktop_entry_refuses_unquotable_paths() {
    assert!(app::desktop_entry(Path::new("/home/a b/bin/up"), None).is_ok());
    for bad in [
        "/home/a\"b/up",
        "/home/$x/up",
        "/home/a`b`/up",
        "/home/a%b/up",
    ] {
        assert!(app::desktop_entry(Path::new(bad), None).is_err(), "{bad}");
    }
}

#[test]
fn generated_scripts_only_accept_plain_https_origins() {
    assert!(distribution::render_scripts("https://downloads.example.org/").is_ok());
    for bad in [
        "http://example.org/",
        "https://example.org",
        "https://example.org/'; rm -rf ~; '/",
        "https://example.org/$(id)/",
    ] {
        assert!(distribution::render_scripts(bad).is_err(), "{bad}");
    }
}

fn catalog(repo: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(repo.join("site/data/catalog.json")).unwrap()).unwrap()
}

#[test]
fn website_catalog_follows_published_releases() {
    let env = Env::new();
    let repo = env.t.path().join("repo");
    let (os, arch) = app::platform().unwrap();
    let web_arch = if arch == "x64" { "x86_64" } else { "aarch64" };

    env.release("1.0.0", true);
    let first = catalog(&repo);
    assert_eq!(first["schema"], 1);
    assert_eq!(first["downloads"].as_array().unwrap().len(), 1);
    let download = &first["downloads"][0];
    assert_eq!(download["os"], os);
    assert_eq!(download["arch"], web_arch);
    assert_eq!(download["offered"], true);
    assert!(download.get("verified").is_none());
    assert_eq!(download["version"], "1.0.0");
    assert_eq!(
        download["install_script"],
        format!("{}install.sh", env.origin)
    );
    assert_eq!(first["releases"][0]["notes"][0], "Release 1.0.0.");
    assert!(
        first["published_at"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .is_ok()
    );

    // Notes accumulate; downloads describe only the newest release.
    env.release("1.1.0", true);
    let second = catalog(&repo);
    let versions: Vec<_> = second["releases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["version"].as_str().unwrap())
        .collect();
    assert_eq!(versions, ["1.0.0", "1.1.0"]);
    assert_eq!(second["downloads"][0]["version"], "1.1.0");

    // A platform that is packaged but not offered is listed as not offered.
    let build = env.t.path().join("build-1.2.0");
    bundle(&build, true);
    let staging = env.t.path().join("staging-1.2.0");
    distribution::package_app(&build, "1.2.0", os, arch, &staging).unwrap();
    env.publish(&staging, &distribution::Listing::default())
        .unwrap();
    assert_eq!(catalog(&repo)["downloads"][0]["offered"], false);

    // Refusals happen before anything is published.
    let build = env.t.path().join("build-1.3.0");
    bundle(&build, true);
    let staging = env.t.path().join("staging-1.3.0");
    distribution::package_app(&build, "1.3.0", os, arch, &staging).unwrap();
    let before = fs::read(repo.join("site/data/catalog.json")).unwrap();
    let unpackaged = distribution::Listing {
        offered: vec!["windows-arm64".into()],
        ..Default::default()
    };
    let err = env.publish(&staging, &unpackaged).unwrap_err().to_string();
    assert!(err.contains("no build for it is packaged"), "{err}");
    let setup = env.t.path().join("setup.exe");
    fs::write(&setup, b"installer").unwrap();
    let no_windows = distribution::Listing {
        windows_setup: Some(setup),
        ..Default::default()
    };
    let err = env.publish(&staging, &no_windows).unwrap_err().to_string();
    assert!(err.contains("needs a packaged Windows build"), "{err}");
    let blank = distribution::Listing {
        notes: vec!["  ".into()],
        ..Default::default()
    };
    assert!(env.publish(&staging, &blank).is_err());
    assert_eq!(
        fs::read_to_string(repo.join("app/bootstrap/latest")).unwrap(),
        "1.2.0\n"
    );
    assert_eq!(
        fs::read(repo.join("site/data/catalog.json")).unwrap(),
        before
    );
    assert!(!repo.join("app/bootstrap/1.3.0").exists());
}

#[test]
fn health_reports_what_the_public_origin_serves() {
    use patcher_core::health;
    let env = Env::new();
    let repo = env.t.path().join("repo");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let now = jiff::Timestamp::now();
    let names = |checks: &[health::Check]| {
        checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.name)
            .collect::<Vec<_>>()
    };

    // Nothing published and no front: unavailable.
    fs::create_dir_all(&repo).unwrap();
    let checks = rt.block_on(health::probe(&env.origin, now)).unwrap();
    assert_eq!(health::status(&checks), "unavailable");

    env.release("1.0.0", true);
    fs::write(repo.join("healthz"), "ok").unwrap();
    let checks = rt.block_on(health::probe(&env.origin, now)).unwrap();
    assert_eq!(names(&checks), Vec::<&str>::new());
    assert_eq!(health::status(&checks), "operational");

    // Metadata close to expiry is a disruption, before clients start refusing it.
    let soon = now + jiff::SignedDuration::from_hours(24 * 400);
    let checks = rt.block_on(health::probe(&env.origin, soon)).unwrap();
    assert_eq!(names(&checks), ["patcher-metadata"]);
    assert_eq!(health::status(&checks), "degraded");

    fs::remove_file(repo.join("install.ps1")).unwrap();
    let checks = rt.block_on(health::probe(&env.origin, now)).unwrap();
    assert_eq!(names(&checks), ["installers"]);

    let output = env.t.path().join("site/data/health.json");
    assert_eq!(
        rt.block_on(health::write(&env.origin, &output)).unwrap(),
        "degraded"
    );
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["schema"], 1);
    assert_eq!(report["status"], "degraded");
    let checked: jiff::Timestamp = report["checked_at"].as_str().unwrap().parse().unwrap();
    assert!((jiff::Timestamp::now() - checked).get_seconds().abs() < 60);
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o644
    );

    assert!(
        rt.block_on(health::probe("http://example.org/", now))
            .is_err(),
        "plain HTTP off loopback is refused"
    );
}

#[test]
fn production_root_keeps_the_root_key_out_of_publishing() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("signing");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let root = rt.block_on(publisher::init_root(&dir, 365)).unwrap();
    let mode = |p: &str| fs::metadata(dir.join(p)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode("root-key.pem"), 0o600);
    assert_eq!(mode("online-key.pem"), 0o600);
    assert_eq!(mode("root.json"), 0o644);

    let signed: serde_json::Value = serde_json::from_slice(&fs::read(&root).unwrap()).unwrap();
    let roles = &signed["signed"]["roles"];
    assert_ne!(roles["root"]["keyids"], roles["targets"]["keyids"]);
    for role in ["snapshot", "timestamp"] {
        assert_eq!(roles[role]["keyids"], roles["targets"]["keyids"]);
    }

    // Nothing in the directory is ever overwritten.
    let before = fs::read(dir.join("root-key.pem")).unwrap();
    assert!(rt.block_on(publisher::init_root(&dir, 365)).is_err());
    assert_eq!(fs::read(dir.join("root-key.pem")).unwrap(), before);

    // The root key alone cannot publish.
    let env = Env::with(t, root, dir.join("root-key.pem"));
    let (os, arch) = app::platform().unwrap();
    let build = env.t.path().join("build-1.0.0");
    bundle(&build, true);
    let staging = env.t.path().join("staging-1.0.0");
    distribution::package_app(&build, "1.0.0", os, arch, &staging).unwrap();
    assert!(
        env.publish(&staging, &distribution::Listing::default())
            .is_err()
    );
    assert!(!env.t.path().join("repo/app/bootstrap/latest").exists());

    // The online key publishes, and the result installs against the root.
    let env = Env {
        key: dir.join("online-key.pem"),
        ..env
    };
    env.release("1.0.0", true);
    ok(&env.cli(&["install"]));
    assert_eq!(env.state().current.as_deref(), Some("1.0.0"));
}

#[test]
fn only_platforms_the_game_runs_on_are_packaged_or_accepted() {
    use patcher_core::model::{AppManifest, game_runs_on};
    for (os, arch) in [
        ("windows", "x64"),
        ("linux", "x64"),
        ("macos", "x64"),
        ("macos", "arm64"),
    ] {
        assert!(game_runs_on(os, arch), "{os}-{arch}");
    }
    for (os, arch) in [
        ("linux", "arm64"),
        ("windows", "arm64"),
        ("linux", "x86"),
        ("freebsd", "x64"),
    ] {
        assert!(!game_runs_on(os, arch), "{os}-{arch}");
    }

    let t = tempfile::tempdir().unwrap();
    let build = t.path().join("build");
    bundle(&build, true);
    let err =
        distribution::package_app(&build, "1.0.0", "linux", "arm64", &t.path().join("staging"))
            .unwrap_err()
            .to_string();
    assert!(err.contains("does not run on linux-arm64"), "{err}");
    assert!(
        !t.path().join("staging").exists(),
        "nothing is written for a refused platform"
    );

    // A signed manifest for such a platform is refused as well.
    distribution::package_app(&build, "1.0.0", "linux", "x64", &t.path().join("ok")).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(t.path().join("ok/app-linux-x64.json")).unwrap()).unwrap();
    manifest["arch"] = "arm64".into();
    let err = AppManifest::parse(&serde_json::to_vec(&manifest).unwrap())
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not run on linux-arm64"), "{err}");
}

#[test]
fn the_confirmed_installation_is_saved_and_survives_uninstall() {
    let t = tempfile::tempdir().unwrap();
    let layout = app::Layout::at(t.path().join("data"));
    assert_eq!(app::saved_installation(&layout), None);
    let game = t.path().join("The Sims 4");
    app::save_installation(&layout, &game).unwrap();
    assert_eq!(app::saved_installation(&layout), Some(game.clone()));

    // A damaged or foreign record is ignored rather than trusted.
    fs::write(
        layout.installation(),
        b"{\"schema\":1,\"path\":\"x\",\"path\":\"y\"}",
    )
    .unwrap();
    assert_eq!(app::saved_installation(&layout), None);
    fs::write(layout.installation(), b"{\"schema\":2,\"path\":\"x\"}").unwrap();
    assert_eq!(app::saved_installation(&layout), None);

    // Uninstall keeps game choices unless data is removed.
    app::save_installation(&layout, &game).unwrap();
    fs::create_dir_all(layout.root.join("bin")).unwrap();
    app::uninstall(&layout, false).unwrap();
    assert_eq!(app::saved_installation(&layout), Some(game));
    app::uninstall(&layout, true).unwrap();
    assert!(!layout.root.exists());
}
