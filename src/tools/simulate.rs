//! The simulate tool (docs/tool-api.md, "simulate tool").

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::check_build_folder;
use super::lint::check_sources;
use super::{Call, EngineTool};
use crate::board::BoardSpec;
use crate::errors::{Category, Diagnostic, Severity, ToolError};
use crate::paths;
use crate::progress::Phases;
use crate::runner::command_line;
use crate::sim::testbench::{self, build_stem};
use crate::sim::{SimLibrary, TOOLCHAIN, gtkwave, toolchain};
use crate::state::EngineState;

pub struct Simulate;

impl EngineTool for Simulate {
    const NAME: &'static str = "simulate";
    const DESCRIPTION: &'static str = "Simulates one testbench with Icarus Verilog and opens \
        its waveform in GTKWave, like apio sim: compiles the design files with the testbench, \
        runs it to write the waveform (.vcd) into the build folder, then starts GTKWave \
        detached and returns without waiting for it. The testbench must call $dumpvars and \
        $finish, and must not call $dumpfile. GTKWave opens the layout file (.gtkw), which \
        holds the signals shown; when it is missing, or is still a default one, the engine \
        writes a default with the testbench's top-level signals, deleted again when GTKWave \
        exits unless the user saved it; a layout the user saved is kept. Set open_viewer to false to only write the waveform, such as \
        when no one is at the screen. A failing testbench is still simulated and shown, with \
        passed false. Compile errors are a design error with diagnostics at file and line. To \
        just check whether testbenches pass, use test.";
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
    /// The target board: a catalog ID such as alhambra-ii (get_catalog lists
    /// them), or a full board definition. It selects the FPGA's primitive
    /// models, such as SB_IO for iCE40.
    pub board: BoardSpec,
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
    /// /home/u/proj/_build/default. It is created if needed.
    pub build_folder: String,
    /// The GTKWave layout file (.gtkw), relative to the source folder, such
    /// as main_tb.gtkw. GTKWave opens it and saves the user's changes to it.
    /// When it is missing or is still a default one, the engine writes a
    /// default there; a layout the user saved is never changed. Its folder
    /// must exist. When omitted, the layout goes into the build folder, as
    /// <testbench>.gtkw.
    #[serde(default)]
    pub layout_file: Option<String>,
    /// Open the waveform in GTKWave. Defaults to true. False only writes the
    /// waveform and the layout.
    #[serde(default = "default_true")]
    pub open_viewer: bool,
    /// Verilog macro definitions, from name to value, such as
    /// {"WIDTH": "8"}. An empty value defines the macro with no text, for
    /// `ifdef. APIO_SIM is always defined as 1.
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
    /// True when the testbench reached $finish without $fatal or $error.
    pub passed: bool,
    /// Compile warnings and the testbench's $fatal, $error and $warning
    /// messages, at file and line.
    pub diagnostics: Vec<Diagnostic>,
    /// What the testbench printed, such as with $display. May span several
    /// lines.
    pub output: String,
    /// The waveform (.vcd), relative to the build folder.
    pub waveform_file: String,
    /// The layout file GTKWave opens, as an absolute path.
    pub layout_file: String,
    /// True if the engine wrote a default layout, false if it kept one the
    /// user saved.
    pub layout_created: bool,
    /// True if GTKWave was started.
    pub viewer_opened: bool,
}

/// Checks the layout file: a relative path ending in .gtkw, in a folder
/// that exists inside the source folder. The file itself may not exist yet.
/// Returns its real path.
pub fn check_layout_file(src: &Path, rel: &str) -> Result<PathBuf, ToolError> {
    let req = |m: String| ToolError::request(format!("layout_file: {m}."));
    paths::check_relative_path(rel).map_err(req)?;
    if !rel.to_lowercase().ends_with(".gtkw") {
        return Err(req(format!("{rel:?} must end with .gtkw")));
    }
    let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let dir = if parent.is_empty() {
        src.to_path_buf()
    } else {
        let dir = paths::resolve_inside(src, parent).map_err(req)?;
        if !dir.is_dir() {
            return Err(req(format!("{parent:?} isn't a folder")));
        }
        dir
    };
    let path = dir.join(name);
    if path.symlink_metadata().is_ok() {
        let real = paths::resolve_inside(src, rel).map_err(req)?;
        if !real.is_file() {
            return Err(req(format!("{rel:?} isn't a file")));
        }
        return Ok(real);
    }
    Ok(path)
}

pub async fn run(
    state: &EngineState,
    input: SimulateInput,
    call: &Call,
) -> Result<SimulateOutput, ToolError> {
    let tb = input.testbench_file.clone();
    let sources = check_sources(
        &input.board,
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
    let layout = match &input.layout_file {
        Some(rel) => check_layout_file(&src, rel)?,
        None => build_folder.join(format!("{}.gtkw", build_stem(&tb))),
    };
    testbench::check_no_dumpfile(&src, &tb)?;
    let includes = sources.include_args()?;

    let needed = state.packages.needing_install(&[TOOLCHAIN])?;
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
        true,
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
        });
        return Err(err);
    };
    let vcd = build_folder.join(&waveform_file);
    let layout_created = gtkwave::write_default_if_needed(&layout, &vcd, &tb)?;

    if input.open_viewer {
        let program = bin.join("gtkwave");
        let args = gtkwave::gtkwave_args(&vcd, &layout);
        call.output
            .command("gtkwave", &command_line(&program, &args));
        let delete = layout.clone();
        gtkwave::launch_detached(&program, &args, &src, &bin, move || {
            gtkwave::delete_if_generated(&delete)
        })?;
    }
    Ok(SimulateOutput {
        passed: r.passed,
        diagnostics: r.diagnostics,
        output: r.output,
        waveform_file,
        layout_file: layout.to_string_lossy().into_owned(),
        layout_created,
        viewer_opened: input.open_viewer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_file_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().canonicalize().unwrap().join("src");
        std::fs::create_dir_all(src.join("tb")).unwrap();
        assert_eq!(
            check_layout_file(&src, "main_tb.gtkw").unwrap(),
            src.join("main_tb.gtkw")
        );
        assert_eq!(
            check_layout_file(&src, "tb/x.GTKW").unwrap(),
            src.join("tb/x.GTKW")
        );
        for (bad, needle) in [
            ("main_tb.v", "must end with .gtkw"),
            ("../x.gtkw", "layout_file"),
            ("nofolder/x.gtkw", "can't be found"),
        ] {
            let err = check_layout_file(&src, bad).unwrap_err();
            assert_eq!(err.category, Category::Request);
            assert!(err.message.contains(needle), "{bad}: {}", err.message);
        }
        std::fs::create_dir(src.join("dir.gtkw")).unwrap();
        assert!(check_layout_file(&src, "dir.gtkw").is_err());
        #[cfg(unix)]
        {
            std::fs::write(tmp.path().join("out.gtkw"), "").unwrap();
            std::os::unix::fs::symlink(tmp.path().join("out.gtkw"), src.join("link.gtkw")).unwrap();
            let err = check_layout_file(&src, "link.gtkw").unwrap_err();
            assert!(err.message.contains("outside"), "{}", err.message);
        }
    }
}
