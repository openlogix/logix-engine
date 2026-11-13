//! The build tool (docs/tool-api.md, Tables 6 to 8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Call, EngineTool};
use crate::board;
use crate::build::{self, BuildRequest};
use crate::errors::{Diagnostic, ToolError};
use crate::paths;
use crate::state::EngineState;

pub struct Build;

impl EngineTool for Build {
    const NAME: &'static str = "build";
    const DESCRIPTION: &'static str = "Builds an FPGA design into a bitstream: synthesizes the \
        Verilog or SystemVerilog sources, places and routes them on the board's FPGA using the \
        constraint file, which maps top-level ports to pins (.pcf for iCE40, .lpf for ECP5, .cst for Gowin, .xdc for Xilinx), and packs the bitstream into \
        the build folder. Give the source files explicitly; the engine doesn't search for them, \
        though `include files are found next to the including file or in the source folder. A \
        build with no changes since the last one returns at once with up_to_date set. The first \
        build may download the toolchain package first, which takes a few minutes. Design \
        problems come back as a design error with diagnostics at file and line. After a \
        successful build, program_board programs the bitstream into the board.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = BuildInput;
    type Output = BuildOutput;
}

/// What to build, and where.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildInput {
    /// The target board's ID: a catalog ID such as icesugar-1-5 (get_catalog
    /// lists them), or the ID of a board in custom_boards_folder.
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
    /// Name of the top-level module, such as main.
    pub top_module: String,
    /// The folder with the design files, as an absolute path.
    pub source_folder: String,
    /// The HDL files to build (.v or .sv), relative to the source folder,
    /// with / as the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// testbenches or included files.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The constraint file, relative to the source folder: .pcf for iCE40,
    /// .lpf for ECP5, .cst for Gowin, .xdc for Xilinx. Another extension is
    /// refused, since each family's tools read only their own format.
    pub constraint_file: String,
    /// Where the build writes its files, as an absolute path. One of its last
    /// two folder names must contain "build", such as
    /// /home/user/proj/_build. It may be inside the source folder but
    /// can't be the source folder itself. It is created if needed.
    pub build_folder: String,
    /// Verilog macro definitions, from name to value, such as
    /// {"USE_PLL": "", "WIDTH": "8"}. An empty value defines the macro with
    /// no text, for `ifdef. SYNTHESIZE is always defined.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Let the tools print more detail, shown in the live output and the
    /// logs. It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// The build result.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildOutput {
    /// Path of the bitstream, relative to the build folder.
    pub bitstream: String,
    /// The board variant the build is for, by its full selector: the one
    /// board_variant picked (the board's first with its values), which
    /// program_board programs by default.
    pub board_variant: crate::catalog::Selector,
    /// True if nothing had changed since the last build, so nothing was
    /// rebuilt.
    pub up_to_date: bool,
    /// Every resource the toolchain reports, sorted by name (ignoring
    /// case): its own name, such as ICESTORM_LC or SLICE_LUTX, a short
    /// description, how many are used and available, and the share used
    /// (None when nextpnr's numbers aren't valid). Many are unused, on
    /// Xilinx FPGAs most: a host shows the used ones first.
    pub resources: Vec<crate::build::summary::Resource>,
    /// The main resources in plain words, for showing to people without
    /// knowing each FPGA family's names: the family's main capacity measure
    /// first (logic cells or LUTs), then flip-flops, RAM blocks,
    /// multipliers, PLLs and I/O pins, each the FPGA has, such as "27 of
    /// 7680 logic cells", with its share.
    pub resources_summary: Vec<crate::build::summary::ResourceSummary>,
    /// The design's clocks, each with a plain name, usually the design's
    /// clock signal, such as "CLK" for the net CLK$SB_IO_IN_$glb_clk, the
    /// net nextpnr reports, and the maximum frequency it can run at in MHz,
    /// rounded to 0.01. A clock that misses its target isn't an error:
    /// compare these with the design's needs.
    pub clocks_summary: Vec<crate::build::summary::ClockSummary>,
    /// The report for people, the preferred way to show a build's resource
    /// use to people, a line each: the main resource always, the
    /// others when used, then each clock, such as "The design uses 27 of
    /// 7680 logic cells (1%)." and "Maximum clock speed: 194.4 MHz
    /// (sys_clk).". Show it as is; its wording can change in any version.
    pub report: String,
    /// The report in one line, for a status bar, such as "Uses 1% of
    /// logic cells, max clock 194.4 MHz." Empty when there's nothing to
    /// report.
    pub report_summary: String,
    /// Warnings from the tools.
    pub warnings: Vec<Diagnostic>,
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

fn identifier() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_$]*$").unwrap())
}

/// Checks a top module name.
pub fn check_top_module(top_module: &str) -> Result<(), ToolError> {
    if top_module.is_empty()
        || top_module
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return Err(ToolError::request(format!(
            "top_module {top_module:?} isn't a module name."
        )));
    }
    Ok(())
}

/// Checks macro definitions: Verilog identifiers with single-line values.
pub fn check_defines(defines: &BTreeMap<String, String>) -> Result<(), ToolError> {
    for (name, value) in defines {
        if !identifier().is_match(name) {
            return Err(ToolError::request(format!(
                "The define name {name:?} isn't a Verilog identifier."
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(ToolError::request(format!(
                "The value of define {name} must be a single line of text."
            )));
        }
    }
    Ok(())
}

/// Checks the source folder and returns its real path.
pub fn check_source_folder(source_folder: &str) -> Result<PathBuf, ToolError> {
    let req = |m: String| ToolError::request(m);
    let src = paths::check_absolute_path(source_folder)
        .map_err(|e| req(format!("source_folder: {e}.")))?;
    let src = dunce::canonicalize(&src).map_err(|e| {
        req(format!(
            "source_folder {source_folder:?} can't be found: {e}."
        ))
    })?;
    if !src.is_dir() {
        return Err(req(format!(
            "source_folder {source_folder:?} isn't a folder."
        )));
    }
    Ok(src)
}

/// Checks that `rel`, from the input field `field`, names a file inside the
/// source folder `src` (a real path).
pub fn check_source_file(src: &Path, field: &str, rel: &str) -> Result<(), ToolError> {
    let req = |m: String| ToolError::request(m);
    paths::check_relative_path(rel).map_err(|e| req(format!("{field}: {e}.")))?;
    let real = paths::resolve_inside(src, rel).map_err(|e| req(format!("{field}: {e}.")))?;
    if !real.is_file() {
        return Err(req(format!("{field}: {rel:?} isn't a file.")));
    }
    Ok(())
}

/// Checks that the constraint file's extension is its FPGA family's format
/// (any case). The tools don't check it: nextpnr-ice40 reads any file given
/// to `--pcf` as PCF, so a misnamed file would otherwise build as if the
/// name were right, or fail with a puzzling parse error.
pub fn check_constraint_extension(
    board: &board::ResolvedBoard,
    rel: &str,
) -> Result<(), ToolError> {
    let family = board.fpga.family;
    let wanted = family.constraint_extension();
    let ext = Path::new(rel).extension().and_then(|e| e.to_str());
    if ext.is_some_and(|e| e.eq_ignore_ascii_case(wanted)) {
        return Ok(());
    }
    // No suggested fix: each family's pin file format is different, so
    // renaming the file is rarely all it takes.
    Err(ToolError::request(format!(
        "The pin file {rel} is the wrong kind for the {}: {} FPGAs use .{wanted} pin files.",
        board.description,
        family.label(),
    )))
}

/// Checks a build folder against the path rules and returns its real path.
pub fn check_build_folder(build_folder: &str, src: &Path) -> Result<PathBuf, ToolError> {
    paths::check_build_folder(build_folder, paths::home_folder().as_deref(), Some(src))
        .map_err(|e| ToolError::request(format!("build_folder: {e}.")))
}

/// Checks the request and turns it into a BuildRequest.
pub fn check(input: BuildInput) -> Result<BuildRequest, ToolError> {
    check_top_module(&input.top_module)?;
    check_defines(&input.defines)?;
    let src = check_source_folder(&input.source_folder)?;
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let board = board::resolve(&input.board, &input.board_variant, &custom)?;
    if input.source_files.is_empty() {
        return Err(ToolError::request("source_files is empty."));
    }
    for f in &input.source_files {
        check_source_file(&src, "source_files", f)?;
    }
    check_source_file(&src, "constraint_file", &input.constraint_file)?;
    check_constraint_extension(&board, &input.constraint_file)?;
    let build_folder = check_build_folder(&input.build_folder, &src)?;
    Ok(BuildRequest {
        board,
        top_module: input.top_module,
        source_folder: src,
        source_files: input.source_files,
        constraint_file: input.constraint_file,
        build_folder,
        defines: input.defines,
        verbose: input.verbose,
    })
}

pub async fn run(
    state: &EngineState,
    input: BuildInput,
    call: &Call,
) -> Result<BuildOutput, ToolError> {
    let request = check(input)?;
    let arch = request.board.fpga.toolchain();
    let board_variant = request.board.variant.selector.clone();
    let r = build::build(state, request, call).await?;
    let summary = crate::build::summary::summarize(arch, &r.resources_usage);
    let clocks = crate::build::summary::summarize_clocks(arch, &r.timing);
    Ok(BuildOutput {
        resources: crate::build::summary::resources(arch, &r.resources_usage),
        report: crate::build::summary::report(&summary, &clocks),
        report_summary: crate::build::summary::report_summary(&summary, &clocks),
        resources_summary: summary,
        clocks_summary: clocks,
        commands: call.commands.take(),
        bitstream: r.bitstream,
        board_variant,
        up_to_date: r.up_to_date,
        warnings: r.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(id: &str) -> board::ResolvedBoard {
        board::resolve(id, &Default::default(), &Default::default()).unwrap()
    }

    #[test]
    fn constraint_file_must_match_the_family() {
        let ice40 = board("alhambra-ii");
        assert!(check_constraint_extension(&ice40, "pinout.pcf").is_ok());
        assert!(check_constraint_extension(&ice40, "pins/Pinout.PCF").is_ok());
        for bad in ["pinout.lpf", "pinout", "pinout.pcf.txt", "pcf"] {
            let err = check_constraint_extension(&ice40, bad).unwrap_err();
            assert_eq!(err.category, crate::errors::Category::Request, "{bad}");
        }
        let err = check_constraint_extension(&ice40, "pins/pinout.lpf").unwrap_err();
        assert_eq!(
            err.message,
            "The pin file pins/pinout.lpf is the wrong kind for the Alhambra II: iCE40 FPGAs use .pcf pin files."
        );
        // No suggested fix: the formats differ, so a rename isn't enough.
        assert!(err.action.is_none());
        assert!(err.data.is_none());
        assert!(check_constraint_extension(&board("icesugar-pro"), "main.lpf").is_ok());
        assert!(check_constraint_extension(&board("sipeed-tang-nano-9k"), "main.cst").is_ok());
        assert!(check_constraint_extension(&board("arty-a7-35t"), "main.xdc").is_ok());
        assert!(check_constraint_extension(&board("arty-a7-35t"), "main.pcf").is_err());
    }
}
