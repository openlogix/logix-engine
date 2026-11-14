//! Driver changes that need administrator rights, on Windows (docs/tool-api.md,
//! install_driver and uninstall_driver). The engine runs itself again as an
//! administrator, through Windows' own permission prompt (UAC), which names
//! Logix Engine, for exactly one of two operations:
//!
//! - install WinUSB on a device's interface with logix-core's wdi-simple
//!   (it must run elevated as a whole: libwdi signs the driver package it
//!   writes only when its process already is, libwdi.c, wdi_prepare_driver),
//! - remove driver packages libwdi installed (pnputil) and the
//!   certificates it signed them with, then ask Windows to look for drivers
//!   again.
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

/// The result file beside a request file.
pub fn result_path(request: &Path) -> PathBuf {
    request.with_extension("result.json")
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
    }
    Ok(())
}

/// The helper's entry point (main.rs, with OPTION): reads the request,
/// runs it, writes the result. The exit code says only whether the result
/// was written.
pub fn helper_main(request_path: &Path) -> ExitCode {
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
#[cfg_attr(not(windows), allow(dead_code))]
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

#[cfg(not(windows))]
fn run(_request: &Request) -> Response {
    Response {
        steps: Vec::new(),
        error: Some("drivers are changed only on Windows".into()),
    }
}

/// Runs `request` in the helper, as an administrator, and returns what it
/// did. Windows asks the user for permission first; a refusal is a setup
/// error. Blocks until the helper ends (call it from a blocking task).
pub fn run_elevated(request: &Request) -> Result<Response, ToolError> {
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
    launch(&exe, &request_path)?;
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
/// installation, which can be slow on a first install.
#[cfg_attr(not(windows), allow(dead_code))]
const TIMEOUT_MS: u32 = 10 * 60 * 1000;

#[cfg(windows)]
fn launch(exe: &Path, request: &Path) -> Result<(), ToolError> {
    windows::launch_elevated(exe, request)
}

#[cfg(not(windows))]
fn launch(_exe: &Path, _request: &Path) -> Result<(), ToolError> {
    Err(ToolError::request("Drivers are changed only on Windows."))
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
    pub fn launch_elevated(exe: &Path, request: &Path) -> Result<(), ToolError> {
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
