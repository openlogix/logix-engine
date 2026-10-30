//! Package install and repair (docs/tool-api.md, "Packages").
//!
//! Engine folder layout:
//!
//! - `packages/<name>/<version>/`: an installed package, with its install
//!   record `logix-install.json`. Several versions can sit side by side.
//! - `packages/<name>/.tmp-<version>-<pid>/`: a package being unpacked. It is
//!   renamed to `<version>` only when complete, so a crash never leaves a
//!   package that looks installed.
//! - `downloads/`: partial downloads, deleted when done.
//! - `locks/<name>.lock`: stops two engine processes from installing the same
//!   package at once.

pub mod manifest;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::errors::{ActionCode, ToolError};
use crate::progress::Phases;
use crate::tools::get_engine_info::PackageStatus;
use manifest::{Manifest, PackageFile, PackageSpec};

/// The install record, written into a package folder when it is complete.
pub const RECORD_FILE: &str = "logix-install.json";

/// Share of an install's progress taken by the download. Unpacking takes
/// the rest.
const DOWNLOAD_SHARE: f64 = 80.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallRecord {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub file: String,
    pub sha256: String,
    pub engine_version: String,
}

/// Installs, repairs and finds the packages of a manifest in an engine folder.
#[derive(Debug, Clone)]
pub struct PackageManager {
    /// The engine folder, with symbolic links resolved.
    engine_folder: PathBuf,
    manifest: Arc<Manifest>,
    platform: String,
    /// Where package files are downloaded from (manifest::GITHUB, or a test
    /// server).
    download_base: String,
}

fn install_error(name: &str, problem: impl std::fmt::Display) -> ToolError {
    ToolError::setup(format!("Can't install the {name} package: {problem}"))
        .with_action(
            ActionCode::InstallPackages,
            format!("Install the {name} package, then retry. Check the network connection and the free disk space."),
        )
}

impl PackageManager {
    pub fn new(
        engine_folder: &Path,
        manifest: Arc<Manifest>,
        platform: &str,
        download_base: &str,
    ) -> Self {
        PackageManager {
            engine_folder: engine_folder.to_path_buf(),
            manifest,
            platform: platform.to_string(),
            download_base: download_base.to_string(),
        }
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn packages_dir(&self, name: &str) -> PathBuf {
        self.engine_folder.join("packages").join(name)
    }

    /// The folder of an installed package version.
    pub fn package_dir(&self, spec: &PackageSpec) -> PathBuf {
        self.packages_dir(&spec.name).join(&spec.version)
    }

    fn file_for<'a>(&self, spec: &'a PackageSpec) -> Option<&'a PackageFile> {
        spec.files.get(&self.platform)
    }

    /// A package's status, from its folder and install record. Fast: it
    /// doesn't verify checksums.
    pub fn status(&self, spec: &PackageSpec) -> PackageStatus {
        let dir = self.package_dir(spec);
        if !dir.exists() {
            return PackageStatus::Missing;
        }
        let record: Option<InstallRecord> = fs::read_to_string(dir.join(RECORD_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        match (record, self.file_for(spec)) {
            (Some(r), Some(f))
                if r.name == spec.name && r.version == spec.version && r.sha256 == f.sha256 =>
            {
                PackageStatus::Installed
            }
            _ => PackageStatus::Broken,
        }
    }

    /// The packages, by name, that aren't installed and intact.
    pub fn needing_install(&self, names: &[&str]) -> Result<Vec<PackageSpec>, ToolError> {
        let mut out = Vec::new();
        for name in names {
            let spec = self.manifest.get(name).ok_or_else(|| {
                ToolError::engine(format!("Package {name} isn't in the manifest."))
            })?;
            if self.status(spec) != PackageStatus::Installed {
                out.push(spec.clone());
            }
        }
        Ok(out)
    }

    /// The error for packages that need installing when auto-install is off.
    pub fn not_installed_error(&self, specs: &[PackageSpec]) -> ToolError {
        let list: Vec<String> = specs
            .iter()
            .map(|s| {
                let status = match self.status(s) {
                    PackageStatus::Broken => "broken",
                    _ => "missing",
                };
                format!("{} {} ({status})", s.name, s.version)
            })
            .collect();
        ToolError::setup(format!(
            "Required packages aren't installed: {}.",
            list.join(", ")
        ))
        .with_action(
            ActionCode::InstallPackages,
            "Install the required packages, then retry. Automatic install is turned off.",
        )
    }

    /// Installs or repairs a package. `phases` must be in the package's
    /// install phase: this reports its percentage. Waits while another
    /// process installs the same package, and does nothing if that leaves it
    /// installed.
    pub async fn install(
        &self,
        spec: &PackageSpec,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        let name = spec.name.as_str();
        let file = self.file_for(spec).ok_or_else(|| {
            ToolError::engine(format!(
                "The {name} package isn't available for {}.",
                self.platform
            ))
        })?;
        for dir in ["packages", "downloads", "locks"] {
            fs::create_dir_all(self.engine_folder.join(dir))
                .map_err(|e| install_error(name, format!("can't create the {dir} folder: {e}")))?;
        }
        let pkg_dir = self.packages_dir(name);
        fs::create_dir_all(&pkg_dir).map_err(|e| install_error(name, e))?;

        let _lock = self.lock(name, cancel).await?;
        if self.status(spec) == PackageStatus::Installed {
            log::info!(
                "Package {name} {} was installed by another process",
                spec.version
            );
            return Ok(());
        }
        log::info!(
            "Installing package {name} {} from {}",
            spec.version,
            file.url(&self.download_base)
        );

        let final_dir = self.package_dir(spec);
        let tmp_dir = pkg_dir.join(format!(".tmp-{}-{}", spec.version, std::process::id()));
        let part = self
            .engine_folder
            .join("downloads")
            .join(format!("{}.part", file.file));
        remove_stale(&pkg_dir, &final_dir);
        let _cleanup = Cleanup(vec![tmp_dir.clone(), part.clone()]);

        self.download(file, &part, name, phases, cancel).await?;

        let (tmp, archive) = (tmp_dir.clone(), part.clone());
        let (cancel2, total) = (cancel.clone(), file.size);
        let progress = Arc::new(AtomicU64::new(0));
        let reporter = progress.clone();
        let unpack =
            tokio::task::spawn_blocking(move || unpack(&archive, &tmp, &cancel2, &reporter));
        // Report unpack progress while it runs.
        let mut unpack = unpack;
        let result = loop {
            tokio::select! {
                r = &mut unpack => break r,
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    let done = progress.load(Ordering::Relaxed) as f64;
                    phases.percent(DOWNLOAD_SHARE + (100.0 - DOWNLOAD_SHARE) * done / total as f64);
                }
            }
        };
        match result {
            Ok(Ok(())) => {}
            Ok(Err(UnpackError::Cancelled)) => {
                return Err(ToolError::cancelled(format!(
                    "Installing {name} was cancelled."
                )));
            }
            Ok(Err(UnpackError::Failed(e))) => {
                return Err(install_error(name, format!("can't unpack it: {e}")));
            }
            Err(e) => return Err(ToolError::engine(format!("Unpacking {name} failed: {e}"))),
        }

        let record = InstallRecord {
            name: spec.name.clone(),
            version: spec.version.clone(),
            platform: self.platform.clone(),
            file: file.file.clone(),
            sha256: file.sha256.clone(),
            engine_version: crate::state::ENGINE_VERSION.to_string(),
        };
        let json = serde_json::to_string_pretty(&record).unwrap_or_default() + "\n";
        fs::write(tmp_dir.join(RECORD_FILE), json).map_err(|e| install_error(name, e))?;
        if final_dir.exists() {
            // A broken install, replaced now that the new one is complete.
            fs::remove_dir_all(&final_dir).map_err(|e| {
                install_error(name, format!("can't remove the broken install: {e}"))
            })?;
        }
        fs::rename(&tmp_dir, &final_dir).map_err(|e| install_error(name, e))?;
        log::info!(
            "Installed package {name} {} in {}",
            spec.version,
            final_dir.display()
        );
        Ok(())
    }

    /// Takes the package's install lock, waiting while another process holds
    /// it. The lock is released when the returned file is dropped.
    async fn lock(&self, name: &str, cancel: &CancellationToken) -> Result<fs::File, ToolError> {
        let path = self
            .engine_folder
            .join("locks")
            .join(format!("{name}.lock"));
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| install_error(name, format!("can't open the lock file: {e}")))?;
        let mut logged = false;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(fs::TryLockError::WouldBlock) => {
                    if !logged {
                        log::info!("Waiting for another engine process installing {name}");
                        logged = true;
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            return Err(ToolError::cancelled(format!("Installing {name} was cancelled.")));
                        }
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                }
                Err(fs::TryLockError::Error(e)) => {
                    return Err(install_error(name, format!("can't lock: {e}")));
                }
            }
        }
    }

    /// Downloads a package file to `part`, checking its size and checksum.
    async fn download(
        &self,
        file: &PackageFile,
        part: &Path,
        name: &str,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        let url = file.url(&self.download_base);
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .user_agent(format!("logix-engine/{}", crate::state::ENGINE_VERSION))
            .build()
            .map_err(|e| ToolError::engine(format!("Can't start downloads: {e}")))?;
        let request = async {
            let response = client.get(&url).send().await?.error_for_status()?;
            Ok::<_, reqwest::Error>(response)
        };
        let mut response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(ToolError::cancelled(format!("Installing {name} was cancelled.")));
            }
            r = request => r.map_err(|e| {
                install_error(name, "the download failed").with_details(format!("{url}\n{e}"))
            })?,
        };
        let mut out = fs::File::create(part).map_err(|e| install_error(name, e))?;
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(ToolError::cancelled(format!("Installing {name} was cancelled.")));
                }
                c = response.chunk() => c.map_err(|e| {
                    install_error(name, "the download was interrupted").with_details(format!("{url}\n{e}"))
                })?,
            };
            let Some(chunk) = chunk else { break };
            received += chunk.len() as u64;
            if received > file.size {
                return Err(install_error(name, "the download is larger than expected")
                    .with_details(url.clone()));
            }
            hasher.update(&chunk);
            out.write_all(&chunk).map_err(|e| install_error(name, e))?;
            phases.percent(DOWNLOAD_SHARE * received as f64 / file.size as f64);
        }
        out.flush().map_err(|e| install_error(name, e))?;
        if received != file.size {
            return Err(install_error(
                name,
                format!("the download has {received} bytes, expected {}", file.size),
            )
            .with_details(url));
        }
        let digest = hex::encode(hasher.finalize());
        if digest != file.sha256 {
            return Err(
                install_error(name, "the downloaded file failed its checksum check").with_details(
                    format!(
                        "{url}\nexpected sha256 {}\ngot      sha256 {digest}",
                        file.sha256
                    ),
                ),
            );
        }
        Ok(())
    }

    /// Checks that packages are installed, installing or repairing them when
    /// auto-install is on. Each package to install is one progress phase:
    /// the caller counts `needing_install` in its phases and passes them here.
    pub async fn ensure(
        &self,
        specs: &[PackageSpec],
        auto_install: bool,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        if specs.is_empty() {
            return Ok(());
        }
        if !auto_install {
            return Err(self.not_installed_error(specs));
        }
        for spec in specs {
            phases.start(format!("Installing {}", spec.name));
            self.install(spec, phases, cancel).await?;
        }
        Ok(())
    }
}

/// Deletes leftovers of interrupted installs of this package: unpack
/// folders of processes that are gone. Never touches `keep`.
fn remove_stale(pkg_dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(pkg_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_tmp = entry.file_name().to_string_lossy().starts_with(".tmp-");
        if is_tmp && path != keep {
            log::info!("Removing an interrupted install: {}", path.display());
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// Deletes the given paths when dropped, if they still exist.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            if p.is_dir() {
                let _ = fs::remove_dir_all(p);
            } else if p.exists() {
                let _ = fs::remove_file(p);
            }
        }
    }
}

enum UnpackError {
    Cancelled,
    Failed(String),
}

/// Counts the bytes read, for progress.
struct Counting<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Unpacks a .tgz into `dest`. Every entry must stay inside `dest`: entries
/// with `..` or absolute paths, or that would go through a symbolic link
/// out of `dest`, fail the whole unpack.
fn unpack(
    archive: &Path,
    dest: &Path,
    cancel: &CancellationToken,
    progress: &Arc<AtomicU64>,
) -> Result<(), UnpackError> {
    let fail = |e: &dyn std::fmt::Display| UnpackError::Failed(e.to_string());
    fs::create_dir_all(dest).map_err(|e| fail(&e))?;
    let file = fs::File::open(archive).map_err(|e| fail(&e))?;
    let counting = Counting {
        inner: std::io::BufReader::new(file),
        count: progress.clone(),
    };
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(counting));
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);
    for entry in tar.entries().map_err(|e| fail(&e))? {
        if cancel.is_cancelled() {
            return Err(UnpackError::Cancelled);
        }
        let mut entry = entry.map_err(|e| fail(&e))?;
        let path = entry.path().map_err(|e| fail(&e))?.into_owned();
        // unpack_in refuses `..` and absolute paths (returning false) and
        // paths whose parent resolves outside dest (an error).
        match entry.unpack_in(dest) {
            Ok(true) => {}
            Ok(false) => {
                return Err(UnpackError::Failed(format!(
                    "the archive has an entry outside its folder: {}",
                    path.display()
                )));
            }
            Err(e) => {
                return Err(UnpackError::Failed(format!("{}: {e}", path.display())));
            }
        }
    }
    Ok(())
}
