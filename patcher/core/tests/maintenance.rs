use patcher_core::{
    engine::{self, Boundary, RunOptions, RunReport},
    fixture,
    model::{Deletion, Manifest},
    publisher,
    storage::{self, Binding, Event, Journal, Policy},
    transfer::{Control, LocalSource},
    trust,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}
fn setup(path: &Path) -> fixture::Config {
    rt().block_on(fixture::create(path)).unwrap()
}
fn load(cfg: &fixture::Config) -> anyhow::Result<trust::VerifiedRelease> {
    rt().block_on(trust::load(
        &cfg.trusted_root,
        cfg.metadata_url.parse()?,
        cfg.targets_url.parse()?,
        &cfg.target,
        &cfg.state.join("tuf"),
    ))
}
fn run_with(
    cfg: &fixture::Config,
    control: &Control,
    opts: RunOptions,
) -> anyhow::Result<RunReport> {
    let r = load(cfg)?;
    let mut p = Policy::load(&cfg.state)?;
    engine::run(
        &r,
        &mut p,
        &cfg.state,
        &LocalSource {
            directory: cfg.local_objects.clone().unwrap(),
        },
        control,
        &|_| {},
        &opts,
    )
}
fn apply(cfg: &fixture::Config, opts: RunOptions) -> anyhow::Result<RunReport> {
    run_with(cfg, &Control::default(), opts)
}
fn game(cfg: &fixture::Config) -> PathBuf {
    Policy::load(&cfg.state).unwrap().install.roots["game"].clone()
}
fn fixture_dir(cfg: &fixture::Config) -> PathBuf {
    cfg.state.parent().unwrap().to_path_buf()
}
/// Publishes the next signed release after `change` edits the current manifest.
fn publish_next(cfg: &fixture::Config, change: impl FnOnce(&mut Manifest)) -> anyhow::Result<u64> {
    let dir = fixture_dir(cfg);
    let path = dir.join("input/release.json");
    let mut m = Manifest::parse(&fs::read(&path)?)?;
    change(&mut m);
    fs::write(&path, serde_json::to_vec_pretty(&m)?)?;
    rt().block_on(publisher::publish(
        &cfg.trusted_root,
        vec![dir.join("keys/fixture-key.pem")],
        &path,
        &dir.join("repository"),
    ))
}
fn assert_converged(cfg: &fixture::Config) {
    let report = apply(cfg, Default::default()).unwrap();
    assert!(
        report.replaced.is_empty(),
        "rerun replaced {:?}",
        report.replaced
    );
    let release = load(cfg).unwrap();
    let p = Policy::load(&cfg.state).unwrap();
    for f in release.manifest().selected(&p.desired).unwrap() {
        let actual = storage::hash_file(&game(cfg).join(&f.path)).unwrap();
        assert_eq!(actual, (f.target_hash.clone(), f.target_size), "{}", f.path);
    }
    assert_eq!(p.applied.unwrap().revision, p.revision);
}

#[test]
fn selected_content_repair_and_noop() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let first = apply(&cfg, Default::default()).unwrap();
    assert_eq!(first.replaced.len(), 2);
    let game = game(&cfg);
    assert!(game.join("EP01/content.package").exists());
    assert!(!game.join("SP01/content.package").exists());
    fs::write(game.join("unmanaged.txt"), "leave me").unwrap();
    let target = game.join("EP01/content.package");
    let expected = storage::hash_file(&target).unwrap();
    fs::write(&target, vec![0; expected.1 as usize]).unwrap();
    let repair = apply(&cfg, Default::default()).unwrap();
    assert_eq!(repair.replaced, vec!["EP01/content.package".to_string()]);
    assert_eq!(storage::hash_file(&target).unwrap(), expected);
    assert_eq!(
        fs::read_to_string(game.join("unmanaged.txt")).unwrap(),
        "leave me"
    );
    let before = fs::metadata(&target).unwrap().modified().unwrap();
    assert!(apply(&cfg, Default::default()).unwrap().replaced.is_empty());
    assert_eq!(before, fs::metadata(&target).unwrap().modified().unwrap());
}

#[test]
fn interruption_at_every_boundary_converges() {
    for boundary in [
        Boundary::AfterIntent,
        Boundary::AfterRename,
        Boundary::AfterCommit,
    ] {
        for n in 1..=2 {
            let t = tempfile::tempdir().unwrap();
            let cfg = setup(&t.path().join("fixture"));
            let opts = RunOptions {
                interrupt: Some((boundary, n)),
                ..Default::default()
            };
            assert!(apply(&cfg, opts).is_err(), "{boundary:?} {n}");
            assert!(Policy::load(&cfg.state).unwrap().applied.is_none());
            // Recovery is repeated: a second interruption at the same point must also converge.
            let _ = apply(
                &cfg,
                RunOptions {
                    interrupt: Some((boundary, 1)),
                    ..Default::default()
                },
            );
            apply(&cfg, Default::default()).unwrap();
            assert_converged(&cfg);
        }
    }
}

#[test]
fn pause_stops_at_boundary_and_resumes() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let control = Control::default();
    let r = load(&cfg).unwrap();
    let mut p = Policy::load(&cfg.state).unwrap();
    let stop = control.clone();
    let result = engine::run(
        &r,
        &mut p,
        &cfg.state,
        &LocalSource {
            directory: cfg.local_objects.clone().unwrap(),
        },
        &control,
        &move |progress| {
            if progress.phase == "applying" {
                stop.stop();
            }
        },
        &Default::default(),
    );
    assert!(result.is_err());
    assert!(Policy::load(&cfg.state).unwrap().applied.is_none());
    apply(&cfg, Default::default()).unwrap();
    assert_converged(&cfg);
}

#[test]
fn selection_change_after_interruption_supersedes_old_run() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let opts = RunOptions {
        interrupt: Some((Boundary::AfterRename, 1)),
        ..Default::default()
    };
    assert!(apply(&cfg, opts).is_err());
    let mut p = Policy::load(&cfg.state).unwrap();
    p.desired.dlc.insert("SP01".into(), true);
    p.revision += 1;
    p.save(&cfg.state).unwrap();
    // Leave the journal in place before the next run to confirm a Supersede record is written.
    let r = load(&cfg).unwrap();
    let mut policy = Policy::load(&cfg.state).unwrap();
    let mut seen = Vec::new();
    let opts = RunOptions {
        interrupt: Some((Boundary::AfterCommit, 1)),
        ..Default::default()
    };
    let _ = engine::run(
        &r,
        &mut policy,
        &cfg.state,
        &LocalSource {
            directory: cfg.local_objects.clone().unwrap(),
        },
        &Control::default(),
        &|_| {},
        &opts,
    );
    for e in Journal::open(&cfg.state.join("journal.jsonl"))
        .unwrap()
        .events
    {
        seen.push(e);
    }
    assert!(seen.iter().any(|e| matches!(e, Event::Supersede { .. })));
    apply(&cfg, Default::default()).unwrap();
    assert!(game(&cfg).join("SP01/content.package").exists());
    assert_converged(&cfg);
}

#[test]
fn damaged_journal_is_set_aside_and_everything_is_rehashed() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let opts = RunOptions {
        interrupt: Some((Boundary::AfterIntent, 2)),
        ..Default::default()
    };
    assert!(apply(&cfg, opts).is_err());
    let path = cfg.state.join("journal.jsonl");
    let mut bytes = fs::read(&path).unwrap();
    bytes[10] ^= 0x01;
    fs::write(&path, bytes).unwrap();
    let report = apply(&cfg, Default::default()).unwrap();
    assert!(report.journal_reset);
    let damaged = fs::read_dir(&cfg.state)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("journal.damaged-")
        })
        .count();
    assert_eq!(damaged, 1);
    assert_converged(&cfg);
}

#[test]
fn same_size_same_mtime_corruption_is_repaired() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    apply(&cfg, Default::default()).unwrap();
    let target = game(&cfg).join("EP01/content.package");
    let original = fs::read(&target).unwrap();
    let mtime = fs::metadata(&target).unwrap().modified().unwrap();
    let mut damaged = original.clone();
    damaged[0] ^= 0xff;
    fs::write(&target, &damaged).unwrap();
    fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), mtime);
    let report = apply(&cfg, Default::default()).unwrap();
    assert_eq!(report.replaced, vec!["EP01/content.package".to_string()]);
    assert_eq!(fs::read(&target).unwrap(), original);
}

#[test]
fn full_verify_and_unreadable_cache_still_converge() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    apply(&cfg, Default::default()).unwrap();
    fs::write(cfg.state.join("hash-cache.json"), "not a cache").unwrap();
    let opts = RunOptions {
        full_verify: true,
        ..Default::default()
    };
    assert!(apply(&cfg, opts).unwrap().replaced.is_empty());
    assert_converged(&cfg);
}

#[cfg(unix)]
#[test]
fn executable_bit_drift_is_repaired() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    apply(&cfg, Default::default()).unwrap();
    let target = game(&cfg).join("Data/base.package");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let report = apply(&cfg, Default::default()).unwrap();
    assert_eq!(report.replaced, vec!["Data/base.package".to_string()]);
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[test]
fn new_release_deletes_only_unmodified_obsolete_files() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    apply(&cfg, Default::default()).unwrap();
    let game = game(&cfg);
    fs::write(game.join("Data/old.package"), "old release content").unwrap();
    fs::write(game.join("Data/edited.package"), "user changed this").unwrap();
    let old = storage::hash_file(&game.join("Data/old.package"))
        .unwrap()
        .0;
    let expected_edit = blake3::hash(b"original content").to_hex().to_string();
    let version = publish_next(&cfg, |m| {
        m.release_id = "fixture-2".into();
        m.deletions = vec![
            Deletion {
                root_id: "game".into(),
                path: "Data/old.package".into(),
                expected_hash: old,
            },
            Deletion {
                root_id: "game".into(),
                path: "Data/edited.package".into(),
                expected_hash: expected_edit,
            },
        ];
    })
    .unwrap();
    assert_eq!(version, 2);
    let report = apply(&cfg, Default::default()).unwrap();
    assert_eq!(report.deleted, vec!["Data/old.package".to_string()]);
    assert_eq!(report.preserved, vec!["Data/edited.package".to_string()]);
    assert!(!game.join("Data/old.package").exists());
    assert_eq!(
        fs::read_to_string(game.join("Data/edited.package")).unwrap(),
        "user changed this"
    );
    assert_eq!(
        Policy::load(&cfg.state).unwrap().applied.unwrap().release,
        "fixture-2"
    );
}

#[test]
fn older_metadata_is_rejected_after_a_newer_release() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let timestamp = fixture_dir(&cfg).join("repository/metadata/timestamp.json");
    let v1 = fs::read(&timestamp).unwrap();
    load(&cfg).unwrap();
    publish_next(&cfg, |m| m.release_id = "fixture-2".into()).unwrap();
    assert_eq!(load(&cfg).unwrap().manifest().release_id, "fixture-2");
    fs::write(&timestamp, v1).unwrap();
    assert!(load(&cfg).is_err());
}

#[test]
fn promotion_refuses_missing_objects_and_keeps_current_release() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let timestamp = fixture_dir(&cfg).join("repository/metadata/timestamp.json");
    let before = fs::read(&timestamp).unwrap();
    let missing = blake3::hash(b"never uploaded").to_hex().to_string();
    let result = publish_next(&cfg, |m| {
        m.release_id = "fixture-2".into();
        m.files[0].full.hash = missing.clone();
        m.files[0].full.decoded_hash = missing.clone();
        m.files[0].target_hash = missing.clone();
    });
    assert!(result.is_err());
    assert_eq!(fs::read(&timestamp).unwrap(), before);
    assert_eq!(load(&cfg).unwrap().manifest().release_id, "fixture-1");
}

#[test]
fn renewal_re_signs_the_current_release() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let dir = fixture_dir(&cfg);
    let digest = load(&cfg).unwrap().digest().to_owned();
    let version = rt()
        .block_on(publisher::renew(
            &cfg.trusted_root,
            vec![dir.join("keys/fixture-key.pem")],
            &dir.join("repository"),
        ))
        .unwrap();
    assert_eq!(version, 2);
    assert_eq!(load(&cfg).unwrap().digest(), digest);
    let roles = publisher::expiry(&dir.join("repository")).unwrap();
    let timestamp = roles.iter().find(|(r, _)| r == "timestamp").unwrap().1;
    assert!(timestamp > jiff::Timestamp::now() + std::time::Duration::from_secs(20 * 3600));
}

#[cfg(unix)]
#[test]
fn published_objects_are_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    for e in fs::read_dir(cfg.local_objects.as_ref().unwrap()).unwrap() {
        let mode = e.unwrap().metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }
}

#[test]
fn corrupted_object_does_not_replace_live_file() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let release = load(&cfg).unwrap();
    let object = &release.manifest().files[0].full;
    fs::write(
        cfg.local_objects.as_ref().unwrap().join(&object.hash),
        "bad",
    )
    .unwrap();
    assert!(apply(&cfg, Default::default()).is_err());
    let p = Policy::load(&cfg.state).unwrap();
    assert!(!p.install.roots["game"].join("Data/base.package").exists());
    assert!(p.applied.is_none());
}

#[test]
fn external_version_is_not_overwritten() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let marker = game(&cfg).join("Game/version.bin");
    fs::write(&marker, "new unknown version").unwrap();
    let error = apply(&cfg, Default::default()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<patcher_core::transfer::Blocker>(),
        Some(&patcher_core::transfer::Blocker::UnknownVersion)
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), "new unknown version");
    assert!(!game(&cfg).join("Data/base.package").exists());
}

#[cfg(unix)]
#[test]
fn escaping_symlink_cannot_be_written() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let outside = t.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, game(&cfg).join("Data")).unwrap();
    assert!(apply(&cfg, Default::default()).is_err());
    assert!(!outside.join("base.package").exists());
}

#[test]
fn tuf_rejects_changed_target() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let targets = t.path().join("fixture/repository/targets");
    for e in fs::read_dir(targets).unwrap() {
        fs::write(e.unwrap().path(), "{}").unwrap();
    }
    assert!(load(&cfg).is_err());
}

#[test]
fn manifest_rejects_duplicates_cycles_traversal_and_disabled_dependencies() {
    assert!(Manifest::parse(br#"{"schema":1,"schema":1}"#).is_err());
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let mut m = load(&cfg).unwrap().manifest().clone();
    m.files[0].path = "../escape".into();
    assert!(m.validate().is_err());
    m.files[0].path = "Data/BASE.package".into();
    m.files[1].path = "Data/base.package".into();
    assert!(m.validate().is_err(), "case-fold collision");
    let mut m = load(&cfg).unwrap().manifest().clone();
    m.groups[0].requires.push("EP01".into());
    assert!(m.validate().is_err(), "cycle");
    let mut m = load(&cfg).unwrap().manifest().clone();
    let sp01 = m.groups.iter_mut().find(|g| g.id == "SP01").unwrap();
    sp01.requires.push("EP01".into());
    m.validate().unwrap();
    let mut selection = patcher_core::model::Selection::default();
    selection.dlc.insert("SP01".into(), true);
    selection.dlc.insert("EP01".into(), false);
    assert!(m.selected(&selection).is_err());
}

#[test]
fn torn_journal_tail_recovers_but_corrupt_record_fails() {
    use std::io::Write;
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("journal");
    let mut j = Journal::open(&p).unwrap();
    j.append(Event::Bind {
        binding: Binding {
            install: "i".into(),
            manifest: "m".into(),
            policy_revision: 1,
        },
    })
    .unwrap();
    drop(j);
    fs::OpenOptions::new()
        .append(true)
        .open(&p)
        .unwrap()
        .write_all(b"{\"torn")
        .unwrap();
    let mut recovered = Journal::open(&p).unwrap();
    assert_eq!(recovered.events.len(), 1);
    // A record appended after trimming the torn tail lands at the end of the valid records.
    recovered
        .append(Event::Complete {
            binding: Binding {
                install: "i".into(),
                manifest: "m".into(),
                policy_revision: 1,
            },
        })
        .unwrap();
    drop(recovered);
    assert_eq!(Journal::open(&p).unwrap().events.len(), 2);
    let mut b = fs::read(&p).unwrap();
    b[2] = b'X';
    fs::write(&p, b).unwrap();
    let error = Journal::open(&p).err().unwrap();
    assert!(error.downcast_ref::<storage::JournalCorrupt>().is_some());
}

#[test]
fn policy_recovers_last_valid_copy_and_fails_closed_when_both_are_bad() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    let mut p = Policy::load(&cfg.state).unwrap();
    p.revision = 2;
    p.save(&cfg.state).unwrap();
    fs::write(cfg.state.join("policy.json"), "bad").unwrap();
    let recovered = Policy::load(&cfg.state).unwrap();
    assert_eq!(recovered.revision, 1);
    assert_eq!(recovered.desired.dlc.get("EP01"), Some(&true));
    fs::write(cfg.state.join("policy.previous.json"), "bad").unwrap();
    assert!(Policy::load(&cfg.state).is_err());
}

#[test]
fn discovery_parses_escaped_valve_paths() {
    let p = patcher_core::discovery::library_paths(
        r#""libraryfolders" { "0" { "path" "/games/Steam" } "1" { "path" "D:\\Steam" } }"#,
    );
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].to_str(), Some("/games/Steam"));
    assert_eq!(p[1].to_str(), Some("D:\\Steam"));
}
