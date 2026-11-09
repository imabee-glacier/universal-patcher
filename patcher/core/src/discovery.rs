use anyhow::{Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub path: PathBuf,
    pub identity: String,
    pub variant: String,
    pub source: String,
    pub maintenance_supported: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    pub checked: Vec<PathBuf>,
}
/// Extract library paths from Valve's quoted key/value format, including escaped paths.
pub fn library_paths(text: &str) -> Vec<PathBuf> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            continue;
        }
        if c == '"' {
            let mut token = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => {
                        if let Some(c) = chars.next() {
                            token.push(c)
                        }
                    }
                    _ => token.push(c),
                }
            }
            tokens.push(token);
        }
    }
    tokens
        .windows(2)
        .filter(|w| w[0] == "path")
        .map(|w| PathBuf::from(&w[1]))
        .collect()
}
/// Where to look for installations on one system, and which variant a match there is.
#[derive(Debug, Clone)]
pub struct Probe {
    /// Steam roots; each may list further libraries in `steamapps/libraryfolders.vdf`.
    pub steam_roots: Vec<PathBuf>,
    /// Game directories checked directly, such as the EA app's default location.
    pub direct: Vec<PathBuf>,
    pub variant: &'static str,
}

impl Probe {
    /// Linux (native, Flatpak and Snap Steam) and Mac Steam roots under `home`.
    pub fn unix(home: &Path, macos: bool) -> Self {
        let roots: &[&str] = if macos {
            &["Library/Application Support/Steam"]
        } else {
            &[
                ".local/share/Steam",
                ".steam/steam",
                ".steam/root",
                ".var/app/com.valvesoftware.Steam/.local/share/Steam",
                "snap/steam/common/.local/share/Steam",
            ]
        };
        Self {
            steam_roots: roots.iter().map(|r| home.join(r)).collect(),
            direct: Vec::new(),
            variant: if macos {
                "macos-native"
            } else {
                "linux-windows"
            },
        }
    }

    /// Default Steam and EA app locations under the Program Files directories.
    pub fn windows(program_files: Option<&Path>, program_files_x86: Option<&Path>) -> Self {
        let mut steam_roots = Vec::new();
        let mut direct = Vec::new();
        for pf in [program_files_x86, program_files].into_iter().flatten() {
            steam_roots.push(pf.join("Steam"));
        }
        if let Some(pf) = program_files {
            direct.push(pf.join("EA Games/The Sims 4"));
        }
        if let Some(pf) = program_files_x86 {
            direct.push(pf.join("Origin Games/The Sims 4"));
        }
        Self {
            steam_roots,
            direct,
            variant: "windows-native",
        }
    }

    pub fn this_system() -> Self {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        if cfg!(windows) {
            Self::windows(
                env("ProgramFiles").as_deref(),
                env("ProgramFiles(x86)").as_deref(),
            )
        } else {
            Self::unix(&env("HOME").unwrap_or_default(), cfg!(target_os = "macos"))
        }
    }
}

pub fn validate(path: &Path, source: &str, variant: &str) -> Result<Candidate> {
    ensure!(
        path.join("Data/Client/ClientFullBuild0.package").is_file(),
        "Base-game data marker not found"
    );
    ensure!(
        path.join("Game/Bin/TS4_x64.exe").is_file(),
        "Sims 4 executable marker not found"
    );
    Ok(Candidate {
        path: fs::canonicalize(path)?,
        identity: crate::engine::identity(path)?,
        variant: variant.into(),
        source: source.into(),
        maintenance_supported: false,
    })
}
pub fn discover(probe: &Probe, saved: Option<&Path>) -> Discovery {
    let mut result = Discovery {
        candidates: Vec::new(),
        checked: Vec::new(),
    };
    if let Some(path) = saved {
        result.checked.push(path.into());
        if let Ok(c) = validate(path, "saved installation", probe.variant) {
            result.candidates.push(c);
            return result;
        }
    }
    let mut libraries = BTreeSet::new();
    for root in &probe.steam_roots {
        libraries.insert(root.clone());
        let vdf = root.join("steamapps/libraryfolders.vdf");
        result.checked.push(vdf.clone());
        if let Ok(text) = fs::read_to_string(vdf) {
            libraries.extend(library_paths(&text));
        }
    }
    let mut places: Vec<(PathBuf, &str)> = libraries
        .into_iter()
        .map(|root| (root.join("steamapps/common/The Sims 4"), "Steam library"))
        .collect();
    places.extend(probe.direct.iter().map(|d| (d.clone(), "EA app")));
    let mut identities = BTreeSet::new();
    for (game, source) in places {
        result.checked.push(game.clone());
        if let Ok(c) = validate(&game, source, probe.variant)
            && identities.insert(c.identity.clone())
        {
            result.candidates.push(c);
        }
    }
    result
}
