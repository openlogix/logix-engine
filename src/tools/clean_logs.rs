//! The clean_logs tool (docs/tool-api.md, clean_logs): deletes a host's old
//! log session folders by the same policy the engine uses for its own
//! (logging.rs, the org's log rule), so every frontend cleans its logs the
//! same way without coding the policy itself.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::paths;
use crate::state::EngineState;

pub struct CleanLogs;

impl EngineTool for CleanLogs {
    const NAME: &'static str = "clean_logs";
    const DESCRIPTION: &'static str = "Deletes a host's old log session folders, by the policy \
        the Logix Engine uses for its own logs. A host that keeps its logs in session folders \
        (one folder per run, named <yyyymmdd>-<hhmmss>-<pid>, holding only .log files) calls it \
        with its logs folder, such as /home/user/.myhost/logs, usually once at startup, and names \
        its current session folder in keep. The newest 10 session folders are kept, at most \
        100 MB in all, none older than 30 days; folders changed in the last 24 hours (a program \
        in them may still run), the ones in keep, and anything that isn't a session folder are \
        never deleted. A logs folder that doesn't exist is fine.";
    const READ_ONLY: bool = false;
    type Input = CleanLogsInput;
    type Output = CleanLogsOutput;
}

/// Whose logs to clean.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanLogsInput {
    /// The host's logs folder, the parent of its session folders, as an
    /// absolute path. Its last folder name must be "logs".
    pub logs_folder: String,
    /// Session folders never to delete, as absolute paths, such as the
    /// host's current session. Omitted means none beyond the policy's own.
    #[serde(default)]
    pub keep: Vec<String>,
}

/// What was deleted.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanLogsOutput {
    /// The deleted session folders, as absolute paths. Empty when there was
    /// nothing to delete.
    pub deleted: Vec<String>,
}

pub fn run(_state: &EngineState, input: CleanLogsInput) -> Result<CleanLogsOutput, ToolError> {
    let real = paths::check_logs_folder(&input.logs_folder, paths::home_folder().as_deref())
        .map_err(|e| ToolError::request(format!("logs_folder: {e}.")))?;
    if !real.is_dir() {
        return Ok(CleanLogsOutput {
            deleted: Vec::new(),
        });
    }
    // The kept folders, resolved the same way, to compare with what's found.
    let keep: Vec<PathBuf> = input
        .keep
        .iter()
        .map(|k| {
            paths::check_absolute_path(k)
                .map(|p| paths::resolve(&p).unwrap_or(p))
                .map_err(|e| ToolError::request(format!("keep: {e}.")))
        })
        .collect::<Result<_, _>>()?;
    let keep: Vec<&Path> = keep.iter().map(PathBuf::as_path).collect();
    let deleted = crate::logging::clean_sessions(&real, &keep, SystemTime::now()).map_err(|e| {
        ToolError::setup(format!(
            "Can't delete old log folders in {}: {e}",
            input.logs_folder
        ))
    })?;
    // Named under the folder as the request gave it.
    let given = PathBuf::from(&input.logs_folder);
    Ok(CleanLogsOutput {
        deleted: deleted
            .iter()
            .map(|p| {
                given
                    .join(p.file_name().unwrap_or_default())
                    .display()
                    .to_string()
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logs_folder_must_be_named_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("stuff");
        let e = paths::check_logs_folder(&bad.display().to_string(), None).unwrap_err();
        assert!(e.contains("must be \"logs\""), "{e}");
        let good = tmp.path().join("logs");
        assert!(paths::check_logs_folder(&good.display().to_string(), None).is_ok());
    }
}
