use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use itertools::Itertools;
use log::*;
use serde::{Deserialize, Serialize};
use sha2::Digest;

const INDEX: &str = "index";
const CRATES: &str = "crates";

/// config.json at the root of the index
#[derive(serde::Serialize)]
pub struct IndexConfig {
    dl: String,
}
impl IndexConfig {
    pub fn from_url(url: &str) -> Self {
        Self {
            dl: format!(
                "{}/{}/{{crate}}/{{crate}}-{{version}}.crate",
                url.trim_end_matches('/'),
                CRATES,
            ),
        }
    }
    pub fn write(&self, index: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(index)?;
        Ok(std::fs::write(
            index.join("config.json"),
            serde_json::to_string_pretty(&self)?,
        )?)
    }
}

// See reg_index/src/util.rs and https://doc.rust-lang.org/cargo/reference/registry-index.html#index-files
pub fn pkg_path(name: &str) -> PathBuf {
    let name = name.to_lowercase();
    match name.len() {
        1 => PathBuf::from("1"),
        2 => PathBuf::from("2"),
        3 => Path::new("3").join(&name[..1]),
        _ => Path::new(&name[0..2]).join(&name[2..4]),
    }
}

#[derive(Serialize, Deserialize)]
struct Dependency {
    name: String,
    req: cargo_metadata::semver::VersionReq,
    features: Vec<String>,
    optional: bool,
    default_features: bool,
    target: Option<cargo_platform::Platform>,
    kind: cargo_metadata::DependencyKind,
    registry: Option<String>,
    package: Option<cargo_metadata::camino::Utf8PathBuf>,
}
impl From<cargo_metadata::Dependency> for Dependency {
    fn from(s: cargo_metadata::Dependency) -> Self {
        Self {
            name: s.name,
            req: s.req,
            features: s.features,
            optional: s.optional,
            default_features: s.uses_default_features,
            target: s.target,
            kind: s.kind,
            // Note source -> registry
            registry: s.source.clone(),
            package: None,
        }
    }
}

fn check_dirty(repository: &Path) -> anyhow::Result<()> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repository)
        .output()?;
    if !out.status.success() {
        // Likely not a git repository
        return Ok(());
    }
    let out = String::from_utf8_lossy(&out.stdout);
    let out = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        // This gets filtered by cargo package anyway
        .filter(|l| !l.contains("Cargo.lock"))
        .collect_vec();
    anyhow::ensure!(out.is_empty(), "Repository not clean: {}. These files would be embedded in the package. Stash them with `git stash -u` or add them to gitignore", out.join(" "));
    Ok(())
}

// https://doc.rust-lang.org/cargo/reference/registry-index.html#json-schema
#[derive(Serialize, Deserialize)]
pub struct IndexMeta {
    name: String,
    vers: cargo_metadata::semver::Version,
    deps: Vec<Dependency>,
    features: BTreeMap<String, Vec<String>>,
    license: Option<String>,
    license_file: Option<cargo_metadata::camino::Utf8PathBuf>,
    cksum: String,
    v: u8,
    yanked: bool,
}
impl IndexMeta {
    pub fn from_package(p: &cargo_metadata::Package, checksum: String) -> Self {
        // TODO: Handle rename?
        let mut deps: Vec<Dependency> = vec![];
        for dep_meta in &p.dependencies {
            let mut dep = Dependency::from(dep_meta.clone());
            if dep.registry.as_ref().map_or(false, |s| {
                s != "registry+https://github.com/rust-lang/crates.io-index"
            }) || dep_meta.path.is_some()
            {
                // Use our registry when the package is a path, a git repository, or another
                // registry.
                dep.registry = None;
            }
            deps.push(dep);
        }
        Self {
            deps,
            name: p.name.clone(),
            vers: p.version.clone(),
            features: p.features.clone(),
            license: p.license.clone(),
            license_file: p.license_file.clone(),
            cksum: checksum,
            v: 2,
            yanked: false,
        }
    }
}

pub struct Registry(pub PathBuf);
impl Registry {
    pub fn package_index(&self, name: &str) -> PathBuf {
        self.0.join(INDEX).join(pkg_path(name)).join(name)
    }
    pub fn read_package(&self, name: &str) -> anyhow::Result<Vec<IndexMeta>> {
        let filename = self.package_index(name);
        if !filename.exists() {
            return Ok(vec![]);
        }
        let mut res = vec![];
        for line in std::fs::read_to_string(&filename)?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            res.push(serde_json::from_str(line)?);
        }
        Ok(res)
    }
    pub fn add_package(
        &self,
        p: &cargo_metadata::Package,
        workspace_metadata: &cargo_metadata::Metadata,
    ) -> anyhow::Result<()> {
        if !p
            .targets
            .iter()
            .any(|t| t.is_lib() || t.kind.contains(&"proc-macro".into()))
        {
            warn!("Skipping non-library package");
            return Ok(());
        }
        // Check if already in the index
        if self
            .read_package(&p.name)?
            .into_iter()
            .any(|p_index| p_index.vers == p.version)
        {
            warn!("Package already in the index, skipping");
            return Ok(());
        }

        check_dirty(workspace_metadata.workspace_root.as_std_path())?;
        // Edit manifest
        info!("Editing manifest");
        let manifest = std::fs::read_to_string(&p.manifest_path)?;
        let mut manifest: cargo_util_schemas::manifest::TomlManifest = toml::from_str(&manifest)?;
        if let Some(package) = &mut manifest.package {
            package.autoexamples = Some(false);
        }
        manifest.bin = None;
        let manifest_orig = p.manifest_path.with_extension("toml.pre-edit");
        std::fs::rename(&p.manifest_path, &manifest_orig)?;
        std::fs::write(&p.manifest_path, toml::to_string_pretty(&manifest)?)?;

        info!("Building package");
        let parent = self.0.join(CRATES).join(&p.name);
        std::fs::create_dir_all(&parent)?;
        // Do not use .with_extension due to the . in the name.
        let crate_dest = parent.join(format!("{}-{}.crate", p.name, p.version));

        let out = std::process::Command::new("cargo")
            .args([
                "package",
                "-p",
                &p.name,
                "--no-verify",
                "--all-features",
                "--allow-dirty",
            ])
            .current_dir(p.manifest_path.parent().unwrap())
            .spawn()?
            .wait()?;
        std::fs::rename(manifest_orig, &p.manifest_path)?;
        anyhow::ensure!(out.success(), "Failed to build package");
        // Hash .crate
        let crate_src = workspace_metadata
            .target_directory
            .as_std_path()
            .join("package")
            .join(crate_dest.file_name().unwrap());
        let mut hasher = sha2::Sha256::new();
        let mut file = std::fs::File::open(&crate_src)?;
        std::io::copy(&mut file, &mut hasher)?;
        let hash = format!("{:x}", hasher.finalize());
        // Copy .crate
        anyhow::ensure!(!crate_dest.exists(), "{:?} already exists", crate_dest);
        std::fs::copy(crate_src, crate_dest)?;

        // Compute metadata
        let metadata = IndexMeta::from_package(p, hash);

        // Write to index
        let index = self.package_index(&p.name);
        std::fs::create_dir_all(index.parent().unwrap())?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(index)?;
        writeln!(f, "{}", serde_json::to_string(&metadata)?)?;
        Ok(())
    }
    pub fn open(root: &Path, url: Option<&str>) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root)?;

        let index = root.join(INDEX);
        if !index.join("config.json").exists() {
            info!("Initializing registry at {:?}", root);
            let Some(url) = &url else {
                anyhow::bail!(
                    "Provide the URL where the registry will be hosted with the --url flag"
                );
            };
            IndexConfig::from_url(url).write(&index)?;
        }
        Ok(Self(root.into()))
    }
}
