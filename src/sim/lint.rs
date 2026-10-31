//! The lint tool's work: Verilator in lint-only mode on the design and its
//! testbenches, with Apio's options.

use std::ffi::OsString;

use super::{SimLibrary, Sources, TOOLCHAIN, dedup, parse, toolchain};
use crate::build::{run_logged, tool_failed};
use crate::errors::{Diagnostic, Severity, ToolError};
use crate::progress::Phases;
use crate::state::EngineState;
use crate::tools::Call;

/// A checked lint request.
#[derive(Debug, Clone)]
pub struct LintRequest {
    pub sources: Sources,
    /// The top-level module, when known.
    pub top_module: Option<String>,
}

/// What lint found.
#[derive(Debug, Clone, PartialEq)]
pub struct LintResult {
    /// True when Verilator found no errors or warnings.
    pub passed: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// Verilator's arguments, in Apio's order: Apio's options and macros, the
/// top module, the request's macros, the FPGA models, then the files.
pub fn verilator_args(req: &LintRequest, lib: &SimLibrary, includes: &[OsString]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--lint-only".into()];
    if !req.sources.verbose {
        args.push("--quiet".into());
    }
    for a in [
        "--bbox-unsup",
        "--timing",
        "-Wno-TIMESCALEMOD",
        "-Wno-MULTITOP",
        "-DSYNTHESIZE",
        "-DAPIO_SIM=0",
    ] {
        args.push(a.into());
    }
    if let Some(top) = &req.top_module {
        args.push("--top-module".into());
        args.push(top.into());
    }
    args.extend(req.sources.define_args());
    args.extend(lib.defines.iter().map(OsString::from));
    args.extend(includes.iter().cloned());
    let mut lib_dir = OsString::from("-I");
    lib_dir.push(&lib.include_dir);
    args.push(lib_dir);
    args.extend(lib.files.iter().map(OsString::from));
    args.extend(req.sources.all_files().into_iter().map(OsString::from));
    args
}

pub async fn lint(
    state: &EngineState,
    req: LintRequest,
    call: &Call,
) -> Result<LintResult, ToolError> {
    let includes = req.sources.include_args()?;
    let needed = state.packages.needing_install(&[TOOLCHAIN])?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 1);
    let (bin, package) = toolchain(state, &needed, &phases, call).await?;
    let lib = SimLibrary::for_board(&req.sources.board, &package);

    phases.start("Linting");
    let args = verilator_args(&req, &lib, &includes);
    let src = &req.sources.source_folder;
    let run = run_logged("lint", "verilator_bin", &bin, &args, src, call).await?;
    let mut diagnostics = parse::parse_verilator(&run.output, &src.to_string_lossy());
    dedup(&mut diagnostics);
    if !run.status.success() && diagnostics.is_empty() {
        return Err(tool_failed("verilator", "linting", &run));
    }
    let passed = run.status.success() && !diagnostics.iter().any(|d| d.severity != Severity::Info);
    Ok(LintResult {
        passed,
        diagnostics,
    })
}
