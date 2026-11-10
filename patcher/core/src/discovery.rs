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
/// Joins a `/`-separated relative path one component at a time, so every OS gets its own
/// separator in paths shown to people.
fn join(base: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(base.to_path_buf(), |p, c| p.join(c))
}

/// A canonical path as people write it. Canonical Windows paths carry the `\\?\` prefix; it is
/// removed where the plain form names the same place (a drive path under the length limit, or a
/// network share). Everywhere else the path is returned unchanged.
pub fn plain(path: PathBuf) -> PathBuf {
    if cfg!(windows)
        && let Some(text) = path.to_str()
        && let Some(plain) = plain_windows(text)
    {
        return PathBuf::from(plain);
    }
    path
}

fn plain_windows(text: &str) -> Option<String> {
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{share}"));
    }
    let rest = text.strip_prefix(r"\\?\")?;
    (rest.len() < 260 && rest.as_bytes().get(1) == Some(&b':')).then(|| rest.to_owned())
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
            steam_roots: roots.iter().map(|r| join(home, r)).collect(),
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
            direct.push(join(pf, "EA Games/The Sims 4"));
        }
        if let Some(pf) = program_files_x86 {
            direct.push(join(pf, "Origin Games/The Sims 4"));
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
            let defaults = Self::windows(
                env("ProgramFiles").as_deref(),
                env("ProgramFiles(x86)").as_deref(),
            );
            #[cfg(windows)]
            let found = defaults.with_windows_extras(
                &windows::registry_games(),
                &windows::registry_steam(),
                &windows::fixed_drives(),
            );
            #[cfg(not(windows))]
            let found = defaults;
            found
        } else {
            Self::unix(&env("HOME").unwrap_or_default(), cfg!(target_os = "macos"))
        }
    }

    /// Adds what the installers recorded in the registry, checked first, and the usual game
    /// and Steam folders on every fixed drive, checked after the defaults.
    pub fn with_windows_extras(
        mut self,
        registry_games: &[PathBuf],
        registry_steam: &[PathBuf],
        drives: &[char],
    ) -> Self {
        let mut direct = registry_games.to_vec();
        direct.append(&mut self.direct);
        let mut steam_roots = registry_steam.to_vec();
        steam_roots.append(&mut self.steam_roots);
        for drive in drives {
            let root = PathBuf::from(format!("{drive}:\\"));
            for game in [
                "Program Files/EA Games/The Sims 4",
                "EA Games/The Sims 4",
                "Program Files (x86)/Origin Games/The Sims 4",
                "Origin Games/The Sims 4",
                "Games/The Sims 4",
            ] {
                direct.push(join(&root, game));
            }
            for steam in [
                "Program Files (x86)/Steam",
                "Program Files/Steam",
                "Steam",
                "SteamLibrary",
                "Games/Steam",
            ] {
                steam_roots.push(join(&root, steam));
            }
        }
        self.direct = unique(direct);
        self.steam_roots = unique(steam_roots);
        self
    }
}

/// Drops repeats, keeping the first. Windows paths compare without regard to case or separator.
fn unique(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|p| {
            let key = if cfg!(windows) {
                p.to_string_lossy().to_lowercase().replace('/', "\\")
            } else {
                p.to_string_lossy().into_owned()
            };
            seen.insert(key)
        })
        .collect()
}

/// What Windows itself records: installer registry entries and the machine's fixed drives.
#[cfg(windows)]
mod windows {
    use std::path::PathBuf;
    use windows_sys::Win32::{
        Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
        },
    };

    const DRIVE_FIXED: u32 = 3;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    pub fn fixed_drives() -> Vec<char> {
        // SAFETY: takes no arguments and only returns a bit mask.
        let mask = unsafe { GetLogicalDrives() };
        (0u8..26)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| char::from(b'A' + i))
            .filter(|d| {
                let root = wide(&format!("{d}:\\"));
                // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call.
                unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
            })
            .collect()
    }

    fn read(root: HKEY, key: &str, value: &str) -> Option<PathBuf> {
        let (key, value) = (wide(key), wide(value));
        let mut buffer = vec![0u16; 2048];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: key and value are NUL-terminated and outlive the call; `bytes` holds the
        // buffer's size in bytes, and the call writes at most that many.
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            return None;
        }
        let chars = (bytes as usize / 2).saturating_sub(1).min(buffer.len());
        let text = String::from_utf16(&buffer[..chars]).ok()?;
        let text = text.trim().trim_end_matches(['\\', '/']).replace('/', "\\");
        (!text.is_empty()).then(|| PathBuf::from(text))
    }

    /// Where the EA app and Origin recorded the game.
    pub fn registry_games() -> Vec<PathBuf> {
        [
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Maxis\The Sims 4"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Maxis\The Sims 4"),
        ]
        .into_iter()
        .filter_map(|(root, key)| read(root, key, "Install Dir"))
        .collect()
    }

    /// Where Steam is installed; its libraryfolders.vdf lists every library on every drive.
    pub fn registry_steam() -> Vec<PathBuf> {
        [
            (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
            (
                HKEY_LOCAL_MACHINE,
                r"SOFTWARE\WOW6432Node\Valve\Steam",
                "InstallPath",
            ),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
        ]
        .into_iter()
        .filter_map(|(root, key, value)| read(root, key, value))
        .collect()
    }
}

pub fn validate(path: &Path, source: &str, variant: &str) -> Result<Candidate> {
    ensure!(
        join(path, "Data/Client/ClientFullBuild0.package").is_file(),
        "Base-game data marker not found"
    );
    ensure!(
        join(path, "Game/Bin/TS4_x64.exe").is_file(),
        "Sims 4 executable marker not found"
    );
    Ok(Candidate {
        path: plain(fs::canonicalize(path)?),
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
    let mut libraries = Vec::new();
    for root in unique(probe.steam_roots.clone()) {
        let vdf = join(&root, "steamapps/libraryfolders.vdf");
        result.checked.push(vdf.clone());
        libraries.push(root);
        if let Ok(text) = fs::read_to_string(vdf) {
            libraries.extend(library_paths(&text));
        }
    }
    // A library may be listed by several roots, and Windows spells one folder many ways.
    let mut places: Vec<(PathBuf, &str)> = unique(libraries)
        .into_iter()
        .map(|root| (join(&root, "steamapps/common/The Sims 4"), "Steam library"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_are_shown_without_the_verbatim_prefix() {
        assert_eq!(
            plain_windows(r"\\?\C:\Program Files\EA Games\The Sims 4").as_deref(),
            Some(r"C:\Program Files\EA Games\The Sims 4")
        );
        assert_eq!(
            plain_windows(r"\\?\UNC\nas\games\The Sims 4").as_deref(),
            Some(r"\\nas\games\The Sims 4")
        );
        // Too long for the plain form, or not a drive path: kept as is.
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(plain_windows(&long), None);
        assert_eq!(plain_windows(r"\\?\Volume{0}\games"), None);
        assert_eq!(plain_windows(r"C:\already\plain"), None);
    }

    #[test]
    fn windows_search_covers_the_registry_and_every_fixed_drive() {
        let pf = PathBuf::from(r"C:\Program Files");
        let pf86 = PathBuf::from(r"C:\Program Files (x86)");
        let registry_game = PathBuf::from(r"E:\Custom\The Sims 4");
        let registry_steam = PathBuf::from(r"F:\Valve\Steam");
        let probe = Probe::windows(Some(&pf), Some(&pf86)).with_windows_extras(
            std::slice::from_ref(&registry_game),
            std::slice::from_ref(&registry_steam),
            &['C', 'D'],
        );
        // What the installers recorded is checked first.
        assert_eq!(probe.direct[0], registry_game);
        assert_eq!(probe.steam_roots[0], registry_steam);
        // The defaults, then the usual folders on each drive.
        assert!(probe.direct.contains(&join(&pf, "EA Games/The Sims 4")));
        for expected in [
            join(Path::new("D:\\"), "EA Games/The Sims 4"),
            join(Path::new("D:\\"), "Program Files/EA Games/The Sims 4"),
        ] {
            assert!(probe.direct.contains(&expected), "{expected:?}");
        }
        for expected in [
            join(Path::new("D:\\"), "SteamLibrary"),
            join(Path::new("D:\\"), "Steam"),
        ] {
            assert!(probe.steam_roots.contains(&expected), "{expected:?}");
        }
        // Nothing is checked twice.
        assert_eq!(unique(probe.direct.clone()), probe.direct);
        assert_eq!(unique(probe.steam_roots.clone()), probe.steam_roots);
        assert_eq!(probe.variant, "windows-native");
    }

    #[test]
    fn a_folder_spelled_two_ways_is_checked_once_on_windows() {
        let a = PathBuf::from(r"c:\program files (x86)\steam");
        let b = PathBuf::from(r"C:\Program Files (x86)\Steam");
        let c = PathBuf::from("C:/Program Files (x86)/Steam");
        let kept = unique(vec![a.clone(), b, c]);
        if cfg!(windows) {
            assert_eq!(kept, vec![a]);
        } else {
            assert_eq!(kept.len(), 3, "other systems compare paths exactly");
        }
    }

    #[test]
    fn relative_paths_join_per_component() {
        let joined = join(Path::new("base"), "steamapps/common/The Sims 4");
        let parts: Vec<_> = joined
            .components()
            .map(|c| c.as_os_str().to_owned())
            .collect();
        assert_eq!(parts, ["base", "steamapps", "common", "The Sims 4"]);
    }
}
