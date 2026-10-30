//! The iCE40 build (docs/tool-api.md, "build tool"): yosys, nextpnr-ice40 and
//! icepack, run with the same commands as Apio, so the bitstreams match.
//!
//! Build folder files: the marker `.logix-build` (written first, it tells
//! clean_build the folder is a build folder), `hardware.json` (yosys),
//! `hardware.asc` and `hardware.pnr` (nextpnr's placed design and report),
//! `hardware.bin` (the bitstream), the logs `synth.log` and `pnr.log`, and
//! the build record `build-record.json` (written last, on success only).
//! These names are a fixed convention of the engine, documented in the tool
//! API, so hosts can rely on them.

pub mod clean;
pub mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::board::ResolvedBoard;
use crate::errors::{Category, Diagnostic, ToolError};
use crate::progress::OutputLevel;
use crate::progress::Phases;
use crate::runner::{RunError, run_tool_with};
use crate::state::EngineState;
use crate::text::to_multiline_text;
use crate::tools::Call;
use parse::ResourceUsage;

pub const MARKER: &str = ".logix-build";
pub const RECORD: &str = "build-record.json";
pub const BITSTREAM: &str = "hardware.bin";
/// Every file a build makes, deleted before a build runs again.
pub const OUTPUTS: &[&str] = &[
    RECORD,
    "hardware.json",
    "hardware.asc",
    "hardware.pnr",
    BITSTREAM,
    "synth.log",
    "pnr.log",
];
/// The package with the iCE40 tools.
pub const TOOLCHAIN: &str = "oss-cad-suite";

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
            let real = source_folder.join(hit).canonicalize().map_err(|e| {
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
    toolchain_version: &str,
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
        "toolchain": [TOOLCHAIN, toolchain_version],
        "board": req.board,
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

/// One build step's tool run, turned into an error when it failed.
async fn run_step(
    step: &str,
    tool: &str,
    bin_dir: &Path,
    args: Vec<OsString>,
    cwd: &Path,
    call: &Call,
) -> Result<String, ToolError> {
    let program = bin_dir.join(tool);
    let on_line = |line: &str| call.output.line(tool, OutputLevel::of(line), line);
    match run_tool_with(&program, &args, cwd, bin_dir, &call.cancel, &on_line).await {
        Ok(run) if run.status.success() => Ok(run.output),
        Ok(run) => {
            let mut err = ToolError::new(
                Category::Tool,
                format!("{tool} failed while {step} ({}).", run.status),
            );
            err.details = to_multiline_text(&run.output);
            Err(err)
        }
        Err(RunError::Cancelled) => Err(ToolError::cancelled("The build was cancelled.")),
        Err(RunError::Spawn(e)) => {
            Err(
                ToolError::new(Category::Tool, format!("Can't start {tool}: {e}")).with_action(
                    crate::errors::ActionCode::InstallPackages,
                    format!("Reinstall the {TOOLCHAIN} package, which may be damaged."),
                ),
            )
        }
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
    let spec = state
        .packages
        .manifest()
        .get(TOOLCHAIN)
        .cloned()
        .ok_or_else(|| ToolError::engine(format!("{TOOLCHAIN} isn't in the manifest.")))?;
    let checksum = inputs_checksum(&req, &includes, &spec.version)?;

    if let Some(record) = read_record(&req.build_folder)
        && record.inputs_checksum == checksum
        && req.build_folder.join(&record.result.bitstream).is_file()
    {
        log::info!("Build in {} is up to date", req.build_folder.display());
        let mut result = record.result;
        result.up_to_date = true;
        return Ok(result);
    }

    let needed = state.packages.needing_install(&[TOOLCHAIN])?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 3);
    state
        .packages
        .ensure(&needed, state.auto_install, &phases, &call.cancel)
        .await?;
    let bin = state.packages.package_dir(&spec).join("bin");

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
    let mut args: Vec<OsString> = vec![
        "-q".into(),
        "-l".into(),
        out.join("synth.log").into(),
        "-p".into(),
        format!(
            "synth_ice40 -top {} -json \"{}\"",
            req.top_module,
            out.join("hardware.json").display()
        )
        .into(),
        "-DSYNTHESIZE".into(),
    ];
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
    let fpga = &req.board.fpga;
    let args: Vec<OsString> = vec![
        format!("--{}", fpga.nextpnr_device).into(),
        "--package".into(),
        fpga.nextpnr_package.clone().into(),
        "--json".into(),
        "hardware.json".into(),
        "--asc".into(),
        "hardware.asc".into(),
        "--report".into(),
        "hardware.pnr".into(),
        "--pcf".into(),
        req.source_folder.join(&req.constraint_file).into(),
        "-q".into(),
        "--log".into(),
        "pnr.log".into(),
    ];
    let pnr = run_step(
        "placing and routing",
        "nextpnr-ice40",
        &bin,
        args,
        out,
        call,
    )
    .await;
    let pnr_out = match &pnr {
        Ok(o) => o.clone(),
        Err(e) => e.details.clone(),
    };
    let parsed = parse::parse_nextpnr(&pnr_out, &req.constraint_file);
    warnings.extend(parsed.warnings);
    pnr.map_err(|e| design_error(e, parsed.errors))?;

    // Bitstream.
    phases.start("Packing bitstream");
    let args: Vec<OsString> = vec!["hardware.asc".into(), BITSTREAM.into()];
    run_step("packing the bitstream", "icepack", &bin, args, out, call).await?;

    let report = fs::read_to_string(out.join("hardware.pnr"))
        .map_err(|e| ToolError::new(Category::Tool, format!("nextpnr wrote no report: {e}")))?;
    let (resources_usage, timing) = parse::parse_report(&report)
        .map_err(|e| ToolError::new(Category::Tool, format!("Can't read nextpnr's report: {e}")))?;

    let result = BuildResult {
        bitstream: BITSTREAM.into(),
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
    let json = serde_json::to_string_pretty(&record).unwrap_or_default() + "\n";
    fs::write(out.join(RECORD), json).map_err(|e| io_error("write the build record", e))?;
    log::info!("Build in {} done", out.display());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_are_found_recursively_and_must_stay_inside() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().canonicalize().unwrap().join("src");
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
