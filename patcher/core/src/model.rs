use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub game_id: String,
    pub variant: String,
    pub release_id: String,
    pub game_version: String,
    pub required_capabilities: Vec<String>,
    pub compatible_bases: Vec<Fingerprint>,
    pub groups: Vec<Group>,
    pub files: Vec<GameFile>,
    pub deletions: Vec<Deletion>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub id: String,
    pub files: Vec<Marker>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub root_id: String,
    pub path: String,
    pub hash: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Base,
    Dlc,
    Language,
    Unlock,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub display_name: String,
    pub kind: GroupKind,
    pub requires: Vec<String>,
    pub file_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    pub executable: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameFile {
    pub id: String,
    pub root_id: String,
    pub path: String,
    pub target_hash: String,
    pub target_size: u64,
    pub attributes: Attributes,
    pub full: Object,
    pub deltas: Vec<Delta>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Raw,
    Zstd,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub hash: String,
    pub encoded_size: u64,
    pub codec: Codec,
    pub decoded_hash: String,
    pub decoded_size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    pub base_hash: String,
    pub base_size: u64,
    pub format: String,
    pub object: Object,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deletion {
    pub root_id: String,
    pub path: String,
    pub expected_hash: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub dlc: BTreeMap<String, bool>,
    pub deferred: BTreeSet<String>,
    pub language: Option<String>,
}

pub fn valid_hash(s: &str) -> Result<()> {
    ensure!(
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Invalid BLAKE3 hash"
    );
    Ok(())
}
pub fn valid_id(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "Invalid identifier"
    );
    Ok(())
}
pub fn valid_path(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 4096
            && !s.contains(['\\', ':', '\0'])
            && !s.chars().any(char::is_control),
        "Unsafe path"
    );
    for part in s.split('/') {
        ensure!(
            !part.is_empty() && part != "." && part != ".." && !part.ends_with([' ', '.']),
            "Unsafe path component"
        );
    }
    Ok(())
}
pub fn path_key(root: &str, path: &str) -> String {
    format!(
        "{root}/{}",
        path.nfkc().flat_map(char::to_lowercase).collect::<String>()
    )
}
impl Object {
    pub fn validate(&self) -> Result<()> {
        valid_hash(&self.hash)?;
        valid_hash(&self.decoded_hash)?;
        ensure!(
            self.encoded_size <= i64::MAX as u64 && self.decoded_size <= i64::MAX as u64,
            "Object size exceeds supported range"
        );
        if self.codec == Codec::Raw {
            ensure!(
                self.hash == self.decoded_hash && self.encoded_size == self.decoded_size,
                "Raw object metadata differs"
            );
        }
        Ok(())
    }
}
impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            "Manifest exceeds 32 MiB limit"
        );
        let value = crate::strict_json::parse(bytes)?;
        let m: Self = serde_json::from_value(value).context("Invalid manifest schema")?;
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "Unsupported manifest schema");
        ensure!(self.game_id == "sims4", "Unsupported game");
        ensure!(
            ["linux-windows", "macos-native", "fixture"].contains(&self.variant.as_str()),
            "Unsupported variant"
        );
        valid_id(&self.release_id)?;
        for cap in &self.required_capabilities {
            ensure!(
                ["files-v1", "raw", "zstd"].contains(&cap.as_str()),
                "Unsupported required capability: {cap}"
            );
        }
        ensure!(
            !self.compatible_bases.is_empty(),
            "No compatible installation fingerprints"
        );
        let mut fingerprints = BTreeSet::new();
        for base in &self.compatible_bases {
            valid_id(&base.id)?;
            ensure!(fingerprints.insert(&base.id), "Duplicate base id");
            ensure!(!base.files.is_empty(), "Empty fingerprint");
            for f in &base.files {
                valid_id(&f.root_id)?;
                valid_path(&f.path)?;
                valid_hash(&f.hash)?;
            }
        }
        let mut paths = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for f in &self.files {
            valid_id(&f.id)?;
            valid_id(&f.root_id)?;
            valid_path(&f.path)?;
            valid_hash(&f.target_hash)?;
            ensure!(ids.insert(&f.id), "Duplicate file id");
            ensure!(
                paths.insert(path_key(&f.root_id, &f.path)),
                "Destination collision"
            );
            f.full.validate()?;
            ensure!(
                f.full.decoded_hash == f.target_hash && f.full.decoded_size == f.target_size,
                "Full object differs from target"
            );
            for d in &f.deltas {
                valid_hash(&d.base_hash)?;
                d.object.validate()?;
                ensure!(
                    d.object.encoded_size < f.full.encoded_size,
                    "Delta is not smaller than full object"
                );
            }
        }
        // A file cannot also be an ancestor directory of another managed file.
        for path in &paths {
            let mut prefix = path.as_str();
            while let Some((p, _)) = prefix.rsplit_once('/') {
                ensure!(!paths.contains(p), "File/directory collision");
                prefix = p;
            }
        }
        let mut group_ids = BTreeSet::new();
        let mut owned = BTreeSet::new();
        for g in &self.groups {
            valid_id(&g.id)?;
            ensure!(group_ids.insert(&g.id), "Duplicate group id");
            ensure!(!g.display_name.trim().is_empty(), "Empty group name");
            for f in &g.file_ids {
                ensure!(ids.contains(f), "Unknown file reference");
                owned.insert(f);
            }
        }
        ensure!(owned == ids, "Ungrouped manifest files");
        for g in &self.groups {
            let mut visiting = BTreeSet::new();
            let mut visited = BTreeSet::new();
            self.visit(&g.id, &mut visiting, &mut visited)?;
        }
        let mut deletions = BTreeSet::new();
        for d in &self.deletions {
            valid_id(&d.root_id)?;
            valid_path(&d.path)?;
            valid_hash(&d.expected_hash)?;
            let k = path_key(&d.root_id, &d.path);
            ensure!(
                !paths.contains(&k) && deletions.insert(k),
                "Conflicting deletion"
            );
        }
        Ok(())
    }
    fn visit<'a>(
        &'a self,
        id: &'a str,
        visiting: &mut BTreeSet<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        ensure!(visiting.insert(id), "Group dependency cycle");
        let g = self
            .groups
            .iter()
            .find(|g| g.id == id)
            .context("Unknown dependency")?;
        for dep in &g.requires {
            self.visit(dep, visiting, done)?;
        }
        visiting.remove(id);
        done.insert(id);
        Ok(())
    }
    pub fn selected<'a>(&'a self, selection: &Selection) -> Result<Vec<&'a GameFile>> {
        for id in selection.dlc.keys() {
            ensure!(
                self.groups
                    .iter()
                    .any(|g| g.id == *id && g.kind == GroupKind::Dlc),
                "Unknown DLC selection: {id}"
            );
        }
        if let Some(lang) = &selection.language {
            ensure!(
                self.groups
                    .iter()
                    .any(|g| g.id == *lang && g.kind == GroupKind::Language),
                "Unknown language"
            );
        }
        let mut done = BTreeSet::new();
        for g in &self.groups {
            let on = match g.kind {
                GroupKind::Base => true,
                GroupKind::Dlc => selection.dlc.get(&g.id) == Some(&true),
                GroupKind::Language => selection.language.as_ref() == Some(&g.id),
                GroupKind::Unlock => false,
            };
            if on {
                self.visit(&g.id, &mut BTreeSet::new(), &mut done)?;
            }
        }
        for id in &done {
            let g = self.groups.iter().find(|g| g.id == *id).unwrap();
            if g.kind == GroupKind::Dlc && selection.dlc.get(*id) != Some(&true) {
                bail!("Selected content requires disabled or unanswered DLC: {id}");
            }
            ensure!(
                g.kind != GroupKind::Unlock,
                "Unlock capability has not been validated"
            );
        }
        let ids: BTreeSet<_> = self
            .groups
            .iter()
            .filter(|g| done.contains(g.id.as_str()))
            .flat_map(|g| g.file_ids.iter())
            .collect();
        Ok(self.files.iter().filter(|f| ids.contains(&f.id)).collect())
    }
}

/// Signed description of one patcher build for one platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub schema: u32,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub required_capabilities: Vec<String>,
    /// A tar archive of the application directory, as a served object.
    pub artifact: Object,
    /// Inclusive range of local state schema versions this build reads.
    pub state_schema_range: [u32; 2],
}

/// Parses `MAJOR.MINOR.PATCH` with no leading zeros.
pub fn parse_version(v: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = v.split('.').collect();
    ensure!(parts.len() == 3, "Version must be MAJOR.MINOR.PATCH");
    let mut n = [0u64; 3];
    for (slot, part) in n.iter_mut().zip(&parts) {
        ensure!(
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0')),
            "Version must be MAJOR.MINOR.PATCH"
        );
        *slot = part.parse()?;
    }
    Ok((n[0], n[1], n[2]))
}

impl AppManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= 1024 * 1024,
            "Application manifest exceeds limit"
        );
        let m: Self = serde_json::from_value(crate::strict_json::parse(bytes)?)
            .context("Invalid application manifest schema")?;
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "Unsupported application manifest schema");
        parse_version(&self.app_version)?;
        ensure!(
            ["linux", "macos", "windows"].contains(&self.os.as_str()),
            "Unsupported operating system"
        );
        ensure!(
            ["x64", "arm64"].contains(&self.arch.as_str()),
            "Unsupported architecture"
        );
        for cap in &self.required_capabilities {
            ensure!(
                ["app-v1", "raw", "zstd"].contains(&cap.as_str()),
                "Unsupported required capability: {cap}"
            );
        }
        self.artifact.validate()?;
        ensure!(
            self.state_schema_range[0] <= self.state_schema_range[1],
            "Invalid state schema range"
        );
        Ok(())
    }
}
