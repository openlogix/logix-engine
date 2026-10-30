//! Programming a board (docs/tool-api.md, "program_board tool"): fill in the
//! programming variant's argument template for the selected device and run
//! the programmer from the toolchain package.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::catalog::{self, ProgrammingVariant};
use crate::devices::Device;
use crate::errors::{ActionCode, Category, ToolError};

/// The executable in the toolchain package for each programmer ID.
pub fn programmer_executable(programmer: &str) -> Option<&'static str> {
    match programmer {
        "openfpgaloader" => Some("openFPGALoader"),
        "iceprog" => Some("iceprog"),
        _ => None,
    }
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
        let mut w = word
            .replace("${VID}", &device.vid)
            .replace("${PID}", &device.pid)
            .replace("${BUS}", &device.bus.to_string())
            .replace("${DEV}", &device.device.to_string());
        if w.contains("${SERIAL}") {
            let serial = device.serial_number.as_deref().ok_or_else(|| {
                ToolError::setup(
                    "The device has no serial number, which the programming command needs.",
                )
            })?;
            w = w.replace("${SERIAL}", serial);
        }
        if w.contains("${BITSTREAM}") {
            let path = bitstream.to_string_lossy();
            w = w.replace("${BITSTREAM}", &path);
        }
        out.push(OsString::from(w));
    }
    Ok(out)
}

/// The program and arguments to run: the variant's programmer with its
/// expanded template, or a custom command whose first word is a program in
/// the toolchain package or an absolute path.
pub fn command(
    variant: &ProgrammingVariant,
    custom: Option<&str>,
    device: &Device,
    bitstream: &Path,
    bin_dir: &Path,
) -> Result<(PathBuf, Vec<OsString>), ToolError> {
    match custom {
        None => {
            let exe = programmer_executable(&variant.programmer).ok_or_else(|| {
                ToolError::request(format!("Unknown programmer {:?}.", variant.programmer))
            })?;
            Ok((bin_dir.join(exe), expand(&variant.args, device, bitstream)?))
        }
        Some(cmd) => {
            let mut words = expand(cmd, device, bitstream)?;
            if words.is_empty() {
                return Err(ToolError::request("programmer_command is empty."));
            }
            let first = PathBuf::from(words.remove(0));
            let program = if first.is_absolute() {
                first
            } else if first.components().count() == 1 && bin_dir.join(&first).is_file() {
                bin_dir.join(&first)
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

fn percent_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d{1,3}(?:\.\d+)?)\s*%").unwrap())
}

/// What a programmer's output line says about progress.
#[derive(Debug, PartialEq)]
pub enum ProgressLine {
    /// The programmer is verifying what it wrote, with a percentage when the
    /// line has one.
    Verifying(Option<f64>),
    /// A percentage of the current step.
    Percent(f64),
    Other,
}

/// Reads progress from openFPGALoader and iceprog output lines.
pub fn progress_line(line: &str) -> ProgressLine {
    let lower = line.to_lowercase();
    let percent = percent_re()
        .captures(line)
        .and_then(|c| c[1].parse::<f64>().ok());
    if lower.contains("verif") || lower.starts_with("reading") {
        return ProgressLine::Verifying(percent);
    }
    match percent {
        Some(p) => ProgressLine::Percent(p),
        None => ProgressLine::Other,
    }
}

/// Turns a failed programmer run into an error, by what its output says.
pub fn failure(programmer: &str, board_name: &str, output: &str, status: &str) -> ToolError {
    let lower = output.to_lowercase();
    let details = crate::text::to_multiline_text(output);
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let err = if has(&[
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
            "--vid ${VID} --pid ${PID} --busdev-num ${BUS}:${DEV} -s ${SERIAL} ${BITSTREAM}",
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
            expand("-s ${SERIAL} ${BITSTREAM}", &no_serial, Path::new("/b"))
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
        let c = catalog::builtin();
        let d = device("0403", "6010", Some("Alhambra II"), None, 9);
        let bin = Path::new("/pkg/bin");
        let v = &c.boards["alhambra-ii"].variants["flash"];
        let (prog, args) = command(v, None, &d, Path::new("/b/hardware.bin"), bin).unwrap();
        assert_eq!(prog, Path::new("/pkg/bin/openFPGALoader"));
        let args: Vec<String> = args.into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args.join(" "),
            "--force-terminal-mode --verify -b ice40_generic --vid 0403 --pid 6010 --busdev-num 1:9 /b/hardware.bin"
        );
        let v = &c.boards["icestick"].variants["flash"];
        let (prog, args) = command(v, None, &d, Path::new("/b/hardware.bin"), bin).unwrap();
        assert_eq!(prog, Path::new("/pkg/bin/iceprog"));
        assert_eq!(args.len(), 3); // -d d:1/9 /b/hardware.bin
    }

    #[test]
    fn custom_commands() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("iceprog"), "").unwrap();
        let d = device("0403", "6010", None, None, 2);
        let v = &catalog::builtin().boards["alhambra-ii"].variants["flash"];
        let b = Path::new("/b/hardware.bin");
        let (p, a) = command(v, Some("iceprog -S ${BITSTREAM}"), &d, b, tmp.path()).unwrap();
        assert_eq!(p, tmp.path().join("iceprog"));
        assert_eq!(
            a,
            vec![OsString::from("-S"), OsString::from("/b/hardware.bin")]
        );
        let (p, _) = command(v, Some("/opt/x/prog ${BITSTREAM}"), &d, b, tmp.path()).unwrap();
        assert_eq!(p, Path::new("/opt/x/prog"));
        for bad in ["rm ${BITSTREAM}", "../iceprog ${BITSTREAM}", "iceprog"] {
            assert_eq!(
                command(v, Some(bad), &d, b, tmp.path())
                    .unwrap_err()
                    .category,
                Category::Request,
                "{bad}"
            );
        }
    }

    #[test]
    fn progress_lines() {
        assert_eq!(
            progress_line("Writing: [====>     ] 45.33%"),
            ProgressLine::Percent(45.33)
        );
        assert_eq!(progress_line("Erasing: 100%"), ProgressLine::Percent(100.0));
        assert_eq!(
            progress_line("Verifying write (May take time)"),
            ProgressLine::Verifying(None)
        );
        assert_eq!(progress_line("reading.."), ProgressLine::Verifying(None));
        assert_eq!(
            progress_line("Verifying: 50.00%"),
            ProgressLine::Verifying(Some(50.0))
        );
        assert_eq!(progress_line("cdone: high"), ProgressLine::Other);
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
        let e = failure("openFPGALoader", "Alhambra II", "segfault", "signal 11");
        assert_eq!(e.category, Category::Tool);
        assert_eq!(e.details, "segfault");
    }
}
