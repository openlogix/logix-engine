//! The clean_build tool (docs/tool-api.md, Tables 20 and 21).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::build::clean;
use crate::errors::ToolError;
use crate::paths;
use crate::state::EngineState;

pub struct CleanBuild;

impl EngineTool for CleanBuild {
    const NAME: &'static str = "clean_build";
    const DESCRIPTION: &'static str = "Deletes build folders, like apio clean. Given a build \
        folder, such as /home/u/proj/_build/default, it deletes it. Given a folder of build \
        folders, such as /home/u/proj/_build, it deletes each build folder directly inside it, \
        then the folder itself if nothing else is left. It only deletes folders the build tool \
        made, so other files are never removed. A folder that doesn't exist is fine.";
    const READ_ONLY: bool = false;
    type Input = CleanBuildInput;
    type Output = CleanBuildOutput;
}

/// What to clean.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanBuildInput {
    /// A build folder, or a folder whose direct subfolders are build
    /// folders, as an absolute path. One of its last two folder names must
    /// contain "build".
    pub folder: String,
}

/// What was deleted.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanBuildOutput {
    /// The deleted folders, as absolute paths, including the given folder
    /// when it was deleted. Empty when there was nothing to delete.
    pub deleted: Vec<String>,
}

pub fn run(_state: &EngineState, input: CleanBuildInput) -> Result<CleanBuildOutput, ToolError> {
    let real = paths::check_build_folder(&input.folder, paths::home_folder().as_deref(), None)
        .map_err(|e| ToolError::request(format!("folder: {e}.")))?;
    let deleted = clean::clean(&PathBuf::from(&input.folder), &real)?;
    Ok(CleanBuildOutput {
        deleted: deleted.iter().map(|p| p.display().to_string()).collect(),
    })
}
