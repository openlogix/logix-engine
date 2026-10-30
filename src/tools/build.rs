//! The build tool (docs/tool-api.md, Tables 6 to 8).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Call, EngineTool};
use crate::board::{self, BoardSpec};
use crate::build::{self, BuildRequest, parse::ResourceUsage};
use crate::errors::{Diagnostic, ToolError};
use crate::paths;
use crate::state::EngineState;

pub struct Build;

impl EngineTool for Build {
    const NAME: &'static str = "build";
    const DESCRIPTION: &'static str = "Builds an FPGA design into a bitstream: synthesizes the \
        Verilog or SystemVerilog sources, places and routes them on the board's FPGA using the \
        constraint file (.pcf, which maps top-level ports to pins), and packs the bitstream into \
        the build folder. Give the source files explicitly; the engine doesn't search for them, \
        though `include files are found next to the including file or in the source folder. A \
        build with no changes since the last one returns at once with up_to_date set. The first \
        build may install the toolchain package first, which takes a few minutes. Design \
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
    /// The target board: a catalog ID such as alhambra-ii (get_catalog lists
    /// them), or a full board definition.
    pub board: BoardSpec,
    /// Name of the top-level module, such as main.
    pub top_module: String,
    /// The folder with the design files, as an absolute path.
    pub source_folder: String,
    /// The HDL files to build (.v or .sv), relative to the source folder,
    /// with / as the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// testbenches or included files.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The constraint file (.pcf), relative to the source folder.
    pub constraint_file: String,
    /// Where the build writes its files, as an absolute path. One of its last
    /// two folder names must contain "build", such as
    /// /home/u/proj/_build/default. It may be inside the source folder but
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
    /// True if nothing had changed since the last build, so nothing was
    /// rebuilt.
    pub up_to_date: bool,
    /// Resource use, by the toolchain's resource name, such as ICESTORM_LC
    /// (logic cells: a LUT and a flip-flop), ICESTORM_RAM (block RAMs) and
    /// SB_IO (I/O pins).
    pub resources_usage: BTreeMap<String, ResourceUsage>,
    /// The maximum frequency each clock can run at, in MHz, by clock net
    /// name. A clock that misses its target isn't an error: compare these
    /// with the design's needs.
    pub timing: BTreeMap<String, f64>,
    /// Warnings from the tools.
    pub warnings: Vec<Diagnostic>,
}

fn identifier() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_$]*$").unwrap())
}

/// Checks the request and turns it into a BuildRequest.
pub fn check(input: BuildInput) -> Result<BuildRequest, ToolError> {
    let board = board::resolve(&input.board)?;
    let req = |m: String| ToolError::request(m);
    if input.top_module.is_empty()
        || input
            .top_module
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return Err(req(format!(
            "top_module {:?} isn't a module name.",
            input.top_module
        )));
    }
    for (name, value) in &input.defines {
        if !identifier().is_match(name) {
            return Err(req(format!(
                "The define name {name:?} isn't a Verilog identifier."
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(req(format!(
                "The value of define {name} must be a single line of text."
            )));
        }
    }
    let src = paths::check_absolute_path(&input.source_folder)
        .map_err(|e| req(format!("source_folder: {e}.")))?;
    let src = src.canonicalize().map_err(|e| {
        req(format!(
            "source_folder {:?} can't be found: {e}.",
            input.source_folder
        ))
    })?;
    if !src.is_dir() {
        return Err(req(format!(
            "source_folder {:?} isn't a folder.",
            input.source_folder
        )));
    }
    if input.source_files.is_empty() {
        return Err(req("source_files is empty.".into()));
    }
    for (field, rel) in input
        .source_files
        .iter()
        .map(|f| ("source_files", f))
        .chain(std::iter::once(("constraint_file", &input.constraint_file)))
    {
        paths::check_relative_path(rel).map_err(|e| req(format!("{field}: {e}.")))?;
        let real = paths::resolve_inside(&src, rel).map_err(|e| req(format!("{field}: {e}.")))?;
        if !real.is_file() {
            return Err(req(format!("{field}: {rel:?} isn't a file.")));
        }
    }
    let build_folder = paths::check_build_folder(
        &input.build_folder,
        paths::home_folder().as_deref(),
        Some(&src),
    )
    .map_err(|e| req(format!("build_folder: {e}.")))?;
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
    let r = build::build(state, request, call).await?;
    Ok(BuildOutput {
        bitstream: r.bitstream,
        up_to_date: r.up_to_date,
        resources_usage: r.resources_usage,
        timing: r.timing,
        warnings: r.warnings,
    })
}
