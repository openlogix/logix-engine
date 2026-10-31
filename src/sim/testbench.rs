//! Compiling and running testbenches with Icarus Verilog, for test and
//! simulate: iverilog compiles the design, the FPGA's primitive models and
//! one testbench into the build folder, then vvp runs it and writes its
//! waveform there, with Apio's commands.
//!
//! Build folder files, for a testbench `rtl/uart_tb.v`: `rtl/uart_tb.out`
//! (the compiled simulation) and `rtl/uart_tb.vcd` (the waveform), the
//! testbench's path without its extension, as in Apio.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use regex::Regex;

use super::{SimLibrary, Sources, dedup, parse};
use crate::build::{MARKER, run_logged, run_watched, tool_failed};
use crate::errors::{Category, Diagnostic, Severity, ToolError};
use crate::text::to_multiline_text;
use crate::tools::Call;

/// One testbench's run.
#[derive(Debug, Clone, PartialEq)]
pub struct TestbenchRun {
    /// The testbench, relative to the source folder.
    pub file: String,
    /// True when the testbench finished without $fatal, $error or a failure
    /// line from Apio's assertion macros.
    pub passed: bool,
    /// Compile warnings and the testbench's $fatal, $error and $warning
    /// messages.
    pub diagnostics: Vec<Diagnostic>,
    /// What the testbench printed.
    pub output: String,
    /// The waveform, relative to the build folder, when the testbench wrote
    /// one.
    pub waveform_file: Option<String>,
    /// True if the engine stopped the run at its first failure.
    pub stopped_at_failure: bool,
    /// When the first failure happened, in the waveform's time units, when
    /// known.
    pub first_failure_ticks: Option<u64>,
}

/// The testbench's path in the build folder without an extension, such as
/// rtl/uart_tb for rtl/uart_tb.v.
pub fn build_stem(testbench: &str) -> String {
    match testbench.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains('/') => stem.to_string(),
        _ => testbench.to_string(),
    }
}

fn dumpfile_call() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\$dumpfile\s*\(").unwrap())
}

/// Apio's rule: a testbench must not call $dumpfile, since the engine
/// decides where the waveform goes. A design error naming the line.
pub fn check_no_dumpfile(source_folder: &Path, testbench: &str) -> Result<(), ToolError> {
    let text = fs::read_to_string(source_folder.join(testbench))
        .map_err(|e| ToolError::request(format!("Can't read {testbench}: {e}")))?;
    let Some(line) = text.lines().position(|l| dumpfile_call().is_match(l)) else {
        return Ok(());
    };
    let message = format!(
        "{testbench} calls $dumpfile. Remove it: the engine sets where the waveform is written."
    );
    let mut err = ToolError::new(Category::Design, message.clone());
    err.diagnostics.push(Diagnostic {
        file: Some(testbench.to_string()),
        line: Some(line as u32 + 1),
        column: None,
        severity: Severity::Error,
        text: "Don't call $dumpfile in a testbench: the engine sets where the waveform is written."
            .into(),
        tool: None,
        time_ns: None,
    });
    Err(err)
}

/// iverilog's arguments, in Apio's order: the output file, the request's
/// macros, APIO_SIM (1 when simulating for a viewer, 0 when testing), the
/// FPGA's models, then the design files and the testbench.
pub fn iverilog_args(
    sources: &Sources,
    lib: &SimLibrary,
    includes: &[OsString],
    testbench: &str,
    out_file: &Path,
    interactive: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-g2012".into()];
    if sources.verbose {
        args.push("-v".into());
    }
    args.push("-o".into());
    args.push(out_file.into());
    args.extend(sources.define_args());
    args.push(format!("-DAPIO_SIM={}", u8::from(interactive)).into());
    args.extend(lib.defines.iter().map(OsString::from));
    args.extend(includes.iter().cloned());
    let mut lib_dir = OsString::from("-I");
    lib_dir.push(&lib.include_dir);
    args.push(lib_dir);
    args.extend(lib.files.iter().map(OsString::from));
    args.extend(sources.source_files.iter().map(OsString::from));
    args.push(testbench.into());
    args
}

/// Turns failed compile output into a design error, or a tool failure when
/// iverilog said nothing readable.
fn compile_error(run: &crate::runner::ToolRun, errors: Vec<Diagnostic>) -> ToolError {
    if errors.is_empty() {
        return tool_failed("iverilog", "compiling", run);
    }
    let first = &errors[0];
    let place = match (&first.file, first.line) {
        (Some(f), Some(l)) => format!("{f}:{l}: "),
        _ => String::new(),
    };
    let mut err = ToolError::new(
        Category::Design,
        format!("The design has errors. {place}{}", first.text),
    );
    err.details = to_multiline_text(&run.output);
    err.diagnostics = errors;
    err
}

/// Writes the build folder marker and creates the folder for the
/// testbench's files.
pub fn prepare_build_folder(build_folder: &Path, stem: &str) -> Result<PathBuf, ToolError> {
    let io = |e: std::io::Error| ToolError::setup(format!("Can't write in the build folder: {e}"));
    fs::create_dir_all(build_folder).map_err(io)?;
    fs::write(
        build_folder.join(MARKER),
        "Logix Engine build folder. clean_build may delete it.\n",
    )
    .map_err(io)?;
    let base = build_folder.join(stem);
    if let Some(parent) = base.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    Ok(base)
}

/// Compiles and runs one testbench. Compile errors are a design error; a
/// testbench that fails is a result. With `stop_at_first_failure`, the
/// engine stops the run at the testbench's first failure (after its "Time:"
/// line), so a batch run fails fast without the testbench knowing the mode. `operation` names the operation in the
/// cancel message. The tools run in the source folder, so a testbench reads
/// data files, such as with $readmemh, relative to it, as in Apio.
#[allow(clippy::too_many_arguments)]
pub async fn compile_and_run(
    operation: &str,
    sources: &Sources,
    lib: &SimLibrary,
    includes: &[OsString],
    bin: &Path,
    build_folder: &Path,
    testbench: &str,
    interactive: bool,
    stop_at_first_failure: bool,
    phases: &crate::progress::Phases,
    call: &Call,
) -> Result<TestbenchRun, ToolError> {
    let src = &sources.source_folder;
    let src_text = src.to_string_lossy().to_string();
    let stem = build_stem(testbench);
    let base = prepare_build_folder(build_folder, &stem)?;
    let out_file = base.with_extension("out");
    let vcd = base.with_extension("vcd");
    for f in [&out_file, &vcd] {
        if f.is_file() {
            fs::remove_file(f)
                .map_err(|e| ToolError::setup(format!("Can't delete {}: {e}", f.display())))?;
        }
    }

    phases.start(format!("Compiling {testbench}"));
    let args = iverilog_args(sources, lib, includes, testbench, &out_file, interactive);
    let run = run_logged(operation, "iverilog", bin, &args, src, call).await?;
    let mut compiled = parse::parse_iverilog(&run.output, &src_text);
    dedup(&mut compiled);
    if !run.status.success() {
        let errors = compiled
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        return Err(compile_error(&run, errors));
    }
    let mut diagnostics: Vec<Diagnostic> = compiled
        .into_iter()
        .filter(|d| d.severity != Severity::Error)
        .collect();

    phases.start(format!("Running {testbench}"));
    let mut dumpfile = OsString::from("-dumpfile=");
    dumpfile.push(&vcd);
    // -i makes vvp send each line at once rather than when it exits, so a
    // failure is seen while the testbench still runs.
    let args: Vec<OsString> = vec!["-i".into(), out_file.into(), dumpfile];
    let stop = call.cancel.child_token();
    // The lines so far, and whether a failure was seen whose "Time:" line is
    // still to come.
    let seen: Mutex<(Vec<String>, bool)> = Mutex::new((Vec::new(), false));
    let watch = |line: &str| {
        let mut seen = seen.lock().unwrap();
        seen.0.push(line.to_string());
        if !stop_at_first_failure {
            return;
        }
        if seen.1 {
            stop.cancel();
            return;
        }
        match parse::classify_vvp_line(line) {
            parse::VvpLine::TaggedFailure => seen.1 = true,
            parse::VvpLine::MacroFailure => stop.cancel(),
            parse::VvpLine::Other => {}
        }
    };
    let run = run_watched(operation, "vvp", bin, &args, src, call, &stop, &watch).await?;
    let (lines, _) = seen.into_inner().unwrap();
    let (output, exited_ok) = match &run {
        Some(run) => (run.output.clone(), run.status.success()),
        None => {
            log::info!("Stopped {testbench} at its first failure");
            (lines.join("\n"), false)
        }
    };
    let messages = parse::parse_vvp(&output, &src_text);
    let failed = messages
        .iter()
        .any(|m| m.diagnostic.severity == Severity::Error);
    let waveform = vcd.is_file();
    let ns_per_tick = if waveform {
        fs::File::open(&vcd)
            .ok()
            .and_then(|f| parse::vcd_timescale_ns(std::io::BufReader::new(f)))
    } else {
        None
    };
    let first_failure_ticks = messages
        .iter()
        .find(|m| m.diagnostic.severity == Severity::Error)
        .and_then(|m| m.ticks);
    diagnostics.extend(messages.into_iter().map(|m| {
        let mut d = m.diagnostic;
        if let (Some(ticks), Some(ns)) = (m.ticks, ns_per_tick) {
            d.time_ns = Some(ticks as f64 * ns);
        }
        d
    }));
    dedup(&mut diagnostics);
    let output = output
        .lines()
        .filter(|l| !l.starts_with("VCD info: dumpfile"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(TestbenchRun {
        file: testbench.to_string(),
        passed: exited_ok && !failed,
        diagnostics,
        output: to_multiline_text(&output),
        waveform_file: waveform.then(|| format!("{stem}.vcd")),
        stopped_at_failure: run.is_none(),
        first_failure_ticks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_stems() {
        assert_eq!(build_stem("main_tb.v"), "main_tb");
        assert_eq!(build_stem("rtl/uart_tb.sv"), "rtl/uart_tb");
        assert_eq!(build_stem("v1.2/x_tb.v"), "v1.2/x_tb");
    }

    #[test]
    fn dumpfile_is_a_design_error_at_its_line() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("a_tb.v"),
            "module a_tb;\ninitial begin\n  $dumpfile (\"x.vcd\");\nend\nendmodule\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("b_tb.v"),
            "module b_tb; initial $dumpvars; endmodule\n",
        )
        .unwrap();
        let err = check_no_dumpfile(tmp.path(), "a_tb.v").unwrap_err();
        assert_eq!(err.category, Category::Design);
        assert_eq!(err.diagnostics[0].line, Some(3));
        assert!(check_no_dumpfile(tmp.path(), "b_tb.v").is_ok());
    }
}
