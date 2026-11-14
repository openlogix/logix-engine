//! USB device scanning, device IDs and board matching (docs/requirements.md,
//! "Device selection"). The engine uses libusb, built in, like the
//! programmers do, so bus and device numbers mean the same to both.
//!
//! Matching runs on every request against the boards it is given (the
//! catalog and the request's own definitions), never against precomputed
//! data, since hosts can define boards at any time.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{UsbFilter, Variant};
use crate::errors::{Action, ActionCode, ToolError};

/// A connected device that the engine can open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// The device's ID, to pass to program_board. Opaque: don't parse it.
    /// It names the device on this computer for a short time: it stays
    /// valid while the device stays on the same USB port, so list the
    /// devices again before asking the user to choose.
    pub id: String,
    /// The kind of device: usb.
    pub kind: DeviceKind,
    /// The device for people: its product name (else its manufacturer's
    /// name, else "USB device"), then its manufacturer and USB IDs, such
    /// as "Alhambra II v1.0A (AlhambraBits, 0403:6010)".
    pub label: String,
    /// USB vendor ID, four lowercase hex digits.
    pub vid: String,
    /// USB product ID, four lowercase hex digits.
    pub pid: String,
    /// The manufacturer name the device reports, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// The product name the device reports, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// The serial number the device reports, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// The device's release number (USB bcdDevice), four lowercase hex
    /// digits, for matching only: not reported to hosts.
    #[serde(skip)]
    #[schemars(skip)]
    pub release: Option<String>,
    /// USB bus number.
    pub bus: u8,
    /// USB device number on its bus. It changes when the device is plugged
    /// in again.
    pub device: u8,
    /// The boards whose USB filters this device matches, best first. Empty
    /// for a device that is no known board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boards: Vec<BoardMatch>,
    /// The boards this device most likely is, best first: those of boards
    /// at its strongest match level, unless that is weak (many boards fit,
    /// so none is likely). Empty when no board is likely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub likely_boards: Vec<String>,
    /// How well the device matches the given build's board in the build's
    /// variant: strong, possible or weak, as in boards. Present only when
    /// list_devices was given a build_folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_match: Option<MatchLevel>,
    /// Where the device is plugged in, such as 1-2.3 (bus, then ports).
    #[serde(skip)]
    #[schemars(skip)]
    pub location: String,
    /// Whether the engine could open the device and read its names. Devices
    /// it can't open need a driver or permissions first.
    #[serde(skip)]
    #[schemars(skip)]
    pub reachable: bool,
    /// On Windows only: the driver the device needs for its board's
    /// programmer (the build's board when list_devices was given a
    /// build_folder, else its best match) and the one it has, for a board
    /// whose programmer needs one. ready false means install_driver first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<crate::drivers::DriverStatus>,
    /// The device's DFU interface (USB class 0xFE, subclass 1), if it has
    /// one: where dfu-util needs its driver.
    #[serde(skip)]
    #[schemars(skip)]
    pub dfu_interface: Option<u8>,
    /// Whether it has several interfaces, each its own device on Windows.
    #[serde(skip)]
    #[schemars(skip)]
    pub composite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    /// A USB device.
    Usb,
}

/// A board a device may be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardMatch {
    /// The board's ID: a catalog ID, or a custom board's ID when custom is
    /// true.
    pub board: String,
    /// True when the board is from the request's custom boards folder rather
    /// than the catalog's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
    /// The board's variants this device fits, in the board's order, each
    /// with its own match level. Use the level of the variant you mean,
    /// such as the build's.
    pub variants: Vec<VariantMatch>,
}

impl BoardMatch {
    /// The strongest match level of the board's variants, for sorting.
    pub fn best_level(&self) -> MatchLevel {
        self.variants
            .iter()
            .map(|v| v.level)
            .max()
            .unwrap_or(MatchLevel::Weak)
    }
}

/// A variant of a board that a device fits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantMatch {
    /// The variant's selector, such as {"connection": "usb", "memory":
    /// "flash"}, as board_variant gives it in requests.
    pub selector: crate::catalog::Selector,
    /// How well the device matches the board in this variant: strong,
    /// possible or weak. See MatchLevel.
    #[serde(rename = "match")]
    pub level: MatchLevel,
}

/// How well a device matches a board, by what it means for the user, not
/// by its cause. The criteria are internal to the engine and change
/// without notice, so hosts must not infer why a device matched; they may
/// only show the level and compare levels (weak < possible < strong). No
/// level proves which board is connected.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum MatchLevel {
    /// The device fits the board, but so do many other boards.
    Weak,
    /// The device could be the board, but a few other boards look the
    /// same.
    Possible,
    /// The device is very likely the board.
    Strong,
}

/// When a device matches boards only by their USB IDs, and at most this
/// many, they are possible matches rather than weak ones.
const FEW_BOARDS: usize = 3;

/// Product names that USB chips report out of the factory, which many boards
/// keep. A match on one of these says little about the board.
pub const FACTORY_DEFAULT_NAMES: &[&str] = &[
    "Dual RS232-HS",            // FTDI FT2232H
    "Single RS232-HS",          // FTDI FT232H
    "Quad RS232-HS",            // FTDI FT4232H
    "USB <-> Serial Converter", // FTDI FT2232D
    "USB <-> Serial",
    "FT232R USB UART",
    "FT231X USB UART",
    "FT230X Basic UART",
    "DAPLink CMSIS-DAP",
    "CMSIS-DAP",
    // Sipeed's debugger firmware (the Tang Nano 9K's, and maybe others'):
    // nothing in it names a board or even the vendor.
    "JTAG Debugger",
];

/// Whether a product name is a USB chip's factory default.
pub fn is_factory_default_name(name: &str) -> bool {
    let name = name.trim();
    FACTORY_DEFAULT_NAMES
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
}

/// The level of a device's match to a variant's filter, on its own
/// (`board_matches` then considers all of the device's matches): strong for
/// a matched serial pattern or a board-specific name, possible for a
/// matched factory default name, weak for USB IDs only or a device whose
/// names can't be read. Internal criteria: hosts only see the level.
pub fn match_level(device: &Device, filter: &UsbFilter) -> MatchLevel {
    if !device.reachable {
        return MatchLevel::Weak;
    }
    if filter.serial_regex.is_some() {
        return MatchLevel::Strong;
    }
    match (&filter.product_regex, &device.product) {
        (Some(_), Some(name)) if is_factory_default_name(name) => MatchLevel::Possible,
        (Some(_), Some(_)) => MatchLevel::Strong,
        _ => MatchLevel::Weak,
    }
}

/// A device the engine sees but can't open, so it can't read its names or
/// program it: it needs a driver or permissions first. Listed only when its
/// USB IDs fit a board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnreachableDevice {
    /// The device's ID, as for devices: for install_driver. Opaque: don't
    /// parse it.
    pub id: String,
    /// The kind of device: usb.
    pub kind: DeviceKind,
    /// The device for people, such as "USB device (0403:6010)": its names
    /// can't be read.
    pub label: String,
    /// USB vendor ID, four lowercase hex digits.
    pub vid: String,
    /// USB product ID, four lowercase hex digits.
    pub pid: String,
    /// USB bus number.
    pub bus: u8,
    /// USB device number on its bus.
    pub device: u8,
    /// The boards whose USB IDs it has. Every match is weak, since its
    /// names can't be read.
    pub boards: Vec<BoardMatch>,
    /// What makes the device usable, such as fix_device_permissions.
    pub action: Action,
    /// On Windows only: the driver it needs for its board's programmer and
    /// the one it has, as for devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<crate::drivers::DriverStatus>,
    /// How well it matches the given build's board: always weak, since its
    /// names can't be read. Present only when list_devices was given a
    /// build_folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_match: Option<MatchLevel>,
}

/// A board to match devices against: its ID, whether it came from the
/// request, and its variants.
pub struct Candidate<'a> {
    pub id: &'a str,
    pub custom: bool,
    pub variants: &'a [Variant],
}

/// Lists the connected USB devices, the ones the engine can't open
/// included, sorted by vendor, product, bus and device.
pub fn scan() -> Result<Vec<Device>, ToolError> {
    let list = rusb::devices().map_err(|e| {
        ToolError::setup(format!("Can't list the USB devices: {e}")).with_action(
            ActionCode::FixDevicePermissions,
            "Check that this program may access USB devices.",
        )
    })?;
    let timeout = Duration::from_millis(500);
    let mut out = Vec::new();
    for d in list.iter() {
        let Ok(desc) = d.device_descriptor() else {
            continue;
        };
        let (mut manufacturer, mut product, mut serial_number) = (None, None, None);
        let mut reachable = false;
        if let Ok(handle) = d.open() {
            reachable = true;
            let lang = handle
                .read_languages(timeout)
                .ok()
                .and_then(|l| l.first().copied());
            if let Some(lang) = lang {
                let clean = |s: Option<String>| {
                    s.map(|s| crate::text::to_text(&s))
                        .filter(|s| !s.is_empty())
                };
                manufacturer = clean(handle.read_manufacturer_string(lang, &desc, timeout).ok());
                product = clean(handle.read_product_string(lang, &desc, timeout).ok());
                serial_number = clean(handle.read_serial_number_string(lang, &desc, timeout).ok());
            }
        }
        // The port path stays the same when the device is plugged in again
        // in the same port; the device number doesn't.
        let location = match d.port_numbers() {
            Ok(ports) if !ports.is_empty() => format!(
                "{}-{}",
                d.bus_number(),
                ports
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            ),
            _ => format!("{}-a{}", d.bus_number(), d.address()),
        };
        let mut device = Device::usb(
            format!("{:04x}", desc.vendor_id()),
            format!("{:04x}", desc.product_id()),
            manufacturer,
            product,
            serial_number,
            d.bus_number(),
            d.address(),
            location,
            reachable,
        );
        device.release = raw_release(&d);
        // Read from the descriptors the system already has, without opening
        // the device, so devices without a usable driver have them too.
        if let Ok(config) = d
            .active_config_descriptor()
            .or_else(|_| d.config_descriptor(0))
        {
            device.composite = config.num_interfaces() > 1;
            device.dfu_interface = config.interfaces().find_map(|i| {
                i.descriptors()
                    .find(|x| x.class_code() == 0xfe && x.sub_class_code() == 1)
                    .map(|x| x.interface_number())
            });
        }
        out.push(device);
    }
    out.sort_by(|a, b| (&a.vid, &a.pid, a.bus, a.device).cmp(&(&b.vid, &b.pid, b.bus, b.device)));
    Ok(out)
}

impl Device {
    /// A USB device, with its ID made from where it is and what it is.
    #[allow(clippy::too_many_arguments)]
    pub fn usb(
        vid: String,
        pid: String,
        manufacturer: Option<String>,
        product: Option<String>,
        serial_number: Option<String>,
        bus: u8,
        device: u8,
        location: String,
        reachable: bool,
    ) -> Device {
        let id = format!(
            "usb:{location}:{vid}:{pid}:{}",
            serial_number.as_deref().unwrap_or("")
        );
        Device {
            id,
            kind: DeviceKind::Usb,
            label: label(manufacturer.as_deref(), product.as_deref(), &vid, &pid),
            vid,
            pid,
            manufacturer,
            product,
            serial_number,
            release: None,
            bus,
            device,
            boards: Vec::new(),
            likely_boards: Vec::new(),
            build_match: None,
            location,
            reachable,
            driver: None,
            dfu_interface: None,
            composite: false,
        }
    }
}

/// A device for people: its product name, else its manufacturer's, else
/// "USB device", then its manufacturer (when not already the name) and USB
/// IDs, such as "Alhambra II v1.0A (AlhambraBits, 0403:6010)".
pub fn label(manufacturer: Option<&str>, product: Option<&str>, vid: &str, pid: &str) -> String {
    fn nonempty(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    let (manufacturer, product) = (nonempty(manufacturer), nonempty(product));
    let name = product.or(manufacturer).unwrap_or("USB device");
    let mut details: Vec<String> = Vec::new();
    if let Some(m) = manufacturer.filter(|m| Some(*m) != Some(name)) {
        details.push(m.to_string());
    }
    details.push(format!("{vid}:{pid}"));
    format!("{name} ({})", details.join(", "))
}

/// The boards a device most likely is, from its matches (best first):
/// those at its strongest level, unless that is weak, since weak matches
/// fit many boards.
pub fn likely_boards(matches: &[BoardMatch]) -> Vec<String> {
    let Some(best) = matches.iter().map(BoardMatch::best_level).max() else {
        return Vec::new();
    };
    if best == MatchLevel::Weak {
        return Vec::new();
    }
    matches
        .iter()
        .filter(|m| m.best_level() == best)
        .map(|m| m.board.clone())
        .collect()
}

/// Whether a device has the filter's USB IDs (case doesn't matter).
/// Whether the device's location is its USB port path, such as 1-2.3, as
/// dfu-util's --path takes it. Without port numbers, the location is the
/// bus and address instead.
pub fn has_port_path(device: &Device) -> bool {
    device.location.split_once('-').is_some_and(|(_, ports)| {
        !ports.is_empty() && ports.bytes().all(|c| c.is_ascii_digit() || c == b'.')
    })
}

/// Whether a device has a filter's USB IDs and, when the filter names one,
/// its release. All of these come from the device descriptor, so they're
/// known even for a device the engine can't open.
pub fn same_ids(device: &Device, filter: &UsbFilter) -> bool {
    device.vid.eq_ignore_ascii_case(&filter.vid)
        && device.pid.eq_ignore_ascii_case(&filter.pid)
        && match (&filter.release, &device.release) {
            (None, _) => true,
            (Some(want), Some(has)) => want.eq_ignore_ascii_case(has),
            (Some(_), None) => false,
        }
}

/// The device's bcdDevice, exactly as reported. rusb only offers it decoded
/// as a decimal version, which loses values that aren't decimal digits,
/// such as 0a00.
fn raw_release(d: &rusb::Device<rusb::GlobalContext>) -> Option<String> {
    let mut desc = std::mem::MaybeUninit::<rusb::ffi::libusb_device_descriptor>::uninit();
    // SAFETY: d is a valid device for the duration of the call, and libusb
    // fills the whole descriptor when it returns 0.
    let rc = unsafe { rusb::ffi::libusb_get_device_descriptor(d.as_raw(), desc.as_mut_ptr()) };
    (rc == 0).then(|| format!("{:04x}", unsafe { desc.assume_init() }.bcdDevice))
}

/// Whether a device passes a board's USB filter: same IDs (and release), a product name
/// matching product_regex and a serial number matching serial_regex, when
/// the filter has them.
pub fn matches(device: &Device, filter: &UsbFilter) -> bool {
    let fits = |re: &Option<String>, value: &Option<String>| match re {
        None => true,
        Some(re) => match (regex::Regex::new(re), value) {
            (Ok(re), Some(v)) => re.is_match(v),
            _ => false,
        },
    };
    same_ids(device, filter)
        && fits(&filter.product_regex, &device.product)
        && fits(&filter.serial_regex, &device.serial_number)
}

/// The boards a device may be, best first: by their best variant's match
/// level, then catalog boards before custom ones, then by board ID. Each
/// variant has its own level. For a device the engine can't open, only the
/// USB IDs are compared (its names are unknown), so every match is weak.
/// When an open device matches only by USB IDs, and only a few boards, those
/// matches are possible rather than weak.
pub fn board_matches(device: &Device, boards: &[Candidate]) -> Vec<BoardMatch> {
    board_matches_and_promotion(device, boards).0
}

/// As board_matches, and whether the device's weak matches were raised to
/// possible (few boards share its USB IDs), so a level computed apart, such
/// as a build's, can be raised the same way.
pub fn board_matches_and_promotion(
    device: &Device,
    boards: &[Candidate],
) -> (Vec<BoardMatch>, bool) {
    let mut out = Vec::new();
    for b in boards {
        let mut variants = Vec::new();
        for c in b.variants {
            let hit = if device.reachable {
                matches(device, &c.usb)
            } else {
                same_ids(device, &c.usb)
            };
            if hit {
                variants.push(VariantMatch {
                    selector: c.selector.clone(),
                    level: match_level(device, &c.usb),
                });
            }
        }
        if !variants.is_empty() {
            out.push(BoardMatch {
                board: b.id.to_string(),
                custom: b.custom,
                variants,
            });
        }
    }
    let promoted = device.reachable
        && !out.is_empty()
        && out.len() <= FEW_BOARDS
        && out.iter().all(|m| m.best_level() == MatchLevel::Weak);
    if promoted {
        for m in &mut out {
            for v in &mut m.variants {
                v.level = MatchLevel::Possible;
            }
        }
    }
    out.sort_by(|a, b| {
        (b.best_level(), a.custom, &a.board).cmp(&(a.best_level(), b.custom, &b.board))
    });
    (out, promoted)
}

/// What makes a device the engine can't open usable, on this platform.
pub fn unreachable_action() -> Action {
    if cfg!(windows) {
        Action {
            code: ActionCode::InstallDrivers,
            description: "Install the driver this device needs with install_driver (ask the user first), then list the devices again."
                .into(),
        }
    } else {
        Action {
            code: ActionCode::FixDevicePermissions,
            description: "Allow this user to access the USB device, then list the devices again."
                .into(),
        }
    }
}

/// Finds the device a device ID names among the connected devices. The ID
/// must come from list_devices on this computer; the device must still be on
/// the same port, with the same USB IDs and serial number. Errors: a request
/// error for an ID that isn't one, a setup error with connect_board when the
/// device is gone or another one is in its place, and a setup error with
/// fix_device_permissions (or install_drivers) when it can't be opened.
pub fn find(devices: &[Device], id: &str, board_name: &str) -> Result<Device, ToolError> {
    let parts: Option<Vec<&str>> = id
        .strip_prefix("usb:")
        .map(|rest| rest.splitn(4, ':').collect());
    let Some([location, vid, pid, _serial]) = parts.as_deref() else {
        return Err(ToolError::request(format!(
            "device: {id:?} isn't a device ID. Use the id of a device from list_devices."
        )));
    };
    let gone = || {
        ToolError::setup(format!(
            "The chosen device isn't connected anymore. Is the {board_name} still plugged in?"
        ))
        .with_action(
            ActionCode::ConnectBoard,
            format!("Connect the {board_name} by USB, list the devices again and choose it."),
        )
    };
    let Some(d) = devices.iter().find(|d| d.location == *location) else {
        return Err(gone());
    };
    if d.vid != *vid || d.pid != *pid || d.id != id {
        return Err(gone());
    }
    if !d.reachable {
        let action = unreachable_action();
        return Err(
            ToolError::setup(format!("The {board_name} was found but can't be opened."))
                .with_action(action.code, action.description),
        );
    }
    Ok(d.clone())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn labels_for_people() {
        assert_eq!(
            label(
                Some("AlhambraBits"),
                Some("Alhambra II v1.0A"),
                "0403",
                "6010"
            ),
            "Alhambra II v1.0A (AlhambraBits, 0403:6010)"
        );
        assert_eq!(
            label(Some("FTDI"), None, "0403", "6010"),
            "FTDI (0403:6010)"
        );
        assert_eq!(
            label(None, Some(" "), "0403", "6010"),
            "USB device (0403:6010)"
        );
        assert_eq!(label(Some("X"), Some("X"), "1d50", "6130"), "X (1d50:6130)");
    }

    #[test]
    fn likely_boards_are_the_strongest_unless_weak() {
        let m = |board: &str, level: MatchLevel| BoardMatch {
            board: board.into(),
            custom: false,
            variants: vec![VariantMatch {
                selector: crate::catalog::Selector::new("usb", "flash"),
                level,
            }],
        };
        use MatchLevel::*;
        assert_eq!(
            likely_boards(&[m("a", Strong), m("b", Possible), m("c", Strong)]),
            ["a", "c"]
        );
        assert_eq!(likely_boards(&[m("a", Possible), m("b", Weak)]), ["a"]);
        assert!(likely_boards(&[m("a", Weak), m("b", Weak)]).is_empty());
        assert!(likely_boards(&[]).is_empty());
    }

    pub fn device(
        vid: &str,
        pid: &str,
        product: Option<&str>,
        serial: Option<&str>,
        dev: u8,
    ) -> Device {
        Device::usb(
            vid.into(),
            pid.into(),
            None,
            product.map(Into::into),
            serial.map(Into::into),
            1,
            dev,
            format!("1-{dev}"),
            true,
        )
    }

    fn filter(re: Option<&str>) -> UsbFilter {
        UsbFilter {
            vid: "0403".into(),
            pid: "6010".into(),
            product_regex: re.map(Into::into),
            serial_regex: None,
            release: None,
        }
    }

    #[test]
    fn matching() {
        let alhambra = device("0403", "6010", Some("Alhambra II v2.0"), Some("A1"), 3);
        let icestick = device("0403", "6010", Some("Dual RS232-HS"), None, 4);
        let f = filter(Some("^Alhambra II.*"));
        assert!(matches(&alhambra, &f));
        assert!(!matches(&icestick, &f));
        assert!(matches(&icestick, &filter(None)));
        assert!(matches(
            &device("0403", "6010", None, None, 5),
            &filter(None)
        ));
        assert!(
            !matches(&device("0403", "6010", None, None, 5), &f),
            "no name, no match"
        );
        assert!(!matches(
            &device("0403", "6014", Some("Alhambra II"), None, 6),
            &f
        ));
        // A release in the filter: only devices that report it, whether or
        // not the engine can open them.
        let bl702 = UsbFilter {
            release: Some("0500".into()),
            ..filter(None)
        };
        let with_release = |r: Option<&str>, reachable: bool| {
            let mut d = device("0403", "6010", Some("JTAG Debugger"), None, 7);
            d.release = r.map(Into::into);
            d.reachable = reachable;
            d
        };
        assert!(matches(&with_release(Some("0500"), true), &bl702));
        assert!(!matches(&with_release(Some("0700"), true), &bl702));
        assert!(
            !matches(&with_release(None, true), &bl702),
            "unknown release"
        );
        assert!(same_ids(&with_release(Some("0500"), false), &bl702));
        assert!(!same_ids(&with_release(Some("0700"), false), &bl702));
        assert!(matches(&with_release(Some("0700"), true), &filter(None)));
        // Matching data only: hosts don't see it.
        let json = serde_json::to_value(with_release(Some("0500"), true)).unwrap();
        assert!(json.get("release").is_none(), "{json}");

        // iCESugar boards share IDs and product name; the serial tells them apart.
        let nano_filter = UsbFilter {
            vid: "1d50".into(),
            pid: "602b".into(),
            product_regex: Some("^DAPLink CMSIS-DAP".into()),
            serial_regex: Some("^0720".into()),
            release: None,
        };
        let pro = device(
            "1d50",
            "602b",
            Some("DAPLink CMSIS-DAP"),
            Some("0710000100380027"),
            7,
        );
        let nano = device(
            "1d50",
            "602b",
            Some("DAPLink CMSIS-DAP"),
            Some("0720000100380027"),
            8,
        );
        let no_serial = device("1d50", "602b", Some("DAPLink CMSIS-DAP"), None, 9);
        assert!(matches(&nano, &nano_filter));
        assert!(!matches(&pro, &nano_filter));
        assert!(!matches(&no_serial, &nano_filter), "no serial, no match");
    }

    #[test]
    fn ids_name_where_and_what_the_device_is() {
        let d = device("0403", "6010", Some("Alhambra II"), Some("A:1"), 3);
        assert_eq!(d.id, "usb:1-3:0403:6010:A:1");
        assert_eq!(
            device("0403", "6010", None, None, 3).id,
            "usb:1-3:0403:6010:"
        );
    }

    #[test]
    fn finding_a_device_by_id() {
        let a = device("0403", "6010", Some("Alhambra II"), Some("A1"), 3);
        let b = device("0403", "6010", Some("Alhambra II"), Some("B2"), 4);
        let list = [a.clone(), b.clone()];
        assert_eq!(find(&list, &b.id, "Alhambra II").unwrap(), b);

        // Plugged in again in the same port: a new device number, same ID.
        let mut again = a.clone();
        again.device = 9;
        assert_eq!(find(&[again.clone()], &a.id, "Alhambra II").unwrap(), again);

        // Gone, or another device in its port.
        let err = find(std::slice::from_ref(&b), &a.id, "Alhambra II").unwrap_err();
        assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);
        let mut other = device("0403", "6010", Some("Alhambra II"), Some("C3"), 3);
        other.location = a.location.clone();
        let err = find(&[other], &a.id, "Alhambra II").unwrap_err();
        assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);

        // Not an ID.
        for bad in ["", "1-3", "usb:1-3", "usb:1-3:0403"] {
            let err = find(&list, bad, "Alhambra II").unwrap_err();
            assert_eq!(err.category, crate::errors::Category::Request, "{bad}");
        }

        // Can't be opened.
        let mut locked = a.clone();
        locked.reachable = false;
        let err = find(&[locked], &a.id, "Alhambra II").unwrap_err();
        assert_eq!(err.category, crate::errors::Category::Setup);
        assert_eq!(err.action.unwrap().code, unreachable_action().code);
    }

    #[test]
    fn matching_boards_with_levels() {
        let catalog = crate::catalog::get();
        let boards: Vec<Candidate> = catalog
            .boards
            .iter()
            .map(|(id, b)| Candidate {
                id,
                custom: false,
                variants: &b.variants,
            })
            .collect();
        let ids = |m: &[BoardMatch]| m.iter().map(|m| m.board.clone()).collect::<Vec<_>>();

        let alhambra = device("0403", "6010", Some("Alhambra II v1.0A"), Some("FT1"), 3);
        let m = board_matches(&alhambra, &boards);
        assert_eq!(m[0].board, "alhambra-ii");
        assert_eq!(m[0].best_level(), MatchLevel::Strong);
        assert!(m[1..].iter().all(|m| m.best_level() == MatchLevel::Weak));
        assert!(
            ids(&m).contains(&"cmod-a7-35t".to_string()),
            "a filter with only USB IDs"
        );
        assert!(!ids(&m).contains(&"alchitry-au".to_string()), "named");
        assert!(!ids(&m).contains(&"icezum".to_string()), "another name");

        // A real Tang Nano 9K: its BL702 debugger reports "JTAG Debugger",
        // a generic name, and release 0500, as on every BL702 Tang board, so
        // the four are possible matches, before the weak ones.
        let mut nano9k = device(
            "0403",
            "6010",
            Some("JTAG Debugger"),
            Some("FactoryAIOT Pro\"3DUal"),
            4,
        );
        nano9k.release = Some("0500".into());
        let m = board_matches(&nano9k, &boards);
        let possible: Vec<_> = m
            .iter()
            .take_while(|m| m.best_level() == MatchLevel::Possible)
            .map(|m| m.board.as_str())
            .collect();
        assert_eq!(
            possible,
            [
                "sipeed-tang-nano-1k",
                "sipeed-tang-nano-4k",
                "sipeed-tang-nano-9k",
                "sipeed-tang-primer-20k-dock",
                "sipeed-tang-primer-20k-lite"
            ]
        );
        assert!(m[5..].iter().all(|m| m.best_level() == MatchLevel::Weak));
        // A real FT2232H (release 0700) is none of the BL702 boards, even
        // when the engine can't open it.
        let mut ft2232h = device("0403", "6010", Some("Alhambra II v1.0A"), None, 5);
        ft2232h.release = Some("0700".into());
        for reachable in [true, false] {
            ft2232h.reachable = reachable;
            let m = board_matches(&ft2232h, &boards);
            assert!(!m.is_empty());
            for bl702 in [
                "sipeed-tang-nano-1k",
                "sipeed-tang-nano-4k",
                "sipeed-tang-nano-9k",
                "sipeed-tang-primer-20k-dock",
                "sipeed-tang-primer-20k-lite",
            ] {
                assert!(!ids(&m).contains(&bl702.to_string()), "{bl702}");
            }
            assert!(
                ids(&m).contains(&"sipeed-tang-nano-20k".to_string()),
                "no release"
            );
        }

        // Both iCESugar-Pro variants, and not the other iCESugar boards.
        let pro = device(
            "1d50",
            "602b",
            Some("DAPLink CMSIS-DAP"),
            Some("0710000100380027"),
            7,
        );
        let m = board_matches(&pro, &boards);
        assert_eq!(ids(&m), ["icesugar-pro"]);
        assert_eq!(m[0].variants.len(), 2);
        assert_eq!(m[0].best_level(), MatchLevel::Strong, "a serial pattern");

        // A factory default name: possible for the boards that expect it,
        // before the weak ones; a board-specific name in the same pattern
        // is strong.
        let ftdi = device("0403", "6010", Some("Dual RS232-HS"), None, 4);
        let m = board_matches(&ftdi, &boards);
        let go = m.iter().find(|m| m.board == "go-board").unwrap();
        assert_eq!(go.best_level(), MatchLevel::Possible);
        assert_eq!(m[0].best_level(), MatchLevel::Possible);
        assert_eq!(m.last().unwrap().best_level(), MatchLevel::Weak);
        let icebreaker = device("0403", "6010", Some("iCEBreaker V1.0e"), None, 5);
        let m = board_matches(&icebreaker, &boards);
        assert_eq!(
            (m[0].board.as_str(), m[0].best_level()),
            ("icebreaker", MatchLevel::Strong)
        );

        // Alchitry boards by their exact names (from Alchitry's loader):
        // the Au+ and the Cu V2 aren't the Au or the Cu.
        for (name, board) in [
            ("Alchitry Au", "alchitry-au"),
            ("Alchitry Au+", "alchitry-au-plus"),
            ("Alchitry Cu", "alchitry-cu"),
        ] {
            let m = board_matches(&device("0403", "6010", Some(name), None, 6), &boards);
            assert_eq!(
                (m[0].board.as_str(), m[0].best_level()),
                (board, MatchLevel::Strong)
            );
            assert!(
                m[1..].iter().all(|m| m.best_level() == MatchLevel::Weak),
                "{name}"
            );
            assert!(
                !ids(&m)
                    .iter()
                    .any(|b| b.starts_with("alchitry") && b != board)
            );
        }
        let cu2 = device("0403", "6010", Some("Alchitry Cu V2"), None, 6);
        assert!(
            !ids(&board_matches(&cu2, &boards))
                .iter()
                .any(|b| b.starts_with("alchitry"))
        );

        // A ULX3S names its FPGA size (fujprog's names); the FT231X's
        // factory name is possibly any ULX3S size, or an Icepi Zero, which
        // keeps that name.
        let ulx3s = device(
            "0403",
            "6015",
            Some("ULX3S FPGA 85K v3.0.8"),
            Some("120001"),
            6,
        );
        let m = board_matches(&ulx3s, &boards);
        assert_eq!(
            (m[0].board.as_str(), m[0].best_level()),
            ("ulx3s-85f", MatchLevel::Strong)
        );
        assert!(!ids(&m).contains(&"ulx3s-12f".to_string()));
        let blank = device("0403", "6015", Some("FT231X USB UART"), None, 6);
        let m = board_matches(&blank, &boards);
        let possible: Vec<_> = m
            .iter()
            .filter(|m| m.best_level() == MatchLevel::Possible)
            .map(|m| m.board.as_str())
            .collect();
        assert_eq!(
            possible,
            [
                "icepi-zero",
                "ulx3s-12f",
                "ulx3s-25f",
                "ulx3s-45f",
                "ulx3s-85f"
            ]
        );

        // Can't be opened: every board with these USB IDs, weak, even when
        // only a few boards have them.
        let mut locked = pro.clone();
        locked.reachable = false;
        let m = board_matches(&locked, &boards);
        assert_eq!(ids(&m), ["icesugar-1-5", "icesugar-nano", "icesugar-pro"]);
        assert!(m.iter().all(|m| m.best_level() == MatchLevel::Weak));

        assert!(board_matches(&device("1234", "5678", None, None, 1), &boards).is_empty());

        // USB IDs only, shared by a few boards: possible; by many: weak.
        let few: Vec<UsbFilter> = (0..FEW_BOARDS).map(|_| filter(None)).collect();
        let variants: Vec<Vec<Variant>> = few
            .iter()
            .map(|f| {
                vec![Variant {
                    usb: f.clone(),
                    ..catalog.boards["alhambra-ii"].variants[0].clone()
                }]
            })
            .collect();
        let names: Vec<String> = (0..FEW_BOARDS + 1).map(|i| format!("b{i}")).collect();
        let candidates = |n: usize| -> Vec<Candidate> {
            (0..n)
                .map(|i| Candidate {
                    id: &names[i],
                    custom: true,
                    variants: &variants[i % variants.len()],
                })
                .collect()
        };
        let plain = device("0403", "6010", Some("Something"), None, 6);
        let (m, promoted) = board_matches_and_promotion(&plain, &candidates(FEW_BOARDS));
        assert!(promoted);
        assert!(m.iter().all(|m| m.best_level() == MatchLevel::Possible));
        let (m, promoted) = board_matches_and_promotion(&plain, &candidates(FEW_BOARDS + 1));
        assert!(!promoted);
        assert!(m.iter().all(|m| m.best_level() == MatchLevel::Weak));
    }

    #[test]
    fn factory_default_names() {
        assert!(is_factory_default_name("Dual RS232-HS"));
        assert!(is_factory_default_name(" dual rs232-hs "));
        assert!(!is_factory_default_name("Alhambra II v1.0A"));
        assert!(!is_factory_default_name("Dual RS232-HS v2"));
    }

    #[test]
    fn scanning_works_here() {
        // Just checks libusb starts; the list depends on the machine.
        let devices = scan().unwrap();
        for d in &devices {
            assert_eq!(d.vid.len(), 4);
            assert!(d.id.starts_with("usb:"));
            let release = d.release.as_deref().unwrap();
            assert!(release.len() == 4 && release.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
}
