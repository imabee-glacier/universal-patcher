//! Synthetic, independently generated content for exercising the real signed update path.
use crate::{
    model::*,
    publisher,
    storage::{Installation, Policy, Preferences},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub state: PathBuf,
    pub trusted_root: PathBuf,
    pub metadata_url: String,
    pub targets_url: String,
    pub target: String,
    pub origin: String,
    pub local_objects: Option<PathBuf>,
}
pub async fn create(dir: &Path) -> Result<Config> {
    ensure!(!dir.exists(), "Fixture directory must be new");
    fs::create_dir_all(dir)?;
    let dir = fs::canonicalize(dir)?;
    let root = publisher::fixture_root(&dir.join("keys")).await?;
    let game = dir.join("game");
    fs::create_dir_all(game.join("Game"))?;
    fs::write(
        game.join("Game/version.bin"),
        b"independent-patcher-fixture-v1",
    )?;
    let repo = dir.join("repository");
    fs::create_dir_all(repo.join("objects"))?;
    let input = dir.join("input");
    fs::create_dir_all(&input)?;
    let mut files = Vec::new();
    let mut groups = Vec::new();
    for (id, name, kind, path, content) in [
        (
            "base",
            "Base game",
            GroupKind::Base,
            "Data/base.package",
            b"Base fixture content\n".as_slice(),
        ),
        (
            "EP01",
            "Expansion fixture",
            GroupKind::Dlc,
            "EP01/content.package",
            b"Selected expansion fixture content\n".as_slice(),
        ),
        (
            "SP01",
            "Stuff pack fixture",
            GroupKind::Dlc,
            "SP01/content.package",
            b"Optional pack fixture content\n".as_slice(),
        ),
    ] {
        let src = input.join(id);
        let data = content.repeat(10000);
        fs::write(&src, data)?;
        let object = publisher::encode(&src, &repo.join("objects"))?;
        files.push(GameFile {
            id: id.into(),
            root_id: "game".into(),
            path: path.into(),
            target_hash: object.decoded_hash.clone(),
            target_size: object.decoded_size,
            attributes: Attributes { executable: false },
            full: object,
            deltas: vec![],
        });
        groups.push(Group {
            id: id.into(),
            display_name: name.into(),
            kind,
            requires: if id == "base" {
                vec![]
            } else {
                vec!["base".into()]
            },
            file_ids: vec![id.into()],
        });
    }
    let (hash, size) = crate::storage::hash_file(&game.join("Game/version.bin"))?;
    let manifest = Manifest {
        schema: 1,
        game_id: "sims4".into(),
        variant: "fixture".into(),
        release_id: "fixture-1".into(),
        game_version: "fixture-1".into(),
        required_capabilities: vec!["files-v1".into()],
        compatible_bases: vec![Fingerprint {
            id: "fixture-base".into(),
            files: vec![Marker {
                root_id: "game".into(),
                path: "Game/version.bin".into(),
                hash,
                size,
            }],
        }],
        groups,
        files,
        deletions: vec![],
    };
    manifest.validate()?;
    let manifest_path = input.join("release.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    publisher::publish(
        &root,
        vec![dir.join("keys/fixture-key.pem")],
        &manifest_path,
        &repo,
    )
    .await?;
    let state = dir.join("state");
    fs::create_dir_all(&state)?;
    let policy = Policy {
        schema: 1,
        game_id: "sims4".into(),
        install: Installation {
            identity: crate::engine::identity(&game)?,
            variant: "fixture".into(),
            roots: BTreeMap::from([("game".into(), game)]),
        },
        revision: 1,
        desired: Selection {
            dlc: BTreeMap::from([("EP01".into(), true), ("SP01".into(), false)]),
            ..Default::default()
        },
        applied: None,
        preferences: Preferences::default(),
        suspended: None,
    };
    policy.save(&state)?;
    let cfg = Config {
        state,
        trusted_root: root,
        metadata_url: url::Url::from_directory_path(repo.join("metadata"))
            .unwrap()
            .to_string(),
        targets_url: url::Url::from_directory_path(repo.join("targets"))
            .unwrap()
            .to_string(),
        target: "release.json".into(),
        origin: url::Url::from_directory_path(&repo).unwrap().to_string(),
        local_objects: Some(repo.join("objects")),
    };
    fs::write(dir.join("config.json"), serde_json::to_vec_pretty(&cfg)?)?;
    Ok(cfg)
}
