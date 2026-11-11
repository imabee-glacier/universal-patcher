//! What runs beside the patcher: whether The Sims 4, its launcher, or the EA app is running,
//! and how the patcher asks the system to open a link or start a program. Everything the
//! interface does with other programs goes through [`Host`], so tests can replace it.
use crate::local::Run;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Programs that are the game itself.
const GAME: [&str; 2] = ["ts4_x64.exe", "ts4_dx9_x64.exe"];
/// The game's own launcher, which starts the game and then exits.
const LAUNCHER: [&str; 1] = ["ts4_launcher_x64.exe"];
/// The EA app and its service, which install and update the game.
const INSTALLER: [&str; 2] = ["eadesktop.exe", "eabackgroundservice.exe"];

/// Which game-related programs are running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    pub game: bool,
    pub launcher: bool,
    pub installer: bool,
}

impl Activity {
    /// True when anything that may write game files is running.
    pub fn busy(self) -> bool {
        self.game || self.launcher || self.installer
    }
}

/// The result of looking for running programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    Seen(Activity),
    /// This system cannot be checked; the reason is shown to the user.
    Unsupported(String),
}

/// The file name at the end of a Unix or Windows path, lowercased.
fn base_name(arg: &str) -> String {
    arg.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

/// Classifies program names or command-line arguments. Only whole file names count, so a
/// program called `notts4_x64.exe` is not the game.
pub fn classify<'a>(names: impl IntoIterator<Item = &'a str>) -> Activity {
    let mut a = Activity::default();
    for name in names {
        let name = base_name(name);
        a.game |= GAME.contains(&name.as_str());
        a.launcher |= LAUNCHER.contains(&name.as_str());
        a.installer |= INSTALLER.contains(&name.as_str());
    }
    a
}

/// Looks at the programs running now.
pub fn detect() -> Detection {
    match running_names() {
        Ok(names) => Detection::Seen(classify(names.iter().map(String::as_str))),
        Err(reason) => Detection::Unsupported(reason),
    }
}

/// On Linux the game runs under Proton or Wine, whose processes carry the Windows program path
/// in their command line, and its name in `comm`.
#[cfg(target_os = "linux")]
fn running_names() -> Result<Vec<String>, String> {
    let entries =
        std::fs::read_dir("/proc").map_err(|e| format!("Cannot list running programs: {e}"))?;
    let mut names = Vec::new();
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let path = entry.path();
        if let Ok(cmd) = std::fs::read(path.join("cmdline")) {
            names.extend(
                String::from_utf8_lossy(&cmd)
                    .split('\0')
                    .filter(|a| !a.is_empty())
                    .map(String::from),
            );
        }
        if let Ok(comm) = std::fs::read_to_string(path.join("comm")) {
            names.push(comm.trim().to_string());
        }
    }
    Ok(names)
}

#[cfg(windows)]
fn running_names() -> Result<Vec<String>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    // SAFETY: takes plain values and returns a handle we close below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Cannot list running programs: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut names = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `snapshot` is a valid snapshot handle and `entry` a sized PROCESSENTRY32W.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        names.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the handle came from CreateToolhelp32Snapshot and is closed once.
    unsafe { CloseHandle(snapshot) };
    Ok(names)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn running_names() -> Result<Vec<String>, String> {
    Err("Detecting a running game is not verified on this system yet".into())
}

/// Everything the interface does with other programs.
pub trait Host: Send + Sync {
    /// Which game programs are running.
    fn detect(&self) -> Detection;
    /// Whether a program is registered to open `scheme:` links; the error is shown as is.
    fn handler(&self, scheme: &str) -> Result<(), String>;
    /// Opens a link with the program registered for it, without waiting for that program.
    fn open(&self, url: &str) -> Result<()>;
    /// Starts a program without waiting for it.
    fn start(&self, run: &Run) -> Result<()>;
    /// The `wine` program, when one is installed.
    fn wine(&self) -> Option<PathBuf>;
    /// Downloads a newer patcher for the next start; returns its version.
    fn stage_app_update(&self) -> Result<Option<String>>;
}

/// The real system. `PATCHER_URL_OPENER` names a program that receives links instead of the
/// system's opener, for trying the interface without starting Steam.
pub struct SystemHost;

fn spawn_detached(mut command: std::process::Command) -> Result<()> {
    crate::app::no_window(&mut command);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn()?;
    // Reap it when it exits so it does not linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

impl Host for SystemHost {
    fn detect(&self) -> Detection {
        detect()
    }

    fn handler(&self, scheme: &str) -> Result<(), String> {
        if std::env::var_os("PATCHER_URL_OPENER").is_some() {
            return Ok(());
        }
        handler(scheme)
    }

    fn open(&self, url: &str) -> Result<()> {
        let command = match std::env::var_os("PATCHER_URL_OPENER") {
            Some(program) => {
                let mut c = std::process::Command::new(program);
                c.arg(url);
                c
            }
            None => opener(url)?,
        };
        spawn_detached(command).context("Could not open the link")
    }

    fn start(&self, run: &Run) -> Result<()> {
        let mut command = std::process::Command::new(&run.program);
        command.args(&run.args).envs(run.env.iter().cloned());
        if let Some(dir) = &run.dir {
            command.current_dir(dir);
        }
        spawn_detached(command).context("Could not start the game")
    }

    fn wine(&self) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join("wine"))
            .find(|candidate| candidate.is_file())
    }

    fn stage_app_update(&self) -> Result<Option<String>> {
        crate::app::stage_update()
    }
}

#[cfg(target_os = "linux")]
fn opener(url: &str) -> Result<std::process::Command> {
    let mut c = std::process::Command::new("xdg-open");
    c.arg(url);
    Ok(c)
}

#[cfg(windows)]
fn opener(url: &str) -> Result<std::process::Command> {
    let mut c = std::process::Command::new("cmd.exe");
    crate::app::no_window(&mut c);
    c.args(["/d", "/c", "start", "", url]);
    Ok(c)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn opener(_url: &str) -> Result<std::process::Command> {
    anyhow::bail!("Opening links is not verified on this system yet")
}

/// Asks the desktop which program opens `scheme:` links.
#[cfg(target_os = "linux")]
fn handler(scheme: &str) -> Result<(), String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", &format!("x-scheme-handler/{scheme}")])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|_| {
            "Cannot tell which program opens Steam links, because xdg-mime is not installed"
                .to_string()
        })?;
    if output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        Ok(())
    } else {
        Err(
            "Steam is not set up to open Steam links on this computer. Start Steam once, then \
             try again"
                .into(),
        )
    }
}

/// Steam registers its links under `HKEY_CLASSES_ROOT\steam`.
#[cfg(windows)]
fn handler(scheme: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = format!(r"{scheme}\Shell\Open\Command")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut bytes = 0u32;
    // SAFETY: `key` is NUL-terminated and outlives the call; a null value name reads the
    // default value, and a null buffer only asks for its size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            key.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(
            "Steam is not set up to open Steam links on this computer. Start Steam once, then \
             try again"
                .into(),
        )
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn handler(_scheme: &str) -> Result<(), String> {
    Err("Starting the game from the patcher is not verified on this system yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_matched_by_whole_file_name() {
        let a = classify([
            r"Z:\home\me\.steam\steamapps\common\The Sims 4\Game\Bin\TS4_x64.exe",
            "/usr/bin/bash",
        ]);
        assert_eq!(
            a,
            Activity {
                game: true,
                launcher: false,
                installer: false
            }
        );
        assert!(!classify(["/tmp/notts4_x64.exe", "TS4_x64.exe.bak"]).busy());
        assert!(classify(["C:/Program Files/Electronic Arts/EA Desktop/EADesktop.exe"]).installer);
        assert!(classify(["TS4_Launcher_x64.exe"]).launcher);
        assert!(classify(["ts4_dx9_x64.exe"]).game);
    }
}
