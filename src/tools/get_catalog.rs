//! The get_catalog tool (docs/tool-api.md, Tables 14 and 15).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board::KnownBoard;
use crate::catalog::{self, Dimension, FpgaDefinition, Selector, Variant};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct GetCatalog;

impl EngineTool for GetCatalog {
    const NAME: &'static str = "get_catalog";
    const DESCRIPTION: &'static str = "Returns board and FPGA definitions from the engine's \
        read-only catalog. Use it to find a board's catalog ID, such as icesugar-1-5, to pass as \
        the board of other tools, or to see a board's FPGA and how it is programmed. Each \
        section is returned only when asked for: {\"boards\": {\"fetch\": true}} returns all \
        boards, and {\"boards\": {\"fetch\": true, \"ids\": [\"icesugar-1-5\"]}} only the listed \
        ones. Boards include those of the request's custom_boards_folder, marked custom, a custom board \
        replacing the catalog board with its ID. The families section gives each FPGA family's name for people, the \
        extension of its pin constraint files and the toolchain that builds it. Board descriptions are the boards' names for \
        people.";
    const READ_ONLY: bool = true;
    type Input = GetCatalogInput;
    type Output = GetCatalogOutput;
}

/// Which catalog entries to return.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCatalogInput {
    /// Which boards to return. Omitted means none.
    #[serde(default)]
    pub boards: Option<CatalogQuery>,
    /// Which FPGAs to return. Omitted means none.
    #[serde(default)]
    pub fpgas: Option<CatalogQuery>,
    /// Which FPGA families to return, by ID, such as ice40 or gw1n.
    /// Omitted means none.
    #[serde(default)]
    pub families: Option<CatalogQuery>,
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
}

/// An FPGA family, as FPGA definitions name it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FamilyInfo {
    /// The family's name for people, with its vendor, such as "Lattice
    /// iCE40", for grouping boards.
    pub label: String,
    /// The family's short name, such as "iCE40", for messages.
    pub short_label: String,
    /// The extension of the family's pin constraint files, without the
    /// dot, such as pcf: the build takes only this format for the family's
    /// boards.
    pub constraint_extension: String,
    /// The family's place in the engine's order (0 first), for listing
    /// families in a fixed order.
    pub order: u32,
    /// The toolchain that builds the family's FPGAs: ice40, ecp5, gowin or
    /// openxc7. For logs and diagnostics; show people the family.
    pub toolchain: catalog::Toolchain,
}

/// Selects entries from one catalog section.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogQuery {
    /// Whether to return this section. Defaults to false.
    #[serde(default)]
    pub fetch: bool,
    /// Catalog IDs of the entries to return. Omitted means all entries. An
    /// ID that isn't in the section is an error.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
}

/// A catalog board, as get_catalog returns it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogBoard {
    /// The board's name for people, such as "iCESugar v1.5".
    pub name: String,
    /// The board's FPGA, as a catalog ID of the fpgas section, such as
    /// ice40hx4k-tq144-8k. Every variant uses it. Boards with the same
    /// fpga_id have the same FPGA.
    pub fpga_id: String,
    /// The standard dimensions, connection then memory, each with the values
    /// the board's variants use, with labels and help from the engine, for
    /// showing the choices.
    pub dimensions: Vec<Dimension>,
    /// The ways the board can be programmed, the board's default first.
    pub variants: Vec<CatalogVariant>,
    /// True for a board from the request's custom_boards_folder, omitted for
    /// catalog boards.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
    /// What every design for the board must do, such as driving a pin
    /// that would otherwise reset the FPGA ({"type": "drive", "name":
    /// "wifi_gpio0", "pins": ["L2"], "level": "high"}), or a setup a person
    /// must do. Omitted when nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: crate::resources::Requires,
    /// What the board wires to FPGA pins, by port name, each of a type with
    /// its own fields: clock (pin, mhz), leds, buttons and switches (pins,
    /// active), rgb_led (red, green, blue, active), led_matrix (leds,
    /// colors, active), uart (tx, rx), pmod (its 8 signal pins), header,
    /// and unclassified (every wired pin no other resource lists). A name
    /// with one pin is a plain port, with several a vector, bit 0 first.
    /// The order of resources means nothing. A resource's requires lists
    /// what must be done for it to work, such as a jumper a clock needs (a
    /// setup). For creating designs and pin files for the board, such as a
    /// blinky. Omitted when none are known; a type a board doesn't list is unknown,
    /// unless the board has an unclassified resource.
    #[serde(default, skip_serializing_if = "indexmap::IndexMap::is_empty")]
    pub resources: catalog::Resources,
    /// Hints for AI agents, and people, writing designs for the board: only
    /// what its other fields can't express. Resources may have their own
    /// ai_hints. Advice to weigh, not instructions. Omitted when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_hints: Option<crate::resources::AiHints>,
}

/// One way to program a catalog board. Which USB devices are the board is
/// the engine's own matter: list_devices gives each device the boards it
/// may be.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogVariant {
    /// The variant's value of each standard dimension, such as
    /// {"connection": "usb", "memory": "flash"}: what board_variant gives in
    /// requests to pick it. No two variants of a board have the same.
    pub selector: Selector,
    /// The variant for people: its connection and memory labels, such as
    /// "USB · Flash" or "FT232H adapter · RAM".
    pub label: String,
    /// The programmer the engine runs: openfpgaloader, iceprog, dfu-util or
    /// icesprog.
    pub programmer: String,
    /// The programmer's arguments, as a template (see the variant fields of
    /// board definitions).
    pub args: String,
    /// Whether programming this way is known to work. False for ways that
    /// haven't been tried on hardware.
    pub tested: bool,
    /// Warnings and missing information for users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_notes: Option<String>,
}

impl From<&Variant> for CatalogVariant {
    fn from(v: &Variant) -> Self {
        CatalogVariant {
            selector: v.selector.clone(),
            label: v.label(),
            programmer: v.programmer.clone(),
            args: v.args.clone(),
            tested: v.tested,
            user_notes: v.user_notes.clone(),
        }
    }
}

impl From<&KnownBoard<'_>> for CatalogBoard {
    fn from(b: &KnownBoard<'_>) -> Self {
        CatalogBoard {
            name: b.description.to_string(),
            fpga_id: b.fpga_id.to_string(),
            dimensions: catalog::derive_dimensions(b.variants),
            variants: b.variants.iter().map(Into::into).collect(),
            custom: b.custom,
            requires: b.requires.clone(),
            resources: b.resources.clone(),
            ai_hints: b.ai_hints.cloned(),
        }
    }
}

/// The requested catalog entries. Sections that weren't fetched are empty.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCatalogOutput {
    /// Boards, by catalog ID.
    pub boards: BTreeMap<String, CatalogBoard>,
    /// FPGA definitions, by catalog ID.
    pub fpgas: BTreeMap<String, FpgaDefinition>,
    /// FPGA families, by ID, such as ice40 or gw1n.
    #[serde(default)]
    pub families: BTreeMap<String, FamilyInfo>,
}

fn select<T: Clone>(
    section: &str,
    all: &BTreeMap<String, T>,
    query: Option<CatalogQuery>,
) -> Result<BTreeMap<String, T>, ToolError> {
    let Some(query) = query.filter(|q| q.fetch) else {
        return Ok(BTreeMap::new());
    };
    let Some(ids) = query.ids else {
        return Ok(all.clone());
    };
    let mut out = BTreeMap::new();
    for id in ids {
        match all.get(&id) {
            Some(entry) => {
                out.insert(id, entry.clone());
            }
            None => {
                return Err(ToolError::request(format!(
                    "{id:?} isn't in the {section} section. Fetch the section without ids to list them all."
                )));
            }
        }
    }
    Ok(out)
}

pub fn run(_state: &EngineState, input: GetCatalogInput) -> Result<GetCatalogOutput, ToolError> {
    answer(input)
}

fn answer(input: GetCatalogInput) -> Result<GetCatalogOutput, ToolError> {
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let known: BTreeMap<String, CatalogBoard> = crate::board::known_boards(&custom)
        .iter()
        .map(|(id, b)| (id.to_string(), b.into()))
        .collect();
    Ok(GetCatalogOutput {
        boards: select("boards", &known, input.boards)?,
        fpgas: select("fpgas", &catalog::get().fpgas, input.fpgas)?,
        families: select("families", &families(), input.families)?,
    })
}

/// Every family the engine supports, by ID.
fn families() -> BTreeMap<String, FamilyInfo> {
    catalog::Family::ALL
        .iter()
        .enumerate()
        .map(|(i, a)| {
            (
                a.id().to_string(),
                FamilyInfo {
                    label: a.full_label().to_string(),
                    short_label: a.label().to_string(),
                    constraint_extension: a.constraint_extension().to_string(),
                    order: i as u32,
                    toolchain: a.toolchain(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Family, Toolchain};
    use serde_json::json;

    #[test]
    fn custom_boards_join_the_boards() {
        let ask = |v: serde_json::Value| {
            let input: GetCatalogInput = serde_json::from_value(v).unwrap();
            answer(input)
        };
        let mine = json!({
            "name": "My board",
            "fpga_id": "ice40hx1k-tq144",
            "variants": [{"selector": {"connection": "usb", "memory": "flash"},
                "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010"}, "tested": false}]
        });
        let mut over = mine.clone();
        over["name"] = json!("My Alhambra");
        let (_tmp, file) =
            crate::custom_boards::test_folder(&json!({"mine": mine, "alhambra-ii": over}));
        let out = ask(json!({
            "boards": {"fetch": true, "ids": ["mine", "alhambra-ii", "icestick"]},
            "custom_boards_folder": file
        }))
        .unwrap();
        let b = &out.boards["mine"];
        assert!(b.custom);
        assert_eq!(b.fpga_id, "ice40hx1k-tq144");
        assert_eq!(b.dimensions.len(), 2);
        // Replaces the catalog board with its ID.
        assert!(out.boards["alhambra-ii"].custom);
        assert_eq!(out.boards["alhambra-ii"].name, "My Alhambra");
        let json = serde_json::to_value(&out).unwrap();
        assert!(json["boards"]["icestick"].get("custom").is_none());
        assert!(json["boards"]["icestick"].get("fpga").is_none());
        assert!(json["boards"]["mine"]["variants"][0].get("usb").is_none());
        // A bad board is a request error, with a diagnostic at its line.
        let (_tmp2, bad) =
            crate::custom_boards::test_folder(&json!({"x": {"name": "x", "variants": []}}));
        let e = ask(json!({"custom_boards_folder": bad})).unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
        // The board is the whole file: a missing field is at its start.
        assert_eq!(e.diagnostics[0].line, Some(1), "{:?}", e.diagnostics);
        assert!(
            e.diagnostics[0]
                .file
                .as_deref()
                .unwrap()
                .replace('\\', "/")
                .ends_with("/boards/x.json")
        );
    }

    #[test]
    fn families_have_labels_pin_file_extensions_and_toolchains() {
        let all = families();
        let q: CatalogQuery = serde_json::from_value(serde_json::json!({"fetch": true})).unwrap();
        let out = select("families", &all, Some(q)).unwrap();
        assert!(
            select("families", &all, None).unwrap().is_empty(),
            "only when asked"
        );
        let ice40 = &out["ice40"];
        assert_eq!(ice40.label, "Lattice iCE40");
        assert_eq!(ice40.short_label, "iCE40");
        assert_eq!(ice40.constraint_extension, "pcf");
        assert_eq!(ice40.order, 0);
        assert_eq!(ice40.toolchain, Toolchain::Ice40);
        assert_eq!(out["gw2a"].short_label, "GW2A");
        assert_eq!(out["gw2a"].toolchain, Toolchain::Gowin);
        assert_eq!(out["zynq7000"].label, "Xilinx Zynq-7000");
        assert_eq!(out["zynq7000"].constraint_extension, "xdc");
        assert_eq!(out["zynq7000"].toolchain, Toolchain::Openxc7);
        assert_eq!(out.len(), Family::ALL.len());
        // Each ID is the one FPGA definitions and catalog file names use.
        for a in Family::ALL {
            assert_eq!(serde_json::to_value(a).unwrap(), a.id());
            assert_eq!(
                serde_json::to_value(a.toolchain()).unwrap(),
                a.toolchain().id()
            );
        }
    }
}
