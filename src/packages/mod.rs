//! Package download and repair (docs/tool-api.md, "Packages").
//!
//! Two packages: `logix-catalog`, the board catalog (one file for every
//! platform), and `logix-core`, the FPGA tools (one file per platform),
//! built by github.com/openlogix/logix-core. The core holds each tool as
//! its upstream release in a folder of its own (a component, such as
//! `oss-cad-suite`), and its `logix-core.json` says how to run each one.
//!
//! Which releases: the engine's remote config (`remote_config.rs`), fetched
//! before an operation when the cached copy is more than 24 hours old, or
//! when a host asks (`check_updates`). There's no built-in list.
//!
//! Engine folder layout:
//!
//! - `remote-config/config.json`: the cached, resolved remote config.
//! - `packages/<name>/<release>/`: a downloaded package, with its receipt
//!   `logix-download.json`. Once a new release is in place, the old one is
//!   deleted.
//! - `packages/<name>/.tmp-<release>-<pid>/`: a package being unpacked. It is
//!   renamed to `<release>` only when complete, so a crash never leaves a
//!   package that looks downloaded.
//! - `downloads/`: partial downloads, deleted when done.
//! - `locks/<name>.lock`: stops two engine processes from downloading the same
//!   package at once.

pub mod manifest;
pub mod remote_config;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::errors::{ActionCode, ToolError};
use crate::progress::Phases;
use crate::tools::get_engine_info::PackageStatus;
use manifest::{Manifest, PackageFile, PackageSpec};
use remote_config::ConfigSource;

/// The package with the board catalog.
pub const CATALOG: &str = "logix-catalog";
/// The package with the FPGA tools.
pub const CORE: &str = "logix-core";
/// The core's own description of its components, at its top.
pub const CORE_METADATA: &str = "logix-core.json";
/// The platform key of a package file that serves every platform.
pub const ANY_PLATFORM: &str = "any";

/// One component of the core, from `logix-core.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct CoreComponent {
    pub description: String,
    /// Its folder, relative to the core.
    pub folder: String,
    pub run: CoreRun,
}

/// How the engine runs a component's programs.
#[derive(Debug, Clone, Deserialize)]
pub struct CoreRun {
    /// The folders for PATH, relative to the core; the first holds the
    /// programs. It may include another component's, as openXC7 includes
    /// the YosysHQ suite's.
    pub path: Vec<String>,
    /// Variables the programs need, each naming a folder relative to the
    /// core, such as VERILATOR_ROOT on Windows.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// The FPGAs a core's tools can build, by toolchain (ice40, ecp5, gowin,
/// openxc7), each in the tools' own terms, such as {"device": "hx1k",
/// "package": "cb132"} (logix-core's docs/design.md, The FPGA list).
pub type CoreFpgas =
    std::collections::BTreeMap<String, Vec<std::collections::BTreeMap<String, String>>>;

/// The components a core folder's `logix-core.json` lists; None without a
/// readable one.
pub fn read_core_components(
    core: &Path,
) -> Option<std::collections::BTreeMap<String, CoreComponent>> {
    let text = fs::read_to_string(core.join(CORE_METADATA)).ok()?;
    let meta: CoreMetadata = serde_json::from_str(&text).ok()?;
    Some(meta.components)
}

#[derive(Debug, Clone, Deserialize)]
struct CoreMetadata {
    components: std::collections::BTreeMap<String, CoreComponent>,
    #[serde(default)]
    fpgas: CoreFpgas,
}

/// The receipt, written into a package folder when it is complete.
pub const RECORD_FILE: &str = "logix-download.json";
/// How this engine lays packages out. Raise it only when that changes, so
/// packages downloaded by older engines are downloaded again; otherwise an
/// engine update reuses them (Remote updates, Receipts).
pub const LAYOUT: u32 = 1;

/// Share of a package's progress taken by the download. Unpacking takes
/// the rest.
const DOWNLOAD_SHARE: f64 = 80.0;

/// A package's receipt: what was downloaded. A package is ready only when
/// its folder has a receipt matching the package set: the same release,
/// URL, platform, SHA-256 and layout. Not the engine version, which is
/// recorded only for diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DownloadRecord {
    pub name: String,
    /// The GitHub repo, owner/name.
    pub repo: String,
    pub release: String,
    pub file: String,
    pub url: String,
    pub platform: String,
    pub size: u64,
    pub sha256: String,
    /// The engine that downloaded it, such as "logix-engine 2026.11.20".
    pub downloaded_by: String,
    pub layout: u32,
}

/// A package whose release changed with a new remote config.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Update {
    /// The package name.
    pub name: String,
    /// The release used before, as yyyy.mm.dd; omitted for a new package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The release used now, as yyyy.mm.dd.
    pub to: String,
}

/// Where the package set comes from, when it's the remote config.
#[derive(Debug, Clone)]
struct Remote {
    source: ConfigSource,
    /// Whether it may use the network (automatic download on).
    network: bool,
    /// When the config in use was read (seconds since 1970), if it was.
    fetched: Arc<RwLock<Option<u64>>>,
    /// The channel and config file it was read with.
    channel: Arc<RwLock<Option<remote_config::Channel>>>,
    /// One refresh at a time.
    lock: Arc<tokio::sync::Mutex<()>>,
}

/// The remote config in use, for get_engine_info.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    pub location: String,
    pub is_override: bool,
    pub fetched: Option<u64>,
    pub channel: Option<remote_config::Channel>,
}

/// Downloads, repairs and finds the packages of a package set in an engine
/// folder.
#[derive(Debug, Clone)]
pub struct PackageManager {
    /// The engine folder, with symbolic links resolved.
    engine_folder: PathBuf,
    set: Arc<RwLock<Arc<Manifest>>>,
    remote: Option<Remote>,
    platform: String,
    /// Where package files are downloaded from (manifest::GITHUB, or a test
    /// server).
    download_base: String,
}

fn download_error(name: &str, problem: impl std::fmt::Display) -> ToolError {
    ToolError::setup(format!("Can't download the {name} package: {problem}"))
        .with_action(
            ActionCode::DownloadPackages,
            format!("Download the {name} package, then retry. Check the network connection and the free disk space."),
        )
}

impl PackageManager {
    /// A package manager with a fixed package set (for tests).
    pub fn new(
        engine_folder: &Path,
        manifest: Arc<Manifest>,
        platform: &str,
        download_base: &str,
    ) -> Self {
        PackageManager {
            engine_folder: engine_folder.to_path_buf(),
            set: Arc::new(RwLock::new(manifest)),
            remote: None,
            platform: platform.to_string(),
            download_base: download_base.to_string(),
        }
    }

    /// A package manager whose set comes from the remote config at `source`:
    /// the cached copy from that source if there is one, refreshed when due.
    /// `network` is whether automatic download is on.
    pub fn with_remote_config(
        engine_folder: &Path,
        source: ConfigSource,
        network: bool,
        platform: &str,
        download_base: &str,
    ) -> Self {
        let cached = remote_config::load_cache(engine_folder).filter(|c| c.matches(&source));
        let fetched = cached.as_ref().map(|c| c.fetched);
        let channel = cached.as_ref().map(|c| c.channel.clone());
        let set = cached.map(|c| c.set).unwrap_or_default();
        PackageManager {
            engine_folder: engine_folder.to_path_buf(),
            set: Arc::new(RwLock::new(Arc::new(set))),
            remote: Some(Remote {
                source,
                network,
                fetched: Arc::new(RwLock::new(fetched)),
                channel: Arc::new(RwLock::new(channel)),
                lock: Arc::new(tokio::sync::Mutex::new(())),
            }),
            platform: platform.to_string(),
            download_base: download_base.to_string(),
        }
    }

    /// The package set in use. Empty until a remote config has been read.
    pub fn manifest(&self) -> Arc<Manifest> {
        self.set.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The names of the packages this engine uses: those every remote config
    /// names, known before one is read; or a fixed set's.
    pub fn package_names(&self) -> Vec<String> {
        match self.remote {
            Some(_) => remote_config::PACKAGES
                .iter()
                .map(|p| p.to_string())
                .collect(),
            None => self
                .manifest()
                .packages
                .iter()
                .map(|p| p.name.clone())
                .collect(),
        }
    }

    /// The remote config in use, if the set comes from one.
    pub fn remote_info(&self) -> Option<RemoteInfo> {
        self.remote.as_ref().map(|r| RemoteInfo {
            location: r.source.location.clone(),
            is_override: r.source.is_override,
            fetched: *r.fetched.read().unwrap_or_else(|e| e.into_inner()),
            channel: r.channel.read().unwrap_or_else(|e| e.into_inner()).clone(),
        })
    }

    /// Reads the remote config again when the cached copy is more than 24
    /// hours old, when there's none yet, or when `force` is set, and switches
    /// to its packages. Returns the packages whose release changed. It uses
    /// the network only with automatic download on, or when `asked` (a host
    /// asked for packages). Without the network, or when reading fails, the
    /// config in use stays (with a warning in the log); it's an error only
    /// when there's none to use, or for the diagnostic override, which never
    /// falls back.
    pub async fn refresh(&self, force: bool, asked: bool) -> Result<Vec<Update>, ToolError> {
        let Some(remote) = &self.remote else {
            return Ok(vec![]);
        };
        let _one = remote.lock.lock().await;
        let have = !self.manifest().packages.is_empty();
        let fetched = *remote.fetched.read().unwrap_or_else(|e| e.into_inner());
        let due = force
            || !have
            || fetched.is_none_or(|t| {
                remote_config::now().saturating_sub(t) >= remote_config::MAX_AGE.as_secs()
            });
        if !due {
            return Ok(vec![]);
        }
        let no_config = |problem: String| {
            let what = if remote.source.is_override {
                format!(
                    "the remote config from {}",
                    remote_config::DEV_REMOTE_CONFIG_VAR
                )
            } else {
                "the engine's remote config".to_string()
            };
            ToolError::setup(format!("Can't read {what}: {problem}")).with_action(
                ActionCode::DownloadPackages,
                "Check the network connection, then retry.",
            )
        };
        if !remote.network && !asked {
            return if have {
                Ok(vec![])
            } else {
                Err(no_config("automatic download is turned off".into()))
            };
        }
        let read = async {
            let client = remote_config::client()?;
            let read = remote_config::read(&remote.source, &client).await?;
            let entries = remote_config::parse(&read.text)
                .map_err(|e| format!("{} {}: {e}", remote.source.location, read.channel.file))?;
            let set =
                remote_config::resolve(&entries, &self.platform, &self.download_base, &client)
                    .await?;
            Ok::<_, String>((set, read.channel))
        };
        let (new, channel) = match read.await {
            Ok(read) => read,
            Err(e) if have && !remote.source.is_override => {
                log::warn!("Can't read the remote config, keeping the one in use: {e}");
                return Ok(vec![]);
            }
            Err(e) => return Err(no_config(e)),
        };
        let old = self.manifest();
        let updates: Vec<Update> = new
            .packages
            .iter()
            .filter_map(|p| {
                let from = old.get(&p.name).map(|o| o.version.clone());
                (from.as_deref() != Some(&p.version)).then(|| Update {
                    name: p.name.clone(),
                    from: from.map(|f| manifest::shown_version(&f)),
                    to: manifest::shown_version(&p.version),
                })
            })
            .collect();
        let now = remote_config::now();
        let cached = remote_config::Cached {
            source: remote.source.location.clone(),
            config_version: remote_config::CONFIG_VERSION.to_string(),
            channel: channel.clone(),
            fetched: now,
            set: new.clone(),
        };
        log::info!(
            "Remote config: channel {} ({}), {}",
            channel.name,
            channel.how,
            channel.file
        );
        *remote.channel.write().unwrap_or_else(|e| e.into_inner()) = Some(channel);
        if let Err(e) = remote_config::save_cache(&self.engine_folder, &cached) {
            log::warn!("Can't save the remote config: {e}");
        }
        for u in &updates {
            log::info!(
                "Remote config: {} {} -> {}",
                u.name,
                u.from.as_deref().unwrap_or("(none)"),
                u.to
            );
        }
        // Packages the config no longer names.
        if let Ok(entries) = fs::read_dir(self.engine_folder.join("packages")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if new.get(&name).is_none() && entry.path().is_dir() {
                    log::info!("Removing package {name}: the remote config no longer names it");
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        *self.set.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(new);
        *remote.fetched.write().unwrap_or_else(|e| e.into_inner()) = Some(now);
        Ok(updates)
    }

    fn packages_dir(&self, name: &str) -> PathBuf {
        self.engine_folder.join("packages").join(name)
    }

    /// The folder of a downloaded package version.
    pub fn package_dir(&self, spec: &PackageSpec) -> PathBuf {
        self.packages_dir(&spec.name).join(&spec.version)
    }

    fn file_for<'a>(&self, spec: &'a PackageSpec) -> Option<&'a PackageFile> {
        spec.files
            .get(&self.platform)
            .or_else(|| spec.files.get(ANY_PLATFORM))
    }

    /// The package set's entry for a package.
    pub fn spec(&self, name: &str) -> Result<PackageSpec, ToolError> {
        self.manifest().get(name).cloned().ok_or_else(|| {
            ToolError::setup(format!(
                "The {name} package isn't known yet: the engine hasn't read its remote config."
            ))
            .with_action(
                ActionCode::DownloadPackages,
                "Download the packages, which reads the remote config first, then retry.",
            )
        })
    }

    /// The core's components, from its `logix-core.json`, when the core is
    /// downloaded and the file is readable.
    pub fn core_components(&self) -> Option<std::collections::BTreeMap<String, CoreComponent>> {
        let spec = self.manifest().get(CORE)?.clone();
        read_core_components(&self.package_dir(&spec))
    }

    /// The FPGAs the downloaded core's tools can build, from its
    /// `logix-core.json`.
    pub fn core_fpgas(&self) -> Option<CoreFpgas> {
        let spec = self.manifest().get(CORE)?.clone();
        let text = fs::read_to_string(self.package_dir(&spec).join(CORE_METADATA)).ok()?;
        let meta: CoreMetadata = serde_json::from_str(&text).ok()?;
        Some(meta.fpgas)
    }

    /// The folder of a core component, such as `oss-cad-suite`.
    pub fn component_dir(&self, component: &str) -> Result<PathBuf, ToolError> {
        let core = self.package_dir(&self.spec(CORE)?);
        Ok(
            match self
                .core_components()
                .and_then(|c| c.get(component).cloned())
            {
                Some(c) => core.join(c.folder),
                None => core.join(component),
            },
        )
    }

    /// The folder with a core component's programs: the first of its PATH
    /// folders in `logix-core.json`.
    pub fn bin_dir(&self, component: &str) -> Result<PathBuf, ToolError> {
        let core = self.package_dir(&self.spec(CORE)?);
        let components = self.core_components().ok_or_else(|| {
            ToolError::setup(format!(
                "The {CORE} package has no readable {CORE_METADATA}."
            ))
            .with_action(
                ActionCode::DownloadPackages,
                format!("Download the {CORE} package again, which may be damaged."),
            )
        })?;
        let c = components.get(component).ok_or_else(|| {
            ToolError::engine(format!("The {CORE} package has no {component} component."))
        })?;
        let first = c.run.path.first().map(String::as_str).unwrap_or(&c.folder);
        Ok(core.join(first))
    }

    /// A package's status, from its folder and receipt. Fast: it doesn't
    /// verify checksums.
    pub fn status(&self, spec: &PackageSpec) -> PackageStatus {
        let dir = self.package_dir(spec);
        if !dir.exists() {
            return PackageStatus::Missing;
        }
        let record: Option<DownloadRecord> = fs::read_to_string(dir.join(RECORD_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        match (record, self.file_for(spec)) {
            (Some(r), Some(f))
                if r.name == spec.name
                    && r.release == spec.version
                    && r.url == f.url(&self.download_base)
                    && r.platform == self.platform
                    && r.sha256 == f.sha256
                    && r.layout == LAYOUT =>
            {
                PackageStatus::Ready
            }
            _ => PackageStatus::Broken,
        }
    }

    /// The packages, by name, that aren't downloaded and intact. Reads the
    /// remote config first when it's due (see `refresh`).
    pub async fn needing_download(&self, names: &[&str]) -> Result<Vec<PackageSpec>, ToolError> {
        self.refresh(false, false).await?;
        let mut out = Vec::new();
        for name in names {
            let spec = self.spec(name)?;
            if self.status(&spec) != PackageStatus::Ready {
                out.push(spec);
            }
        }
        Ok(out)
    }

    /// The error for packages that need downloading when auto-download is off.
    pub fn not_ready_error(&self, specs: &[PackageSpec]) -> ToolError {
        let list: Vec<String> = specs
            .iter()
            .map(|s| {
                let status = match self.status(s) {
                    PackageStatus::Broken => "broken",
                    _ => "missing",
                };
                format!(
                    "{} {} ({status})",
                    s.name,
                    manifest::shown_version(&s.version)
                )
            })
            .collect();
        ToolError::setup(format!(
            "Required packages aren't downloaded: {}.",
            list.join(", ")
        ))
        .with_action(
            ActionCode::DownloadPackages,
            "Download the required packages, then retry. Automatic download is turned off.",
        )
    }

    /// Downloads or repairs a package. `phases` must be in the package's
    /// download phase: this reports its percentage. Waits while another
    /// process downloads the same package, and does nothing if that leaves it
    /// downloaded.
    pub async fn download(
        &self,
        spec: &PackageSpec,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        self.download_or_redownload(spec, phases, cancel, false)
            .await
    }

    /// Downloads a package again, even if it looks ready, replacing the
    /// current copy. This fixes files damaged or deleted
    /// after the download, which the status check doesn't see. The old copy
    /// stays until the new one is complete.
    pub async fn redownload(
        &self,
        spec: &PackageSpec,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        self.download_or_redownload(spec, phases, cancel, true)
            .await
    }

    async fn download_or_redownload(
        &self,
        spec: &PackageSpec,
        phases: &Phases,
        cancel: &CancellationToken,
        force: bool,
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
                .map_err(|e| download_error(name, format!("can't create the {dir} folder: {e}")))?;
        }
        let pkg_dir = self.packages_dir(name);
        fs::create_dir_all(&pkg_dir).map_err(|e| download_error(name, e))?;

        let _lock = self.lock(name, cancel).await?;
        if !force && self.status(spec) == PackageStatus::Ready {
            log::info!(
                "Package {name} {} was downloaded by another process",
                spec.version
            );
            return Ok(());
        }
        log::info!(
            "{} package {name} {} from {}",
            if force {
                "Downloading again"
            } else {
                "Downloading"
            },
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

        let size = self
            .download_file(file, &part, name, phases, cancel)
            .await?;

        let (tmp, archive) = (tmp_dir.clone(), part.clone());
        let (cancel2, total) = (cancel.clone(), size.max(1));
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
                    "Downloading {name} was cancelled."
                )));
            }
            Ok(Err(UnpackError::Failed(e))) => {
                return Err(download_error(name, format!("can't unpack it: {e}")));
            }
            Err(e) => return Err(ToolError::engine(format!("Unpacking {name} failed: {e}"))),
        }

        let record = DownloadRecord {
            name: spec.name.clone(),
            repo: format!("{}/{}", file.owner, file.repo),
            release: spec.version.clone(),
            file: file.file.clone(),
            url: file.url(&self.download_base),
            platform: self.platform.clone(),
            size,
            sha256: file.sha256.clone(),
            downloaded_by: format!("logix-engine {}", crate::state::ENGINE_VERSION),
            layout: LAYOUT,
        };
        let json = serde_json::to_string_pretty(&record).unwrap_or_default() + "\n";
        fs::write(tmp_dir.join(RECORD_FILE), json).map_err(|e| download_error(name, e))?;
        if final_dir.exists() {
            // A broken download, replaced now that the new one is complete.
            fs::remove_dir_all(&final_dir).map_err(|e| {
                download_error(name, format!("can't remove the broken download: {e}"))
            })?;
        }
        fs::rename(&tmp_dir, &final_dir).map_err(|e| download_error(name, e))?;
        log::info!(
            "Downloaded package {name} {} in {}",
            spec.version,
            final_dir.display()
        );
        // The new release is in place: delete the old ones.
        if let Ok(entries) = fs::read_dir(&pkg_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let old = entry.file_name().to_string_lossy().into_owned();
                if path != final_dir && !old.starts_with(".tmp-") && path.is_dir() {
                    log::info!(
                        "Removing package {name} {old}: replaced by {}",
                        spec.version
                    );
                    let _ = fs::remove_dir_all(&path);
                }
            }
        }
        Ok(())
    }

    /// Takes the package's download lock, waiting while another process holds
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
            .map_err(|e| download_error(name, format!("can't open the lock file: {e}")))?;
        let mut logged = false;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(fs::TryLockError::WouldBlock) => {
                    if !logged {
                        log::info!("Waiting for another engine process downloading {name}");
                        logged = true;
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            return Err(ToolError::cancelled(format!("Downloading {name} was cancelled.")));
                        }
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                }
                Err(fs::TryLockError::Error(e)) => {
                    return Err(download_error(name, format!("can't lock: {e}")));
                }
            }
        }
    }

    /// Downloads a package file to `part`, checking its size (when known)
    /// and checksum. Returns its size.
    async fn download_file(
        &self,
        file: &PackageFile,
        part: &Path,
        name: &str,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<u64, ToolError> {
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
                return Err(ToolError::cancelled(format!("Downloading {name} was cancelled.")));
            }
            r = request => r.map_err(|e| {
                download_error(name, "the download failed").with_details(format!("{url}\n{e}"))
            })?,
        };
        // The size the set gives, or else the one the server announces, for
        // progress and the length check.
        let expected = file.size.or(response.content_length());
        let mut out = fs::File::create(part).map_err(|e| download_error(name, e))?;
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(ToolError::cancelled(format!("Downloading {name} was cancelled.")));
                }
                c = response.chunk() => c.map_err(|e| {
                    download_error(name, "the download was interrupted").with_details(format!("{url}\n{e}"))
                })?,
            };
            let Some(chunk) = chunk else { break };
            received += chunk.len() as u64;
            if expected.is_some_and(|n| received > n) {
                return Err(download_error(name, "the download is larger than expected")
                    .with_details(url.clone()));
            }
            hasher.update(&chunk);
            out.write_all(&chunk).map_err(|e| download_error(name, e))?;
            if let Some(n) = expected {
                phases.percent(DOWNLOAD_SHARE * received as f64 / n.max(1) as f64);
            }
        }
        out.flush().map_err(|e| download_error(name, e))?;
        if let Some(n) = expected
            && received != n
        {
            return Err(download_error(
                name,
                format!("the download has {received} bytes, expected {n}"),
            )
            .with_details(url));
        }
        let digest = hex::encode(hasher.finalize());
        if digest != file.sha256 {
            return Err(
                download_error(name, "the downloaded file failed its checksum check").with_details(
                    format!(
                        "{url}\nexpected sha256 {}\ngot      sha256 {digest}",
                        file.sha256
                    ),
                ),
            );
        }
        Ok(received)
    }

    /// Checks that packages are downloaded, downloading or repairing them when
    /// auto-download is on. Each package to download is one progress phase:
    /// the caller counts `needing_download` in its phases and passes them here.
    pub async fn ensure(
        &self,
        specs: &[PackageSpec],
        auto_download: bool,
        phases: &Phases,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        if specs.is_empty() {
            return Ok(());
        }
        if !auto_download {
            return Err(self.not_ready_error(specs));
        }
        for spec in specs {
            phases.start(format!(
                "Downloading {} {}",
                spec.name,
                manifest::shown_version(&spec.version)
            ));
            self.download(spec, phases, cancel).await?;
        }
        Ok(())
    }
}

/// Deletes leftovers of interrupted downloads of this package: unpack
/// folders of processes that are gone. Never touches `keep`.
fn remove_stale(pkg_dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(pkg_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_tmp = entry.file_name().to_string_lossy().starts_with(".tmp-");
        if is_tmp && path != keep {
            log::info!("Removing an interrupted download: {}", path.display());
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
