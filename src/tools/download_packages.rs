//! The download_packages tool (docs/tool-api.md, Tables 18 and 19).
//! Deprecated: check_updates does it all (with cached_config_ok, the same as
//! this tool without packages or force). It's kept working for hosts that
//! call it, and will be dropped once Logix IDE's config names an engine
//! without it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::get_engine_info::{PackageInfo, package_list};
use super::{Call, EngineTool};
use crate::errors::ToolError;
use crate::progress::Phases;
use crate::state::EngineState;

pub struct DownloadPackages;

impl EngineTool for DownloadPackages {
    const NAME: &'static str = "download_packages";
    const DESCRIPTION: &'static str = "Deprecated: call check_updates instead. Without packages \
        and force, this is the same as check_updates with cached_config_ok: it downloads or \
        repairs the engine's toolchain packages, reading the remote config first when the \
        saved one is more than 24 hours old, and check_only downloads nothing and says what \
        would download. Kept for hosts that still call it.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = DownloadPackagesInput;
    type Output = DownloadPackagesOutput;
}

/// Which packages to download or repair.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadPackagesInput {
    /// Names of the packages to download or repair, as get_engine_info lists
    /// them: logix-catalog or logix-core. Omitted means every package the
    /// engine uses.
    #[serde(default)]
    pub packages: Option<Vec<String>>,
    /// Download the packages again even if they are ready, replacing the
    /// current copies. Use it when a package's tools fail in
    /// ways that suggest damaged files. Defaults to false. Don't use it while
    /// other operations are running.
    #[serde(default)]
    pub force: bool,
    /// Download nothing: only read the remote config when it's due and say
    /// what a download would get, in to_download, with sizes. Quick: use it
    /// to tell the user how much will download before downloading. Defaults
    /// to false.
    #[serde(default)]
    pub check_only: bool,
}

/// A package a download gets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageDownload {
    /// The package's name, such as logix-core.
    pub name: String,
    /// The version it downloads.
    pub version: String,
    /// The download's size in bytes, when the release or its download
    /// server says. Omitted when neither does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// What was downloaded, and every package's status afterwards.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadPackagesOutput {
    /// Names of the packages this call downloaded, repaired or downloaded again.
    /// Empty when they were all downloaded already and force was off.
    pub downloaded: Vec<String>,
    /// The packages that needed downloading when the call started, with their
    /// sizes: with check_only, what a download would get; otherwise what this
    /// call downloaded. Empty when everything is ready.
    pub to_download: Vec<PackageDownload>,
    /// Every package the engine uses, with its status.
    pub packages: Vec<PackageInfo>,
    /// What this call did, for people, a sentence each, such as
    /// "Downloaded logix-core 2026.11.22.": one per package it downloaded,
    /// repaired or downloaded again. With check_only, what a download would
    /// get, such as "To download: logix-core 2026.11.22 (412 MB)." and, for
    /// more than one, a total. Empty when there was nothing to do.
    pub summary: Vec<String>,
}

/// A size for people, in MB (decimal, as download sizes are usually given),
/// rounded up, so never "0 MB".
pub fn shown_size(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000).max(1))
}

/// The summary of a check: "To download: <name> <version> (<size>)." for
/// each package, and a total when there's more than one with known sizes.
pub fn to_download_lines(list: &[PackageDownload]) -> Vec<String> {
    let mut lines: Vec<String> = list
        .iter()
        .map(|p| match p.size {
            Some(n) => format!("To download: {} {} ({}).", p.name, p.version, shown_size(n)),
            None => format!("To download: {} {}.", p.name, p.version),
        })
        .collect();
    let sizes: Vec<u64> = list.iter().filter_map(|p| p.size).collect();
    if list.len() > 1 && !sizes.is_empty() {
        let total = shown_size(sizes.iter().sum());
        lines.push(if sizes.len() == list.len() {
            format!("In all: {total}.")
        } else {
            format!("In all: more than {total}.")
        });
    }
    lines
}

/// "Downloaded <name> <version>." for each package downloaded, with the
/// version it has now.
pub fn downloaded_lines(names: &[String], packages: &[PackageInfo]) -> Vec<String> {
    names
        .iter()
        .map(|name| match packages.iter().find(|p| &p.name == name) {
            Some(p) => format!("Downloaded {name} {}.", p.version),
            None => format!("Downloaded {name}."),
        })
        .collect()
}

pub async fn run(
    state: &EngineState,
    input: DownloadPackagesInput,
    call: &Call,
) -> Result<DownloadPackagesOutput, ToolError> {
    let all = state.packages.package_names();
    let names = match input.packages {
        None => all.clone(),
        Some(names) => {
            if let Some(bad) = names.iter().find(|n| !all.contains(n)) {
                return Err(ToolError::request(format!(
                    "Unknown package {bad:?}. The engine's packages are: {}.",
                    all.join(", ")
                )));
            }
            names
        }
    };
    // check_only changes nothing, as check_updates' does.
    if input.check_only {
        let preview = state.packages.preview(false, true).await?;
        let mut to_download = Vec::new();
        for spec in preview
            .to_download
            .iter()
            .filter(|s| names.contains(&s.name))
        {
            to_download.push(PackageDownload {
                name: spec.name.clone(),
                version: crate::packages::manifest::shown_version(&spec.version),
                size: state.packages.download_size(spec).await,
            });
        }
        return Ok(DownloadPackagesOutput {
            downloaded: vec![],
            summary: to_download_lines(&to_download),
            to_download,
            packages: package_list(state),
        });
    }
    // The host asked: read the remote config first if it's due, even with
    // automatic download off.
    state.packages.refresh(false, true).await?;
    let mut seen = std::collections::HashSet::new();
    let names: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| seen.insert(*n))
        .collect();
    let needed = if input.force {
        names
            .iter()
            .filter_map(|n| state.packages.manifest().get(n).cloned())
            .collect()
    } else {
        state.packages.needing_download(&names).await?
    };
    let mut to_download = Vec::new();
    for spec in &needed {
        to_download.push(PackageDownload {
            name: spec.name.clone(),
            version: crate::packages::manifest::shown_version(&spec.version),
            size: state.packages.download_size(spec).await,
        });
    }
    let phases = Phases::new(call.progress.clone(), needed.len());
    // The host asked, so this downloads even when automatic download is off.
    if input.force {
        for spec in &needed {
            phases.start(format!("Downloading again {}", spec.name));
            state
                .packages
                .redownload(spec, &phases, &call.cancel)
                .await?;
        }
    } else {
        state
            .packages
            .ensure(&needed, true, &phases, &call.cancel)
            .await?;
    }
    let downloaded: Vec<String> = needed.into_iter().map(|s| s.name).collect();
    let packages = package_list(state);
    Ok(DownloadPackagesOutput {
        summary: downloaded_lines(&downloaded, &packages),
        downloaded,
        to_download,
        packages,
    })
}
