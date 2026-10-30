//! The get_engine_info tool (docs/tool-api.md, Tables 16 and 17).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::state::{self, EngineState};

pub struct GetEngineInfo;

impl EngineTool for GetEngineInfo {
    const NAME: &'static str = "get_engine_info";
    const DESCRIPTION: &'static str = "Returns the engine's version, platform, launch settings and \
        package status. Call it first, to check that the engine is new enough and to see which \
        toolchain packages are installed. It is fast and local: it never uses the network. \
        Missing packages are installed automatically by the tools that need them, unless \
        auto_install is off.";
    const READ_ONLY: bool = true;
    type Input = GetEngineInfoInput;
    type Output = GetEngineInfoOutput;
}

/// The get_engine_info tool takes no inputs.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEngineInfoInput {}

/// The engine's version, platform, launch settings and package status.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEngineInfoOutput {
    /// The engine release, as a date tag (yyyy-mm-dd, or yyyy-mm-dd-N for a
    /// later release on the same day). Compare versions by date.
    #[schemars(pattern(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}(-[1-9][0-9]*)?$"))]
    pub engine_version: String,
    /// The MCP protocol versions the engine accepts, newest first.
    pub protocol_versions: Vec<String>,
    /// The platform the engine runs on, such as darwin-arm64.
    pub platform: String,
    /// The engine folder, as the host set it with the --engine-folder
    /// command-line argument. The engine keeps its packages and logs there.
    pub engine_folder: String,
    /// Whether the engine installs and repairs packages automatically when a
    /// tool needs them. On by default. The host turns it off with the
    /// --no-auto-install command-line argument.
    pub auto_install: bool,
    /// The command-line arguments the engine was launched with, as given.
    pub command_line: Vec<String>,
    /// The log file this engine process writes, in the logs subfolder of the
    /// engine folder. Useful when reporting a problem.
    pub log_file: String,
    /// Every package in the engine's manifest, with its status.
    pub packages: Vec<PackageInfo>,
}

/// A toolchain package and whether it's installed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageInfo {
    /// The package name, such as oss-cad-suite.
    pub name: String,
    /// The package release, as a date tag (yyyy-mm-dd).
    #[schemars(pattern(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}(-[1-9][0-9]*)?$"))]
    pub version: String,
    pub status: PackageStatus,
}

/// Whether a package is ready to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackageStatus {
    /// Installed and intact.
    Installed,
    /// Not installed.
    Missing,
    /// Installed but incomplete or damaged. It is repaired when a tool needs
    /// it, unless auto-install is off.
    Broken,
}

pub fn run(
    state: &EngineState,
    _input: GetEngineInfoInput,
) -> Result<GetEngineInfoOutput, ToolError> {
    Ok(GetEngineInfoOutput {
        engine_version: state::ENGINE_VERSION.to_string(),
        protocol_versions: state::PROTOCOL_VERSIONS
            .iter()
            .map(|v| v.to_string())
            .collect(),
        platform: state::platform(),
        engine_folder: state.engine_folder.clone(),
        auto_install: state.auto_install,
        command_line: state.command_line.clone(),
        log_file: state.log_file.display().to_string(),
        packages: state
            .packages
            .manifest()
            .packages
            .iter()
            .map(|spec| PackageInfo {
                name: spec.name.clone(),
                version: spec.version.clone(),
                status: state.packages.status(spec),
            })
            .collect(),
    })
}
