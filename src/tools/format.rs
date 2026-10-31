//! The format tool (docs/tool-api.md, "format tool").

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::{check_source_file, check_source_folder};
use super::{Call, EngineTool};
use crate::errors::{Diagnostic, ToolError};
use crate::format::{self, EXTENSIONS, FileResult, FileStatus, FormatRequest};
use crate::state::EngineState;

pub struct Format;

impl EngineTool for Format {
    const NAME: &'static str = "format";
    const DESCRIPTION: &'static str = "Formats Verilog and SystemVerilog files in place with \
        Verible's formatter, like apio format. List the files to format (.v, .sv, .vh or \
        .svh); the engine doesn't search for them. Formatting changes only layout, never \
        meaning. Each file is rewritten only when its layout changes; files lists each one \
        as formatted, unchanged or failed. A file Verible can't parse, such as one with a \
        syntax error, is left unchanged: passed is false and diagnostics gives the error \
        at file and line. That is a result, not an error. Code between // verilog_format: \
        off and // verilog_format: on comments is left as is. The first call may install \
        the Verible package first.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = FormatInput;
    type Output = FormatOutput;
}

/// What to format.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatInput {
    /// The folder with the files, as an absolute path.
    pub source_folder: String,
    /// The files to format (.v, .sv, .vh or .svh), relative to the source
    /// folder, with / as the separator, such as ["main.v", "rtl/defs.vh"].
    /// Each is rewritten in place. A file may be listed only once.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// Extra options for Verible's formatter, each as --name or
    /// --name=value, such as ["--column_limit=80",
    /// "--indentation_spaces=4"], like Apio's format-verible-options.
    /// Defaults to none, which gives Verible's default style.
    #[serde(default)]
    pub verible_options: Vec<String>,
    /// Let the formatter print more detail, shown in the live output and
    /// the logs. It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// What format did.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatOutput {
    /// True when every file is formatted now, whether it was rewritten or
    /// already formatted.
    pub passed: bool,
    /// Each file, in the request's order, with what happened to it.
    pub files: Vec<FileResult>,
    /// Why files failed, such as syntax errors at file, line and column.
    pub diagnostics: Vec<Diagnostic>,
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

/// Checks the request and turns it into a FormatRequest.
pub fn check(input: FormatInput) -> Result<FormatRequest, ToolError> {
    let req = |m: String| ToolError::request(m);
    let src = check_source_folder(&input.source_folder)?;
    if input.source_files.is_empty() {
        return Err(ToolError::request("source_files is empty."));
    }
    let mut seen = BTreeSet::new();
    for f in &input.source_files {
        check_source_file(&src, "source_files", f)?;
        if !EXTENSIONS.iter().any(|e| f.ends_with(e)) {
            return Err(req(format!(
                "source_files: {f:?} isn't a Verilog or SystemVerilog file. Its name must \
                 end in {}.",
                EXTENSIONS.join(", ")
            )));
        }
        if f.starts_with('-') {
            return Err(req(format!(
                "source_files: {f:?} starts with '-', which the formatter reads as an option."
            )));
        }
        if !seen.insert(f) {
            return Err(req(format!(
                "source_files: {f:?} is listed more than once."
            )));
        }
    }
    for o in &input.verible_options {
        format::check_option(o).map_err(|e| req(format!("verible_options: {e}.")))?;
    }
    Ok(FormatRequest {
        source_folder: src,
        files: input.source_files,
        options: input.verible_options,
        verbose: input.verbose,
    })
}

pub async fn run(
    state: &EngineState,
    input: FormatInput,
    call: &Call,
) -> Result<FormatOutput, ToolError> {
    let r = format::format(state, check(input)?, call).await?;
    Ok(FormatOutput {
        commands: call.commands.take(),
        passed: r.files.iter().all(|f| f.status != FileStatus::Failed),
        files: r.files,
        diagnostics: r.diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Category;
    use serde_json::json;
    use std::path::Path;

    fn input(src: &Path, extra: serde_json::Value) -> FormatInput {
        let mut v = json!({
            "source_folder": src,
            "source_files": ["main.v"],
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    fn project() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(src.join("rtl")).unwrap();
        for f in [
            "main.v",
            "rtl/defs.vh",
            "top.sv",
            "inc.svh",
            "main.pcf",
            "-x.v",
        ] {
            std::fs::write(src.join(f), "").unwrap();
        }
        (tmp, src)
    }

    #[test]
    fn a_good_request() {
        let (_tmp, src) = project();
        let req = check(input(
            &src,
            json!({
                "source_files": ["main.v", "rtl/defs.vh", "top.sv", "inc.svh"],
                "verible_options": ["--column_limit=80"],
            }),
        ))
        .unwrap();
        assert_eq!(req.source_folder, src);
        assert_eq!(req.files.len(), 4);
        assert_eq!(req.options, ["--column_limit=80"]);
    }

    #[test]
    fn request_errors() {
        let (_tmp, src) = project();
        for (extra, needle) in [
            (json!({"source_files": []}), "source_files is empty"),
            (json!({"source_files": ["nope.v"]}), "can't be found"),
            (json!({"source_files": ["../x.v"]}), "source_files"),
            (json!({"source_files": ["main.pcf"]}), "isn't a Verilog"),
            (json!({"source_files": ["-x.v"]}), "starts with '-'"),
            (
                json!({"source_files": ["main.v", "main.v"]}),
                "listed more than once",
            ),
            (json!({"verible_options": ["other.v"]}), "verible_options"),
            (json!({"verible_options": ["--inplace"]}), "isn't allowed"),
            (json!({"source_folder": "rel"}), "source_folder"),
        ] {
            let err = check(input(&src, extra.clone())).unwrap_err();
            assert_eq!(err.category, Category::Request, "{extra}");
            assert!(err.message.contains(needle), "{extra}: {}", err.message);
        }
    }
}
