//! What the diagram shows of a module, and how: the rules the prototype
//! settled with the user (2026-11-11). Clocks and resets that go to two or
//! more places, and fixed values, are drawn as labels instead of wires;
//! memory initialization isn't drawn; FPGA blocks get plain names; and
//! registers are named after the signal they hold. Each block, label, pin
//! and signal is described in the model, for the viewer's hover details and
//! for AI agents.

use std::collections::{BTreeMap, HashMap, HashSet};

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;

use super::netlist::{self, Bit, Cell, Module, plain_name};
use super::schematic::{Node, Port, Role, Schematic};
use super::symbols::{Shape, label_size};

/// A place in the design files.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Source {
    /// The file, named as the request named it: relative to the source
    /// folder, or as display_paths says.
    pub file: String,
    pub line: u32,
}

/// What a block is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// Logic from the design's code: a register, an adder, a multiplexer.
    Logic,
    /// An instance of another module of the design.
    Module,
    /// One of the FPGA's own blocks, such as a PLL or an I/O pin.
    Fpga,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    /// input or output.
    pub direction: String,
    pub width: usize,
    /// The signals it connects to, by name, or empty when they have none.
    pub signal: String,
    /// Drawn with a label instead of a wire.
    pub by_label: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub kind: BlockKind,
    /// The color category: reg, arith, cmp, mux, gate, mem, module, fpga.
    pub category: &'static str,
    /// What it is, in plain words, such as Register or PLL.
    pub title: String,
    /// The cell type as yosys names it, such as $dff or SB_PLL40_CORE.
    #[serde(rename = "type")]
    pub cell_type: String,
    /// Its width in bits, when the type has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    /// A register's name: the signal it holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A module instance's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// A module instance's module, by its key in the diagram's modules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Where the block comes from in the design files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// A memory's initial contents: where they're loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_init: Option<Source>,
    /// A memory or memory port's memory, by name: its read and write ports
    /// share it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// Inputs fixed to a value, by port, such as {"B": "1"}.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub fixed: IndexMap<String, String>,
    pub ports: IndexMap<String, PortInfo>,
}

/// A label drawn instead of a wire.
#[derive(Debug, Clone, Serialize)]
pub struct Label {
    /// The signal's name, or the fixed value.
    pub text: String,
    /// signal or fixed.
    pub kind: &'static str,
    /// in: the label feeds a port; out: a signal goes to the label.
    pub direction: &'static str,
    /// The block whose port it's at, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    pub bits: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pin {
    pub direction: String,
    pub width: usize,
    pub bits: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    /// Its name, or empty for a signal synthesis made.
    pub name: String,
    pub width: usize,
}

/// One module in the model.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleModel {
    /// The module's name.
    pub name: String,
    /// Whether it's the design's top module.
    pub top: bool,
    /// Where the module is defined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Its drawing, relative to the diagram's folder, when it was drawn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub svg_file: Option<String>,
    /// Why it wasn't drawn, when it wasn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub blocks: IndexMap<String, Block>,
    pub labels: IndexMap<String, Label>,
    pub pins: IndexMap<String, Pin>,
    /// Each net bit's signal.
    pub signals: BTreeMap<u64, Signal>,
    /// The bits that are clocks.
    pub clocks: Vec<u64>,
}

/// What the rules need to know about the rest of the design.
pub struct Context<'a> {
    /// The design's modules, by yosys name, to their key in the diagram.
    pub module_keys: &'a HashMap<String, String>,
    /// The FPGA family's plain block names.
    pub fpga_name: fn(&str) -> Option<&'static str>,
    /// A yosys `src` attribute as a source place.
    pub source: &'a (dyn Fn(&str) -> Option<Source> + Sync),
}

/// The wire classes the drawing needs: clocks, and signals and fixed values
/// drawn as labels.
#[derive(Debug, Default)]
pub struct WireClasses {
    pub clocks: HashSet<u64>,
    pub labeled: HashSet<u64>,
    pub fixed: HashSet<u64>,
}

impl WireClasses {
    pub fn class(&self, bits: &[u64]) -> Option<&'static str> {
        if bits.iter().any(|b| self.fixed.contains(b)) {
            Some("fixed")
        } else if bits.iter().any(|b| self.clocks.contains(b)) {
            Some("clock")
        } else if bits.iter().any(|b| self.labeled.contains(b)) {
            Some("global")
        } else {
            None
        }
    }
}

/// Ports that carry a clock.
fn is_clock_port(port: &str) -> bool {
    matches!(
        port.to_ascii_lowercase().as_str(),
        "clk" | "clock" | "sys_clk"
    )
}

fn reset_name() -> Regex {
    Regex::new(r"(?i)(^|_)(rst|rstn|reset|resetn|nreset)($|_)").unwrap()
}

/// Yosys's internal cell types in plain words.
fn friendly(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "$add" => "Adder",
        "$sub" => "Subtractor",
        "$mul" => "Multiplier",
        "$div" => "Divider",
        "$mod" => "Remainder",
        "$neg" => "Negate",
        "$eq" => "Equal comparator",
        "$ne" => "Not-equal comparator",
        "$lt" => "Less-than comparator",
        "$le" => "Less-or-equal comparator",
        "$gt" => "Greater-than comparator",
        "$ge" => "Greater-or-equal comparator",
        "$mux" | "$pmux" | "$bmux" => "Multiplexer",
        "$not" => "Inverter",
        "$and" => "AND gate",
        "$or" => "OR gate",
        "$xor" => "XOR gate",
        "$xnor" => "XNOR gate",
        "$logic_not" => "Logical NOT",
        "$logic_and" => "Logical AND",
        "$logic_or" => "Logical OR",
        "$reduce_and" => "AND of all bits",
        "$reduce_or" => "OR of all bits",
        "$reduce_xor" => "XOR of all bits",
        "$reduce_xnor" => "XNOR of all bits",
        "$reduce_bool" => "Any bit set",
        "$shl" | "$sshl" => "Shift left",
        "$shr" => "Shift right",
        "$sshr" => "Arithmetic shift right",
        "$shift" | "$shiftx" => "Shifter",
        "$dff" => "Register",
        "$dffe" => "Register with enable",
        "$adff" => "Register with async reset",
        "$adffe" => "Register with async reset and enable",
        "$sdff" => "Register with reset",
        "$sdffe" | "$sdffce" => "Register with reset and enable",
        "$dffsr" | "$dffsre" => "Register with set and reset",
        "$aldff" | "$aldffe" => "Register with async load",
        "$dlatch" | "$adlatch" | "$dlatchsr" => "Latch",
        "$mem" | "$mem_v2" => "Memory",
        "$memrd" | "$memrd_v2" => "Memory read port",
        "$memwr" | "$memwr_v2" => "Memory write port",
        "$pos" => "Buffer",
        "$concat" => "Concatenation",
        "$tribuf" => "Tri-state buffer",
        super::as_written::REG => "Register",
        super::as_written::LOGIC => "Logic",
        _ => return None,
    })
}

/// A cell type's color category.
fn category(ty: &str) -> &'static str {
    if ty == super::as_written::REG
        || ty.starts_with("$dff")
        || ty.starts_with("$adff")
        || ty.starts_with("$sdff")
        || ty.starts_with("$aldff")
        || ty.starts_with("$dlatch")
        || ty.starts_with("$adlatch")
    {
        return "reg";
    }
    match ty {
        "$add" | "$sub" | "$mul" | "$neg" | "$shl" | "$shr" | "$sshr" | "$sshl" | "$div"
        | "$mod" | "$shift" | "$shiftx" => "arith",
        "$eq" | "$ne" | "$lt" | "$le" | "$gt" | "$ge" => "cmp",
        "$mux" | "$pmux" | "$bmux" => "mux",
        _ if ty.starts_with("$logic_") || ty.starts_with("$reduce_") => "cmp",
        _ if ty.starts_with("$mem") => "mem",
        _ => "gate",
    }
}

/// A fixed value as a label shows it: decimal, or hex from 10 up; "don't
/// care" when it has undefined bits (yosys's x: an input left unconnected,
/// whose value doesn't matter).
fn fixed_text(bits: &[Bit]) -> String {
    let digits: Option<String> = bits
        .iter()
        .rev()
        .map(|b| match b {
            Bit::Const(c) if c == "0" || c == "1" => Some(c.as_str()),
            _ => None,
        })
        .collect();
    match digits.and_then(|d| u64::from_str_radix(&d, 2).ok()) {
        Some(v) if bits.len() == 1 || v < 10 => v.to_string(),
        Some(v) => format!("0x{v:x}"),
        None => DONT_CARE.into(),
    }
}

/// The longest title shown above a block; longer ones are cut, with "…".
const MAX_TITLE: usize = 18;

/// A title, cut to `MAX_TITLE` characters.
fn short_title(t: &str) -> String {
    if t.chars().count() <= MAX_TITLE {
        t.to_string()
    } else {
        let cut: String = t.chars().take(MAX_TITLE - 1).collect();
        format!("{cut}…")
    }
}

/// The short sign inside a generic logic block, saying what it is.
fn glyph(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "$memrd" | "$memrd_v2" => "RD",
        "$memwr" | "$memwr_v2" => "WR",
        "$mem" | "$mem_v2" => "MEM",
        "$reduce_and" => "&",
        "$reduce_or" | "$reduce_bool" => "|",
        "$reduce_xor" => "^",
        "$reduce_xnor" => "~^",
        "$ne" => "≠",
        "$mul" => "×",
        "$div" => "÷",
        "$mod" => "%",
        "$neg" => "−",
        "$shl" | "$sshl" => "<<",
        "$shr" => ">>",
        "$sshr" => ">>>",
        "$shift" | "$shiftx" => "<>",
        "$bmux" => "MUX",
        "$tribuf" => "Z",
        _ if ty.starts_with("$dlatch") || ty.starts_with("$adlatch") => "LAT",
        _ => return None,
    })
}

/// The label of an input whose value doesn't matter.
pub const DONT_CARE: &str = "don't care";

/// The most bits a fixed value may have to be drawn as a label.
const MAX_LABEL_BITS: usize = 32;

/// A cell as the rules work on it.
struct Work {
    ty: String,
    dirs: IndexMap<String, String>,
    conns: IndexMap<String, Vec<Bit>>,
    cell: Cell,
}

impl Work {
    fn dir(&self, port: &str) -> &str {
        match self.dirs.get(port).map(String::as_str) {
            Some("output") => "output",
            // Unknown and inout ports are drawn as inputs.
            _ => "input",
        }
    }
}

/// Builds a module's schematic and model.
pub fn build(
    yosys_name: &str,
    module: &Module,
    cx: &Context,
) -> (Schematic, ModuleModel, WireClasses) {
    let src_of = |attrs: &IndexMap<String, serde_json::Value>| {
        netlist::text(attrs.get("src")).and_then(|s| (cx.source)(s))
    };
    let mut next_bit = 1 + module
        .cells
        .values()
        .flat_map(|c| c.connections.values().flatten())
        .chain(module.ports.values().flat_map(|p| &p.bits))
        .chain(module.netnames.values().flat_map(|n| &n.bits))
        .filter_map(Bit::net)
        .max()
        .unwrap_or(1);
    let mut fresh = |n: usize| -> Vec<Bit> {
        let bits = (next_bit..next_bit + n as u64).map(Bit::Net).collect();
        next_bit += n as u64;
        bits
    };

    // Memory initialization isn't logic: it's left out, and the memory
    // says where its initial contents are loaded.
    let mut inits: HashMap<String, Option<Source>> = HashMap::new();
    let mut work: IndexMap<String, Work> = IndexMap::new();
    for (name, c) in &module.cells {
        if c.ty.starts_with("$meminit") {
            let memid = netlist::text(c.parameters.get("MEMID"))
                .unwrap_or("")
                .to_string();
            inits.entry(memid).or_insert_with(|| src_of(&c.attributes));
            continue;
        }
        work.insert(
            name.clone(),
            Work {
                ty: c.ty.clone(),
                dirs: c.port_directions.clone(),
                conns: c.connections.clone(),
                cell: c.clone(),
            },
        );
    }

    let mut blocks: IndexMap<String, Block> = IndexMap::new();
    for (name, w) in &work {
        let t = &w.ty;
        let source = src_of(&w.cell.attributes);
        let width = ["WIDTH", "Y_WIDTH"]
            .iter()
            .find_map(|p| netlist::number(w.cell.parameters.get(*p)));
        let block = if let Some(key) = cx.module_keys.get(t) {
            Block {
                kind: BlockKind::Module,
                category: "module",
                title: plain_name(t),
                cell_type: plain_name(t),
                width: None,
                name: None,
                instance: Some(plain_name(name)),
                module: Some(key.clone()),
                source,
                memory_init: None,
                memory: None,
                fixed: IndexMap::new(),
                ports: IndexMap::new(),
            }
        } else if !t.starts_with('$') {
            Block {
                kind: BlockKind::Fpga,
                category: "fpga",
                title: (cx.fpga_name)(t).unwrap_or(t).to_string(),
                cell_type: t.clone(),
                width: None,
                name: None,
                instance: Some(plain_name(name)),
                module: None,
                source,
                memory_init: None,
                memory: None,
                fixed: IndexMap::new(),
                ports: IndexMap::new(),
            }
        } else {
            let memory_init = if t.starts_with("$mem") {
                let memid = netlist::text(w.cell.parameters.get("MEMID")).unwrap_or("");
                inits.get(memid).cloned().flatten()
            } else {
                None
            };
            Block {
                kind: BlockKind::Logic,
                category: category(t),
                title: friendly(t).unwrap_or(t).to_string(),
                cell_type: t.clone(),
                width,
                name: None,
                instance: None,
                module: None,
                source,
                memory_init,
                memory: netlist::text(w.cell.parameters.get("MEMID"))
                    .filter(|_| t.starts_with("$mem"))
                    .map(plain_name),
                fixed: IndexMap::new(),
                ports: IndexMap::new(),
            }
        };
        blocks.insert(name.clone(), block);
    }

    // Signal names, by bit: the first name yosys gives each.
    let mut named: HashMap<u64, String> = HashMap::new();
    for (n, nn) in &module.netnames {
        if !nn.hidden() {
            for b in nn.bits.iter().filter_map(Bit::net) {
                named.entry(b).or_insert_with(|| plain_name(n));
            }
        }
    }
    let clocks: HashSet<u64> = work
        .values()
        .flat_map(|w| {
            w.conns
                .iter()
                .filter(|(p, _)| is_clock_port(p))
                .flat_map(|(_, bits)| bits.iter().filter_map(Bit::net))
        })
        .collect();

    // Clocks and resets, drawn as labels when they go to two or more places
    // (block inputs and module outputs); one with a single destination stays
    // a wire.
    let reset = reset_name();
    let mut globals: HashSet<u64> = clocks.clone();
    globals.extend(
        named
            .iter()
            .filter(|(_, n)| reset.is_match(n))
            .map(|(b, _)| *b),
    );
    let mut sinks: HashMap<String, usize> = HashMap::new();
    let mut count = |bits: &[Bit]| {
        let names: HashSet<&String> = bits
            .iter()
            .filter_map(Bit::net)
            .filter(|b| globals.contains(b))
            .filter_map(|b| named.get(&b))
            .collect();
        for n in names {
            *sinks.entry(n.clone()).or_default() += 1;
        }
    };
    for w in work.values() {
        for (p, bits) in &w.conns {
            if w.dir(p) == "input" {
                count(bits);
            }
        }
    }
    for p in module.ports.values() {
        if p.direction == "output" {
            count(&p.bits);
        }
    }
    globals.retain(|b| {
        named
            .get(b)
            .is_some_and(|n| sinks.get(n).copied().unwrap_or(0) >= 2)
    });

    let mut labels: IndexMap<String, Label> = IndexMap::new();
    let mut label_nodes: Vec<Node> = vec![];
    let mut add_label = |labels: &mut IndexMap<String, Label>,
                         direction: &'static str,
                         text: &str,
                         bits: Vec<Bit>,
                         kind: &'static str,
                         block: Option<&str>,
                         port: Option<&str>| {
        let id = format!("l{}", labels.len());
        let size = label_size(text);
        let shape = if direction == "in" {
            Shape::LabelIn(size)
        } else {
            Shape::LabelOut(size)
        };
        let mut n = Node::new(id.clone(), shape, Role::Label(id.clone()));
        if direction == "in" {
            n.outputs = vec![Port::new("Y", bits.clone())];
        } else {
            n.inputs = vec![Port::new("A", bits.clone())];
        }
        n.text = Some((
            text.to_string(),
            if kind == "fixed" { "fixed" } else { "signal" },
        ));
        label_nodes.push(n);
        labels.insert(
            id,
            Label {
                text: text.to_string(),
                kind,
                direction,
                block: block.map(str::to_string),
                port: port.map(str::to_string),
                bits: bits.iter().filter_map(Bit::net).collect(),
            },
        );
    };

    // An input on a clock or reset: a fresh net from a label with its name.
    let mut by_label: HashMap<(String, String), String> = HashMap::new();
    for (name, w) in work.iter_mut() {
        let ports: Vec<String> = w.conns.keys().cloned().collect();
        for p in ports {
            let bits = &w.conns[&p];
            if w.dir(&p) != "input"
                || bits.is_empty()
                || !bits
                    .iter()
                    .all(|b| b.net().is_some_and(|n| globals.contains(&n)))
            {
                continue;
            }
            let names: HashSet<&String> =
                bits.iter().filter_map(|b| named.get(&b.net()?)).collect();
            if names.len() != 1 {
                continue;
            }
            let text = (*names.iter().next().unwrap()).clone();
            let new = fresh(bits.len());
            w.conns.insert(p.clone(), new.clone());
            by_label.insert((name.clone(), p.clone()), text.clone());
            add_label(
                &mut labels,
                "in",
                &text,
                new,
                "signal",
                Some(name),
                Some(&p),
            );
        }
    }
    // A block driving one of them: its output goes to a label too, since the
    // signal's inputs are labels now.
    let labeled_names: HashSet<String> = by_label.values().cloned().collect();
    for (name, w) in &work {
        for (p, bits) in &w.conns {
            if w.dir(p) != "output" || bits.is_empty() {
                continue;
            }
            let Some(nets) = bits.iter().map(Bit::net).collect::<Option<Vec<u64>>>() else {
                continue;
            };
            let names: HashSet<&String> = nets.iter().filter_map(|b| named.get(b)).collect();
            if names.len() == 1
                && labeled_names.contains(*names.iter().next().unwrap())
                && nets.iter().all(|b| globals.contains(b))
            {
                let text = (*names.iter().next().unwrap()).clone();
                add_label(
                    &mut labels,
                    "out",
                    &text,
                    bits.clone(),
                    "signal",
                    Some(name),
                    Some(p),
                );
            }
        }
    }
    // A pin whose signal now only feeds labels: a label after it.
    let still_used: HashSet<u64> = work
        .values()
        .flat_map(|w| {
            w.conns
                .iter()
                .filter(|(p, _)| w.dir(p) == "input")
                .flat_map(|(_, bits)| bits.iter().filter_map(Bit::net))
        })
        .collect();
    for (pn, pv) in &module.ports {
        if pv.direction == "input"
            && !pv.bits.is_empty()
            && pv.bits.iter().all(|b| {
                b.net()
                    .is_some_and(|n| globals.contains(&n) && !still_used.contains(&n))
            })
        {
            add_label(
                &mut labels,
                "out",
                &plain_name(pn),
                pv.bits.clone(),
                "signal",
                None,
                None,
            );
        }
    }

    // Fixed inputs: a short wire from a label with the value, instead of
    // shared constant blocks and their long wires.
    let mut pins: IndexMap<String, Vec<Bit>> = module
        .ports
        .iter()
        .map(|(n, p)| (n.clone(), p.bits.clone()))
        .collect();
    let all_fixed = |bits: &[Bit]| {
        !bits.is_empty()
            && bits.len() <= MAX_LABEL_BITS
            && bits.iter().all(|b| matches!(b, Bit::Const(_)))
    };
    for (name, w) in work.iter_mut() {
        let ports: Vec<String> = w.conns.keys().cloned().collect();
        for p in ports {
            let bits = w.conns[&p].clone();
            if w.dir(&p) != "input" || !all_fixed(&bits) {
                continue;
            }
            let text = fixed_text(&bits);
            let new = fresh(bits.len());
            w.conns.insert(p.clone(), new.clone());
            if let Some(b) = blocks.get_mut(name) {
                b.fixed.insert(p.clone(), text.clone());
            }
            // An input whose value doesn't matter is left unconnected: a
            // fresh net with nothing on it draws no wire (the user's choice,
            // 2026-11-11). The model and the port's hover still say so.
            if text != DONT_CARE {
                add_label(&mut labels, "in", &text, new, "fixed", Some(name), Some(&p));
            }
        }
    }
    for (pn, pv) in &module.ports {
        if pv.direction != "input" && all_fixed(&pv.bits) {
            let text = fixed_text(&pv.bits);
            let new = fresh(pv.bits.len());
            pins.insert(pn.clone(), new.clone());
            if text == DONT_CARE {
                continue;
            }
            add_label(
                &mut labels,
                "in",
                &text,
                new,
                "fixed",
                None,
                Some(&plain_name(pn)),
            );
        }
    }

    // Each bit's signal: a named one where there is one, else yosys's own,
    // marked unnamed.
    let mut nets: BTreeMap<u64, (String, bool, usize)> = BTreeMap::new();
    for (n, nn) in &module.netnames {
        let hidden = nn.hidden();
        for b in nn.bits.iter().filter_map(Bit::net) {
            let replace = match nets.get(&b) {
                None => true,
                Some((_, was_hidden, _)) => !hidden && *was_hidden,
            };
            if replace {
                nets.insert(b, (plain_name(n), hidden, nn.bits.len()));
            }
        }
    }
    let mut classes = WireClasses {
        clocks: clocks.clone(),
        ..Default::default()
    };
    for l in labels.values() {
        let name = if l.kind == "fixed" {
            classes.fixed.extend(&l.bits);
            if l.text == DONT_CARE {
                "not connected (don't care)".to_string()
            } else {
                format!("fixed value {}", l.text)
            }
        } else {
            classes.labeled.extend(&l.bits);
            l.text.clone()
        };
        for b in &l.bits {
            nets.insert(*b, (name.clone(), false, l.bits.len()));
        }
    }

    // Each block's ports: direction and what they connect to.
    for (name, w) in &work {
        let Some(block) = blocks.get_mut(name) else {
            continue;
        };
        for (p, bits) in &w.conns {
            let mut names: Vec<&str> = vec![];
            for b in bits.iter().filter_map(Bit::net) {
                if let Some((n, false, _)) = nets.get(&b)
                    && !names.contains(&n.as_str())
                {
                    names.push(n);
                }
            }
            let label = by_label.get(&(name.clone(), p.clone()));
            block.ports.insert(
                p.clone(),
                PortInfo {
                    direction: w.dir(p).to_string(),
                    width: bits.len(),
                    signal: label.cloned().unwrap_or_else(|| names.join(", ")),
                    by_label: label.is_some(),
                },
            );
        }
        // A register is named after the signal it holds; a block drawn as
        // written, after its signal.
        if let Some(n) = netlist::text(w.cell.parameters.get(super::as_written::NAME)) {
            block.name = Some(n.to_string());
        } else if block.category == "reg"
            && let Some(q) = block.ports.get("Q")
            && !q.signal.is_empty()
        {
            block.name = Some(q.signal.clone());
        }
    }

    // The schematic: blocks, pins, then labels.
    let mut nodes: Vec<Node> = vec![];
    for (i, (name, w)) in work.iter().enumerate() {
        let block = &blocks[name];
        let shape = match block.kind {
            BlockKind::Logic => Shape::for_cell(&w.ty, w.conns.keys().map(String::as_str)),
            _ => Shape::Generic,
        };
        let mut n = Node::new(format!("c{i}"), shape, Role::Block(name.clone()));
        for (p, bits) in &w.conns {
            let port = Port::new(p, bits.clone());
            if w.dir(p) == "output" {
                n.outputs.push(port);
            } else {
                n.inputs.push(port);
            }
        }
        n.category = Some(block.category);
        n.is_dff = w.ty == "$dff";
        // Above a block only a name worth reading: a module's, a register's
        // or a memory's (shared by its ports), cut when long. What other
        // blocks are shows as a short sign inside, and in full on hover.
        n.title = match block.kind {
            BlockKind::Module => Some(block.title.clone()),
            BlockKind::Fpga => None,
            BlockKind::Logic => block.memory.clone().or_else(|| block.name.clone()),
        }
        .map(|t| short_title(&t));
        if shape == Shape::Generic && block.kind == BlockKind::Logic {
            n.glyph = glyph(&w.ty).map(str::to_string);
        }
        // Submodules and blocks drawn as written name their inputs; a
        // module's outputs too (a written block's output is its title).
        n.port_names = block.kind == BlockKind::Module || w.ty.starts_with("$logix_");
        n.output_names = block.kind == BlockKind::Module;
        n.memory = block.memory.clone();
        nodes.push(n);
    }
    let mut pin_models: IndexMap<String, Pin> = IndexMap::new();
    for (i, (pn, pv)) in module.ports.iter().enumerate() {
        let bits = pins[pn].clone();
        let name = plain_name(pn);
        let input = pv.direction == "input";
        let mut n = Node::new(
            format!("p{i}"),
            if input { Shape::PinIn } else { Shape::PinOut },
            Role::Pin(name.clone()),
        );
        if input {
            n.outputs = vec![Port::new("Y", bits.clone())];
        } else {
            n.inputs = vec![Port::new("A", bits.clone())];
        }
        n.title = Some(name.clone());
        n.category = Some("port");
        nodes.push(n);
        pin_models.insert(
            name,
            Pin {
                direction: pv.direction.clone(),
                width: pv.bits.len(),
                bits: pv.bits.iter().filter_map(Bit::net).collect(),
            },
        );
    }
    nodes.extend(label_nodes);

    let mut clock_list: Vec<u64> = clocks.into_iter().collect();
    clock_list.sort_unstable();
    let model = ModuleModel {
        name: plain_name(yosys_name),
        top: module.is_top(),
        source: src_of(&module.attributes),
        svg_file: None,
        note: None,
        blocks,
        labels,
        pins: pin_models,
        signals: nets
            .into_iter()
            .map(|(b, (name, hidden, width))| {
                (
                    b,
                    Signal {
                        name: if hidden { String::new() } else { name },
                        width,
                    },
                )
            })
            .collect(),
        clocks: clock_list,
    };
    (Schematic { nodes, next_bit }, model, classes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::netlist::Netlist;

    fn module(json: &str) -> Module {
        let n: Netlist = serde_json::from_str(json).unwrap();
        n.modules.into_iter().next().unwrap().1
    }

    fn cx_build(m: &Module) -> (Schematic, ModuleModel, WireClasses) {
        let keys = HashMap::new();
        let source = |s: &str| {
            let (f, rest) = s.split_once(':')?;
            Some(Source {
                file: f.into(),
                line: rest.split('.').next()?.parse().ok()?,
            })
        };
        build(
            "main",
            m,
            &Context {
                module_keys: &keys,
                fpga_name: |t| (t == "SB_IO").then_some("I/O pin"),
                source: &source,
            },
        )
    }

    /// Two registers on one clock, one with a fixed input, and a reset.
    const DESIGN: &str = r#"{"modules":{"main":{
      "attributes":{"top":1,"src":"main.v:1.1-20.10"},
      "ports":{"clk":{"direction":"input","bits":[2]},"rst":{"direction":"input","bits":[3]},
               "led":{"direction":"output","bits":[4]},"zero":{"direction":"output","bits":["0"]}},
      "cells":{
        "$procdff$1":{"type":"$dff","attributes":{"src":"main.v:5.3-5.20"},
          "port_directions":{"CLK":"input","D":"input","Q":"output"},
          "connections":{"CLK":[2],"D":[5],"Q":[4]}},
        "$procdff$2":{"type":"$sdff","parameters":{"WIDTH":1},
          "port_directions":{"CLK":"input","SRST":"input","D":"input","Q":"output"},
          "connections":{"CLK":[2],"SRST":[3],"D":["1"],"Q":[5]}},
        "pad":{"type":"SB_IO","port_directions":{"PACKAGE_PIN":"inout"},"connections":{"PACKAGE_PIN":[3]}},
        "$meminit$9":{"type":"$meminit_v2","parameters":{"MEMID":"\\rom"},"attributes":{"src":"main.v:9.5-9.30"},
          "connections":{}},
        "$memrd$8":{"type":"$memrd_v2","parameters":{"MEMID":"\\rom"},
          "port_directions":{"ADDR":"input","EN":"input","DATA":"output"},"connections":{"ADDR":[5],"EN":["x"],"DATA":[6]}}
      },
      "netnames":{"clk":{"hide_name":0,"bits":[2]},"rst":{"hide_name":0,"bits":[3]},
                  "led":{"hide_name":0,"bits":[4]},"$auto$1":{"hide_name":1,"bits":[5]},
                  "q":{"hide_name":0,"bits":[5]}}
    }}}"#;

    #[test]
    fn the_prototype_rules() {
        let (s, m, classes) = cx_build(&module(DESIGN));
        // The clock goes to two registers: labels, and a label after its pin.
        let clk: Vec<&Label> = m.labels.values().filter(|l| l.text == "clk").collect();
        assert_eq!(clk.iter().filter(|l| l.direction == "in").count(), 2);
        assert_eq!(clk.iter().filter(|l| l.direction == "out").count(), 1);
        // The reset goes to the register and the I/O block: labels too.
        assert_eq!(m.labels.values().filter(|l| l.text == "rst").count(), 3);
        // A fixed input: a label with its value, and on the block.
        let reg = &m.blocks["$procdff$2"];
        assert_eq!(reg.fixed["D"], "1");
        assert_eq!(reg.title, "Register with reset");
        assert_eq!(reg.name.as_deref(), Some("q"));
        assert_eq!(m.blocks["$memrd$8"].memory.as_deref(), Some("rom"));
        // An input whose value doesn't matter: no label, no wire, but the
        // model says so.
        assert_eq!(m.blocks["$memrd$8"].fixed["EN"], DONT_CARE);
        assert!(!m.labels.values().any(|l| l.port.as_deref() == Some("EN")));
        let node = |name: &str| {
            s.nodes
                .iter()
                .find(|n| n.role == Role::Block(name.into()))
                .unwrap()
        };
        // A memory port: its memory's name above, a sign inside.
        assert_eq!(node("$memrd$8").title.as_deref(), Some("rom"));
        assert_eq!(node("$memrd$8").glyph.as_deref(), Some("RD"));
        assert_eq!(node("$procdff$2").title.as_deref(), Some("q"));
        assert_eq!(node("pad").title, None);
        assert!(reg.ports["CLK"].by_label);
        assert_eq!(reg.ports["CLK"].signal, "clk");
        // A fixed module output: a label too.
        assert!(
            m.labels
                .values()
                .any(|l| l.kind == "fixed" && l.port.as_deref() == Some("zero"))
        );
        // Memory initialization: left out, and on the memory.
        assert!(!m.blocks.contains_key("$meminit$9"));
        assert_eq!(
            m.blocks["$memrd$8"].memory_init,
            Some(Source {
                file: "main.v".into(),
                line: 9
            })
        );
        assert_eq!(m.blocks["pad"].kind, BlockKind::Fpga);
        assert_eq!(m.blocks["pad"].title, "I/O pin");
        assert_eq!(
            m.source,
            Some(Source {
                file: "main.v".into(),
                line: 1
            })
        );
        assert_eq!(m.clocks, [2]);
        assert_eq!(m.signals[&5].name, "q");
        assert_eq!(classes.class(&[2]), Some("clock"));
        // Every node: 4 cells, 4 pins and the labels.
        assert_eq!(s.nodes.len(), 4 + 4 + m.labels.len());
    }

    #[test]
    fn a_signal_with_one_destination_stays_a_wire() {
        let one = DESIGN.replace(r#""CLK":[2],"SRST""#, r#""CLK":[7],"SRST""#);
        let (_, m, _) = cx_build(&module(&one));
        assert!(!m.labels.values().any(|l| l.text == "clk"));
    }

    #[test]
    fn long_titles_are_cut() {
        assert_eq!(short_title("bram_data_out"), "bram_data_out");
        assert_eq!(
            short_title("a_very_long_register_name"),
            "a_very_long_regis…"
        );
        assert_eq!(glyph("$memwr_v2"), Some("WR"));
        assert_eq!(glyph("$sdffe"), None);
        assert_eq!(glyph("$add"), None);
    }

    #[test]
    fn fixed_values() {
        let c = |s: &str| {
            s.chars()
                .rev()
                .map(|c| Bit::Const(c.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(fixed_text(&c("1")), "1");
        assert_eq!(fixed_text(&c("0101")), "5");
        assert_eq!(fixed_text(&c("11111111")), "0xff");
        assert_eq!(fixed_text(&c("1x")), "don't care");
    }
}
