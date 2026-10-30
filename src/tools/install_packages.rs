//! The install_packages tool (docs/tool-api.md, Tables 18 and 19).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::get_engine_info::{PackageInfo, package_list};
use super::{Call, EngineTool};
use crate::errors::ToolError;
use crate::progress::Phases;
use crate::state::EngineState;

pub struct InstallPackages;

impl EngineTool for InstallPackages {
    const NAME: &'static str = "install_packages";
    const DESCRIPTION: &'static str = "Installs or repairs the engine's toolchain packages, \
        downloading them from the internet. Packages that are already installed and intact are \
        left alone, unless force is set to download and install them again. Other tools install the packages they need on their own, so call this only \
        to install ahead of time, such as before going offline, or when a tool returns a setup \
        error with the install_packages action (for example when the engine was launched with \
        automatic install turned off). A download can take a few minutes; the tool reports \
        progress and can be cancelled.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = InstallPackagesInput;
    type Output = InstallPackagesOutput;
}

/// Which packages to install or repair.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallPackagesInput {
    /// Names of the packages to install or repair, as get_engine_info lists
    /// them, such as oss-cad-suite. Omitted means every package in the
    /// engine's manifest.
    #[serde(default)]
    pub packages: Option<Vec<String>>,
    /// Download and install the packages again even if they are installed,
    /// replacing the installed copies. Use it when a package's tools fail in
    /// ways that suggest damaged files. Defaults to false. Don't use it while
    /// other operations are running.
    #[serde(default)]
    pub force: bool,
}

/// What was installed, and every package's status afterwards.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallPackagesOutput {
    /// Names of the packages this call installed, repaired or reinstalled.
    /// Empty when they were all installed already and force was off.
    pub installed: Vec<String>,
    /// Every package in the engine's manifest, with its status.
    pub packages: Vec<PackageInfo>,
}

pub async fn run(
    state: &EngineState,
    input: InstallPackagesInput,
    call: &Call,
) -> Result<InstallPackagesOutput, ToolError> {
    let all: Vec<String> = state
        .packages
        .manifest()
        .packages
        .iter()
        .map(|p| p.name.clone())
        .collect();
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
        state.packages.needing_install(&names)?
    };
    let phases = Phases::new(call.progress.clone(), needed.len());
    // The host asked, so this installs even when automatic install is off.
    if input.force {
        for spec in &needed {
            phases.start(format!("Reinstalling {}", spec.name));
            state
                .packages
                .reinstall(spec, &phases, &call.cancel)
                .await?;
        }
    } else {
        state
            .packages
            .ensure(&needed, true, &phases, &call.cancel)
            .await?;
    }
    Ok(InstallPackagesOutput {
        installed: needed.into_iter().map(|s| s.name).collect(),
        packages: package_list(state),
    })
}
