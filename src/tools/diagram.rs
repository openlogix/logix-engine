//! The diagram tool (docs/tool-api.md, "diagram tool").

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::{
    check_build_folder, check_defines, check_source_file, check_source_folder, check_top_module,
};
use super::{Call, EngineTool};
use crate::diagram::{self, DiagramRequest};
use crate::errors::{Diagnostic, ToolError};
use crate::state::EngineState;

pub struct Diagram;

impl EngineTool for Diagram {
    const NAME: &'static str = "diagram";
    const DESCRIPTION: &'static str = "Draws the design as schematic diagrams, one per module: \
        registers, adders, comparators, multiplexers, memories, the FPGA's own blocks and \
        other modules as symbols, with the wires between them. Yosys reads the design as \
        written, without synthesizing it, and the engine lays out and draws every module \
        itself. Writes, in the build folder's diagram/ folder (replaced by each call): \
        diagram.html, one self-contained page with every module (click a module to open it, \
        drag to pan, scroll to zoom, hover anything for its details; it needs no network or \
        other software, so any web browser shows it), each module's SVG, and diagram.json, a \
        model of every module's blocks, ports, signals and source lines. To show the user \
        the diagram, set open to true, which opens the page in their web browser, or give \
        them page_file's path. List the design files; the engine doesn't search for them. \
        Errors in the design, such as a syntax error or a top module that isn't in the \
        files, are design errors with diagnostics at file and line. The first call may \
        download the toolchain package first, which takes a few minutes.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = DiagramInput;
    type Output = DiagramOutput;
}

/// What to draw.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagramInput {
    /// The target board's ID: a catalog ID such as icesugar-1-5 (get_catalog
    /// lists them), or the ID of a board in custom_boards_folder. Its FPGA
    /// family names the FPGA's own blocks, such as a PLL or an I/O pin.
    pub board: String,
    /// The board variant: some or all of the selector of one of the board's
    /// variants, which get_catalog lists. It doesn't change the diagram.
    /// Omitted or {} means its first variant.
    #[serde(default)]
    pub board_variant: Option<crate::catalog::VariantChoice>,
    /// The host's own boards: the absolute path of a folder of board files,
    /// one board each in the catalog's format, named by board ID. board can
    /// name one of them. Send the same folder to every tool. Omitted means
    /// none.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// The module the diagram starts at: the design's top module, such as
    /// main, or any module of the design files, such as uart, drawn with its
    /// parameters' default values. The modules it uses are drawn too, down
    /// to levels.
    pub top_module: String,
    /// The folder with the design files, as an absolute path.
    pub source_folder: String,
    /// The design files (.v or .sv), relative to the source folder, with /
    /// as the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// included files or testbenches.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The build folder, as an absolute path. The diagram is written to its
    /// diagram/ folder. It may or may not be inside the source folder, and
    /// is created if needed.
    pub build_folder: String,
    /// Verilog macro definitions, from name to value, such as
    /// {"WIDTH": "8"}. An empty value defines the macro with no text, for
    /// `ifdef. SYNTHESIZE is always defined, as in build.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Open the page in the user's default web browser when it's written,
    /// so they see the diagram at once. Defaults to false. opened says
    /// whether it was.
    #[serde(default)]
    pub open: bool,
    /// How many levels of modules below top_module are drawn, such as 1
    /// for top_module and the modules it uses. Deeper modules are in the
    /// model with a note, not drawn; a diagram with top_module set to one of
    /// them shows it. Defaults to 3.
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub levels: Option<usize>,
    /// Let yosys print more detail, shown in the live output and the logs.
    /// It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
    /// How to name paths in what people read (diagnostics, messages,
    /// testbench output, live tool output, the diagram's source places):
    /// files named as a whole, then folders whose files keep their path
    /// below the folder under the folder's new name ("" gives names relative
    /// to it); the first match in each list wins. Paths are compared by real
    /// location. Never applied to command lines or fields that are paths.
    /// Omitted means paths read as the request gave them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_paths: Option<crate::display_paths::DisplayPaths>,
}

/// One module of the diagram.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagramModule {
    /// The module's key in diagram.json and the page, such as main or
    /// uart. Usually its name; a module used with different parameters gets
    /// one key per version, such as genrom and genrom-2.
    pub key: String,
    /// The module's name.
    pub name: String,
    /// Whether it's the top module.
    pub top: bool,
    /// Its SVG, relative to the build folder, such as diagram/main.svg.
    /// Omitted when it wasn't drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svg_file: Option<String>,
    /// How many blocks its diagram has: cells, pins, labels, bus splits and
    /// joins.
    pub blocks: usize,
    /// Why it wasn't drawn, such as a module too large to draw readably.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What diagram made.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagramOutput {
    /// The page with every module, relative to the build folder:
    /// diagram/diagram.html. Opening it in a web browser shows the diagram.
    pub page_file: String,
    /// The model, relative to the build folder: diagram/diagram.json. For
    /// each module: its blocks (each with its kind, plain title, yosys type,
    /// source file and line, ports and what they connect to), labels, pins
    /// and signals.
    pub model_file: String,
    /// Every module, the top module first.
    pub modules: Vec<DiagramModule>,
    /// Whether the page was opened in the web browser (open).
    pub opened: bool,
    /// Yosys warnings while reading the design, at file and line where
    /// known. Often empty.
    pub diagnostics: Vec<Diagnostic>,
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

/// Checks the request and turns it into a DiagramRequest.
pub fn check(input: DiagramInput) -> Result<DiagramRequest, ToolError> {
    let req = |m: String| ToolError::request(m);
    check_top_module(&input.top_module)?;
    check_defines(&input.defines)?;
    let src = check_source_folder(&input.source_folder)?;
    // Yosys reads the files from a script, where a double quote would end
    // the file name.
    if src.to_string_lossy().contains('"') {
        return Err(req(
            "source_folder: the path has a double quote (\"), which yosys can't read.".into(),
        ));
    }
    if input.source_files.is_empty() {
        return Err(req("source_files is empty.".into()));
    }
    let mut seen = BTreeSet::new();
    for f in &input.source_files {
        check_source_file(&src, "source_files", f)?;
        if f.contains('"') {
            return Err(req(format!(
                "source_files: {f:?} has a double quote (\"), which yosys can't read."
            )));
        }
        if !seen.insert(f) {
            return Err(req(format!(
                "source_files: {f:?} is listed more than once."
            )));
        }
    }
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let board = crate::board::resolve(&input.board, &input.board_variant, &custom)?;
    let build_folder = check_build_folder(&input.build_folder, &src)?;
    Ok(DiagramRequest {
        source_folder: src,
        source_files: input.source_files,
        build_folder,
        top_module: input.top_module,
        defines: input.defines,
        board,
        verbose: input.verbose,
        open: input.open,
        levels: input.levels.unwrap_or(diagram::DEFAULT_LEVELS),
    })
}

pub async fn run(
    state: &EngineState,
    input: DiagramInput,
    call: &Call,
) -> Result<DiagramOutput, ToolError> {
    let r = diagram::diagram(state, check(input)?, call).await?;
    Ok(DiagramOutput {
        page_file: r.page_file,
        model_file: r.model_file,
        modules: r
            .modules
            .into_iter()
            .map(|m| DiagramModule {
                key: m.key,
                name: m.name,
                top: m.top,
                svg_file: m.svg_file,
                blocks: m.blocks,
                note: m.note,
            })
            .collect(),
        opened: r.opened,
        diagnostics: r.diagnostics,
        commands: call.commands.take(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Category;
    use serde_json::json;
    use std::path::Path;

    fn input(src: &Path, extra: serde_json::Value) -> DiagramInput {
        let mut v = json!({
            "board": "icesugar-1-5",
            "top_module": "main",
            "source_folder": src,
            "source_files": ["main.v"],
            "build_folder": src.join("_build"),
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    fn project() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = dunce::canonicalize(tmp.path()).unwrap();
        std::fs::create_dir_all(src.join("rtl")).unwrap();
        // Windows doesn't allow a double quote in a file name.
        let quote = if cfg!(windows) {
            None
        } else {
            Some("q\"uote.v")
        };
        for f in ["main.v", "rtl/uart.sv"].into_iter().chain(quote) {
            std::fs::write(src.join(f), "").unwrap();
        }
        (tmp, src)
    }

    #[test]
    fn a_good_request() {
        let (_tmp, src) = project();
        let req = check(input(
            &src,
            json!({"source_files": ["main.v", "rtl/uart.sv"], "defines": {"W": "8"}, "open": true}),
        ))
        .unwrap();
        assert_eq!(req.source_folder, src);
        assert_eq!(req.build_folder, src.join("_build"));
        assert_eq!(req.source_files.len(), 2);
        assert!(req.open);
        assert_eq!(req.board.fpga.toolchain(), crate::catalog::Toolchain::Ice40);
    }

    #[test]
    fn request_errors() {
        let (_tmp, src) = project();
        for (extra, needle) in [
            (json!({"source_files": []}), "source_files is empty"),
            (json!({"source_files": ["nope.v"]}), "can't be found"),
            (json!({"source_files": ["../x.v"]}), "source_files"),
            // On Windows such a file can't exist, so it can't be found.
            (
                json!({"source_files": ["q\"uote.v"]}),
                if cfg!(windows) {
                    "can't be found"
                } else {
                    "double quote"
                },
            ),
            (
                json!({"source_files": ["main.v", "main.v"]}),
                "listed more than once",
            ),
            (json!({"top_module": "a b"}), "top_module"),
            (json!({"defines": {"1x": ""}}), "define name"),
            (json!({"build_folder": "rel"}), "build_folder"),
            (json!({"source_folder": "rel"}), "source_folder"),
            (json!({"board": "no-such-board"}), "no-such-board"),
        ] {
            let err = check(input(&src, extra.clone())).unwrap_err();
            assert_eq!(err.category, Category::Request, "{extra}");
            assert!(err.message.contains(needle), "{extra}: {}", err.message);
        }
        // The old graph tool's format field is gone.
        let v = json!({"board": "icesugar-1-5", "top_module": "main", "source_folder": src,
            "source_files": ["main.v"], "build_folder": src.join("_build"), "format": "svg"});
        assert!(serde_json::from_value::<DiagramInput>(v).is_err());
    }
}
