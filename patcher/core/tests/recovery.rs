//! Recovery after interruptions and failures at every journal, rename and flush point, and
//! reclamation of downloaded objects.
use patcher_core::{
    engine::{self, Boundary, RunOptions, RunReport},
    fixture,
    model::{Deletion, Manifest, Object},
    publisher,
    storage::{self, Event, Journal, Policy},
    transfer::{Control, LocalSource, ObjectSource},
    trust,
};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Counts fetches, and downloads: fetches the download cache could not answer.
struct Counting {
    inner: LocalSource,
    fetches: RefCell<Vec<String>>,
    downloads: RefCell<Vec<String>>,
}

impl ObjectSource for Counting {
    fn fetch(
        &self,
        object: &Object,
        cache: &Path,
        control: &Control,
        progress: &dyn Fn(u64, u64),
    ) -> anyhow::Result<PathBuf> {
        self.fetches.borrow_mut().push(object.hash.clone());
        if !cache.join(&object.hash).exists() {
            self.downloads.borrow_mut().push(object.hash.clone());
        }
        self.inner.fetch(object, cache, control, progress)
    }
}

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
fn counting(cfg: &fixture::Config) -> Counting {
    Counting {
        inner: LocalSource {
            directory: cfg.local_objects.clone().unwrap(),
        },
        fetches: RefCell::default(),
        downloads: RefCell::default(),
    }
}
fn run_on(
    cfg: &fixture::Config,
    source: &Counting,
    interrupt: Option<(Boundary, usize)>,
) -> anyhow::Result<RunReport> {
    let r = load(cfg)?;
    let mut p = Policy::load(&cfg.state)?;
    engine::run(
        &r,
        &mut p,
        &cfg.state,
        source,
        &Control::default(),
        &|_| {},
        &RunOptions {
            interrupt,
            ..Default::default()
        },
    )
}
fn apply(cfg: &fixture::Config) -> anyhow::Result<RunReport> {
    run_on(cfg, &counting(cfg), None)
}
fn game(cfg: &fixture::Config) -> PathBuf {
    Policy::load(&cfg.state).unwrap().install.roots["game"].clone()
}
fn fixture_dir(cfg: &fixture::Config) -> PathBuf {
    cfg.state.parent().unwrap().to_path_buf()
}
fn publish_next(cfg: &fixture::Config, change: impl FnOnce(&mut Manifest)) {
    let dir = fixture_dir(cfg);
    let path = dir.join("input/release.json");
    let mut m = Manifest::parse(&fs::read(&path).unwrap()).unwrap();
    change(&mut m);
    fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    rt().block_on(publisher::publish(
        &cfg.trusted_root,
        vec![dir.join("keys/fixture-key.pem")],
        &path,
        &dir.join("repository"),
    ))
    .unwrap();
}
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}
fn temporaries(cfg: &fixture::Config) -> Vec<PathBuf> {
    files_under(&game(cfg))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".patcher-")
        })
        .collect()
}
fn downloads(cfg: &fixture::Config) -> Vec<String> {
    match fs::read_dir(cfg.state.join("downloads")) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}
/// Selected files whose destination already holds the target, by object hash.
fn completed(cfg: &fixture::Config) -> BTreeSet<String> {
    let release = load(cfg).unwrap();
    let p = Policy::load(&cfg.state).unwrap();
    release
        .manifest()
        .selected(&p.desired)
        .unwrap()
        .into_iter()
        .filter(|f| {
            storage::hash_file(&game(cfg).join(&f.path)).ok()
                == Some((f.target_hash.clone(), f.target_size))
        })
        .map(|f| f.full.hash.clone())
        .collect()
}
fn plant_unrelated(cfg: &fixture::Config) {
    let g = game(cfg);
    fs::write(g.join("unmanaged.txt"), "leave me").unwrap();
    fs::create_dir_all(g.join("Saves")).unwrap();
    fs::write(g.join("Saves/slot.save"), "a save").unwrap();
}
fn assert_unrelated(cfg: &fixture::Config) {
    let g = game(cfg);
    assert_eq!(
        fs::read_to_string(g.join("unmanaged.txt")).unwrap(),
        "leave me"
    );
    assert_eq!(
        fs::read_to_string(g.join("Saves/slot.save")).unwrap(),
        "a save"
    );
}
fn assert_converged(cfg: &fixture::Config) {
    let source = counting(cfg);
    let report = run_on(cfg, &source, None).unwrap();
    assert!(report.replaced.is_empty(), "rerun replaced {report:?}");
    assert!(source.downloads.borrow().is_empty());
    let release = load(cfg).unwrap();
    let p = Policy::load(&cfg.state).unwrap();
    for f in release.manifest().selected(&p.desired).unwrap() {
        let actual = storage::hash_file(&game(cfg).join(&f.path)).unwrap();
        assert_eq!(actual, (f.target_hash.clone(), f.target_size), "{}", f.path);
    }
    let applied = p.applied.unwrap();
    assert_eq!(applied.revision, p.revision);
    assert_eq!(applied.manifest_hash, release.digest());
    assert!(temporaries(cfg).is_empty(), "{:?}", temporaries(cfg));
    assert!(downloads(cfg).is_empty(), "{:?}", downloads(cfg));
    assert_eq!(fs::read(cfg.state.join("journal.jsonl")).unwrap(), b"");
}
/// Ready means the applied policy names the current release; an interrupted run never gets there.
fn assert_not_ready(cfg: &fixture::Config, context: &str) {
    let digest = load(cfg).unwrap().digest().to_owned();
    let applied = Policy::load(&cfg.state).unwrap().applied;
    assert!(
        applied.is_none_or(|a| a.manifest_hash != digest),
        "reported Ready after {context}"
    );
}
fn append_torn_record(cfg: &fixture::Config) {
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(cfg.state.join("journal.jsonl"))
        .unwrap()
        .write_all(b"{\"sequence\":")
        .unwrap();
}

/// Interrupts or fails a run at `boundary`, then recovers repeatedly, interrupted again at the
/// first occurrence of the same point, and finally without interruption. Recovery must converge,
/// never report Ready early, never download an object for a file already in place, and leave
/// unrelated files alone.
fn interrupt_and_recover(cfg: &fixture::Config, boundary: Boundary, n: usize) {
    let context = format!("{boundary:?} {n}");
    plant_unrelated(cfg);
    let source = counting(cfg);
    assert!(
        run_on(cfg, &source, Some((boundary, n))).is_err(),
        "{context}"
    );
    assert_not_ready(cfg, &context);
    if matches!(boundary, Boundary::WriteFails | Boundary::TempSyncFails) {
        assert!(temporaries(cfg).is_empty(), "{context}: failed output kept");
    }
    for round in 0..3 {
        if round == 1 {
            append_torn_record(cfg);
        }
        let done = completed(cfg);
        let source = counting(cfg);
        let result = run_on(cfg, &source, Some((boundary, 1)));
        if result.is_err() {
            assert_not_ready(cfg, &context);
        }
        for hash in source.downloads.borrow().iter() {
            assert!(
                !done.contains(hash),
                "{context}: downloaded an object for a completed file"
            );
        }
        assert_unrelated(cfg);
    }
    let done = completed(cfg);
    let source = counting(cfg);
    run_on(cfg, &source, None).unwrap_or_else(|e| panic!("{context}: {e:#}"));
    for hash in source.downloads.borrow().iter() {
        assert!(!done.contains(hash), "{context}: duplicate download");
    }
    assert_converged(cfg);
    assert_unrelated(cfg);
}

#[test]
fn every_replacement_boundary_and_failure_converges() {
    for boundary in [
        Boundary::AfterTempSync,
        Boundary::BeforeIntent,
        Boundary::AfterIntent,
        Boundary::AfterRenameBeforeSync,
        Boundary::AfterRename,
        Boundary::AfterCommit,
        Boundary::WriteFails,
        Boundary::TempSyncFails,
        Boundary::DirSyncFails,
    ] {
        for n in 1..=2 {
            let t = tempfile::tempdir().unwrap();
            let cfg = setup(&t.path().join("fixture"));
            interrupt_and_recover(&cfg, boundary, n);
        }
    }
    for boundary in [Boundary::BeforeComplete, Boundary::AfterComplete] {
        let t = tempfile::tempdir().unwrap();
        let cfg = setup(&t.path().join("fixture"));
        interrupt_and_recover(&cfg, boundary, 1);
    }
}

#[test]
fn every_deletion_boundary_and_failure_converges() {
    for boundary in [
        Boundary::AfterDeleteIntent,
        Boundary::AfterDelete,
        Boundary::AfterDeleteSync,
        Boundary::AfterDeleteCommit,
        Boundary::DeleteSyncFails,
    ] {
        for n in 1..=2 {
            let t = tempfile::tempdir().unwrap();
            let cfg = setup(&t.path().join("fixture"));
            apply(&cfg).unwrap();
            let g = game(&cfg);
            let mut deletions = Vec::new();
            for name in ["Data/old-a.package", "Data/old-b.package"] {
                fs::write(g.join(name), format!("obsolete {name}")).unwrap();
                deletions.push(Deletion {
                    root_id: "game".into(),
                    path: name.into(),
                    expected_hash: storage::hash_file(&g.join(name)).unwrap().0,
                });
            }
            publish_next(&cfg, |m| {
                m.release_id = "fixture-2".into();
                m.deletions = deletions;
            });
            interrupt_and_recover(&cfg, boundary, n);
            assert!(!g.join("Data/old-a.package").exists());
            assert!(!g.join("Data/old-b.package").exists());
            assert_eq!(
                Policy::load(&cfg.state).unwrap().applied.unwrap().release,
                "fixture-2"
            );
        }
    }
}

#[test]
fn consumed_download_is_reclaimed_after_an_interrupted_commit() {
    for boundary in [
        Boundary::AfterRenameBeforeSync,
        Boundary::AfterRename,
        Boundary::AfterCommit,
    ] {
        let t = tempfile::tempdir().unwrap();
        let cfg = setup(&t.path().join("fixture"));
        assert!(run_on(&cfg, &counting(&cfg), Some((boundary, 1))).is_err());
        assert!(!downloads(&cfg).is_empty());
        apply(&cfg).unwrap();
        assert!(
            downloads(&cfg).is_empty(),
            "{boundary:?}: {:?}",
            downloads(&cfg)
        );
    }
}

#[test]
fn reclaim_keeps_objects_the_plan_still_needs() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    assert!(run_on(&cfg, &counting(&cfg), Some((Boundary::BeforeIntent, 2))).is_err());
    let orphan = blake3::hash(b"no longer planned").to_hex().to_string();
    let cache = cfg.state.join("downloads");
    fs::write(cache.join(format!("{orphan}.part")), "partial").unwrap();
    fs::write(cache.join(format!("{orphan}.resume")), "{}").unwrap();
    fs::write(cache.join("not-ours.txt"), "kept").unwrap();
    // With the published objects gone, the run can finish only from the kept download.
    let objects = cfg.local_objects.clone().unwrap();
    fs::rename(&objects, objects.with_extension("moved")).unwrap();
    let source = counting(&cfg);
    run_on(&cfg, &source, None).unwrap();
    assert!(source.downloads.borrow().is_empty());
    assert_eq!(downloads(&cfg), vec!["not-ours.txt".to_string()]);
}

#[test]
fn verified_temporary_output_is_committed_without_fetching() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    assert!(run_on(&cfg, &counting(&cfg), Some((Boundary::AfterIntent, 2))).is_err());
    assert_eq!(temporaries(&cfg).len(), 1);
    fs::remove_dir_all(cfg.state.join("downloads")).unwrap();
    let objects = cfg.local_objects.clone().unwrap();
    fs::rename(&objects, objects.with_extension("moved")).unwrap();
    // Interrupted again between the recovered rename and its commit record.
    let source = counting(&cfg);
    assert!(run_on(&cfg, &source, Some((Boundary::AfterRecoveredCommit, 1))).is_err());
    assert!(source.fetches.borrow().is_empty());
    let source = counting(&cfg);
    let report = run_on(&cfg, &source, None).unwrap();
    assert!(
        source.fetches.borrow().is_empty(),
        "recovery fetched objects"
    );
    assert!(report.replaced.is_empty(), "{report:?}");
    fs::rename(objects.with_extension("moved"), &objects).unwrap();
    assert_converged(&cfg);

    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    assert!(run_on(&cfg, &counting(&cfg), Some((Boundary::AfterIntent, 2))).is_err());
    fs::remove_dir_all(cfg.state.join("downloads")).unwrap();
    let source = counting(&cfg);
    let report = run_on(&cfg, &source, None).unwrap();
    assert!(
        source.fetches.borrow().is_empty(),
        "recovery fetched objects"
    );
    assert_eq!(report.replaced, vec!["EP01/content.package".to_string()]);
    assert_converged(&cfg);
}

#[test]
fn invalid_temporary_output_is_removed_and_rebuilt() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    assert!(run_on(&cfg, &counting(&cfg), Some((Boundary::AfterIntent, 2))).is_err());
    let temporary = temporaries(&cfg).pop().unwrap();
    let mut bytes = fs::read(&temporary).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&temporary, bytes).unwrap();
    assert!(run_on(&cfg, &counting(&cfg), Some((Boundary::BeforeComplete, 1))).is_err());
    let events = Journal::open(&cfg.state.join("journal.jsonl"))
        .unwrap()
        .events;
    assert!(events.iter().any(|e| matches!(e, Event::Abandon { .. })));
    let report = apply(&cfg).unwrap();
    assert!(report.replaced.is_empty(), "{report:?}");
    assert_converged(&cfg);
}

#[test]
fn temporary_output_for_deselected_content_is_not_committed() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    assert!(run_on(&cfg, &counting(&cfg), Some((Boundary::AfterIntent, 2))).is_err());
    assert_eq!(temporaries(&cfg).len(), 1);
    let mut p = Policy::load(&cfg.state).unwrap();
    p.desired.dlc.insert("EP01".into(), false);
    p.revision += 1;
    p.save(&cfg.state).unwrap();
    apply(&cfg).unwrap();
    assert!(!game(&cfg).join("EP01/content.package").exists());
    assert!(temporaries(&cfg).is_empty());
    assert_converged(&cfg);
}

#[test]
fn shared_object_is_downloaded_once() {
    let t = tempfile::tempdir().unwrap();
    let cfg = setup(&t.path().join("fixture"));
    publish_next(&cfg, |m| {
        m.release_id = "fixture-2".into();
        let mut copy = m.files.iter().find(|f| f.id == "base").unwrap().clone();
        copy.id = "base-copy".into();
        copy.path = "Data/copy.package".into();
        m.files.push(copy);
        let base = m.groups.iter_mut().find(|g| g.id == "base").unwrap();
        base.file_ids.push("base-copy".into());
    });
    let source = counting(&cfg);
    let report = run_on(&cfg, &source, None).unwrap();
    assert_eq!(report.replaced.len(), 3);
    let downloads = source.downloads.borrow();
    let unique: BTreeSet<_> = downloads.iter().collect();
    assert_eq!(downloads.len(), unique.len(), "{downloads:?}");
    drop(downloads);
    assert_converged(&cfg);
}
