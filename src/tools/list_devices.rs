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
        the one it needs (WinUSB), on which USB interface, the one it has and whether it's ready; \
        on Linux, every device that fits a board has driver: its udev rule's state. Each device has \
        who, its name for people, and driver has texts for people: its state, and the question to \
        ask before install_driver (add) or uninstall_driver (remove). Give board, the user's \
        chosen board, to name devices by it, list its devices first and get status: whether \
        it's connected, in words for people (with build_folder, the build's board). When \
        custom_boards_folder has a problem, the devices are still listed, against the \
        catalog's boards only, and custom_boards_problem has the error.";
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
    /// request. Problems in any file don't stop list_devices: the devices
    /// are matched against the catalog's boards only, and
    /// custom_boards_problem has the error every other tool returns (with
    /// build_folder, they're a request error). Send the same folder to
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
    /// The board the user has chosen, by ID, such as the one selected in a
    /// frontend's toolbar (a catalog board, or one in
    /// custom_boards_folder). When given, a device that may be it is named
    /// as it in who and in the driver texts, and devices are listed in
    /// this order: those that may be this board, then those that may be
    /// other boards, then the rest (USB order within each). Without
    /// build_folder only: with it, devices are listed best match first.
    /// Omitted means no board.
    #[serde(default)]
    pub board: Option<String>,
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
    /// Only with board or build_folder: whether that board is connected, for
    /// people and hosts, so every frontend says it the same way. code is
    /// connected (one device may be it), several (more than one: ask the
    /// user which), unreachable (none can be opened, but one that can't has
    /// its USB IDs: action and the message say what makes it usable) or
    /// not_connected (none); message says it in a sentence naming the
    /// board, such as "The Alhambra II isn't connected. Connect it by USB,
    /// then try again."; hints are more lines to show under it, a line
    /// each (for not_connected: what is connected, and tips about the
    /// cable and power); action is the suggested action, as in errors,
    /// when there's something to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<BoardStatus>,
    /// When custom_boards_folder has a problem: the error every other board
    /// tool returns for it (a request error with a diagnostic for each
    /// problem, at file, line and column). list_devices still lists the
    /// devices then, matched against the catalog's boards only, so device
    /// scans keep working while the user fixes the folder. With
    /// build_folder, a problem is a request error, as in program_board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_boards_problem: Option<ToolError>,
    /// The USB serial ports the engine couldn't tie to a listed device,
    /// sorted by vendor ID, product ID, interface and name: such as a
    /// device that gives the serial port list another serial number than
    /// its USB listing, two adapters without serial numbers that look the
    /// same, or a device the engine can't open. Every USB serial port is
    /// either here or in a device's serial_ports, so a host can always let
    /// the user choose any of them (open_serial, without device). Omitted
    /// when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_serial_ports: Vec<crate::serial::OtherSerialPort>,
}

/// Whether the request's board is connected (ListDevicesOutput.status).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardStatus {
    /// connected, several, unreachable or not_connected.
    pub code: StatusCode,
    /// The status in a sentence for people, naming the board.
    pub message: String,
    /// More lines for people, shown under message a line each. Often
    /// empty.
    #[serde(default)]
    pub hints: Vec<String>,
    /// The suggested action, as in errors: connect_board when it isn't
    /// connected, or the unreachable device's own action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<crate::errors::Action>,
}

/// BoardStatus.code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusCode {
    /// One connected device may be the board.
    Connected,
    /// Several connected devices may be the board: ask the user which.
    Several,
    /// None can be opened, but a device that can't be opened has the
    /// board's USB IDs: it needs a driver or permissions first.
    Unreachable,
    /// No connected device may be the board.
    NotConnected,
}

/// The drivers Windows has on the present devices with some USB IDs, on an
/// interface (drivers::current_drivers; a fake in tests).
pub type DriverLookup<'a> = &'a dyn Fn(&str, &str, u8, bool) -> Vec<Option<String>>;

pub fn run_with(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ListDevicesOutput, ToolError> {
    let lookup = |v: &str, p: &str, i: u8, c: bool| drivers::current_drivers(v, p, i, c);
    // A folder with a problem counts as none (custom_boards_problem says
    // so), so the serial ports are found as the devices were matched.
    let custom_folder = load_custom(&input)?
        .1
        .is_none()
        .then(|| input.custom_boards_folder.clone())
        .flatten();
    let build_folder = input.build_folder.clone();
    let board = input.board.clone();
    let mut out = run_with_drivers(
        input,
        scan,
        cfg!(windows).then_some(&lookup as DriverLookup),
    )?;
    if cfg!(target_os = "linux") {
        add_udev_statuses(&mut out, &drivers::udev_rule);
    }
    add_serial_ports(
        &mut out,
        &crate::serial::usb_ports(),
        custom_folder.as_deref(),
        build_folder.as_deref(),
        board.as_deref(),
    )?;
    Ok(out)
}

/// The request's custom boards, and the folder's problem when it has one:
/// list_devices then matches against the catalog's boards only, instead of
/// failing as every other board tool does (docs/requirements.md, Custom
/// definitions: device scans keep working while the user fixes the
/// folder). With a build folder, a problem stays a request error: the
/// build's board may be one of them, as for program_board.
fn load_custom(
    input: &ListDevicesInput,
) -> Result<(crate::board::CustomBoards, Option<ToolError>), ToolError> {
    match crate::custom_boards::load(input.custom_boards_folder.as_deref()) {
        Ok(custom) => Ok((custom, None)),
        Err(e)
            if input.build_folder.is_none() && e.category == crate::errors::Category::Request =>
        {
            log::info!(
                "list_devices: the custom boards folder has a problem: {}",
                e.message
            );
            Ok((Default::default(), Some(e)))
        }
        Err(e) => Err(e),
    }
}

/// Gives each device its serial ports, from the system's USB serial ports
/// (crate::serial::usb_ports; made up in tests), and the port of its
/// board's UART (Device.serial_port). The board is the build's, else
/// `board` when the device may be it, else the device's only likely board.
pub fn add_serial_ports(
    out: &mut ListDevicesOutput,
    ports: &[(crate::serial::DeviceKey, crate::serial::DevicePort)],
    custom_boards_folder: Option<&str>,
    build_folder: Option<&str>,
    board: Option<&str>,
) -> Result<(), ToolError> {
    let custom = crate::custom_boards::load(custom_boards_folder)?;
    let known = crate::board::known_boards(&custom);
    let build_board = match build_folder {
        Some(folder) => super::program_board::read_build(folder)?.1.board.board_id,
        None => None,
    };
    let found = crate::serial::ports_of_devices(&out.devices, ports);
    out.other_serial_ports = crate::serial::other_ports(&found, ports);
    for (d, device_ports) in out.devices.iter_mut().zip(found) {
        let board_id = build_board.clone().or_else(|| {
            board
                .filter(|b| d.boards.iter().any(|m| m.board == *b))
                .map(str::to_string)
                .or_else(|| match d.likely_boards.as_slice() {
                    [only] => Some(only.clone()),
                    _ => None,
                })
        });
        // The board's first UART by name that says its USB interface.
        let interface = board_id
            .as_deref()
            .and_then(|id| known.get(id))
            .and_then(|b| {
                let mut uarts: Vec<_> = b
                    .resources
                    .iter()
                    .filter_map(|(name, r)| match r {
                        crate::resources::Resource::Uart(u) => Some((name, u.usb_interface)),
                        _ => None,
                    })
                    .collect();
                uarts.sort_by(|a, b| a.0.cmp(b.0));
                uarts.into_iter().find_map(|(_, i)| i)
            });
        d.serial_port = crate::serial::uart_port(&device_ports, interface);
        // The programmer's interface: the board's variant this device fits
        // (else its first), by what its programmer opens (drivers::needed).
        let programmer = board_id
            .as_deref()
            .and_then(|id| known.get(id))
            .and_then(|b| {
                let variant = b
                    .variants
                    .iter()
                    .find(|v| devices::matches(d, &v.usb))
                    .or_else(|| b.variants.first())?;
                drivers::needed(variant).map(|n| drivers::interface_number(n, d.dfu_interface))
            });
        let mut device_ports = device_ports;
        for p in &mut device_ports {
            p.role = if d.serial_port.as_deref() == Some(p.port.as_str()) {
                Some(crate::serial::PortRole::Uart)
            } else if p.interface.is_some() && p.interface == programmer {
                Some(crate::serial::PortRole::Programmer)
            } else {
                None
            };
            p.description = crate::serial::port_description(p.role, &d.who);
        }
        d.serial_ports = device_ports;
    }
    Ok(())
}

/// Logix Engine's udev rule for some USB IDs, by its file name, when it's
/// there (drivers::udev_rule; a fake in tests).
pub type RuleLookup<'a> = &'a dyn Fn(&str, &str) -> Option<String>;

/// On Linux: every device that fits a board gets its udev rule status
/// (drivers::udev_status), ready when the engine can open it, with its texts
/// for people (named by the device's who).
pub fn add_udev_statuses(out: &mut ListDevicesOutput, rule: RuleLookup) {
    for d in out.devices.iter_mut().filter(|d| !d.boards.is_empty()) {
        let mut status = drivers::udev_status(true, rule(&d.vid, &d.pid));
        crate::driver_texts::fill(&mut status, &d.who);
        d.driver = Some(status);
    }
    for d in &mut out.unreachable {
        let mut status = drivers::udev_status(false, rule(&d.vid, &d.pid));
        crate::driver_texts::fill(&mut status, &d.who);
        d.driver = Some(status);
    }
}

/// As run_with, with Windows' drivers from `drivers` (None: not on Windows,
/// so no driver statuses).
pub fn run_with_drivers(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
    drivers: Option<DriverLookup>,
) -> Result<ListDevicesOutput, ToolError> {
    let (custom, custom_boards_problem) = load_custom(&input)?;
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
    let (build_board, build_variant): (Option<String>, Option<Variant>) = match &input.build_folder
    {
        Some(folder) => {
            let built = super::program_board::read_build(folder)?.1.board;
            let upload = super::program_board::upload_board(&built, &custom, folder)?;
            (built.board_id.clone(), Some(upload.variant))
        }
        None => (None, None),
    };
    let build_usb = build_variant.as_ref().map(|v| v.usb.clone());
    let mut out = ListDevicesOutput {
        devices: Vec::new(),
        unreachable: Vec::new(),
        status: None,
        custom_boards_problem,
        other_serial_ports: Vec::new(),
    };
    // Every connected device, for the status's hints: the ones that can be
    // opened, and those that can't whose USB IDs fit a board.
    let mut seen = Seen::default();
    for mut d in scan()? {
        let (matches, promoted) = devices::board_matches_and_promotion(&d, &boards);
        if d.reachable {
            seen.reachable.push(d.label.clone());
        } else if !matches.is_empty() {
            seen.locked.push(devices::label(None, None, &d.vid, &d.pid));
        }
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
                who: String::new(),
                vid: d.vid,
                pid: d.pid,
                bus: d.bus,
                device: d.device,
                boards: matches,
                action: devices::unreachable_action(),
                fix: crate::driver_texts::fix(),
                note: String::new(),
                rank: 0,
                driver,
                build_match,
            });
        }
    }
    if build_usb.is_some() {
        // Best first; the sort is stable, so equal ones keep the USB order.
        out.devices
            .sort_by_key(|d| std::cmp::Reverse(d.build_match));
    } else if let Some(board) = input.board.as_deref() {
        // The chosen board's devices first, then other boards', then the
        // rest (stable: USB order within each).
        out.devices.sort_by_key(|d| group(&d.boards, Some(board)));
        out.unreachable
            .sort_by_key(|d| group(&d.boards, Some(board)));
    }
    // Names and texts for people.
    let name = |id: &str| {
        known
            .get(id)
            .map(|b| b.description.to_string())
            .unwrap_or_else(|| id.to_string())
    };
    let chosen = input.board.as_deref();
    // The board the user means: the chosen one, else the build's.
    let meant = chosen.or(build_board.as_deref());
    for d in &mut out.devices {
        d.who = who(&d.boards, chosen, &d.label, &name);
        d.other_boards = other_boards(&d.boards, &d.likely_boards, meant);
        if let Some(driver) = &mut d.driver {
            crate::driver_texts::fill(driver, &d.who);
        }
    }
    for d in &mut out.unreachable {
        d.who = who(&d.boards, chosen, &d.label, &name);
        d.note = unreachable_note(&d.boards, meant, &d.fix, &name);
        if let Some(driver) = &mut d.driver {
            crate::driver_texts::fill(driver, &d.who);
        }
    }
    rank(&mut out, meant);
    out.status = match (&build_usb, meant) {
        (Some(_), _) => Some(board_status(&out, None, meant, &seen, &name)),
        (None, Some(board)) => Some(board_status(&out, Some(board), meant, &seen, &name)),
        (None, None) => None,
    };
    Ok(out)
}

/// The connected devices seen by a scan, for people: the labels of those
/// the engine can open, and of those it can't whose USB IDs fit a board.
#[derive(Default)]
struct Seen {
    reachable: Vec<String>,
    locked: Vec<String>,
}

/// A device's group in the one-list order: 0 when it may be `board`, 1
/// when it may be other boards, 2 otherwise.
fn group(boards: &[BoardMatch], board: Option<&str>) -> u8 {
    if board.is_some_and(|b| boards.iter().any(|m| m.board == b)) {
        0
    } else if !boards.is_empty() {
        1
    } else {
        2
    }
}

/// Gives every device and unreachable device its place in one list of
/// both: by group (the board meant first), then devices before unreachable
/// ones, each in its list's order.
fn rank(out: &mut ListDevicesOutput, meant: Option<&str>) {
    let mut keys: Vec<(u8, usize, usize)> = out
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| (group(&d.boards, meant), 0, i))
        .chain(
            out.unreachable
                .iter()
                .enumerate()
                .map(|(i, d)| (group(&d.boards, meant), 1, i)),
        )
        .collect();
    keys.sort();
    for (place, (_, list, i)) in keys.into_iter().enumerate() {
        if list == 0 {
            out.devices[i].rank = place;
        } else {
            out.unreachable[i].rank = place;
        }
    }
}

/// The boards worth naming after a device's likely boards: the rest, or
/// with a strong match only the meant board, when the device may be it
/// (the boards dozens of devices share would only be noise).
pub fn other_boards(boards: &[BoardMatch], likely: &[String], meant: Option<&str>) -> Vec<String> {
    let strong = boards.iter().any(|b| b.best_level() == MatchLevel::Strong);
    boards
        .iter()
        .map(|b| b.board.clone())
        .filter(|b| !likely.contains(b))
        .filter(|b| !strong || meant == Some(b.as_str()))
        .collect()
}

/// What a device that can't be opened may be, and what to do, for people.
pub fn unreachable_note(
    boards: &[BoardMatch],
    meant: Option<&str>,
    fix: &str,
    name: &dyn Fn(&str) -> String,
) -> String {
    let first = if let Some(b) = meant.filter(|m| boards.iter().any(|x| x.board == *m)) {
        format!("This may be your {}: it has the same USB IDs.", name(b))
    } else {
        match boards {
            [only] => format!(
                "This may be an FPGA board, the {}: it has its USB IDs.",
                name(&only.board)
            ),
            _ => format!(
                "This may be an FPGA board: {} boards have its USB IDs.",
                boards.len()
            ),
        }
    };
    if fix.is_empty() {
        first
    } else {
        format!("{first} {fix}")
    }
}

/// Whether the board is connected: with `board`, among every device; else
/// (a build folder) among the devices listed, which fit the build already.
fn board_status(
    out: &ListDevicesOutput,
    board: Option<&str>,
    meant: Option<&str>,
    seen: &Seen,
    name: &dyn Fn(&str) -> String,
) -> BoardStatus {
    let fits = |boards: &[BoardMatch]| board.is_none_or(|b| boards.iter().any(|m| m.board == b));
    let devices = out.devices.iter().filter(|d| fits(&d.boards)).count();
    let locked = out.unreachable.iter().find(|d| fits(&d.boards));
    let board_name = meant.map(name).unwrap_or_else(|| "board".into());
    status_for(devices, locked, &board_name, seen)
}

/// The status for `devices` connected devices that may be the board (named
/// `board_name`, such as "Alhambra II"), and the first device that can't be
/// opened but may be it.
fn status_for(
    devices: usize,
    locked: Option<&UnreachableDevice>,
    board_name: &str,
    seen: &Seen,
) -> BoardStatus {
    let status = |code, message: String, hints, action| BoardStatus {
        code,
        message,
        hints,
        action,
    };
    match (devices, locked) {
        (1, _) => status(
            StatusCode::Connected,
            format!("The {board_name} is connected."),
            Vec::new(),
            None,
        ),
        (0, Some(u)) => status(
            StatusCode::Unreachable,
            format!("The {board_name} was found but can't be opened. {}", u.fix)
                .trim_end()
                .to_string(),
            Vec::new(),
            Some(u.action.clone()),
        ),
        (0, None) => status(
            StatusCode::NotConnected,
            format!("The {board_name} isn't connected. Connect it by USB, then try again."),
            connection_hints(seen),
            Some(crate::errors::Action {
                code: crate::errors::ActionCode::ConnectBoard,
                description: format!("Connect the {board_name} by USB, then retry."),
            }),
        ),
        (n, _) => status(
            StatusCode::Several,
            format!("{n} connected devices could be the {board_name}. Choose the one to program."),
            Vec::new(),
            None,
        ),
    }
}

/// What's connected, with tips about the cable and power, a line each,
/// for when the board isn't found.
fn connection_hints(seen: &Seen) -> Vec<String> {
    let locked = seen
        .locked
        .iter()
        .map(|l| format!("  • {l} can't be opened. {}", crate::driver_texts::fix()));
    if seen.reachable.is_empty() {
        return std::iter::once("No USB devices are connected.".to_string())
            .chain(locked)
            .chain([
                "Check that the board is plugged in and powered, and try another cable (some USB cables only charge and don't carry data).".to_string(),
            ])
            .collect();
    }
    std::iter::once("Connected USB devices:".to_string())
        .chain(seen.reachable.iter().map(|l| format!("  • {l}")))
        .chain(locked)
        .chain([
            "If your board isn't listed, check the cable and power.".to_string(),
            "If it's listed, check that the selected board matches it.".to_string(),
        ])
        .collect()
}

/// A device for people in a sentence: "the <board>" when it may be the
/// chosen board, or when it may be only one board, else its label. Never
/// the first of several boards with its USB IDs, which named a Tang Nano
/// 9k "the Alchitry Au" (the user's report, 2026-11-14).
pub fn who(
    boards: &[BoardMatch],
    chosen: Option<&str>,
    label: &str,
    name: &dyn Fn(&str) -> String,
) -> String {
    if let Some(board) = chosen.filter(|c| boards.iter().any(|b| b.board == *c)) {
        return format!("the {}", name(board));
    }
    match boards {
        [only] => format!("the {}", name(&only.board)),
        _ => label.to_string(),
    }
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
    /// Its name in Device Manager: its board's, when only one board fits it
    /// (or the build's board), else the device's own label.
    pub name: String,
    /// The same for people in a sentence: "the Alhambra II", or the label.
    /// Never the first of many boards with its USB IDs, which named a Tang
    /// Nano 9k "the Alchitry Au" (the user's report, 2026-11-14).
    pub who: String,
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
    board: Option<String>,
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
        board: board.clone(),
    };
    let mut out = run_with_drivers(input, move || Ok(scanned), cfg!(windows).then_some(drivers))?;
    if cfg!(target_os = "linux") {
        add_udev_statuses(&mut out, &drivers::udev_rule);
    }
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
    let board_name = |id: &str| {
        known
            .get(id)
            .map(|b| b.description.to_string())
            .unwrap_or_else(|| id.to_string())
    };
    let name = if boards.len() == 1 {
        board_name(&best.board)
    } else {
        device.label.clone()
    };
    let who = who(&boards, board.as_deref(), &device.label, &board_name);
    Ok(Located {
        device,
        name,
        who,
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

    /// The Alhambra II's two ports: interface 1 its UART (the catalog's
    /// usb_interface), interface 0 its programmer's (FTDI's first channel).
    #[test]
    fn serial_ports_get_their_roles() {
        let mut d = device("0403", "6010", Some("Alhambra II v1.0A"), Some("A1"), 3);
        d.likely_boards = vec!["alhambra-ii".into()];
        d.who = "the Alhambra II".into();
        let mut out = ListDevicesOutput {
            devices: vec![d],
            unreachable: Vec::new(),
            status: None,
            custom_boards_problem: None,
            other_serial_ports: Vec::new(),
        };
        let key = crate::serial::DeviceKey {
            vid: 0x0403,
            pid: 0x6010,
            serial_number: Some("A1".into()),
        };
        let port = |name: &str, interface: u8| crate::serial::DevicePort {
            port: name.into(),
            interface: Some(interface),
            role: None,
            description: String::new(),
        };
        let ports = vec![
            (key.clone(), port("/dev/ttyUSB0", 0)),
            (key, port("/dev/ttyUSB1", 1)),
        ];
        add_serial_ports(&mut out, &ports, None, None, None).unwrap();
        let d = &out.devices[0];
        assert_eq!(d.serial_port.as_deref(), Some("/dev/ttyUSB1"));
        let roles: Vec<_> = d
            .serial_ports
            .iter()
            .map(|p| (p.port.as_str(), p.role, p.description.as_str()))
            .collect();
        assert_eq!(
            roles,
            [
                (
                    "/dev/ttyUSB0",
                    Some(crate::serial::PortRole::Programmer),
                    "The Alhambra II's programming port"
                ),
                (
                    "/dev/ttyUSB1",
                    Some(crate::serial::PortRole::Uart),
                    "The Alhambra II's serial port"
                ),
            ]
        );
    }
    /// The Tang Nano 9K on macOS (2026-11-25): its BL702 gives the USB
    /// listing one serial number and the serial port list another. Its two
    /// ports are its own anyway, the catalog's usb_interface picks the UART,
    /// and a CH340 adapter's port, seen twice by two lookalike adapters, is
    /// in other_serial_ports.
    #[test]
    fn mismatched_serial_numbers_still_find_the_ports() {
        let mut nano = device(
            "0403",
            "6010",
            Some("JTAG Debugger"),
            Some("FactoryAIOT Pro\"3DUal"),
            3,
        );
        nano.likely_boards = vec!["sipeed-tang-nano-9k".into()];
        nano.who = "the Sipeed Tang Nano 9k".into();
        let mut out = ListDevicesOutput {
            devices: vec![
                nano,
                device("1a86", "7523", Some("USB Serial"), None, 4),
                device("1a86", "7523", Some("USB Serial"), None, 5),
            ],
            unreachable: Vec::new(),
            status: None,
            custom_boards_problem: None,
            other_serial_ports: Vec::new(),
        };
        let key = |vid, pid, serial: Option<&str>| crate::serial::DeviceKey {
            vid,
            pid,
            serial_number: serial.map(Into::into),
        };
        let port = |name: &str, interface: u8, description: &str| crate::serial::DevicePort {
            port: name.into(),
            interface: Some(interface),
            role: None,
            description: description.into(),
        };
        let ports = vec![
            (
                key(0x0403, 0x6010, Some("FactoryAIOT Pro")),
                port("/dev/cu.usbserial-1100", 0, ""),
            ),
            (
                key(0x0403, 0x6010, Some("FactoryAIOT Pro")),
                port("/dev/cu.usbserial-1101", 1, ""),
            ),
            (
                key(0x1a86, 0x7523, None),
                port(
                    "/dev/cu.wchusbserial-1",
                    0,
                    "A serial port of USB Serial (1a86:7523)",
                ),
            ),
        ];
        add_serial_ports(&mut out, &ports, None, None, None).unwrap();
        let d = &out.devices[0];
        assert_eq!(d.serial_port.as_deref(), Some("/dev/cu.usbserial-1101"));
        assert_eq!(d.serial_ports.len(), 2);
        assert!(out.devices[1].serial_ports.is_empty());
        assert_eq!(
            out.other_serial_ports,
            [crate::serial::OtherSerialPort {
                port: "/dev/cu.wchusbserial-1".into(),
                vid: "1a86".into(),
                pid: "7523".into(),
                serial_number: None,
                interface: Some(0),
                description: "A serial port of USB Serial (1a86:7523)".into(),
            }]
        );
    }

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
        assert_eq!(driver.interface, Some(0));
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
    fn on_macos_no_driver_is_reported() {
        let out = list(
            json!({}),
            vec![device("0403", "6010", Some("Alhambra II v1.0A"), None, 2)],
        )
        .unwrap();
        assert_eq!(
            out.devices[0].driver.is_some(),
            cfg!(any(windows, target_os = "linux"))
        );
    }

    // On Linux every device that fits a board gets its udev rule status,
    // HID programmers' too (the iCESugar's iCELink), and no other device.
    #[test]
    fn on_linux_boards_get_their_udev_rule_status() {
        let mut locked = device("0403", "6010", None, None, 5);
        locked.reachable = false;
        let devices = vec![
            device("1d50", "602b", Some("CMSIS-DAP"), Some("0700ab"), 2),
            locked,
            device("05ac", "8262", Some("Keyboard"), None, 3),
        ];
        let input: ListDevicesInput = serde_json::from_value(json!({})).unwrap();
        let mut out = run_with_drivers(input, move || Ok(devices), None).unwrap();
        let rule = |v: &str, p: &str| (v == "1d50").then(|| drivers::udev_rule_name(v, p));
        add_udev_statuses(&mut out, &rule);
        let icesugar = out.devices.iter().find(|d| d.vid == "1d50").unwrap();
        let driver = icesugar.driver.clone().unwrap();
        assert_eq!(
            (
                driver.needed.as_str(),
                driver.current.as_deref(),
                driver.ready
            ),
            ("udev-rule", Some("70-logix-engine-1d50-602b.rules"), true)
        );
        // With its texts for people, named by the device.
        assert_eq!(icesugar.who, "the iCESugar v1.5");
        assert!(driver.can_remove && !driver.can_add);
        assert_eq!(
            driver.remove.unwrap().lead,
            "This removes Logix Engine's udev rule for the iCESugar v1.5."
        );
        // Unreachable: a fix for people, and the rule to add.
        assert_eq!(out.unreachable[0].fix, crate::driver_texts::fix());
        assert_eq!(
            out.unreachable[0].driver.as_ref().unwrap().tag.as_deref(),
            Some("no udev rule yet")
        );
        let keyboard = out.devices.iter().find(|d| d.vid == "05ac").unwrap();
        assert_eq!(keyboard.driver, None);
        let driver = out.unreachable[0].driver.clone().unwrap();
        assert_eq!(
            (
                driver.needed.as_str(),
                driver.interface,
                driver.current,
                driver.ready
            ),
            ("udev-rule", None, None, false)
        );
        let json = serde_json::to_value(&out.unreachable[0]).unwrap();
        assert!(json["driver"].get("interface").is_none(), "{json}");
    }

    // The chosen board names a device that may be it, and its devices come
    // first, then other boards', then the rest.
    #[test]
    fn the_chosen_board_names_its_devices_and_comes_first() {
        let devices = || {
            vec![
                device("05ac", "8262", Some("Keyboard"), None, 1),
                device("0403", "6010", Some("Alhambra II v1.0A"), None, 2),
                device("1d50", "602b", Some("CMSIS-DAP"), Some("0700ab"), 3),
            ]
        };
        let out = list(json!({}), devices()).unwrap();
        let order: Vec<&str> = out.devices.iter().map(|d| d.vid.as_str()).collect();
        // Without a board: the scan's order. The iCESugar's iCELink is one
        // board: its name.
        assert_eq!(order, ["05ac", "0403", "1d50"]);
        let icesugar = out.devices.iter().find(|d| d.vid == "1d50").unwrap();
        assert_eq!(icesugar.who, "the iCESugar v1.5");

        let out = list(json!({"board": "alhambra-ii"}), devices()).unwrap();
        let order: Vec<&str> = out.devices.iter().map(|d| d.vid.as_str()).collect();
        assert_eq!(order, ["0403", "1d50", "05ac"]);
        assert_eq!(out.devices[0].who, "the Alhambra II");
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
        // A folder with a problem: the devices are still listed, against
        // the catalog only, with the problem every other tool returns.
        let (_t3, bad) = crate::custom_boards::test_folder(
            &json!({"Bad ID": {"name": "x", "fpga_id": "x", "variants": []}}),
        );
        let out = list(json!({"custom_boards_folder": bad}), alhambra()).unwrap();
        let problem = out.custom_boards_problem.as_ref().unwrap();
        assert_eq!(problem.category, crate::errors::Category::Request);
        assert!(!problem.diagnostics.is_empty());
        assert_eq!(out.devices[0].likely_boards, ["alhambra-ii"]);
        assert!(out.devices[0].boards.iter().all(|m| !m.custom));
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["custom_boards_problem"]["category"], "request");
        // Without a problem, the field isn't there.
        let (_t4, file) = crate::custom_boards::test_folder(&mine("mine"));
        let v =
            serde_json::to_value(list(json!({"custom_boards_folder": file}), alhambra()).unwrap())
                .unwrap();
        assert!(v.get("custom_boards_problem").is_none(), "{v}");
        // The folder must still be an absolute path: that's the request's
        // own mistake, not the folder's.
        let e = list(json!({"custom_boards_folder": "boards"}), alhambra());
        assert!(e.unwrap().custom_boards_problem.is_some());
    }

    #[test]
    fn the_status_says_whether_the_board_is_connected() {
        let alhambra = || device("0403", "6010", Some("Alhambra II v1.0A"), Some("A1"), 2);
        let keyboard = || device("05ac", "8262", Some("Keyboard"), None, 1);
        let status = |board: &str, devices: Vec<Device>| {
            list(json!({"board": board}), devices)
                .unwrap()
                .status
                .unwrap()
        };
        // Without a board or a build: none.
        assert!(list(json!({}), vec![alhambra()]).unwrap().status.is_none());

        let s = status("alhambra-ii", vec![keyboard(), alhambra()]);
        assert_eq!(s.code, StatusCode::Connected);
        assert_eq!(s.message, "The Alhambra II is connected.");
        assert!(s.hints.is_empty() && s.action.is_none());

        let mut second = alhambra();
        second.id = "usb:1-3:0403:6010:A2".into();
        let s = status("alhambra-ii", vec![alhambra(), second]);
        assert_eq!(s.code, StatusCode::Several);
        assert_eq!(
            s.message,
            "2 connected devices could be the Alhambra II. Choose the one to program."
        );

        let s = status("alhambra-ii", vec![keyboard()]);
        assert_eq!(s.code, StatusCode::NotConnected);
        assert_eq!(
            s.message,
            "The Alhambra II isn't connected. Connect it by USB, then try again."
        );
        assert_eq!(
            s.action.as_ref().map(|a| a.code),
            Some(crate::errors::ActionCode::ConnectBoard)
        );
        assert_eq!(
            s.hints,
            [
                "Connected USB devices:",
                "  • Keyboard (05ac:8262)",
                "If your board isn't listed, check the cable and power.",
                "If it's listed, check that the selected board matches it.",
            ]
        );
        let s = status("alhambra-ii", vec![]);
        assert_eq!(
            s.hints,
            [
                "No USB devices are connected.",
                "Check that the board is plugged in and powered, and try another cable (some USB cables only charge and don't carry data).",
            ]
        );

        // Found but can't be opened: its fix, and its action.
        let mut locked = alhambra();
        locked.reachable = false;
        let s = status("alhambra-ii", vec![keyboard(), locked.clone()]);
        assert_eq!(s.code, StatusCode::Unreachable);
        assert_eq!(
            s.message,
            format!(
                "The Alhambra II was found but can't be opened. {}",
                crate::driver_texts::fix()
            )
        );
        assert_eq!(s.action, Some(devices::unreachable_action()));
        // Another board's devices don't count, but the hints list the
        // device that can't be opened.
        let s = status("icesugar-1-5", vec![keyboard(), locked]);
        assert_eq!(s.code, StatusCode::NotConnected);
        assert_eq!(
            s.hints[2],
            format!(
                "  • USB device (0403:6010) can't be opened. {}",
                crate::driver_texts::fix()
            )
        );
    }

    #[test]
    fn devices_that_cant_be_opened_get_a_note() {
        let name = |id: &str| match id {
            "alhambra-ii" => "Alhambra II".to_string(),
            other => other.to_string(),
        };
        let m = |b: &str| BoardMatch {
            board: b.into(),
            custom: false,
            variants: Vec::new(),
        };
        let fix = "Add a udev rule for it, so it can be programmed and its name read.";
        assert_eq!(
            unreachable_note(
                &[m("alchitry-au"), m("alhambra-ii")],
                Some("alhambra-ii"),
                fix,
                &name
            ),
            format!("This may be your Alhambra II: it has the same USB IDs. {fix}")
        );
        assert_eq!(
            unreachable_note(&[m("alhambra-ii")], None, fix, &name),
            format!("This may be an FPGA board, the Alhambra II: it has its USB IDs. {fix}")
        );
        assert_eq!(
            unreachable_note(
                &[m("alchitry-au"), m("alhambra-ii")],
                Some("icestick"),
                fix,
                &name
            ),
            format!("This may be an FPGA board: 2 boards have its USB IDs. {fix}")
        );
        // In the result, by the request's board.
        let mut d = device("0403", "6010", None, None, 3);
        d.reachable = false;
        let out = list(json!({"board": "alhambra-ii"}), vec![d]).unwrap();
        assert!(
            out.unreachable[0]
                .note
                .starts_with("This may be your Alhambra II: it has the same USB IDs."),
            "{}",
            out.unreachable[0].note
        );
    }

    #[test]
    fn other_boards_leave_out_the_noise_of_a_strong_match() {
        let alhambra = || device("0403", "6010", Some("Alhambra II v1.0A"), None, 2);
        let out = list(json!({}), vec![alhambra()]).unwrap();
        let d = &out.devices[0];
        assert_eq!(d.likely_boards, ["alhambra-ii"]);
        // A strong match: only the meant board would be named, and none is.
        assert!(d.other_boards.is_empty(), "{:?}", d.other_boards);
        // A weaker board it may be, when it's the one meant.
        let weaker = d
            .boards
            .iter()
            .map(|b| b.board.clone())
            .find(|b| !d.likely_boards.contains(b))
            .expect("a weaker board");
        let out = list(json!({"board": weaker}), vec![alhambra()]).unwrap();
        assert_eq!(out.devices[0].other_boards, [weaker]);
        // No strong match: every board that isn't likely.
        let generic = device("0403", "6010", Some("Dual RS232-HS"), None, 2);
        let out = list(json!({}), vec![generic]).unwrap();
        let d = &out.devices[0];
        assert!(
            !d.boards
                .iter()
                .any(|b| b.best_level() == MatchLevel::Strong)
        );
        let rest: Vec<&String> = d
            .boards
            .iter()
            .map(|b| &b.board)
            .filter(|b| !d.likely_boards.contains(b))
            .collect();
        assert_eq!(d.other_boards.iter().collect::<Vec<_>>(), rest);
        assert!(!d.other_boards.is_empty());
    }

    #[test]
    fn rank_orders_devices_and_unreachable_ones_as_one_list() {
        let mut locked = device("0403", "6010", None, None, 4);
        locked.reachable = false;
        let devices = vec![
            device("05ac", "8262", Some("Keyboard"), None, 1),
            device("1d50", "602b", Some("CMSIS-DAP"), Some("0700ab"), 2),
            locked,
        ];
        let ranks = |board: serde_json::Value| {
            let out = list(board, devices.clone()).unwrap();
            let mut all: Vec<(usize, String)> = out
                .devices
                .iter()
                .map(|d| (d.rank, d.vid.clone()))
                .chain(
                    out.unreachable
                        .iter()
                        .map(|d| (d.rank, format!("locked {}", d.vid))),
                )
                .collect();
            all.sort();
            all.into_iter().map(|(_, v)| v).collect::<Vec<_>>()
        };
        // The board's devices first (the one that can't be opened too),
        // then other boards', then the rest.
        assert_eq!(
            ranks(json!({"board": "alhambra-ii"})),
            ["locked 0403", "1d50", "05ac"]
        );
        // Without a board: boards first, devices before unreachable ones.
        assert_eq!(ranks(json!({})), ["1d50", "locked 0403", "05ac"]);
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
