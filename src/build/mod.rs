//! The build (docs/tool-api.md, "build tool"), run with each toolchain's
//! standard open source flow: yosys, nextpnr-ice40 and icepack for
//! iCE40; yosys, nextpnr-ecp5 and ecppack for ECP5; yosys,
//! nextpnr-himbaechel and gowin_pack for Gowin; yosys, nextpnr-xilinx,
//! fasm2frames and xc7frames2bit for Xilinx.
//!
//! Build folder files: the marker `.logix-build` (written first, it tells
//! clean_build the folder is a build folder), `hardware.json` (yosys),
//! `hardware.asc` (iCE40), `hardware.config` (ECP5), `hardware.pnr.json`
//! (Gowin) or `hardware.fasm` (Xilinx), nextpnr's placed design, and
//! `hardware.pnr`, its report, `hardware.frames` (Xilinx, the frames the
//! bitstream is made from), `hardware.bin` (iCE40), `hardware.bit` (ECP5
//! and Xilinx) or `hardware.fs` (Gowin), the bitstream, the logs `synth.log` and `pnr.log`, and
//! the build record `build-record.json` (written last, on success only).
//! These names are a fixed convention of the engine, documented in the tool
//! API, so hosts can rely on them.

pub mod clean;
pub mod descriptions;
pub mod parse;
pub mod summary;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::board::ResolvedBoard;
use crate::catalog::{FpgaDefinition, Toolchain};
use crate::errors::{Category, Diagnostic, ToolError};
use crate::progress::Phases;
use crate::runner::{RunError, command_line, run_tool_with};
use crate::state::EngineState;
use crate::text::to_multiline_text;
use crate::tools::Call;
use parse::ResourceUsage;

pub const MARKER: &str = ".logix-build";
pub const RECORD: &str = "build-record.json";
/// Every file a build makes, for any toolchain, deleted before a build
/// runs again.
pub const OUTPUTS: &[&str] = &[
    RECORD,
    "hardware.json",
    "hardware.asc",
    "hardware.config",
    "hardware.pnr.json",
    "hardware.fasm",
    "hardware.frames",
    "hardware.pnr",
    "hardware.bin",
    "hardware.bit",
    "hardware.fs",
    "synth.log",
    "pnr.log",
];

/// The bitstream's file name in the build folder.
pub fn bitstream_name(arch: Toolchain) -> &'static str {
    match arch {
        Toolchain::Ice40 => "hardware.bin",
        Toolchain::Ecp5 => "hardware.bit",
        Toolchain::Gowin => "hardware.fs",
        Toolchain::Openxc7 => "hardware.bit",
    }
}

/// yosys's synthesis script (its -p argument), writing the
/// netlist to `json`. It starts by making sure the top module isn't a black
/// box: yosys's read_verilog makes an empty module one (a module whose only
/// ports are unused inputs, or none, such as a board's starting design), and
/// then nextpnr can't find the top module, and on Xilinx synth_xilinx adds
/// no I/O buffers, which nextpnr-xilinx refuses. `hierarchy` first, since
/// modules are read lazily; `=` because selections skip black boxes. Other
/// empty modules, such as a primitive's stub, stay black boxes.
pub fn synth_script(fpga: &FpgaDefinition, top: &str, json: &Path) -> String {
    format!(
        "hierarchy -top {top}; setattr -mod -unset blackbox ={top}; {}",
        synth_command(fpga, top, json)
    )
}

/// The toolchain's synthesis command.
fn synth_command(fpga: &FpgaDefinition, top: &str, json: &Path) -> String {
    let json = json.display();
    match fpga.toolchain() {
        Toolchain::Ice40 => format!("synth_ice40 -top {top} -json \"{json}\""),
        Toolchain::Ecp5 => format!("synth_ecp5 -top {top} -json \"{json}\""),
        Toolchain::Gowin => {
            let family = match fpga.family.yosys_family() {
                Some(f) => format!(" -family {f}"),
                None => String::new(),
            };
            format!("synth_gowin -top {top}{family}; write_json \"{json}\"")
        }
        Toolchain::Openxc7 => {
            format!("synth_xilinx -arch xc7 -top {top}; write_json \"{json}\"")
        }
    }
}

/// The place-and-route program and its arguments, plus the log
/// file. Run in the build folder. nextpnr is told the top module too:
/// otherwise it guesses it from the netlist, and fails ("Failed to
/// autodetect top module") on a design without logic, such as one whose
/// only port is an input, or none.
pub fn pnr_command(
    fpga: &FpgaDefinition,
    top: &str,
    constraint_file: &Path,
    verbose: bool,
) -> (&'static str, Vec<OsString>) {
    let mut args: Vec<OsString> = vec!["--top".into(), top.into()];
    if matches!(fpga.toolchain(), Toolchain::Ice40 | Toolchain::Ecp5) {
        args.push(format!("--{}", fpga.nextpnr_device).into());
        args.extend(["--package".into(), fpga.package().into()]);
    }
    let tool = match fpga.toolchain() {
        Toolchain::Ice40 => {
            for a in ["--json", "hardware.json", "--asc", "hardware.asc"] {
                args.push(a.into());
            }
            args.extend(["--report".into(), "hardware.pnr".into(), "--pcf".into()]);
            args.push(constraint_file.into());
            "nextpnr-ice40"
        }
        Toolchain::Ecp5 => {
            args.push("--speed".into());
            args.push(fpga.nextpnr_speed.clone().unwrap_or_default().into());
            for a in ["--json", "hardware.json", "--textcfg", "hardware.config"] {
                args.push(a.into());
            }
            args.extend(["--report".into(), "hardware.pnr".into(), "--lpf".into()]);
            args.push(constraint_file.into());
            args.extend(["--timing-allow-fail".into(), "--force".into()]);
            "nextpnr-ecp5"
        }
        Toolchain::Gowin => {
            args.extend(["--device".into(), fpga.nextpnr_device.clone().into()]);
            for a in ["--json", "hardware.json", "--write", "hardware.pnr.json"] {
                args.push(a.into());
            }
            args.extend(["--report".into(), "hardware.pnr".into()]);
            if let Some(family) = &fpga.nextpnr_family {
                args.extend(["--vopt".into(), format!("family={family}").into()]);
            }
            let mut cst = OsString::from("cst=");
            cst.push(constraint_file);
            args.extend(["--vopt".into(), cst]);
            "nextpnr-himbaechel"
        }
        Toolchain::Openxc7 => {
            args.extend(["--device".into(), fpga.nextpnr_device.clone().into()]);
            let mut xdc = OsString::from("xdc=");
            xdc.push(constraint_file);
            args.extend(["-o".into(), xdc, "-o".into(), "fasm=hardware.fasm".into()]);
            for a in ["--json", "hardware.json", "--report", "hardware.pnr"] {
                args.push(a.into());
            }
            "nextpnr-xilinx"
        }
    };
    args.extend(["--log".into(), "pnr.log".into()]);
    if !verbose {
        args.push("-q".into());
    }
    (tool, args)
}

/// The programs that pack the bitstream, with their arguments, the standard ones.
/// Run in order in the build folder. `package_dir` is the folder of the
/// core component with the place-and-route tools (see `pnr_package`).
pub fn pack_commands(
    fpga: &FpgaDefinition,
    package_dir: &Path,
) -> Vec<(&'static str, Vec<OsString>)> {
    match fpga.toolchain() {
        Toolchain::Ice40 => vec![(
            "icepack",
            vec!["hardware.asc".into(), "hardware.bin".into()],
        )],
        Toolchain::Ecp5 => vec![(
            "ecppack",
            vec![
                "--compress".into(),
                "--db".into(),
                package_dir.join("share/trellis/database").into(),
                "hardware.config".into(),
                "hardware.bit".into(),
            ],
        )],
        Toolchain::Gowin => vec![(
            "gowin_pack",
            vec![
                "-d".into(),
                fpga.packer_device.clone().unwrap_or_default().into(),
                "-o".into(),
                "hardware.fs".into(),
                "hardware.pnr.json".into(),
            ],
        )],
        Toolchain::Openxc7 => {
            let device = &fpga.nextpnr_device;
            let db = package_dir
                .join("share/nextpnr/external/prjxray-db")
                .join(fpga.family.prjxray_family().unwrap_or_default());
            vec![
                (
                    "fasm2frames",
                    vec![
                        "--part".into(),
                        device.into(),
                        "--db-root".into(),
                        db.clone().into(),
                        "hardware.fasm".into(),
                        "hardware.frames".into(),
                    ],
                ),
                (
                    "xc7frames2bit",
                    vec![
                        "--part_file".into(),
                        db.join(device).join("part.yaml").into(),
                        "--part_name".into(),
                        device.into(),
                        "--frm_file".into(),
                        "hardware.frames".into(),
                        "--output_file".into(),
                        "hardware.bit".into(),
                    ],
                ),
            ]
        }
    }
}

/// The core component with the build tools: yosys, and every toolchain's
/// place-and-route and packing tools except Xilinx's.
pub const TOOLCHAIN: &str = "oss-cad-suite";
/// The core component with Xilinx's place-and-route and packing tools.
pub const XILINX_TOOLCHAIN: &str = "openxc7";

/// The core component with the toolchain's place-and-route and packing tools.
pub fn pnr_package(arch: Toolchain) -> &'static str {
    match arch {
        Toolchain::Openxc7 => XILINX_TOOLCHAIN,
        _ => TOOLCHAIN,
    }
}

/// A checked build request.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub board: ResolvedBoard,
    pub top_module: String,
    /// The real path of the source folder.
    pub source_folder: PathBuf,
    pub source_files: Vec<String>,
    pub constraint_file: String,
    /// The real path of the build folder (it may not exist yet).
    pub build_folder: PathBuf,
    pub defines: BTreeMap<String, String>,
    /// Let the tools print more, without changing the result.
    pub verbose: bool,
}

/// What a build returns (Table 7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildResult {
    pub bitstream: String,
    pub up_to_date: bool,
    pub resources_usage: BTreeMap<String, ResourceUsage>,
    pub timing: BTreeMap<String, f64>,
    pub warnings: Vec<Diagnostic>,
}

/// The build record, written to the build folder when a build succeeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRecord {
    pub engine_version: String,
    /// What the build depended on, as a checksum: the next build with the
    /// same inputs is up to date.
    pub inputs_checksum: String,
    /// The full board the build used, read by program_board.
    pub board: ResolvedBoard,
    pub board_checksum: String,
    pub result: BuildResult,
}

/// Reads a build record, if there is a valid one.
pub fn read_record(build_folder: &Path) -> Option<BuildRecord> {
    let text = fs::read_to_string(build_folder.join(RECORD)).ok()?;
    serde_json::from_str(&text).ok()
}

fn include_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r#"^\s*`include\s+"([^"]+)""#).unwrap())
}

/// Finds the files included by the source files, recursively. An include is
/// looked up in the including file's folder, then in the source folder, as
/// yosys does when run in the source folder. Includes that aren't found are
/// left for yosys to report. Returns paths relative to the source folder.
pub fn find_includes(
    source_folder: &Path,
    files: &[String],
) -> Result<BTreeSet<String>, ToolError> {
    let mut found = BTreeSet::new();
    let mut todo: Vec<String> = files.to_vec();
    let mut seen: BTreeSet<String> = files.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        let Ok(text) = fs::read_to_string(source_folder.join(&rel)) else {
            continue;
        };
        let dir = Path::new(&rel).parent().unwrap_or(Path::new(""));
        for line in text.lines() {
            let Some(c) = include_re().captures(line) else {
                continue;
            };
            let name = &c[1];
            let candidates = [dir.join(name), PathBuf::from(name)];
            let Some(hit) = candidates.iter().find(|p| source_folder.join(p).is_file()) else {
                continue;
            };
            let real = dunce::canonicalize(source_folder.join(hit)).map_err(|e| {
                ToolError::request(format!("Can't read the included file {name:?}: {e}"))
            })?;
            let inside = real.strip_prefix(source_folder).map_err(|_| {
                ToolError::request(format!(
                    "{rel} includes {name:?}, which is outside the source folder. Included files must be under the source folder."
                ))
            })?;
            let inside = inside.to_string_lossy().replace('\\', "/");
            if seen.insert(inside.clone()) {
                found.insert(inside.clone());
                todo.push(inside);
            }
        }
    }
    Ok(found)
}

fn file_checksum(path: &Path) -> Result<String, ToolError> {
    let data = fs::read(path)
        .map_err(|e| ToolError::request(format!("Can't read {}: {e}", path.display())))?;
    Ok(hex::encode(Sha256::digest(&data)))
}

/// A checksum of everything the build depends on: the engine, the toolchain
/// package, the board, the options, and the contents of every source,
/// included and constraint file.
pub fn inputs_checksum(
    req: &BuildRequest,
    includes: &BTreeSet<String>,
    core_version: &str,
) -> Result<String, ToolError> {
    let mut contents = BTreeMap::new();
    for rel in req
        .source_files
        .iter()
        .chain(includes.iter())
        .chain(std::iter::once(&req.constraint_file))
    {
        contents.insert(rel.clone(), file_checksum(&req.source_folder.join(rel))?);
    }
    let key = serde_json::json!({
        "engine_version": crate::state::ENGINE_VERSION,
        "core": [crate::packages::CORE, core_version],
        "board": req.board.build_key(),
        "top_module": req.top_module,
        "source_files": req.source_files,
        "constraint_file": req.constraint_file,
        "defines": req.defines,
        "contents": contents,
    });
    Ok(hex::encode(Sha256::digest(key.to_string().as_bytes())))
}

fn io_error(what: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::setup(format!("Can't {what}: {e}"))
}

/// Deletes the previous build's outputs, so a failed build leaves no stale
/// bitstream. Only the engine's own files.
fn delete_outputs(build_folder: &Path) -> Result<(), ToolError> {
    for name in OUTPUTS {
        let p = build_folder.join(name);
        if p.is_file() {
            fs::remove_file(&p).map_err(|e| io_error(&format!("delete {}", p.display()), e))?;
        }
    }
    Ok(())
}

/// Runs a toolchain program with its command line and output sent as live
/// output. Whatever its exit status, the run is returned; cancelling and a
/// program that can't start are errors. `operation` names the operation in
/// the cancel message, such as "build".
pub async fn run_logged(
    operation: &str,
    tool: &str,
    bin_dir: &Path,
    args: &[OsString],
    cwd: &Path,
    call: &Call,
) -> Result<crate::runner::ToolRun, ToolError> {
    run_watched(
        operation,
        tool,
        bin_dir,
        args,
        cwd,
        call,
        &call.cancel,
        &|_| {},
    )
    .await?
    .ok_or_else(|| ToolError::cancelled(format!("The {operation} was cancelled.")))
}

/// Like run_logged, also calling `watch` with each output line. Cancelling
/// `stop`, a child of the call's cancel token, ends the run early: then it
/// returns None, and the caller has seen the output through `watch`.
#[allow(clippy::too_many_arguments)]
pub async fn run_watched(
    operation: &str,
    tool: &str,
    bin_dir: &Path,
    args: &[OsString],
    cwd: &Path,
    call: &Call,
    stop: &tokio_util::sync::CancellationToken,
    watch: &(dyn Fn(&str) + Send + Sync),
) -> Result<Option<crate::runner::ToolRun>, ToolError> {
    let program = crate::commands::program_in(bin_dir, tool);
    call.output.command(tool, &command_line(&program, args));
    call.commands
        .push(crate::commands::record(&program, args, cwd, bin_dir));
    let shown = crate::output_rules::ToolOutput::new(tool, call.output.as_ref());
    let on_line = |line: &str, redraw: bool| {
        shown.line(line, redraw);
        watch(line);
    };
    match run_tool_with(&program, args, cwd, bin_dir, stop, &on_line).await {
        Ok(run) => Ok(Some(run)),
        Err(RunError::Cancelled) if !call.cancel.is_cancelled() => Ok(None),
        Err(RunError::Cancelled) => Err(ToolError::cancelled(format!(
            "The {operation} was cancelled."
        ))),
        Err(RunError::Spawn(e)) => {
            Err(
                ToolError::new(Category::Tool, format!("Can't start {tool}: {e}")).with_action(
                    crate::errors::ActionCode::DownloadPackages,
                    format!(
                        "Download the {} package again, which may be damaged.",
                        crate::packages::CORE
                    ),
                ),
            )
        }
    }
}

/// A tool that exited with an error status, as a tool error with its output.
pub fn tool_failed(tool: &str, step: &str, run: &crate::runner::ToolRun) -> ToolError {
    let mut err = ToolError::new(
        Category::Tool,
        format!("{tool} failed while {step} ({}).", run.status),
    );
    err.details = to_multiline_text(&run.output);
    err
}

/// One build step's tool run, turned into an error when it failed.
async fn run_step(
    step: &str,
    tool: &str,
    bin_dir: &Path,
    args: Vec<OsString>,
    cwd: &Path,
    call: &Call,
) -> Result<String, ToolError> {
    let run = run_logged("build", tool, bin_dir, &args, cwd, call).await?;
    if run.status.success() {
        Ok(run.output)
    } else {
        Err(tool_failed(tool, step, &run))
    }
}

/// Turns a failed step with ERROR lines into a design error.
fn design_error(step_error: ToolError, errors: Vec<Diagnostic>) -> ToolError {
    if errors.is_empty() || step_error.category != Category::Tool {
        return step_error;
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
    err.details = step_error.details;
    err.diagnostics = errors;
    err
}

/// Runs a build, or returns the previous result when nothing changed.
pub async fn build(
    state: &EngineState,
    req: BuildRequest,
    call: &Call,
) -> Result<BuildResult, ToolError> {
    let includes = find_includes(&req.source_folder, &req.source_files)?;
    // The core release the remote config names, which the up-to-date check
    // compares (reading the config first when it's due).
    state.packages.refresh(false, false).await?;
    let core = state.packages.spec(crate::packages::CORE)?;
    let fpga = &req.board.fpga;
    let checksum = inputs_checksum(&req, &includes, &core.version)?;

    if let Some(record) = read_record(&req.build_folder)
        && record.inputs_checksum == checksum
        && req.build_folder.join(&record.result.bitstream).is_file()
    {
        log::info!("Build in {} is up to date", req.build_folder.display());
        // The same bitstream, but the board may differ in what doesn't
        // change it, such as its variant or name: keep the request's, which
        // program_board uses.
        if record.board.checksum() != req.board.checksum() {
            write_record(
                &req.build_folder,
                &BuildRecord {
                    board_checksum: req.board.checksum(),
                    board: req.board.clone(),
                    ..record.clone()
                },
            )?;
        }
        let mut result = record.result;
        result.up_to_date = true;
        return Ok(result);
    }

    let needed = state
        .packages
        .needing_download(&[crate::packages::CORE])
        .await?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 3);
    state
        .packages
        .ensure(&needed, state.auto_download, &phases, &call.cancel)
        .await?;
    let bin = state.packages.bin_dir(TOOLCHAIN)?;
    // The place-and-route and packing tools' component.
    let pnr = pnr_package(fpga.toolchain());
    let pnr_dir = state.packages.component_dir(pnr)?;
    let pnr_bin = state.packages.bin_dir(pnr)?;

    let out = &req.build_folder;
    fs::create_dir_all(out).map_err(|e| io_error("create the build folder", e))?;
    fs::write(
        out.join(MARKER),
        "Logix Engine build folder. clean_build may delete it.\n",
    )
    .map_err(|e| io_error("write in the build folder", e))?;
    delete_outputs(out)?;

    let src_text = req.source_folder.to_string_lossy().to_string();
    let mut warnings = Vec::new();

    // Synthesis, run in the source folder so files are reported as given.
    phases.start("Synthesizing");
    let mut args: Vec<OsString> = Vec::new();
    if !req.verbose {
        args.push("-q".into());
    }
    let base: [OsString; 5] = [
        "-l".into(),
        out.join("synth.log").into(),
        "-p".into(),
        synth_script(&req.board.fpga, &req.top_module, &out.join("hardware.json")).into(),
        "-DSYNTHESIZE".into(),
    ];
    args.extend(base);
    for (name, value) in &req.defines {
        args.push(format!("-D{name}={value}").into());
    }
    args.extend(req.source_files.iter().map(OsString::from));
    let synth = run_step(
        "synthesizing",
        "yosys",
        &bin,
        args,
        &req.source_folder,
        call,
    )
    .await;
    let synth_out = match &synth {
        Ok(o) => o.clone(),
        Err(e) => e.details.clone(),
    };
    let parsed = parse::parse_yosys(&synth_out, &src_text);
    warnings.extend(parsed.warnings);
    synth.map_err(|e| design_error(e, parsed.errors))?;

    // Place and route.
    phases.start("Placing and routing");
    let (pnr_tool, args) = pnr_command(
        fpga,
        &req.top_module,
        &req.source_folder.join(&req.constraint_file),
        req.verbose,
    );
    let pnr = run_step("placing and routing", pnr_tool, &pnr_bin, args, out, call).await;
    let pnr_out = match &pnr {
        Ok(o) => o.clone(),
        Err(e) => e.details.clone(),
    };
    let parsed = parse::parse_nextpnr(&pnr_out, &req.constraint_file);
    warnings.extend(parsed.warnings);
    pnr.map_err(|e| design_error(e, parsed.errors))?;

    // Bitstream.
    phases.start("Packing bitstream");
    for (pack_tool, args) in pack_commands(fpga, &pnr_dir) {
        run_step(
            "packing the bitstream",
            pack_tool,
            &pnr_bin,
            args,
            out,
            call,
        )
        .await?;
    }

    let report = fs::read_to_string(out.join("hardware.pnr"))
        .map_err(|e| ToolError::new(Category::Tool, format!("nextpnr wrote no report: {e}")))?;
    let (resources_usage, timing) = parse::parse_report(&report)
        .map_err(|e| ToolError::new(Category::Tool, format!("Can't read nextpnr's report: {e}")))?;

    // Verbose tools can print a warning more than once.
    let mut seen = std::collections::HashSet::new();
    warnings.retain(|w| seen.insert((w.file.clone(), w.line, w.text.clone())));
    let result = BuildResult {
        bitstream: bitstream_name(fpga.toolchain()).into(),
        up_to_date: false,
        resources_usage,
        timing,
        warnings,
    };
    let record = BuildRecord {
        engine_version: crate::state::ENGINE_VERSION.into(),
        inputs_checksum: checksum,
        board_checksum: req.board.checksum(),
        board: req.board.clone(),
        result: result.clone(),
    };
    write_record(out, &record)?;
    log::info!("Build in {} done", out.display());
    Ok(result)
}

fn write_record(build_folder: &Path, record: &BuildRecord) -> Result<(), ToolError> {
    let json = serde_json::to_string_pretty(record).unwrap_or_default() + "\n";
    fs::write(build_folder.join(RECORD), json).map_err(|e| io_error("write the build record", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_are_found_recursively_and_must_stay_inside() {
        let tmp = tempfile::tempdir().unwrap();
        let src = dunce::canonicalize(tmp.path()).unwrap().join("src");
        fs::create_dir_all(src.join("rtl")).unwrap();
        fs::write(
            src.join("main.v"),
            "`include \"defs.vh\"\nmodule main; endmodule\n",
        )
        .unwrap();
        fs::write(src.join("defs.vh"), "  `include \"rtl/more.vh\"\n").unwrap();
        fs::write(
            src.join("rtl/sub.v"),
            "`include \"local.vh\"\n`include \"defs.vh\"\n",
        )
        .unwrap();
        fs::write(src.join("rtl/local.vh"), "").unwrap();
        fs::write(src.join("rtl/more.vh"), "// `include \"commented.vh\"\n").unwrap();
        let found = find_includes(&src, &["main.v".into(), "rtl/sub.v".into()]).unwrap();
        let found: Vec<_> = found.into_iter().collect();
        assert_eq!(found, vec!["defs.vh", "rtl/local.vh", "rtl/more.vh"]);

        // Not found: left for yosys to report.
        fs::write(src.join("x.v"), "`include \"missing.vh\"\n").unwrap();
        assert!(find_includes(&src, &["x.v".into()]).unwrap().is_empty());

        // Outside the source folder.
        fs::write(tmp.path().join("outside.vh"), "").unwrap();
        fs::write(src.join("y.v"), "`include \"../outside.vh\"\n").unwrap();
        let err = find_includes(&src, &["y.v".into()]).unwrap_err();
        assert_eq!(err.category, Category::Request);
        assert!(err.message.contains("outside the source folder"));
    }
}
