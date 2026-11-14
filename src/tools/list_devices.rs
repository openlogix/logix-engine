//! The list_devices tool (docs/tool-api.md, Tables 11 to 13).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::catalog::Variant;
use crate::devices::{self, BoardMatch, Candidate, Device, MatchLevel, UnreachableDevice};
use crate::drivers::{self, DriverStatus};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct ListDevices;

impl EngineTool for ListDevices {
    const NAME: &'static str = "list_devices";
    const DESCRIPTION: &'static str = "Lists the USB devices connected to this computer, with \
        their USB IDs, names and serial numbers, and for each the boards it may be: catalog \
        boards and the request's custom_boards_folder, best first, each with the variants it fits and \
        how well it matches: strong (very likely this board), possible (a few other boards look \
        the same) or weak (many boards fit). Show the level as it is: how the engine decides it \
        is internal and changes without notice. \
        program_board needs the id of one of these devices. Given a build_folder, it lists only \
        the devices that fit that build's board and variant, best first, each with its \
        build_match: program the only one, or ask the user when there are several. \
        Devices that need a driver or permissions first are listed apart, under unreachable, \
        with the fix. On Windows, a device whose board's programmer needs a driver has driver: \
        the one it needs (WinUSB), on which USB interface, the one it has and whether it's ready.";
    const READ_ONLY: bool = true;
    type Input = ListDevicesInput;
    type Output = ListDevicesOutput;
}

/// Optionally, the host's own boards to match the devices against, besides
/// the catalog's, and a build whose board the devices must fit.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesInput {
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
    /// The build folder of a successful build, as an absolute path. When
    /// given, only devices that fit the build's board in its variant are
    /// listed (in devices and unreachable), each with build_match, best
    /// first: the devices program_board can program that build into. The
    /// build's board comes from its build record, custom boards included.
    #[serde(default)]
    pub build_folder: Option<String>,
}

/// The connected devices.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesOutput {
    /// The connected USB devices the engine can open, sorted by vendor ID,
    /// product ID, bus and device number.
    pub devices: Vec<Device>,
    /// Devices the engine sees but can't open, so it can't program them or
    /// read their names, and whose USB IDs fit a board: each with the fix,
    /// such as permissions or a driver. Other devices it can't open are left
    /// out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreachable: Vec<UnreachableDevice>,
}

/// The drivers Windows has on the present devices with some USB IDs, on an
/// interface (drivers::current_drivers; a fake in tests).
pub type DriverLookup<'a> = &'a dyn Fn(&str, &str, u8, bool) -> Vec<Option<String>>;

pub fn run_with(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ListDevicesOutput, ToolError> {
    let lookup = |v: &str, p: &str, i: u8, c: bool| drivers::current_drivers(v, p, i, c);
    run_with_drivers(
        input,
        scan,
        cfg!(windows).then_some(&lookup as DriverLookup),
    )
}

/// As run_with, with Windows' drivers from `drivers` (None: not on Windows,
/// so no driver statuses).
pub fn run_with_drivers(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
    drivers: Option<DriverLookup>,
) -> Result<ListDevicesOutput, ToolError> {
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let known = crate::board::known_boards(&custom);
    let boards: Vec<Candidate> = known
        .values()
        .map(|b| Candidate {
            id: b.id,
            custom: b.custom,
            variants: b.variants,
        })
        .collect();

    // The build's board as program_board will program it.
    let build_variant: Option<Variant> = match &input.build_folder {
        Some(folder) => {
            let built = super::program_board::read_build(folder)?.1.board;
            Some(super::program_board::upload_board(&built, &custom, folder)?.variant)
        }
        None => None,
    };
    let build_usb = build_variant.as_ref().map(|v| v.usb.clone());
    let mut out = ListDevicesOutput {
        devices: Vec::new(),
        unreachable: Vec::new(),
    };
    for mut d in scan()? {
        let build_level = match &build_usb {
            None => None,
            Some(usb) => {
                let fits = if d.reachable {
                    devices::matches(&d, usb)
                } else {
                    devices::same_ids(&d, usb)
                };
                if !fits {
                    continue;
                }
                Some(devices::match_level(&d, usb))
            }
        };
        let (matches, promoted) = devices::board_matches_and_promotion(&d, &boards);
        let driver = drivers.and_then(|lookup| {
            driver_status(&d, &matches, &boards, build_variant.as_ref(), lookup)
        });
        // The build's level is raised as the device's other matches are.
        let build_match = build_level.map(|l| {
            if promoted && l == MatchLevel::Weak {
                MatchLevel::Possible
            } else {
                l
            }
        });
        if d.reachable {
            d.driver = driver;
            d.likely_boards = devices::likely_boards(&matches);
            d.boards = matches;
            d.build_match = build_match;
            out.devices.push(d);
        } else if !matches.is_empty() || build_match.is_some() {
            out.unreachable.push(UnreachableDevice {
                id: d.id,
                kind: d.kind,
                label: devices::label(None, None, &d.vid, &d.pid),
                vid: d.vid,
                pid: d.pid,
                bus: d.bus,
                device: d.device,
                boards: matches,
                action: devices::unreachable_action(),
                driver,
                build_match,
            });
        }
    }
    if build_usb.is_some() {
        // Best first; the sort is stable, so equal ones keep the USB order.
        out.devices
            .sort_by_key(|d| std::cmp::Reverse(d.build_match));
    }
    Ok(out)
}

/// The driver a device needs for its board's programmer, on Windows, and the
/// one it has: for the build's variant when there's a build, else for the
/// first variant of its best match. None when that programmer needs none,
/// or the device matches no board.
fn driver_status(
    d: &Device,
    matches: &[BoardMatch],
    boards: &[Candidate],
    build_variant: Option<&Variant>,
    lookup: DriverLookup,
) -> Option<DriverStatus> {
    let variant = match build_variant {
        Some(v) => v,
        None => {
            let best = matches.first()?;
            let selector = &best.variants.first()?.selector;
            boards
                .iter()
                .find(|b| b.id == best.board && b.custom == best.custom)?
                .variants
                .iter()
                .find(|v| &v.selector == selector)?
        }
    };
    let interface = drivers::interface_number(drivers::needed(variant)?, d.dfu_interface);
    Some(drivers::status(
        interface,
        &lookup(&d.vid, &d.pid, interface, d.composite),
    ))
}

/// A device a driver tool acts on, with what it needs.
pub struct Located {
    /// The device as scanned (its interfaces included).
    pub device: Device,
    /// Its board's name, for people and Device Manager.
    pub board_name: String,
    /// The driver it needs for that board and the one it has; None when
    /// the board's programmer needs none.
    pub driver: Option<DriverStatus>,
    /// Other connected devices with the same USB IDs, for people: Windows
    /// gives them the same driver.
    pub same_ids: Vec<String>,
}

/// Whether two device IDs name the same device: the same port and USB IDs
/// (the serial number may appear once a driver lets the engine read it).
pub fn same_device(a: &str, b: &str) -> bool {
    let key = |id: &str| id.rsplit_once(':').map(|(k, _)| k.to_string());
    key(a).is_some() && key(a) == key(b)
}

/// Finds the device `id` names (from list_devices, reachable or not), its
/// board (the build's with a build folder, else its best match) and its
/// driver status, for install_driver and uninstall_driver.
pub fn locate(
    id: &str,
    custom_boards_folder: Option<String>,
    build_folder: Option<String>,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
    drivers: DriverLookup,
) -> Result<Located, ToolError> {
    if !id.starts_with("usb:") {
        return Err(ToolError::request(format!(
            "device: {id:?} isn't a device ID. Use the id of a device from list_devices."
        )));
    }
    let scanned = scan()?;
    let Some(device) = scanned.iter().find(|d| same_device(&d.id, id)).cloned() else {
        return Err(
            ToolError::setup("The device isn't connected anymore.").with_action(
                crate::errors::ActionCode::ConnectBoard,
                "Connect the board, list the devices again and retry.",
            ),
        );
    };
    let same_ids: Vec<String> = scanned
        .iter()
        .filter(|d| d.vid == device.vid && d.pid == device.pid && !same_device(&d.id, id))
        .map(|d| d.label.clone())
        .collect();
    let custom = crate::custom_boards::load(custom_boards_folder.as_deref())?;
    let input = ListDevicesInput {
        custom_boards_folder,
        display_paths: None,
        build_folder,
    };
    let out = run_with_drivers(input, move || Ok(scanned), Some(drivers))?;
    let (boards, driver) = match out.devices.iter().find(|d| same_device(&d.id, id)) {
        Some(d) => (d.boards.clone(), d.driver.clone()),
        None => match out.unreachable.iter().find(|d| same_device(&d.id, id)) {
            Some(d) => (d.boards.clone(), d.driver.clone()),
            None => (Vec::new(), None),
        },
    };
    let Some(best) = boards.first() else {
        return Err(ToolError::request(format!(
            "device: {} is no board the engine knows (or not the build's board), so it doesn't know which driver it needs.",
            device.label
        )));
    };
    let known = crate::board::known_boards(&custom);
    let board_name = known
        .get(best.board.as_str())
        .map(|b| b.description.to_string())
        .unwrap_or_else(|| best.board.clone());
    Ok(Located {
        device,
        board_name,
        driver,
        same_ids,
    })
}

pub fn run(_state: &EngineState, input: ListDevicesInput) -> Result<ListDevicesOutput, ToolError> {
    run_with(input, devices::scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::tests::device;
    use serde_json::json;

    fn list(
        input: serde_json::Value,
        devices: Vec<Device>,
    ) -> Result<ListDevicesOutput, ToolError> {
        let input: ListDevicesInput = serde_json::from_value(input).unwrap();
        run_with(input, move || Ok(devices))
    }

    /// list_devices as on Windows, with `current` as every interface's
    /// driver, and the interfaces the lookup was asked about.
    fn list_on_windows(
        devices: Vec<Device>,
        current: Option<&str>,
    ) -> (ListDevicesOutput, Vec<(String, u8, bool)>) {
        let asked = std::cell::RefCell::new(Vec::new());
        let lookup = |v: &str, p: &str, i: u8, c: bool| {
            asked.borrow_mut().push((format!("{v}:{p}"), i, c));
            vec![current.map(String::from)]
        };
        let input: ListDevicesInput = serde_json::from_value(json!({})).unwrap();
        let out = run_with_drivers(input, move || Ok(devices), Some(&lookup)).unwrap();
        (out, asked.into_inner())
    }

    #[test]
    fn on_windows_ftdi_boards_need_winusb_on_their_first_interface() {
        let mut alhambra = device("0403", "6010", Some("Alhambra II v1.0A"), None, 2);
        alhambra.composite = true;
        let (out, asked) = list_on_windows(vec![alhambra.clone()], Some("ftdibus"));
        let driver = out.devices[0].driver.clone().unwrap();
        assert_eq!(driver.needed, "winusb");
        assert_eq!(driver.interface, 0);
        assert_eq!(driver.current.as_deref(), Some("ftdibus"));
        assert!(!driver.ready);
        assert_eq!(asked, [("0403:6010".to_string(), 0, true)]);
        let json = serde_json::to_value(&out.devices[0]).unwrap();
        assert_eq!(json["driver"]["ready"], false);

        let (out, _) = list_on_windows(vec![alhambra], Some("winusb"));
        assert!(out.devices[0].driver.as_ref().unwrap().ready);
    }

    #[test]
    fn on_windows_the_icesugar_needs_no_driver() {
        let icesugar = device("1d50", "602b", Some("CMSIS-DAP"), Some("0700ab"), 2);
        let (out, asked) = list_on_windows(vec![icesugar], Some("hidusb"));
        assert_eq!(out.devices[0].boards[0].board, "icesugar-1-5");
        assert_eq!(out.devices[0].driver, None);
        assert!(asked.is_empty());
    }

    #[test]
    fn unreachable_devices_have_an_id_and_their_driver() {
        let mut d = device("0403", "6010", None, None, 5);
        d.reachable = false;
        d.composite = true;
        let id = d.id.clone();
        let (out, _) = list_on_windows(vec![d], None);
        let u = &out.unreachable[0];
        assert_eq!(u.id, id);
        let driver = u.driver.as_ref().unwrap();
        assert_eq!((driver.current.as_deref(), driver.ready), (None, false));
    }

    #[test]
    fn elsewhere_no_driver_is_reported() {
        let out = list(
            json!({}),
            vec![device("0403", "6010", Some("Alhambra II v1.0A"), None, 2)],
        )
        .unwrap();
        assert_eq!(out.devices[0].driver.is_some(), cfg!(windows));
    }

    #[test]
    fn devices_come_with_the_boards_they_may_be() {
        let out = list(
            json!({}),
            vec![
                device(
                    "1209",
                    "5af0",
                    Some("OrangeCrab r0.2 DFU bootloader"),
                    None,
                    2,
                ),
                device("0403", "6014", Some("Single RS232-HS"), None, 3),
                device("1234", "5678", Some("A mouse"), None, 4),
            ],
        )
        .unwrap();
        let first = |i: usize| out.devices[i].boards.first().cloned();
        let bootloader = first(0).unwrap();
        // Two boards share the OrangeCrab bootloader's name: both named.
        assert_eq!(bootloader.board, "orangecrab-r02-25f");
        assert_eq!(bootloader.best_level(), MatchLevel::Strong);
        assert_eq!(out.devices[0].boards[1].board, "orangecrab-r02-85f");
        assert_eq!(
            bootloader.variants,
            [crate::devices::VariantMatch {
                selector: crate::catalog::Selector::new("bootloader", "flash"),
                level: MatchLevel::Strong,
            }]
        );
        // The FT232H adapter: the ButterStick's adapter variants, among others.
        let adapter = &out.devices[1].boards;
        let butterstick = adapter
            .iter()
            .find(|m| m.board == "butterstick-r10-2g-85k")
            .unwrap();
        assert_eq!(butterstick.best_level(), MatchLevel::Weak);
        assert_eq!(
            butterstick.variants[0].selector,
            crate::catalog::Selector::new("ft232h", "ram")
        );
        assert_eq!(butterstick.variants[0].level, MatchLevel::Weak);
        assert!(out.devices[2].boards.is_empty());
        assert!(out.unreachable.is_empty());
        // Labels for people, and the boards each device most likely is:
        // the strongest, never only weak ones.
        assert_eq!(
            out.devices[0].label,
            "OrangeCrab r0.2 DFU bootloader (1209:5af0)"
        );
        assert_eq!(
            out.devices[0].likely_boards,
            ["orangecrab-r02-25f", "orangecrab-r02-85f"]
        );
        // An FT232H with its default name: several boards use one, so they
        // are possible matches, and likely; the boards matching only its
        // USB IDs (weak) aren't.
        assert_eq!(
            out.devices[1].likely_boards,
            ["arice1", "fpga101", "vsdsquadron-fm"]
        );
        assert!(
            !out.devices[1]
                .likely_boards
                .contains(&"butterstick-r10-2g-85k".to_string())
        );
        assert!(out.devices[2].likely_boards.is_empty());
    }

    #[test]
    fn custom_boards_are_matched_too() {
        let mine = |id: &str| {
            json!({id: {
                "name": "My Alhambra", "fpga_id": "ice40hx4k-tq144-8k",
                "variants": [{
                    "selector": {"connection": "usb", "memory": "flash"},
                    "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                    "usb": {"vid": "0403", "pid": "6010", "product_regex": "^Alhambra"},
                    "tested": true
                }]
            }})
        };
        let alhambra = || vec![device("0403", "6010", Some("Alhambra II"), None, 3)];
        let (_t1, file) = crate::custom_boards::test_folder(&mine("mine"));
        let out = list(json!({"custom_boards_folder": file}), alhambra()).unwrap();
        let strong: Vec<_> = out.devices[0]
            .boards
            .iter()
            .filter(|m| m.best_level() == MatchLevel::Strong)
            .map(|m| (m.board.as_str(), m.custom))
            .collect();
        assert_eq!(strong, [("alhambra-ii", false), ("mine", true)]);
        // One with a catalog board's ID replaces it.
        let (_t2, file) = crate::custom_boards::test_folder(&mine("alhambra-ii"));
        let out = list(json!({"custom_boards_folder": file}), alhambra()).unwrap();
        let named: Vec<_> = out.devices[0]
            .boards
            .iter()
            .filter(|m| m.board == "alhambra-ii")
            .map(|m| m.custom)
            .collect();
        assert_eq!(named, [true]);
        // A bad list is a request error.
        let (_t3, bad) = crate::custom_boards::test_folder(
            &json!({"Bad ID": {"name": "x", "fpga_id": "x", "variants": []}}),
        );
        let e = list(json!({"custom_boards_folder": bad}), vec![]).unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
    }

    #[test]
    fn each_variant_has_its_own_level() {
        let config = |memory: &str, usb: serde_json::Value| {
            json!({
                "selector": {"connection": "usb", "memory": memory},
                "programmer": "iceprog",
                "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                "usb": usb,
                "tested": true
            })
        };
        // Variants with different filters, as a board file or the catalog
        // may have.
        let variants: Vec<crate::catalog::Variant> = serde_json::from_value(json!([
            config(
                "flash",
                json!({"vid": "0403", "pid": "6010", "product_regex": "^Dual RS232-HS"})
            ),
            config("ram", json!({"vid": "0403", "pid": "6010"})),
        ]))
        .unwrap();
        let boards = [Candidate {
            id: "mine",
            custom: true,
            variants: &variants,
        }];
        let d = device("0403", "6010", Some("Dual RS232-HS"), None, 3);
        let (matches, _) = devices::board_matches_and_promotion(&d, &boards);
        let mine = &matches[0];
        let rated: Vec<_> = mine
            .variants
            .iter()
            .map(|v| (v.selector.memory.as_str(), v.level))
            .collect();
        assert_eq!(
            rated,
            [("flash", MatchLevel::Possible), ("ram", MatchLevel::Weak)]
        );
    }

    #[test]
    fn devices_that_cant_be_opened_are_listed_apart_when_they_may_be_boards() {
        let mut ftdi = device("0403", "6010", None, None, 3);
        ftdi.reachable = false;
        let mut mouse = device("1234", "5678", None, None, 4);
        mouse.reachable = false;
        let out = list(json!({}), vec![ftdi, mouse]).unwrap();
        assert!(out.devices.is_empty());
        assert_eq!(out.unreachable.len(), 1);
        let u = &out.unreachable[0];
        assert_eq!((u.vid.as_str(), u.pid.as_str()), ("0403", "6010"));
        assert!(u.boards.iter().any(|m| m.board == "alhambra-ii"));
        assert!(u.boards.iter().all(|m| m.best_level() == MatchLevel::Weak));
        assert_eq!(u.action, devices::unreachable_action());
        let v = serde_json::to_value(&out).unwrap();
        // An id, for install_driver; program_board still refuses it.
        assert!(
            v["unreachable"][0]["id"]
                .as_str()
                .unwrap()
                .starts_with("usb:")
        );
        assert!(
            v["unreachable"][0]["boards"][0].get("match").is_none(),
            "per variant only"
        );
        assert_eq!(
            v["unreachable"][0]["boards"][0]["variants"][0]["match"],
            "weak"
        );
    }
}
