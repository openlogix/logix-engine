//! Device changes that need administrator rights (docs/tool-api.md,
//! install_driver and uninstall_driver). The engine runs itself again as an
//! administrator for exactly one operation, through the system's own
//! permission prompt.
//!
//! On Windows, through UAC, which names Logix Engine:
//!
//! - install WinUSB on a device's interface with logix-core's wdi-simple
//!   (it must run elevated as a whole: libwdi signs the driver package it
//!   writes only when its process already is, libwdi.c, wdi_prepare_driver),
//! - remove driver packages libwdi installed (pnputil) and the
//!   certificates it signed them with, then ask Windows to look for drivers
//!   again.
//!
//! On Linux, through pkexec (polkit), whose password dialog the desktop
//! shows, from Logix IDE and from Logix Engine under an AI client alike:
//!
//! - add Logix Engine's udev rule for a pair of USB IDs, or remove it, then
//!   ask udev to apply its rules again to the devices.
//!
//! Without a desktop to ask (no polkit agent, such as over SSH) or without
//! pkexec, it's a setup error with the commands the user can run instead.
//!
//! The request is a JSON file in a fresh temporary folder; the helper writes
//! its result, with every program it ran, its exit code and output, beside
//! it. The helper is narrow on purpose: typed fields, checked again before
//! use, never a command line from the request.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

use crate::errors::ToolError;

/// The engine's hidden command-line option that runs the helper (internal,
/// not for hosts): `logix-engine --elevated-driver-helper <request.json>`.
pub const OPTION: &str = "--elevated-driver-helper";

/// What the helper is asked to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Request {
    /// Install WinUSB with wdi-simple.
    InstallDriver {
        /// wdi-simple.exe, in logix-core's wdi-simple folder.
        wdi_simple: PathBuf,
        vid: String,
        pid: String,
        /// The interface, for a composite device; None for a
        /// single-interface one.
        interface: Option<u8>,
        /// The device's name as Device Manager will show it.
        name: String,
        /// Where libwdi writes the driver package (the temporary folder).
        dest: PathBuf,
    },
    /// Remove libwdi's driver packages and certificates for a device.
    UninstallDriver {
        /// oem<N>.inf names, from drivers::libwdi_packages.
        packages: Vec<String>,
        /// libwdi's certificate subject, drivers::certificate_subject.
        certificate: String,
    },
    /// Add Logix Engine's udev rule for these USB IDs (Linux). The helper
    /// makes the file's name and text itself (drivers::udev_rule_name and
    /// udev_rule_text).
    AddUdevRule {
        vid: String,
        pid: String,
        /// The board, for the rule's comment.
        name: String,
    },
    /// Remove Logix Engine's udev rule for these USB IDs (Linux).
    RemoveUdevRule { vid: String, pid: String },
}

/// One program the helper ran, or one step it took itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// What ran, for the log, such as "pnputil /delete-driver oem12.inf".
    pub command: String,
    /// Its exit code; None when it couldn't be started.
    pub exit_code: Option<i32>,
    /// Its output and error output, or why it couldn't start.
    pub output: String,
}

/// What the helper did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Response {
    pub steps: Vec<Step>,
    /// Why it refused or stopped, if it did.
    pub error: Option<String>,
}

impl Response {
    /// Every step, for an error's details and the log.
    pub fn details(&self) -> String {
        self.steps
            .iter()
            .map(|s| {
                let code = s
                    .exit_code
                    .map_or("not started".to_string(), |c| format!("exit code {c}"));
                format!("$ {} ({code})\n{}", s.command, s.output.trim_end())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The program a step ran, as live output names tools: its file name
/// without folder or `.exe` (`wdi-simple`, `pnputil`), or `logix-engine`
/// for the helper's own steps, such as removing a certificate.
pub fn step_tool(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("");
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    match name {
        "wdi-simple" | "pnputil" | "udevadm" => name.to_string(),
        _ => "logix-engine".to_string(),
    }
}

/// Sends what the helper did to the live output, step by step: its command
/// line, then its output lines. The helper runs hidden, as an
/// administrator, so its output comes once it's done.
pub fn report(response: &Response, output: &dyn crate::progress::OutputSink) {
    use crate::progress::{OutputLevel, ToolLine};
    for step in &response.steps {
        let tool = step_tool(&step.command);
        output.command(&tool, &step.command);
        for text in step
            .output
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.is_empty())
        {
            output.line(&ToolLine {
                tool: &tool,
                level: OutputLevel::of(text),
                text,
                redraw: false,
                original: None,
            });
        }
    }
}

/// The result file beside a request file.
pub fn result_path(request: &Path) -> PathBuf {
    request.with_extension("result.json")
}

/// The file the helper writes first, beside the request: on Linux, the
/// sign that the user gave permission and the helper runs.
pub fn started_path(request: &Path) -> PathBuf {
    request.with_extension("started")
}

/// Checks a request before anything runs: the helper never trusts it more
/// than the engine that wrote it.
pub fn check(request: &Request) -> Result<(), String> {
    let hex4 = |s: &str| s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit());
    match request {
        Request::InstallDriver {
            wdi_simple,
            vid,
            pid,
            name,
            ..
        } => {
            let file = wdi_simple
                .file_name()
                .map(|f| f.to_string_lossy().to_ascii_lowercase());
            let folder = wdi_simple
                .parent()
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_ascii_lowercase());
            if file.as_deref() != Some("wdi-simple.exe")
                || folder.as_deref() != Some("wdi-simple")
                || !wdi_simple.is_absolute()
            {
                return Err(format!(
                    "not logix-core's wdi-simple: {}",
                    wdi_simple.display()
                ));
            }
            if !hex4(vid) || !hex4(pid) {
                return Err(format!("not USB IDs: {vid}:{pid}"));
            }
            if name.is_empty() || name.chars().any(|c| c.is_control() || c == '"') {
                return Err(format!("not a device name: {name:?}"));
            }
        }
        Request::UninstallDriver {
            packages,
            certificate,
        } => {
            if let Some(p) = packages
                .iter()
                .find(|p| !crate::drivers::is_package_name(p))
            {
                return Err(format!("not a driver package: {p:?}"));
            }
            let ok = certificate.starts_with("USB\\VID_")
                && certificate.ends_with(" (libwdi autogenerated)")
                && !certificate.chars().any(|c| c.is_control() || c == '"');
            if !ok {
                return Err(format!("not a libwdi certificate: {certificate:?}"));
            }
        }
        Request::AddUdevRule { vid, pid, name } => {
            if !hex4(vid) || !hex4(pid) {
                return Err(format!("not USB IDs: {vid}:{pid}"));
            }
            if name.is_empty() || name.chars().any(|c| c.is_control()) {
                return Err(format!("not a device name: {name:?}"));
            }
        }
        Request::RemoveUdevRule { vid, pid } => {
            if !hex4(vid) || !hex4(pid) {
                return Err(format!("not USB IDs: {vid}:{pid}"));
            }
        }
    }
    Ok(())
}

/// The helper's entry point (main.rs, with OPTION): reads the request,
/// runs it, writes the result. The exit code says only whether the result
/// was written.
pub fn helper_main(request_path: &Path) -> ExitCode {
    let _ = std::fs::write(started_path(request_path), "");
    let response = match std::fs::read_to_string(request_path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str::<Request>(&t).map_err(|e| e.to_string()))
    {
        Ok(request) => match check(&request) {
            Ok(()) => run(&request),
            Err(e) => Response {
                steps: Vec::new(),
                error: Some(e),
            },
        },
        Err(e) => Response {
            steps: Vec::new(),
            error: Some(format!("can't read the request: {e}")),
        },
    };
    let text = serde_json::to_string_pretty(&response).unwrap_or_default();
    match std::fs::write(result_path(request_path), text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Runs a program, capturing its output.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn step(program: &Path, args: &[std::ffi::OsString]) -> Step {
    let command = std::iter::once(program.display().to_string())
        .chain(args.iter().map(|a| a.to_string_lossy().into_owned()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut cmd = std::process::Command::new(program);
    // The helper runs with its console hidden; its programs share it.
    cmd.args(args);
    match cmd.output() {
        Ok(out) => Step {
            command,
            exit_code: out.status.code(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        },
        Err(e) => Step {
            command,
            exit_code: None,
            output: e.to_string(),
        },
    }
}

#[cfg(windows)]
fn run(request: &Request) -> Response {
    let mut response = Response::default();
    let system32 =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32");
    match request {
        Request::InstallDriver {
            wdi_simple,
            vid,
            pid,
            interface,
            name,
            dest,
        } => {
            let args = crate::drivers::wdi_simple_args(
                vid,
                pid,
                interface.unwrap_or(0),
                interface.is_some(),
                name,
                dest,
            );
            response.steps.push(step(wdi_simple, &args));
        }
        Request::AddUdevRule { .. } | Request::RemoveUdevRule { .. } => {
            response.error = Some("udev rules are changed only on Linux".into());
        }
        Request::UninstallDriver {
            packages,
            certificate,
        } => {
            let pnputil = system32.join("pnputil.exe");
            for p in packages {
                let args: Vec<std::ffi::OsString> = vec![
                    "/delete-driver".into(),
                    p.into(),
                    "/uninstall".into(),
                    "/force".into(),
                ];
                response.steps.push(step(&pnputil, &args));
            }
            for store in ["Root", "TrustedPublisher"] {
                response
                    .steps
                    .push(windows::delete_certificates(store, certificate));
            }
            response
                .steps
                .push(step(&pnputil, &["/scan-devices".into()]));
        }
    }
    response
}

#[cfg(target_os = "linux")]
fn run(request: &Request) -> Response {
    use crate::drivers::{UDEV_RULES_FOLDER, udev_rule_name, udev_rule_text};
    let mut response = Response::default();
    let (vid, pid) = match request {
        Request::AddUdevRule { vid, pid, name } => {
            let path = Path::new(UDEV_RULES_FOLDER).join(udev_rule_name(vid, pid));
            let written = std::fs::write(&path, udev_rule_text(vid, pid, name));
            response.steps.push(Step {
                command: format!("write {}", path.display()),
                exit_code: Some(if written.is_ok() { 0 } else { 1 }),
                output: written.err().map(|e| e.to_string()).unwrap_or_default(),
            });
            (vid, pid)
        }
        Request::RemoveUdevRule { vid, pid } => {
            let path = Path::new(UDEV_RULES_FOLDER).join(udev_rule_name(vid, pid));
            let removed = match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
            response.steps.push(Step {
                command: format!("delete {}", path.display()),
                exit_code: Some(if removed.is_ok() { 0 } else { 1 }),
                output: removed.err().map(|e| e.to_string()).unwrap_or_default(),
            });
            (vid, pid)
        }
        _ => {
            response.error = Some("drivers are changed only on Windows".into());
            return response;
        }
    };
    if response.steps.iter().any(|s| s.exit_code != Some(0)) {
        return response;
    }
    let Some(udevadm) = [
        "/usr/bin/udevadm",
        "/bin/udevadm",
        "/usr/sbin/udevadm",
        "/sbin/udevadm",
    ]
    .iter()
    .map(Path::new)
    .find(|p| p.is_file()) else {
        response.error = Some("udevadm wasn't found".into());
        return response;
    };
    let os = |a: &[&str]| a.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
    response
        .steps
        .push(step(udevadm, &os(&["control", "--reload-rules"])));
    // The device itself (libusb's programmers), then the HID nodes (their
    // own USB IDs are their parent's, which --attr-match can't see).
    response.steps.push(step(
        udevadm,
        &os(&[
            "trigger",
            "--action=change",
            "--subsystem-match=usb",
            &format!("--attr-match=idVendor={vid}"),
            &format!("--attr-match=idProduct={pid}"),
        ]),
    ));
    response.steps.push(step(
        udevadm,
        &os(&["trigger", "--action=change", "--subsystem-match=hidraw"]),
    ));
    // Removing a rule takes away no access a plugged-in device already got,
    // so reconnect it in software, as a replug does: it then gets only what
    // the remaining rules give (the user's request, 2026-11-17).
    if matches!(request, Request::RemoveUdevRule { .. }) {
        let root = Path::new(SYSFS_USB_DEVICES);
        let devices = usb_devices_with_ids(root, vid, pid);
        for dir in &devices {
            response.steps.extend(reconnect(dir));
        }
        if !devices.is_empty() {
            response.steps.push(wait_for_devices(
                root,
                vid,
                pid,
                devices.len(),
                std::time::Duration::from_secs(10),
            ));
        }
    }
    response
        .steps
        .push(step(udevadm, &os(&["settle", "--timeout=10"])));
    response
}

/// Where Linux lists the USB devices, one folder each.
#[cfg(target_os = "linux")]
const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";

/// Reconnects a USB device in software, as unplugging it and plugging it
/// in again does: its hub port off, then on again (the port's `disable`
/// file, Linux 4.20 and later). The kernel removes the device and finds it
/// again under a new device number, so udev makes its device files again
/// with only the access the current rules give. Its `authorized` file
/// isn't enough: 0 then 1 only detaches and attaches its drivers, and its
/// device file keeps the old mode, group and ACL (found on Ubuntu 22.04,
/// 2026-11-18).
#[cfg(target_os = "linux")]
fn reconnect(dir: &Path) -> Vec<Step> {
    // Found now: the device's own folder goes while its port is off.
    let disable = match std::fs::canonicalize(dir.join("port")) {
        Ok(port) => port.join("disable"),
        Err(e) => {
            return vec![Step {
                command: format!("find the hub port of {}", dir.display()),
                exit_code: Some(1),
                output: e.to_string(),
            }];
        }
    };
    let mut steps = Vec::new();
    for value in ["1", "0"] {
        let written = std::fs::write(&disable, value);
        let ok = written.is_ok();
        steps.push(Step {
            command: format!("write {value} to {}", disable.display()),
            exit_code: Some(if ok { 0 } else { 1 }),
            output: written.err().map(|e| e.to_string()).unwrap_or_default(),
        });
        if !ok {
            break;
        }
        if value == "1" {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    steps
}

/// Waits until `count` devices with these USB IDs are connected again
/// under `root`, after their ports were turned on again: the kernel finds
/// a device a moment later. A step that fails after `timeout`.
#[cfg(target_os = "linux")]
fn wait_for_devices(
    root: &Path,
    vid: &str,
    pid: &str,
    count: usize,
    timeout: std::time::Duration,
) -> Step {
    let start = std::time::Instant::now();
    loop {
        let found = usb_devices_with_ids(root, vid, pid).len();
        if found >= count || start.elapsed() >= timeout {
            return Step {
                command: format!("wait for {count} {vid}:{pid} device(s) to reconnect"),
                exit_code: Some(if found >= count { 0 } else { 1 }),
                output: if found >= count {
                    String::new()
                } else {
                    format!("{found} reconnected after {} s", timeout.as_secs())
                },
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The folders of the connected USB devices with these USB IDs, under
/// `root` (`/sys/bus/usb/devices`; another folder in tests): those whose
/// `idVendor` and `idProduct` files say so.
pub fn usb_devices_with_ids(root: &Path, vid: &str, pid: &str) -> Vec<PathBuf> {
    let read = |dir: &Path, file: &str| {
        std::fs::read_to_string(dir.join(file))
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|dir| {
            read(dir, "idVendor") == vid.to_ascii_lowercase()
                && read(dir, "idProduct") == pid.to_ascii_lowercase()
        })
        .collect();
    out.sort();
    out
}

#[cfg(not(any(windows, target_os = "linux")))]
fn run(_request: &Request) -> Response {
    Response {
        steps: Vec::new(),
        error: Some("nothing is changed on this system".into()),
    }
}

/// Runs `request` in the helper, as an administrator, and returns what it
/// did. Windows asks the user for permission first; a refusal is a setup
/// error. Blocks until the helper ends (call it from a blocking task).
/// Runs a request in the helper, as an administrator. `on_started` is
/// called once Windows has the user's permission and the helper runs.
pub fn run_elevated(request: &Request, on_started: impl FnOnce()) -> Result<Response, ToolError> {
    check(request).map_err(|e| ToolError::engine(format!("Bad driver request: {e}")))?;
    let folder = TempFolder::new().ok_or_else(|| {
        ToolError::engine("Can't create a temporary folder for the driver change.")
    })?;
    let request_path = folder.0.join("request.json");
    std::fs::write(
        &request_path,
        serde_json::to_string_pretty(request).unwrap_or_default(),
    )
    .map_err(|e| ToolError::engine(format!("Can't write the driver request: {e}")))?;
    let exe = std::env::current_exe()
        .map_err(|e| ToolError::engine(format!("Can't find the engine's own program: {e}")))?;
    launch(&exe, &request_path, request, on_started)?;
    let text = std::fs::read_to_string(result_path(&request_path))
        .map_err(|e| ToolError::engine(format!("The driver helper left no result: {e}")))?;
    let response: Response = serde_json::from_str(&text)
        .map_err(|e| ToolError::engine(format!("The driver helper's result is unreadable: {e}")))?;
    log::info!("Driver helper:\n{}", response.details());
    Ok(response)
}

/// A temporary folder for one driver change, deleted afterwards.
struct TempFolder(PathBuf);

impl TempFolder {
    fn new() -> Option<TempFolder> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("logix-engine-driver-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(TempFolder(dir))
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// How long a driver change may take: libwdi waits for Windows' own
/// installation, which can be slow on a first install. On Linux, it
/// includes the time the user takes to answer the password dialog.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const TIMEOUT_MS: u32 = 10 * 60 * 1000;

#[cfg(windows)]
fn launch(
    exe: &Path,
    request_path: &Path,
    _request: &Request,
    on_started: impl FnOnce(),
) -> Result<(), ToolError> {
    windows::launch_elevated(exe, request_path, on_started)
}

#[cfg(target_os = "linux")]
fn launch(
    exe: &Path,
    request_path: &Path,
    request: &Request,
    on_started: impl FnOnce(),
) -> Result<(), ToolError> {
    linux::launch_elevated(exe, request_path, request, on_started)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn launch(
    _exe: &Path,
    _request_path: &Path,
    _request: &Request,
    _on_started: impl FnOnce(),
) -> Result<(), ToolError> {
    Err(ToolError::request("Nothing needs changing on this system."))
}

/// The setup error when Linux can't ask the user for permission (no pkexec,
/// or no password dialog, such as over SSH): the commands that do the same
/// in a terminal, in its message's details.
pub fn no_prompt_error(request: &Request, why: &str) -> ToolError {
    let commands = match request {
        Request::AddUdevRule { vid, pid, name } => {
            crate::drivers::udev_rule_commands(vid, pid, name)
        }
        Request::RemoveUdevRule { vid, pid } => format!(
            "sudo rm {}/{}\nsudo udevadm control --reload-rules\nsudo udevadm trigger",
            crate::drivers::UDEV_RULES_FOLDER,
            crate::drivers::udev_rule_name(vid, pid)
        ),
        _ => String::new(),
    };
    ToolError::setup(format!(
        "Linux can't ask for your password here ({why}). Run the commands in the details in a terminal instead."
    ))
    .with_details(commands)
    .with_action(
        crate::errors::ActionCode::FixDevicePermissions,
        "Run the commands in the details in a terminal (sudo asks for the user's password), then list the devices again.",
    )
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::{Request, TIMEOUT_MS, no_prompt_error, started_path};
    use crate::errors::{ActionCode, ToolError};

    /// Runs `exe OPTION request` as root through pkexec, which asks the
    /// user with the desktop's password dialog, and waits for it.
    /// `on_started` is called once the helper runs (it writes its started
    /// file first), so after the user gave permission.
    pub fn launch_elevated(
        exe: &Path,
        request_path: &Path,
        request: &Request,
        on_started: impl FnOnce(),
    ) -> Result<(), ToolError> {
        let Some(pkexec) = ["/usr/bin/pkexec", "/bin/pkexec"]
            .iter()
            .map(Path::new)
            .find(|p| p.is_file())
        else {
            return Err(no_prompt_error(request, "pkexec isn't installed"));
        };
        let mut child = std::process::Command::new(pkexec)
            .arg(exe)
            .arg(super::OPTION)
            .arg(request_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ToolError::setup(format!("Can't run pkexec: {e}")))?;
        let start = Instant::now();
        let mut on_started = Some(on_started);
        let status = loop {
            if on_started.is_some() && started_path(request_path).exists() {
                (on_started.take().unwrap())();
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => return Err(ToolError::engine(format!("Can't wait for pkexec: {e}"))),
            }
            if start.elapsed() > Duration::from_millis(TIMEOUT_MS.into()) {
                let _ = child.kill();
                return Err(ToolError::setup(
                    "The change took too long. Unplug the board, plug it in again and retry.",
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let output = child
            .wait_with_output()
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default();
        log::info!("pkexec exited with {status}: {}", output.trim());
        if started_path(request_path).exists() {
            if let Some(f) = on_started.take() {
                f();
            }
            return Ok(());
        }
        // pkexec: 126 when the user dismissed the dialog, 127 when they
        // couldn't be asked (no polkit agent) or weren't authorized.
        match status.code() {
            Some(126) => Err(ToolError::setup(
                "Linux didn't get permission to change the device's access.",
            )
            .with_action(
                ActionCode::FixDevicePermissions,
                "Try again, and enter your password when Linux asks for it.",
            )),
            Some(127) if output.contains("No authentication agent") => Err(no_prompt_error(
                request,
                "no password dialog, such as over SSH",
            )),
            _ => Err(ToolError::setup(format!(
                "Linux didn't let Logix Engine change the device's access: {}",
                output.trim()
            ))
            .with_action(
                ActionCode::FixDevicePermissions,
                "Try again, and enter your password when Linux asks for it.",
            )),
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows_sys::Win32::Security::Cryptography::{
        CERT_FIND_SUBJECT_STR_W, CERT_STORE_PROV_SYSTEM_W, CERT_SYSTEM_STORE_LOCAL_MACHINE,
        CertCloseStore, CertDeleteCertificateFromStore, CertFindCertificateInStore, CertOpenStore,
        PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    use super::{Step, TIMEOUT_MS};
    use crate::errors::{ActionCode, ToolError};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Runs `exe OPTION request` as an administrator ("runas": Windows asks
    /// the user) and waits for it.
    pub fn launch_elevated(
        exe: &Path,
        request: &Path,
        on_started: impl FnOnce(),
    ) -> Result<(), ToolError> {
        let verb = wide("runas".as_ref());
        let file = wide(exe.as_os_str());
        let params = wide(format!("{} \"{}\"", super::OPTION, request.display()).as_ref());
        // SAFETY: the structure is zeroed then filled with valid, live
        // strings; the process handle is waited on and closed.
        unsafe {
            let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = params.as_ptr();
            info.nShow = SW_HIDE;
            if ShellExecuteExW(&mut info) == 0 {
                let err = GetLastError();
                if err == ERROR_CANCELLED {
                    return Err(ToolError::setup(
                        "Windows didn't get permission to change the driver.",
                    )
                    .with_action(
                        ActionCode::InstallDrivers,
                        "Try again, and answer Yes when Windows asks whether Logix Engine may make changes.",
                    ));
                }
                return Err(ToolError::setup(format!(
                    "Can't run the driver helper as an administrator (Windows error {err})."
                )));
            }
            if info.hProcess.is_null() {
                return Err(ToolError::engine("The driver helper didn't start."));
            }
            on_started();
            let waited = WaitForSingleObject(info.hProcess, TIMEOUT_MS);
            let mut code = 0u32;
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
            if waited != 0 {
                return Err(ToolError::setup(
                    "The driver change took too long. Unplug the board, plug it in again and retry.",
                ));
            }
            log::info!("Driver helper exited with {code}");
        }
        Ok(())
    }

    /// Deletes every certificate whose subject has `subject` from the local
    /// machine's `store`.
    pub fn delete_certificates(store: &str, subject: &str) -> Step {
        let command = format!("delete certificate {subject:?} from {store}");
        let store_name = wide(store.as_ref());
        let subject_w = wide(subject.as_ref());
        let mut deleted = 0;
        // SAFETY: a system store opened, searched and closed; each found
        // context is consumed by CertDeleteCertificateFromStore, so the
        // search restarts from the beginning.
        unsafe {
            let handle = CertOpenStore(
                CERT_STORE_PROV_SYSTEM_W,
                0,
                0,
                CERT_SYSTEM_STORE_LOCAL_MACHINE,
                store_name.as_ptr().cast(),
            );
            if handle.is_null() {
                return Step {
                    command,
                    exit_code: None,
                    output: format!("can't open the store (Windows error {})", GetLastError()),
                };
            }
            loop {
                let found = CertFindCertificateInStore(
                    handle,
                    X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                    0,
                    CERT_FIND_SUBJECT_STR_W,
                    subject_w.as_ptr().cast(),
                    std::ptr::null(),
                );
                if found.is_null() || CertDeleteCertificateFromStore(found) == 0 {
                    break;
                }
                deleted += 1;
            }
            CertCloseStore(handle, 0);
        }
        Step {
            command,
            exit_code: Some(0),
            output: format!("{deleted} deleted"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install() -> Request {
        Request::InstallDriver {
            wdi_simple: if cfg!(windows) {
                "C:\\core\\wdi-simple\\wdi-simple.exe".into()
            } else {
                "/core/wdi-simple/wdi-simple.exe".into()
            },
            vid: "0403".into(),
            pid: "6010".into(),
            interface: Some(0),
            name: "Alhambra II".into(),
            dest: "/tmp/x".into(),
        }
    }

    #[test]
    fn requests_round_trip_and_are_checked() {
        let r = install();
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"op\":\"install-driver\""), "{text}");
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), r);
        assert_eq!(check(&r), Ok(()));

        let mut bad = install();
        if let Request::InstallDriver { wdi_simple, .. } = &mut bad {
            *wdi_simple = "/core/bin/cmd.exe".into();
        }
        assert!(check(&bad).is_err());
        let mut bad = install();
        if let Request::InstallDriver { name, .. } = &mut bad {
            *name = "x\" & del".into();
        }
        assert!(check(&bad).is_err());

        let un = Request::UninstallDriver {
            packages: vec!["oem12.inf".into()],
            certificate: "USB\\VID_0403&PID_6010&MI_00 (libwdi autogenerated)".into(),
        };
        assert_eq!(check(&un), Ok(()));
        let bad = Request::UninstallDriver {
            packages: vec!["usb.inf".into()],
            certificate: "USB\\VID_0403&PID_6010&MI_00 (libwdi autogenerated)".into(),
        };
        assert!(check(&bad).is_err());
        let bad = Request::UninstallDriver {
            packages: vec![],
            certificate: "Microsoft Root".into(),
        };
        assert!(check(&bad).is_err());
    }

    #[test]
    fn the_helper_writes_its_result_beside_the_request() {
        let tmp = tempfile::tempdir().unwrap();
        let req = tmp.path().join("request.json");
        std::fs::write(&req, "{\"op\":\"format-disk\"}").unwrap();
        assert_eq!(helper_main(&req), ExitCode::SUCCESS);
        let res: Response =
            serde_json::from_str(&std::fs::read_to_string(result_path(&req)).unwrap()).unwrap();
        assert!(res.error.unwrap().contains("can't read the request"));
        assert!(res.steps.is_empty());
    }

    #[test]
    fn udev_requests_are_checked() {
        let add = Request::AddUdevRule {
            vid: "0403".into(),
            pid: "6010".into(),
            name: "the Alhambra II".into(),
        };
        let text = serde_json::to_string(&add).unwrap();
        assert!(text.contains("\"op\":\"add-udev-rule\""), "{text}");
        assert_eq!(check(&add), Ok(()));
        let bad_ids = Request::RemoveUdevRule {
            vid: "0403/..".into(),
            pid: "6010".into(),
        };
        assert!(check(&bad_ids).is_err());
        let bad_name = Request::AddUdevRule {
            vid: "0403".into(),
            pid: "6010".into(),
            name: "x\nSUBSYSTEM==\"usb\", MODE=\"0666\"".into(),
        };
        assert!(check(&bad_name).is_err());
    }

    #[test]
    fn without_a_password_dialog_the_commands_are_given() {
        let add = Request::AddUdevRule {
            vid: "0403".into(),
            pid: "6010".into(),
            name: "the Alhambra II".into(),
        };
        let err = no_prompt_error(&add, "no password dialog, such as over SSH");
        assert_eq!(err.category, crate::errors::Category::Setup);
        assert!(err.message.contains("over SSH"), "{}", err.message);
        assert_eq!(
            err.details,
            crate::drivers::udev_rule_commands("0403", "6010", "the Alhambra II")
        );
        let remove = Request::RemoveUdevRule {
            vid: "0403".into(),
            pid: "6010".into(),
        };
        let err = no_prompt_error(&remove, "pkexec isn't installed");
        assert!(
            err.details
                .starts_with("sudo rm /etc/udev/rules.d/70-logix-engine-0403-6010.rules\n"),
            "{}",
            err.details
        );
        assert_eq!(
            err.action.unwrap().code,
            crate::errors::ActionCode::FixDevicePermissions
        );
    }

    #[test]
    fn finds_usb_devices_by_their_ids() {
        let tmp = tempfile::tempdir().unwrap();
        for (name, vid, pid) in [
            ("1-2", "0403", "6010"),
            ("1-3", "1d50", "602b"),
            ("1-4", "0403", "6010"),
        ] {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("idVendor"), format!("{vid}\n")).unwrap();
            std::fs::write(dir.join("idProduct"), format!("{pid}\n")).unwrap();
        }
        // An interface folder has no IDs of its own (Linux names it
        // "1-2:1.0"; a name Windows allows here, where the tests run too).
        std::fs::create_dir_all(tmp.path().join("1-2.1.0")).unwrap();
        let found = usb_devices_with_ids(tmp.path(), "0403", "6010");
        assert_eq!(found, [tmp.path().join("1-2"), tmp.path().join("1-4")]);
        assert!(usb_devices_with_ids(tmp.path(), "dead", "beef").is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reconnects_a_device_by_turning_its_port_off_and_on() {
        let tmp = tempfile::tempdir().unwrap();
        let port = tmp.path().join("1-3.4.3/1-3.4.3:1.0/1-3.4.3-port1");
        std::fs::create_dir_all(&port).unwrap();
        std::fs::write(port.join("disable"), "0\n").unwrap();
        let device = tmp.path().join("1-3.4.3.1");
        std::fs::create_dir_all(&device).unwrap();
        std::os::unix::fs::symlink(&port, device.join("port")).unwrap();
        let steps = reconnect(&device);
        let disable = std::fs::canonicalize(&port).unwrap().join("disable");
        let commands: Vec<_> = steps.iter().map(|s| s.command.clone()).collect();
        assert_eq!(
            commands,
            [
                format!("write 1 to {}", disable.display()),
                format!("write 0 to {}", disable.display()),
            ]
        );
        assert!(steps.iter().all(|s| s.exit_code == Some(0)));
        assert_eq!(std::fs::read_to_string(&disable).unwrap(), "0");

        // No port (a root hub, or a kernel without it): one failed step.
        let other = tmp.path().join("1-5");
        std::fs::create_dir_all(&other).unwrap();
        let steps = reconnect(&other);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].exit_code, Some(1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn waits_for_reconnected_devices() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("1-2");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("idVendor"), "0403\n").unwrap();
        std::fs::write(dir.join("idProduct"), "6010\n").unwrap();
        let short = std::time::Duration::from_millis(200);
        assert_eq!(
            wait_for_devices(tmp.path(), "0403", "6010", 1, short).exit_code,
            Some(0)
        );
        let missing = wait_for_devices(tmp.path(), "0403", "6010", 2, short);
        assert_eq!(missing.exit_code, Some(1));
        assert!(
            missing.output.contains("1 reconnected"),
            "{}",
            missing.output
        );
    }

    #[test]
    fn steps_name_their_tools() {
        assert_eq!(
            step_tool(r"C:\\x\\wdi-simple\\wdi-simple.exe --vid 0x0403"),
            "wdi-simple"
        );
        assert_eq!(step_tool("pnputil /scan-devices"), "pnputil");
        assert_eq!(step_tool("/usr/bin/udevadm settle"), "udevadm");
        assert_eq!(
            step_tool("write /etc/udev/rules.d/70-logix-engine-0403-6010.rules"),
            "logix-engine"
        );
        assert_eq!(
            step_tool("delete certificate \"x\" from Root"),
            "logix-engine"
        );
    }

    #[test]
    fn details_list_every_step() {
        let r = Response {
            steps: vec![Step {
                command: "pnputil /scan-devices".into(),
                exit_code: Some(0),
                output: "Scanning\n".into(),
            }],
            error: None,
        };
        assert_eq!(
            r.details(),
            "$ pnputil /scan-devices (exit code 0)\nScanning"
        );
    }
}
