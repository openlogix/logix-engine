//! The diagram tool's work (docs/tool-api.md, "diagram tool"): yosys reads
//! the design without synthesizing it, every module is drawn as a
//! schematic, laid out by ELK (logix-core's elkjs, run in the engine), and
//! the engine writes each module's SVG, a model of the design and a page
//! that shows them all.

pub mod layout;
pub mod netlist;
pub mod rules;
pub mod schematic;
pub mod symbols;

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::board::ResolvedBoard;
use crate::build::parse::parse_yosys;
use crate::build::{run_logged, tool_failed};
use crate::catalog::Toolchain;
use crate::errors::{Category, Diagnostic, Severity, ToolError};
use crate::progress::Phases;
use crate::sim::{SimLibrary, toolchain};
use crate::state::EngineState;
use crate::tools::Call;

use layout::Stop;
use rules::{ModuleModel, Source};

/// The folder in the build folder the diagram is written to, replaced by
/// each call.
pub const FOLDER: &str = "diagram";
/// The page that shows every module.
pub const PAGE: &str = "diagram.html";
/// The model of the design: every module's blocks, labels, pins and
/// signals.
pub const MODEL: &str = "diagram.json";
/// Yosys's netlist, which the diagram is drawn from.
pub const NETLIST: &str = "netlist.json";

/// The core component with ELK.
pub const ELKJS: &str = "elkjs";
/// The ELK bundle in it.
const ELK_BUNDLE: &str = "lib/elk.bundled.js";

/// Modules with more blocks than this aren't drawn: their diagram would be
/// unreadable, and laying it out would take minutes.
pub const MAX_BLOCKS: usize = 1500;
/// How many levels of modules below the top module are drawn by default.
pub const DEFAULT_LEVELS: usize = 3;
/// Above this, ELK spends less effort on fewer wire crossings, so large
/// modules are drawn in seconds.
const THOROUGH_BLOCKS: usize = 150;
/// The most time one module's layout may take.
const LAYOUT_LIMIT: Duration = Duration::from_secs(60);

/// The default look of the SVGs, scoped to them (`svg.logix-diagram`).
/// Hosts change it with the variables, such as
/// `--logix-diagram-register`, without touching the files.
pub const STYLE: &str = include_str!("diagram.css");
/// The page's template.
const VIEWER: &str = include_str!("viewer.html");

/// A checked diagram request.
#[derive(Debug, Clone)]
pub struct DiagramRequest {
    /// The real path of the source folder.
    pub source_folder: PathBuf,
    /// The design files, relative to the source folder.
    pub source_files: Vec<String>,
    /// The real path of the build folder.
    pub build_folder: PathBuf,
    pub top_module: String,
    pub defines: BTreeMap<String, String>,
    pub board: ResolvedBoard,
    pub verbose: bool,
    /// Open the page in the system's web browser when it's written.
    pub open: bool,
    /// How many levels of modules below the top module are drawn.
    pub levels: usize,
}

/// One module of the diagram.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleSummary {
    /// The module's key in the model and the page, also its SVG's name.
    pub key: String,
    /// The module's name.
    pub name: String,
    pub top: bool,
    /// Its drawing, relative to the build folder, when it was drawn.
    pub svg_file: Option<String>,
    /// How many blocks it has: cells, pins and labels.
    pub blocks: usize,
    /// Why it wasn't drawn, when it wasn't.
    pub note: Option<String>,
}

/// What diagram made.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagramResult {
    /// The page, relative to the build folder.
    pub page_file: String,
    /// The model, relative to the build folder.
    pub model_file: String,
    /// Every module, the top module first.
    pub modules: Vec<ModuleSummary>,
    /// Yosys warnings while reading the design.
    pub diagnostics: Vec<Diagnostic>,
    /// Whether the page was opened in the system's web browser.
    pub opened: bool,
}

/// Opens the page in the system's default web browser, detached, and says
/// whether it started.
fn open_page(page: &Path) -> bool {
    let (program, args): (&str, Vec<OsString>) = if cfg!(target_os = "macos") {
        ("/usr/bin/open", vec![page.into()])
    } else if cfg!(windows) {
        (
            "cmd",
            vec!["/c".into(), "start".into(), "".into(), page.into()],
        )
    } else {
        ("xdg-open", vec![page.into()])
    };
    let dir = page.parent().unwrap_or(page);
    match crate::sim::gtkwave::launch_detached(
        Path::new(program),
        &args,
        dir,
        Path::new("/usr/bin"),
        || {},
    ) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Can't open the diagram's page: {}", e.message);
            false
        }
    }
}

/// Quotes a path for a yosys script.
fn yosys_quote(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Yosys's arguments: the FPGA's primitives read as black boxes (so their
/// ports have directions), then the design, as written, without synthesis;
/// only the design's own modules are written. Run in the source folder, with
/// the files as the request names them, so the names yosys gives cells
/// (`$main.v:13$add$5`) and source places hold no folder.
pub fn yosys_args(req: &DiagramRequest, libraries: &[PathBuf]) -> Vec<OsString> {
    let libs: String = libraries
        .iter()
        .map(|l| format!("read_verilog -lib {}; ", yosys_quote(l)))
        .collect();
    let files: Vec<String> = req
        .source_files
        .iter()
        .map(|f| yosys_quote(Path::new(f)))
        .collect();
    let netlist = req.build_folder.join(FOLDER).join(NETLIST);
    let mut args: Vec<OsString> = vec![
        "-p".into(),
        format!(
            "{libs}read_verilog -sv {}; hierarchy -top {}; proc; opt -full; clean -purge; \
             select =A:blackbox =A:whitebox %u %n; write_json -compat-int -selected {}",
            files.join(" "),
            req.top_module,
            yosys_quote(&netlist)
        )
        .into(),
        "-DSYNTHESIZE".into(),
    ];
    for (name, value) in &req.defines {
        args.push(format!("-D{name}={value}").into());
    }
    if !req.verbose {
        args.push("-q".into());
    }
    args
}

/// Yosys's message when the top module isn't in the design.
fn missing_top(output: &str) -> bool {
    output.contains("not found!") && output.contains("Module")
        || output.contains("did not match any module")
}

/// The error for a yosys run that failed: a design error with the problems
/// it reported, or a tool error when it reported none.
fn yosys_error(run: &crate::runner::ToolRun, req: &DiagramRequest) -> ToolError {
    let src = req.source_folder.to_string_lossy();
    let mut errors = parse_yosys(&run.output, &src).errors;
    if missing_top(&run.output) {
        errors = vec![Diagnostic {
            file: None,
            line: None,
            column: None,
            severity: Severity::Error,
            text: format!(
                "The top module {} isn't in the design files.",
                req.top_module
            ),
            tool: Some("yosys".into()),
            time_ns: None,
        }];
    }
    let failed = tool_failed("yosys", "reading the design", run);
    let Some(first) = errors.first() else {
        return failed;
    };
    let place = match (&first.file, first.line) {
        (Some(f), Some(l)) => format!("{f}:{l}: "),
        _ => String::new(),
    };
    let mut err = ToolError::new(
        Category::Design,
        format!("The design has errors. {place}{}", first.text),
    );
    err.details = failed.details;
    err.diagnostics = errors;
    err
}

/// ELK's options: balanced layers and placement, straight wires where they
/// can be, and even spacing (the prototype's, 2026-11-11). Large modules
/// get a faster placement (Brandes and Köpf's, about ten times faster than
/// network simplex on a 500-block module) and less effort on crossings.
fn layout_options(blocks: usize) -> Value {
    let large = blocks > THOROUGH_BLOCKS;
    json!({
        "org.eclipse.elk.algorithm": "layered",
        "org.eclipse.elk.layered.layering.strategy": "NETWORK_SIMPLEX",
        "org.eclipse.elk.layered.nodePlacement.strategy": if large { "BRANDES_KOEPF" } else { "NETWORK_SIMPLEX" },
        "org.eclipse.elk.layered.nodePlacement.favorStraightEdges": "true",
        "org.eclipse.elk.layered.thoroughness": if large { "7" } else { "40" },
        "org.eclipse.elk.layered.spacing.nodeNodeBetweenLayers": "45",
        "org.eclipse.elk.spacing.nodeNode": "30",
        "org.eclipse.elk.layered.spacing.edgeNodeBetweenLayers": "20",
        "org.eclipse.elk.layered.spacing.edgeEdgeBetweenLayers": "12",
    })
}

/// Plain names of the FPGA's own blocks, by family. Others keep their type.
pub fn fpga_name(toolchain: Toolchain) -> fn(&str) -> Option<&'static str> {
    match toolchain {
        Toolchain::Ice40 => |t| {
            Some(match t {
                "SB_LUT4" => "Logic cell (4-input LUT)",
                "SB_CARRY" => "Carry logic",
                "SB_RAM40_4K" | "SB_RAM40_4KNR" | "SB_RAM40_4KNW" | "SB_RAM40_4KNRNW" => {
                    "RAM block (4 kbit)"
                }
                "SB_SPRAM256KA" => "Single-port RAM (256 kbit)",
                "SB_RGBA_DRV" | "SB_RGB_DRV" => "RGB LED driver",
                "SB_LED_DRV_CUR" => "LED current driver",
                "SB_IO" | "SB_IO_OD" | "SB_IO_I3C" => "I/O pin",
                "SB_GB" | "SB_GB_IO" => "Global clock buffer",
                "SB_PLL40_CORE" | "SB_PLL40_PAD" | "SB_PLL40_2_PAD" | "SB_PLL40_2F_CORE"
                | "SB_PLL40_2F_PAD" => "PLL",
                "SB_HFOSC" => "High-frequency oscillator",
                "SB_LFOSC" => "Low-frequency oscillator",
                "SB_MAC16" => "DSP block (16-bit multiply-accumulate)",
                "SB_WARMBOOT" => "Warm boot",
                "SB_I2C" => "I2C block",
                "SB_SPI" => "SPI block",
                _ if t.starts_with("SB_DFF") => "Flip-flop",
                _ => return None,
            })
        },
        Toolchain::Ecp5 => |t| {
            Some(match t {
                "LUT4" => "Logic cell (4-input LUT)",
                "TRELLIS_FF" => "Flip-flop",
                "TRELLIS_IO" | "BB" | "IB" | "OB" | "OBZ" => "I/O pin",
                "DP16KD" => "RAM block (18 kbit)",
                "TRELLIS_DPR16X4" => "Distributed RAM",
                "MULT18X18D" => "Multiplier (18x18)",
                "ALU54B" => "DSP adder (54-bit)",
                "EHXPLLL" => "PLL",
                "OSCG" => "Oscillator",
                "DCCA" => "Clock buffer",
                "DCSC" => "Clock selector",
                "USRMCLK" => "Configuration flash clock",
                "GSR" | "SGSR" => "Global set and reset",
                "JTAGG" => "JTAG block",
                "IDDRX1F" | "ODDRX1F" | "IDDRX2F" | "ODDRX2F" => "Double data rate I/O",
                "DTR" => "Temperature sensor",
                "EXTREFB" | "DCUA" => "SERDES block",
                _ => return None,
            })
        },
        Toolchain::Gowin => |t| {
            Some(match t {
                "LUT1" | "LUT2" | "LUT3" | "LUT4" => "Logic cell (LUT)",
                "IBUF" | "OBUF" | "IOBUF" | "TBUF" | "ELVDS_IBUF" | "ELVDS_OBUF" | "TLVDS_IBUF"
                | "TLVDS_OBUF" => "I/O pin",
                "rPLL" | "PLLVR" | "PLL" | "PLLA" => "PLL",
                "OSC" | "OSCH" | "OSCZ" | "OSCF" | "OSCO" | "OSCW" => "Oscillator",
                "BUFG" | "DQCE" | "DCS" => "Clock buffer",
                "SP" | "SPX9" | "DP" | "DPX9" | "SDP" | "SDPX9" | "SDPB" | "SDPX9B" | "DPB"
                | "DPX9B" | "pROM" | "pROMX9" => "RAM block",
                "MULT9X9" | "MULT18X18" | "MULT36X36" | "MULTALU18X18" | "MULTADDALU18X18" => {
                    "Multiplier"
                }
                "CLKDIV" => "Clock divider",
                "IDDR" | "ODDR" | "IDDRC" | "ODDRC" => "Double data rate I/O",
                "GSR" => "Global set and reset",
                _ if t.starts_with("DFF") => "Flip-flop",
                _ => return None,
            })
        },
        Toolchain::Openxc7 => |t| {
            Some(match t {
                "IBUF" | "OBUF" | "IOBUF" | "OBUFT" | "IBUFDS" | "OBUFDS" | "IBUFG" => "I/O pin",
                "BUFG" | "BUFGCE" | "BUFGCTRL" | "BUFH" | "BUFR" | "BUFIO" => "Clock buffer",
                "PLLE2_BASE" | "PLLE2_ADV" => "PLL",
                "MMCME2_BASE" | "MMCME2_ADV" => "Clock manager (MMCM)",
                "RAMB18E1" | "RAMB36E1" => "RAM block",
                "DSP48E1" => "DSP block",
                "IDDR" | "ODDR" => "Double data rate I/O",
                "STARTUPE2" => "Startup block",
                "XADC" => "Analog-to-digital converter",
                "CARRY4" => "Carry logic",
                _ if t.starts_with("LUT") => "Logic cell (LUT)",
                _ if t.starts_with("FD") => "Flip-flop",
                _ => return None,
            })
        },
    }
}

/// File-safe keys for the modules, unique, from their plain names.
fn module_keys(names: &[String]) -> HashMap<String, String> {
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut keys = HashMap::new();
    for n in names {
        let base: String = netlist::plain_name(n)
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = if base.is_empty() {
            "module".into()
        } else {
            base
        };
        let count = used.entry(base.to_lowercase()).or_insert(0);
        *count += 1;
        let key = if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        };
        keys.insert(n.clone(), key);
    }
    keys
}

/// A place from a yosys `src` attribute ("/abs/main.v:13.5-13.20", several
/// joined by "|"), named as diagnostics name files: relative to the source
/// folder, or as display_paths says.
fn source_place(
    src: &str,
    source_folder: &Path,
    display: &crate::display_paths::Display,
) -> Option<Source> {
    static PLACE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = PLACE.get_or_init(|| Regex::new(r"^(.+?):(\d+)(\.|$)").unwrap());
    let first = src.split('|').next()?;
    let c = re.captures(first)?;
    let path = Path::new(&c[1]);
    let file = match path.strip_prefix(source_folder) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => path.to_string_lossy().into_owned(),
    };
    Some(Source {
        file: display.file(&file),
        line: c[2].parse().ok()?,
    })
}

/// Draws the diagram.
pub async fn diagram(
    state: &EngineState,
    req: DiagramRequest,
    call: &Call,
) -> Result<DiagramResult, ToolError> {
    let needed = state
        .packages
        .needing_download(&[crate::packages::CORE])
        .await?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 2);
    let (bin, package_dir) = toolchain(state, &needed, &phases, call).await?;
    let elk = state.packages.component_dir(ELKJS)?.join(ELK_BUNDLE);
    diagram_with(&bin, &package_dir, &elk, &req, call, &phases).await
}

/// Draws the diagram with the toolchain in `bin` (its package in
/// `package_dir`) and ELK's bundle at `elk`; the rest of diagram, for tests.
pub async fn diagram_with(
    bin: &Path,
    package_dir: &Path,
    elk: &Path,
    req: &DiagramRequest,
    call: &Call,
    phases: &Phases,
) -> Result<DiagramResult, ToolError> {
    let out = req.build_folder.join(FOLDER);
    let io = |what: &str, e: std::io::Error| {
        ToolError::engine(format!("Can't {what} in the build folder: {e}."))
    };
    // Remove the last diagram first, so a failure never leaves an old one
    // looking current.
    match std::fs::remove_dir_all(&out) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(io("delete the old diagram", e));
        }
        _ => {}
    }
    std::fs::create_dir_all(&out).map_err(|e| io("create the diagram folder", e))?;
    if !elk.is_file() {
        return Err(ToolError::setup(format!(
            "The {} package has no {ELKJS} ({ELK_BUNDLE}), which draws diagrams.",
            crate::packages::CORE
        ))
        .with_action(
            crate::errors::ActionCode::DownloadPackages,
            format!(
                "Download the {} package again, which may be damaged.",
                crate::packages::CORE
            ),
        ));
    }

    phases.start("Reading the design");
    let libraries = SimLibrary::for_board(&req.board, package_dir).lint_files;
    let run = run_logged(
        "diagram",
        "yosys",
        bin,
        &yosys_args(req, &libraries),
        &req.source_folder,
        call,
    )
    .await?;
    if !run.status.success() {
        return Err(yosys_error(&run, req));
    }
    let src = req.source_folder.to_string_lossy();
    let diagnostics = parse_yosys(&run.output, &src).warnings;
    let text = std::fs::read_to_string(out.join(NETLIST))
        .map_err(|e| tool_failed("yosys", &format!("writing the netlist ({e})"), &run))?;
    let net: netlist::Netlist = serde_json::from_str(&text)
        .map_err(|e| ToolError::engine(format!("Can't read yosys's netlist: {e}.")))?;

    let mut layouts = layout::Layouts::new(elk, call.cancel.clone());
    let (models, top) = match draw_all(
        net,
        req.levels,
        req.board.fpga.toolchain(),
        &req.source_folder,
        &call.display,
        &out,
        &mut layouts,
        phases,
    )
    .await
    {
        Ok(d) => d,
        Err(Stop::Cancelled) => return Err(ToolError::cancelled("The diagram was cancelled.")),
        // A module's failed layout is noted on it; these are the rest.
        Err(Stop::TooLong) => return Err(ToolError::engine("Drawing the diagram took too long.")),
        Err(Stop::Failed(e)) => {
            return Err(ToolError::engine(format!("Can't draw the diagram: {e}.")));
        }
    };

    let modules: Vec<ModuleSummary> = models
        .iter()
        .map(|(key, m, blocks)| ModuleSummary {
            key: key.clone(),
            name: m.name.clone(),
            top: m.top,
            svg_file: m.svg_file.as_ref().map(|f| format!("{FOLDER}/{f}")),
            blocks: *blocks,
            note: m.note.clone(),
        })
        .collect();
    let model = json!({
        "format": 1,
        "top": top,
        "modules": models.iter().map(|(k, m, _)| (k.clone(), json!(m))).collect::<serde_json::Map<_, _>>(),
    });
    std::fs::write(
        out.join(MODEL),
        serde_json::to_string_pretty(&model).unwrap() + "\n",
    )
    .map_err(|e| io("write the diagram's model", e))?;
    // The page: the model with each module's SVG, in one file.
    let mut page_data = model.clone();
    for (key, m, _) in &models {
        if let Some(f) = &m.svg_file {
            let svg = std::fs::read_to_string(out.join(f))
                .map_err(|e| io("read a module's drawing", e))?;
            page_data["modules"][key]["svg"] = Value::String(svg);
        }
    }
    let data = serde_json::to_string(&page_data)
        .unwrap()
        .replace("</", "<\\/")
        .replace("<!--", "<\\!--");
    std::fs::write(out.join(PAGE), VIEWER.replace("/*__DATA__*/null", &data))
        .map_err(|e| io("write the diagram's page", e))?;
    let opened = req.open && open_page(&out.join(PAGE));
    Ok(DiagramResult {
        opened,
        page_file: format!("{FOLDER}/{PAGE}"),
        model_file: format!("{FOLDER}/{MODEL}"),
        modules,
        diagnostics,
    })
}

/// Every module's model, with its block count, the top module first, and
/// the top module's key; each drawn module's SVG is written to `folder`.
type Drawn = (Vec<(String, ModuleModel, usize)>, String);

/// How deep each module is below the top module (0 for the top), along its
/// instances; modules the top doesn't use aren't listed.
fn depths(net: &netlist::Netlist, top: &str) -> HashMap<String, usize> {
    let mut depth = HashMap::from([(top.to_string(), 0)]);
    let mut queue = std::collections::VecDeque::from([top.to_string()]);
    while let Some(m) = queue.pop_front() {
        let d = depth[&m];
        for c in net
            .modules
            .get(&m)
            .into_iter()
            .flat_map(|m| m.cells.values())
        {
            if net.modules.contains_key(&c.ty) && !depth.contains_key(&c.ty) {
                depth.insert(c.ty.clone(), d + 1);
                queue.push_back(c.ty.clone());
            }
        }
    }
    depth
}

/// A diagram with more blocks than this is large: its status says so.
pub const LARGE_BLOCKS: usize = 300;

#[allow(clippy::too_many_arguments)]
async fn draw_all(
    net: netlist::Netlist,
    levels: usize,
    toolchain: Toolchain,
    source_folder: &Path,
    display: &crate::display_paths::Display,
    folder: &Path,
    layouts: &mut layout::Layouts,
    phases: &Phases,
) -> Result<Drawn, Stop> {
    let mut names: Vec<String> = net.modules.keys().cloned().collect();
    // The top module first, then the others as yosys lists them.
    names.sort_by_key(|n| !net.modules[n].is_top());
    let keys = module_keys(&names);
    let top_name = names.first().cloned().unwrap_or_default();
    let depth = depths(&net, &top_name);
    let source = |s: &str| source_place(s, source_folder, display);
    let cx = rules::Context {
        module_keys: &keys,
        fpga_name: fpga_name(toolchain),
        source: &source,
    };
    // Every module's schematic first, so the phase can say when the
    // diagram is large.
    let mut built = vec![];
    for name in &names {
        let (mut schematic, model, classes) = rules::build(name, &net.modules[name], &cx);
        schematic.add_constants();
        schematic.add_splits_joins();
        let deep = depth.get(name).is_none_or(|d| *d > levels);
        built.push((name, schematic, model, classes, deep));
    }
    let drawn_blocks: usize = built
        .iter()
        .filter(|(_, s, _, _, deep)| !deep && s.nodes.len() <= MAX_BLOCKS)
        .map(|(_, s, _, _, _)| s.nodes.len())
        .sum();
    phases.start(if drawn_blocks > LARGE_BLOCKS {
        "Preparing the diagram (large)"
    } else {
        "Preparing the diagram"
    });
    let top_plain = netlist::plain_name(&top_name);
    let mut out = vec![];
    for (name, schematic, mut model, classes, deep) in built {
        let key = keys[name].clone();
        let blocks = schematic.nodes.len();
        if deep {
            model.note = Some(format!(
                "Not drawn: it's more than {levels} levels below {top_plain}. A diagram that starts at it, or one with more levels, shows it."
            ));
        } else if blocks > MAX_BLOCKS {
            model.note = Some(format!(
                "Not drawn: it has {blocks} blocks, more than the {MAX_BLOCKS} a diagram can show readably. Split it into smaller modules to see them."
            ));
        } else {
            let graph = schematic.layout_graph(&layout_options(blocks));
            match layouts.layout(&graph.graph.to_string(), LAYOUT_LIMIT).await {
                Ok(laid) => {
                    let laid: Value = serde_json::from_str(&laid)
                        .map_err(|e| Stop::Failed(format!("ELK's result: {e}")))?;
                    let svg = schematic.render(&laid, &graph, &|b| classes.class(b), &key);
                    let file = format!("{key}.svg");
                    std::fs::write(folder.join(&file), svg)
                        .map_err(|e| Stop::Failed(format!("can't write {file}: {e}")))?;
                    model.svg_file = Some(file);
                }
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
                Err(Stop::TooLong) => {
                    model.note = Some(format!(
                        "Not drawn: laying out its {blocks} blocks took more than {} seconds. Split it into smaller modules to see them.",
                        LAYOUT_LIMIT.as_secs()
                    ));
                }
                Err(Stop::Failed(e)) => {
                    log::warn!("Diagram: the layout of {key} failed: {e}");
                    model.note = Some(format!("Not drawn: its layout failed ({e})."));
                }
            }
        }
        out.push((key, model, blocks));
    }
    let top = out
        .iter()
        .find(|(_, m, _)| m.top)
        .or(out.first())
        .map(|(k, _, _)| k.clone())
        .unwrap_or_default();
    Ok((out, top))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> DiagramRequest {
        DiagramRequest {
            source_folder: "/p/my proj".into(),
            source_files: vec!["main.v".into(), "rtl/uart.sv".into()],
            build_folder: "/p/my proj/_build".into(),
            top_module: "main".into(),
            defines: BTreeMap::from([("WIDTH".into(), "8".into()), ("FAST".into(), "".into())]),
            board: crate::board::resolve("icesugar-1-5", &None, &Default::default()).unwrap(),
            verbose: false,
            open: false,
            levels: DEFAULT_LEVELS,
        }
    }

    #[test]
    fn standard_command() {
        let args = yosys_args(&req(), &["/pkg/share/yosys/ice40/cells_sim.v".into()]);
        // The fake /p folder joins with \ on Windows.
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            args,
            [
                "-p",
                "read_verilog -lib \"/pkg/share/yosys/ice40/cells_sim.v\"; \
                 read_verilog -sv \"main.v\" \"rtl/uart.sv\"; \
                 hierarchy -top main; proc; opt -full; clean -purge; \
                 select =A:blackbox =A:whitebox %u %n; \
                 write_json -compat-int -selected \"/p/my proj/_build/diagram/netlist.json\"",
                "-DSYNTHESIZE",
                "-DFAST=",
                "-DWIDTH=8",
                "-q",
            ]
        );
        let verbose = DiagramRequest {
            verbose: true,
            ..req()
        };
        assert!(!yosys_args(&verbose, &[]).contains(&"-q".into()));
    }

    /// A run that exited with code 1.
    fn run(output: &str) -> crate::runner::ToolRun {
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(1 << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(1)
        };
        crate::runner::ToolRun {
            status,
            output: output.into(),
        }
    }

    #[test]
    fn yosys_failures() {
        let e = yosys_error(
            &run("/p/my proj/rtl/uart.sv:2: ERROR: syntax error, unexpected ';'\n"),
            &req(),
        );
        assert_eq!(e.category, Category::Design);
        assert_eq!(
            e.message,
            "The design has errors. rtl/uart.sv:2: syntax error, unexpected ';'"
        );
        // Real yosys output for a top module that isn't in the files.
        let e = yosys_error(&run("ERROR: Module `nope' not found!\n"), &req());
        assert_eq!(e.category, Category::Design);
        assert_eq!(
            e.message,
            "The design has errors. The top module main isn't in the design files."
        );
        let e = yosys_error(&run("Segmentation fault\n"), &req());
        assert_eq!(e.category, Category::Tool);
    }

    #[test]
    fn module_keys_are_unique_and_file_safe() {
        let names: Vec<String> = ["main", "$paramod$1\\genrom", "$paramod$2\\genrom", "a.b"]
            .map(String::from)
            .to_vec();
        let k = module_keys(&names);
        assert_eq!(k["main"], "main");
        assert_eq!(k["$paramod$1\\genrom"], "genrom");
        assert_eq!(k["$paramod$2\\genrom"], "genrom-2");
        assert_eq!(k["a.b"], "a_b");
    }

    #[test]
    fn source_places() {
        let d = crate::display_paths::Display::default();
        let p = source_place(
            "/p/my proj/rtl/uart.sv:12.3-12.40|/p/x.v:1",
            Path::new("/p/my proj"),
            &d,
        );
        assert_eq!(
            p,
            Some(Source {
                file: "rtl/uart.sv".into(),
                line: 12
            })
        );
        assert_eq!(source_place("nothing", Path::new("/p"), &d), None);
        assert_eq!(fpga_name(Toolchain::Ice40)("SB_DFFE"), Some("Flip-flop"));
        assert_eq!(fpga_name(Toolchain::Ecp5)("EHXPLLL"), Some("PLL"));
        assert_eq!(fpga_name(Toolchain::Gowin)("rPLL"), Some("PLL"));
        assert_eq!(fpga_name(Toolchain::Openxc7)("PLLE2_BASE"), Some("PLL"));
        assert_eq!(fpga_name(Toolchain::Ice40)("SB_NEW"), None);
    }
}
