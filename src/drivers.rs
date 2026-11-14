//! USB drivers on Windows (docs/requirements.md, Milestone 3 plan, drivers;
//! docs/tool-api.md, list_devices). The programmers reach most boards
//! through libusb, which on Windows can use only an interface that has
//! WinUSB (or libusbK or libusb0), not the driver Windows picks by itself,
//! such as FTDI's. Which interface needs it follows from the variant's
//! programmer and connection, so the catalog has no driver field: the
//! engine knows its programmers.
//!
//! Elsewhere (macOS, Linux) there are no drivers to install: nothing here
//! is reported.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::Variant;

/// Which of a device's USB interfaces a programmer opens, when it needs
/// WinUSB on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needed {
    /// Interface 0: FTDI chips' first channel (JTAG or SPI), which iceprog
    /// and openFPGALoader's FTDI cables use, and the boards openFPGALoader
    /// knows by name (Digilent, Alchitry, Tang Nano, ULX3S and others,
    /// FTDI chips or chips that act like one). Other channels, such as an
    /// FT2232's serial port, keep their driver.
    First,
    /// The device's DFU interface (USB class 0xFE, subclass 1), which
    /// dfu-util opens: a board's bootloader.
    Dfu,
}

/// The interface a variant's programmer needs WinUSB on, on Windows; None
/// when it needs no driver: icesprog (the iCESugar's iCELink) and CMSIS-DAP
/// adapters are HID devices, which Windows' own driver serves.
pub fn needed(variant: &Variant) -> Option<Needed> {
    match variant.programmer.as_str() {
        "icesprog" => None,
        "dfu-util" => Some(Needed::Dfu),
        "openfpgaloader" if is_cmsis_dap(&variant.args) => None,
        _ => Some(Needed::First),
    }
}

/// Whether openFPGALoader's arguments name the CMSIS-DAP cable.
fn is_cmsis_dap(args: &str) -> bool {
    let words: Vec<&str> = args.split_whitespace().collect();
    words
        .windows(2)
        .any(|w| matches!(w[0], "-c" | "--cable") && w[1].eq_ignore_ascii_case("cmsisdap"))
        || words
            .iter()
            .any(|w| w.eq_ignore_ascii_case("--cable=cmsisdap"))
}

/// The driver a device needs for a board's programmer on Windows, and the
/// one it has: in list_devices, on Windows only, for devices whose board
/// needs one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriverStatus {
    /// The driver the board's programmer needs: winusb.
    pub needed: String,
    /// The USB interface that needs it, such as 0 (an FTDI chip's first
    /// channel; its other channels, such as a serial port, keep their
    /// driver).
    pub interface: u8,
    /// The driver Windows has on that interface now, by its service name in
    /// lowercase, such as ftdibus (FTDI's), usbser or winusb. Omitted when
    /// it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// Whether the programmer can use the interface as it is: true with
    /// WinUSB, and with libusbK or libusb0, which older guides installed.
    /// False means install_driver (WinUSB) first.
    pub ready: bool,
}

/// The drivers libusb can use, by service name.
const LIBUSB_DRIVERS: &[&str] = &["winusb", "libusbk", "libusb0"];

/// The interface number `needed` means on a device: 0, or its DFU
/// interface (0 when it has none that says so).
pub fn interface_number(needed: Needed, dfu_interface: Option<u8>) -> u8 {
    match needed {
        Needed::First => 0,
        Needed::Dfu => dfu_interface.unwrap_or(0),
    }
}

/// The status of a device's interface, from the drivers Windows has on the
/// present devices with its USB IDs and interface (`current`, one per
/// device: Windows gives drivers by USB IDs and interface, so devices with
/// the same ones normally share it). Ready only when every one is.
pub fn status(interface: u8, current: &[Option<String>]) -> DriverStatus {
    let ready = |c: &Option<String>| c.as_deref().is_some_and(|s| LIBUSB_DRIVERS.contains(&s));
    let shown = current
        .iter()
        .find(|c| !ready(c))
        .or(current.first())
        .cloned()
        .flatten();
    DriverStatus {
        needed: "winusb".into(),
        interface,
        ready: !current.is_empty() && current.iter().all(ready),
        current: shown,
    }
}

/// The driver Windows has on each present device with these USB IDs, on
/// `interface` (for a device with several interfaces, Windows' device for
/// that interface; for a single-interface device, the device itself): one
/// entry per device, its service name in lowercase, None without a driver.
/// Empty when Windows lists none (and on other systems).
pub fn current_drivers(
    vid: &str,
    pid: &str,
    interface: u8,
    composite: bool,
) -> Vec<Option<String>> {
    #[cfg(windows)]
    {
        windows::current_drivers(vid, pid, interface, composite)
    }
    #[cfg(not(windows))]
    {
        let _ = (vid, pid, interface, composite);
        Vec::new()
    }
}

/// Whether a Windows device instance ID (such as
/// USB\VID_0403&PID_6010&MI_00\6&2E6F&0&0000) is the device with these USB
/// IDs, on `interface` when composite.
pub fn instance_matches(
    instance: &str,
    vid: &str,
    pid: &str,
    interface: u8,
    composite: bool,
) -> bool {
    let instance = instance.to_ascii_uppercase();
    let ids = format!(
        "USB\\VID_{}&PID_{}",
        vid.to_ascii_uppercase(),
        pid.to_ascii_uppercase()
    );
    let Some(rest) = instance.strip_prefix(&ids) else {
        return false;
    };
    if composite {
        rest.starts_with(&format!("&MI_{interface:02X}\\"))
    } else {
        rest.starts_with('\\')
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        DIGCF_ALLCLASSES, DIGCF_PRESENT, SP_DEVINFO_DATA, SPDRP_SERVICE,
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    pub fn current_drivers(
        vid: &str,
        pid: &str,
        interface: u8,
        composite: bool,
    ) -> Vec<Option<String>> {
        let mut out = Vec::new();
        let enumerator = wide("USB");
        // SAFETY: a device information set for the present USB devices,
        // walked with correctly sized structures and buffers, and destroyed
        // at the end.
        unsafe {
            let set = SetupDiGetClassDevsW(
                std::ptr::null(),
                enumerator.as_ptr(),
                std::ptr::null_mut(),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            );
            if set == INVALID_HANDLE_VALUE as _ {
                log::info!("Can't list the USB devices' drivers");
                return out;
            }
            let mut index = 0;
            loop {
                let mut info: SP_DEVINFO_DATA = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiEnumDeviceInfo(set, index, &mut info) == 0 {
                    break;
                }
                index += 1;
                let mut id = [0u16; 512];
                if SetupDiGetDeviceInstanceIdW(
                    set,
                    &info,
                    id.as_mut_ptr(),
                    id.len() as u32,
                    std::ptr::null_mut(),
                ) == 0
                {
                    continue;
                }
                let instance = from_wide(&id);
                if !super::instance_matches(&instance, vid, pid, interface, composite) {
                    continue;
                }
                // The service of the device's driver; none without one.
                let mut service = [0u16; 256];
                let found = SetupDiGetDeviceRegistryPropertyW(
                    set,
                    &info,
                    SPDRP_SERVICE,
                    std::ptr::null_mut(),
                    service.as_mut_ptr().cast(),
                    (service.len() * 2) as u32,
                    std::ptr::null_mut(),
                ) != 0;
                let service = from_wide(&service).to_ascii_lowercase();
                log::info!(
                    "Driver of {instance}: {}",
                    if found { service.as_str() } else { "none" }
                );
                out.push((found && !service.is_empty()).then_some(service));
            }
            SetupDiDestroyDeviceInfoList(set);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(programmer: &str, args: &str) -> Variant {
        serde_json::from_value(serde_json::json!({
            "selector": {"connection": "usb", "memory": "flash"},
            "programmer": programmer,
            "args": args,
            "usb": {"vid": "0403", "pid": "6010"},
            "tested": true
        }))
        .unwrap()
    }

    #[test]
    fn the_programmer_says_which_interface_needs_winusb() {
        assert_eq!(
            needed(&variant("iceprog", "-d i:0x${VID}:0x${PID} ${BITSTREAM}")),
            Some(Needed::First)
        );
        assert_eq!(
            needed(&variant("openfpgaloader", "-c ft2232 ${BITSTREAM}")),
            Some(Needed::First)
        );
        assert_eq!(
            needed(&variant(
                "openfpgaloader",
                "--board arty_a7_35t ${BITSTREAM}"
            )),
            Some(Needed::First)
        );
        assert_eq!(
            needed(&variant("dfu-util", "-p ${USB_PATH} -D ${BITSTREAM}")),
            Some(Needed::Dfu)
        );
        assert_eq!(needed(&variant("icesprog", "${BITSTREAM}")), None);
        assert_eq!(
            needed(&variant("openfpgaloader", "-c cmsisdap ${BITSTREAM}")),
            None
        );
        assert_eq!(
            needed(&variant("openfpgaloader", "--cable=cmsisdap ${BITSTREAM}")),
            None
        );
    }

    // Every catalog variant gets an answer, and the boards known to need
    // none (the iCESugar, CMSIS-DAP adapters) get none.
    #[test]
    fn every_catalog_variant_has_an_answer() {
        let catalog = crate::catalog::get();
        let mut kinds = std::collections::BTreeMap::new();
        for (id, board) in &catalog.boards {
            for v in &board.variants {
                kinds.insert((id.as_str(), v.selector.connection.as_str()), needed(v));
            }
        }
        assert_eq!(kinds[&("icesugar-1-5", "usb")], None);
        assert_eq!(kinds[&("icesugar-pro", "usb")], None);
        assert_eq!(kinds[&("alhambra-ii", "usb")], Some(Needed::First));
        assert_eq!(kinds[&("fomu", "bootloader")], Some(Needed::Dfu));
    }

    #[test]
    fn instances_match_by_ids_and_interface() {
        let composite = "USB\\VID_0403&PID_6010&MI_00\\6&2E6F1A2B&0&0000";
        assert!(instance_matches(composite, "0403", "6010", 0, true));
        assert!(!instance_matches(composite, "0403", "6010", 1, true));
        assert!(!instance_matches(composite, "0403", "6010", 0, false));
        assert!(!instance_matches(composite, "0403", "6014", 0, true));
        let single = "USB\\VID_0403&PID_6014\\FT9ABCDE";
        assert!(instance_matches(single, "0403", "6014", 0, false));
        assert!(instance_matches(
            &single.to_lowercase(),
            "0403",
            "6014",
            0,
            false
        ));
        assert!(!instance_matches(single, "0403", "6014", 0, true));
        // A parent composite device isn't an interface.
        assert!(!instance_matches(
            "USB\\VID_0403&PID_6010\\FT1",
            "0403",
            "6010",
            0,
            true
        ));
    }

    #[test]
    fn ready_only_with_a_driver_libusb_can_use() {
        let s = |v: &[Option<&str>]| {
            status(
                0,
                &v.iter().map(|c| c.map(String::from)).collect::<Vec<_>>(),
            )
        };
        assert!(s(&[Some("winusb")]).ready);
        assert!(s(&[Some("libusbk")]).ready);
        let ftdi = s(&[Some("ftdibus")]);
        assert!(!ftdi.ready);
        assert_eq!(ftdi.current.as_deref(), Some("ftdibus"));
        let none = s(&[None]);
        assert!(!none.ready);
        assert_eq!(none.current, None);
        // Two boards with the same IDs: ready only if both are, showing the
        // one that isn't.
        let mixed = s(&[Some("winusb"), Some("ftdibus")]);
        assert!(!mixed.ready);
        assert_eq!(mixed.current.as_deref(), Some("ftdibus"));
        // Not found (such as just unplugged): not ready, nothing current.
        assert!(!s(&[]).ready);
    }

    #[test]
    fn interfaces_follow_the_need() {
        assert_eq!(interface_number(Needed::First, Some(2)), 0);
        assert_eq!(interface_number(Needed::Dfu, Some(2)), 2);
        assert_eq!(interface_number(Needed::Dfu, None), 0);
    }

    // On Windows, the query runs against the runner's own USB devices: an
    // ID no device has gives nothing.
    #[test]
    fn devices_nobody_has_have_no_drivers() {
        assert!(current_drivers("fffe", "fffe", 0, true).is_empty());
        assert!(current_drivers("fffe", "fffe", 0, false).is_empty());
    }
}
