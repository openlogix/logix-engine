//! A debug build: build with debug_file, the engine's build with the
//! debugger inserted (docs/debugger.md, Build integration).
//!
//! 1. A yosys pre-pass on the user's sources flattens the design and
//!    exposes the debug file's nets as ports of the top module (`expose`,
//!    and `expose -cut` for nets the debugger drives), writing
//!    debug_src/design_debug.v.
//! 2. The engine writes debug_src/debug_top.v (the design, the debugger and
//!    the override muxes), debug_src/debug.<ext> (the user's constraints
//!    plus the UART pins, in the family's format) and
//!    debug_src/logix_debugger.v.
//! 3. The engine's own build builds those into the build folder, and
//!    debug_map.json goes next to the bitstream.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::catalog::{Toolchain, VariantChoice};
use crate::errors::{Category, Diagnostic, ToolError};
use crate::state::EngineState;
use crate::tools::Call;
use crate::tools::build as engine_build;

use super::config::{self, DebugConfig};
use super::protocol::{self, DebugMap, MapDrive, MapWatch};

/// The debugger's Verilog, written into each debug build.
const DEBUGGER_V: &str = include_str!("hdl/logix_debugger.v");

/// The folder in the build folder for the generated sources.
const GEN_FOLDER: &str = "debug_src";
/// The generated top module, and the debugger's UART ports.
const TOP: &str = "logix_debug_top";
const TX_PORT: &str = "logix_debug_tx";
const RX_PORT: &str = "logix_debug_rx";

/// The debug build's request, from build's.
struct DebugBuildInput {
    board: String,
    board_variant: Option<VariantChoice>,
    custom_boards_folder: Option<String>,
    display_paths: Option<crate::display_paths::DisplayPaths>,
    top_module: String,
    source_folder: String,
    source_files: Vec<String>,
    constraint_file: String,
    debug_file: String,
    build_folder: String,
    defines: BTreeMap<String, String>,
    verbose: bool,
}

/// A debug build's own result, build's debug.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugBuildInfo {
    /// The debug map, relative to the build folder: debug_map.json.
    pub debug_map: String,
    /// The build ID the debugger reports, 8 hex digits.
    pub build_id: String,
    /// The nets the debugger drives.
    pub drive: Vec<MapDrive>,
    /// The nets the debugger reads.
    pub watch: Vec<MapWatch>,
    /// The debugger's own share, synthesized alone.
    pub debugger: DebuggerUse,
}

/// The debugger's resources, from synthesizing it alone (an estimate:
/// placing it with the design can pack it a little differently).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebuggerUse {
    pub luts: u32,
    pub flip_flops: u32,
    /// Each as a share of the FPGA's LUTs and flip-flops (on iCE40, its
    /// logic cells, each of which has one of each), such as "8%": rounded
    /// up, so "0%" only when none. Omitted when the build didn't report
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luts_percent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_flops_percent: Option<String>,
    /// For people, such as "The debugger uses about 460 LUTs (9%) and 250
    /// flip-flops (5%) of the FPGA's 5280 logic cells."
    pub report: String,
}

/// The FPGA's LUTs and flip-flops, from the engine build's resources.
#[derive(Clone, Copy)]
struct Capacity {
    luts: u64,
    flip_flops: u64,
}

fn capacity(toolchain: Toolchain, build: &Value) -> Option<Capacity> {
    let available = |name: &str| {
        build["resources"]
            .as_array()?
            .iter()
            .find(|r| r["name"] == name)?["available"]
            .as_u64()
            .filter(|&n| n > 0)
    };
    match toolchain {
        // ICESTORM_LC: one LUT and one flip-flop each.
        Toolchain::Ice40 => available("ICESTORM_LC").map(|n| Capacity {
            luts: n,
            flip_flops: n,
        }),
        Toolchain::Ecp5 => Some(Capacity {
            luts: available("TRELLIS_COMB")?,
            flip_flops: available("TRELLIS_FF")?,
        }),
        Toolchain::Gowin => Some(Capacity {
            luts: available("LUT4")?,
            flip_flops: available("DFF")?,
        }),
        Toolchain::Openxc7 => Some(Capacity {
            luts: available("SLICE_LUTX")?,
            flip_flops: available("SLICE_FFX")?,
        }),
    }
}

/// The toolchain's folder in yosys's share, such as ice40.
fn yosys_arch(toolchain: Toolchain) -> &'static str {
    match toolchain {
        Toolchain::Ice40 => "ice40",
        Toolchain::Ecp5 => "ecp5",
        Toolchain::Gowin => "gowin",
        Toolchain::Openxc7 => "xilinx",
    }
}

/// The toolchain's yosys cell library, such as +/ice40/cells_sim.v.
pub(crate) fn cells_sim(toolchain: Toolchain) -> String {
    format!("+/{}/cells_sim.v", yosys_arch(toolchain))
}

/// The family's yosys synth command, without -top, as the engine runs it.
fn synth_command(family: crate::catalog::Family) -> String {
    let toolchain = family.toolchain();
    let mut cmd = format!("synth_{}", yosys_arch(toolchain));
    if toolchain == Toolchain::Openxc7 {
        cmd.push_str(" -arch xc7");
    } else if let Some(f) = family.yosys_family() {
        cmd.push_str(&format!(" -family {f}"));
    }
    cmd
}

/// A share, rounded up as the engine's reports do: "0%" only for none.
fn percent(used: u32, total: u64) -> String {
    format!("{}%", (used as u64 * 100).div_ceil(total))
}

/// Builds `request` with the debugger from `debug_file` inserted.
pub async fn run(
    state: &EngineState,
    request: engine_build::BuildInput,
    debug_file: String,
    call: &Call,
) -> Result<engine_build::BuildOutput, ToolError> {
    let input = DebugBuildInput {
        board: request.board,
        board_variant: request.board_variant,
        custom_boards_folder: request.custom_boards_folder,
        display_paths: request.display_paths,
        top_module: request.top_module,
        source_folder: request.source_folder,
        source_files: request.source_files,
        constraint_file: request.constraint_file,
        debug_file,
        build_folder: request.build_folder,
        defines: request.defines,
        verbose: request.verbose,
    };
    // The request, checked as build checks it.
    engine_build::check_top_module(&input.top_module)?;
    engine_build::check_defines(&input.defines)?;
    let src = engine_build::check_source_folder(&input.source_folder)?;
    if input.source_files.is_empty() {
        return Err(ToolError::request("source_files is empty."));
    }
    for f in &input.source_files {
        engine_build::check_source_file(&src, "source_files", f)?;
    }
    engine_build::check_source_file(&src, "constraint_file", &input.constraint_file)?;
    engine_build::check_source_file(&src, "debug_file", &input.debug_file)?;
    let out = engine_build::check_build_folder(&input.build_folder, &src)?;

    let config_text = read(&src.join(&input.debug_file), "the debug file")?;
    let config = config::parse(&config_text)
        .and_then(|c| in_top(c, &input.top_module))
        .map_err(|e| file_error(&input.debug_file, &e))?;

    state.ensure_catalog(call).await?;
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let board = crate::board::resolve(&input.board, &input.board_variant, &custom)?;
    let toolchain = board.fpga.family.toolchain();
    engine_build::check_constraint_extension(&board, &input.constraint_file)?;
    let pcf_text = read(&src.join(&input.constraint_file), "the constraint file")?;
    let pins = constraint_pins(toolchain, &pcf_text);
    let link = uplink(&config, &board, &pins, &input.constraint_file)?;

    // The toolchain, as the engine's build gets it.
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

    let gen_dir = out.join(GEN_FOLDER);
    std::fs::create_dir_all(&gen_dir)
        .map_err(|e| ToolError::engine(format!("Can't create {}: {e}", gen_dir.display())))?;

    // 1. The pre-pass.
    let pin_ports = pin_ports(&config, &pins, &input.constraint_file)?;
    let nets: Vec<_> = unique_nets(&config)
        .into_iter()
        .filter(|(n, _)| !pin_ports.contains_key(n))
        .collect();
    let mut design = prepass(
        &input,
        &config_text,
        toolchain,
        &nets,
        &src,
        &gen_dir,
        &bin,
        call,
    )
    .await?;
    for pin in pin_ports.keys() {
        design.widths.insert(pin.clone(), 1);
    }
    design.pin_ports = pin_ports;

    // 2. The layout, the build ID and the generated files.
    let mut map = layout(&input, &config, &design, &link)?;
    let design_v = read(&gen_dir.join("design_debug.v"), "the pre-pass output")?;
    let mut hasher = Sha256::new();
    hasher.update(design_v.as_bytes());
    // The debugger's own Verilog too: a board with an older debugger must
    // not pass as this build.
    hasher.update(DEBUGGER_V.as_bytes());
    hasher.update(serde_json::to_string(&map).unwrap_or_default().as_bytes());
    let digest = hasher.finalize();
    let id = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    map.build_id = format!("{id:08x}");

    let parts: Vec<Vec<String>> = config.watch.iter().map(|i| i.parts.clone()).collect();
    let out_parts: Vec<Vec<String>> = config.drive.iter().map(|o| o.parts.clone()).collect();
    let top_v = debug_top(
        &input.top_module,
        &design,
        &map,
        &parts,
        &out_parts,
        &link,
        id,
    );
    write(&gen_dir.join("debug_top.v"), &top_v)?;
    // The map names nets as the debug file does, from the top module.
    for i in &mut map.watch {
        if !i.net.starts_with(PIN_PREFIX) && i.members.is_empty() {
            i.net = format!("{}.{}", input.top_module, i.net);
        }
    }
    for o in &mut map.drive {
        if o.members.is_empty() {
            o.net = format!("{}.{}", input.top_module, o.net);
        }
    }
    write(&gen_dir.join("logix_debugger.v"), DEBUGGER_V)?;
    let constraints = format!(
        "{}\n{} Added by the debug build: the debugger's UART.\n{}",
        pcf_text.trim_end(),
        toolchain.constraint_comment(),
        uart_constraints(toolchain, &link)
    );
    let constraint_name = format!("debug.{}", toolchain.constraint_extension());
    write(&gen_dir.join(&constraint_name), &constraints)?;

    // 3. The engine's build of the generated files.
    let inner = engine_build::BuildInput {
        board: input.board.clone(),
        board_variant: input.board_variant.clone(),
        custom_boards_folder: input.custom_boards_folder.clone(),
        display_paths: input.display_paths.clone(),
        top_module: TOP.into(),
        source_folder: gen_dir.to_string_lossy().to_string(),
        source_files: vec![
            "design_debug.v".into(),
            "debug_top.v".into(),
            "logix_debugger.v".into(),
        ],
        constraint_file: constraint_name,
        build_folder: out.to_string_lossy().to_string(),
        defines: BTreeMap::new(),
        verbose: input.verbose,
        lint: false,
        debug_file: None,
    };
    let mut build = Box::pin(engine_build::run(state, inner, call)).await?;
    let value = serde_json::to_value(&build).unwrap_or_default();

    write(
        &out.join(protocol::MAP_FILE),
        &(serde_json::to_string_pretty(&map).unwrap_or_default() + "\n"),
    )?;
    let debugger = debugger_use(
        board.fpga.family,
        &map,
        &link,
        &gen_dir,
        &bin,
        call,
        capacity(toolchain, &value),
    )
    .await;

    // The design's own files are the ones checked, as for a normal build.
    build.checked_files = super::super::tools::checked_files(
        &src,
        &input.source_files,
        &[&input.constraint_file, &input.debug_file],
    );
    build.debug = Some(DebugBuildInfo {
        debug_map: protocol::MAP_FILE.into(),
        build_id: map.build_id.clone(),
        watch: map.watch,
        drive: map.drive,
        debugger,
    });
    Ok(build)
}

/// What a path to a package pin starts with: pin.18 is the pin the
/// constraint file puts at 18.
const PIN_PREFIX: &str = "pin.";

/// For each watched pin.<pin>, the top-level port bit the constraint file
/// puts on that pin, as a Verilog expression such as sw[0].
fn pin_ports(
    config: &DebugConfig,
    pins: &BTreeMap<String, String>,
    pcf_name: &str,
) -> Result<BTreeMap<String, String>, ToolError> {
    let mut out = BTreeMap::new();
    for part in config.watch.iter().flat_map(|i| &i.parts) {
        let Some(pin) = part.strip_prefix(PIN_PREFIX) else {
            continue;
        };
        let port = pins
            .iter()
            .find(|(_, p)| p.as_str() == pin)
            .map(|(port, _)| port.clone())
            .ok_or_else(|| {
                ToolError::new(
                    Category::Design,
                    format!(
                        "{part}: {pcf_name} puts no port on pin {pin}. A pin path reads the port the constraint file places there."
                    ),
                )
            })?;
        let expr = match port.split_once('[') {
            Some((base, bit)) => format!("{}[{bit}", vid(base)),
            None => vid(&port),
        };
        out.insert(part.clone(), expr);
    }
    Ok(out)
}

/// The debug file's nets without the top module's name, which every path
/// starts with (main.u_ctrl.state), as the pre-pass names them
/// (u_ctrl.state). Host names default to the whole path.
fn in_top(mut config: DebugConfig, top: &str) -> Result<DebugConfig, String> {
    if top == "pin" {
        return Err(
            "The top module is called pin, which debug paths use for package pins (pin.18): rename the top module to debug it."
                .into(),
        );
    }
    for p in config.drive.iter().flat_map(|o| &o.parts) {
        if p.starts_with(PIN_PREFIX) {
            return Err(format!(
                "{p}: version 1 reads pins but can't drive them. Drive an internal net instead."
            ));
        }
    }
    let strip = |net: &mut String, name: &mut Option<String>| -> Result<(), String> {
        if net.starts_with(PIN_PREFIX) {
            name.get_or_insert_with(|| net.clone());
            return Ok(());
        }
        let rest = net
            .strip_prefix(top)
            .and_then(|r| r.strip_prefix('.'))
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
                format!(
                    "The net {net:?} must start with the top module's name, {top}: write {top}.{net}."
                )
            })?;
        let rest = rest.to_string();
        name.get_or_insert_with(|| net.clone());
        *net = rest;
        Ok(())
    };
    for i in &mut config.watch {
        if i.is_list() {
            for p in &mut i.parts {
                strip(p, &mut None)?;
            }
        } else {
            strip(&mut i.net, &mut i.name)?;
            i.parts = vec![i.net.clone()];
        }
    }
    for o in &mut config.drive {
        if o.is_list() {
            for p in &mut o.parts {
                strip(p, &mut None)?;
            }
        } else {
            strip(&mut o.net, &mut o.name)?;
            o.parts = vec![o.net.clone()];
        }
    }
    Ok(config)
}

// --- The uplink -------------------------------------------------------------

struct Link {
    tx: String,
    rx: String,
    /// The pins' I/O standard, such as LVCMOS33, when the catalog's UART
    /// gives one.
    io_standard: Option<String>,
    baud: u32,
    clock_mhz: f64,
    clks_per_bit: u32,
}

/// The UART's pins and speed, and the clock's frequency, checked.
fn uplink(
    config: &DebugConfig,
    board: &crate::board::ResolvedBoard,
    pins: &BTreeMap<String, String>,
    pcf_name: &str,
) -> Result<Link, ToolError> {
    let resources = serde_json::to_value(&board.resources).unwrap_or_default();
    let of_type = |t: &str| {
        resources
            .as_object()
            .and_then(|m| m.values().find(|r| r["type"] == t).cloned())
    };
    let up = &config.uplink;
    let mut io_standard = None;
    let (tx, rx) = match &up.resource {
        Some(name) => {
            let uarts: Vec<String> = resources
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter(|(_, r)| r["type"] == "uart")
                        .map(|(k, _)| k.clone())
                        .collect()
                })
                .unwrap_or_default();
            let known = if uarts.is_empty() {
                format!(
                    "{} has no UART in the catalog: give tx and rx pins instead",
                    board.name
                )
            } else {
                format!("{}'s UARTs: {}", board.name, uarts.join(", "))
            };
            let r = &resources[name.as_str()];
            if r.is_null() {
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "uplink.resource {name:?} isn't a resource of {}. {known}.",
                        board.name
                    ),
                ));
            }
            if r["type"] != "uart" {
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "uplink.resource {name:?} is of type {}, not uart. {known}.",
                        r["type"].as_str().unwrap_or("resource")
                    ),
                ));
            }
            let pin = |f: &str| r[f].as_str().map(str::to_string);
            io_standard = pin("io_standard");
            match (pin("tx"), pin("rx")) {
                (Some(tx), Some(rx)) => (tx, rx),
                _ => {
                    return Err(ToolError::request(format!(
                        "The catalog's {name} on {} lacks its tx or rx pin: give tx and rx instead.",
                        board.name
                    )));
                }
            }
        }
        None => (
            up.tx.clone().unwrap_or_default(),
            up.rx.clone().unwrap_or_default(),
        ),
    };
    let baud = up.baud.unwrap_or(115_200);
    if tx == rx {
        return Err(ToolError::request(
            "uplink.tx and uplink.rx are the same pin.",
        ));
    }
    // Decided (Q6): the design can't use the debugger's UART pins.
    for (port, pin) in pins {
        if *pin == tx || *pin == rx {
            return Err(ToolError::new(
                Category::Design,
                format!(
                    "{pcf_name} puts {port} on pin {pin}, which the debugger's UART needs. Version 1 can't share the UART with the design: remove {port} from the debug build's design, or give the debugger other pins in uplink."
                ),
            ));
        }
    }
    // The clock: the board's, when the clock port is on its pin.
    let Some(clock_pin) = pins.get(&config.clock) else {
        return Err(ToolError::new(
            Category::Design,
            format!(
                "The debug file's clock {:?} isn't a port in {pcf_name}: name the design's clock input.",
                config.clock
            ),
        ));
    };
    let board_clock = of_type("clock")
        .filter(|c| c["pin"].as_str() == Some(clock_pin.as_str()))
        .and_then(|c| c["mhz"].as_f64());
    let clock_mhz = match (config.clock_mhz, board_clock) {
        (Some(m), _) => m,
        (None, Some(m)) => m,
        (None, None) => {
            return Err(ToolError::request(format!(
                "The clock {} is on pin {clock_pin}, which isn't the board's clock in the catalog: give its frequency as clock_mhz in the debug file.",
                config.clock
            )));
        }
    };
    let ratio = clock_mhz * 1e6 / baud as f64;
    let clks_per_bit = ratio.round() as u32;
    let off = (ratio - clks_per_bit as f64).abs() / ratio;
    if clks_per_bit < 8 || off > 0.02 {
        return Err(ToolError::request(format!(
            "A {clock_mhz} MHz clock can't make {baud} baud closely enough: choose another baud in uplink."
        )));
    }
    Ok(Link {
        tx,
        rx,
        io_standard,
        baud,
        clock_mhz,
        clks_per_bit,
    })
}

/// Port to pin, from the toolchain's constraint file.
fn constraint_pins(toolchain: Toolchain, text: &str) -> BTreeMap<String, String> {
    match toolchain {
        Toolchain::Ecp5 => lpf_pins(text),
        Toolchain::Gowin => cst_pins(text),
        Toolchain::Openxc7 => xdc_pins(text),
        Toolchain::Ice40 => pcf_pins(text),
    }
}

/// The constraints that put the debugger's UART on its pins, in the
/// toolchain's format.
fn uart_constraints(toolchain: Toolchain, link: &Link) -> String {
    let mut out = String::new();
    for (port, pin) in [(TX_PORT, &link.tx), (RX_PORT, &link.rx)] {
        match toolchain {
            Toolchain::Ecp5 => {
                let _ = writeln!(out, "LOCATE COMP \"{port}\" SITE \"{pin}\";");
                if let Some(io) = &link.io_standard {
                    let _ = writeln!(out, "IOBUF PORT \"{port}\" IO_TYPE={io};");
                }
            }
            Toolchain::Gowin => {
                let _ = writeln!(out, "IO_LOC \"{port}\" {pin};");
                if let Some(io) = &link.io_standard {
                    let _ = writeln!(out, "IO_PORT \"{port}\" IO_TYPE={io};");
                }
            }
            Toolchain::Openxc7 => {
                let _ = writeln!(out, "set_property PACKAGE_PIN {pin} [get_ports {port}]");
                if let Some(io) = &link.io_standard {
                    let _ = writeln!(out, "set_property IOSTANDARD {io} [get_ports {port}]");
                }
            }
            Toolchain::Ice40 => {
                let _ = writeln!(out, "set_io {port} {pin}");
            }
        }
    }
    out
}

/// Port to pin, from a .lpf's LOCATE COMP statements, such as
/// LOCATE COMP "led[0]" SITE "A11"; (statements end at ;, comments start
/// with # or //).
pub fn lpf_pins(text: &str) -> BTreeMap<String, String> {
    let code: String = text
        .lines()
        .map(|l| {
            let l = l.split('#').next().unwrap_or("");
            l.split("//").next().unwrap_or("")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut pins = BTreeMap::new();
    for statement in code.split(';') {
        let words: Vec<String> = statement
            .split_whitespace()
            .map(|w| w.trim_matches('"').to_string())
            .collect();
        let is = |i: usize, w: &str| words.get(i).is_some_and(|x| x.eq_ignore_ascii_case(w));
        if is(0, "LOCATE")
            && is(1, "COMP")
            && is(3, "SITE")
            && let Some(pin) = words.get(4)
        {
            pins.insert(words[2].clone(), pin.clone());
        }
    }
    pins
}

/// Port to pin, from a .cst's IO_LOC statements, such as
/// IO_LOC "led[0]" 10; (statements end at ;, comments start with //).
pub fn cst_pins(text: &str) -> BTreeMap<String, String> {
    let code: String = text
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let mut pins = BTreeMap::new();
    for statement in code.split(';') {
        let words: Vec<String> = statement
            .split_whitespace()
            .map(|w| w.trim_matches('"').to_string())
            .collect();
        if words
            .first()
            .is_some_and(|w| w.eq_ignore_ascii_case("IO_LOC"))
            && let [_, port, pin, ..] = words.as_slice()
        {
            pins.insert(port.clone(), pin.clone());
        }
    }
    pins
}

/// Port to pin, from a .xdc's PACKAGE_PIN properties, in either form:
/// set_property PACKAGE_PIN E3 [get_ports sys_clk] or
/// set_property -dict { PACKAGE_PIN E3 IOSTANDARD LVCMOS33 } [get_ports { sys_clk }]
/// (comments start with #; braces and quotes around names are dropped).
pub fn xdc_pins(text: &str) -> BTreeMap<String, String> {
    // A name without the brackets, braces, quotes and ; around it, keeping
    // its own index brackets, as in led[0].
    fn name(word: &str) -> String {
        let mut w = word.trim_end_matches(';').to_string();
        while w.ends_with(']') && w.matches(']').count() > w.matches('[').count() {
            w.pop();
        }
        w.trim_matches(|c| c == '{' || c == '}' || c == '"')
            .to_string()
    }
    let mut pins = BTreeMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<String> = line
            .split_whitespace()
            .map(name)
            .filter(|w| !w.is_empty())
            .collect();
        if words.first().map(String::as_str) != Some("set_property") {
            continue;
        }
        let after = |key: &str| {
            let i = words
                .iter()
                .position(|w| w.trim_start_matches('[').eq_ignore_ascii_case(key))?;
            words.get(i + 1)
        };
        if let (Some(pin), Some(port)) = (after("PACKAGE_PIN"), after("get_ports")) {
            pins.insert(port.clone(), pin.clone());
        }
    }
    pins
}

/// Port to pin, from a .pcf's set_io lines.
pub fn pcf_pins(text: &str) -> BTreeMap<String, String> {
    let mut pins = BTreeMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"set_io") {
            continue;
        }
        let rest: Vec<&str> = words[1..]
            .iter()
            .copied()
            .filter(|w| !w.starts_with('-'))
            .collect();
        if let [port, pin, ..] = rest.as_slice() {
            pins.insert(port.to_string(), pin.to_string());
        }
    }
    pins
}

// --- The pre-pass -----------------------------------------------------------

/// Every net the file names, once, with whether the debugger drives it.
fn unique_nets(config: &DebugConfig) -> Vec<(String, bool)> {
    let mut nets: Vec<(String, bool)> = Vec::new();
    for p in config.watch.iter().flat_map(|i| &i.parts) {
        if !nets.iter().any(|(n, _)| n == p) {
            nets.push((p.clone(), false));
        }
    }
    for o in &config.drive {
        for p in &o.parts {
            match nets.iter_mut().find(|(n, _)| n == p) {
                Some((_, d)) => *d = true,
                None => nets.push((p.clone(), true)),
            }
        }
    }
    nets
}

/// A top-level port of the design after the pre-pass.
#[derive(Debug, Clone)]
struct Port {
    name: String,
    direction: String,
    width: usize,
    /// Its declared range, such as "[3:0] ", or "" for one bit.
    range: String,
}

struct Design {
    /// The top module's own ports, as the user's top declares them.
    ports: Vec<Port>,
    /// Widths of the nets the file names.
    widths: BTreeMap<String, usize>,
    /// The watched pin.<pin>s' port bits (pin_ports).
    pin_ports: BTreeMap<String, String>,
}

/// A yosys select pattern for a net. Brackets are wildcards in patterns, so
/// they become ?, and the count check catches any other match.
fn pattern(net: &str) -> String {
    format!("w:{}", net.replace(['[', ']'], "?"))
}

#[allow(clippy::too_many_arguments)]
async fn prepass(
    input: &DebugBuildInput,
    config_text: &str,
    toolchain: Toolchain,
    nets: &[(String, bool)],
    src: &Path,
    gen_dir: &Path,
    bin: &Path,
    call: &Call,
) -> Result<Design, ToolError> {
    let q = |p: &Path| format!("\"{}\"", p.to_string_lossy());
    // No optimization before the cut: proc's own opt_expr and opt_clean put
    // a constant net's value straight into its loads (`wire en = 1'b0;`),
    // leaving the net with nothing to drive. Synthesis optimizes later.
    let mut script = format!(
        "read_verilog -lib {cells}; hierarchy -top {top}; proc -noopt; flatten; \
         select {top}; write_json -selected {pre}; select -clear; ",
        cells = cells_sim(toolchain),
        top = input.top_module,
        pre = q(&gen_dir.join("pre.json"))
    );
    for (net, _) in nets {
        let _ = write!(script, "select -assert-count 1 {}; ", pattern(net));
    }
    let cuts: Vec<String> = nets
        .iter()
        .filter(|(_, d)| *d)
        .map(|(n, _)| pattern(n))
        .collect();
    // A driven net set by a plain connection, such as to a constant, gets a
    // buffer cell: expose -cut needs a driver cell to cut at, and would
    // otherwise move the net's loads onto the constant.
    if !cuts.is_empty() {
        let _ = write!(script, "insbuf {}; ", cuts.join(" "));
    }
    let reads: Vec<String> = nets.iter().map(|(n, _)| pattern(n)).collect();
    let _ = write!(script, "expose {}; ", reads.join(" "));
    if !cuts.is_empty() {
        let _ = write!(script, "expose -cut {}; ", cuts.join(" "));
    }
    let _ = write!(
        script,
        "opt_clean; write_verilog -noattr {v}; select {top}; write_json -selected {post}",
        v = q(&gen_dir.join("design_debug.v")),
        top = input.top_module,
        post = q(&gen_dir.join("post.json"))
    );
    let mut args: Vec<OsString> = vec![
        "-l".into(),
        gen_dir.join("prepass.log").into(),
        "-p".into(),
        script.into(),
        "-DSYNTHESIZE".into(),
        // Lets a design differ in debug builds.
        "-DLOGIX_DEBUG".into(),
    ];
    if !input.verbose {
        args.insert(0, "-q".into());
    }
    for (name, value) in &input.defines {
        args.push(format!("-D{name}={value}").into());
    }
    args.extend(input.source_files.iter().map(OsString::from));
    let program = crate::commands::program_in(bin, "yosys");
    let run = crate::runner::run_tool(&program, &args, src, bin, &call.cancel)
        .await
        .map_err(|e| match e {
            crate::runner::RunError::Cancelled => {
                ToolError::cancelled("The debug build was cancelled.")
            }
            crate::runner::RunError::Spawn(e) => ToolError::setup(format!("Can't run yosys: {e}")),
        })?;
    if !run.status.success() {
        return Err(prepass_error(&run.output, nets, input, src));
    }

    let pre = read_json(&gen_dir.join("pre.json"))?;
    let post = read_json(&gen_dir.join("post.json"))?;
    let top = &input.top_module;
    // Nets only the FPGA's pins may take, refused here rather than by the
    // place and route, in the debug file's terms.
    let pre_top = &pre["modules"][top];
    for (net, _) in nets {
        let bits = &pre_top["netnames"][net]["bits"];
        if let Some(why) = crate::debug::pads::unreachable(pre_top, bits, top) {
            let full = format!("{top}.{net}");
            let quoted = format!("\"{full}\"");
            let line = config_text
                .lines()
                .position(|l| l.contains(&quoted))
                .map_or(1, |i| i as u32 + 1);
            let message = format!("The debugger can't watch or drive {full}. {why}");
            let mut err = ToolError::new(
                Category::Design,
                format!("{}: {message} (at line {line}).", input.debug_file),
            );
            err.diagnostics = vec![diagnostic(&input.debug_file, line, &message)];
            return Err(err);
        }
    }
    let own: Vec<String> = pre["modules"][top]["ports"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let module = &post["modules"][top];
    let port = |name: &str| -> Option<Port> {
        let p = &module["ports"][name];
        let direction = p["direction"].as_str()?.to_string();
        let width = p["bits"].as_array()?.len();
        let n = &module["netnames"][name];
        let offset = n["offset"].as_u64().unwrap_or(0) as usize;
        let upto = n["upto"].as_u64() == Some(1);
        let range = if width == 1 && offset == 0 {
            String::new()
        } else if upto {
            format!("[{}:{}] ", offset, offset + width - 1)
        } else {
            format!("[{}:{}] ", offset + width - 1, offset)
        };
        Some(Port {
            name: name.to_string(),
            direction,
            width,
            range,
        })
    };
    let ports: Vec<Port> = own.iter().filter_map(|n| port(n)).collect();
    let mut widths = BTreeMap::new();
    for &(ref net, drives) in nets {
        let is_port = own.contains(net);
        if is_port && drives {
            return Err(ToolError::new(
                Category::Design,
                format!(
                    "The debug file drives {}.{net}, a port of the top module: version 1 can drive internal nets only.",
                    input.top_module
                ),
            ));
        }
        let found = port(net).filter(|p| is_port || p.direction == "output");
        // A cut net must keep its driver: had the cut put the net's loads
        // on its driver's side, the two ports would share their bits.
        let bits = |name: &str| module["ports"][name]["bits"].clone();
        let cut_ok = !drives
            || (port(&format!("{net}.i")).is_some() && bits(net) != bits(&format!("{net}.i")));
        // A driven net must still reach something, or the debugger would
        // drive nothing.
        if drives && cut_ok && !used(module, &bits(&format!("{net}.i")), &format!("{net}.i")) {
            return Err(ToolError::new(
                Category::Design,
                format!(
                    "Nothing in the design uses {}.{net}, so driving it would change nothing: debug a net the design reads.",
                    input.top_module
                ),
            ));
        }
        // A driven net needs a value of its own for when it is released.
        if drives && cut_ok && !driven(module, &bits(net)) {
            return Err(ToolError::new(
                Category::Design,
                format!(
                    "Nothing in the design drives {}.{net}, so released it would have no value. Give it one for the debugger to override, such as `wire {net} = 1'b0;`.",
                    input.top_module
                ),
            ));
        }
        match found {
            Some(p) if cut_ok => {
                widths.insert(net.clone(), p.width);
            }
            _ => {
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "The pre-pass couldn't expose {}.{net}. It may have no driver or be a constant: debug another net.",
                        input.top_module
                    ),
                ));
            }
        }
    }
    Ok(Design {
        ports,
        widths,
        pin_ports: BTreeMap::new(),
    })
}

/// True when a cell, or a port other than `own`, reads any of `bits`.
fn used(module: &Value, bits: &Value, own: &str) -> bool {
    let Some(bits) = bits.as_array() else {
        return false;
    };
    let wanted: Vec<&Value> = bits.iter().filter(|b| b.is_number()).collect();
    let hit = |v: &Value| {
        v.as_array()
            .is_some_and(|list| list.iter().any(|b| wanted.contains(&b)))
    };
    let in_cells = module["cells"].as_object().is_some_and(|cells| {
        cells.values().any(|c| {
            c["connections"]
                .as_object()
                .is_some_and(|conns| conns.values().any(hit))
        })
    });
    let in_ports = module["ports"]
        .as_object()
        .is_some_and(|ports| ports.iter().any(|(n, p)| n != own && hit(&p["bits"])));
    in_cells || in_ports
}

/// Whether every bit of a net has a driver: a constant, a cell's output or
/// one of the module's inputs.
fn driven(module: &Value, bits: &Value) -> bool {
    let Some(bits) = bits.as_array() else {
        return false;
    };
    let mut drivers: Vec<&Value> = Vec::new();
    if let Some(cells) = module["cells"].as_object() {
        for c in cells.values() {
            let Some(conns) = c["connections"].as_object() else {
                continue;
            };
            for (port, list) in conns {
                if c["port_directions"][port].as_str() != Some("input") {
                    drivers.extend(list.as_array().into_iter().flatten());
                }
            }
        }
    }
    if let Some(ports) = module["ports"].as_object() {
        for p in ports.values() {
            if p["direction"].as_str() == Some("input") {
                drivers.extend(p["bits"].as_array().into_iter().flatten());
            }
        }
    }
    bits.iter().all(|b| match b.as_str() {
        Some(c) => c == "0" || c == "1",
        None => drivers.contains(&b),
    })
}

/// The pre-pass's failure, in the user's terms.
fn prepass_error(
    output: &str,
    nets: &[(String, bool)],
    input: &DebugBuildInput,
    src: &Path,
) -> ToolError {
    let missing = output.lines().find_map(|l| {
        let rest = l.split("Assertion failed: selection contains ").nth(1)?;
        let count = rest.split_whitespace().next()?.to_string();
        let pat = rest.rsplit(": ").next()?.trim().to_string();
        let net = nets.iter().find(|(n, _)| pattern(n) == pat)?.0.clone();
        Some((net, count))
    });
    if let Some((net, count)) = missing {
        let what = if count == "0" {
            "isn't in the design".to_string()
        } else {
            format!("matches {count} nets")
        };
        return ToolError::new(
            Category::Design,
            format!(
                "{}: the net {}.{net} {what}. Name a whole net by its instance path, starting with the top module, such as {}.u_ctrl.state.",
                input.debug_file, input.top_module, input.top_module
            ),
        )
        .with_details(output);
    }
    let parsed = crate::build::parse::parse_yosys(output, &src.to_string_lossy());
    let first = parsed
        .errors
        .first()
        .map(|d| d.text.clone())
        .unwrap_or_else(|| "yosys failed".into());
    let mut err = ToolError::new(Category::Design, format!("The design has errors. {first}"))
        .with_details(output);
    err.diagnostics = parsed.errors;
    err
}

// --- Layout and the generated top ------------------------------------------

fn layout(
    input: &DebugBuildInput,
    config: &DebugConfig,
    design: &Design,
    link: &Link,
) -> Result<DebugMap, ToolError> {
    let mut inputs = Vec::new();
    let mut offset = 0;
    // A list combines one-bit nets: each member must be one.
    let members_one_bit = |list: &str, is_list: bool, parts: &[String]| -> Result<(), ToolError> {
        if !is_list {
            return Ok(());
        }
        for p in parts {
            let w = design.widths[p];
            if w != 1 {
                let path = if p.starts_with(PIN_PREFIX) {
                    p.clone()
                } else {
                    format!("{}.{p}", input.top_module)
                };
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "{}: the bus {list} lists {path}, which is {w} bits. A net list combines one-bit nets: list its bits' nets, or name the whole bus as one net.",
                        input.debug_file
                    ),
                ));
            }
        }
        Ok(())
    };
    for i in &config.watch {
        members_one_bit(&i.net, i.is_list(), &i.parts)?;
        let width = i.parts.iter().map(|p| design.widths[p]).sum();
        let members = match &i.spec {
            config::NetSpec::List(list) => list.clone(),
            _ => Vec::new(),
        };
        inputs.push(MapWatch {
            name: i.name.clone().unwrap_or_else(|| i.net.clone()),
            net: i.net.clone(),
            members,
            width,
            offset,
            format: i.format.map(|f| f.as_str().to_string()),
        });
        offset += width;
    }
    let read_bits = offset;
    let mut outputs = Vec::new();
    let mut offset = 0;
    let mut enables = 0;
    for o in &config.drive {
        members_one_bit(&o.net, o.is_list(), &o.parts)?;
        let width = o.parts.iter().map(|p| design.widths[p]).sum();
        let members = match &o.spec {
            config::NetSpec::List(list) => list.clone(),
            _ => Vec::new(),
        };
        let enable_bit = enables;
        enables += 1;
        outputs.push(MapDrive {
            name: o.name.clone().unwrap_or_else(|| o.net.clone()),
            net: o.net.clone(),
            width,
            offset,
            members,
            format: o.format.map(|f| f.as_str().to_string()),
            enable_bit,
        });
        offset += width;
    }
    // A READ reply carries the read nets, the write values and the enables.
    let (read_bits, write_bits, enables) = (read_bits.max(1), offset.max(1), enables.max(1));
    let reply_bits = read_bits + write_bits + enables;
    if reply_bits.div_ceil(8) > protocol::MAX_PAYLOAD {
        return Err(ToolError::new(
            Category::Design,
            format!(
                "The debug file watches {read_bits} bits and drives {write_bits}, {} bytes with the enables; version 1 takes at most {} ({} bits): watch or drive fewer nets.",
                reply_bits.div_ceil(8),
                protocol::MAX_PAYLOAD,
                protocol::MAX_PAYLOAD * 8
            ),
        ));
    }
    Ok(DebugMap {
        format: protocol::MAP_FORMAT.into(),
        protocol: protocol::PROTOCOL_VERSION,
        build_id: String::new(),
        board: input.board.clone(),
        top_module: input.top_module.clone(),
        clock: config.clock.clone(),
        clock_mhz: link.clock_mhz,
        baud: link.baud,
        uplink_resource: config.uplink.resource.clone(),
        tx_pin: link.tx.clone(),
        rx_pin: link.rx.clone(),
        read_bits,
        write_bits,
        enables,
        watch: inputs,
        drive: outputs,
    })
}

/// A Verilog identifier for a name, escaped when it isn't a plain one.
fn vid(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain {
        name.to_string()
    } else {
        format!("\\{name} ")
    }
}

/// `parts`, `out_parts`: each watched and driven entry's paths, most significant
/// first (as the pre-pass names them).
fn debug_top(
    user_top: &str,
    design: &Design,
    map: &DebugMap,
    parts: &[Vec<String>],
    out_parts: &[Vec<String>],
    link: &Link,
    id: u32,
) -> String {
    let read_w = map.watch.iter().map(|i| i.width).sum::<usize>();
    let write_w = map.drive.iter().map(|o| o.width).sum::<usize>();
    let enables = map.drive.len();
    let own = |n: &str| design.ports.iter().any(|p| p.name == n);

    // One wire per exposed design net, named by its order.
    let mut wires: Vec<(String, usize)> = Vec::new();
    let mut wire_of = BTreeMap::new();
    for net in parts.iter().flatten().chain(out_parts.iter().flatten()) {
        if !own(net) && !design.pin_ports.contains_key(net) && !wire_of.contains_key(net) {
            wire_of.insert(net.clone(), format!("logix_dbg_net{}", wires.len()));
            wires.push((net.clone(), design.widths[net]));
        }
    }
    let signal = |net: &str| {
        design
            .pin_ports
            .get(net)
            .or(wire_of.get(net))
            .cloned()
            .unwrap_or_else(|| vid(net))
    };

    let mut v = String::new();
    let _ = writeln!(
        v,
        "// Generated by Logix Engine's debug build: {user_top} with the Logix debugger.\n\
         // Build ID {id:08x}. Don't edit: each debug build writes it again.\n"
    );
    let names: Vec<String> = design
        .ports
        .iter()
        .map(|p| vid(&p.name))
        .chain([TX_PORT.to_string(), RX_PORT.to_string()])
        .collect();
    let _ = writeln!(v, "module {TOP} ({});", names.join(", "));
    for p in &design.ports {
        let _ = writeln!(v, "  {} {}{};", p.direction, p.range, vid(&p.name));
    }
    let _ = writeln!(v, "  output {TX_PORT};\n  input {RX_PORT};\n");
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_rd;", read_w.max(1) - 1);
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_wr;", write_w.max(1) - 1);
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_en;", enables.max(1) - 1);
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_pl;", enables.max(1) - 1);
    for (net, width) in &wires {
        let _ = writeln!(v, "  wire [{}:0] {}; // {net}", width - 1, wire_of[net]);
    }

    // The design, with its exposed nets.
    let mut conns: Vec<String> = design
        .ports
        .iter()
        .map(|p| format!(".{}({})", vid(&p.name), vid(&p.name)))
        .collect();
    for (net, _) in &wires {
        conns.push(format!(".{}({})", vid(net), wire_of[net]));
    }
    // What the design sees of each driven net: the debugger's value while
    // driven, the design's own while released, and flipped during a pulse.
    // Watching a driven net reads this, not the design's side of the cut.
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (o, ps) in map.drive.iter().zip(out_parts) {
        // The parts take the entry's bits, the first the most significant.
        let mut top_bit = o.offset + o.width;
        for p in ps {
            let w = design.widths[p];
            let value = format!("logix_dbg_wr[{}:{}]", top_bit - 1, top_bit - w);
            top_bit -= w;
            let b = o.enable_bit;
            let mut driven = format!("logix_dbg_en[{b}] ? {value} : {}", signal(p));
            // A one-bit entry can pulse: its value flips for one clock.
            if o.width == 1 {
                driven = format!("logix_dbg_pl[{b}] ^ ({driven})");
            }
            let name = format!("{}_seen", signal(p));
            let _ = writeln!(v, "  wire [{}:0] {name} = {driven};", w - 1);
            conns.push(format!(".{}({name})", vid(&format!("{p}.i"))));
            seen.insert(p.clone(), name);
        }
    }
    let _ = writeln!(
        v,
        "\n  {} u_design (\n      {}\n  );\n",
        vid(user_top),
        conns.join(",\n      ")
    );

    // The read space, first net in the low bits.
    // Each watched entry's parts are already most significant first.
    let bits: Vec<String> = parts
        .iter()
        .rev()
        .flatten()
        .map(|p| seen.get(p).cloned().unwrap_or_else(|| signal(p)))
        .collect();
    let rd = if bits.is_empty() {
        "1'b0".to_string()
    } else {
        format!("{{{}}}", bits.join(", "))
    };
    let _ = writeln!(v, "  assign logix_dbg_rd = {rd};\n");
    let _ = writeln!(
        v,
        "  logix_debugger #(\n      .CLKS_PER_BIT({}),\n      .READ_W({}),\n      .WRITE_W({}),\n      .OVR_W({}),\n      .BUILD_ID(32'h{id:08x})\n  ) u_logix_debugger (\n      .clk({}),\n      .rx({RX_PORT}),\n      .tx({TX_PORT}),\n      .rd_in(logix_dbg_rd),\n      .wr_out(logix_dbg_wr),\n      .ovr_en(logix_dbg_en),\n      .pulse(logix_dbg_pl)\n  );\n\nendmodule",
        link.clks_per_bit,
        read_w.max(1),
        write_w.max(1),
        enables.max(1),
        vid(&map.clock)
    );
    v
}

// --- The debugger's own share ----------------------------------------------

async fn debugger_use(
    family: crate::catalog::Family,
    map: &DebugMap,
    link: &Link,
    gen_dir: &Path,
    bin: &Path,
    call: &Call,
    cap: Option<Capacity>,
) -> DebuggerUse {
    let toolchain = family.toolchain();
    let synth = synth_command(family);
    let read_w = map.watch.iter().map(|i| i.width).sum::<usize>().max(1);
    let write_w = map.drive.iter().map(|o| o.width).sum::<usize>().max(1);
    let enables = map.drive.len().max(1);
    let script = format!(
        "read_verilog logix_debugger.v; chparam -set CLKS_PER_BIT {} -set READ_W {read_w} \
         -set WRITE_W {write_w} -set OVR_W {enables} logix_debugger; \
         {synth} -top logix_debugger; flatten; tee -o debugger_stat.txt stat",
        link.clks_per_bit,
    );
    let args: Vec<OsString> = vec!["-q".into(), "-p".into(), script.into()];
    let program = crate::commands::program_in(bin, "yosys");
    let ok = crate::runner::run_tool(&program, &args, gen_dir, bin, &call.cancel)
        .await
        .is_ok_and(|r| r.status.success());
    let stat = if ok {
        std::fs::read_to_string(gen_dir.join("debugger_stat.txt")).unwrap_or_default()
    } else {
        String::new()
    };
    let (mut luts, mut ffs) = (0u32, 0u32);
    for line in stat.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        if let [n, cell] = w.as_slice()
            && let Ok(n) = n.parse::<u32>()
        {
            let (l, f) = cell_use(toolchain, cell);
            luts += l * n;
            ffs += f * n;
        }
    }
    let luts_percent = cap.map(|c| percent(luts, c.luts));
    let flip_flops_percent = cap.map(|c| percent(ffs, c.flip_flops));
    let report = match (luts, cap) {
        (0, _) => "The debugger's own share couldn't be measured.".to_string(),
        (_, Some(c)) if toolchain == Toolchain::Ice40 => format!(
            "The debugger uses about {luts} LUTs ({}) and {ffs} flip-flops ({}) of the FPGA's {} logic cells.",
            percent(luts, c.luts),
            percent(ffs, c.flip_flops),
            c.luts
        ),
        (_, Some(c)) => format!(
            "The debugger uses about {luts} of the FPGA's {} LUTs ({}) and {ffs} of its {} flip-flops ({}).",
            c.luts,
            percent(luts, c.luts),
            c.flip_flops,
            percent(ffs, c.flip_flops)
        ),
        (_, None) => format!("The debugger uses about {luts} LUTs and {ffs} flip-flops of that."),
    };
    DebuggerUse {
        luts,
        flip_flops: ffs,
        luts_percent,
        flip_flops_percent,
        report,
    }
}

/// The LUTs and flip-flops one cell of the synthesized debugger takes.
fn cell_use(toolchain: Toolchain, cell: &str) -> (u32, u32) {
    match toolchain {
        Toolchain::Ice40 if cell == "SB_LUT4" => (1, 0),
        Toolchain::Ice40 if cell.starts_with("SB_DFF") => (0, 1),
        Toolchain::Ecp5 if cell == "LUT4" => (1, 0),
        // A carry cell is two LUTs of a slice.
        Toolchain::Ecp5 if cell == "CCU2C" => (2, 0),
        Toolchain::Ecp5 if cell == "TRELLIS_FF" => (0, 1),
        // LUT1 to LUT4 each take a LUT4; an ALU (adder bit) takes one too.
        Toolchain::Gowin if cell.starts_with("LUT") || cell == "ALU" => (1, 0),
        Toolchain::Gowin if cell.starts_with("DFF") => (0, 1),
        // LUT1 to LUT6, inverters and shift registers each take a slice
        // LUT, and a RAM64M or RAM32M (LUT RAM) four; FDRE, FDSE, FDCE and
        // FDPE a slice flip-flop. CARRY4 has its own logic.
        Toolchain::Openxc7 if cell == "RAM64M" || cell == "RAM32M" => (4, 0),
        Toolchain::Openxc7
            if cell.starts_with("LUT") || cell.starts_with("SRL") || cell == "INV" =>
        {
            (1, 0)
        }
        Toolchain::Openxc7 if cell.starts_with("FD") => (0, 1),
        _ => (0, 0),
    }
}

// --- Files ------------------------------------------------------------------

fn read(path: &Path, what: &str) -> Result<String, ToolError> {
    std::fs::read_to_string(path)
        .map_err(|e| ToolError::request(format!("Can't read {what}, {}: {e}", path.display())))
}

pub(crate) fn read_json(path: &Path) -> Result<Value, ToolError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ToolError::new(
            Category::Tool,
            format!("The pre-pass wrote no {}: {e}", path.display()),
        )
    })?;
    serde_json::from_str(&text).map_err(|e| {
        ToolError::new(
            Category::Tool,
            format!("Can't read {}: {e}", path.display()),
        )
    })
}

fn write(path: &Path, text: &str) -> Result<(), ToolError> {
    std::fs::write(path, text)
        .map_err(|e| ToolError::engine(format!("Can't write {}: {e}", path.display())))
}

/// A problem in the debug file, as a design error on that file.
fn file_error(file: &str, message: &str) -> ToolError {
    let line = message
        .split("at line ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.trim_end_matches(',').parse::<u32>().ok());
    let mut err = ToolError::new(Category::Design, format!("{file}: {message}"));
    err.diagnostics = vec![diagnostic(file, line.unwrap_or(1), message)];
    err
}

fn diagnostic(file: &str, line: u32, message: &str) -> Diagnostic {
    Diagnostic {
        file: Some(file.to_string()),
        line: Some(line),
        column: None,
        severity: crate::errors::Severity::Error,
        text: message.to_string(),
        tool: Some("build".into()),
        time_ns: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpf_pins_read_locate_comp() {
        let p = lpf_pins(
            "# x\nLOCATE COMP \"sys_clk\" SITE \"P6\";\nIOBUF PORT \"sys_clk\" IO_TYPE=LVCMOS33;\n\
             locate comp \"led[0]\"\n  site \"A11\"; // c\nLOCATE COMP x SITE B9; LOCATE COMP y SITE A9;\n",
        );
        assert_eq!(p["sys_clk"], "P6");
        assert_eq!(p["led[0]"], "A11");
        assert_eq!(p["x"], "B9");
        assert_eq!(p["y"], "A9");
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn cst_pins_read_io_loc() {
        let p = cst_pins(
            "// x\nIO_LOC \"sys_clk\" 52;\nIO_PORT \"sys_clk\" IO_TYPE=LVCMOS33;\n\
             io_loc \"led[0]\" 10; // c\nIO_LOC x 17; IO_LOC y 18;\n",
        );
        assert_eq!(p["sys_clk"], "52");
        assert_eq!(p["led[0]"], "10");
        assert_eq!(p["x"], "17");
        assert_eq!(p["y"], "18");
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn xdc_pins_read_package_pin() {
        let p = xdc_pins(
            "# x\nset_property PACKAGE_PIN E3 [get_ports sys_clk]\n\
             set_property IOSTANDARD LVCMOS33 [get_ports sys_clk]\n\
             set_property -dict { PACKAGE_PIN H5 IOSTANDARD LVCMOS33 } [get_ports { led[0] }]; # c\n\
             set_property -dict {PACKAGE_PIN J5} [get_ports {led[1]}]\n",
        );
        assert_eq!(p["sys_clk"], "E3");
        assert_eq!(p["led[0]"], "H5");
        assert_eq!(p["led[1]"], "J5");
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn pcf_pins_read_set_io() {
        let p = pcf_pins("# x\nset_io sys_clk 35\nset_io -nowarn led[0] 40 # c\n");
        assert_eq!(p["sys_clk"], "35");
        assert_eq!(p["led[0]"], "40");
    }

    #[test]
    fn paths_start_with_the_top() {
        let c = config::tests::parse_json(
            r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"main.u.count"},{"net":"main.blink","name":"b"}]}"#,
        )
        .unwrap();
        let c = in_top(c, "main").unwrap();
        assert_eq!(c.watch[0].net, "u.count");
        assert_eq!(c.watch[0].name.as_deref(), Some("main.u.count"));
        assert_eq!(c.watch[1].name.as_deref(), Some("b"));
        for bad in ["u.count", "mainx.count", "main"] {
            let c =
                config::tests::parse_json(&format!(r#"{{"clock":"c","uplink":{{"kind":"uart","resource":"uart"}},"watch":[{{"net":"{bad}"}}]}}"#)).unwrap();
            assert!(
                in_top(c, "main").unwrap_err().contains("must start"),
                "{bad}"
            );
        }
    }

    #[test]
    fn pins_read_the_port_placed_there() {
        let c = config::tests::parse_json(
            r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"pin.18"},{"net":"pin.19"}]}"#,
        )
        .unwrap();
        let c = in_top(c, "main").unwrap();
        assert_eq!(c.watch[0].net, "pin.18");
        let pins = pcf_pins("set_io sw[0] 18\nset_io key 19\n");
        let p = pin_ports(&c, &pins, "p.pcf").unwrap();
        assert_eq!(p["pin.18"], "sw[0]");
        assert_eq!(p["pin.19"], "key");
        let none = pcf_pins("set_io key 20\n");
        assert!(pin_ports(&c, &none, "p.pcf").is_err());
        assert!(in_top(c.clone(), "pin").is_err());
    }

    #[test]
    fn debug_file_errors_carry_their_line() {
        let e = config::parse("clock = \"c\"\nuplink = { kind = \"uart\"\n").unwrap_err();
        let err = file_error("debug.toml", &e);
        assert_eq!(err.diagnostics[0].line, Some(2), "{e}");
    }

    #[test]
    fn percents_round_up() {
        assert_eq!(percent(550, 7680), "8%");
        assert_eq!(percent(1, 7680), "1%");
        assert_eq!(percent(0, 7680), "0%");
        assert_eq!(percent(7680, 7680), "100%");
    }

    #[test]
    fn identifiers_escape_paths() {
        assert_eq!(vid("count"), "count");
        assert_eq!(vid("u_counter.count"), "\\u_counter.count ");
        assert_eq!(pattern("gen[2].u.err"), "w:gen?2?.u.err");
    }
}
