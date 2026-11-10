//! The user's own Sims 4 installation, read in place: which packs are installed and complete,
//! which languages are installed, and how the game is started. Nothing here needs a published
//! release, and nothing here writes inside the game folder.
use serde::Serialize;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
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
/// Steam's id for The Sims 4.
pub const STEAM_APP_ID: u32 = 1_222_670;

/// True for an installation inside a Steam library.
pub fn is_steam(root: &Path, source: &str) -> bool {
    source == "Steam library"
        || root
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("steamapps"))
}

/// A program to start, with its arguments, extra environment and working folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(String, OsString)>,
    pub dir: Option<PathBuf>,
}

/// How the patcher starts this installation. The game is started wherever it came from: a
/// store, a plain folder, or a Wine prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A Steam library on a system where the game needs Steam's compatibility layer: Steam
    /// starts it.
    Steam,
    /// The game program itself, directly or through Wine.
    Program(Run),
    /// Nothing here can start it; the reason says what is missing.
    Unsupported(String),
}

/// The game program, relative to the game folder.
pub const GAME_PROGRAM: &str = "Game/Bin/TS4_x64.exe";

/// The Wine prefix a game folder lies in: the folder holding the `drive_c` above it.
pub fn wine_prefix(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == "drive_c"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// What this system offers for starting a Windows program that is not native here.
#[derive(Debug, Clone, Default)]
pub struct Runners {
    /// Steam links can be opened.
    pub steam_links: bool,
    /// The `wine` program, when installed.
    pub wine: Option<PathBuf>,
}

/// Decides how to start the game at `root`. `windows` and `mac` name the system the patcher
/// runs on.
pub fn how_to_launch_on(
    root: &Path,
    source: &str,
    windows: bool,
    mac: bool,
    runners: &Runners,
) -> Launch {
    if mac && root.extension().is_some_and(|e| e == "app") {
        return Launch::Program(Run {
            program: "/usr/bin/open".into(),
            args: vec![root.as_os_str().to_owned()],
            env: vec![],
            dir: None,
        });
    }
    let program = GAME_PROGRAM
        .split('/')
        .fold(root.to_path_buf(), |p, c| p.join(c));
    if !program.is_file() {
        return Launch::Unsupported(format!("The game program is missing ({GAME_PROGRAM})."));
    }
    let dir = program.parent().map(Path::to_path_buf);
    if windows {
        return Launch::Program(Run {
            program,
            args: vec![],
            env: vec![],
            dir,
        });
    }
    if is_steam(root, source) && runners.steam_links {
        return Launch::Steam;
    }
    match &runners.wine {
        Some(wine) => Launch::Program(Run {
            program: wine.clone(),
            args: vec![program.into_os_string()],
            env: wine_prefix(root)
                .map(|prefix| ("WINEPREFIX".to_string(), prefix.into_os_string()))
                .into_iter()
                .collect(),
            dir,
        }),
        None => Launch::Unsupported(
            "This is the Windows version of the game, and Wine is not installed, so it cannot \
             be started on this computer."
                .into(),
        ),
    }
}

pub fn how_to_launch(root: &Path, source: &str, runners: &Runners) -> Launch {
    how_to_launch_on(
        root,
        source,
        cfg!(windows),
        cfg!(target_os = "macos"),
        runners,
    )
}
