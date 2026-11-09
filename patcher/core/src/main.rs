use anyhow::Result;
use clap::{Parser, Subcommand};
use patcher_core::{
    app,
    engine::{self, Boundary, RunOptions},
    fixture::{self, Config},
    model::Manifest,
    publisher,
    storage::Policy,
    transfer::{Control, HttpSource, LocalSource, ObjectSource},
    trust,
};
use std::{fs, path::PathBuf};
#[derive(Parser)]
#[command(version, about = "Verified file maintenance and release preparation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List and classify every entry of a source archive without extracting it.
    Inventory {
        archive: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Encode and verify objects for a release from an archive and a recipe.
    Prepare {
        archive: PathBuf,
        #[arg(long)]
        recipe: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Check a release manifest against schema version 1.
    Validate { manifest: PathBuf },
    /// Sign a release and promote it into a repository; timestamp.json is replaced last.
    Publish {
        manifest: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long = "key", required = true)]
        keys: Vec<PathBuf>,
        #[arg(long)]
        repository: PathBuf,
    },
    /// Re-sign the currently promoted release before its metadata expires.
    Renew {
        #[arg(long)]
        root: PathBuf,
        #[arg(long = "key", required = true)]
        keys: Vec<PathBuf>,
        #[arg(long)]
        repository: PathBuf,
    },
    /// Exit with failure if live metadata expires within the given number of hours.
    Expiry {
        #[arg(long)]
        repository: PathBuf,
        #[arg(long, default_value_t = 12)]
        within_hours: u64,
    },
    /// Report detected installations and every location checked.
    Discover,
    /// Install the latest release for this user (used by install.sh and install.ps1).
    Install {
        /// Download origin; defaults to the one built into this engine.
        #[arg(long)]
        origin: Option<String>,
        /// Trusted application root; defaults to the one built into this engine.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Download a newer release; it becomes active the next time the patcher starts.
    Update {
        #[arg(long)]
        origin: Option<String>,
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Start the patcher, activating a downloaded update first when it is safe to do so.
    Launch {
        /// Arguments passed to the interface.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Remove the patcher. Game choices and recovery records are kept unless --remove-data.
    Uninstall {
        #[arg(long)]
        remove_data: bool,
    },
    /// Package one platform build for publishing.
    PackageApp {
        bundle: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        os: String,
        #[arg(long)]
        arch: String,
        #[arg(long)]
        staging: PathBuf,
    },
    /// Sign and publish every packaged build in a staging directory, with the install scripts.
    PublishApp {
        staging: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long = "key", required = true)]
        keys: Vec<PathBuf>,
        #[arg(long)]
        repository: PathBuf,
        /// Public origin written into install.sh and install.ps1, ending in /.
        #[arg(long)]
        origin: String,
        /// Platform offered on the website as a verified download, for example linux-x64.
        /// Repeat for each platform that passed its checks; others are listed as not verified.
        #[arg(long = "offer")]
        offered: Vec<String>,
        /// Release notes file; paragraphs are separated by blank lines.
        #[arg(long)]
        notes: Option<PathBuf>,
        /// Windows graphical installer built for this version.
        #[arg(long)]
        windows_setup: Option<PathBuf>,
    },
    /// Probe the public origin and write the website's health status.
    Health {
        /// Public origin, ending in /.
        #[arg(long)]
        origin: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Create a signed synthetic test release, game and configuration.
    Fixture { directory: PathBuf },
    /// Maintain the configured installation once.
    Run {
        #[arg(long)]
        config: PathBuf,
        /// Unattended run: skipped unless background maintenance is enabled and the network allows it.
        #[arg(long)]
        scheduled: bool,
        /// Hash every selected file instead of trusting cached hashes.
        #[arg(long)]
        verify: bool,
        #[arg(long, hide = true)]
        interrupt_after_rename: Option<usize>,
    },
}
fn main() {
    if let Err(e) = execute() {
        eprintln!("{e:#}");
        std::process::exit(1)
    }
}
/// Unattended runs also fetch patcher updates; failures here never block game maintenance.
fn stage_app_update() {
    let Ok(layout) = app::Layout::for_user() else {
        return;
    };
    if !layout.launcher().is_file() {
        return;
    }
    let result = app::Source::configured(None, None)
        .and_then(|source| app::update(&source, &layout, &Control::default()));
    match result {
        Ok(Some(v)) => println!("Universal Patcher {v} is ready; it starts next time."),
        Ok(None) => {}
        Err(e) => eprintln!("Patcher update check failed: {e:#}"),
    }
}
fn print_report(report: engine::RunReport) {
    match serde_json::to_string(&report) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("Cannot render run report: {e}"),
    }
}
fn execute() -> Result<()> {
    match Cli::parse().command {
        Command::Inventory { archive, output } => {
            let audit = publisher::inventory(&archive)?;
            patcher_core::storage::atomic_write(&output, &serde_json::to_vec_pretty(&audit)?)?;
            println!(
                "{} files inventoried; {} pack directories; production readiness is unverified.",
                audit.files.len(),
                audit.pack_ids.len()
            );
        }
        Command::Prepare {
            archive,
            recipe,
            output,
        } => {
            let m = publisher::prepare(&archive, &recipe, &output)?;
            println!(
                "Prepared {} file objects; release remains unsigned.",
                m.files.len()
            );
        }
        Command::Validate { manifest } => {
            let m = Manifest::parse(&fs::read(manifest)?)?;
            println!(
                "Valid schema: {} files, {} groups. \
                 Compatibility is not established by schema validation.",
                m.files.len(),
                m.groups.len()
            );
        }
        Command::Publish {
            manifest,
            root,
            keys,
            repository,
        } => {
            let version = tokio::runtime::Runtime::new()?.block_on(publisher::publish(
                &root,
                keys,
                &manifest,
                &repository,
            ))?;
            println!("Promoted metadata version {version}");
        }
        Command::Renew {
            root,
            keys,
            repository,
        } => {
            let version = tokio::runtime::Runtime::new()?.block_on(publisher::renew(
                &root,
                keys,
                &repository,
            ))?;
            println!("Renewed metadata as version {version}");
        }
        Command::Expiry {
            repository,
            within_hours,
        } => {
            let limit = jiff::Timestamp::now()
                + std::time::Duration::from_secs(within_hours.saturating_mul(3600));
            let mut expiring = Vec::new();
            for (role, expires) in publisher::expiry(&repository)? {
                println!("{role} expires {expires}");
                if expires <= limit {
                    expiring.push(role);
                }
            }
            anyhow::ensure!(
                expiring.is_empty(),
                "Metadata expires within {within_hours} hours: {}",
                expiring.join(", ")
            );
        }
        Command::Install { origin, root } => {
            let source = app::Source::configured(origin.as_deref(), root.as_deref())?;
            let layout = app::Layout::for_user()?;
            let (version, notes) = app::install(&source, &layout)?;
            println!("Universal Patcher {version} is installed.");
            println!("Start it from your applications menu, or run: universal-patcher launch");
            for note in notes {
                println!("{note}");
            }
        }
        Command::Update { origin, root } => {
            let source = app::Source::configured(origin.as_deref(), root.as_deref())?;
            match app::update(&source, &app::Layout::for_user()?, &Control::default())? {
                Some(v) => println!("Universal Patcher {v} is ready; it starts next time."),
                None => println!("Universal Patcher is up to date."),
            }
        }
        Command::Launch { args } => {
            match app::launch(&app::Layout::for_user()?, &args, app::STARTUP_CHECK)? {
                app::Launched::Started(_) => {}
                app::Launched::RolledBack { failed, running } => eprintln!(
                    "Universal Patcher {failed} did not start correctly, so {running} was started \
                     instead. {failed} will not be tried again."
                ),
            }
        }
        Command::Uninstall { remove_data } => {
            let layout = app::Layout::for_user()?;
            if let Err(e) =
                patcher_core::os::set_schedule(false, &layout.launcher(), &layout.config())
            {
                eprintln!("Background maintenance could not be removed: {e:#}");
            }
            app::uninstall(&layout, remove_data)?;
            if remove_data {
                println!("Universal Patcher and its saved choices were removed.");
            } else {
                println!(
                    "Universal Patcher was removed. Your choices are kept in {}; \
                     run again with --remove-data to delete them.",
                    layout.root.display()
                );
            }
        }
        Command::PackageApp {
            bundle,
            version,
            os,
            arch,
            staging,
        } => {
            let m =
                patcher_core::distribution::package_app(&bundle, &version, &os, &arch, &staging)?;
            println!(
                "Packaged {} {}-{}: {} bytes to download.",
                m.app_version, m.os, m.arch, m.artifact.encoded_size
            );
        }
        Command::PublishApp {
            staging,
            root,
            keys,
            repository,
            origin,
            offered,
            notes,
            windows_setup,
        } => {
            let notes = match notes {
                Some(path) => fs::read_to_string(path)?
                    .split("\n\n")
                    .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|p| !p.is_empty())
                    .collect(),
                None => Vec::new(),
            };
            let listing = patcher_core::distribution::Listing {
                offered,
                notes,
                windows_setup,
            };
            let (version, metadata) = tokio::runtime::Runtime::new()?.block_on(
                patcher_core::distribution::publish_app(
                    &root,
                    keys,
                    &staging,
                    &repository,
                    &origin,
                    &listing,
                ),
            )?;
            println!("Published Universal Patcher {version} as metadata version {metadata}.");
        }
        Command::Health { origin, output } => {
            let status = tokio::runtime::Runtime::new()?
                .block_on(patcher_core::health::write(&origin, &output))?;
            println!("Service status: {status}");
            anyhow::ensure!(status == "operational", "Service is {status}");
        }
        Command::Discover => {
            let probe = patcher_core::discovery::Probe::this_system();
            let found = patcher_core::discovery::discover(&probe, None);
            println!("{}", serde_json::to_string_pretty(&found)?);
        }
        Command::Fixture { directory } => {
            tokio::runtime::Runtime::new()?.block_on(fixture::create(&directory))?;
            println!(
                "Created signed synthetic fixture at {}",
                directory.display()
            );
        }
        Command::Run {
            config,
            scheduled,
            verify,
            interrupt_after_rename,
        } => {
            let cfg: Config =
                serde_json::from_value(patcher_core::strict_json::parse(&fs::read(config)?)?)?;
            // The same session lock the interface holds, so policy edits never interleave.
            let _session = patcher_core::storage::Lock::acquire(&cfg.state.join("session.lock"))?;
            let mut policy = Policy::load(&cfg.state)?;
            if scheduled {
                if !policy.preferences.background {
                    println!("Background maintenance is off; nothing to do.");
                    return Ok(());
                }
                if policy.suspended.is_some() {
                    println!("Maintenance is paused or stopped by the user; nothing to do.");
                    return Ok(());
                }
                let network = patcher_core::os::network();
                if !patcher_core::os::unattended_allowed(network, policy.preferences.metered) {
                    println!(
                        "Deferred: unattended transfers are not allowed on a {network:?} network."
                    );
                    return Ok(());
                }
            }
            if scheduled {
                stage_app_update();
            }
            let release = tokio::runtime::Runtime::new()?.block_on(trust::load(
                &cfg.trusted_root,
                cfg.metadata_url.parse()?,
                cfg.targets_url.parse()?,
                &cfg.target,
                &cfg.state.join("tuf"),
            ))?;
            if policy.adopt_new_packs(release.manifest()) {
                policy.save(&cfg.state)?;
            }
            let report = |p: engine::Progress| match serde_json::to_string(&p) {
                Ok(line) => println!("{line}"),
                Err(e) => eprintln!("Cannot render progress: {e}"),
            };
            let opts = RunOptions {
                interrupt: interrupt_after_rename.map(|n| (Boundary::AfterRename, n)),
                full_verify: verify,
            };
            let source: Box<dyn ObjectSource> = match cfg.local_objects {
                Some(directory) => {
                    anyhow::ensure!(
                        release.manifest().variant == "fixture",
                        "Local object source is only for fixtures"
                    );
                    Box::new(LocalSource { directory })
                }
                None => Box::new(HttpSource::new(
                    cfg.origin.parse()?,
                    policy.preferences.bandwidth_bytes_per_second,
                )),
            };
            let result = engine::run(
                &release,
                &mut policy,
                &cfg.state,
                source.as_ref(),
                &Control::default(),
                &report,
                &opts,
            )?;
            print_report(result);
        }
    }
    Ok(())
}
