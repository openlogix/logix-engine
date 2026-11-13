//! The lint tool (docs/tool-api.md, "lint tool").

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::{check_defines, check_source_file, check_source_folder, check_top_module};
use super::{Call, EngineTool};
use crate::board;
use crate::errors::{Diagnostic, ToolError};
use crate::sim::Sources;
use crate::sim::lint::{self, LintRequest};
use crate::state::EngineState;

pub struct Lint;

impl EngineTool for Lint {
    const NAME: &'static str = "lint";
    const DESCRIPTION: &'static str = "Checks Verilog and SystemVerilog sources for errors and \
        likely mistakes with Verilator, without building. It is much faster \
        than build and finds problems synthesis accepts, such as width mismatches. List the \
        design files and, optionally, the testbenches; the engine doesn't search for them. The \
        board selects the FPGA's primitive models, so designs that use them lint cleanly. \
        Problems found are the result, not an error: passed is false and diagnostics lists \
        each problem at file and line, with Verilator's warning code at the end of the text, \
        which a /* verilator lint_off CODE */ comment can silence. The first call may download \
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
    /// The target board's ID: a catalog ID such as icesugar-1-5 (get_catalog
    /// lists them), or the ID of a board in custom_boards_folder. It selects the FPGA's
    /// primitive models, such as SB_IO for iCE40, EHXPLLL for ECP5, rPLL for
    /// Gowin or PLLE2_BASE for Xilinx.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub board: String,
    /// The board variant: some or all of the selector of one of the board's
    /// variants, which get_catalog lists, such as {"connection": "usb",
    /// "memory": "ram"} or just {"memory": "ram"}. The board's first
    /// variant with these values is used. Omitted or {} means its first
    /// variant, its default.
    #[serde(default)]
    pub board_variant: Option<crate::catalog::VariantChoice>,
    /// The host's own boards: the absolute path of a folder of board files,
    /// one board each in the catalog's format, named by board ID, such as
    /// boards/my-board.json (a project's boards folder). board can name one
    /// of them; one with a catalog board's ID replaces that board in this
    /// request. Problems in any file come back as a request error with a
    /// diagnostic for each, at file, line and column. Send the same folder to
    /// every tool. Omitted means none.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// How to name paths in what people read (diagnostics, messages,
    /// testbench output, live tool output, diagram labels): files named as
    /// a whole, then folders whose files keep their path below the folder
    /// under the folder's new name ("" gives names relative to it); the
    /// first match in each list wins. Paths are compared by real location.
    /// Never applied to command lines or fields that are paths. Omitted
    /// means paths read as the request gave them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_paths: Option<crate::display_paths::DisplayPaths>,
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
    /// `ifdef. SYNTHESIZE is defined unless synthesize is false, and
    /// LOGIX_SIM is defined as 0.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Define SYNTHESIZE, so `ifdef SYNTHESIZE picks the code for synthesis,
    /// as in build. False leaves it undefined, to check the simulation code
    /// instead. Defaults to true.
    #[serde(default = "default_true")]
    pub synthesize: bool,
    /// Let Verilator print more detail, shown in the live output and the
    /// logs. It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

fn default_true() -> bool {
    true
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
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

/// Checks the source fields lint, test and simulate share: the board, the
/// source folder, the design files and the testbenches. A file may be
/// listed only once.
#[allow(clippy::too_many_arguments)]
pub fn check_sources(
    board: &str,
    variant: &crate::board::VariantInput,
    custom_boards_folder: Option<&str>,
    source_folder: &str,
    source_files: Vec<String>,
    testbench_files: Vec<String>,
    defines: BTreeMap<String, String>,
    verbose: bool,
) -> Result<Sources, ToolError> {
    check_defines(&defines)?;
    let src = check_source_folder(source_folder)?;
    let custom = crate::custom_boards::load(custom_boards_folder)?;
    let board = board::resolve(board, variant, &custom)?;
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
            &input.board_variant,
            input.custom_boards_folder.as_deref(),
            &input.source_folder,
            input.source_files,
            input.testbench_files,
            input.defines,
            input.verbose,
        )?,
        top_module: input.top_module,
        synthesize: input.synthesize,
    })
}

pub async fn run(
    state: &EngineState,
    input: LintInput,
    call: &Call,
) -> Result<LintOutput, ToolError> {
    let r = lint::lint(state, check(input)?, call).await?;
    Ok(LintOutput {
        commands: call.commands.take(),
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
        let src = dunce::canonicalize(tmp.path()).unwrap();
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
    fn verilator_gets_its_options_then_the_files() {
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
        let config = Path::new("/engine/tmp/models.vlt");
        let args: Vec<String> = lint::verilator_args(&req, &lib, &includes, config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            // The fake /pkg folder joins with \ on Windows.
            args.join(" ").replace('\\', "/"),
            "--lint-only --quiet --bbox-unsup --timing -Wno-TIMESCALEMOD -Wno-MULTITOP \
             -DSYNTHESIZE -DLOGIX_SIM=0 --top-module main -DA= -DWIDTH=8 \
             -DNO_ICE40_DEFAULT_ASSIGNMENTS -Irtl -I/pkg/share/yosys/ice40 \
             /engine/tmp/models.vlt /pkg/share/yosys/ice40/cells_sim.v main.v rtl/uart.v main_tb.v"
        );

        let mut verbose = req.clone();
        verbose.sources.verbose = true;
        verbose.top_module = None;
        verbose.synthesize = false;
        let args: Vec<OsString> = lint::verilator_args(&verbose, &lib, &[], config);
        assert!(!args.contains(&"--quiet".into()));
        assert!(!args.contains(&"--top-module".into()));
        assert!(!args.contains(&"-DSYNTHESIZE".into()));
        assert!(args.contains(&"-DLOGIX_SIM=0".into()));
    }

    #[test]
    fn the_verilator_config_turns_off_warnings_in_the_models_only() {
        let (_tmp, src) = project();
        let req = check(input(&src, json!({}))).unwrap();
        let lib = SimLibrary::for_board(&req.sources.board, Path::new("/pkg"));
        assert_eq!(
            // The fake /pkg folder joins with \ on Windows, escaped as \\.
            lib.verilator_config().replace("\\\\", "/"),
            "`verilator_config\n\
             lint_off -file \"/pkg/share/yosys/ice40/*\"\n\
             lint_off -rule COMBDLY -file \"/pkg/share/yosys/ice40/*\"\n"
        );
        let engine = tempfile::tempdir().unwrap();
        let a = lint::write_verilator_config(engine.path(), &lib).unwrap();
        let b = lint::write_verilator_config(engine.path(), &lib).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with(engine.path().join("tmp")));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), lib.verilator_config());
        assert_eq!(
            std::fs::read_dir(engine.path().join("tmp"))
                .unwrap()
                .count(),
            1
        );
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
