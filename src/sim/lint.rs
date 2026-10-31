//! The lint tool's work: Verilator in lint-only mode on the design and its
//! testbenches, with Apio's options.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

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
    /// Define SYNTHESIZE, so `ifdef SYNTHESIZE picks the synthesis code.
    /// False lints the simulation code instead, like apio lint --nosynth.
    pub synthesize: bool,
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
pub fn verilator_args(
    req: &LintRequest,
    lib: &SimLibrary,
    includes: &[OsString],
    config: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--lint-only".into()];
    if !req.sources.verbose {
        args.push("--quiet".into());
    }
    for a in [
        "--bbox-unsup",
        "--timing",
        "-Wno-TIMESCALEMOD",
        "-Wno-MULTITOP",
    ] {
        args.push(a.into());
    }
    if req.synthesize {
        args.push("-DSYNTHESIZE".into());
    }
    args.push("-DAPIO_SIM=0".into());
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
    args.push(config.into());
    args.extend(lib.files.iter().map(OsString::from));
    args.extend(req.sources.all_files().into_iter().map(OsString::from));
    args
}

/// Writes the Verilator config for the models into the engine folder's
/// tmp folder, named by its content, so it is written once per package
/// version and never half-written.
pub fn write_verilator_config(
    engine_folder: &Path,
    lib: &SimLibrary,
) -> Result<PathBuf, ToolError> {
    let text = lib.verilator_config();
    let hash = hex::encode(Sha256::digest(text.as_bytes()));
    let dir = engine_folder.join("tmp");
    let path = dir.join(format!("models-{}.vlt", &hash[..16]));
    if fs::read_to_string(&path).is_ok_and(|t| t == text) {
        return Ok(path);
    }
    let io = |e: std::io::Error| ToolError::setup(format!("Can't write in the engine folder: {e}"));
    fs::create_dir_all(&dir).map_err(io)?;
    // A unique name per write, since calls can run at the same time.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = dir.join(format!(
        "models-{}.{}-{n}.part",
        &hash[..16],
        std::process::id()
    ));
    fs::write(&part, &text).map_err(io)?;
    fs::rename(&part, &path).map_err(io)?;
    Ok(path)
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

    let config = write_verilator_config(&state.engine_folder_resolved, &lib)?;

    phases.start("Linting");
    let args = verilator_args(&req, &lib, &includes, &config);
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
