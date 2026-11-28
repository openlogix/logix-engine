//! list_nets: the nets of a design that a debug file can name, so a host
//! can offer them to choose from (such as Logix IDE's net picker).
//!
//! It runs yosys's read and elaboration only, the same first steps as the
//! debug build's pre-pass (read, hierarchy, proc, flatten), and lists the
//! top module's named nets after flattening, named as debug files name them:
//! from the top module, with a dot between instance levels
//! (`top.u_counter.count`).

use std::collections::BTreeMap;
use std::ffi::OsString;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::debug::build::{cells_sim, read_json};
use crate::debug::pads::unreachable;
use crate::errors::{Category, ToolError};
use crate::state::EngineState;
use crate::tools::build as engine_build;
use crate::tools::{Call, EngineTool};

pub struct ListNets;

impl EngineTool for ListNets {
    const NAME: &'static str = "list_nets";
    const DESCRIPTION: &'static str = "Lists the nets of a design that a debug file can watch \
        or drive: each named net of the top module after the design is read and flattened, \
        with its path as debug files name it (from the top module, such as \
        top.u_counter.count), its width in bits, its kind (input, output or inout for the \
        top module's ports, internal otherwise), whether a debug file can watch it and drive \
        it, and why_not, the reason when it can't do both (a port of the top module can be \
        watched but not driven; a net wired to a cell port only the FPGA's pin may take, \
        such as the iCE40 UltraPlus RGB LED driver's outputs, can be neither). It only reads and elaborates the design (no \
        synthesis), so it is quick. The board is needed for its FPGA's cell library.";
    const READ_ONLY: bool = true;
    type Input = ListNetsInput;
    type Output = ListNetsOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListNetsInput {
    /// The target board's ID, as for build.
    pub board: String,
    /// The board variant, as for build.
    #[serde(default)]
    pub board_variant: Option<crate::catalog::VariantChoice>,
    /// A folder of custom boards, as for build.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// The design's top module.
    pub top_module: String,
    /// The absolute folder the source files are in.
    pub source_folder: String,
    /// The design's Verilog files, relative to source_folder.
    pub source_files: Vec<String>,
    /// Verilog defines, as for build.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNetsOutput {
    /// The nets, sorted by path.
    pub nets: Vec<Net>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Net {
    /// Its path as debug files name it, from the top module.
    pub path: String,
    /// Its width in bits.
    pub width: usize,
    /// input, output or inout for a port of the top module; internal otherwise.
    pub kind: String,
    /// Whether a debug file can watch it.
    pub can_watch: bool,
    /// Whether a debug file can drive it.
    pub can_drive: bool,
    /// Why it can't be watched or driven, when it can't be both.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
}

pub async fn run(
    state: &EngineState,
    input: ListNetsInput,
    call: &Call,
) -> Result<ListNetsOutput, ToolError> {
    engine_build::check_top_module(&input.top_module)?;
    engine_build::check_defines(&input.defines)?;
    let src = engine_build::check_source_folder(&input.source_folder)?;
    if input.source_files.is_empty() {
        return Err(ToolError::request("source_files is empty."));
    }
    for f in &input.source_files {
        engine_build::check_source_file(&src, "source_files", f)?;
    }
    state.ensure_catalog(call).await?;
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let board = crate::board::resolve(&input.board, &input.board_variant, &custom)?;
    let toolchain = board.fpga.family.toolchain();

    let needed = state
        .packages
        .needing_download(&[crate::packages::CORE])
        .await?;
    if !needed.is_empty() {
        let phases = crate::progress::Phases::new(call.progress.clone(), needed.len());
        state
            .packages
            .ensure(&needed, state.auto_download, &phases, &call.cancel)
            .await?;
    }
    let bin = state.packages.bin_dir(crate::build::TOOLCHAIN)?;

    let tmp = tempfile::tempdir()
        .map_err(|e| ToolError::engine(format!("Can't make a temporary folder: {e}")))?;
    let json_path = tmp.path().join("nets.json");
    let script = format!(
        "read_verilog -lib {cells}; hierarchy -top {top}; proc -noopt; flatten; \
         select {top}; write_json -selected \"{json}\"",
        cells = cells_sim(toolchain),
        top = input.top_module,
        json = json_path.to_string_lossy()
    );
    let mut args: Vec<OsString> = vec![
        "-q".into(),
        "-p".into(),
        script.into(),
        "-DSYNTHESIZE".into(),
        // As in a debug build, whose nets these are.
        "-DLOGIX_DEBUG".into(),
    ];
    for (name, value) in &input.defines {
        args.push(format!("-D{name}={value}").into());
    }
    args.extend(input.source_files.iter().map(OsString::from));
    let program = crate::commands::program_in(&bin, "yosys");
    let run = crate::runner::run_tool(&program, &args, &src, &bin, &call.cancel)
        .await
        .map_err(|e| match e {
            crate::runner::RunError::Cancelled => ToolError::cancelled("list_nets was cancelled."),
            crate::runner::RunError::Spawn(e) => ToolError::setup(format!("Can't run yosys: {e}")),
        })?;
    if !run.status.success() {
        let tail: Vec<&str> = run
            .output
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let tail = tail[tail.len().saturating_sub(10)..].join("\n");
        return Err(ToolError::new(
            Category::Design,
            format!("yosys couldn't read the design:\n{tail}"),
        ));
    }
    let json = read_json(&json_path)?;
    Ok(ListNetsOutput {
        nets: nets(&json, &input.top_module),
    })
}

/// The top module's named nets in yosys's JSON, named from the top module.
fn nets(json: &serde_json::Value, top: &str) -> Vec<Net> {
    let module = &json["modules"][top];
    let mut nets: Vec<Net> = module["netnames"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, n)| n["hide_name"].as_u64() != Some(1) && !name.starts_with('$'))
        .map(|(name, n)| {
            let width = n["bits"].as_array().map_or(0, |b| b.len());
            let kind = module["ports"][name]["direction"]
                .as_str()
                .unwrap_or("internal")
                .to_string();
            let pad = unreachable(module, &n["bits"], top);
            let port = kind != "internal";
            let why_not = pad.clone().or_else(|| {
                port.then(|| {
                    "It is a port of the top module: debug files can watch it, but drive internal nets only.".to_string()
                })
            });
            Net {
                path: format!("{top}.{name}"),
                width,
                kind,
                can_watch: pad.is_none(),
                can_drive: pad.is_none() && !port,
                why_not,
            }
        })
        .collect();
    nets.sort_by(|a, b| a.path.cmp(&b.path));
    nets
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_named_nets_from_the_top() {
        let j = json!({"modules": {"top": {
            "ports": {"clk": {"direction": "input", "bits": [2]}},
            "netnames": {
                "clk": {"hide_name": 0, "bits": [2]},
                "u_cnt.count": {"hide_name": 0, "bits": [3, 4, 5, 6]},
                "$auto$1": {"hide_name": 1, "bits": [7]}
            }
        }}});
        let n = nets(&j, "top");
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].path, "top.clk");
        assert_eq!(n[0].kind, "input");
        assert_eq!(n[1].path, "top.u_cnt.count");
        assert_eq!(n[1].width, 4);
        assert_eq!(n[1].kind, "internal");
        assert!(n[0].can_watch && !n[0].can_drive && n[0].why_not.is_some());
        assert!(n[1].can_watch && n[1].can_drive && n[1].why_not.is_none());
    }

    #[test]
    fn nets_on_pad_only_ports_can_be_neither() {
        let j = json!({"modules": {"top": {
            "ports": {"rgb": {"direction": "output", "bits": [2, 3, 4]}},
            "cells": {"drv": {"type": "SB_RGBA_DRV", "connections": {
                "RGB0": [2], "RGB1": [3], "RGB2": [4], "RGB0PWM": [5]
            }}},
            "netnames": {
                "rgb": {"hide_name": 0, "bits": [2, 3, 4]},
                "on": {"hide_name": 0, "bits": [5]}
            }
        }}});
        let n = nets(&j, "top");
        assert_eq!(n[0].path, "top.on");
        assert!(n[0].can_watch && n[0].can_drive);
        assert_eq!(n[1].path, "top.rgb");
        assert!(!n[1].can_watch && !n[1].can_drive);
        assert!(n[1].why_not.as_deref().unwrap().contains("top.on"));
    }
}
