//! One module as a schematic: its blocks and the wires between them, laid
//! out by ELK and drawn as SVG. The bus splits and joins, fixed values and
//! wiring follow netlistsvg (MIT), which the prototype used; the engine draws
//! with its own code and the current elkjs.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use indexmap::IndexMap;
use serde_json::{Value, json};

use super::netlist::Bit;
use super::symbols::{FIRST_PORT_Y, PORT_GAP, Shape};

/// A port of a node and the bits it connects to.
#[derive(Debug, Clone)]
pub struct Port {
    pub key: String,
    pub bits: Vec<Bit>,
}

impl Port {
    pub fn new(key: &str, bits: Vec<Bit>) -> Port {
        Port {
            key: key.to_string(),
            bits,
        }
    }

    /// The bits as netlistsvg keys them: ",1,2,3,".
    fn signature(&self) -> String {
        signature(&self.bits)
    }
}

fn signature(bits: &[Bit]) -> String {
    let mut s = String::from(",");
    for b in bits {
        match b {
            Bit::Net(n) => write!(s, "{n},").unwrap(),
            Bit::Const(c) => write!(s, "{c},").unwrap(),
        }
    }
    s
}

fn parse_signature(s: &str) -> Vec<Bit> {
    s.trim_matches(',')
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().map_or_else(|_| Bit::Const(p.into()), Bit::Net))
        .collect()
}

/// What a node stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    /// A cell of the module, by its yosys name.
    Block(String),
    /// A module port, by name.
    Pin(String),
    /// A label, by its id in the model.
    Label(String),
    /// A fixed value too wide for a label, as text.
    Constant(String),
    Split,
    Join,
}

#[derive(Debug, Clone)]
pub struct Node {
    /// The node's id in the layout.
    pub id: String,
    pub shape: Shape,
    pub role: Role,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    /// The text above the block, if any.
    pub title: Option<String>,
    /// A label's text and its class.
    pub text: Option<(String, &'static str)>,
    /// The block's color category, such as `reg` (its class is `cat-reg`).
    pub category: Option<&'static str>,
    /// A plain register ($dff): wires from it get no layout priority, as in
    /// netlistsvg.
    pub is_dff: bool,
    /// A short sign drawn inside a generic block, such as RD.
    pub glyph: Option<String>,
    /// A memory port's memory: hovering one of its blocks lights up the
    /// others.
    pub memory: Option<String>,
    /// A generic block that shows its ports' names inside it, inputs on the
    /// left and outputs on the right, and is as wide as they need: module
    /// instances and blocks drawn as written (the user's decision,
    /// 2026-11-22: a module's own logic as one block per signal).
    pub port_names: bool,
    /// With `port_names`, its outputs' names too.
    pub output_names: bool,
}

impl Node {
    pub fn new(id: String, shape: Shape, role: Role) -> Node {
        Node {
            id,
            shape,
            role,
            inputs: vec![],
            outputs: vec![],
            title: None,
            text: None,
            category: None,
            is_dff: false,
            glyph: None,
            memory: None,
            port_names: false,
            output_names: false,
        }
    }

    /// The node's width and height.
    pub fn size(&self) -> (f64, f64) {
        let (w, h) = self.shape.size();
        if !self.shape.is_stretchy() {
            return (w, h);
        }
        let (ins, outs) = (self.inputs.len() as f64, self.outputs.len() as f64);
        let n = if ins > outs { ins } else { outs };
        (self.width(w), (h + PORT_GAP * (n - 2.0)).max(20.0))
    }

    /// A generic block's width: wide enough for its port names and its
    /// sign when it shows them.
    fn width(&self, w: f64) -> f64 {
        if !(self.port_names && self.shape == Shape::Generic) {
            return w;
        }
        let widest = |ports: &[Port]| {
            ports
                .iter()
                .map(|p| text_width(&p.key))
                .fold(0.0_f64, f64::max)
        };
        let outs = if self.output_names {
            widest(&self.outputs)
        } else {
            0.0
        };
        let names = widest(&self.inputs) + outs;
        let sign = self.glyph.as_deref().map_or(0.0, |g| text_width(g) + 8.0);
        w.max(names + sign + 16.0)
    }

    /// Where a port is, relative to the node.
    pub fn port_position(&self, key: &str, input: bool) -> Option<(f64, f64)> {
        if !self.shape.is_stretchy() {
            return self.shape.port(key);
        }
        let list = if input { &self.inputs } else { &self.outputs };
        let i = list.iter().position(|p| p.key == key)? as f64;
        let stacked = FIRST_PORT_Y + i * PORT_GAP;
        Some(match (self.shape, input) {
            (Shape::Generic, true) => (0.0, stacked),
            (Shape::Generic, false) => (self.size().0, stacked),
            (Shape::Split, true) => (0.0, FIRST_PORT_Y),
            (Shape::Split, false) => (4.0, stacked),
            (Shape::Join, true) => (0.0, stacked),
            (_, _) => (5.0, FIRST_PORT_Y),
        })
    }

    /// The layout's ports: every port of a fixed symbol, connected or not,
    /// or the node's own ports.
    fn layout_ports(&self) -> Vec<(String, f64, f64)> {
        if !self.shape.is_stretchy() {
            return self
                .shape
                .ports()
                .iter()
                .map(|(k, x, y)| (k.to_string(), *x, *y))
                .collect();
        }
        let mut out = vec![];
        for (list, input) in [(&self.inputs, true), (&self.outputs, false)] {
            for p in list {
                let (x, y) = self.port_position(&p.key, input).unwrap_or_default();
                out.push((p.key.clone(), x, y));
            }
        }
        out
    }
}

/// A net and the ports it connects.
struct Wire {
    net: String,
    /// (node index, port key) pairs.
    drivers: Vec<(usize, String)>,
    riders: Vec<(usize, String)>,
}

pub struct Schematic {
    pub nodes: Vec<Node>,
    /// A net number no bit uses yet.
    pub next_bit: u64,
}

impl Schematic {
    /// Fixed values left after the labels (wider than a label shows, or
    /// mixed with signals): each run of fixed bits in an input becomes a
    /// constant block, shared by every input with the same value, as
    /// netlistsvg does.
    pub fn add_constants(&mut self) {
        let mut by_value: HashMap<String, Vec<u64>> = HashMap::new();
        let mut made: Vec<Node> = vec![];
        let mut next = self.next_bit;
        for node in &mut self.nodes {
            for port in &mut node.inputs {
                let mut run: Vec<usize> = vec![];
                let len = port.bits.len();
                for i in 0..=len {
                    if i < len && matches!(port.bits[i], Bit::Const(_)) {
                        run.push(i);
                        continue;
                    }
                    if run.is_empty() {
                        continue;
                    }
                    // The value, most significant bit first.
                    let value: String = run
                        .iter()
                        .rev()
                        .map(|&j| match &port.bits[j] {
                            Bit::Const(c) => c.as_str(),
                            Bit::Net(_) => "",
                        })
                        .collect();
                    let nets = by_value.entry(value.clone()).or_insert_with(|| {
                        let nets: Vec<u64> = (0..run.len() as u64).map(|k| next + k).collect();
                        next += run.len() as u64;
                        let mut c = Node::new(
                            format!("k{}", made.len()),
                            Shape::Constant,
                            Role::Constant(constant_text(&value)),
                        );
                        c.outputs =
                            vec![Port::new("Y", nets.iter().map(|n| Bit::Net(*n)).collect())];
                        c.title = Some(constant_text(&value));
                        c.category = Some("const");
                        made.push(c);
                        nets
                    });
                    for (k, &j) in run.iter().enumerate() {
                        port.bits[j] = Bit::Net(nets[k]);
                    }
                    run.clear();
                }
            }
        }
        self.next_bit = next;
        self.nodes.extend(made);
    }

    /// The bus splits and joins: the fewest that connect every input to the
    /// outputs that drive its bits (netlistsvg's algorithm).
    pub fn add_splits_joins(&mut self) {
        let mut outputs: Vec<String> = self
            .nodes
            .iter()
            .flat_map(|n| n.outputs.iter().map(Port::signature))
            .collect();
        let inputs: Vec<String> = self
            .nodes
            .iter()
            .flat_map(|n| n.inputs.iter().map(Port::signature))
            .collect();
        let mut riders = inputs.clone();
        let mut splits: IndexMap<String, Vec<String>> = IndexMap::new();
        let mut joins: IndexMap<String, Vec<String>> = IndexMap::new();
        for input in &inputs {
            gather(
                &mut outputs,
                &mut riders,
                input,
                0,
                input.len(),
                &mut splits,
                &mut joins,
            );
        }
        let mut added = vec![];
        for (i, (target, sources)) in joins.into_iter().enumerate() {
            let bits = parse_signature(&target);
            let mut n = Node::new(format!("j{i}"), Shape::Join, Role::Join);
            n.inputs = dedup(sources)
                .into_iter()
                .map(|s| Port::new(&s, slice(&bits, &s)))
                .collect();
            n.outputs = vec![Port::new("Y", bits)];
            added.push(n);
        }
        for (i, (source, targets)) in splits.into_iter().enumerate() {
            let bits = parse_signature(&source);
            let mut n = Node::new(format!("s{i}"), Shape::Split, Role::Split);
            n.inputs = vec![Port::new("A", bits.clone())];
            n.outputs = dedup(targets)
                .into_iter()
                .map(|t| Port::new(&t, slice(&bits, &t)))
                .collect();
            added.push(n);
        }
        self.nodes.extend(added);
    }

    fn wires(&self) -> Vec<Wire> {
        let mut riders: IndexMap<String, Vec<(usize, String)>> = IndexMap::new();
        let mut drivers: IndexMap<String, Vec<(usize, String)>> = IndexMap::new();
        for (i, n) in self.nodes.iter().enumerate() {
            for p in &n.inputs {
                riders
                    .entry(p.signature())
                    .or_default()
                    .push((i, p.key.clone()));
            }
            for p in &n.outputs {
                drivers
                    .entry(p.signature())
                    .or_default()
                    .push((i, p.key.clone()));
            }
        }
        let mut nets: Vec<String> = riders.keys().cloned().collect();
        for k in drivers.keys() {
            if !riders.contains_key(k) {
                nets.push(k.clone());
            }
        }
        nets.into_iter()
            .map(|net| Wire {
                drivers: drivers.get(&net).cloned().unwrap_or_default(),
                riders: riders.get(&net).cloned().unwrap_or_default(),
                net,
            })
            .collect()
    }

    /// The graph for ELK, with the layout options, and each edge's net and
    /// meeting point (a dummy node's number and whether the edge starts
    /// there), by edge index.
    pub fn layout_graph(&self, options: &Value) -> LayoutGraph {
        let mut children: Vec<Value> = self
            .nodes
            .iter()
            .map(|n| {
                let (w, h) = n.size();
                let ports: Vec<Value> = n
                    .layout_ports()
                    .into_iter()
                    .map(|(k, x, y)| {
                        json!({"id": format!("{}.{k}", n.id), "width": 0, "height": 0, "x": x, "y": y})
                    })
                    .collect();
                let mut child = json!({
                    "id": n.id, "width": w, "height": h, "ports": ports,
                    "layoutOptions": {"org.eclipse.elk.portConstraints": "FIXED_POS"},
                });
                // The title above the block: ELK makes room for it, so it
                // doesn't run into its neighbours.
                if let Some(t) = &n.title {
                    child["labels"] = json!([{"text": t, "width": text_width(t), "height": TITLE_HEIGHT}]);
                    child["layoutOptions"]["org.eclipse.elk.nodeLabels.placement"] =
                        json!("[H_CENTER, V_TOP, OUTSIDE]");
                }
                child
            })
            .collect();
        let mut edges: Vec<Value> = vec![];
        let mut nets: Vec<String> = vec![];
        let mut meets: Vec<Option<(usize, bool)>> = vec![];
        let mut dummies = 0;
        let port_id = |(i, k): &(usize, String)| format!("{}.{k}", self.nodes[*i].id);
        for w in self.wires() {
            if !w.drivers.is_empty() && !w.riders.is_empty() {
                for d in &w.drivers {
                    for r in &w.riders {
                        let mut e = json!({
                            "id": format!("e{}", edges.len()),
                            "sources": [port_id(d)], "targets": [port_id(r)],
                        });
                        if !self.nodes[d.0].is_dff {
                            e["layoutOptions"] =
                                json!({"org.eclipse.elk.layered.priority.direction": 10});
                        }
                        edges.push(e);
                        nets.push(w.net.clone());
                        meets.push(None);
                    }
                }
            } else if w.drivers.len() > 1 || w.riders.len() > 1 {
                // Several drivers and no riders, or the other way round: a
                // point they all meet at, removed when drawing.
                let number = dummies;
                let dummy = format!("$d_{number}");
                dummies += 1;
                children.push(json!({
                    "id": dummy, "width": 0, "height": 0,
                    "ports": [{"id": format!("{dummy}.p"), "width": 0, "height": 0}],
                    "layoutOptions": {"org.eclipse.elk.portConstraints": "FIXED_SIDE"},
                }));
                let to_dummy = w.riders.is_empty();
                for p in if to_dummy { &w.drivers } else { &w.riders } {
                    let (s, t) = if to_dummy {
                        (port_id(p), format!("{dummy}.p"))
                    } else {
                        (format!("{dummy}.p"), port_id(p))
                    };
                    edges.push(json!({
                        "id": format!("e{}", edges.len()),
                        "sources": [s], "targets": [t],
                    }));
                    nets.push(w.net.clone());
                    meets.push(Some((number, !to_dummy)));
                }
            }
        }
        LayoutGraph {
            graph: json!({"id": "root", "layoutOptions": options, "children": children, "edges": edges}),
            nets,
            meets,
        }
    }

    /// The SVG of the laid-out schematic. `classes` gives each wire's extra
    /// class from its bits, such as `clock`.
    pub fn render(
        &self,
        laid: &Value,
        layout: &LayoutGraph,
        classes: &dyn Fn(&[u64]) -> Option<&'static str>,
        module_key: &str,
    ) -> String {
        let positions: HashMap<&str, Placed> = laid["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let label = c["labels"][0]["x"]
                    .as_f64()
                    .zip(c["labels"][0]["y"].as_f64());
                Some((
                    c["id"].as_str()?,
                    (c["x"].as_f64()?, c["y"].as_f64()?, label),
                ))
            })
            .collect();
        let width = laid["width"].as_f64().unwrap_or(100.0);
        let height = laid["height"].as_f64().unwrap_or(100.0);
        // ELK's edges, by index, in the order they were given.
        let mut edges: Vec<Value> = vec![Value::Null; layout.nets.len()];
        for e in laid["edges"].as_array().into_iter().flatten() {
            if let Some(i) = edge_index(e)
                && i < edges.len()
            {
                edges[i] = e.clone();
            }
        }
        remove_dummy_edges(&mut edges, &layout.meets);

        let mut blocks = String::new();
        let mut boxes = String::new();
        for n in &self.nodes {
            let Some(&(x, y, label)) = positions.get(n.id.as_str()) else {
                continue;
            };
            render_node(n, x, y, label, &mut blocks, &mut boxes);
        }
        let mut wires = String::new();
        for (e, net) in edges.iter().zip(&layout.nets) {
            let bits: Vec<u64> = parse_signature(net).iter().filter_map(Bit::net).collect();
            let list = bits
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let class = match classes(&bits) {
                Some(c) => format!("wire {c}"),
                None => "wire".into(),
            };
            for s in e["sections"].as_array().into_iter().flatten() {
                let mut points = vec![point(&s["startPoint"])];
                points.extend(s["bendPoints"].as_array().into_iter().flatten().map(point));
                points.push(point(&s["endPoint"]));
                for pair in points.windows(2) {
                    let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
                    write!(
                        wires,
                        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" class="{class}" data-bits="{list}"/>"#,
                        num(x1),
                        num(y1),
                        num(x2),
                        num(y2)
                    )
                    .unwrap();
                }
            }
            for j in e["junctionPoints"].as_array().into_iter().flatten() {
                let (x, y) = point(j);
                write!(
                    wires,
                    r#"<circle cx="{}" cy="{}" r="2" class="{class} junction" data-bits="{list}"/>"#,
                    num(x),
                    num(y)
                )
                .unwrap();
            }
        }
        format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" class="logix-diagram" "#,
                r#"viewBox="0 0 {w} {h}" width="{w}" height="{h}" data-module="{m}">"#,
                "<style>{css}</style>",
                r#"<g class="blocks">{blocks}</g><g class="wires">{wires}</g><g class="ports">{boxes}</g></svg>"#
            ),
            w = num(width),
            h = num(height),
            m = esc(module_key),
            css = super::STYLE,
            blocks = blocks,
            wires = wires,
            boxes = boxes,
        )
    }
}

/// Where ELK put a node, and its title if it has one.
type Placed = (f64, f64, Option<(f64, f64)>);

/// The graph ELK lays out, with what drawing needs to know about its edges.
pub struct LayoutGraph {
    pub graph: Value,
    /// Each edge's net, by edge index.
    pub nets: Vec<String>,
    /// Each edge's meeting point, if any: the dummy node's number and
    /// whether the edge starts there.
    pub meets: Vec<Option<(usize, bool)>>,
}

impl LayoutGraph {
    pub fn node_count(&self) -> usize {
        self.graph["children"].as_array().map_or(0, Vec::len)
    }
}

fn edge_index(e: &Value) -> Option<usize> {
    e["id"].as_str()?.strip_prefix('e')?.parse().ok()
}

fn point(v: &Value) -> (f64, f64) {
    (
        v["x"].as_f64().unwrap_or_default(),
        v["y"].as_f64().unwrap_or_default(),
    )
}

/// Draws a node into `out`, and its port squares into `boxes` (drawn above
/// the wires, so no wire runs into one).
fn render_node(
    n: &Node,
    x: f64,
    y: f64,
    label: Option<(f64, f64)>,
    out: &mut String,
    boxes: &mut String,
) {
    let (w, h) = n.size();
    let mut class = String::from("block");
    if let Some(c) = n.category {
        write!(class, " cat-{c}").unwrap();
    }
    let data = match &n.role {
        Role::Block(name) => match &n.memory {
            Some(m) => format!(r#" data-block="{}" data-memory="{}""#, esc(name), esc(m)),
            None => format!(r#" data-block="{}""#, esc(name)),
        },
        Role::Pin(name) => format!(r#" data-pin="{}""#, esc(name)),
        Role::Label(id) => {
            class.push_str(" label");
            format!(r#" data-label="{}""#, esc(id))
        }
        Role::Constant(v) => format!(r#" data-const="{}""#, esc(v)),
        Role::Split | Role::Join => {
            let bits = match n.role {
                Role::Split => &n.inputs[0].bits,
                _ => &n.outputs[0].bits,
            };
            let kind = if n.role == Role::Split {
                "split"
            } else {
                "join"
            };
            class.push_str(" splitjoin");
            format!(
                r#" data-sj="{}" data-kind="{kind}" data-bits="{}""#,
                esc(&n.id),
                bits.iter()
                    .filter_map(Bit::net)
                    .map(|b| b.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    };
    write!(
        out,
        r#"<g class="{class}"{data} transform="translate({},{})">{}"#,
        num(x),
        num(y),
        match n.shape {
            Shape::Generic => format!(
                r#"<rect width="{}" height="{}" class="body"/>"#,
                num(w),
                num(h)
            ),
            _ => n.shape.draw(h),
        }
    )
    .unwrap();
    if n.port_names && n.shape == Shape::Generic {
        // Each port's name, inside the block next to its port.
        let outputs: &[Port] = if n.output_names { &n.outputs } else { &[] };
        for (ports, input) in [(n.inputs.as_slice(), true), (outputs, false)] {
            for (i, p) in ports.iter().enumerate() {
                let y = FIRST_PORT_Y + i as f64 * PORT_GAP + 3.5;
                let (x, anchor) = if input {
                    (4.0, "start")
                } else {
                    (w - 4.0, "end")
                };
                write!(
                    out,
                    r#"<text x="{}" y="{}" text-anchor="{anchor}" class="portname">{}</text>"#,
                    num(x),
                    num(y),
                    esc(&p.key)
                )
                .unwrap();
            }
        }
    }
    if let Some(t) = &n.title {
        // Where ELK put it, else centered just above the block.
        let (tx, ty) = match label {
            Some((lx, ly)) => (lx + text_width(t) / 2.0, ly + TITLE_HEIGHT - 2.0),
            None => (w / 2.0, -4.0),
        };
        write!(
            out,
            r#"<text x="{}" y="{}" class="nodelabel">{}</text>"#,
            num(tx),
            num(ty),
            esc(t)
        )
        .unwrap();
    }
    if let Some(g) = &n.glyph {
        // In the middle; with input names, on the right, clear of them
        // (outputs aren't named there).
        let x = if n.port_names {
            w - 5.0 - text_width(g) / 2.0
        } else {
            w / 2.0
        };
        write!(
            out,
            r#"<text x="{}" y="{}" class="glyph">{}</text>"#,
            num(x),
            num(h / 2.0 + 3.5),
            esc(g)
        )
        .unwrap();
    }
    if let Some((t, cls)) = &n.text {
        match n.shape {
            Shape::LabelIn(_) => write!(
                out,
                r#"<text x="{}" y="3" text-anchor="end" class="lbltext {cls}">{}</text>"#,
                num(w - 3.0),
                esc(t)
            ),
            _ => write!(
                out,
                r#"<text x="3" y="3" class="lbltext {cls}">{}</text>"#,
                esc(t)
            ),
        }
        .unwrap();
    }
    out.push_str("</g>");

    let square = |boxes: &mut String, bx: f64, by: f64, data: String| {
        write!(
            boxes,
            r#"<rect x="{}" y="{}" width="5" height="5" class="portbox"{data}/>"#,
            num(bx - 2.5),
            num(by - 2.5)
        )
        .unwrap();
    };
    match &n.role {
        Role::Block(name) => {
            for (list, input) in [(&n.inputs, true), (&n.outputs, false)] {
                for p in list {
                    if let Some((px, py)) = n.port_position(&p.key, input) {
                        let data =
                            format!(r#" data-block="{}" data-port="{}""#, esc(name), esc(&p.key));
                        square(boxes, x + px, y + py, data);
                    }
                }
            }
        }
        Role::Split | Role::Join => {
            let whole = |bits: &[Bit]| format!("[{}:0]", bits.len().saturating_sub(1));
            for (list, input) in [(&n.inputs, true), (&n.outputs, false)] {
                for p in list {
                    let Some((_, py)) = n.port_position(&p.key, input) else {
                        continue;
                    };
                    let combined = (n.role == Role::Split) == input;
                    let part = if combined {
                        whole(&p.bits)
                    } else {
                        verilog_part(&p.key)
                    };
                    // Beside the bar, outside it.
                    let px = if input { -3.0 } else { 5.0 + 3.0 };
                    let data = format!(
                        r#" data-sj="{}" data-part="{part}" data-side="{}""#,
                        esc(&n.id),
                        if input { "in" } else { "out" }
                    );
                    square(boxes, x + px, y + py + 0.5, data);
                }
            }
        }
        _ => {}
    }
}

/// A part of a bus in Verilog's notation, from netlistsvg's "lo:hi" or
/// "i": "[hi:lo]" or "[i]".
pub fn verilog_part(key: &str) -> String {
    match key.split_once(':') {
        Some((lo, hi)) => format!("[{hi}:{lo}]"),
        None => format!("[{key}]"),
    }
}

/// A fixed value, shown as netlistsvg does: hex when long.
fn constant_text(value: &str) -> String {
    if value.len() > 3 && value.chars().all(|c| c == '0' || c == '1') {
        let tail = &value[value.len().saturating_sub(128)..];
        match u128::from_str_radix(tail, 2) {
            Ok(n) => format!("0x{n:x}"),
            Err(_) => value.to_string(),
        }
    } else {
        value.to_string()
    }
}

fn dedup(list: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    list.into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// The bits of `signals` an index string names: "3" or "2:5".
fn slice(signals: &[Bit], indices: &str) -> Vec<Bit> {
    match indices.split_once(':') {
        None => indices
            .parse::<usize>()
            .ok()
            .and_then(|i| signals.get(i).cloned())
            .into_iter()
            .collect(),
        Some((a, b)) => {
            let (a, b) = (
                a.parse::<usize>().unwrap_or(0),
                b.parse::<usize>().unwrap_or(0),
            );
            signals
                .get(a..=b.min(signals.len().saturating_sub(1)))
                .map(<[Bit]>::to_vec)
                .unwrap_or_default()
        }
    }
}

/// The index string netlistsvg labels a part with: where `query` is in
/// `bits`, as "i" or "lo:hi".
fn indices(bits: &str, query: &str, start: usize) -> String {
    let at = bits.find(query).map_or(start, |i| i.max(start));
    let first = bits[..at].split(',').count() - 1;
    let last = first + query.split(',').count() - 3;
    if first == last {
        first.to_string()
    } else {
        format!("{first}:{last}")
    }
}

fn add(map: &mut IndexMap<String, Vec<String>>, key: &str, value: String) {
    map.entry(key.to_string()).or_default().push(value);
}

/// netlistsvg's `gather`: finds how the input `to_solve` is made from the
/// outputs (`inputs` here, as in netlistsvg, the signals available to
/// connect from), recording splits of outputs and joins into inputs. A loop
/// where netlistsvg recurses at the end, so wide buses can't overflow the
/// stack.
fn gather(
    inputs: &mut Vec<String>,
    outputs: &mut Vec<String>,
    to_solve: &str,
    mut start: usize,
    mut end: usize,
    splits: &mut IndexMap<String, Vec<String>>,
    joins: &mut IndexMap<String, Vec<String>>,
) {
    loop {
        if let Some(i) = outputs.iter().position(|o| o == to_solve) {
            outputs.remove(i);
        }
        if start >= to_solve.len() || end < start + 2 {
            return;
        }
        let query = &to_solve[start..end];
        if inputs.iter().any(|s| s == query) {
            if query != to_solve {
                add(joins, to_solve, indices(to_solve, query, start));
            }
            start = end - 1;
            end = to_solve.len();
            continue;
        }
        if let Some(index) = inputs.iter().position(|s| s.contains(query)) {
            if query != to_solve {
                add(joins, to_solve, indices(to_solve, query, start));
            }
            let part = indices(&inputs[index], query, 0);
            let from = inputs[index].clone();
            add(splits, &from, part);
            inputs.push(query.to_string());
            start = end - 1;
            end = to_solve.len();
            continue;
        }
        if outputs.iter().any(|s| s.contains(query)) {
            if query != to_solve {
                add(joins, to_solve, indices(to_solve, query, start));
            }
            let query = query.to_string();
            gather(inputs, &mut vec![], &query, 0, query.len(), splits, joins);
            inputs.push(query);
            return;
        }
        end = start + query[..query.len() - 1].rfind(',').map_or(0, |i| i + 1);
    }
}

/// netlistsvg's removeDummyEdges: each meeting point's edges end at the
/// bend nearest to it instead, so they join there.
fn remove_dummy_edges(edges: &mut [Value], meets: &[Option<(usize, bool)>]) {
    let mut n = 0;
    loop {
        let group: Vec<usize> = meets
            .iter()
            .enumerate()
            .filter(|(i, m)| m.is_some_and(|(d, _)| d == n) && !edges[*i].is_null())
            .map(|(i, _)| i)
            .collect();
        if group.is_empty() {
            if meets.iter().flatten().any(|(d, _)| *d > n) {
                n += 1;
                continue;
            }
            return;
        }
        let is_source = meets[group[0]].is_some_and(|(_, s)| s);
        let first = &edges[group[0]]["sections"][0];
        let at = point(if is_source {
            &first["startPoint"]
        } else {
            &first["endPoint"]
        });
        let near = group
            .iter()
            .filter_map(|&i| {
                let bends = edges[i]["sections"][0]["bendPoints"].as_array()?;
                let b = if is_source {
                    bends.first()
                } else {
                    bends.last()
                }?;
                Some(point(b))
            })
            .min_by(|a, b| {
                let d = |p: &(f64, f64)| (at.0 - p.0).abs() + (at.1 - p.1).abs();
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(at);
        let new_end = json!({"x": near.0, "y": near.1});
        let mut directions = HashSet::new();
        for &i in &group {
            let s = &mut edges[i]["sections"][0];
            if is_source {
                s["startPoint"] = new_end.clone();
                if let Some(b) = s["bendPoints"].as_array_mut()
                    && !b.is_empty()
                {
                    b.remove(0);
                }
            } else {
                s["endPoint"] = new_end.clone();
                if let Some(b) = s["bendPoints"].as_array_mut() {
                    b.pop();
                }
            }
            let next = match s["bendPoints"].as_array().filter(|b| !b.is_empty()) {
                Some(b) => point(if is_source { &b[0] } else { &b[b.len() - 1] }),
                None => point(if is_source {
                    &s["endPoint"]
                } else {
                    &s["startPoint"]
                }),
            };
            directions.insert(if next.0 > near.0 {
                0
            } else if next.0 < near.0 {
                1
            } else if next.1 > near.1 {
                2
            } else {
                3
            });
        }
        if directions.len() < 3 {
            for &i in &group {
                if let Some(j) = edges[i]["junctionPoints"].as_array_mut() {
                    j.retain(|p| point(p) != near);
                }
            }
        }
        n += 1;
    }
}

/// A title's height, and its width at 6 units per character (10-unit
/// monospace text).
const TITLE_HEIGHT: f64 = 11.0;
fn text_width(t: &str) -> f64 {
    t.chars().count() as f64 * 6.0
}

/// A coordinate, rounded to hundredths.
pub fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == 0.0 { "0".into() } else { format!("{r}") }
}

/// Text for an SVG or HTML attribute or element.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets(v: &[u64]) -> Vec<Bit> {
        v.iter().map(|n| Bit::Net(*n)).collect()
    }

    fn node(id: &str, ins: &[(&str, &[u64])], outs: &[(&str, &[u64])]) -> Node {
        let mut n = Node::new(id.into(), Shape::Generic, Role::Block(id.into()));
        n.inputs = ins.iter().map(|(k, b)| Port::new(k, nets(b))).collect();
        n.outputs = outs.iter().map(|(k, b)| Port::new(k, nets(b))).collect();
        n
    }

    #[test]
    fn a_part_of_a_bus_is_split_out() {
        // A 4-bit counter whose bit 3 drives an LED.
        let mut s = Schematic {
            nodes: vec![
                node("reg", &[], &[("Q", &[2, 3, 4, 5])]),
                node("add", &[("A", &[2, 3, 4, 5])], &[]),
                node("led", &[("A", &[5])], &[]),
            ],
            next_bit: 6,
        };
        s.add_splits_joins();
        let split = s.nodes.iter().find(|n| n.role == Role::Split).unwrap();
        assert_eq!(split.inputs[0].bits, nets(&[2, 3, 4, 5]));
        assert_eq!(split.outputs.len(), 1);
        assert_eq!(split.outputs[0].key, "3");
        assert_eq!(verilog_part(&split.outputs[0].key), "[3]");
        assert!(!s.nodes.iter().any(|n| n.role == Role::Join));
    }

    #[test]
    fn parts_are_joined_into_a_bus() {
        let mut s = Schematic {
            nodes: vec![
                node("a", &[], &[("Y", &[2, 3])]),
                node("b", &[], &[("Y", &[4])]),
                node("c", &[("A", &[2, 3, 4])], &[]),
            ],
            next_bit: 5,
        };
        s.add_splits_joins();
        let join = s.nodes.iter().find(|n| n.role == Role::Join).unwrap();
        let keys: Vec<&str> = join.inputs.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["0:1", "2"]);
        assert_eq!(verilog_part("0:1"), "[1:0]");
        assert_eq!(join.outputs[0].bits, nets(&[2, 3, 4]));
    }

    #[test]
    fn wide_buses_dont_overflow() {
        let wide: Vec<u64> = (2..2002).collect();
        let mut s = Schematic {
            nodes: vec![
                node("a", &[], &[("Y", &wide)]),
                node("b", &[("A", &wide[..1000])], &[]),
            ],
            next_bit: 3000,
        };
        s.add_splits_joins();
        assert!(s.nodes.iter().any(|n| n.role == Role::Split));
    }

    #[test]
    fn fixed_values_become_shared_constants() {
        let mut s = Schematic {
            nodes: vec![node("a", &[], &[]), node("b", &[], &[])],
            next_bit: 10,
        };
        for n in &mut s.nodes {
            n.inputs = vec![Port::new(
                "A",
                vec![Bit::Net(3), Bit::Const("1".into()), Bit::Const("0".into())],
            )];
        }
        s.add_constants();
        let consts: Vec<&Node> = s
            .nodes
            .iter()
            .filter(|n| matches!(n.role, Role::Constant(_)))
            .collect();
        assert_eq!(consts.len(), 1);
        assert_eq!(consts[0].title.as_deref(), Some("01"));
        assert_eq!(s.nodes[0].inputs[0].bits, s.nodes[1].inputs[0].bits);
        assert_eq!(s.nodes[0].inputs[0].bits[0], Bit::Net(3));
        assert_eq!(constant_text("0000000011111111"), "0xff");
    }

    #[test]
    fn generic_blocks_grow_with_their_ports() {
        let n = node(
            "m",
            &[("a", &[1]), ("b", &[2]), ("c", &[3])],
            &[("y", &[4])],
        );
        assert_eq!(n.size(), (30.0, 60.0));
        assert_eq!(n.port_position("c", true), Some((0.0, 50.0)));
        assert_eq!(n.port_position("y", false), Some((30.0, 10.0)));
        assert_eq!(num(12.3456), "12.35");
        assert_eq!(num(-0.0), "0");
        assert_eq!(esc("a<\"b\">&"), "a&lt;&quot;b&quot;&gt;&amp;");
    }
}
