//! OS adapter: network cost and scheduled maintenance. Unverified platforms report Unsupported.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Unmetered,
    Metered,
    Unknown,
}

/// Maps NetworkManager's `NMMetered` value to a network cost.
pub fn metered_from_nm(value: u32) -> Network {
    match value {
        1 | 3 => Network::Metered,
        2 | 4 => Network::Unmetered,
        _ => Network::Unknown,
    }
}

/// Current network cost. Anything that cannot be determined is Unknown.
pub fn network() -> Network {
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("busctl")
            .args([
                "get-property",
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
                "Metered",
            ])
            .output();
        let Ok(output) = output else {
            return Network::Unknown;
        };
        if !output.status.success() {
            return Network::Unknown;
        }
        // Output form: "u 4"
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .map_or(Network::Unknown, metered_from_nm)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Network::Unknown
    }
}

/// Whether an unattended run may transfer data on the current network.
pub fn unattended_allowed(network: Network, metered_opt_in: bool) -> bool {
    network == Network::Unmetered || metered_opt_in
}

const UNIT: &str = "universal-patcher-maintenance";

fn unit_dir() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").context("Home directory is unavailable")?)
            .join(".config"),
    };
    Ok(base.join("systemd/user"))
}

fn quote(path: &Path) -> Result<String> {
    let text = path.to_str().context("Path is not valid UTF-8")?;
    ensure!(
        !text.contains(['"', '\\', '\n', '%', '$']),
        "Path contains characters systemd cannot quote safely"
    );
    Ok(format!("\"{text}\""))
}

/// Unit file contents for scheduled maintenance: at login and every six hours.
pub fn systemd_units(engine: &Path, config: &Path) -> Result<(String, String)> {
    let service = format!(
        "[Unit]\n\
         Description=Universal Patcher scheduled maintenance\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={} run --scheduled --config {}\n\
         Nice=10\n\
         IOSchedulingClass=idle\n",
        quote(engine)?,
        quote(config)?
    );
    let timer = format!(
        "[Unit]\n\
         Description=Universal Patcher scheduled maintenance\n\
         \n\
         [Timer]\n\
         OnStartupSec=2min\n\
         OnUnitActiveSec=6h\n\
         RandomizedDelaySec=10min\n\
         Unit={UNIT}.service\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n"
    );
    Ok((service, timer))
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .context("systemctl is unavailable")?;
    ensure!(
        status.success(),
        "systemctl --user {} failed",
        args.join(" ")
    );
    Ok(())
}

/// Installs or removes scheduled maintenance. Idempotent.
pub fn set_schedule(enabled: bool, engine: &Path, config: &Path) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("Scheduled maintenance is unsupported: this platform's scheduler is not validated");
    }
    let dir = unit_dir()?;
    let service = dir.join(format!("{UNIT}.service"));
    let timer = dir.join(format!("{UNIT}.timer"));
    if enabled {
        ensure!(
            engine.is_file(),
            "Maintenance engine is missing from this installation"
        );
        let (service_text, timer_text) = systemd_units(engine, config)?;
        crate::storage::atomic_write(&service, service_text.as_bytes())?;
        crate::storage::atomic_write(&timer, timer_text.as_bytes())?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--now", &format!("{UNIT}.timer")])
    } else {
        if timer.exists() {
            systemctl(&["disable", "--now", &format!("{UNIT}.timer")])?;
        }
        for path in [&timer, &service] {
            match fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        systemctl(&["daemon-reload"])
    }
}
