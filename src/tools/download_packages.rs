//! The download_packages tool (docs/tool-api.md, Tables 18 and 19).

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
    const DESCRIPTION: &'static str = "Downloads or repairs the engine's toolchain packages from \
        the internet, into its engine folder. Packages that are already ready are left alone, \
        unless force is set to download them again. Other tools download the packages they need on their own, so call this only \
        to download ahead of time, such as before going offline, or when a tool returns a setup \
        error with the download_packages action (for example when the engine was launched with \
        automatic download turned off). A download can take a few minutes; the tool reports \
        progress and can be cancelled.";
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
}

/// What was downloaded, and every package's status afterwards.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadPackagesOutput {
    /// Names of the packages this call downloaded, repaired or downloaded again.
    /// Empty when they were all downloaded already and force was off.
    pub downloaded: Vec<String>,
    /// Every package the engine uses, with its status.
    pub packages: Vec<PackageInfo>,
    /// What this call did, for people, a sentence each, such as
    /// "Downloaded logix-core 2026.11.22.": one per package it downloaded,
    /// repaired or downloaded again. Empty when there was nothing to do.
    pub summary: Vec<String>,
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
        packages,
    })
}
