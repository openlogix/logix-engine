//! The read-only catalog of boards and FPGAs (docs/tool-api.md). It is data,
//! not code: catalog/boards.json and catalog/fpgas.json, imported from Apio's
//! definitions by scripts/import-apio-catalog.py and built into the engine.
//! The catalog format is part of the tool API.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The programmers the engine can run, by catalog ID.
pub const PROGRAMMERS: &[&str] = &["openfpgaloader", "iceprog"];

/// The variables a programming variant's argument template may use.
pub const TEMPLATE_VARIABLES: &[&str] = &["VID", "PID", "BUS", "DEV", "SERIAL", "BITSTREAM"];

/// A board: its FPGA and how to program it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardDefinition {
    /// The board's name for people, such as "Alhambra II".
    pub description: String,
    /// The board's FPGA, as a catalog ID of the fpgas section, such as
    /// ice40hx4k-tq144-8k.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub fpga_id: String,
    /// The name of the programming variant used by default, such as flash.
    pub default_variant: String,
    /// The ways to program the board, by variant name.
    pub variants: BTreeMap<String, ProgrammingVariant>,
}

/// One way to program a board, such as to flash with a particular cable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgrammingVariant {
    /// The programmer to run: openfpgaloader or iceprog.
    pub programmer: String,
    /// The programmer's arguments, as a template. The engine replaces
    /// ${VID}, ${PID}, ${BUS} and ${DEV} with the device's USB vendor ID,
    /// product ID, bus and device number, ${SERIAL} with its serial number,
    /// and ${BITSTREAM} with the bitstream path.
    pub args: String,
    /// The USB filter that finds the board's device.
    pub usb: UsbFilter,
}

/// Which USB devices are this board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsbFilter {
    /// USB vendor ID, four lowercase hex digits, such as 0403.
    #[schemars(pattern(r"^[0-9a-f]{4}$"))]
    pub vid: String,
    /// USB product ID, four lowercase hex digits, such as 6010.
    #[schemars(pattern(r"^[0-9a-f]{4}$"))]
    pub pid: String,
    /// A regular expression the device's product name must match, when
    /// several boards share the same USB IDs. Omitted means any product name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_regex: Option<String>,
}

/// An FPGA: its architecture, part and toolchain parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FpgaDefinition {
    /// The FPGA family: ice40.
    pub architecture: Architecture,
    /// The vendor's part number, such as ICE40HX4K-TQ144.
    pub part_number: String,
    /// The device size, such as 8k.
    pub size: String,
    /// The device type nextpnr uses, such as hx8k.
    pub nextpnr_device: String,
    /// The package nextpnr uses, such as tq144:4k.
    pub nextpnr_package: String,
}

/// FPGA families. Milestone 1 supports iCE40 only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    /// Lattice iCE40.
    Ice40,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardsFile {
    #[allow(dead_code)]
    source: String,
    boards: BTreeMap<String, BoardDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FpgasFile {
    #[allow(dead_code)]
    source: String,
    fpgas: BTreeMap<String, FpgaDefinition>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub boards: BTreeMap<String, BoardDefinition>,
    pub fpgas: BTreeMap<String, FpgaDefinition>,
}

/// Lowercase letters, digits and hyphens.
pub fn is_catalog_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn is_usb_id(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks an argument template: only known ${...} variables, and
/// ${BITSTREAM} present.
pub fn check_template(args: &str) -> Result<(), String> {
    let mut rest = args;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed ${{ in {args:?}"))?;
        let name = &after[..end];
        if !TEMPLATE_VARIABLES.contains(&name) {
            return Err(format!(
                "unknown variable ${{{name}}} in {args:?}. Known: {}",
                TEMPLATE_VARIABLES
                    .iter()
                    .map(|v| format!("${{{v}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        rest = &after[end + 1..];
    }
    if !args.contains("${BITSTREAM}") {
        return Err(format!("{args:?} doesn't use ${{BITSTREAM}}"));
    }
    Ok(())
}

impl BoardDefinition {
    /// Checks a board definition, apart from its FPGA reference.
    pub fn validate(&self) -> Result<(), String> {
        if !is_catalog_id(&self.fpga_id) {
            return Err(format!("bad fpga_id {:?}", self.fpga_id));
        }
        if !self.variants.contains_key(&self.default_variant) {
            return Err(format!(
                "default_variant {:?} isn't one of its variants",
                self.default_variant
            ));
        }
        for (name, v) in &self.variants {
            let ctx = |e: String| format!("variant {name}: {e}");
            if !PROGRAMMERS.contains(&v.programmer.as_str()) {
                return Err(ctx(format!(
                    "unknown programmer {:?}. Known: {}",
                    v.programmer,
                    PROGRAMMERS.join(", ")
                )));
            }
            check_template(&v.args).map_err(ctx)?;
            if !is_usb_id(&v.usb.vid) || !is_usb_id(&v.usb.pid) {
                return Err(ctx("USB IDs must be four lowercase hex digits".into()));
            }
            if let Some(re) = &v.usb.product_regex {
                regex::Regex::new(re).map_err(|e| ctx(format!("bad product_regex: {e}")))?;
            }
        }
        Ok(())
    }
}

impl Catalog {
    pub fn parse(boards_json: &str, fpgas_json: &str) -> Result<Catalog, String> {
        let boards: BoardsFile =
            serde_json::from_str(boards_json).map_err(|e| format!("boards: {e}"))?;
        let fpgas: FpgasFile =
            serde_json::from_str(fpgas_json).map_err(|e| format!("fpgas: {e}"))?;
        let catalog = Catalog {
            boards: boards.boards,
            fpgas: fpgas.fpgas,
        };
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), String> {
        for id in self.fpgas.keys() {
            if !is_catalog_id(id) {
                return Err(format!("bad FPGA ID {id:?}"));
            }
        }
        for (id, b) in &self.boards {
            if !is_catalog_id(id) {
                return Err(format!("bad board ID {id:?}"));
            }
            b.validate().map_err(|e| format!("board {id}: {e}"))?;
            if !self.fpgas.contains_key(&b.fpga_id) {
                return Err(format!("board {id}: unknown FPGA {}", b.fpga_id));
            }
        }
        Ok(())
    }
}

/// The catalog built into this engine release.
pub fn builtin() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::parse(
            include_str!("../catalog/boards.json"),
            include_str!("../catalog/fpgas.json"),
        )
        .expect("the built-in catalog is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_has_the_milestone_1_boards() {
        let c = builtin();
        assert_eq!(c.fpgas.len(), 40);
        assert_eq!(c.boards.len(), 22);
        let alhambra = &c.boards["alhambra-ii"];
        assert_eq!(alhambra.fpga_id, "ice40hx4k-tq144-8k");
        let v = &alhambra.variants[&alhambra.default_variant];
        assert_eq!(v.programmer, "openfpgaloader");
        assert_eq!(v.usb.vid, "0403");
        assert_eq!(v.usb.pid, "6010");
        assert_eq!(c.fpgas["ice40hx4k-tq144-8k"].nextpnr_package, "tq144:4k");
        assert!(c.boards.values().all(|b| {
            b.variants
                .values()
                .all(|v| v.programmer == "openfpgaloader" || v.programmer == "iceprog")
        }));
    }

    #[test]
    fn templates() {
        assert!(check_template("-d d:${BUS}/${DEV} ${BITSTREAM}").is_ok());
        assert!(check_template("--vid ${VID}").is_err());
        assert!(check_template("${BIN_FILE}").is_err());
        assert!(check_template("${BITSTREAM").is_err());
    }

    #[test]
    fn bad_boards_are_refused() {
        let ok = builtin().boards["alhambra-ii"].clone();
        let mut b = ok.clone();
        b.default_variant = "sram".into();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        b.variants.get_mut("flash").unwrap().programmer = "iceburn".into();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        b.variants.get_mut("flash").unwrap().usb.vid = "0403A".into();
        assert!(b.validate().is_err());
        let mut b = ok;
        b.variants.get_mut("flash").unwrap().usb.product_regex = Some("(".into());
        assert!(b.validate().is_err());
    }
}
