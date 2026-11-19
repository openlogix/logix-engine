//! A board's resources (Logix Catalog's docs/model.md, Resources): what the
//! board wires to FPGA pins, by port name, each of a type with its own
//! fields, such as a clock's pin and frequency or an RGB LED's three colors.
//! For creating designs and pin files for the board, such as a blinky;
//! builds never use them. Resources may share pins (an LED on a PMOD pin);
//! within one resource, pins are unique.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Family, Toolchain};

/// A board's resources: port name to resource, by type in `Resource`'s
/// order (clocks first), each type in the board file's order. The order
/// means nothing to tools; it's for people reading a board, the same in
/// every frontend. The name is the port a design uses, a Verilog
/// identifier: one pin gives a plain port, several a vector, bit 0 first.
pub type Resources = IndexMap<String, Resource>;

/// Reads a board's resources and puts them in type order (`in_type_order`),
/// for `#[serde(deserialize_with)]` wherever a board is read.
pub fn deserialize_in_type_order<'de, D>(d: D) -> Result<Resources, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(in_type_order(Resources::deserialize(d)?))
}

/// The resources by type, in the order `Resource` declares its types:
/// clocks, LEDs, RGB LEDs, LED matrices, buttons, switches, UARTs, PMODs,
/// other headers, unclassified pins. A stable sort: each type keeps the
/// board file's order.
pub fn in_type_order(mut resources: Resources) -> Resources {
    resources.sort_by(|_, a, _, b| a.rank().cmp(&b.rank()));
    resources
}

/// The logic level that turns something on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActiveLevel {
    High,
    Low,
}

/// An I/O standard, one common name for every FPGA family; pin file writers
/// translate it (IO_TYPE for ECP5 and Gowin, IOSTANDARD for Xilinx). iCE40
/// pin files have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IoStandard {
    #[serde(rename = "LVCMOS33")]
    Lvcmos33,
    #[serde(rename = "LVCMOS25")]
    Lvcmos25,
    #[serde(rename = "LVCMOS18")]
    Lvcmos18,
    #[serde(rename = "LVCMOS15")]
    Lvcmos15,
    #[serde(rename = "LVCMOS12")]
    Lvcmos12,
    #[serde(rename = "LVTTL")]
    Lvttl,
    #[serde(rename = "LVDS")]
    Lvds,
    #[serde(rename = "SSTL15")]
    Sstl15,
    #[serde(rename = "SSTL135")]
    Sstl135,
}

impl IoStandard {
    /// Whether pin files for the toolchain can name it: none on iCE40;
    /// ECP5 has no LVTTL.
    pub fn fits(self, toolchain: Toolchain) -> bool {
        match toolchain {
            Toolchain::Ice40 => false,
            Toolchain::Ecp5 => self != IoStandard::Lvttl,
            Toolchain::Gowin | Toolchain::Openxc7 => true,
        }
    }

    pub fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
}

/// Where facts come from: a resource's or a requirement's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// hardware (tried on the board), schematic, maker (the maker's docs,
    /// pin files or examples), litex or amaranth (imported from their board
    /// files), or user (reported, not yet checked); strongest first.
    pub kind: SourceKind,
    /// The document, file and commit, or report.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Hardware,
    Schematic,
    Maker,
    Litex,
    Amaranth,
    User,
}

/// What must be done for a board, or a resource, to work: in its
/// `requires`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Requirement {
    /// Pins a design must drive to a level, as a port of its own.
    Drive(Drive),
    /// Something a person must do to the hardware, such as closing a jumper.
    Setup(Setup),
}

/// What a board or a resource requires; empty when nothing.
pub type Requires = Vec<Requirement>;

/// Pins a design must drive to a level: a design for the board (a board's
/// requirement) or one that uses the resource (a resource's) includes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Drive {
    /// The port name, a Verilog identifier, unique on the board among
    /// resources and other drives, such as wifi_gpio0.
    pub name: String,
    /// The FPGA pins, bit 0 first.
    #[schemars(length(min = 1))]
    pub pins: Vec<String>,
    /// The level to drive the pins to: high or low.
    pub level: ActiveLevel,
    /// The pins' I/O standard, when the FPGA family's pin files name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io_standard: Option<IoStandard>,
    /// Why, for people, such as "Keeps the ESP32 from resetting the board."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Where the facts come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

/// Something a person must do to the hardware. Generated designs leave out
/// what needs a setup, so they work on the board as shipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    /// What to do, for people, such as "Short jumper JP2".
    pub text: String,
    /// Where the facts come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

/// `ai_hints`, on a board or a resource: lines of text for AI agents, and
/// people, writing designs for the board, never read by tools (Logix
/// Catalog's docs/model.md, AI hints).
pub type AiHints = Vec<String>;

/// The longest line of `ai_hints`, in characters, as Logix Catalog's schema
/// allows.
pub const MAX_AI_HINT_CHARS: usize = 90;

/// Problems in an `ai_hints` list, each as (field path under it, such as
/// "1", or "" for the list, message): the schema's rules, a non-empty list
/// of non-empty single lines of at most `MAX_AI_HINT_CHARS` characters.
pub fn ai_hints_problems(hints: Option<&AiHints>) -> Vec<(String, String)> {
    let Some(hints) = hints else {
        return Vec::new();
    };
    if hints.is_empty() {
        return vec![(
            String::new(),
            "Leave ai_hints out instead of an empty list.".into(),
        )];
    }
    let mut out = Vec::new();
    for (i, h) in hints.iter().enumerate() {
        let text = if h.trim().is_empty() {
            "An empty hint.".to_string()
        } else if h.contains(['\n', '\r']) {
            "A hint is one line: make each line its own hint.".to_string()
        } else if h.chars().count() > MAX_AI_HINT_CHARS {
            format!(
                "A hint is at most {MAX_AI_HINT_CHARS} characters, this one {}: split it.",
                h.chars().count()
            )
        } else {
            continue;
        };
        out.push((i.to_string(), text));
    }
    out
}

/// Defines a resource type's struct: its own fields, then the fields every
/// resource may have.
macro_rules! resource_type {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $($(#[$fm])* pub $field: $ty,)*
            /// The pins' I/O standard, such as LVCMOS33, when known and
            /// the FPGA family's pin files name one (not iCE40).
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub io_standard: Option<IoStandard>,
            /// Where the facts come from.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub source: Option<Source>,
            /// What must be done for it to work, such as a jumper a clock
            /// needs (a setup) or a pin to drive (a drive). Omitted when
            /// nothing.
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub requires: Requires,
            /// Warnings and missing information for users.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub user_notes: Option<String>,
            /// Hints for AI agents, and people, writing designs that use
            /// this resource: only what its other fields can't express, such
            /// as an LED on the FPGA's RGB LED driver pins. Advice to weigh,
            /// not instructions. Short lines, always last. Omitted when none.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub ai_hints: Option<AiHints>,
        }
    };
}

resource_type!(
    /// An oscillator wired to one FPGA pin.
    Clock {
        /// The FPGA pin, as the board's pin files name it, such as 35.
        pin: String,
        /// The frequency in MHz, such as 12.
        mhz: f64,
    }
);

resource_type!(
    /// One or more of a kind (LEDs, buttons, switches), a pin each, bit 0
    /// first.
    PinsActive {
        /// The FPGA pins, bit 0 first.
        #[schemars(length(min = 1))]
        pins: Vec<String>,
        /// The level that turns them on: high or low. Left out when not
        /// known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<ActiveLevel>,
    }
);

resource_type!(
    /// One RGB LED: a pin per color.
    RgbLed {
        red: String,
        green: String,
        blue: String,
        /// The level that turns a color on. Left out when not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<ActiveLevel>,
    }
);

/// A pin per color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    pub red: String,
    pub green: String,
    pub blue: String,
}

resource_type!(
    /// RGB LEDs in a matrix: an LED lights when its pin and a color's pin
    /// are both active.
    LedMatrix {
        /// A pin per LED.
        #[schemars(length(min = 1))]
        leds: Vec<String>,
        /// A pin per color, shared by the LEDs.
        colors: Colors,
        /// The level that turns them on. Left out when not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<ActiveLevel>,
    }
);

resource_type!(
    /// A serial port as the FPGA sees it: tx is the FPGA's output.
    Uart {
        tx: String,
        rx: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rts: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cts: Option<String>,
    }
);

resource_type!(
    /// A list of pins: for pmod, the 8 signal pins in header order (1 to 4,
    /// then 7 to 10); for unclassified, every FPGA pin the board wires to
    /// something no other resource describes.
    Pins {
        #[schemars(length(min = 1))]
        pins: Vec<String>,
    }
);

resource_type!(
    /// Any other header: every position in order, null for power and ground.
    Header {
        #[schemars(length(min = 1))]
        pins: Vec<Option<String>>,
    }
);

/// What a board wires to FPGA pins, by type. The variants' order is the
/// order of a board's resources (`in_type_order`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Resource {
    /// An oscillator: pin, mhz.
    Clock(Clock),
    /// LEDs: pins, active.
    Leds(PinsActive),
    /// One RGB LED: red, green, blue, active.
    RgbLed(RgbLed),
    /// RGB LEDs in a matrix: leds, colors, active.
    LedMatrix(LedMatrix),
    /// Push buttons: pins, active.
    Buttons(PinsActive),
    /// Switches: pins, active.
    Switches(PinsActive),
    /// A serial port: tx, rx, rts, cts.
    Uart(Uart),
    /// A PMOD header: its 8 signal pins.
    Pmod(Pins),
    /// Another header: every position, null for power and ground.
    Header(Header),
    /// Every wired pin no other resource describes; a board with one lists
    /// all its wired pins.
    Unclassified(Pins),
}

impl Resource {
    /// Its type's place in a board's resources: the order the enum declares.
    fn rank(&self) -> u8 {
        match self {
            Resource::Clock(_) => 0,
            Resource::Leds(_) => 1,
            Resource::RgbLed(_) => 2,
            Resource::LedMatrix(_) => 3,
            Resource::Buttons(_) => 4,
            Resource::Switches(_) => 5,
            Resource::Uart(_) => 6,
            Resource::Pmod(_) => 7,
            Resource::Header(_) => 8,
            Resource::Unclassified(_) => 9,
        }
    }

    /// Its pins as (field, pin), in order, nulls left out.
    pub fn pins(&self) -> Vec<(String, &str)> {
        fn list<'a>(name: &str, ps: &'a [String]) -> Vec<(String, &'a str)> {
            ps.iter()
                .enumerate()
                .map(|(i, p)| (format!("{name}/{i}"), p.as_str()))
                .collect()
        }
        match self {
            Resource::Clock(c) => vec![("pin".into(), c.pin.as_str())],
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p) => {
                list("pins", &p.pins)
            }
            Resource::RgbLed(r) => vec![
                ("red".into(), r.red.as_str()),
                ("green".into(), r.green.as_str()),
                ("blue".into(), r.blue.as_str()),
            ],
            Resource::LedMatrix(m) => {
                let mut out = list("leds", &m.leds);
                out.push(("colors/red".into(), m.colors.red.as_str()));
                out.push(("colors/green".into(), m.colors.green.as_str()));
                out.push(("colors/blue".into(), m.colors.blue.as_str()));
                out
            }
            Resource::Uart(u) => {
                let mut out = vec![("tx".into(), u.tx.as_str()), ("rx".into(), u.rx.as_str())];
                for (n, p) in [("rts", &u.rts), ("cts", &u.cts)] {
                    if let Some(p) = p {
                        out.push((n.into(), p.as_str()));
                    }
                }
                out
            }
            Resource::Pmod(p) | Resource::Unclassified(p) => list("pins", &p.pins),
            Resource::Header(h) => h
                .pins
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.as_deref().map(|p| (format!("pins/{i}"), p)))
                .collect(),
        }
    }

    pub fn io_standard(&self) -> Option<IoStandard> {
        match self {
            Resource::Clock(c) => c.io_standard,
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p) => p.io_standard,
            Resource::RgbLed(r) => r.io_standard,
            Resource::LedMatrix(m) => m.io_standard,
            Resource::Uart(u) => u.io_standard,
            Resource::Pmod(p) | Resource::Unclassified(p) => p.io_standard,
            Resource::Header(h) => h.io_standard,
        }
    }

    /// What must be done for it to work.
    pub fn requires(&self) -> &Requires {
        match self {
            Resource::Clock(c) => &c.requires,
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p) => &p.requires,
            Resource::RgbLed(r) => &r.requires,
            Resource::LedMatrix(m) => &m.requires,
            Resource::Uart(u) => &u.requires,
            Resource::Pmod(p) | Resource::Unclassified(p) => &p.requires,
            Resource::Header(h) => &h.requires,
        }
    }

    /// Its hints for whoever writes designs.
    pub fn ai_hints(&self) -> Option<&AiHints> {
        match self {
            Resource::Clock(c) => c.ai_hints.as_ref(),
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p) => p.ai_hints.as_ref(),
            Resource::RgbLed(r) => r.ai_hints.as_ref(),
            Resource::LedMatrix(m) => m.ai_hints.as_ref(),
            Resource::Uart(u) => u.ai_hints.as_ref(),
            Resource::Pmod(p) | Resource::Unclassified(p) => p.ai_hints.as_ref(),
            Resource::Header(h) => h.ai_hints.as_ref(),
        }
    }

    /// How many LEDs it is: a resource of LEDs counts each, an RGB LED one.
    pub fn led_count(&self) -> usize {
        match self {
            Resource::Leds(p) => p.pins.len(),
            Resource::RgbLed(_) => 1,
            Resource::LedMatrix(m) => m.leds.len(),
            _ => 0,
        }
    }

    /// A clock's frequency in MHz.
    pub fn clock_mhz(&self) -> Option<f64> {
        match self {
            Resource::Clock(c) => Some(c.mhz),
            _ => None,
        }
    }
}

/// A Verilog identifier: a letter or underscore, then letters, digits and
/// underscores.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A board's resource problems, each as (resource name, field path under it
/// such as "pins/1", or "" for the name itself, message), for the catalog's
/// checks and custom boards' diagnostics. `family` is the board's FPGA family,
/// when known.
pub fn problems(resources: &Resources, family: Option<Family>) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut names: std::collections::BTreeSet<String> = resources.keys().cloned().collect();
    for (name, r) in resources {
        let mut add = |field: &str, text: String| out.push((name.clone(), field.to_string(), text));
        if !is_identifier(name) {
            add(
                "",
                format!(
                    "{name:?} isn't a port name: use letters, digits and underscores, starting with a letter."
                ),
            );
        }
        let pins = r.pins();
        let mut seen = std::collections::BTreeSet::new();
        for (field, p) in &pins {
            if !is_pin(p) {
                add(field, format!("{p:?} isn't a pin name."));
            } else if !seen.insert(*p) {
                add(field, format!("Pin {p} is in this resource twice."));
            }
        }
        for (field, text) in requirement_problems(r.requires(), family, &mut names) {
            add(&format!("requires/{field}"), text);
        }
        match r {
            Resource::Clock(c) if !(c.mhz > 0.0 && c.mhz.is_finite()) => {
                add("mhz", format!("{} isn't a frequency in MHz.", c.mhz));
            }
            Resource::Pmod(p) if p.pins.len() != 8 => {
                add(
                    "pins",
                    "A PMOD lists its 8 signal pins: 1 to 4, then 7 to 10.".into(),
                );
            }
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p)
                if p.pins.is_empty() =>
            {
                add("pins", "At least one pin.".into());
            }
            Resource::Unclassified(p) if p.pins.is_empty() => {
                add("pins", "At least one pin.".into());
            }
            _ => {}
        }
        if let Some(text) = io_standard_problem(r.io_standard(), family) {
            add("io_standard", text);
        }
        for (field, text) in ai_hints_problems(r.ai_hints()) {
            add(format!("ai_hints/{field}").trim_end_matches('/'), text);
        }
    }
    out
}

/// A board's own `requires` problems, each as (field path under it, such
/// as "0/name", message). Drive names must differ from the resources' names
/// and their drives' names too.
pub fn board_requires_problems(
    requires: &Requires,
    resources: &Resources,
    family: Option<Family>,
) -> Vec<(String, String)> {
    let mut names: std::collections::BTreeSet<String> = resources.keys().cloned().collect();
    for r in resources.values() {
        for q in r.requires() {
            if let Requirement::Drive(d) = q {
                names.insert(d.name.clone());
            }
        }
    }
    requirement_problems(requires, family, &mut names)
}

/// Problems in a list of requirements, each as (field path under it, such
/// as "0/pins/1", message); `names` holds the port names taken so far and
/// gets each drive's.
fn requirement_problems(
    requires: &Requires,
    family: Option<Family>,
    names: &mut std::collections::BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, q) in requires.iter().enumerate() {
        let mut add = |field: &str, text: String| out.push((format!("{i}/{field}"), text));
        match q {
            Requirement::Drive(d) => {
                if !is_identifier(&d.name) {
                    add(
                        "name",
                        format!(
                            "{:?} isn't a port name: use letters, digits and underscores, starting with a letter.",
                            d.name
                        ),
                    );
                } else if !names.insert(d.name.clone()) {
                    add(
                        "name",
                        format!("Another resource or drive is named {}.", d.name),
                    );
                }
                if d.pins.is_empty() {
                    add("pins", "At least one pin.".into());
                }
                let mut seen = std::collections::BTreeSet::new();
                for (j, p) in d.pins.iter().enumerate() {
                    if !is_pin(p) {
                        add(&format!("pins/{j}"), format!("{p:?} isn't a pin name."));
                    } else if !seen.insert(p) {
                        add(
                            &format!("pins/{j}"),
                            format!("Pin {p} is in this drive twice."),
                        );
                    }
                }
                if let Some(text) = io_standard_problem(d.io_standard, family) {
                    add("io_standard", text);
                }
                if d.why.as_deref().is_some_and(|w| w.trim().is_empty()) {
                    add("why", "Say why, or leave it out.".into());
                }
            }
            Requirement::Setup(s) => {
                if s.text.trim().is_empty() {
                    add("text", "Say what to do.".into());
                }
            }
        }
    }
    out
}

/// A pin name: not empty, without spaces, quotes or semicolons.
fn is_pin(p: &str) -> bool {
    !p.is_empty() && !p.chars().any(|c| c.is_whitespace() || c == '"' || c == ';')
}

/// The problem with an I/O standard the FPGA family's pin files can't name.
fn io_standard_problem(std: Option<IoStandard>, family: Option<Family>) -> Option<String> {
    let (std, family) = (std?, family?);
    (!std.fits(family.toolchain())).then(|| {
        format!(
            "{} isn't an I/O standard for {}{}.",
            std.name(),
            family.full_label(),
            if family.toolchain() == Toolchain::Ice40 {
                ": its pin files have none"
            } else {
                ""
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_type_order_is_stable() {
        // A board file's own order: an LED, a PMOD, a clock, another LED.
        let r: Resources = serde_json::from_str(
            r#"{
                "led_a": {"type": "leds", "pins": ["1"]},
                "pmod": {"type": "pmod", "pins": ["2", "3", "4", "5", "6", "7", "8", "9"]},
                "clk": {"type": "clock", "pin": "10", "mhz": 12},
                "led_b": {"type": "leds", "pins": ["11"]}
            }"#,
        )
        .unwrap();
        let names: Vec<_> = in_type_order(r).into_keys().collect();
        assert_eq!(names, ["clk", "led_a", "led_b", "pmod"]);
    }

    fn res(v: serde_json::Value) -> Resources {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn types_read_and_write_as_in_the_catalog() {
        let text = serde_json::json!({
            "clk": {"type": "clock", "pin": "35", "mhz": 12.5},
            "leds": {"type": "leds", "pins": ["11", "37"], "active": "low"},
            "rgb": {"type": "rgb_led", "red": "41", "green": "39", "blue": "40"},
            "m": {"type": "led_matrix", "leds": ["C13"], "colors": {"red": "T1", "green": "U1", "blue": "R1"},
                  "io_standard": "LVCMOS33", "source": {"kind": "schematic", "ref": "sheet 5"}},
            "uart": {"type": "uart", "tx": "9", "rx": "6"},
            "p": {"type": "pmod", "pins": ["1", "2", "3", "4", "5", "6", "7", "8"]},
            "h": {"type": "header", "pins": ["1", null]},
            "osc": {"type": "clock", "pin": "20", "mhz": 12.5, "requires": [
                {"type": "setup", "text": "Short jumper R16.", "source": {"kind": "maker"}},
                {"type": "drive", "name": "osc_en", "pins": ["21"], "level": "high", "why": "Turns the oscillator on."}
            ]}
        });
        let r = res(text.clone());
        assert_eq!(serde_json::to_value(&r).unwrap(), text);
        // The file's order is kept.
        assert_eq!(r.keys().next().map(String::as_str), Some("clk"));
        assert_eq!(
            r["m"].led_count() + r["leds"].led_count() + r["rgb"].led_count(),
            4
        );
        assert_eq!(r["clk"].clock_mhz(), Some(12.5));
        // Unknown types and fields are errors.
        for bad in [
            serde_json::json!({"b": {"type": "lamp", "pins": ["1"]}}),
            serde_json::json!({"b": {"type": "leds", "pins": ["1"], "color": "red"}}),
            serde_json::json!({"b": {"type": "clock", "pin": "1"}}),
            serde_json::json!({"b": {"type": "clock", "pin": "1", "mhz": 1, "requires": [{"type": "wire"}]}}),
            serde_json::json!({"b": {"type": "clock", "pin": "1", "mhz": 1, "requires": [{"type": "drive", "name": "x", "pins": ["2"]}]}}),
            serde_json::json!({"b": {"type": "clock", "pin": "1", "mhz": 1, "requires": [{"type": "setup", "text": "x", "level": "high"}]}}),
        ] {
            assert!(serde_json::from_value::<Resources>(bad).is_err());
        }
    }

    #[test]
    fn ai_hints_follow_the_schema_rules() {
        let check = |h: &[&str]| {
            let h: AiHints = h.iter().map(|s| s.to_string()).collect();
            ai_hints_problems(Some(&h))
        };
        assert!(ai_hints_problems(None).is_empty());
        assert!(check(&["A hint.", &"x".repeat(MAX_AI_HINT_CHARS)]).is_empty());
        assert_eq!(check(&[])[0].0, "");
        let got: Vec<_> = check(&["ok", " ", "two\nlines", &"é".repeat(91)])
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(got, ["1", "2", "3"]);
    }

    #[test]
    fn problems_are_found() {
        let t = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        let r = res(serde_json::json!({
            "2clk": {"type": "clock", "pin": "35", "mhz": 0},
            "leds": {"type": "leds", "pins": ["1", "1"]},
            "p": {"type": "pmod", "pins": ["1"], "io_standard": "LVCMOS33"}
        }));
        assert_eq!(
            problems(&r, Some(Family::Ice40)),
            [
                t(
                    "2clk",
                    "",
                    "\"2clk\" isn't a port name: use letters, digits and underscores, starting with a letter."
                ),
                t("2clk", "mhz", "0 isn't a frequency in MHz."),
                t("leds", "pins/1", "Pin 1 is in this resource twice."),
                t(
                    "p",
                    "pins",
                    "A PMOD lists its 8 signal pins: 1 to 4, then 7 to 10."
                ),
                t(
                    "p",
                    "io_standard",
                    "LVCMOS33 isn't an I/O standard for Lattice iCE40: its pin files have none."
                ),
            ]
        );
        // Requirements: a resource's, then the board's, names unique across
        // resources and drives.
        let r = res(serde_json::json!({
            "clk": {"type": "clock", "pin": "35", "mhz": 12, "requires": [
                {"type": "setup", "text": " "},
                {"type": "drive", "name": "en", "pins": ["2", "2", "a b"], "level": "high", "io_standard": "LVCMOS33"}
            ]},
            "led": {"type": "leds", "pins": ["1"], "requires": [
                {"type": "drive", "name": "en", "pins": ["3"], "level": "low", "why": ""}
            ]}
        }));
        assert_eq!(
            problems(&r, Some(Family::Ice40)),
            [
                t("clk", "requires/0/text", "Say what to do."),
                t("clk", "requires/1/pins/1", "Pin 2 is in this drive twice."),
                t("clk", "requires/1/pins/2", "\"a b\" isn't a pin name."),
                t(
                    "clk",
                    "requires/1/io_standard",
                    "LVCMOS33 isn't an I/O standard for Lattice iCE40: its pin files have none."
                ),
                t(
                    "led",
                    "requires/0/name",
                    "Another resource or drive is named en."
                ),
                t("led", "requires/0/why", "Say why, or leave it out."),
            ]
        );
        let board: Requires = serde_json::from_value(serde_json::json!([
            {"type": "drive", "name": "en", "pins": ["4"], "level": "high"},
            {"type": "drive", "name": "clk", "pins": ["5"], "level": "high"},
            {"type": "drive", "name": "gpio0", "pins": [], "level": "high"}
        ]))
        .unwrap();
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            board_requires_problems(&board, &r, Some(Family::Ice40)),
            [
                s("0/name", "Another resource or drive is named en."),
                s("1/name", "Another resource or drive is named clk."),
                s("2/pins", "At least one pin."),
            ]
        );
        // Resources may share pins.
        let shared = res(serde_json::json!({
            "led": {"type": "leds", "pins": ["1"]},
            "p": {"type": "pmod", "pins": ["1", "2", "3", "4", "5", "6", "7", "8"]}
        }));
        assert!(problems(&shared, Some(Family::Ice40)).is_empty());
    }
}
