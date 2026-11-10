//! The user's own Sims 4 installation, read in place: which packs are installed and complete,
//! which languages are installed, whether files changed since the last check, and launching the
//! game. Nothing here needs a published release, and nothing here writes inside the game folder.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Expansion,
    Game,
    Stuff,
    Kit,
    Free,
}

/// Packs by folder id. Names are the published pack names; an id found on disk that is not
/// listed here is still shown, under its id, with its kind taken from the prefix.
const CATALOG: &[(&str, &str, Kind)] = &[
    ("EP01", "Get to Work", Kind::Expansion),
    ("EP02", "Get Together", Kind::Expansion),
    ("EP03", "City Living", Kind::Expansion),
    ("EP04", "Cats & Dogs", Kind::Expansion),
    ("EP05", "Seasons", Kind::Expansion),
    ("EP06", "Get Famous", Kind::Expansion),
    ("EP07", "Island Living", Kind::Expansion),
    ("EP08", "Discover University", Kind::Expansion),
    ("EP09", "Eco Lifestyle", Kind::Expansion),
    ("EP10", "Snowy Escape", Kind::Expansion),
    ("EP11", "Cottage Living", Kind::Expansion),
    ("EP12", "High School Years", Kind::Expansion),
    ("EP13", "Growing Together", Kind::Expansion),
    ("EP14", "Horse Ranch", Kind::Expansion),
    ("EP15", "For Rent", Kind::Expansion),
    ("EP16", "Lovestruck", Kind::Expansion),
    ("EP17", "Life & Death", Kind::Expansion),
    ("EP18", "Businesses & Hobbies", Kind::Expansion),
    ("EP19", "Enchanted by Nature", Kind::Expansion),
    ("EP20", "Adventure Awaits", Kind::Expansion),
    ("GP01", "Outdoor Retreat", Kind::Game),
    ("GP02", "Spa Day", Kind::Game),
    ("GP03", "Dine Out", Kind::Game),
    ("GP04", "Vampires", Kind::Game),
    ("GP05", "Parenthood", Kind::Game),
    ("GP06", "Jungle Adventure", Kind::Game),
    ("GP07", "StrangerVille", Kind::Game),
    ("GP08", "Realm of Magic", Kind::Game),
    ("GP09", "Journey to Batuu", Kind::Game),
    ("GP10", "Dream Home Decorator", Kind::Game),
    ("GP11", "My Wedding Stories", Kind::Game),
    ("GP12", "Werewolves", Kind::Game),
    ("FP01", "Holiday Celebration", Kind::Free),
    ("SP01", "Luxury Party", Kind::Stuff),
    ("SP02", "Perfect Patio", Kind::Stuff),
    ("SP03", "Cool Kitchen", Kind::Stuff),
    ("SP04", "Spooky", Kind::Stuff),
    ("SP05", "Movie Hangout", Kind::Stuff),
    ("SP06", "Romantic Garden", Kind::Stuff),
    ("SP07", "Kids Room", Kind::Stuff),
    ("SP08", "Backyard", Kind::Stuff),
    ("SP09", "Vintage Glamour", Kind::Stuff),
    ("SP10", "Bowling Night", Kind::Stuff),
    ("SP11", "Fitness", Kind::Stuff),
    ("SP12", "Toddler", Kind::Stuff),
    ("SP13", "Laundry Day", Kind::Stuff),
    ("SP14", "My First Pet", Kind::Stuff),
    ("SP15", "Moschino", Kind::Stuff),
    ("SP16", "Tiny Living", Kind::Stuff),
    ("SP17", "Nifty Knitting", Kind::Stuff),
    ("SP18", "Paranormal", Kind::Stuff),
    ("SP20", "Throwback Fit", Kind::Kit),
    ("SP21", "Country Kitchen", Kind::Kit),
    ("SP22", "Bust the Dust", Kind::Kit),
    ("SP23", "Courtyard Oasis", Kind::Kit),
    ("SP24", "Fashion Street", Kind::Kit),
    ("SP25", "Industrial Loft", Kind::Kit),
    ("SP26", "Incheon Arrivals", Kind::Kit),
    ("SP28", "Modern Menswear", Kind::Kit),
    ("SP29", "Blooming Rooms", Kind::Kit),
    ("SP30", "Carnaval Streetwear", Kind::Kit),
    ("SP31", "Decor to the Max", Kind::Kit),
    ("SP32", "Moonlight Chic", Kind::Kit),
    ("SP33", "Little Campers", Kind::Kit),
    ("SP34", "First Fits", Kind::Kit),
    ("SP35", "Desert Luxe", Kind::Kit),
    ("SP36", "Pastel Pop", Kind::Kit),
    ("SP37", "Everyday Clutter", Kind::Kit),
    ("SP38", "Simtimates Collection", Kind::Kit),
    ("SP39", "Bathroom Clutter", Kind::Kit),
    ("SP40", "Greenhouse Haven", Kind::Kit),
    ("SP41", "Basement Treasures", Kind::Kit),
    ("SP42", "Grunge Revival", Kind::Kit),
    ("SP43", "Book Nook", Kind::Kit),
    ("SP44", "Poolside Splash", Kind::Kit),
    ("SP45", "Modern Luxe", Kind::Kit),
    ("SP46", "Home Chef Hustle", Kind::Stuff),
    ("SP47", "Castle Estate", Kind::Kit),
    ("SP48", "Goth Galore", Kind::Kit),
    ("SP49", "Crystal Creations", Kind::Stuff),
];

/// Pack ids a full installation contains beyond the named ones above. Their cards show the id
/// until a confirmed name is added; they are listed so a missing one shows as not installed.
const UNNAMED: &[&str] = &[
    "SP50", "SP51", "SP52", "SP53", "SP54", "SP55", "SP56", "SP57", "SP58", "SP59", "SP60", "SP61",
    "SP62", "SP63", "SP64", "SP65", "SP66", "SP67", "SP68", "SP69", "SP70", "SP71", "SP72", "SP73",
    "SP74", "SP81",
];

/// Files every pack folder contains in a complete installation, and every `Delta/<pack>`
/// folder beside it. Taken from a full installation's file inventory (106 packs).
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

pub fn is_pack_id(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 4
        && matches!(&b[..2], b"EP" | b"GP" | b"SP" | b"FP")
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Expansion => "expansion",
            Kind::Game => "game",
            Kind::Stuff => "stuff",
            Kind::Kit => "kit",
            Kind::Free => "free",
        }
    }
}

/// A pack's kind from its id.
pub fn kind_of(id: &str) -> Kind {
    describe(id).1
}

fn describe(id: &str) -> (String, Kind) {
    if let Some((_, name, kind)) = CATALOG.iter().find(|(i, _, _)| *i == id) {
        return ((*name).into(), *kind);
    }
    let kind = match &id[..2] {
        "EP" => Kind::Expansion,
        "GP" => Kind::Game,
        "FP" => Kind::Free,
        _ => Kind::Kit,
    };
    (id.into(), kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Installed,
    /// The pack's folder exists but required files are missing or empty.
    Incomplete,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalPack {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub status: Status,
    /// Bytes on disk, including the pack's `Delta` folder.
    pub bytes: u64,
    /// Required files that are missing or empty, relative to the game folder.
    pub missing: Vec<String>,
}

fn tree_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tree_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn present(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Every known pack, and every pack folder found on disk, with its installed state.
pub fn scan(root: &Path) -> Vec<LocalPack> {
    let mut ids: Vec<String> = CATALOG
        .iter()
        .map(|(id, _, _)| *id)
        .chain(UNNAMED.iter().copied())
        .map(String::from)
        .collect();
    if let Ok(entries) = fs::read_dir(root) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_pack_id(&name) && e.path().is_dir() && !ids.contains(&name) {
                ids.push(name);
            }
        }
    }
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let (name, kind) = describe(&id);
            let main = root.join(&id);
            let delta = root.join("Delta").join(&id);
            if !main.is_dir() {
                return LocalPack {
                    id,
                    name,
                    kind,
                    status: Status::NotInstalled,
                    bytes: 0,
                    missing: Vec::new(),
                };
            }
            let missing: Vec<String> = PACK_FILES
                .iter()
                .filter(|f| !present(&main.join(f)))
                .map(|f| format!("{id}/{f}"))
                .chain(
                    DELTA_FILES
                        .iter()
                        .filter(|f| !present(&delta.join(f)))
                        .map(|f| format!("Delta/{id}/{f}")),
                )
                .collect();
            LocalPack {
                status: if missing.is_empty() {
                    Status::Installed
                } else {
                    Status::Incomplete
                },
                bytes: tree_bytes(&main) + tree_bytes(&delta),
                id,
                name,
                kind,
                missing,
            }
        })
        .collect()
}

/// Installed game languages, as codes such as `ENG_US`, from the base game's string files.
pub fn languages(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(root.join("Data").join("Client"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let code = name.strip_prefix("Strings_")?.strip_suffix(".package")?;
            let valid = code.len() == 6
                && code.as_bytes()[3] == b'_'
                && code.bytes().all(|b| b == b'_' || b.is_ascii_uppercase());
            valid.then(|| code.to_string())
        })
        .collect();
    found.sort();
    found
}

/// The pack a game file belongs to, or `None` for the base game.
pub fn owner(relative: &str) -> Option<String> {
    let mut parts = relative.split('/');
    let first = parts.next()?;
    if is_pack_id(first) {
        return Some(first.into());
    }
    if first == "Delta"
        && let Some(id) = parts.next()
        && is_pack_id(id)
    {
        return Some(id.into());
    }
    None
}

/// The folders whose files belong to the game: the base game and every pack. Installers,
/// support tools and soundtracks are left out; saves and mods live elsewhere.
fn in_scope(relative: &str) -> bool {
    let first = relative.split('/').next().unwrap_or_default();
    matches!(first, "Data" | "Delta" | "Game") || is_pack_id(first)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Recorded {
    pub size: u64,
    pub hash: String,
}

/// The fingerprint of every game file at the last accepted check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema: u32,
    pub recorded_at: String,
    pub files: BTreeMap<String, Recorded>,
}

impl Baseline {
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let baseline: Self =
            serde_json::from_value(crate::strict_json::parse(&bytes).ok()?).ok()?;
        (baseline.schema == 1).then_some(baseline)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        crate::storage::atomic_write(path, &serde_json::to_vec(self)?)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PackCheck {
    pub files: u64,
    pub changed: Vec<String>,
    pub missing: Vec<String>,
    pub added: Vec<String>,
}

impl PackCheck {
    pub fn clean(&self) -> bool {
        self.changed.is_empty() && self.missing.is_empty() && self.added.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    /// True when there was no earlier check to compare with; this check became the baseline.
    pub first: bool,
    pub files: u64,
    pub bytes: u64,
    /// Results by pack id, and `base` for the base game.
    pub packs: BTreeMap<String, PackCheck>,
    /// The fingerprints just taken, for accepting them as the new baseline.
    #[serde(skip)]
    pub current: BTreeMap<String, Recorded>,
}

pub struct CheckProgress {
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf, u64)>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        let relative = path
            .strip_prefix(root)?
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if !in_scope(&relative) {
            continue;
        }
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if kind.is_file() {
            out.push((relative, path, entry.metadata()?.len()));
        }
    }
    Ok(())
}

/// Hashes every game file and compares it with `baseline`. Read-only: the game folder is never
/// written. Without a baseline the result becomes the first one. `stop` ends the check early.
pub fn check(
    root: &Path,
    baseline: Option<&Baseline>,
    stop: &AtomicBool,
    mut progress: impl FnMut(CheckProgress),
) -> Result<CheckReport> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    let bytes_total: u64 = files.iter().map(|f| f.2).sum();
    let mut current = BTreeMap::new();
    let mut bytes_done = 0;
    for (i, (relative, path, size)) in files.iter().enumerate() {
        if stop.load(Ordering::Acquire) {
            bail!("Check stopped");
        }
        progress(CheckProgress {
            files_done: i,
            files_total: files.len(),
            bytes_done,
            bytes_total,
            current: relative.clone(),
        });
        let (hash, read) = crate::storage::hash_file(path)?;
        ensure!(
            read == *size,
            "{relative} changed while it was being checked"
        );
        current.insert(relative.clone(), Recorded { size: read, hash });
        bytes_done += size;
    }
    let mut packs: BTreeMap<String, PackCheck> = BTreeMap::new();
    let key = |relative: &str| owner(relative).unwrap_or_else(|| "base".into());
    for (relative, now) in &current {
        let entry = packs.entry(key(relative)).or_default();
        entry.files += 1;
        match baseline.and_then(|b| b.files.get(relative)) {
            Some(before) if before != now => entry.changed.push(relative.clone()),
            None if baseline.is_some() => entry.added.push(relative.clone()),
            _ => {}
        }
    }
    if let Some(b) = baseline {
        for relative in b.files.keys().filter(|r| !current.contains_key(*r)) {
            packs
                .entry(key(relative))
                .or_default()
                .missing
                .push(relative.clone());
        }
    }
    Ok(CheckReport {
        first: baseline.is_none(),
        files: current.len() as u64,
        bytes: bytes_total,
        packs,
        current,
    })
}

/// Steam's id for The Sims 4.
const STEAM_APP_ID: u32 = 1_222_670;

/// Starts the game the way its store expects: through Steam for a Steam installation (Steam
/// then applies its own launch options), otherwise the game executable, which the EA app takes
/// over on Windows.
pub fn launch(root: &Path, source: &str) -> Result<()> {
    let steam = source == "Steam library" || root.to_string_lossy().contains("steamapps");
    let mut command = if steam {
        let url = format!("steam://rungameid/{STEAM_APP_ID}");
        if cfg!(windows) {
            let mut c = std::process::Command::new("cmd.exe");
            crate::app::no_window(&mut c);
            c.args(["/d", "/c", "start", "", &url]);
            c
        } else if cfg!(target_os = "macos") {
            let mut c = std::process::Command::new("open");
            c.arg(url);
            c
        } else {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(url);
            c
        }
    } else if cfg!(windows) {
        let exe = root.join("Game").join("Bin").join("TS4_x64.exe");
        ensure!(exe.is_file(), "The game executable was not found");
        let mut c = std::process::Command::new(exe);
        c.current_dir(root.join("Game").join("Bin"));
        c
    } else {
        bail!("Launching this installation is only available through Steam on this system")
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().context("Could not start the game")?;
    Ok(())
}
