//! Programming a board (docs/tool-api.md, "program_board tool"): fill in the
//! programming variant's argument template for the selected device and run
//! the programmer from the toolchain package.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::catalog::{self, FpgaDefinition, Toolchain, Variant};
use crate::devices::{self, Device};
use crate::errors::{ActionCode, Category, ToolError};

/// The executable in the toolchain package for each programmer ID.
pub fn programmer_executable(programmer: &str) -> Option<&'static str> {
    match programmer {
        "openfpgaloader" => Some("openFPGALoader"),
        "iceprog" => Some("iceprog"),
        "dfu-util" => Some("dfu-util"),
        "icesprog" => Some("icesprog"),
        _ => None,
    }
}

/// Whether a programming command tells the programmer which device to open:
/// by bus and device number, by USB port path, or by a serial number that no other connected
/// device with the same USB IDs has.
pub fn names_the_device(template: &str, devices: &[Device], device: &Device) -> bool {
    if template.contains("${BUS}") && template.contains("${DEV}") {
        return true;
    }
    if template.contains("${USB_PATH}") && devices::has_port_path(device) {
        return true;
    }
    template.contains("${SERIAL_NUM}")
        && device.serial_number.is_some()
        && !devices.iter().any(|d| {
            d.location != device.location
                && d.vid == device.vid
                && d.pid == device.pid
                && d.serial_number == device.serial_number
        })
}

/// A command that doesn't name the device makes the programmer open the
/// first device with its USB IDs (icesprog, dfu-util with -d VID:PID, and
/// openFPGALoader with --board or -c alone). That could be another board, so
/// the engine runs it only when no other connected device, opened or not,
/// has the chosen device's USB IDs.
pub fn check_alone(
    template: &str,
    devices: &[Device],
    device: &Device,
    board_name: &str,
) -> Result<(), ToolError> {
    if names_the_device(template, devices, device) {
        return Ok(());
    }
    let others = devices
        .iter()
        .filter(|d| d.location != device.location && d.vid == device.vid && d.pid == device.pid)
        .count();
    if others == 0 {
        return Ok(());
    }
    let devices_word = if others == 1 {
        "device has"
    } else {
        "devices have"
    };
    Err(ToolError::setup(format!(
        "{others} other connected {devices_word} the {board_name}'s USB IDs {}:{}, and its programming command can't tell them apart.",
        device.vid, device.pid
    ))
    .with_action(
        ActionCode::DisconnectDevices,
        format!("Disconnect the other devices, leaving only the {board_name}, then retry."),
    ))
}

/// Fills in a template for a device and bitstream. The template is split
/// into words first, so a path with spaces stays one argument.
pub fn expand(
    template: &str,
    device: &Device,
    bitstream: &Path,
) -> Result<Vec<OsString>, ToolError> {
    catalog::check_template(template)
        .map_err(|e| ToolError::request(format!("Bad programming command: {e}.")))?;
    let mut out = Vec::new();
    for word in template.split_whitespace() {
        let mut w = fill(word, device)?;
        if w.contains("${BITSTREAM}") {
            let path = bitstream.to_string_lossy();
            w = w.replace("${BITSTREAM}", &path);
        }
        out.push(OsString::from(w));
    }
    Ok(out)
}

/// Fills in the device's variables in one word of a template.
fn fill(word: &str, device: &Device) -> Result<String, ToolError> {
    let mut w = word
        .replace("${VID}", &device.vid)
        .replace("${PID}", &device.pid)
        .replace("${BUS}", &device.bus.to_string())
        .replace("${DEV}", &device.device.to_string());
    if w.contains("${SERIAL_NUM}") {
        let serial = device.serial_number.as_deref().ok_or_else(|| {
            ToolError::setup(
                "The device has no serial number, which the programming command needs.",
            )
        })?;
        w = w.replace("${SERIAL_NUM}", serial);
    }
    if w.contains("${USB_PATH}") {
        if !devices::has_port_path(device) {
            return Err(ToolError::setup(
                "The device's USB port path is unknown, which the programming command needs.",
            ));
        }
        w = w.replace("${USB_PATH}", &device.location);
    }
    Ok(w)
}

/// The program and arguments to run: the variant's programmer with its
/// expanded template, or a custom command whose first word is a program in
/// the toolchain package or an absolute path.
/// The programmer's own flag for printing more, used with verbose.
pub fn verbose_flag(programmer: &str) -> Option<&'static str> {
    match programmer {
        "openfpgaloader" => Some("--verbose"),
        "iceprog" => Some("-v"),
        "dfu-util" => Some("-v"),
        _ => None,
    }
}

/// How the board will be programmed, as the engine says before it starts,
/// such as "Programming the RAM of the butterstick r1.0 through the FT232H
/// adapter.". `board` is the board's name (its description, without the
/// variant: the line says it). An untested configuration ends with
/// " (untested)" before the period.
pub fn plan(board: &str, c: &Variant) -> Option<String> {
    let memory = memory_word(c)?;
    let connection = Some(c.selector.connection.as_str());
    let through = match connection {
        Some("usb") => " through its USB port".to_string(),
        Some("bootloader") => " through its USB bootloader".to_string(),
        Some("unspecified") | None => " through USB".to_string(),
        Some(other) => {
            let label = catalog::CONNECTIONS
                .iter()
                .find(|(id, _, _)| *id == other)
                .map(|(_, label, _)| *label)
                .unwrap_or(other);
            format!(" through the {label}")
        }
    };
    let untested = if c.tested { "" } else { " (untested)" };
    Some(format!(
        "Programming the {memory} of the {board}{through}{untested}."
    ))
}

/// What a successful programming means, such as "Programmed the RAM of the
/// RAM: the design is lost when the board is powered off.".
pub fn done(board: &str, c: &Variant) -> Option<String> {
    let memory = memory_word(c)?;
    let stays = if memory == "RAM" { "is lost" } else { "stays" };
    Some(format!(
        "Programmed the {memory} of the {board}: the design {stays} when the board is powered off."
    ))
}

/// The progress phase while the programmer writes.
pub fn writing_phase(c: &Variant) -> &'static str {
    match memory_word(c) {
        Some("RAM") => "Writing RAM",
        _ => "Writing flash",
    }
}

fn memory_word(c: &Variant) -> Option<&'static str> {
    match Some(c.selector.memory.as_str()) {
        Some("flash") => Some("flash"),
        Some("ram") => Some("RAM"),
        _ => None,
    }
}

pub fn command(
    variant: &Variant,
    custom: Option<&str>,
    device: &Device,
    bitstream: &Path,
    bin_dir: &Path,
    verbose: bool,
) -> Result<(PathBuf, Vec<OsString>), ToolError> {
    match custom {
        None => {
            let exe = programmer_executable(&variant.programmer).ok_or_else(|| {
                ToolError::request(format!("Unknown programmer {:?}.", variant.programmer))
            })?;
            let mut args = Vec::new();
            if verbose && let Some(flag) = verbose_flag(&variant.programmer) {
                args.push(OsString::from(flag));
            }
            args.extend(expand(&variant.args, device, bitstream)?);
            Ok((crate::commands::program_in(bin_dir, exe), args))
        }
        Some(cmd) => {
            let mut words = expand(cmd, device, bitstream)?;
            if words.is_empty() {
                return Err(ToolError::request("programmer_command is empty."));
            }
            let first = PathBuf::from(words.remove(0));
            let program = if first.is_absolute() {
                first
            } else if first.components().count() == 1
                && crate::commands::program_in(bin_dir, &first.to_string_lossy()).is_file()
            {
                crate::commands::program_in(bin_dir, &first.to_string_lossy())
            } else {
                return Err(ToolError::request(format!(
                    "programmer_command must start with a program from the toolchain package, such as openFPGALoader or iceprog, or an absolute path, not {:?}.",
                    first.display()
                )));
            };
            Ok((program, words))
        }
    }
}

/// The openFPGALoader options that choose the connection, with whether each
/// takes a value. The FPGA check's --detect command keeps only these from
/// the variant's command, so it can't write anything.
const CONNECTION_OPTIONS: &[(&str, bool)] = &[
    ("--board", true),
    ("-b", true),
    ("--cable", true),
    ("-c", true),
    ("--busdev-num", true),
    ("--vid", true),
    ("--pid", true),
    ("--pins", true),
    ("--freq", true),
    ("--ftdi-channel", true),
    ("--cable-index", true),
    ("--usb-serial-num", true),
    ("--ftdi-serial", true),
    ("--index-chain", true),
    ("--invert-read-edge", false),
    ("--force-terminal-mode", false),
];

/// Whether the engine checks the connected FPGA's ID code before programming
/// with this variant: openFPGALoader over JTAG (not iCE40, which it programs
/// through the flash, and not DFU or SPI modes), for an FPGA with a
/// jtag_idcode.
pub fn checks_fpga(variant: &Variant, fpga: &FpgaDefinition) -> bool {
    variant.programmer == "openfpgaloader"
        && fpga.toolchain() != Toolchain::Ice40
        && fpga.jtag_idcode.is_some()
        && !variant
            .args
            .split_whitespace()
            .any(|w| matches!(w, "--dfu" | "--spi" | "--xvc" | "--passive-serial"))
}

/// The arguments of openFPGALoader's --detect run for the FPGA check: the
/// variant's connection options only, then --detect.
pub fn detect_args(variant: &Variant, device: &Device) -> Result<Vec<OsString>, ToolError> {
    let words: Vec<&str> = variant.args.split_whitespace().collect();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let option = CONNECTION_OPTIONS
            .iter()
            .find(|(name, _)| w == *name || w.starts_with(&format!("{name}=")));
        if let Some((name, takes_value)) = option {
            kept.push(w);
            if *takes_value && w == *name && i + 1 < words.len() {
                kept.push(words[i + 1]);
                i += 1;
            }
        }
        i += 1;
    }
    let mut args = Vec::new();
    for w in kept {
        args.push(OsString::from(fill(w, device)?));
    }
    args.push(OsString::from("--detect"));
    Ok(args)
}

/// An FPGA that openFPGALoader's --detect found.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedFpga {
    pub idcode: u32,
    /// openFPGALoader's model name, such as xc7a35.
    pub model: Option<String>,
}

/// Reads the FPGAs from openFPGALoader's --detect output: one block per
/// device on the JTAG chain, "index 0:", then tab-indented "idcode 0x...",
/// "manufacturer", "family", "model" and "irlength" lines. Other devices on
/// the chain, such as a Zynq's processor, have a "type" line instead and
/// aren't FPGAs. An "Unknown device with IDCODE: 0x..." error counts as an
/// FPGA without a model.
pub fn parse_detect(output: &str) -> Vec<DetectedFpga> {
    let mut found = Vec::new();
    let mut current: Option<(u32, Option<String>, bool)> = None;
    let finish = |c: Option<(u32, Option<String>, bool)>, found: &mut Vec<DetectedFpga>| {
        if let Some((idcode, model, false)) = c {
            found.push(DetectedFpga { idcode, model });
        }
    };
    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("index ") && t.ends_with(':') {
            finish(current.take(), &mut found);
        } else if let Some(rest) = t.strip_prefix("idcode") {
            let hex = rest.trim().trim_start_matches("0x");
            if let Ok(code) = u32::from_str_radix(hex, 16) {
                finish(current.take(), &mut found);
                current = Some((code, None, false));
            }
        } else if let Some(rest) = t.strip_prefix("model") {
            if let Some(c) = current.as_mut() {
                c.1 = Some(rest.trim().to_string());
            }
        } else if t.starts_with("type") {
            if let Some(c) = current.as_mut() {
                c.2 = true;
            }
        } else if let Some(pos) = t.find("Unknown device with IDCODE: 0x") {
            let hex: String = t[pos + 30..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                found.push(DetectedFpga {
                    idcode: code,
                    model: None,
                });
            }
        }
    }
    finish(current.take(), &mut found);
    found
}

/// Checks that the FPGA found is the build's. `others` names the boards
/// with the found FPGA that the device most likely is, to suggest.
pub fn check_fpga(
    found: &[DetectedFpga],
    fpga: &FpgaDefinition,
    board_name: &str,
    others: impl Fn(u32) -> Vec<String>,
) -> Result<(), ToolError> {
    let Some(want) = fpga.jtag_idcode.as_deref().and_then(catalog::parse_idcode) else {
        return Ok(());
    };
    if found.iter().any(|f| f.idcode == want) {
        return Ok(());
    }
    let Some(first) = found.first() else {
        return Err(ToolError::setup(format!(
            "No FPGA was found on the device chosen for the {board_name}, so nothing was written."
        ))
        .with_action(
            ActionCode::ConnectBoard,
            format!("Connect the {board_name} and make sure it is powered, then retry."),
        ));
    };
    let name = match &first.model {
        Some(m) => format!("{} (ID code 0x{:08x})", m.to_uppercase(), first.idcode),
        None => format!("an FPGA with ID code 0x{:08x}", first.idcode),
    };
    let mut message = format!(
        "The connected board has {name}, but the build is for the {board_name}, which has {} (ID code 0x{want:08x}). Nothing was written.",
        fpga.part_number
    );
    let boards = others(first.idcode);
    if !boards.is_empty() {
        message.push_str(&format!(
            " Boards with that FPGA that the device most likely is: {}.",
            boards.join(", ")
        ));
    }
    Err(ToolError::setup(message).with_action(
        ActionCode::ConnectBoard,
        format!("Connect the {board_name}, or build for the board that is connected, then retry."),
    ))
}

/// Turns a failed programmer run into an error, by what its output says.
pub fn failure(programmer: &str, board_name: &str, output: &str, status: &str) -> ToolError {
    let lower = output.to_lowercase();
    let details = crate::text::to_multiline_text(output);
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let err = if has(&["more than one dfu capable"]) {
        ToolError::setup(format!(
            "Several connected devices could be the {board_name}, and {programmer} won't choose."
        ))
        .with_action(
            ActionCode::DisconnectDevices,
            format!("Disconnect the other boards, leaving only the {board_name}, then retry."),
        )
    } else if has(&["no dfu capable usb device"]) {
        ToolError::setup(format!(
            "The {board_name} wasn't found in its bootloader (DFU) mode."
        ))
        .with_action(
            ActionCode::ConnectBoard,
            format!(
                "Connect the {board_name} by USB in its bootloader mode, usually by holding its button while plugging it in, then retry."
            ),
        )
    } else if has(&[
        "insufficient permissions",
        "permission denied",
        "access denied",
        "libusb_error_access",
    ]) {
        ToolError::setup(format!("The {board_name} was found but can't be opened.")).with_action(
            ActionCode::FixDevicePermissions,
            "Allow this user to access the USB device, then retry.",
        )
    } else if has(&["unable to claim", "claim interface", "resource busy"]) {
        ToolError::setup(format!(
            "The {board_name} is in use by another program or driver."
        ))
        .with_action(
            ActionCode::FixDevicePermissions,
            "Close other programs using the board, or unload the system driver that claims it, then retry.",
        )
    } else if has(&[
        "device not found",
        "can't find",
        "cannot find",
        "no device found",
        "not found",
        "icelink open fail",
    ]) {
        ToolError::setup(format!("The {board_name} wasn't found by {programmer}.")).with_action(
            ActionCode::ConnectBoard,
            format!("Connect the {board_name} by USB, then retry."),
        )
    } else {
        ToolError::new(
            Category::Tool,
            format!("{programmer} failed to program the {board_name} ({status})."),
        )
    };
    let mut err = err;
    err.details = details;
    err
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::tests::device;

    #[test]
    fn templates_expand_per_word() {
        let d = device("0403", "6010", Some("Alhambra II"), Some("A1"), 7);
        let args = expand(
            "--vid ${VID} --pid ${PID} --busdev-num ${BUS}:${DEV} -s ${SERIAL_NUM} ${BITSTREAM}",
            &d,
            Path::new("/my proj/_build/hardware.bin"),
        )
        .unwrap();
        let args: Vec<String> = args.into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args,
            [
                "--vid",
                "0403",
                "--pid",
                "6010",
                "--busdev-num",
                "1:7",
                "-s",
                "A1",
                "/my proj/_build/hardware.bin"
            ]
        );
        let no_serial = device("0403", "6010", None, None, 7);
        assert_eq!(
            expand("-s ${SERIAL_NUM} ${BITSTREAM}", &no_serial, Path::new("/b"))
                .unwrap_err()
                .category,
            Category::Setup
        );
        assert_eq!(
            expand("${NOPE} ${BITSTREAM}", &no_serial, Path::new("/b"))
                .unwrap_err()
                .category,
            Category::Request
        );
    }

    #[test]
    fn catalog_commands() {
        let c = catalog::get();
        let d = device("0403", "6010", Some("Alhambra II"), None, 9);
        let bin = Path::new("/pkg/bin");
        let v = &c.boards["alhambra-ii"].variants[0];
        let (prog, args) = command(v, None, &d, Path::new("/b/hardware.bin"), bin, false).unwrap();
        assert_eq!(prog, Path::new("/pkg/bin/openFPGALoader"));
        let args: Vec<String> = args.into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args.join(" "),
            "--force-terminal-mode --verify -b ice40_generic --vid 0403 --pid 6010 --busdev-num 1:9 /b/hardware.bin"
        );
        let (_, args) = command(v, None, &d, Path::new("/b/hardware.bin"), bin, true).unwrap();
        assert_eq!(args[0], "--verbose");
        let v = &c.boards["icestick"].variants[0];
        let (prog, args) = command(v, None, &d, Path::new("/b/hardware.bin"), bin, false).unwrap();
        assert_eq!(prog, Path::new("/pkg/bin/iceprog"));
        assert_eq!(args.len(), 3); // -d d:1/9 /b/hardware.bin
    }

    #[test]
    fn says_how_the_board_is_programmed() {
        let c = catalog::get();
        let alhambra = &c.boards["alhambra-ii"].variants[0];
        assert_eq!(
            plan("Alhambra II", alhambra).unwrap(),
            "Programming the flash of the Alhambra II through its USB port."
        );
        assert_eq!(
            done("Alhambra II", alhambra).unwrap(),
            "Programmed the flash of the Alhambra II: the design stays when the board is powered off."
        );
        assert_eq!(writing_phase(alhambra), "Writing flash");
        let stick = &c.boards["butterstick-r10-2g-85k"].variants;
        assert_eq!(
            plan("butterstick r1.0", &stick[0]).unwrap(),
            "Programming the flash of the butterstick r1.0 through its USB bootloader."
        );
        assert_eq!(
            plan("butterstick r1.0", &stick[2]).unwrap(),
            "Programming the RAM of the butterstick r1.0 through the FT232H adapter."
        );
        assert_eq!(
            done("butterstick r1.0", &stick[2]).unwrap(),
            "Programmed the RAM of the butterstick r1.0: the design is lost when the board is powered off."
        );
        assert_eq!(writing_phase(&stick[2]), "Writing RAM");
        let arty = &c.boards["arty-a7-35t"].variants[0];
        assert!(!arty.tested);
        assert_eq!(
            plan("Arty A7-35T (Xilinx, Artix7)", arty).unwrap(),
            "Programming the flash of the Arty A7-35T (Xilinx, Artix7) through its USB port (untested)."
        );
    }

    #[test]
    fn custom_commands() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("iceprog"), "").unwrap();
        let d = device("0403", "6010", None, None, 2);
        let v = &catalog::get().boards["alhambra-ii"].variants[0];
        let b = Path::new("/b/hardware.bin");
        let (p, a) = command(v, Some("iceprog -S ${BITSTREAM}"), &d, b, tmp.path(), false).unwrap();
        assert_eq!(p, tmp.path().join("iceprog"));
        assert_eq!(
            a,
            vec![OsString::from("-S"), OsString::from("/b/hardware.bin")]
        );
        let (p, _) = command(
            v,
            Some("/opt/x/prog ${BITSTREAM}"),
            &d,
            b,
            tmp.path(),
            false,
        )
        .unwrap();
        assert_eq!(p, Path::new("/opt/x/prog"));
        for bad in ["rm ${BITSTREAM}", "../iceprog ${BITSTREAM}", "iceprog"] {
            assert_eq!(
                command(v, Some(bad), &d, b, tmp.path(), false)
                    .unwrap_err()
                    .category,
                Category::Request,
                "{bad}"
            );
        }
    }

    #[test]
    fn commands_that_dont_name_the_device_run_only_alone() {
        let nano = device("1d50", "602b", Some("DAPLink CMSIS-DAP"), Some("0720"), 3);
        let pro = device("1d50", "602b", Some("DAPLink CMSIS-DAP"), Some("0710"), 4);
        let other = device("0403", "6010", None, None, 5);
        let both = [nano.clone(), pro, other.clone()];
        let icesprog = "-w ${BITSTREAM}";
        assert!(
            check_alone(
                icesprog,
                &[nano.clone(), other.clone()],
                &nano,
                "iCESugar-nano"
            )
            .is_ok()
        );
        let e = check_alone(icesprog, &both, &nano, "iCESugar-nano").unwrap_err();
        assert_eq!(e.action.unwrap().code, ActionCode::DisconnectDevices);
        assert!(
            e.message.starts_with("1 other connected device has"),
            "{}",
            e.message
        );

        // Bus and device number, or a serial number only this device has.
        assert!(check_alone("--busdev-num ${BUS}:${DEV} ${BITSTREAM}", &both, &nano, "x").is_ok());
        assert!(check_alone("-s ${SERIAL_NUM} ${BITSTREAM}", &both, &nano, "x").is_ok());
        let twin = device("1d50", "602b", Some("DAPLink CMSIS-DAP"), Some("0720"), 6);
        assert!(
            check_alone(
                "-s ${SERIAL_NUM} ${BITSTREAM}",
                &[nano.clone(), twin],
                &nano,
                "x"
            )
            .is_err()
        );
        assert!(check_alone("-d ${VID}:${PID} -D ${BITSTREAM}", &both, &nano, "x").is_err());
    }

    // openFPGALoader 1.2.0's --detect output, in the format its main.cpp
    // prints (an Arty A7-35T, and a Zynq chain with its processor first).
    const DETECT_ARTY: &str = "index 0:\n\tidcode 0x362d093\n\tmanufacturer xilinx\n\tfamily artix a7 35t\n\tmodel  xc7a35\n\tirlength 6\n";
    const DETECT_ZYNQ: &str = "index 0:\n\tidcode   0x4ba00477\n\ttype     ARM cortex A9\n\tirlength 4\nindex 1:\n\tidcode 0x3727093\n\tmanufacturer xilinx\n\tfamily zynq\n\tmodel  xc7z020\n\tirlength 6\n";

    #[test]
    fn detect_output_lists_the_fpgas() {
        assert_eq!(
            parse_detect(DETECT_ARTY),
            [DetectedFpga {
                idcode: 0x0362d093,
                model: Some("xc7a35".into())
            }]
        );
        assert_eq!(
            parse_detect(DETECT_ZYNQ),
            [DetectedFpga {
                idcode: 0x03727093,
                model: Some("xc7z020".into())
            }]
        );
        assert_eq!(
            parse_detect(
                "Unknown device with IDCODE: 0x0362c093 (manufacturer: 0x049 (xilinx), part: 0x62 vers: 0x0"
            ),
            [DetectedFpga {
                idcode: 0x0362c093,
                model: None
            }]
        );
        assert!(parse_detect("JTAG init failed with: no device found\n").is_empty());
    }

    #[test]
    fn the_fpga_check_runs_detect_with_the_connection_options_only() {
        let c = catalog::get();
        let d = device("0403", "6010", Some("Digilent USB Device"), None, 9);
        let words = |board: &str, i: usize| -> Vec<String> {
            let v = &c.boards[board].variants[i];
            assert!(
                checks_fpga(v, &c.fpgas[&c.boards[board].fpga_id]),
                "{board}"
            );
            detect_args(v, &d)
                .unwrap()
                .into_iter()
                .map(|a| a.into_string().unwrap())
                .collect()
        };
        // Arty A7-35T, flash: --write-flash, --verify and the bitstream go.
        assert_eq!(
            words("arty-a7-35t", 0).join(" "),
            "--force-terminal-mode --board arty_a7_35t --busdev-num 1:9 --detect"
        );
        // A cable, --pins and the vid/pid stay; -v, --file-type bin and
        // --unprotect-flash go.
        let icepi = c.boards["icepi-zero"].variants[0].args.clone();
        assert!(icepi.contains("--write-flash"), "{icepi}");
        assert_eq!(
            words("icepi-zero", 0).join(" "),
            "--force-terminal-mode -c ft231X --pins=7:3:5:6 --busdev-num 1:9 --detect"
        );
        assert_eq!(
            words("icesugar-pro", 0).join(" "),
            "--force-terminal-mode -c cmsisdap --vid 0x0403 --pid 0x6010 --detect"
        );
        // Not checked: iCE40 (programmed through the flash), other
        // programmers, and FPGAs without an ID code.
        let alhambra = &c.boards["alhambra-ii"];
        assert!(!checks_fpga(
            &alhambra.variants[0],
            &c.fpgas[&alhambra.fpga_id]
        ));
        let orangecrab = &c.boards["orangecrab-r02-25f"];
        assert!(!checks_fpga(
            &orangecrab.variants[0],
            &c.fpgas[&orangecrab.fpga_id]
        ));
        let arty = &c.boards["arty-a7-35t"];
        let no_code = FpgaDefinition {
            jtag_idcode: None,
            ..c.fpgas[&arty.fpga_id].clone()
        };
        assert!(!checks_fpga(&arty.variants[0], &no_code));
    }

    #[test]
    fn the_fpga_check_refuses_another_fpga() {
        let c = catalog::get();
        let fpga = &c.fpgas[&c.boards["arty-a7-35t"].fpga_id];
        let none = |_| vec![];
        assert!(check_fpga(&parse_detect(DETECT_ARTY), fpga, "Arty A7-35T", none).is_ok());

        // An Arty A7-100T connected for a 35T build.
        let a100 = parse_detect(
            &DETECT_ARTY
                .replace("0x362d093", "0x3631093")
                .replace("xc7a35", "xc7a100"),
        );
        let e = check_fpga(&a100, fpga, "Arty A7-35T", |code| {
            assert_eq!(code, 0x03631093);
            vec!["Arty A7-100T".into()]
        })
        .unwrap_err();
        assert_eq!(e.category, Category::Setup);
        assert_eq!(e.action.as_ref().unwrap().code, ActionCode::ConnectBoard);
        assert_eq!(
            e.message,
            "The connected board has XC7A100 (ID code 0x03631093), but the build is for the Arty A7-35T, which has XC7A35T-1CSG324 (ID code 0x0362d093). Nothing was written. Boards with that FPGA that the device most likely is: Arty A7-100T."
        );

        // A chip openFPGALoader doesn't know, and no FPGA at all.
        let unknown =
            parse_detect("Unknown device with IDCODE: 0x12345093 (manufacturer: 0x049 (xilinx)");
        let e = check_fpga(&unknown, fpga, "Arty A7-35T", none).unwrap_err();
        assert!(
            e.message
                .starts_with("The connected board has an FPGA with ID code 0x12345093,"),
            "{}",
            e.message
        );
        let e = check_fpga(&[], fpga, "Arty A7-35T", none).unwrap_err();
        assert!(e.message.starts_with("No FPGA was found"), "{}", e.message);
    }

    #[test]
    fn port_paths_name_the_device() {
        let nano = device("1d50", "6130", Some("OrangeCrab DFU"), None, 3);
        let twin = device("1d50", "6130", Some("OrangeCrab DFU"), None, 4);
        let dfu = "-d ${VID}:${PID} -p ${USB_PATH} -a 0 -D ${BITSTREAM}";
        assert!(check_alone(dfu, &[nano.clone(), twin], &nano, "OrangeCrab").is_ok());
        let args = expand(dfu, &nano, Path::new("/b/hardware.bit")).unwrap();
        assert_eq!(args[3], "1-3");
        // Without port numbers the location is the bus and address, which
        // dfu-util's --path can't take.
        let mut no_ports = nano.clone();
        no_ports.location = "1-a3".into();
        assert!(!devices::has_port_path(&no_ports));
        assert_eq!(
            expand(dfu, &no_ports, Path::new("/b"))
                .unwrap_err()
                .category,
            Category::Setup
        );
    }

    #[test]
    fn failures() {
        // Real output, oss-cad-suite 2026-10-22, with no such device.
        let e = failure(
            "openFPGALoader",
            "Alhambra II",
            "Cable VID overridden\nCable PID overriddenunable to open ftdi device: -3 (device not found) (USB bus 250 addr 3)\n\nError: Failed to claim cable",
            "exit 1",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::ConnectBoard);
        let e = failure(
            "iceprog",
            "iCEstick",
            "init..\nCan't find iCE FTDI USB device (device string d:250/3).\nABORT.",
            "exit 2",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::ConnectBoard);

        let e = failure(
            "iceprog",
            "iCEstick",
            "Can't find iCE FTDI USB device (vendor_id 0x0403, device_id 0x6010).",
            "exit 2",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::ConnectBoard);
        let e = failure(
            "openFPGALoader",
            "Alhambra II",
            "unable to open ftdi device: -8 (usb_open() failed) LIBUSB_ERROR_ACCESS",
            "exit 1",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::FixDevicePermissions);
        let e = failure(
            "openFPGALoader",
            "Alhambra II",
            "unable to claim usb device",
            "exit 1",
        );
        assert!(e.message.contains("in use"));
        // Real output, dfu-util 0.11-dev from oss-cad-suite, with no device.
        let e = failure(
            "dfu-util",
            "OrangeCrab r0.2 25K",
            "dfu-util 0.11-dev\n\ndfu-util: Warning: File too short for DFU suffix\ndfu-util: A valid DFU suffix will be required in a future dfu-util release\ndfu-util: No DFU capable USB device available",
            "exit 74",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::ConnectBoard);
        assert!(e.message.contains("bootloader"));
        // dfu-util's message from its source (main.c) for several matches.
        let e = failure(
            "dfu-util",
            "Fomu",
            "dfu-util: More than one DFU capable USB device found! Try `--list' and specify the serial number or disconnect all but one device",
            "exit 74",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::DisconnectDevices);
        // icesprog's message with no iCELink (its source, and issue reports).
        let e = failure(
            "icesprog",
            "iCESugar-nano",
            "iCELink open fail!",
            "exit 255",
        );
        assert_eq!(e.action.unwrap().code, ActionCode::ConnectBoard);
        let e = failure("openFPGALoader", "Alhambra II", "segfault", "signal 11");
        assert_eq!(e.category, Category::Tool);
        assert_eq!(e.details, "segfault");
    }
}
