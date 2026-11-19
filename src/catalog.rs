//! The read-only catalog of boards and FPGAs (docs/tool-api.md). It is data,
//! not code: Logix Catalog (github.com/openlogix/logix-catalog), one file per
//! board in its format, read at runtime from the downloaded logix-catalog
//! package (`crate::packages::CATALOG`), or from the folder
//! LOGIX_ENGINE_DEV_CATALOG_FOLDER names. Nothing is built in. The catalog
//! format is part of the tool API.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::resources::{ActiveLevel, Resource, Resources};

/// The variable naming a catalog folder (a logix-catalog clone) that the
/// engine reads instead of its downloaded package: a diagnostic setting,
/// for trying catalog changes without a release.
pub const DEV_CATALOG_VAR: &str = "LOGIX_ENGINE_DEV_CATALOG_FOLDER";

/// Where the loaded catalog comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The catalog's version: its logix-catalog release (yyyy-mm-dd), or
    /// "local" for a folder from DEV_CATALOG_VAR.
    pub version: String,
    /// The folder it was read from.
    pub folder: PathBuf,
    /// True for a folder from DEV_CATALOG_VAR.
    pub local: bool,
}

/// The loaded catalog. Each load leaks its catalog, so code can borrow it
/// for the whole run; a load happens only when a new catalog arrives.
static LOADED: RwLock<Option<(&'static Catalog, Source)>> = RwLock::new(None);

/// The loaded catalog, or an empty one before the first load. Tools that
/// need boards load it first (`crate::state::EngineState::ensure_catalog`).
/// Unit tests read the repo's catalog/ copy.
pub fn get() -> &'static Catalog {
    if let Some((c, _)) = *LOADED.read().unwrap_or_else(|e| e.into_inner()) {
        return c;
    }
    if cfg!(test) {
        use_test_copy();
        return get();
    }
    static EMPTY: OnceLock<Catalog> = OnceLock::new();
    EMPTY.get_or_init(|| Catalog {
        boards: BTreeMap::new(),
        fpgas: BTreeMap::new(),
    })
}

/// Where the loaded catalog comes from; None before the first load.
pub fn source() -> Option<Source> {
    LOADED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|(_, s)| s.clone())
}

/// Reads the catalog in `folder` (with boards/ and fpgas/) and makes it the
/// loaded one, unless the same source is loaded already.
pub fn load(folder: &Path, version: &str, local: bool) -> Result<(), String> {
    let source = Source {
        version: version.to_string(),
        folder: folder.to_path_buf(),
        local,
    };
    if self::source().as_ref() == Some(&source) {
        return Ok(());
    }
    let catalog = read_folder(folder)?;
    log::info!(
        "Board catalog {version} ({} boards) from {}",
        catalog.boards.len(),
        folder.display()
    );
    *LOADED.write().unwrap_or_else(|e| e.into_inner()) =
        Some((Box::leak(Box::new(catalog)), source));
    Ok(())
}

/// Loads the repo's catalog/ copy of Logix Catalog, the engine's test data
/// (scripts/update-catalog.sh). For tests only.
#[doc(hidden)]
pub fn use_test_copy() {
    let folder = test_copy_folder();
    load(&folder, "test", true).expect("catalog/ is a valid catalog");
}

/// The repo's catalog/ copy, for tests.
#[doc(hidden)]
pub fn test_copy_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog")
}

/// The catalog's FPGAs that a core's tools can't build, by ID, each with
/// what the core lacks: the check of an engine configuration, a catalog with
/// a core (`crate::packages::CoreFpgas`, from the core's logix-core.json).
pub fn not_in_core(catalog: &Catalog, core: &crate::packages::CoreFpgas) -> Vec<String> {
    let mut out = Vec::new();
    for (id, f) in &catalog.fpgas {
        let toolchain = f.toolchain().id();
        // The core's terms for this FPGA's build fields.
        let mut want = BTreeMap::from([("device", f.nextpnr_device.clone())]);
        match f.toolchain() {
            Toolchain::Ice40 => {
                want.insert("package", f.nextpnr_package.clone().unwrap_or_default());
            }
            Toolchain::Ecp5 => {
                want.insert("package", f.nextpnr_package.clone().unwrap_or_default());
                want.insert("speed", f.nextpnr_speed.clone().unwrap_or_default());
            }
            Toolchain::Gowin => {
                want.insert("packer_device", f.packer_device.clone().unwrap_or_default());
            }
            Toolchain::Openxc7 => {}
        }
        let found = core.get(toolchain).is_some_and(|parts| {
            parts.iter().any(|p| {
                want.iter()
                    .all(|(k, v)| p.get(*k).map(String::as_str) == Some(v.as_str()))
            })
        });
        if !found {
            let terms: Vec<String> = want.iter().map(|(k, v)| format!("{k} {v}")).collect();
            out.push(format!("{id} ({toolchain}: {})", terms.join(", ")));
        }
    }
    out
}

/// The `.json` files in `folder`, as (name without .json, text), by name.
pub fn json_files(folder: &Path) -> Result<Vec<(String, String)>, String> {
    let entries =
        std::fs::read_dir(folder).map_err(|e| format!("can't read {}: {e}", folder.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == "json")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("can't read {}: {e}", path.display()))?;
            out.push((stem.to_string(), text));
        }
    }
    out.sort();
    Ok(out)
}

/// The catalog in `folder`: its boards/ and fpgas/ files.
pub fn read_folder(folder: &Path) -> Result<Catalog, String> {
    let boards = json_files(&folder.join("boards"))?;
    let fpgas = json_files(&folder.join("fpgas"))?;
    fn pairs(v: &[(String, String)]) -> Vec<(&str, &str)> {
        v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
    }
    Catalog::parse(&pairs(&boards), &pairs(&fpgas))
        .map_err(|e| format!("the board catalog in {} is invalid: {e}", folder.display()))
}

/// The programmers the engine can run, by catalog ID.
pub const PROGRAMMERS: &[&str] = &["openfpgaloader", "iceprog", "dfu-util", "icesprog"];

/// The variables a programming variant's argument template may use.
pub const TEMPLATE_VARIABLES: &[&str] = &[
    "VID",
    "PID",
    "BUS",
    "DEV",
    "SERIAL_NUM",
    "USB_PATH",
    "BITSTREAM",
];

/// A board: its FPGA and its variants. A board is a name: two boards with
/// different names are unrelated, even when their hardware is identical, as
/// are the sizes of a board sold with several FPGA sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardDefinition {
    /// The board's name for people, such as "iCESugar v1.5".
    pub name: String,
    /// The board's FPGA, as a catalog ID of the fpgas section, such as
    /// ice40hx4k-tq144-8k. Every variant uses it.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub fpga_id: String,
    /// The standard dimensions, connection then memory, each with the values
    /// the board's variants use, with labels and help from the engine, for
    /// showing the choices. Derived from the variants: ignored in inputs and
    /// not written in the catalog file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    /// The ways the board can be programmed, such as through its USB port
    /// to flash, each with how to program it. The first is the board's
    /// default. Variants differ only in how the board is programmed: all
    /// have the same FPGA and pin file, so the same project works with each.
    #[schemars(length(min = 1))]
    pub variants: Vec<Variant>,
    /// What every design for the board must do, such as driving a pin
    /// that would otherwise reset the FPGA ({"type": "drive", "name":
    /// "wifi_gpio0", "pins": ["L2"], "level": "high"}), or a setup a person
    /// must do. Omitted when nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: crate::resources::Requires,
    /// What the board wires to FPGA pins, by port name, each of a type with
    /// its own fields, such as {"clk": {"type": "clock", "pin": "35",
    /// "mhz": 12}}: for creating designs and pin files for the board, such
    /// as a blinky. Facts only: builds never use them. Boards list what is
    /// known; a type a board doesn't list is unknown, unless the board has an
    /// unclassified resource, which lists every wired pin.
    #[serde(
        default,
        deserialize_with = "crate::resources::deserialize_in_type_order",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub resources: Resources,
    /// Hints for AI agents, and people, writing designs for the board: only
    /// what its other fields can't express. Advice to weigh, not
    /// instructions. Short lines, always last. Omitted when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_hints: Option<crate::resources::AiHints>,
}

/// One kind of choice, such as the memory to program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dimension {
    /// The dimension's ID, used in variants, such as
    /// connection or memory.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub id: String,
    /// The dimension's name for people, such as "Connection".
    pub label: String,
    /// The possible values, in the order to show them.
    #[schemars(length(min = 1))]
    pub values: Vec<DimensionValue>,
}

/// One value of a dimension, such as RAM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionValue {
    /// The value's ID, such as ram.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub id: String,
    /// The value's name for people, such as "RAM".
    pub label: String,
    /// A short explanation for people, such as "Faster to upload; lost when
    /// the board is powered off."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// Which of a board's variants: a value of each standard dimension. In a
/// board definition it says where the variant sits; in requests
/// (board_variant) it picks the variant with these values.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    /// How the computer reaches the board: usb, bootloader, ft2232h,
    /// ft232h, cmsis-dap, usb-blaster or unspecified (see the dimensions).
    pub connection: String,
    /// Where the design goes: flash (kept at power-off) or ram (lost at
    /// power-off).
    pub memory: String,
}

impl Selector {
    pub fn new(connection: &str, memory: &str) -> Selector {
        Selector {
            connection: connection.into(),
            memory: memory.into(),
        }
    }

    /// The selector's value of a standard dimension, by dimension ID.
    pub fn value(&self, dimension: &str) -> Option<&str> {
        match dimension {
            "connection" => Some(&self.connection),
            "memory" => Some(&self.memory),
            _ => None,
        }
    }
}

/// A board variant in a request: some or all of a selector's values, such
/// as {"memory": "ram"}. The board's first variant with these values is
/// used, so {} means its first variant, its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantChoice {
    /// The connection, such as usb or ft232h. Omitted means any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// The memory, flash or ram. Omitted means any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

impl VariantChoice {
    /// Whether a variant with `selector` has every value of the choice.
    pub fn fits(&self, selector: &Selector) -> bool {
        self.connection
            .as_ref()
            .is_none_or(|c| *c == selector.connection)
            && self.memory.as_ref().is_none_or(|m| *m == selector.memory)
    }
}

impl From<Selector> for VariantChoice {
    fn from(s: Selector) -> Self {
        VariantChoice {
            connection: Some(s.connection),
            memory: Some(s.memory),
        }
    }
}

impl std::fmt::Display for VariantChoice {
    /// As in messages: connection usb, memory flash; only the values given.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = [("connection", &self.connection), ("memory", &self.memory)]
            .into_iter()
            .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k} {v}")))
            .collect();
        if parts.is_empty() {
            write!(f, "no values")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

impl std::fmt::Display for Selector {
    /// As in messages: connection usb, memory flash.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "connection {}, memory {}", self.connection, self.memory)
    }
}

/// One way to program a board: its selector (its connection and memory),
/// and how to program the board that way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    /// The variant's value of each standard dimension, such as
    /// {"connection": "usb", "memory": "flash"}: what board_variant gives
    /// in requests to pick it. No two variants of a board have the same.
    pub selector: Selector,
    /// The programmer to run: openfpgaloader, iceprog, dfu-util or icesprog.
    pub programmer: String,
    /// The programmer's arguments, as a template. The engine replaces
    /// ${VID}, ${PID}, ${BUS} and ${DEV} with the device's USB vendor ID,
    /// product ID, bus and device number, ${SERIAL_NUM} with its serial number,
    /// ${USB_PATH} with its USB port path (such as 1-2.3, as dfu-util's -p
    /// takes it), and ${BITSTREAM} with the bitstream path.
    pub args: String,
    /// The USB filter that finds the board's device.
    pub usb: UsbFilter,
    /// Whether programming this way is known to work: tested on the board,
    /// or used unchanged in a tool that relies on it. False for ways that
    /// haven't been tried on hardware, such as a board moved to a different
    /// programmer.
    pub tested: bool,
    /// Warnings and missing information for users, such as a USB filter that
    /// also matches other boards or a step needed before programming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_notes: Option<String>,
    /// For the board definition's maintainers: where the settings come from
    /// and why they changed. Optional. get_catalog never returns it.
    #[serde(default, skip_serializing)]
    pub maintainer_notes: Option<String>,
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
    /// A regular expression the device's serial number must match, for
    /// boards that share USB IDs and product names but have serial numbers
    /// that tell them apart. Omitted means any serial number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_regex: Option<String>,
    /// The device's release number (USB bcdDevice), four lowercase hex
    /// digits, such as 0500, that the device must report, for boards that
    /// share USB IDs but differ in it. On FTDI chips, and on board debuggers
    /// that act like them, it tells the chip type: 0700 for an FT2232H, 0500
    /// for an FT2232C/D or a debugger that acts like one. Omitted means any
    /// release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(r"^[0-9a-f]{4}$"))]
    pub release: Option<String>,
}

/// An FPGA: its family, part and toolchain parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FpgaDefinition {
    /// The FPGA family, the name on the chip: ice40, ecp5, gw1n, gw2a,
    /// gw5a, artix7, spartan7 or zynq7000.
    pub family: Family,
    /// The vendor's part number, such as ICE40HX4K-TQ144.
    pub part_number: String,
    /// The device size, such as 8k.
    pub size: String,
    /// The device nextpnr uses, such as hx8k for iCE40, 25k for ECP5,
    /// GW1NR-LV9QN88PC6/I5 for Gowin or xc7a35tcsg324-1 for Xilinx.
    pub nextpnr_device: String,
    /// The package nextpnr uses, such as tq144:4k for iCE40 or CABGA256 for
    /// ECP5. Required for iCE40 and ECP5; Gowin and Xilinx have none,
    /// since their nextpnr_device names the package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextpnr_package: Option<String>,
    /// The speed grade nextpnr uses, such as 6. ECP5 only, where it's
    /// required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextpnr_speed: Option<String>,
    /// The device family nextpnr uses, such as GW1N-9C, when nextpnr can't
    /// tell it from nextpnr_device. Gowin only, optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextpnr_family: Option<String>,
    /// The device the bitstream packer (gowin_pack) uses, such as GW1N-9C.
    /// Gowin only, where it's required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packer_device: Option<String>,
    /// The FPGA's JTAG ID code, such as 0x0362d093, as openFPGALoader's
    /// --detect reports it. Before programming over JTAG, the engine reads
    /// the connected FPGA's ID code and refuses to program a different
    /// FPGA, such as a board of another size than the build's. It
    /// identifies the chip, not the size a board uses it as, so FPGAs made
    /// from the same chip share it. ECP5, Gowin and Xilinx only, optional:
    /// without it there is no check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jtag_idcode: Option<String>,
}

impl FpgaDefinition {
    /// What the build depends on: the settings the synthesis, place and
    /// route and packing tools use. Names (part number, size) and the JTAG
    /// ID code don't change the bitstream, so changing them doesn't
    /// rebuild.
    pub fn build_key(&self) -> serde_json::Value {
        serde_json::json!({
            "family": self.family,
            "nextpnr_device": self.nextpnr_device,
            "nextpnr_package": self.nextpnr_package,
            "nextpnr_speed": self.nextpnr_speed,
            "nextpnr_family": self.nextpnr_family,
            "packer_device": self.packer_device,
        })
    }
}

/// Reads a JTAG ID code written as 0x and 8 lowercase hex digits, such as
/// 0x0362d093.
pub fn parse_idcode(code: &str) -> Option<u32> {
    let hex = code.strip_prefix("0x")?;
    if hex.len() != 8
        || !hex
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

impl FpgaDefinition {
    /// The toolchain that builds the FPGA, from its family.
    pub fn toolchain(&self) -> Toolchain {
        self.family.toolchain()
    }

    /// Checks that the FPGA has the fields its toolchain needs, and only
    /// those.
    pub fn validate(&self) -> Result<(), String> {
        let word = |name: &str, v: &str| {
            if v.is_empty()
                || !v
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b":-_/".contains(&c))
            {
                Err(format!("bad {name} {v:?}"))
            } else {
                Ok(())
            }
        };
        word("nextpnr_device", &self.nextpnr_device)?;
        let family = self.family.id();
        let fields = [
            ("nextpnr_package", &self.nextpnr_package),
            ("nextpnr_speed", &self.nextpnr_speed),
            ("nextpnr_family", &self.nextpnr_family),
            ("packer_device", &self.packer_device),
        ];
        for (name, value) in fields {
            let (allowed, required) = match (self.toolchain(), name) {
                (Toolchain::Ice40, "nextpnr_package") => (true, true),
                (Toolchain::Ecp5, "nextpnr_package" | "nextpnr_speed") => (true, true),
                (Toolchain::Gowin, "packer_device") => (true, true),
                (Toolchain::Gowin, "nextpnr_family") => (true, false),
                _ => (false, false),
            };
            match value {
                Some(v) if allowed => word(name, v)?,
                Some(_) => return Err(format!("{family} FPGAs have no {name}")),
                None if required => return Err(format!("{family} FPGAs need {name}")),
                None => {}
            }
        }
        if let Some(code) = &self.jtag_idcode {
            if self.toolchain() == Toolchain::Ice40 {
                return Err(format!("{family} FPGAs have no jtag_idcode"));
            }
            if parse_idcode(code).is_none() {
                return Err(format!(
                    "bad jtag_idcode {code:?}: write it as 0x and 8 lowercase hex digits"
                ));
            }
        }
        // It names a folder in the Project X-Ray database.
        if self.toolchain() == Toolchain::Openxc7 && self.nextpnr_device.contains('/') {
            return Err(format!("bad nextpnr_device {:?}", self.nextpnr_device));
        }
        Ok(())
    }

    /// The nextpnr package, empty when there is none.
    pub fn package(&self) -> &str {
        self.nextpnr_package.as_deref().unwrap_or_default()
    }
}

/// FPGA families: the names on the chips and their datasheets. Each
/// family's FPGAs are in the catalog's `fpgas/<family>.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Lattice iCE40.
    Ice40,
    /// Lattice ECP5.
    Ecp5,
    /// Gowin GW1N (LittleBee).
    Gw1n,
    /// Gowin GW2A (Arora).
    Gw2a,
    /// Gowin GW5A (Arora V).
    Gw5a,
    /// AMD/Xilinx Artix-7.
    Artix7,
    /// AMD/Xilinx Spartan-7.
    Spartan7,
    /// AMD/Xilinx Zynq-7000.
    Zynq7000,
}

impl Family {
    /// Every family, in the engine's order.
    pub const ALL: [Family; 8] = [
        Family::Ice40,
        Family::Ecp5,
        Family::Gw1n,
        Family::Gw2a,
        Family::Gw5a,
        Family::Artix7,
        Family::Spartan7,
        Family::Zynq7000,
    ];

    /// The family's ID, as FPGA definitions and the catalog's file names
    /// give it, such as "ice40".
    pub fn id(self) -> &'static str {
        match self {
            Family::Ice40 => "ice40",
            Family::Ecp5 => "ecp5",
            Family::Gw1n => "gw1n",
            Family::Gw2a => "gw2a",
            Family::Gw5a => "gw5a",
            Family::Artix7 => "artix7",
            Family::Spartan7 => "spartan7",
            Family::Zynq7000 => "zynq7000",
        }
    }

    /// The family with this ID.
    pub fn from_id(id: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The family's full name for people, with its vendor, such as
    /// "Lattice iCE40".
    pub fn full_label(self) -> &'static str {
        match self {
            Family::Ice40 => "Lattice iCE40",
            Family::Ecp5 => "Lattice ECP5",
            Family::Gw1n => "Gowin GW1N",
            Family::Gw2a => "Gowin GW2A",
            Family::Gw5a => "Gowin GW5A",
            Family::Artix7 => "Xilinx Artix-7",
            Family::Spartan7 => "Xilinx Spartan-7",
            Family::Zynq7000 => "Xilinx Zynq-7000",
        }
    }

    /// The family's name for people, such as "iCE40".
    pub fn label(self) -> &'static str {
        match self {
            Family::Ice40 => "iCE40",
            Family::Ecp5 => "ECP5",
            Family::Gw1n => "GW1N",
            Family::Gw2a => "GW2A",
            Family::Gw5a => "GW5A",
            Family::Artix7 => "Artix-7",
            Family::Spartan7 => "Spartan-7",
            Family::Zynq7000 => "Zynq-7000",
        }
    }

    /// The toolchain that builds the family's FPGAs.
    pub fn toolchain(self) -> Toolchain {
        match self {
            Family::Ice40 => Toolchain::Ice40,
            Family::Ecp5 => Toolchain::Ecp5,
            Family::Gw1n | Family::Gw2a | Family::Gw5a => Toolchain::Gowin,
            Family::Artix7 | Family::Spartan7 | Family::Zynq7000 => Toolchain::Openxc7,
        }
    }

    /// The family yosys's synth_gowin synthesizes for (-family), when not
    /// its default, GW1N. Gowin only.
    pub fn yosys_family(self) -> Option<&'static str> {
        match self {
            Family::Gw2a => Some("gw2a"),
            Family::Gw5a => Some("gw5a"),
            _ => None,
        }
    }

    /// The family's folder in the Project X-Ray database, which the
    /// openXC7 bitstream tools read. Xilinx only.
    pub fn prjxray_family(self) -> Option<&'static str> {
        match self {
            Family::Artix7 => Some("artix7"),
            Family::Spartan7 => Some("spartan7"),
            Family::Zynq7000 => Some("zynq7"),
            _ => None,
        }
    }

    /// The extension of the family's constraint file format, without the
    /// dot: its toolchain's.
    pub fn constraint_extension(self) -> &'static str {
        self.toolchain().constraint_extension()
    }
}

/// The toolchains: how the engine builds a family's FPGAs, each with its
/// own flow. For logs, diagnostics and the API; people see families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Toolchain {
    /// iCE40: nextpnr-ice40 and icepack.
    Ice40,
    /// ECP5: nextpnr-ecp5 and ecppack.
    Ecp5,
    /// Gowin: nextpnr-himbaechel's gowin uarch and gowin_pack.
    Gowin,
    /// openXC7, for Xilinx 7-series: nextpnr-xilinx and the openXC7 tools.
    Openxc7,
}

impl Toolchain {
    /// The toolchain's ID, such as "openxc7". It also names the schema of
    /// its families' FPGA files.
    pub fn id(self) -> &'static str {
        match self {
            Toolchain::Ice40 => "ice40",
            Toolchain::Ecp5 => "ecp5",
            Toolchain::Gowin => "gowin",
            Toolchain::Openxc7 => "openxc7",
        }
    }

    /// The extension of the constraint file format, without the dot. Each
    /// place-and-route tool reads only its own format, and takes the format
    /// from its command-line option, not the file name.
    pub fn constraint_extension(self) -> &'static str {
        match self {
            Toolchain::Ice40 => "pcf",
            Toolchain::Ecp5 => "lpf",
            Toolchain::Gowin => "cst",
            Toolchain::Openxc7 => "xdc",
        }
    }
}

/// The schema of a board's file (Logix Catalog's
/// schemas/logix-board-v1.json, published at this address). Board files may
/// name it in a `"$schema"` key, so editors can check and complete them; the
/// engine refuses a file that names another one. Until the first release it
/// changes without a new version.
pub const BOARDS_SCHEMA: &str = "https://www.openlogix.org/schemas/logix-board-v1.json";

/// The schema an FPGA family's file, `fpgas/<family>.json`, names in its
/// `"$schema"` key: one per toolchain, shared by its families, owned by
/// Logix Catalog.
pub fn fpgas_schema(family: Family) -> String {
    let name = family.toolchain().id();
    format!("https://www.openlogix.org/schemas/logix-fpgas-{name}-v1.json")
}

/// The key a board file may use to name its schema.
pub const SCHEMA_KEY: &str = "$schema";

/// A board file's `$schema` value when it isn't `BOARDS_SCHEMA`: the
/// problem's message.
pub fn schema_problem(value: &serde_json::Value) -> Option<String> {
    if value.as_str() == Some(BOARDS_SCHEMA) {
        return None;
    }
    Some(format!(
        "Unknown $schema {value}: this Logix Engine reads {BOARDS_SCHEMA}."
    ))
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

pub fn is_usb_id(s: &str) -> bool {
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

impl Variant {
    /// Checks the programming fields.
    pub fn validate_programming(&self) -> Result<(), String> {
        if !PROGRAMMERS.contains(&self.programmer.as_str()) {
            return Err(format!(
                "unknown programmer {:?}. Known: {}",
                self.programmer,
                PROGRAMMERS.join(", ")
            ));
        }
        check_template(&self.args)?;
        if !is_usb_id(&self.usb.vid) || !is_usb_id(&self.usb.pid) {
            return Err("USB IDs must be four lowercase hex digits".into());
        }
        if self.usb.release.as_deref().is_some_and(|r| !is_usb_id(r)) {
            return Err("a USB release must be four lowercase hex digits".into());
        }
        if let Some(re) = &self.usb.product_regex {
            regex::Regex::new(re).map_err(|e| format!("bad product_regex: {e}"))?;
        }
        if let Some(re) = &self.usb.serial_regex {
            regex::Regex::new(re).map_err(|e| format!("bad serial_regex: {e}"))?;
        }
        Ok(())
    }

    /// The variant's value of a standard dimension, by dimension ID.
    pub fn value(&self, dimension: &str) -> Option<&str> {
        self.selector.value(dimension)
    }

    /// The variant for people: its value of each standard dimension, in
    /// their order, such as "FT232H adapter · RAM" or "USB · Flash".
    pub fn label(&self) -> String {
        STANDARD_DIMENSIONS
            .iter()
            .filter_map(|(dim, _, values)| {
                let v = self.value(dim)?;
                values
                    .iter()
                    .find(|(id, _, _)| *id == v)
                    .map(|(_, l, _)| *l)
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The variant's values as people read them, such as "FT232H adapter,
    /// RAM", in the dimensions' order.
    pub fn labels(&self, dimensions: &[Dimension]) -> Vec<String> {
        dimensions
            .iter()
            .filter_map(|d| {
                let v = self.value(&d.id)?;
                d.values.iter().find(|x| x.id == v).map(|x| x.label.clone())
            })
            .collect()
    }
}

/// A standard value: its ID, its label for people and a short explanation.
pub type StandardValue = (&'static str, &'static str, &'static str);

/// How the computer reaches the board.
pub const CONNECTIONS: &[StandardValue] = &[
    (
        "usb",
        "USB",
        "The board's own USB port, through the programmer chip on the board.",
    ),
    (
        "bootloader",
        "USB bootloader",
        "The board's own USB port, with the board in its bootloader mode.",
    ),
    (
        "ft2232h",
        "FT2232H adapter",
        "A USB adapter with an FTDI FT2232H chip, connected to the board's JTAG pins.",
    ),
    (
        "ft232h",
        "FT232H adapter",
        "A USB adapter with an FTDI FT232H chip, connected to the board's JTAG pins.",
    ),
    (
        "cmsis-dap",
        "CMSIS-DAP adapter",
        "A CMSIS-DAP (DAPLink) USB adapter, connected to the board's JTAG pins.",
    ),
    (
        "usb-blaster",
        "USB-Blaster",
        "An Altera USB-Blaster or compatible adapter, connected to the board's JTAG pins.",
    ),
    (
        "unspecified",
        "USB",
        "Programmed through USB; see the board's documentation for how to connect it.",
    ),
];

/// Where the design goes.
pub const MEMORIES: &[StandardValue] = &[
    ("flash", "Flash", "Kept when the board is powered off."),
    (
        "ram",
        "RAM",
        "Faster to upload; lost when the board is powered off.",
    ),
];

/// The standard dimensions, in order: ID, label and values.
pub const STANDARD_DIMENSIONS: &[(&str, &str, &[StandardValue])] = &[
    ("connection", "Connection", CONNECTIONS),
    ("memory", "Memory", MEMORIES),
];

/// The standard dimensions, connection then memory, each with the values
/// the variants use, in the standard order.
pub fn derive_dimensions(variants: &[Variant]) -> Vec<Dimension> {
    STANDARD_DIMENSIONS
        .iter()
        .map(|(id, label, values)| Dimension {
            id: id.to_string(),
            label: label.to_string(),
            values: values
                .iter()
                .filter(|(v, _, _)| variants.iter().any(|x| x.value(id) == Some(*v)))
                .map(|(v, l, h)| DimensionValue {
                    id: v.to_string(),
                    label: l.to_string(),
                    help: Some(h.to_string()),
                })
                .collect(),
        })
        .collect()
}

/// Checks a board's variants: at least one, each with a selector of
/// standard values (no two the same) and valid programming fields.
pub fn validate_variants(variants: &[Variant]) -> Result<(), String> {
    if variants.is_empty() {
        return Err("no variants".into());
    }
    let mut selectors = std::collections::BTreeSet::new();
    for v in variants {
        let ctx = |e: String| format!("variant ({}): {e}", v.selector);
        for (dim, _, values) in STANDARD_DIMENSIONS {
            let value = v.value(dim).unwrap_or_default();
            if !values.iter().any(|(x, _, _)| *x == value) {
                return Err(ctx(format!(
                    "{value:?} isn't a {dim} value. Known: {}",
                    values
                        .iter()
                        .map(|(x, _, _)| *x)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        if !selectors.insert(&v.selector) {
            return Err(ctx("another variant has the same selector".into()));
        }
        v.validate_programming().map_err(ctx)?;
    }
    Ok(())
}

/// The variant with the selector's values, or the first (the default)
/// when there is none. A selector the board has no variant for is an
/// error for people that lists the board's selectors.
pub fn select_variant<'a>(
    variants: &'a [Variant],
    choice: Option<&VariantChoice>,
) -> Result<&'a Variant, String> {
    match choice {
        None => variants
            .first()
            .ok_or_else(|| "the board has no variants".to_string()),
        Some(c) => variants
            .iter()
            .find(|v| c.fits(&v.selector))
            .ok_or_else(|| {
                format!(
                    "the board has no variant with {c}. Its variants: {}",
                    variants
                        .iter()
                        .map(|v| format!("({})", v.selector))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }),
    }
}

impl BoardDefinition {
    /// Checks a board definition, apart from its FPGA references.
    pub fn validate(&self) -> Result<(), String> {
        if !is_catalog_id(&self.fpga_id) {
            return Err(format!("bad fpga_id {:?}", self.fpga_id));
        }
        validate_variants(&self.variants)
    }
}

impl Catalog {
    /// The catalog from its files: each board's file by ID (its file name
    /// without .json), with its `$schema`, and each `fpgas/<family>.json`
    /// by family: its FPGAs by ID, their family the file's name.
    pub fn parse(
        board_files: &[(&str, &str)],
        fpga_files: &[(&str, &str)],
    ) -> Result<Catalog, String> {
        let mut boards = BTreeMap::new();
        for (id, text) in board_files {
            let mut map: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(text).map_err(|e| format!("boards/{id}.json: {e}"))?;
            let schema = map
                .remove(SCHEMA_KEY)
                .ok_or_else(|| format!("boards/{id}.json: no {SCHEMA_KEY}"))?;
            if let Some(problem) = schema_problem(&schema) {
                return Err(format!("boards/{id}.json: {problem}"));
            }
            let mut b: BoardDefinition = serde_json::from_value(serde_json::Value::Object(map))
                .map_err(|e| format!("boards/{id}.json: {e}"))?;
            b.dimensions = derive_dimensions(&b.variants);
            boards.insert(id.to_string(), b);
        }
        let mut fpgas = BTreeMap::new();
        for (name, text) in fpga_files {
            let file = format!("fpgas/{name}.json");
            let family = Family::from_id(name)
                .ok_or_else(|| format!("{file}: {name:?} isn't an FPGA family"))?;
            let mut entries: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
            let want = fpgas_schema(family);
            match entries.remove(SCHEMA_KEY) {
                Some(v) if v.as_str() == Some(want.as_str()) => {}
                Some(v) => return Err(format!("{file}: unknown $schema {v}: expected {want}")),
                None => return Err(format!("{file}: no {SCHEMA_KEY}")),
            }
            for (id, entry) in entries {
                let serde_json::Value::Object(mut fields) = entry else {
                    return Err(format!("{file}: {id}: not an object"));
                };
                if fields.contains_key("family") {
                    return Err(format!(
                        "{file}: {id}: no family field: the file's name gives it"
                    ));
                }
                fields.insert("family".into(), serde_json::Value::from(family.id()));
                let f: FpgaDefinition = serde_json::from_value(serde_json::Value::Object(fields))
                    .map_err(|e| format!("{file}: {id}: {e}"))?;
                if fpgas.insert(id.clone(), f).is_some() {
                    return Err(format!("{file}: {id}: in another FPGA file too"));
                }
            }
        }
        let catalog = Catalog { boards, fpgas };
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), String> {
        for (id, f) in &self.fpgas {
            if !is_catalog_id(id) {
                return Err(format!("bad FPGA ID {id:?}"));
            }
            f.validate().map_err(|e| format!("FPGA {id}: {e}"))?;
        }
        for (id, b) in &self.boards {
            if !is_catalog_id(id) {
                return Err(format!("bad board ID {id:?}"));
            }
            b.validate().map_err(|e| format!("board {id}: {e}"))?;
            let Some(fpga) = self.fpgas.get(&b.fpga_id) else {
                return Err(format!("board {id}: unknown FPGA {}", b.fpga_id));
            };
            if let Some((name, field, text)) =
                crate::resources::problems(&b.resources, Some(fpga.family))
                    .into_iter()
                    .next()
            {
                return Err(format!("board {id}: resources/{name}/{field}: {text}"));
            }
            if let Some((field, text)) = crate::resources::board_requires_problems(
                &b.requires,
                &b.resources,
                Some(fpga.family),
            )
            .into_iter()
            .next()
            {
                return Err(format!("board {id}: requires/{field}: {text}"));
            }
            if let Some((field, text)) = crate::resources::ai_hints_problems(b.ai_hints.as_ref())
                .into_iter()
                .next()
            {
                return Err(format!("board {id}: ai_hints/{field}: {text}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fpgas_need_their_toolchains_fields() {
        let ecp5 = FpgaDefinition {
            family: Family::Ecp5,
            part_number: "LFE5U-25F-6BG256C".into(),
            size: "25k".into(),
            nextpnr_device: "25k".into(),
            nextpnr_package: Some("CABGA256".into()),
            nextpnr_speed: Some("6".into()),
            nextpnr_family: None,
            packer_device: None,
            jtag_idcode: Some("0x41111043".into()),
        };
        assert!(ecp5.validate().is_ok());
        let no_speed = FpgaDefinition {
            nextpnr_speed: None,
            ..ecp5.clone()
        };
        assert_eq!(
            no_speed.validate().unwrap_err(),
            "ecp5 FPGAs need nextpnr_speed"
        );
        let ice40 = FpgaDefinition {
            family: Family::Ice40,
            ..ecp5.clone()
        };
        assert_eq!(
            ice40.validate().unwrap_err(),
            "ice40 FPGAs have no nextpnr_speed"
        );
        let bad = FpgaDefinition {
            nextpnr_package: Some("a b".into()),
            ..ecp5.clone()
        };
        assert!(bad.validate().unwrap_err().contains("nextpnr_package"));

        let gowin = FpgaDefinition {
            family: Family::Gw1n,
            part_number: "GW1NR-LV9QN88PC6/I5".into(),
            size: "9k".into(),
            nextpnr_device: "GW1NR-LV9QN88PC6/I5".into(),
            nextpnr_package: None,
            nextpnr_speed: None,
            nextpnr_family: Some("GW1N-9C".into()),
            packer_device: Some("GW1N-9C".into()),
            jtag_idcode: Some("0x0100481b".into()),
        };
        assert!(gowin.validate().is_ok());
        let no_packer = FpgaDefinition {
            packer_device: None,
            ..gowin.clone()
        };
        assert_eq!(
            no_packer.validate().unwrap_err(),
            "gw1n FPGAs need packer_device"
        );
        let with_package = FpgaDefinition {
            nextpnr_package: Some("QN88".into()),
            ..gowin.clone()
        };
        assert_eq!(
            with_package.validate().unwrap_err(),
            "gw1n FPGAs have no nextpnr_package"
        );
        let ecp5_with_family = FpgaDefinition {
            nextpnr_family: Some("GW1N-9C".into()),
            ..ecp5.clone()
        };
        assert_eq!(
            ecp5_with_family.validate().unwrap_err(),
            "ecp5 FPGAs have no nextpnr_family"
        );
        let xilinx = FpgaDefinition {
            family: Family::Artix7,
            part_number: "XC7A35T-1CSG324".into(),
            size: "35k".into(),
            nextpnr_device: "xc7a35tcsg324-1".into(),
            nextpnr_package: None,
            nextpnr_speed: None,
            nextpnr_family: None,
            packer_device: None,
            jtag_idcode: Some("0x0362d093".into()),
        };
        assert!(xilinx.validate().is_ok());
        for bad in ["0x0362D093", "0362d093", "0x362d093", "0x0362d0930"] {
            let e = FpgaDefinition {
                jtag_idcode: Some(bad.into()),
                ..xilinx.clone()
            }
            .validate()
            .unwrap_err();
            assert!(e.starts_with("bad jtag_idcode"), "{bad}: {e}");
        }
        assert!(
            FpgaDefinition {
                jtag_idcode: None,
                ..xilinx.clone()
            }
            .validate()
            .is_ok(),
            "optional"
        );
        assert_eq!(parse_idcode("0x0362d093"), Some(0x0362d093));
        let ice40_with_idcode = FpgaDefinition {
            family: Family::Ice40,
            nextpnr_speed: None,
            ..ecp5.clone()
        };
        assert_eq!(
            ice40_with_idcode.validate().unwrap_err(),
            "ice40 FPGAs have no jtag_idcode"
        );
        // It names a folder in the Project X-Ray database.
        let bad_device = FpgaDefinition {
            nextpnr_device: "../etc".into(),
            ..xilinx
        };
        assert!(
            bad_device
                .validate()
                .unwrap_err()
                .contains("nextpnr_device")
        );
    }

    #[test]
    fn families_name_their_toolchains_and_tool_families() {
        use Family::*;
        let gowin: Vec<_> = Family::ALL
            .into_iter()
            .filter(|f| f.toolchain() == Toolchain::Gowin)
            .collect();
        assert_eq!(gowin, [Gw1n, Gw2a, Gw5a]);
        assert_eq!(
            [Gw1n, Gw2a, Gw5a].map(Family::yosys_family),
            [None, Some("gw2a"), Some("gw5a")]
        );
        assert_eq!(
            [Artix7, Spartan7, Zynq7000].map(Family::prjxray_family),
            [Some("artix7"), Some("spartan7"), Some("zynq7")]
        );
        assert_eq!(Zynq7000.toolchain(), Toolchain::Openxc7);
        assert_eq!(Family::from_id("gw5a"), Some(Gw5a));
        assert_eq!(Family::from_id("gowin"), None);
        assert_eq!(
            fpgas_schema(Gw2a),
            "https://www.openlogix.org/schemas/logix-fpgas-gowin-v1.json"
        );
    }

    #[test]
    fn ftdi_commands_name_the_chosen_device() {
        // openFPGALoader's FTDI drivers open the device by bus and address
        // when told, so every such command names the device the engine
        // chose; otherwise openFPGALoader would open the first one with the
        // USB IDs, and the engine would refuse when several are connected.
        // (Numato's 2a19 cables are FTDI chips too. USB-Blaster and
        // CMSIS-DAP can't be told, so they keep that refusal.)
        let mut checked = 0;
        for (name, b) in &get().boards {
            for c in &b.variants {
                if c.programmer == "openfpgaloader"
                    && ["0403", "2a19"].contains(&c.usb.vid.as_str())
                {
                    assert!(
                        c.args.contains("--busdev-num ${BUS}:${DEV}"),
                        "{name} ({}): {}",
                        c.selector,
                        c.args
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 70, "{checked}");
    }

    #[test]
    fn dfu_util_commands_name_the_chosen_device() {
        // dfu-util opens the device at a USB port path when given --path
        // (-p), in the same bus-port.port form as the engine's location, so
        // two identical boards in their bootloaders don't block each other.
        let mut checked = 0;
        for (name, b) in &get().boards {
            for c in &b.variants {
                if c.programmer == "dfu-util" {
                    assert!(
                        c.args.contains("-p ${USB_PATH}"),
                        "{name} ({}): {}",
                        c.selector,
                        c.args
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 9);
    }

    #[test]
    fn jtag_boards_have_their_fpgas_id_code() {
        // Every board programmed over JTAG by openFPGALoader can be checked
        // for the wrong FPGA before programming, so its FPGA needs its ID
        // code.
        let c = get();
        let mut checked = 0;
        for (name, b) in &c.boards {
            let fpga = &c.fpgas[&b.fpga_id];
            for v in &b.variants {
                if v.programmer == "openfpgaloader" && fpga.toolchain() != Toolchain::Ice40 {
                    assert!(
                        crate::program::checks_fpga(v, fpga),
                        "{name} ({}): {} has no jtag_idcode",
                        v.selector,
                        b.fpga_id
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 90, "{checked}");
        // Spot checks against openFPGALoader 1.2.0's part.hpp.
        for (id, code) in [
            ("xc7a35tcsg324-1", "0x0362d093"),
            ("xc7a100tcsg324-1", "0x03631093"),
            ("lfe5u-25f-6bg256c", "0x41111043"),
            ("lfe5um5g-85f-8bg381c", "0x81113043"),
            ("gw1nr-lv9qn88pc6-i5", "0x0100481b"),
        ] {
            assert_eq!(c.fpgas[id].jtag_idcode.as_deref(), Some(code), "{id}");
        }
    }

    #[test]
    fn the_catalog_has_every_toolchains_boards() {
        let c = get();
        let count = |a: Toolchain| {
            let fpgas = c.fpgas.values().filter(|f| f.toolchain() == a).count();
            let boards = c
                .boards
                .values()
                .filter(|b| c.fpgas[&b.fpga_id].toolchain() == a)
                .count();
            (fpgas, boards)
        };
        assert_eq!(count(Toolchain::Ice40), (40, 27));
        assert_eq!(count(Toolchain::Ecp5), (39, 22));
        assert_eq!(count(Toolchain::Gowin), (241, 9));
        assert_eq!(count(Toolchain::Openxc7), (212, 23));
        // Every family has FPGAs.
        for family in Family::ALL {
            assert!(c.fpgas.values().any(|f| f.family == family), "{family:?}");
        }
        let arty = &c.fpgas[&c.boards["arty-a7-35t"].fpga_id];
        assert_eq!(arty.nextpnr_device, "xc7a35tcsg324-1");
        assert_eq!(arty.family, Family::Artix7);
        let nano = &c.fpgas[&c.boards["sipeed-tang-nano-9k"].fpga_id];
        assert_eq!(nano.nextpnr_device, "GW1NR-LV9QN88PC6/I5");
        assert_eq!(nano.nextpnr_family.as_deref(), Some("GW1N-9C"));
        assert_eq!(nano.packer_device.as_deref(), Some("GW1N-9C"));
        assert_eq!(nano.family, Family::Gw1n);
        let primer = &c.fpgas[&c.boards["sipeed-tang-primer-20k-dock"].fpga_id];
        assert_eq!(primer.family, Family::Gw2a);
        let sugar = &c.boards["icesugar-pro"];
        let f = &c.fpgas[&sugar.fpga_id];
        assert_eq!(
            (
                f.nextpnr_device.as_str(),
                f.package(),
                f.nextpnr_speed.as_deref()
            ),
            ("25k", "CABGA256", Some("6"))
        );
        let alhambra = &c.boards["alhambra-ii"];
        assert_eq!(alhambra.fpga_id, "ice40hx4k-tq144-8k");
        let v = &alhambra.variants[0];
        assert_eq!(v.programmer, "openfpgaloader");
        assert_eq!(v.usb.vid, "0403");
        assert_eq!(v.usb.pid, "6010");
        // Every board has both standard dimensions, here with one value each.
        assert_eq!(
            alhambra
                .dimensions
                .iter()
                .map(|d| (d.id.as_str(), d.values.len()))
                .collect::<Vec<_>>(),
            [("connection", 1), ("memory", 1)]
        );
        assert_eq!(v.selector.connection, "usb");
        assert_eq!(v.selector.memory, "flash");
        assert_eq!(c.fpgas["ice40hx4k-tq144-8k"].package(), "tq144:4k");
        let variants = || c.boards.values().flat_map(|b| &b.variants);
        assert_eq!(c.boards.len(), 81);
        assert_eq!(variants().count(), 128);
        // Users pick boards by description, so no two boards share one.
        let mut descriptions: Vec<&str> = c.boards.values().map(|b| b.name.as_str()).collect();
        descriptions.sort();
        descriptions.dedup();
        assert_eq!(descriptions.len(), c.boards.len());
        for p in PROGRAMMERS {
            assert!(variants().any(|v| v.programmer == *p), "no board uses {p}");
        }
        // Boards with a choice: connections and memories. Sizes are separate
        // boards, since the pins may differ.
        let memory_of = |id: &str, i: usize| c.boards[id].variants[i].selector.memory.clone();
        let dims = |id: &str| -> Vec<usize> {
            c.boards[id]
                .dimensions
                .iter()
                .map(|d| d.values.len())
                .collect()
        };
        for (id, choices) in [
            ("icesugar-pro", [1, 2]),
            ("colorlight-i5-v7-0", [1, 2]),
            ("colorlight-5a-75e-v71", [3, 2]),
            ("colorlight-i9-v7-2", [4, 2]),
            ("fleafpga-ohm", [3, 2]),
            ("arty-a7-35t", [1, 2]),
            ("sipeed-tang-nano", [1, 2]),
            // The bootloader writes flash, the adapters RAM.
            ("butterstick-r10-2g-85k", [3, 2]),
            // Zynq: openFPGALoader can't write their flash.
            ("pynq-z1", [1, 1]),
            // The ULX3S may have a bootloader at the start of its flash.
            ("ulx3s-12f", [1, 1]),
        ] {
            assert_eq!(dims(id), choices, "{id}");
        }
        // Flash is the default wherever a board has it: the design stays
        // when the board is powered off, as users expect.
        assert_eq!(memory_of("arty-a7-35t", 0), "flash");
        assert_eq!(memory_of("arty-a7-35t", 1), "ram");
        for (id, b) in &c.boards {
            let cs = &b.variants;
            if cs.iter().any(|v| v.selector.memory == "flash") {
                assert_eq!(cs[0].selector.memory, "flash", "{id}");
            }
        }
        // Where openFPGALoader can't tell the part (no --board), the flash
        // command names it, so it loads the right SPI-over-JTAG bridge.
        for b in c.boards.values() {
            if c.fpgas[&b.fpga_id].toolchain() != Toolchain::Openxc7 {
                continue;
            }
            for v in &b.variants {
                if v.selector.memory == "flash" {
                    assert!(
                        v.args.contains("--board ") || v.args.contains("--fpga-part "),
                        "{}",
                        b.name
                    );
                }
            }
        }
        let with_choices = c
            .boards
            .values()
            .filter(|b| b.dimensions.iter().any(|d| d.values.len() > 1));
        assert_eq!(with_choices.count(), 32);
        // Where the design goes follows from the command: openFPGALoader
        // writes RAM unless told to write flash (iCE40 always flash).
        let memory = |id: &str| c.boards[id].variants[0].selector.memory.clone();
        assert_eq!(memory("arty-a7-35t"), "flash");
        assert_eq!(memory("sipeed-tang-nano-9k"), "flash");
        assert_eq!(memory("sipeed-tang-nano"), "flash");
        assert_eq!(memory("ulx3s-12f"), "ram");
        assert_eq!(memory("icestick"), "flash");
        for b in c.boards.values() {
            for v in &b.variants {
                let flag = v.args.split(' ').any(|w| w == "-f" || w == "--write-flash");
                if v.programmer == "openfpgaloader"
                    && c.fpgas[&b.fpga_id].toolchain() != Toolchain::Ice40
                {
                    assert_eq!(v.selector.memory == "flash", flag, "{}", b.name);
                }
            }
        }
        // Labels and help come from the engine.
        let ft = &c.boards["butterstick-r10-2g-85k"].dimensions[0].values[2];
        assert_eq!(
            (ft.id.as_str(), ft.label.as_str()),
            ("ft232h", "FT232H adapter")
        );
        assert!(ft.help.as_deref().unwrap().contains("JTAG pins"));
        // Boards moved to another programmer are marked untested, with a note for users.
        for id in ["ulx3s-12f", "ulx3s-85f", "odt-icyblue-feather"] {
            for v in &c.boards[id].variants {
                assert!(!v.tested, "{id}");
                assert!(v.user_notes.is_some(), "{id}");
            }
        }
        // maintainer_notes are never required (the user's decision,
        // 2026-11-07).
        // Maintainer notes stay in the catalog file.
        let json = serde_json::to_value(&c.boards["ulx3s-12f"]).unwrap();
        let first = &json["variants"][0];
        assert!(first.get("maintainer_notes").is_none());
        assert!(first.get("user_notes").is_some());
        assert!(c.boards["alhambra-ii"].variants[0].tested);
        // The iCESugar boards share USB IDs; their serial numbers start with
        // a board ID (icesprog's source): 0700, 0710 Pro, 0720 nano.
        for (id, re) in [
            ("icesugar-1-5", "^0700"),
            ("icesugar-pro", "^0710"),
            ("icesugar-nano", "^0720"),
        ] {
            for v in &c.boards[id].variants {
                assert_eq!(v.usb.serial_regex.as_deref(), Some(re), "{id}");
            }
        }
    }

    #[test]
    fn templates() {
        assert!(check_template("-d d:${BUS}/${DEV} ${BITSTREAM}").is_ok());
        assert!(check_template("--vid ${VID}").is_err());
        assert!(check_template("${BIN_FILE}").is_err());
        // Renamed before the first release, so they read as USB terms.
        assert!(check_template("-p ${PORT} ${BITSTREAM}").is_err());
        assert!(check_template("-s ${SERIAL} ${BITSTREAM}").is_err());
        assert!(check_template("${BITSTREAM").is_err());
    }

    /// The top-level alternatives of a regular expression: `a|(b|c)` has
    /// two, `a` and `(b|c)`.
    fn top_level_alternatives(re: &str) -> Vec<String> {
        let (mut out, mut cur, mut depth, mut chars) = (Vec::new(), String::new(), 0, re.chars());
        let mut in_class = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    cur.push(c);
                    cur.extend(chars.next());
                    continue;
                }
                '[' if !in_class => in_class = true,
                ']' if in_class => in_class = false,
                '(' if !in_class => depth += 1,
                ')' if !in_class => depth -= 1,
                '|' if !in_class && depth == 0 => {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        out.push(cur);
        out
    }

    /// Catalog name patterns match from the start of the name in every
    /// alternative: in `^(A)|(B)`, `^` anchors only A, so B would match
    /// anywhere in the name. Write `^(A|B)` or `^A|^B`.
    #[test]
    fn catalog_name_patterns_are_anchored() {
        assert_eq!(top_level_alternatives("^(A)|(B)"), ["^(A)", "(B)"]);
        assert_eq!(top_level_alternatives("^(A|B)$"), ["^(A|B)$"]);
        assert_eq!(top_level_alternatives("^[|]\\|x"), ["^[|]\\|x"]);
        for (id, b) in &get().boards {
            for v in &b.variants {
                for re in [&v.usb.product_regex, &v.usb.serial_regex]
                    .into_iter()
                    .flatten()
                {
                    for alt in top_level_alternatives(re) {
                        assert!(alt.starts_with('^'), "{id} ({}): {re:?}", v.selector);
                    }
                }
            }
        }
    }

    #[test]
    fn bad_boards_are_refused() {
        let ok = get().boards["alhambra-ii"].clone();
        fn first(b: &mut BoardDefinition) -> &mut Variant {
            &mut b.variants[0]
        }
        let mut b = ok.clone();
        b.variants.clear();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).programmer = "iceburn".into();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.vid = "0403A".into();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.product_regex = Some("(".into());
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.serial_regex = Some("(".into());
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.release = Some("700".into());
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.release = Some("0700".into());
        assert!(b.validate().is_ok());
        // The same selector twice, an unknown connection or memory value.
        let mut b = ok.clone();
        let mut twin = b.variants[0].clone();
        twin.args = "--other".into();
        b.variants.push(twin);
        assert!(b.validate().unwrap_err().contains("same selector"));
        let mut b = ok.clone();
        first(&mut b).selector.connection = "serial".into();
        assert!(b.validate().unwrap_err().contains("unspecified"));
        let mut b = ok;
        first(&mut b).selector.memory = "eeprom".into();
        assert!(b.validate().unwrap_err().contains("flash, ram"));
    }

    #[test]
    fn resources_in_type_order() {
        // Every board lists its resources by type, clocks first, whatever
        // its file's order.
        let types = |b: &BoardDefinition| -> Vec<String> {
            b.resources
                .values()
                .map(|r| {
                    serde_json::to_value(r).unwrap()["type"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect()
        };
        let order = [
            "clock",
            "leds",
            "rgb_led",
            "led_matrix",
            "buttons",
            "switches",
            "uart",
            "pmod",
            "header",
            "unclassified",
        ];
        for (id, b) in &get().boards {
            let ranks: Vec<usize> = types(b)
                .iter()
                .map(|t| order.iter().position(|o| o == t).unwrap())
                .collect();
            assert!(ranks.is_sorted(), "{id}: {:?}", types(b));
        }
    }

    #[test]
    fn resources_of_the_catalog() {
        let c = get();
        // Every board has its clock or LEDs.
        let without: Vec<_> = c
            .boards
            .iter()
            .filter(|(_, b)| b.resources.is_empty())
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(without.is_empty(), "{without:?}");
        assert_eq!(
            serde_json::to_value(&c.boards["alhambra-ii"].resources).unwrap(),
            serde_json::json!({
                "sys_clk": {"type": "clock", "pin": "49", "mhz": 12.0},
                "leds": {"type": "leds", "pins": ["45", "44", "43", "42", "41", "39", "38", "37"], "active": "high"}
            })
        );
        // A clock that needs a jumper says so.
        let Resource::Clock(up) = &c.boards["upduino3"].resources["sys_clk"] else {
            panic!("a clock");
        };
        assert_eq!((up.pin.as_str(), up.mhz), ("20", 12.0));
        let [crate::resources::Requirement::Setup(jumper)] = up.requires.as_slice() else {
            panic!("a setup");
        };
        assert!(jumper.text.contains("R16"));
        // Jumpers are setups, never only notes: no resource note says one.
        for (id, b) in &c.boards {
            for (name, r) in &b.resources {
                let notes = serde_json::to_value(r).unwrap()["user_notes"].clone();
                assert!(
                    !notes.to_string().to_lowercase().contains("jumper"),
                    "{id} {name}"
                );
            }
        }
        // Pins every design must drive: the ULX3S's ESP32 GPIO0, Fomu's USB.
        let drives = |id: &str| -> Vec<(String, Vec<String>, ActiveLevel)> {
            c.boards[id]
                .requires
                .iter()
                .filter_map(|q| match q {
                    crate::resources::Requirement::Drive(d) => {
                        Some((d.name.clone(), d.pins.clone(), d.level))
                    }
                    crate::resources::Requirement::Setup(_) => None,
                })
                .collect()
        };
        for size in ["12f", "25f", "45f", "85f"] {
            assert_eq!(
                drives(&format!("ulx3s-{size}")),
                [("wifi_gpio0".into(), vec!["L2".into()], ActiveLevel::High)]
            );
        }
        let fomu: Vec<_> = drives("fomu")
            .into_iter()
            .map(|d| (d.0, d.1[0].clone(), d.2))
            .collect();
        assert_eq!(
            fomu,
            [
                ("usb_dp".into(), "A1".into(), ActiveLevel::Low),
                ("usb_dn".into(), "A2".into(), ActiveLevel::Low),
                ("usb_dp_pu".into(), "A4".into(), ActiveLevel::Low),
            ]
        );
        assert_eq!(
            c.boards.values().filter(|b| !b.requires.is_empty()).count(),
            5
        );
        // The ButterStick's RGB LEDs are a matrix, from its schematic.
        let Resource::LedMatrix(m) = &c.boards["butterstick-r10-2g-85k"].resources["leds"] else {
            panic!("a matrix");
        };
        assert_eq!((m.leds.len(), m.colors.red.as_str()), (7, "T1"));
        assert!(m.source.is_some());
        // The Tang Primer 20K is a board per carrier: only the Dock has LEDs,
        // the Lite has a clock and four PMODs, from its schematic.
        assert_eq!(c.boards["sipeed-tang-primer-20k-dock"].resources.len(), 2);
        let lite = &c.boards["sipeed-tang-primer-20k-lite"].resources;
        let types: Vec<_> = lite
            .iter()
            .map(|(name, r)| {
                (
                    name.as_str(),
                    serde_json::to_value(r).unwrap()["type"].clone(),
                )
            })
            .collect();
        assert_eq!(
            types,
            [
                ("sys_clk", "clock".into()),
                ("pmod_j2", "pmod".into()),
                ("pmod_j6", "pmod".into()),
                ("pmod_j7", "pmod".into()),
                ("pmod_j8", "pmod".into()),
            ]
        );
        // ECP5 and Xilinx pin files name an I/O standard: every resource has it.
        for (id, b) in &c.boards {
            let arch = c.fpgas[&b.fpga_id].toolchain();
            if matches!(arch, Toolchain::Ecp5 | Toolchain::Openxc7) {
                for (name, r) in &b.resources {
                    assert!(r.io_standard().is_some(), "{id} {name}");
                }
            }
        }
    }

    #[test]
    fn not_in_core_names_what_the_core_lacks() {
        let c = get();
        let entry = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // A core that builds only one FPGA.
        let core = crate::packages::CoreFpgas::from([(
            "ice40".to_string(),
            vec![entry(&[("device", "up5k"), ("package", "sg48")])],
        )]);
        let missing = not_in_core(c, &core);
        assert_eq!(missing.len(), c.fpgas.len() - 1);
        assert!(!missing.iter().any(|m| m.starts_with("ice40up5k-sg48 ")));
        assert!(
            missing.contains(&"ice40hx1k-cb132 (ice40: device hx1k, package cb132)".to_string())
        );
        assert!(
            missing.contains(
                &"lfe5u-25f-6bg256c (ecp5: device 25k, package CABGA256, speed 6)".to_string()
            ),
            "{missing:?}"
        );
    }

    #[test]
    fn fpga_files_give_their_family() {
        let files = json_files(&test_copy_folder().join("fpgas")).unwrap();
        let names: Vec<&str> = files.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(
            names,
            [
                "artix7", "ecp5", "gw1n", "gw2a", "gw5a", "ice40", "spartan7", "zynq7000"
            ]
        );
        let f = r#""part_number": "P", "size": "1k", "nextpnr_device": "d", "nextpnr_package": "p", "nextpnr_speed": "6""#;
        // Each file names its toolchain's schema.
        let file = |schema: &str, fields: &str| {
            let schema = format!("https://www.openlogix.org/schemas/logix-fpgas-{schema}-v1.json");
            format!(r#"{{"$schema": "{schema}", "a": {{{fields}}}}}"#)
        };
        let c = Catalog::parse(&[], &[("ecp5", &file("ecp5", f))]).unwrap();
        assert_eq!(c.fpgas["a"].family, Family::Ecp5);
        let err = |files: &[(&str, &str)]| Catalog::parse(&[], files).unwrap_err();
        assert_eq!(
            err(&[("ecp5", &file("ecp5", f)), ("ice40", &file("ice40", f))]),
            "fpgas/ice40.json: a: in another FPGA file too"
        );
        let with_family = format!(r#""family": "ecp5", {f}"#);
        assert!(err(&[("ecp5", &file("ecp5", &with_family))]).contains("the file's name gives it"));
        assert_eq!(
            err(&[("gowin", &file("gowin", f))]),
            "fpgas/gowin.json: \"gowin\" isn't an FPGA family"
        );
        // Families of one toolchain share its schema.
        let g = r#""part_number": "P", "size": "1k", "nextpnr_device": "d", "packer_device": "p""#;
        let c = Catalog::parse(&[], &[("gw5a", &file("gowin", g))]).unwrap();
        assert_eq!(c.fpgas["a"].family, Family::Gw5a);
        assert!(
            err(&[("gw5a", &file("gw5a", g))])
                .contains("expected https://www.openlogix.org/schemas/logix-fpgas-gowin-v1.json")
        );
        assert_eq!(
            err(&[("ecp5", &format!(r#"{{"a": {{{f}}}}}"#))]),
            "fpgas/ecp5.json: no $schema"
        );
        assert_eq!(
            err(&[("ecp5", &file("gowin", f))]),
            "fpgas/ecp5.json: unknown $schema \"https://www.openlogix.org/schemas/logix-fpgas-gowin-v1.json\": \
             expected https://www.openlogix.org/schemas/logix-fpgas-ecp5-v1.json"
        );
    }

    #[test]
    fn the_catalog_comes_from_logix_catalog() {
        let schema: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(test_copy_folder().join("schemas/logix-board-v1.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(schema["$id"], BOARDS_SCHEMA);
        let fpga_files = json_files(&test_copy_folder().join("fpgas")).unwrap();
        let fpgas: Vec<(&str, &str)> = fpga_files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let fpgas = &fpgas[..];
        // A board file without its $schema, or with another, is refused.
        let board = |schema: &str| {
            format!(
                r#"{{{schema}"name": "X", "fpga_id": "ice40up5k-sg48", "variants": [{{"selector": {{"connection": "usb", "memory": "flash"}}, "programmer": "iceprog", "args": "${{BITSTREAM}}", "usb": {{"vid": "0403", "pid": "6014"}}, "tested": true}}]}}"#
            )
        };
        let ok = board(&format!("\"$schema\": \"{BOARDS_SCHEMA}\", "));
        assert!(
            Catalog::parse(&[("x", &ok)], fpgas).is_ok(),
            "{:?}",
            Catalog::parse(&[("x", &ok)], fpgas).err()
        );
        assert!(
            Catalog::parse(&[("x", &board(""))], fpgas)
                .unwrap_err()
                .contains("no $schema")
        );
        let other =
            board("\"$schema\": \"https://www.openlogix.org/schemas/logix-boards-v1.json\", ");
        assert!(
            Catalog::parse(&[("x", &other)], fpgas)
                .unwrap_err()
                .contains("Unknown $schema")
        );
    }
}
