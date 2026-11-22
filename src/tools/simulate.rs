//! The simulate tool (docs/tool-api.md, "simulate tool").

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::check_build_folder;
use super::lint::check_sources;
use super::{Call, EngineTool};
use crate::errors::{Category, Diagnostic, Severity, ToolError};
use crate::progress::Phases;
use crate::sim::testbench::{self, Ending, RunOptions, build_stem};
use crate::sim::{SimLibrary, toolchain};
use crate::state::EngineState;

pub struct Simulate;

impl EngineTool for Simulate {
    const NAME: &'static str = "simulate";
    const DESCRIPTION: &'static str = "Simulates one testbench with Icarus Verilog and opens \
        its waveform in Open Logix's waveform viewer: compiles the design files with the \
        testbench, runs it to write the waveform (.vcd) into the build folder, writes the \
        viewer, a self-contained page with the waveform in it (viewer_file), and opens it in \
        a window of its own (Logix Waveforms). The testbench must call $dumpvars and $finish, and must not \
        call $dumpfile. The viewer shows the testbench's own signals, opens at the first \
        failure, and saves the person's view as they change it, in view_file (<testbench>.view), \
        unless save_view is false. If the window can't open, viewer_problem says why. Set open_viewer to \
        false to only write the waveform and the page, such as when no one is at the screen \
        or the host shows the page in a window of its own. A failing testbench runs to the end \
        and is shown, with passed false. Compile errors are a design error with diagnostics at \
        file and line. To just check whether testbenches pass, use test.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = SimulateInput;
    type Output = SimulateOutput;
}

fn default_true() -> bool {
    true
}

/// Which testbench to simulate, and how to show it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimulateInput {
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
    /// The folder with the design files, as an absolute path. The testbench
    /// runs in it, so it reads data files, such as with $readmemh, relative
    /// to it.
    pub source_folder: String,
    /// The design files (.v or .sv), relative to the source folder, with / as
    /// the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// testbenches or included files.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The testbench to simulate, relative to the source folder, such as
    /// main_tb.v.
    pub testbench_file: String,
    /// Where the compiled testbench and its waveform go, as an absolute path.
    /// One of its last two folder names must contain "build", such as
    /// /home/user/proj/_build. It is created if needed.
    pub build_folder: String,
    /// Open the waveform viewer's page (viewer_file) in a window of its own
    /// (Logix Waveforms), for the person at the screen. Defaults to true.
    /// False only writes the waveform and the page, such as for a host that
    /// shows the page in a window of its own.
    #[serde(default = "default_true")]
    pub open_viewer: bool,
    /// The view the waveform viewer opens and saves (the signals shown,
    /// their formats and colors, the zoom), relative to the source folder,
    /// such as main_tb.view. Its folder must exist; the file may not yet.
    /// When omitted, the view is <testbench>.view in the build folder, kept
    /// until the build folder is cleaned. When there's no view yet, the
    /// viewer opens its default view, the testbench's own signals.
    #[serde(default)]
    pub view_file: Option<String>,
    /// Let the waveform viewer save the person's view (to view_file) as
    /// they change it. Defaults to true. False opens the view without
    /// writing anything, such as for a host that keeps views its own way.
    #[serde(default = "default_true")]
    pub save_view: bool,
    /// Stop the testbench when simulated time reaches this many nanoseconds,
    /// such as one that never calls $finish. Defaults to no limit.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub max_sim_time_ns: Option<u64>,
    /// Stop the testbench after this many seconds of real time, such as one
    /// stuck in a loop. Defaults to no limit.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub timeout_seconds: Option<u64>,
    /// Verilog macro definitions, from name to value, such as
    /// {"WIDTH": "8"}. An empty value defines the macro with no text, for
    /// `ifdef. LOGIX_SIM is always defined as 1.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Let iverilog print more detail, shown in the live output and the
    /// logs. It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// The simulation result.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimulateOutput {
    /// True when the testbench reached $finish without $fatal, $error or a
    /// failure line from common testbench assertion macros (*** ASSERTION FAILED, ERROR
    /// at), which skip $fatal under simulation.
    pub passed: bool,
    /// Compile warnings and the testbench's $fatal, $error and $warning
    /// messages, at file and line.
    pub diagnostics: Vec<Diagnostic>,
    /// What the testbench printed, such as with $display. May span several
    /// lines.
    pub output: String,
    /// How many failures the testbench reported. diagnostics lists the first
    /// 100 of them, each with its time.
    pub failure_count: u64,
    /// How the run ended: finished (the testbench ended itself),
    /// sim_time_limit or timeout (the engine stopped it), or no_finish (it
    /// ran out of things to simulate without calling $finish, a failure).
    pub ending: Ending,
    /// For people, when the engine stopped it at a limit or it never called
    /// $finish (ending timeout, sim_time_limit or no_finish), as in test.
    /// Omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending_text: Option<String>,
    /// The waveform (.vcd), relative to the build folder.
    pub waveform_file: String,
    /// True if the waveform viewer's page opened in a window.
    pub viewer_opened: bool,
    /// Why the page didn't open, in plain words for the person at the
    /// screen, such as a missing library; only when open_viewer was true
    /// and it didn't open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_problem: Option<String>,
    /// Open Logix's waveform viewer, a self-contained page with the
    /// waveform in it, relative to the build folder, such as main_tb.html.
    pub viewer_file: String,
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

pub async fn run(
    state: &EngineState,
    input: SimulateInput,
    call: &Call,
) -> Result<SimulateOutput, ToolError> {
    if input.max_sim_time_ns == Some(0) {
        return Err(ToolError::request("max_sim_time_ns must be at least 1."));
    }
    if input.timeout_seconds == Some(0) {
        return Err(ToolError::request("timeout_seconds must be at least 1."));
    }
    let tb = input.testbench_file.clone();
    let sources = check_sources(
        &input.board,
        &input.board_variant,
        input.custom_boards_folder.as_deref(),
        &input.source_folder,
        input.source_files,
        vec![tb.clone()],
        input.defines,
        input.verbose,
    )
    .map_err(|mut e| {
        e.message = e.message.replace("testbench_files", "testbench_file");
        e
    })?;
    let src = sources.source_folder.clone();
    let build_folder = check_build_folder(&input.build_folder, &src)?;
    let view_file = input
        .view_file
        .as_deref()
        .map(|rel| crate::wave::check_view_file(&src, rel))
        .transpose()?;
    testbench::check_no_dumpfile(&src, &tb)?;
    let includes = sources.include_args()?;

    let needed = state
        .packages
        .needing_download(&[crate::packages::CORE])
        .await?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 2);
    let (bin, package) = toolchain(state, &needed, &phases, call).await?;
    let lib = SimLibrary::for_board(&sources.board, &package);
    let r = testbench::compile_and_run(
        "simulation",
        &sources,
        &lib,
        &includes,
        &bin,
        &build_folder,
        &tb,
        RunOptions {
            interactive: true,
            stop_at_first_failure: false,
            max_sim_time_ns: input.max_sim_time_ns,
            timeout: input.timeout_seconds.map(std::time::Duration::from_secs),
        },
        &phases,
        call,
    )
    .await?;

    let Some(waveform_file) = r.waveform_file else {
        let mut err = ToolError::new(
            Category::Design,
            format!(
                "{tb} wrote no waveform. Call $dumpvars in the testbench, such as $dumpvars(0, main_tb)."
            ),
        );
        err.details = r.output;
        err.diagnostics = r.diagnostics;
        err.diagnostics.push(Diagnostic {
            file: Some(tb.clone()),
            line: None,
            column: None,
            severity: Severity::Error,
            text: "The testbench wrote no waveform. Call $dumpvars in it.".into(),
            tool: None,
            time_ns: None,
        });
        return Err(err);
    };
    let vcd = build_folder.join(&waveform_file);

    // Open Logix's waveform viewer: its page, with the waveform in it.
    let stem = build_stem(&tb);
    let viewer_file = crate::wave::page_name(&stem);
    let (text, truncated) = crate::wave::read_waveform(&vcd, crate::wave::MAX_WAVEFORM_BYTES)?;
    // The view: the request's, else <testbench>.view in the build folder.
    let view_path = view_file
        .clone()
        .unwrap_or_else(|| build_folder.join(format!("{stem}.{}", crate::wave::VIEW_EXTENSION)));
    let view_name = view_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let data = crate::wave::RunInfo {
        source_folder: &src,
        testbench: &tb,
        waveform: &waveform_file,
        vcd: text,
        truncated,
        diagnostics: &r.diagnostics,
        failure_count: r.failure_count,
        view_file: view_name,
        view: crate::wave::read_view(&view_path),
    }
    .page_data();
    let page = build_folder.join(&viewer_file);
    crate::wave::write_page(&page, &data)?;

    let viewer_problem = input.open_viewer.then(|| {
        let name = crate::viewer::app_name(&state.packages, crate::viewer::WAVEFORMS);
        let project = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut declared = std::collections::BTreeMap::new();
        if input.save_view {
            declared.insert("view".to_string(), view_path.clone());
        }
        crate::viewer::present(
            &state.packages,
            &crate::viewer::Page {
                kind: crate::viewer::WAVEFORMS,
                page: &page,
                title: crate::viewer::title(&name, &[&project, &stem]),
                declared,
                source_folder: Some(&src),
            },
        )
    });
    let viewer_opened = input.open_viewer && viewer_problem.as_ref().is_some_and(Option::is_none);
    Ok(SimulateOutput {
        commands: call.commands.take(),
        passed: r.passed,
        diagnostics: r.diagnostics,
        output: r.output,
        failure_count: r.failure_count,
        ending_text: r.ending.text(&tb),
        ending: r.ending,
        waveform_file,
        viewer_opened,
        viewer_problem: viewer_problem.flatten(),
        viewer_file,
    })
}
