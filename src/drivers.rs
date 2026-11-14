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

/// Windows' hardware ID of a device's interface, as libwdi writes it:
/// USB\VID_0403&PID_6010&MI_00 for a composite device's interface,
/// USB\VID_0403&PID_6014 for a single-interface device. Upper-case hex.
pub fn hardware_id(vid: &str, pid: &str, interface: u8, composite: bool) -> String {
    let mut id = format!(
        "USB\\VID_{}&PID_{}",
        vid.to_ascii_uppercase(),
        pid.to_ascii_uppercase()
    );
    if composite {
        id.push_str(&format!("&MI_{interface:02X}"));
    }
    id
}

/// The subject of the certificate libwdi creates to sign a driver package
/// for `hardware_id`, and adds to the machine's Root and TrustedPublisher
/// stores (libwdi.c, SelfSignFile): what uninstall_driver removes.
pub fn certificate_subject(hardware_id: &str) -> String {
    format!("{hardware_id} (libwdi autogenerated)")
}

/// wdi-simple's arguments to install WinUSB (type 0) on a device's
/// interface: --iid only for a composite device (it also marks the device
/// as composite), the device's name as Device Manager shows it, and the
/// folder libwdi writes the driver package into.
pub fn wdi_simple_args(
    vid: &str,
    pid: &str,
    interface: u8,
    composite: bool,
    name: &str,
    dest: &std::path::Path,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--vid".into(),
        format!("0x{vid}").into(),
        "--pid".into(),
        format!("0x{pid}").into(),
    ];
    if composite {
        args.push("--iid".into());
        args.push(interface.to_string().into());
    }
    args.extend([
        "--type".into(),
        "0".into(),
        "--name".into(),
        name.into(),
        "--dest".into(),
        dest.as_os_str().to_owned(),
        // Info: what libwdi did, for the log.
        "--log".into(),
        "1".into(),
    ]);
    args
}

/// What a libwdi error code (wdi-simple's exit code, libwdi.h) means, for
/// people.
pub fn wdi_error(code: i32) -> &'static str {
    match code {
        -1 => "a file couldn't be read or written",
        -2 => "an invalid parameter",
        -3 => "access was denied",
        -4 => "the device isn't connected anymore",
        -5 => "something it needs wasn't found",
        -6 => "Windows is busy with another driver installation",
        -7 => "it timed out",
        -9 => "another driver installation is pending: restart Windows and try again",
        -10 => "it was interrupted",
        -11 => "Windows ran out of a resource",
        -12 => "it isn't supported on this version of Windows",
        -13 => "the driver already exists",
        -14 => "the installation was cancelled",
        -15 => "it needs administrator rights",
        -16 => "it's running in a 32-bit program on 64-bit Windows",
        -17 => "the driver's description file has an error",
        -18 => "the driver's catalog file is missing",
        -19 => "the driver package couldn't be signed",
        _ => "an unexpected error",
    }
}

/// The text of an INF file: UTF-16LE with its byte order mark (as libwdi
/// writes them), or UTF-8 (with or without a mark), else Latin-1.
pub fn decode_inf(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// The providers of the driver packages libwdi writes (its INF templates:
/// winusb and usbser say libwdi; libusbK and libusb0, which older guides
/// installed with Zadig, say libusbK and libusb-win32). Never a vendor's.
const LIBWDI_PROVIDERS: &[&str] = &["libwdi", "libusbk", "libusb-win32"];

/// Whether an INF file is a driver package libwdi wrote for `hardware_id`:
/// a libwdi provider, and libwdi's DeviceID string naming exactly that
/// hardware ID (without its USB\ prefix), as libwdi's templates have them.
/// Packages from Windows or a vendor (such as FTDI's) never match.
pub fn is_libwdi_package_for(inf: &str, hardware_id: &str) -> bool {
    let want = hardware_id
        .strip_prefix("USB\\")
        .unwrap_or(hardware_id)
        .to_ascii_uppercase();
    let value = |line: &str, key: &str| -> Option<String> {
        let (k, v) = line.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(key)
            .then(|| v.trim().trim_matches('"').to_string())
    };
    let (mut provider, mut device) = (false, false);
    for line in inf.lines() {
        let line = line.trim();
        if let Some(v) = value(line, "Provider") {
            provider |= LIBWDI_PROVIDERS.contains(&v.to_ascii_lowercase().as_str());
        }
        if let Some(v) = value(line, "DeviceID") {
            device |= v.to_ascii_uppercase() == want;
        }
    }
    provider && device
}

/// The driver packages libwdi installed for `hardware_id`: Windows' copies
/// of them, oem<N>.inf in its INF folder, by file name (what pnputil
/// /delete-driver takes), sorted.
pub fn libwdi_packages(inf_folder: &std::path::Path, hardware_id: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(inf_folder) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            let oem = lower.starts_with("oem") && lower.ends_with(".inf");
            if !oem || !is_package_name(&lower) {
                return None;
            }
            let text = decode_inf(&std::fs::read(e.path()).ok()?);
            is_libwdi_package_for(&text, hardware_id).then_some(lower)
        })
        .collect();
    out.sort();
    out
}

/// Whether a name is a driver package's in Windows' INF folder: oem<N>.inf.
pub fn is_package_name(name: &str) -> bool {
    name.strip_prefix("oem")
        .and_then(|r| r.strip_suffix(".inf"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Windows' INF folder, where it keeps the installed driver packages.
pub fn inf_folder() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    std::path::PathBuf::from(root).join("INF")
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        DIGCF_ALLCLASSES, DIGCF_PRESENT, SP_DEVINFO_DATA, SPDRP_SERVICE,
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
    };

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
            // INVALID_HANDLE_VALUE, -1, whatever HDEVINFO's type.
            if set as isize == -1 {
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

    #[test]
    fn hardware_ids_and_certificates_are_libwdi_s() {
        let id = hardware_id("0403", "6010", 0, true);
        assert_eq!(id, "USB\\VID_0403&PID_6010&MI_00");
        assert_eq!(
            hardware_id("1d50", "614e", 0, false),
            "USB\\VID_1D50&PID_614E"
        );
        assert_eq!(
            hardware_id("0403", "6010", 11, true),
            "USB\\VID_0403&PID_6010&MI_0B"
        );
        assert_eq!(
            certificate_subject(&id),
            "USB\\VID_0403&PID_6010&MI_00 (libwdi autogenerated)"
        );
    }

    #[test]
    fn wdi_simple_gets_the_interface_only_for_composite_devices() {
        let dest = std::path::Path::new("/tmp/driver");
        let words = |a: Vec<std::ffi::OsString>| {
            a.into_iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            words(wdi_simple_args(
                "0403",
                "6010",
                0,
                true,
                "Alhambra II",
                dest
            )),
            "--vid 0x0403 --pid 0x6010 --iid 0 --type 0 --name Alhambra II --dest /tmp/driver --log 1"
        );
        assert_eq!(
            words(wdi_simple_args(
                "1209",
                "5af0",
                0,
                false,
                "OrangeCrab",
                dest
            )),
            "--vid 0x1209 --pid 0x5af0 --type 0 --name OrangeCrab --dest /tmp/driver --log 1"
        );
    }

    #[test]
    fn libwdi_errors_are_explained() {
        assert_eq!(wdi_error(-14), "the installation was cancelled");
        assert!(wdi_error(-9).contains("restart Windows"));
        assert_eq!(wdi_error(-99), "an unexpected error");
    }

    /// A WinUSB package as libwdi writes it (winusb.inf.in, filled in).
    const LIBWDI_INF: &str = "; usb_device.inf\r\n[Strings]\r\nDeviceName = \"Alhambra II\"\r\nDeviceID   = \"VID_0403&PID_6010&MI_00\"\r\n\r\n[Version]\r\nSignature   = \"$Windows NT$\"\r\nProvider    = \"libwdi\"\r\n";

    fn utf16(text: &str) -> Vec<u8> {
        let mut out = vec![0xff, 0xfe];
        out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    #[test]
    fn libwdi_packages_are_found_by_provider_and_hardware_id() {
        let id = "USB\\VID_0403&PID_6010&MI_00";
        assert_eq!(decode_inf(&utf16(LIBWDI_INF)), LIBWDI_INF);
        assert!(is_libwdi_package_for(&decode_inf(&utf16(LIBWDI_INF)), id));
        assert!(is_libwdi_package_for(LIBWDI_INF, id));
        // Another interface, other IDs: not this one's.
        assert!(!is_libwdi_package_for(
            LIBWDI_INF,
            "USB\\VID_0403&PID_6010&MI_01"
        ));
        assert!(!is_libwdi_package_for(LIBWDI_INF, "USB\\VID_0403&PID_6010"));
        // libusbK's, from an older Zadig guide: also libwdi's.
        let libusbk = LIBWDI_INF.replace("\"libwdi\"", "\"libusbK\"");
        assert!(is_libwdi_package_for(&libusbk, id));
        // FTDI's own: never.
        let ftdi = "[Version]\nProvider=%FTDI%\n[Strings]\nFTDI=\"FTDI\"\nDeviceID = \"VID_0403&PID_6010&MI_00\"\n";
        assert!(!is_libwdi_package_for(ftdi, id));
        // In a folder: only oem<N>.inf files, sorted.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("oem12.inf"), utf16(LIBWDI_INF)).unwrap();
        std::fs::write(tmp.path().join("oem3.inf"), libusbk).unwrap();
        std::fs::write(tmp.path().join("oem4.inf"), ftdi).unwrap();
        std::fs::write(tmp.path().join("usb.inf"), LIBWDI_INF).unwrap();
        assert_eq!(libwdi_packages(tmp.path(), id), ["oem12.inf", "oem3.inf"]);
        assert!(libwdi_packages(&tmp.path().join("missing"), id).is_empty());
        assert!(is_package_name("oem12.inf"));
        assert!(!is_package_name("oem.inf"));
        assert!(!is_package_name("oem1.inf & del"));
    }

    // On Windows, the scan reads Windows' own INF folder: no package names
    // a made-up device.
    #[test]
    fn no_libwdi_package_for_a_made_up_device() {
        assert!(libwdi_packages(&inf_folder(), "USB\\VID_FFFE&PID_FFFE&MI_00").is_empty());
    }

    // On Windows, the query runs against the runner's own USB devices: an
    // ID no device has gives nothing.
    #[test]
    fn devices_nobody_has_have_no_drivers() {
        assert!(current_drivers("fffe", "fffe", 0, true).is_empty());
        assert!(current_drivers("fffe", "fffe", 0, false).is_empty());
    }
}
