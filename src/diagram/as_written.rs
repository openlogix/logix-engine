//! Modules drawn as written (the user's decision, 2026-11-22): each module
//! as its submodules, FPGA blocks and memories, plus one block per signal
//! its own code drives, instead of the multiplexers, adders, comparators
//! and bus splits yosys turns that code into. A register is a block with
//! its clock; any other driven signal, a block of logic. A block's inputs
//! are the named signals its logic reads (inputs, registers, submodule
//! outputs, the signal itself for a counter), each a port named after the
//! signal, so the drawing reads like the code. A value computed straight
//! into a submodule's port, such as `.valid(a || b)`, gets a block named
//! after that port.
//!
//! It rewrites yosys's netlist before the rules (`rules.rs`) draw it: the
//! drawing code stays the same, and the rules know the two block types,
//! `REG` and `LOGIC`. A register block is drawn without a label, its clock
//! input as a triangle (schematic.rs).

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use serde_json::Value;

use super::netlist::{Bit, Cell, Module, plain_name};

/// A register: a signal a clocked process drives.
pub const REG: &str = "$logix_reg";
/// Any other signal the module's logic drives.
pub const LOGIC: &str = "$logix_logic";
/// The parameter holding a block's signal name.
pub const NAME: &str = "NAME";

/// Kept as they are: submodules and FPGA blocks (yosys names them without a
/// `$`, or `$paramod…` for a module with parameters), and memories.
fn kept(ty: &str) -> bool {
    !ty.starts_with('$') || ty.starts_with("$paramod") || ty.starts_with("$mem")
}

/// A register or a latch.
fn is_flop(ty: &str) -> bool {
    ["$dff", "$adff", "$sdff", "$aldff", "$dlatch", "$adlatch"]
        .iter()
        .any(|p| ty.starts_with(p))
}

/// A flop's clock input.
fn is_clock(port: &str) -> bool {
    port == "CLK"
}

/// One block of the drawing: a signal and what its logic reads.
struct Group {
    /// The signal's name, as people write it.
    name: String,
    /// The bits the logic drives.
    out: Vec<Bit>,
    reg: bool,
    clock: Vec<Bit>,
    /// What the logic reads: the signal's name and all its bits.
    sources: IndexMap<String, Vec<Bit>>,
    /// Where the logic is written.
    src: Option<Value>,
}

/// The module as written: its kept cells as they are, and one block per
/// driven signal in place of the rest.
pub fn as_written(m: &Module) -> Module {
    let dir = |c: &Cell, p: &str| c.port_directions.get(p).map(String::as_str) == Some("output");

    // Each bit's signal: the first name yosys gives it that people wrote.
    let mut name_of: HashMap<u64, String> = HashMap::new();
    let mut bits_of: IndexMap<String, Vec<Bit>> = IndexMap::new();
    for (n, nn) in &m.netnames {
        if nn.hidden() {
            continue;
        }
        for b in nn.bits.iter().filter_map(Bit::net) {
            name_of.entry(b).or_insert_with(|| n.clone());
        }
        bits_of.insert(n.clone(), nn.bits.clone());
    }

    // What drives each bit: a logic cell (by index), or a kept cell's port.
    let logic: Vec<(&String, &Cell)> = m.cells.iter().filter(|(_, c)| !kept(&c.ty)).collect();
    let mut driver: HashMap<u64, usize> = HashMap::new();
    for (i, (_, c)) in logic.iter().enumerate() {
        for (p, bits) in &c.connections {
            if dir(c, p) {
                for b in bits.iter().filter_map(Bit::net) {
                    driver.insert(b, i);
                }
            }
        }
    }
    let mut kept_out: HashMap<u64, (String, String)> = HashMap::new();
    for (n, c) in m.cells.iter().filter(|(_, c)| kept(&c.ty)) {
        for (p, bits) in &c.connections {
            if dir(c, p) {
                for b in bits.iter().filter_map(Bit::net) {
                    kept_out.insert(b, (n.clone(), p.clone()));
                }
            }
        }
    }

    // The block of the logic driving `out`: everything it reads, back to
    // named signals and kept cells' outputs.
    let trace = |name: String, out: Vec<Bit>| -> Group {
        let mut g = Group {
            name,
            out: out.clone(),
            reg: false,
            clock: vec![],
            sources: IndexMap::new(),
            src: None,
        };
        let mut seen: HashSet<usize> = HashSet::new();
        let mut todo: Vec<usize> = vec![];
        for b in out.iter().filter_map(Bit::net) {
            if let Some(&i) = driver.get(&b)
                && seen.insert(i)
            {
                let c = logic[i].1;
                g.reg |= is_flop(&c.ty);
                if g.src.is_none() {
                    g.src = c.attributes.get("src").cloned();
                }
                todo.push(i);
            }
        }
        while let Some(i) = todo.pop() {
            let c = logic[i].1;
            for (p, bits) in &c.connections {
                if dir(c, p) {
                    continue;
                }
                if is_flop(&c.ty) && is_clock(p) {
                    if g.clock.is_empty() {
                        g.clock = bits.clone();
                    }
                    continue;
                }
                for b in bits.iter().filter_map(Bit::net) {
                    if let Some(n) = name_of.get(&b) {
                        g.sources
                            .entry(plain_name(n))
                            .or_insert_with(|| bits_of[n].clone());
                    } else if let Some(&j) = driver.get(&b) {
                        if seen.insert(j) {
                            todo.push(j);
                        }
                    } else if let Some((cell, port)) = kept_out.get(&b) {
                        g.sources
                            .entry(format!("{}.{}", plain_name(cell), port))
                            .or_insert_with(|| m.cells[cell].connections[port].clone());
                    }
                }
            }
        }
        g
    };

    // A block per named signal the logic drives, in the order yosys lists
    // the names, then per kept cell's input computed in place.
    let mut groups: Vec<Group> = vec![];
    let mut covered: HashSet<u64> = HashSet::new();
    for (n, bits) in &bits_of {
        let out: Vec<Bit> = bits
            .iter()
            .filter(|b| {
                b.net().is_some_and(|b| {
                    driver.contains_key(&b) && name_of.get(&b) == Some(n) && !covered.contains(&b)
                })
            })
            .cloned()
            .collect();
        if out.is_empty() {
            continue;
        }
        covered.extend(out.iter().filter_map(Bit::net));
        groups.push(trace(plain_name(n), out));
    }
    for (cn, c) in m.cells.iter().filter(|(_, c)| kept(&c.ty)) {
        for (p, bits) in &c.connections {
            if dir(c, p) {
                continue;
            }
            let out: Vec<Bit> = bits
                .iter()
                .filter(|b| {
                    b.net().is_some_and(|b| {
                        driver.contains_key(&b)
                            && !name_of.contains_key(&b)
                            && !covered.contains(&b)
                    })
                })
                .cloned()
                .collect();
            if out.is_empty() {
                continue;
            }
            covered.extend(out.iter().filter_map(Bit::net));
            groups.push(trace(format!("{}.{}", plain_name(cn), p), out));
        }
    }

    let mut cells: IndexMap<String, Cell> = m
        .cells
        .iter()
        .filter(|(_, c)| kept(&c.ty))
        .map(|(n, c)| (n.clone(), c.clone()))
        .collect();
    for g in groups {
        let mut connections: IndexMap<String, Vec<Bit>> = IndexMap::new();
        let mut port_directions: IndexMap<String, String> = IndexMap::new();
        if g.reg && !g.clock.is_empty() {
            connections.insert("CLK".into(), g.clock);
            port_directions.insert("CLK".into(), "input".into());
        }
        for (s, bits) in g.sources {
            port_directions.insert(s.clone(), "input".into());
            connections.insert(s, bits);
        }
        let out = if g.reg { "Q" } else { "Y" };
        port_directions.insert(out.into(), "output".into());
        connections.insert(out.into(), g.out.clone());
        let mut parameters = IndexMap::new();
        parameters.insert(NAME.into(), Value::String(g.name.clone()));
        parameters.insert("WIDTH".into(), Value::from(g.out.len() as u64));
        let mut attributes = IndexMap::new();
        if let Some(src) = g.src {
            attributes.insert("src".into(), src);
        }
        cells.insert(
            format!("{}${}", if g.reg { REG } else { LOGIC }, g.name),
            Cell {
                ty: if g.reg { REG } else { LOGIC }.into(),
                parameters,
                attributes,
                port_directions,
                connections,
            },
        );
    }
    Module {
        attributes: m.attributes.clone(),
        ports: m.ports.clone(),
        cells,
        netnames: m
            .netnames
            .iter()
            .filter(|(_, n)| !n.hidden())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::netlist::{ExtPort, NetName};

    fn cell(ty: &str, ports: &[(&str, &str, Vec<u64>)]) -> Cell {
        Cell {
            ty: ty.into(),
            parameters: IndexMap::new(),
            attributes: IndexMap::from([("src".into(), Value::from("main.v:7.3-9.6"))]),
            port_directions: ports
                .iter()
                .map(|(p, d, _)| (p.to_string(), d.to_string()))
                .collect(),
            connections: ports
                .iter()
                .map(|(p, _, b)| (p.to_string(), b.iter().map(|&n| Bit::Net(n)).collect()))
                .collect(),
        }
    }

    fn net(bits: &[u64], hidden: bool) -> NetName {
        NetName {
            hide_name: Value::from(u64::from(hidden)),
            bits: bits.iter().map(|&n| Bit::Net(n)).collect(),
        }
    }

    /// A counter that counts when `run` is 1, its top bit on `led`, and a
    /// submodule whose input is `run && tick`, computed in its port.
    fn design() -> Module {
        Module {
            attributes: IndexMap::new(),
            ports: IndexMap::from([
                (
                    "clk".into(),
                    ExtPort {
                        direction: "input".into(),
                        bits: vec![Bit::Net(2)],
                    },
                ),
                (
                    "run".into(),
                    ExtPort {
                        direction: "input".into(),
                        bits: vec![Bit::Net(3)],
                    },
                ),
                (
                    "led".into(),
                    ExtPort {
                        direction: "output".into(),
                        bits: vec![Bit::Net(11)],
                    },
                ),
            ]),
            cells: IndexMap::from([
                (
                    "$add$1".into(),
                    cell(
                        "$add",
                        &[
                            ("A", "input", vec![10, 11]),
                            ("B", "input", vec![20, 21]),
                            ("Y", "output", vec![30, 31]),
                        ],
                    ),
                ),
                (
                    "$mux$2".into(),
                    cell(
                        "$mux",
                        &[
                            ("A", "input", vec![10, 11]),
                            ("B", "input", vec![30, 31]),
                            ("S", "input", vec![3]),
                            ("Y", "output", vec![40, 41]),
                        ],
                    ),
                ),
                (
                    "$dff$3".into(),
                    cell(
                        "$dff",
                        &[
                            ("CLK", "input", vec![2]),
                            ("D", "input", vec![40, 41]),
                            ("Q", "output", vec![10, 11]),
                        ],
                    ),
                ),
                (
                    "$and$4".into(),
                    cell(
                        "$logic_and",
                        &[
                            ("A", "input", vec![3]),
                            ("B", "input", vec![50]),
                            ("Y", "output", vec![60]),
                        ],
                    ),
                ),
                (
                    "timer".into(),
                    cell(
                        "timer",
                        &[
                            ("clk", "input", vec![2]),
                            ("go", "input", vec![60]),
                            ("tick", "output", vec![50]),
                        ],
                    ),
                ),
            ]),
            netnames: IndexMap::from([
                ("clk".into(), net(&[2], false)),
                ("run".into(), net(&[3], false)),
                ("counter".into(), net(&[10, 11], false)),
                ("led".into(), net(&[11], false)),
                ("tick".into(), net(&[50], false)),
                ("$add$1_Y".into(), net(&[30, 31], true)),
                ("$and$4_Y".into(), net(&[60], true)),
            ]),
        }
    }

    #[test]
    fn a_register_is_one_block_reading_named_signals() {
        let m = as_written(&design());
        // No adder, multiplexer, register or AND gate left: the submodule,
        // a register block and the port's logic block.
        let types: Vec<&str> = m.cells.values().map(|c| c.ty.as_str()).collect();
        assert_eq!(types, ["timer", REG, LOGIC], "{types:?}");
        let reg = &m.cells[&format!("{REG}$counter")];
        assert_eq!(reg.parameters[NAME], "counter");
        assert_eq!(reg.connections["CLK"], vec![Bit::Net(2)]);
        // It reads itself (it counts) and run, both whole signals.
        let inputs: Vec<&String> = reg
            .connections
            .keys()
            .filter(|k| reg.port_directions[*k] == "input")
            .collect();
        assert_eq!(inputs, ["CLK", "counter", "run"]);
        assert_eq!(reg.connections["Q"], vec![Bit::Net(10), Bit::Net(11)]);
        assert_eq!(reg.attributes["src"], "main.v:7.3-9.6");
    }

    #[test]
    fn logic_in_a_submodule_port_is_named_after_the_port() {
        let m = as_written(&design());
        let and = &m.cells[&format!("{LOGIC}$timer.go")];
        assert_eq!(and.parameters[NAME], "timer.go");
        assert_eq!(and.connections["Y"], vec![Bit::Net(60)]);
        let inputs: Vec<&String> = and
            .connections
            .keys()
            .filter(|k| and.port_directions[*k] == "input")
            .collect();
        assert_eq!(inputs, ["run", "tick"]);
    }

    #[test]
    fn a_signal_with_two_names_is_one_block() {
        // led is counter's top bit: counter is named first, so led adds no
        // block, and the hidden names are gone.
        let m = as_written(&design());
        assert!(!m.cells.keys().any(|k| k.ends_with("$led")));
        assert!(m.netnames.keys().all(|k| !k.starts_with('$')));
    }
}
