//! The format tool's work: Verible's formatter on each file, in place, with
//! Apio's command (docs/tool-api.md, "format tool").

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::build::parse::relative_file;
use crate::build::{run_logged, tool_failed};
use crate::errors::{Diagnostic, Severity, ToolError};
use crate::progress::Phases;
use crate::state::EngineState;
use crate::text::to_text;
use crate::tools::Call;

/// The package with Verible.
pub const VERIBLE: &str = "verible";

/// The formatter program in the package's bin folder.
pub const FORMATTER: &str = "verible-verilog-format";

/// The file types the formatter takes, as Apio's format command: Verilog,
/// SystemVerilog and their include files.
pub const EXTENSIONS: &[&str] = &[".v", ".sv", ".vh", ".svh"];

/// Formatter options a request can't pass: ones that read options or
/// files from elsewhere, and --inplace, which the engine always passes.
const REFUSED_OPTIONS: &[&str] = &["flagfile", "fromenv", "tryfromenv", "inplace"];

/// A checked format request.
#[derive(Debug, Clone)]
pub struct FormatRequest {
    /// The real path of the source folder.
    pub source_folder: PathBuf,
    /// The files to format, relative to the source folder.
    pub files: Vec<String>,
    /// Extra formatter options, each `--name` or `--name=value`.
    pub options: Vec<String>,
    pub verbose: bool,
}

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// The file was rewritten with the formatted text.
    Formatted,
    /// The file was already formatted and wasn't changed.
    Unchanged,
    /// The formatter couldn't format the file, such as for a syntax error.
    /// The file wasn't changed, and diagnostics say why.
    Failed,
}

/// One file's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileResult {
    /// The file, relative to the source folder, as given in the request.
    pub file: String,
    pub status: FileStatus,
}

/// What format did.
#[derive(Debug, Clone, PartialEq)]
pub struct FormatResult {
    pub files: Vec<FileResult>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Checks an extra formatter option: `--name` or `--name=value`, so it
/// can't name a file to format, and not one of the refused options.
pub fn check_option(option: &str) -> Result<(), String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^--([A-Za-z0-9_]+)(=.*)?$").unwrap());
    let Some(c) = re.captures(option) else {
        return Err(format!(
            "{option:?} isn't an option of the form --name or --name=value, such as \
             --column_limit=80"
        ));
    };
    let name = &c[1];
    if REFUSED_OPTIONS.contains(&name) {
        return Err(format!("the option --{name} isn't allowed"));
    }
    Ok(())
}

/// The formatter's arguments for one file: Apio's command, the request's
/// options, then the file.
pub fn formatter_args(req: &FormatRequest, file: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--nofailsafe_success".into(), "--inplace".into()];
    args.extend(req.options.iter().map(OsString::from));
    if req.verbose && !req.options.iter().any(|o| o == "--verbose") {
        args.push("--verbose".into());
    }
    args.push(file.into());
    args
}

fn location_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?:[^:\s][^:]*: )?(?P<file>[^:\s][^:]*):(?P<line>\d+):(?P<col>\d+)(?:-\d+)?: (?P<text>.*)$",
        )
        .unwrap()
    })
}

/// Verible's message when a statement is too complex to lay out.
const SEARCH_LIMIT: &str = "Some token partitions failed to complete within the search limit";

/// Reads the formatter's output for `file` into diagnostics: syntax errors
/// at file, line and column, and a statement too complex to lay out.
pub fn parse_verible(output: &str, file: &str, source_folder: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        if let Some(c) = location_line().captures(line) {
            out.push(Diagnostic {
                file: relative_file(&c["file"], source_folder).or(Some(file.to_string())),
                line: c["line"].parse().ok(),
                column: c["col"].parse().ok(),
                severity: Severity::Error,
                text: to_text(&c["text"]),
                tool: Some("verible".into()),
                time_ns: None,
            });
        } else if line.contains(SEARCH_LIMIT) {
            out.push(Diagnostic {
                file: Some(file.to_string()),
                line: None,
                column: None,
                severity: Severity::Error,
                text: format!(
                    "{SEARCH_LIMIT}, so the file can't be formatted. A larger limit, such \
                     as the formatter option --max_search_states=2000000, may help, or \
                     the code can be left unformatted between // verilog_format: off \
                     and // verilog_format: on comments."
                ),
                tool: Some("verible".into()),
                time_ns: None,
            });
        }
    }
    out
}

/// Makes sure Verible is installed and returns its bin folder. `phases`
/// already counts one phase per package to install.
async fn verible(
    state: &EngineState,
    needed: &[crate::packages::manifest::PackageSpec],
    phases: &Phases,
    call: &Call,
) -> Result<PathBuf, ToolError> {
    let spec = state
        .packages
        .manifest()
        .get(VERIBLE)
        .cloned()
        .ok_or_else(|| ToolError::engine(format!("{VERIBLE} isn't in the manifest.")))?;
    state
        .packages
        .ensure(needed, state.auto_install, phases, &call.cancel)
        .await?;
    Ok(state.packages.package_dir(&spec).join("bin"))
}

/// Formats each file in turn, like apio format. A file that fails doesn't
/// stop the others.
pub async fn format(
    state: &EngineState,
    req: FormatRequest,
    call: &Call,
) -> Result<FormatResult, ToolError> {
    let needed = state.packages.needing_install(&[VERIBLE])?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 1);
    let bin = verible(state, &needed, &phases, call).await?;
    phases.start("Formatting");
    format_with(&bin, &req, call, &phases).await
}

/// Formats with the formatter in `bin`; the rest of format, for tests.
pub async fn format_with(
    bin: &Path,
    req: &FormatRequest,
    call: &Call,
    phases: &Phases,
) -> Result<FormatResult, ToolError> {
    let src = &req.source_folder;
    let src_text = src.to_string_lossy();
    let mut result = FormatResult {
        files: Vec::new(),
        diagnostics: Vec::new(),
    };
    for (i, file) in req.files.iter().enumerate() {
        let path = src.join(file);
        let read = |p: &Path| {
            std::fs::read(p)
                .map_err(|e| ToolError::request(format!("source_files: can't read {file:?}: {e}.")))
        };
        let before = read(&path)?;
        let args = formatter_args(req, file);
        let run = run_logged("format", FORMATTER, bin, &args, src, call).await?;
        let status = if run.status.success() {
            if read(&path)? == before {
                FileStatus::Unchanged
            } else {
                FileStatus::Formatted
            }
        } else {
            let found = parse_verible(&run.output, file, &src_text);
            if found.is_empty() {
                return Err(tool_failed(
                    "verible-verilog-format",
                    &format!("formatting {file}"),
                    &run,
                ));
            }
            result.diagnostics.extend(found);
            FileStatus::Failed
        };
        result.files.push(FileResult {
            file: file.clone(),
            status,
        });
        phases.percent(100.0 * (i + 1) as f64 / req.files.len() as f64);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_must_be_named_flags() {
        for ok in [
            "--column_limit=80",
            "--verbose",
            "--line_terminator=LF",
            "--x=",
        ] {
            assert!(check_option(ok).is_ok(), "{ok}");
        }
        for bad in [
            "main.v",
            "/etc/x.v",
            "-v",
            "--",
            "--line_terminator LF",
            "--flagfile=/tmp/f",
            "--inplace",
            "--fromenv=x",
        ] {
            assert!(check_option(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn apios_command_then_options_then_the_file() {
        let req = FormatRequest {
            source_folder: "/p".into(),
            files: vec!["rtl/a.v".into()],
            options: vec!["--column_limit=80".into()],
            verbose: true,
        };
        let args: Vec<String> = formatter_args(&req, "rtl/a.v")
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args.join(" "),
            "--nofailsafe_success --inplace --column_limit=80 --verbose rtl/a.v"
        );
    }

    // Real output of verible-verilog-format v0.0-4294-gc1d8f5e8.
    #[test]
    fn syntax_errors_from_real_output() {
        let out = "bad2.v: bad2.v:2:8: syntax error at token \"=\"\n\
                   bad2.v:4:6-8: syntax error at token \"end\"\n\n";
        let d = parse_verible(out, "bad2.v", "/p");
        assert_eq!(d.len(), 2);
        assert_eq!(
            (d[0].file.as_deref(), d[0].line, d[0].column),
            (Some("bad2.v"), Some(2), Some(8))
        );
        assert_eq!(d[0].text, "syntax error at token \"=\"");
        assert_eq!((d[1].line, d[1].column), (Some(4), Some(6)));
        assert_eq!(d[1].text, "syntax error at token \"end\"");
        assert!(d.iter().all(|d| d.severity == Severity::Error));

        let d = parse_verible(
            "rtl/pp.v: rtl/pp.v:4:1-5: syntax error at token \"`else\"\n",
            "rtl/pp.v",
            "/p",
        );
        assert_eq!(
            (d[0].file.as_deref(), d[0].line, d[0].column),
            (Some("rtl/pp.v"), Some(4), Some(1))
        );
    }

    #[test]
    fn search_limit_from_real_output() {
        let out = "big.v: *** Some token partitions failed to complete within the search limit:\n\
                   [module main (], policy: fit-else-expand, (origin: \"module\")\n\
                   *** end of partially formatted partition list\n\
                   [other error status]\n";
        let d = parse_verible(out, "big.v", "/p");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].file.as_deref(), Some("big.v"));
        assert_eq!(d[0].line, None);
        assert!(d[0].text.contains("--max_search_states"), "{}", d[0].text);
    }

    #[test]
    fn other_output_is_not_a_diagnostic() {
        for out in [
            "ok.v: Already formatted, no change.\n",
            "ERROR: Unknown command line flag 'bogus'. Did you mean: lines ?\n",
            "missing.v: No such file or directory\n",
        ] {
            assert!(parse_verible(out, "x.v", "/p").is_empty(), "{out}");
        }
    }
}
