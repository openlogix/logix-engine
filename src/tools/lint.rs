//! The lint tool (docs/tool-api.md, "lint tool").

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::{check_defines, check_source_file, check_source_folder, check_top_module};
use super::{Call, EngineTool};
use crate::board::{self, BoardSpec};
use crate::errors::{Diagnostic, ToolError};
use crate::sim::Sources;
use crate::sim::lint::{self, LintRequest};
use crate::state::EngineState;

pub struct Lint;

impl EngineTool for Lint {
    const NAME: &'static str = "lint";
    const DESCRIPTION: &'static str = "Checks Verilog and SystemVerilog sources for errors and \
        likely mistakes with Verilator, without building, like apio lint. It is much faster \
        than build and finds problems synthesis accepts, such as width mismatches. List the \
        design files and, optionally, the testbenches; the engine doesn't search for them. The \
        board selects the FPGA's primitive models, so designs that use them lint cleanly. \
        Problems found are the result, not an error: passed is false and diagnostics lists \
        each problem at file and line, with Verilator's warning code at the end of the text, \
        which a /* verilator lint_off CODE */ comment can silence. The first call may install \
        the toolchain package first, which takes a few minutes. Writes nothing.";
    const READ_ONLY: bool = true;
    const OPEN_WORLD: bool = true;
    type Input = LintInput;
    type Output = LintOutput;
}

/// What to lint.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LintInput {
    /// The target board: a catalog ID such as alhambra-ii (get_catalog lists
    /// them), or a full board definition. It selects the FPGA's primitive
    /// models, such as SB_IO for iCE40.
    pub board: BoardSpec,
    /// Name of the top-level module, such as main. When omitted, Verilator
    /// checks every module that no other module uses.
    #[serde(default)]
    pub top_module: Option<String>,
    /// The folder with the design files, as an absolute path.
    pub source_folder: String,
    /// The design files to check (.v or .sv), relative to the source folder,
    /// with / as the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// included files.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// Testbench files to check too, such as ["main_tb.v"], relative to the
    /// source folder. Defaults to none.
    #[serde(default)]
    pub testbench_files: Vec<String>,
    /// Verilog macro definitions, from name to value, such as
    /// {"WIDTH": "8"}. An empty value defines the macro with no text, for
    /// `ifdef. SYNTHESIZE is always defined, and APIO_SIM is defined as 0.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Let Verilator print more detail, shown in the live output and the
    /// logs. It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// What lint found.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LintOutput {
    /// True when Verilator found no errors and no warnings.
    pub passed: bool,
    /// The problems found, in Verilator's order: errors and warnings at file
    /// and line where known.
    pub diagnostics: Vec<Diagnostic>,
}

/// Checks the source fields lint, test and simulate share: the board, the
/// source folder, the design files and the testbenches. A file may be
/// listed only once.
pub fn check_sources(
    board: &BoardSpec,
    source_folder: &str,
    source_files: Vec<String>,
    testbench_files: Vec<String>,
    defines: BTreeMap<String, String>,
    verbose: bool,
) -> Result<Sources, ToolError> {
    let board = board::resolve(board)?;
    check_defines(&defines)?;
    let src = check_source_folder(source_folder)?;
    if source_files.is_empty() {
        return Err(ToolError::request("source_files is empty."));
    }
    let mut seen = BTreeSet::new();
    for (field, f) in source_files
        .iter()
        .map(|f| ("source_files", f))
        .chain(testbench_files.iter().map(|f| ("testbench_files", f)))
    {
        check_source_file(&src, field, f)?;
        if !seen.insert(f.clone()) {
            return Err(ToolError::request(format!(
                "{field}: {f:?} is listed more than once."
            )));
        }
    }
    Ok(Sources {
        board,
        source_folder: src,
        source_files,
        testbench_files,
        defines,
        verbose,
    })
}

/// Checks the request and turns it into a LintRequest.
pub fn check(input: LintInput) -> Result<LintRequest, ToolError> {
    if let Some(top) = &input.top_module {
        check_top_module(top)?;
    }
    Ok(LintRequest {
        sources: check_sources(
            &input.board,
            &input.source_folder,
            input.source_files,
            input.testbench_files,
            input.defines,
            input.verbose,
        )?,
        top_module: input.top_module,
    })
}

pub async fn run(
    state: &EngineState,
    input: LintInput,
    call: &Call,
) -> Result<LintOutput, ToolError> {
    let r = lint::lint(state, check(input)?, call).await?;
    Ok(LintOutput {
        passed: r.passed,
        diagnostics: r.diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Category;
    use crate::sim::SimLibrary;
    use serde_json::json;
    use std::ffi::OsString;
    use std::path::Path;

    fn input(src: &Path, extra: serde_json::Value) -> LintInput {
        let mut v = json!({
            "board": "alhambra-ii",
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
        std::fs::write(
            src.join("main.v"),
            "`include \"defs.vh\"\nmodule main; endmodule\n",
        )
        .unwrap();
        std::fs::write(src.join("rtl/defs.vh"), "").unwrap();
        std::fs::write(src.join("rtl/uart.v"), "module uart; endmodule\n").unwrap();
        std::fs::write(src.join("main_tb.v"), "module main_tb; endmodule\n").unwrap();
        (tmp, src)
    }

    #[test]
    fn verilator_gets_apios_options_then_the_files() {
        let (_tmp, src) = project();
        let req = check(input(
            &src,
            json!({
                "top_module": "main",
                "source_files": ["main.v", "rtl/uart.v"],
                "testbench_files": ["main_tb.v"],
                "defines": {"WIDTH": "8", "A": ""},
            }),
        ))
        .unwrap();
        let lib = SimLibrary::for_board(&req.sources.board, Path::new("/pkg"));
        let includes = req.sources.include_args().unwrap();
        let args: Vec<String> = lint::verilator_args(&req, &lib, &includes)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args.join(" "),
            "--lint-only --quiet --bbox-unsup --timing -Wno-TIMESCALEMOD -Wno-MULTITOP \
             -DSYNTHESIZE -DAPIO_SIM=0 --top-module main -DA= -DWIDTH=8 \
             -DNO_ICE40_DEFAULT_ASSIGNMENTS -Irtl -I/pkg/share/yosys/ice40 \
             /pkg/share/yosys/ice40/cells_sim.v main.v rtl/uart.v main_tb.v"
        );

        let mut verbose = req.clone();
        verbose.sources.verbose = true;
        verbose.top_module = None;
        let args: Vec<OsString> = lint::verilator_args(&verbose, &lib, &[]);
        assert!(!args.contains(&"--quiet".into()));
        assert!(!args.contains(&"--top-module".into()));
    }

    #[test]
    fn request_errors() {
        let (_tmp, src) = project();
        for (extra, needle) in [
            (json!({"source_files": []}), "source_files is empty"),
            (json!({"source_files": ["nope.v"]}), "can't be found"),
            (json!({"source_files": ["../x.v"]}), "source_files"),
            (
                json!({"testbench_files": ["main.v"]}),
                "listed more than once",
            ),
            (json!({"top_module": "a b"}), "isn't a module name"),
            (json!({"defines": {"1x": ""}}), "isn't a Verilog identifier"),
            (json!({"board": "no-such-board"}), "no-such-board"),
        ] {
            let err = check(input(&src, extra.clone())).unwrap_err();
            assert_eq!(err.category, Category::Request, "{extra}");
            assert!(err.message.contains(needle), "{extra}: {}", err.message);
        }
    }
}
