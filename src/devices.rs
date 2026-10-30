//! USB device scanning and matching (docs/requirements.md, "Device
//! selection"). The engine uses libusb, built in, like the programmers do, so
//! bus and device numbers mean the same to both.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::UsbFilter;
use crate::errors::{ActionCode, ToolError};

/// A connected device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// The kind of device: usb.
    pub kind: DeviceKind,
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
    /// USB bus number.
    pub bus: u8,
    /// USB device number on its bus.
    pub device: u8,
    /// Whether the device matches the given board. Present only when a board
    /// was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches_board: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    /// A USB device.
    Usb,
}

/// Lists the connected USB devices, sorted by vendor, product, bus and
/// device. Names that can't be read, such as when the device can't be
/// opened, are left out.
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
        if let Ok(handle) = d.open() {
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
        out.push(Device {
            kind: DeviceKind::Usb,
            vid: format!("{:04x}", desc.vendor_id()),
            pid: format!("{:04x}", desc.product_id()),
            manufacturer,
            product,
            serial_number,
            bus: d.bus_number(),
            device: d.address(),
            matches_board: None,
        });
    }
    out.sort_by(|a, b| (&a.vid, &a.pid, a.bus, a.device).cmp(&(&b.vid, &b.pid, b.bus, b.device)));
    Ok(out)
}

/// Whether a device passes a board's USB filter: same IDs (case doesn't
/// matter), and a product name matching product_regex when there is one.
pub fn matches(device: &Device, filter: &UsbFilter) -> bool {
    if !device.vid.eq_ignore_ascii_case(&filter.vid)
        || !device.pid.eq_ignore_ascii_case(&filter.pid)
    {
        return false;
    }
    match &filter.product_regex {
        None => true,
        Some(re) => match (regex::Regex::new(re), &device.product) {
            (Ok(re), Some(product)) => re.is_match(product),
            _ => false,
        },
    }
}

/// Picks the one device to program: matching the filter, and the serial
/// number when given. No match and several matches are setup errors with
/// connect_board and select_device; the latter lists the candidates.
pub fn select(
    devices: &[Device],
    filter: &UsbFilter,
    serial_number: Option<&str>,
    board_name: &str,
) -> Result<Device, ToolError> {
    let candidates: Vec<&Device> = devices
        .iter()
        .filter(|d| matches(d, filter))
        .filter(|d| serial_number.is_none_or(|s| d.serial_number.as_deref() == Some(s)))
        .collect();
    match candidates.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let what = match serial_number {
                Some(s) => format!("No {board_name} with serial number {s} is connected."),
                None => format!("No {board_name} is connected."),
            };
            Err(ToolError::setup(what).with_action(
                ActionCode::ConnectBoard,
                format!("Connect the {board_name} by USB, then retry."),
            ))
        }
        many => {
            let mut err = ToolError::setup(format!("Found {} matching devices.", many.len()))
                .with_action(
                    ActionCode::SelectDevice,
                    "Choose one device and retry with its serial number.",
                );
            let list: Vec<serde_json::Value> = many
                .iter()
                .map(|d| serde_json::to_value(d).unwrap_or_default())
                .collect();
            let mut data = serde_json::Map::new();
            data.insert("candidates".into(), list.into());
            err.data = Some(data);
            Err(err)
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn device(
        vid: &str,
        pid: &str,
        product: Option<&str>,
        serial: Option<&str>,
        dev: u8,
    ) -> Device {
        Device {
            kind: DeviceKind::Usb,
            vid: vid.into(),
            pid: pid.into(),
            manufacturer: None,
            product: product.map(Into::into),
            serial_number: serial.map(Into::into),
            bus: 1,
            device: dev,
            matches_board: None,
        }
    }

    fn filter(re: Option<&str>) -> UsbFilter {
        UsbFilter {
            vid: "0403".into(),
            pid: "6010".into(),
            product_regex: re.map(Into::into),
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
        assert!(matches(
            &device("0403", "6010", Some("Alhambra II"), None, 6),
            &UsbFilter {
                vid: "0403".into(),
                pid: "6010".into(),
                product_regex: None,
            }
        ));
    }

    #[test]
    fn selection() {
        let f = filter(Some("^Alhambra II"));
        let a = device("0403", "6010", Some("Alhambra II"), Some("A1"), 3);
        let b = device("0403", "6010", Some("Alhambra II"), Some("B2"), 4);
        let other = device("0403", "6010", Some("Dual RS232-HS"), None, 5);

        assert_eq!(
            select(&[a.clone(), other.clone()], &f, None, "Alhambra II").unwrap(),
            a
        );

        let err = select(std::slice::from_ref(&other), &f, None, "Alhambra II").unwrap_err();
        assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);
        assert!(err.message.contains("Alhambra II"));

        let err = select(&[a.clone(), b.clone()], &f, None, "Alhambra II").unwrap_err();
        assert_eq!(err.action.as_ref().unwrap().code, ActionCode::SelectDevice);
        assert_eq!(err.message, "Found 2 matching devices.");
        let cands = &err.data.unwrap()["candidates"];
        assert_eq!(cands[1]["serial_number"], "B2");

        assert_eq!(
            select(&[a.clone(), b.clone()], &f, Some("B2"), "Alhambra II").unwrap(),
            b
        );
        let err = select(&[a, b], &f, Some("Z9"), "Alhambra II").unwrap_err();
        assert!(err.message.contains("Z9"));
    }

    #[test]
    fn scanning_works_here() {
        // Just checks libusb starts; the list depends on the machine.
        let devices = scan().unwrap();
        for d in &devices {
            assert_eq!(d.vid.len(), 4);
        }
    }
}
