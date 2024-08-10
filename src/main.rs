use std::path::Path;
use std::path::PathBuf;

use clap::Parser;
use itertools::Itertools;
use log::*;

use cargo_depot::Registry;

#[derive(Parser)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
pub enum MainFlags {
    Depot(Flags),
}

/// Create an maintain a simple cargo alternative registry.
#[derive(Parser)]
#[clap(about, version)]
pub struct Flags {
    /// Local path to the registry
    #[clap(long)]
    registry: PathBuf,
    /// URL of the registry, only needed for initialization
    #[clap(long)]
    url: Option<String>,
    /// Paths to crates (local workspaces or HTTP links to tarballs).
    crates: Vec<String>,
}

fn process_workspace(workspace: impl AsRef<Path>, registry: &Registry) -> anyhow::Result<()> {
    let workspace = workspace.as_ref();
    info!("Processing workspace {:?}", workspace);
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path("Cargo.toml")
        .current_dir(workspace)
        .exec()?;
    let packages = metadata
        .workspace_packages()
        .into_iter()
        .filter(|p| p.publish.as_ref().map_or(true, |v| !v.is_empty()))
        .collect_vec();
    info!(
        "Found {} packages: {}",
        packages.len(),
        packages.iter().map(|p| &p.name).join(", ")
    );
    for p in packages {
        info!("Processing {}", p.name);
        registry.add_package(p, &metadata)?;
    }
    Ok(())
}

fn main_impl() -> anyhow::Result<()> {
    let MainFlags::Depot(args) = MainFlags::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let registry = Registry::open(&args.registry, args.url.as_deref())?;

    for c in &args.crates {
        if c.starts_with("https://") || c.starts_with("http://") {
            info!("Downloading from {}", c);
            let tar = flate2::read::GzDecoder::new(ureq::get(c).call()?.into_reader());
            let mut archive = tar::Archive::new(tar);
            let output = tempfile::tempdir()?;
            archive.unpack(&output)?;
            // Find the workspace
            let Some(workspace) = std::fs::read_dir(&output)?
                .filter_map(|d| d.ok())
                .filter(|d| d.path().join("Cargo.toml").exists())
                .map(|d| d.path())
                .next()
            else {
                anyhow::bail!("Failed to find cargo workspace at the first level of the tarball");
            };
            process_workspace(workspace, &registry)?;
        } else {
            process_workspace(c, &registry)?;
        }
    }

    info!("Done");

    Ok(())
}
fn main() {
    if let Err(e) = main_impl() {
        error!("{}", e);
        std::process::exit(2);
    }
}
