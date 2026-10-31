//! The test tool (docs/tool-api.md, "test tool").

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::check_build_folder;
use super::lint::check_sources;
use super::{Call, EngineTool};
use crate::board::BoardSpec;
use crate::errors::{Diagnostic, ToolError};
use crate::progress::Phases;
use crate::sim::testbench::{self, TestbenchRun};
use crate::sim::{SimLibrary, Sources, TOOLCHAIN, toolchain};
use crate::state::EngineState;

pub struct Test;

impl EngineTool for Test {
    const NAME: &'static str = "test";
    const DESCRIPTION: &'static str = "Runs testbenches with Icarus Verilog, like apio test: \
        compiles the design files with each testbench, runs it, and reports whether it passed. \
        A testbench fails by calling $fatal or $error; it must end with $finish, and must not \
        call $dumpfile, since the engine sets where its waveform goes (call $dumpvars to \
        write one). Every listed testbench runs, even after one fails. A failing testbench is \
        the result, not an error: passed is false, and each testbench's diagnostics and output \
        say why. Compile errors are a design error with diagnostics at file and line. The \
        first call may install the toolchain package first, which takes a few minutes. To \
        look at a testbench's signals in a waveform viewer, use simulate.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = TestInput;
    type Output = TestOutput;
}

/// Which testbenches to run.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestInput {
    /// The target board: a catalog ID such as alhambra-ii (get_catalog lists
    /// them), or a full board definition. It selects the FPGA's primitive
    /// models, such as SB_IO for iCE40.
    pub board: BoardSpec,
    /// The folder with the design files, as an absolute path. The testbenches
    /// run in it, so they read data files, such as with $readmemh, relative
    /// to it.
    pub source_folder: String,
    /// The design files (.v or .sv), relative to the source folder, with / as
    /// the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// testbenches or included files.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The testbenches to run, in order, relative to the source folder, such
    /// as ["main_tb.v", "rtl/uart_tb.v"]. Each is compiled with the design
    /// files and run on its own.
    #[schemars(length(min = 1))]
    pub testbench_files: Vec<String>,
    /// Where the compiled testbenches and their waveforms go, as an absolute
    /// path. One of its last two folder names must contain "build", such as
    /// /home/u/proj/_build/default. It is created if needed.
    pub build_folder: String,
    /// Verilog macro definitions, from name to value, such as
    /// {"WIDTH": "8"}. An empty value defines the macro with no text, for
    /// `ifdef. APIO_SIM is always defined as 0.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Let iverilog print more detail, shown in the live output and the
    /// logs. It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// One testbench's result.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestbenchResult {
    /// The testbench, relative to the source folder.
    pub file: String,
    /// True when it ran to the end without $fatal or $error.
    pub passed: bool,
    /// Compile warnings and the testbench's $fatal, $error and $warning
    /// messages, at file and line.
    pub diagnostics: Vec<Diagnostic>,
    /// What the testbench printed, such as with $display. May span several
    /// lines.
    pub output: String,
    /// The waveform (.vcd), relative to the build folder, when the testbench
    /// wrote one with $dumpvars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform_file: Option<String>,
}

impl From<TestbenchRun> for TestbenchResult {
    fn from(r: TestbenchRun) -> Self {
        TestbenchResult {
            file: r.file,
            passed: r.passed,
            diagnostics: r.diagnostics,
            output: r.output,
            waveform_file: r.waveform_file,
        }
    }
}

/// The test results.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestOutput {
    /// True when every testbench passed.
    pub passed: bool,
    /// Each testbench's result, in the order given.
    pub testbenches: Vec<TestbenchResult>,
}

pub async fn run(
    state: &EngineState,
    input: TestInput,
    call: &Call,
) -> Result<TestOutput, ToolError> {
    if input.testbench_files.is_empty() {
        return Err(ToolError::request("testbench_files is empty."));
    }
    let sources: Sources = check_sources(
        &input.board,
        &input.source_folder,
        input.source_files,
        input.testbench_files,
        input.defines,
        input.verbose,
    )?;
    let build_folder = check_build_folder(&input.build_folder, &sources.source_folder)?;
    for tb in &sources.testbench_files {
        testbench::check_no_dumpfile(&sources.source_folder, tb)?;
    }
    let includes = sources.include_args()?;

    let needed = state.packages.needing_install(&[TOOLCHAIN])?;
    let phases = Phases::new(
        call.progress.clone(),
        needed.len() + 2 * sources.testbench_files.len(),
    );
    let (bin, package) = toolchain(state, &needed, &phases, call).await?;
    let lib = SimLibrary::for_board(&sources.board, &package);

    let mut results = Vec::new();
    for tb in &sources.testbench_files {
        let r = testbench::compile_and_run(
            "test",
            &sources,
            &lib,
            &includes,
            &bin,
            &build_folder,
            tb,
            false,
            &phases,
            call,
        )
        .await?;
        results.push(TestbenchResult::from(r));
    }
    Ok(TestOutput {
        passed: results.iter().all(|r| r.passed),
        testbenches: results,
    })
}
