//! The uninstall_driver tool (docs/tool-api.md, uninstall_driver): undoes
//! install_driver, as an administrator after the system asks the user
//! (src/elevated.rs). On Windows, removes the driver packages libwdi
//! installed for a board's USB interface, and the certificates libwdi
//! signed them with, then asks Windows to look for drivers again, so the
//! device gets back Windows' own choice (such as FTDI's). On Linux, removes
//! Logix Engine's udev rule for the device's USB IDs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::install_driver::{lookup, not_needed, settle};
use super::list_devices;
use super::{Call, EngineTool};
use crate::devices;
use crate::drivers::{self, DriverStatus};
use crate::elevated::{self, Request};
use crate::errors::{ActionCode, ToolError};
use crate::progress::Phases;
use crate::state::EngineState;

pub struct UninstallDriver;

impl EngineTool for UninstallDriver {
    const NAME: &'static str = "uninstall_driver";
    const DESCRIPTION: &'static str = "Windows and Linux. Undoes install_driver for a device \
        from list_devices. Ask the user first. On Windows: removes every driver package libwdi \
        installed for the board's USB interface (by WinUSB, and by Zadig with libusbK or \
        libusb0 from older guides; never Windows' or a vendor's), the certificates libwdi \
        signed them with, then asks Windows to look for drivers again, so the device gets \
        Windows' own choice back (such as FTDI's, for vendor tools); Windows asks whether \
        Logix Engine may make changes. On Linux: removes Logix Engine's udev rule for the \
        device's USB IDs (never another rule); the desktop asks for the user's password. It \
        applies to every device with the same USB IDs. Does nothing when there's nothing of \
        Logix Engine's to remove.";
    const READ_ONLY: bool = false;
    type Input = UninstallDriverInput;
    type Output = UninstallDriverOutput;
}

/// The device, and how to tell its board.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UninstallDriverInput {
    /// The device's id from list_devices, in devices or unreachable.
    pub device: String,
    /// The host's own boards, as for list_devices. Omitted means none.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// A successful build's folder, as for install_driver. Omitted means
    /// the device's best match.
    #[serde(default)]
    pub build_folder: Option<String>,
    /// How to name paths in what people read. Omitted means as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_paths: Option<crate::display_paths::DisplayPaths>,
}

/// What was done.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UninstallDriverOutput {
    /// The device's id now (it can change with its driver).
    pub device: String,
    /// The device's driver now, as Windows reports it after looking again;
    /// it may take a moment more to settle.
    pub driver: DriverStatus,
    /// What was removed: on Windows, the driver packages, by their names in
    /// Windows (oem<N>.inf); on Linux, the udev rule's file. Empty when there
    /// was nothing, so nothing was done.
    pub removed: Vec<String>,
}

pub async fn run(
    _state: &EngineState,
    input: UninstallDriverInput,
    call: &Call,
) -> Result<UninstallDriverOutput, ToolError> {
    if cfg!(target_os = "linux") {
        return run_linux(input, call).await;
    }
    if !cfg!(windows) {
        return Err(not_needed());
    }
    let find = lookup();
    let located = list_devices::locate(
        &input.device,
        input.custom_boards_folder.clone(),
        input.build_folder.clone(),
        devices::scan,
        &find,
    )?;
    let Some(driver) = located.driver.clone() else {
        return Err(ToolError::request(format!(
            "No driver is needed for {}: its programmer uses Windows' own.",
            located.who
        )));
    };
    let d = &located.device;
    let hardware_id =
        drivers::hardware_id(&d.vid, &d.pid, driver.interface.unwrap_or(0), d.composite);
    let packages = drivers::libwdi_packages(&drivers::inf_folder(), &hardware_id);
    if packages.is_empty() {
        return Ok(UninstallDriverOutput {
            device: d.id.clone(),
            driver,
            removed: Vec::new(),
        });
    }

    let phases = std::sync::Arc::new(Phases::new(call.progress.clone(), 3));
    phases.start("Waiting for permission");
    call.output.message(&format!(
        "Removing the WinUSB driver of {} ({}). Windows asks whether Logix Engine may make changes: answer Yes.",
        located.who,
        packages.join(", ")
    ));
    let request = Request::UninstallDriver {
        packages: packages.clone(),
        certificate: drivers::certificate_subject(&hardware_id),
    };
    let (started, said) = (phases.clone(), call.output.clone());
    let response = tokio::task::spawn_blocking(move || {
        elevated::run_elevated(&request, || {
            // Each phase its own line, so the last one says where it is.
            said.message("Permission given. Removing the driver.");
            started.start("Removing driver")
        })
    })
    .await
    .map_err(|e| ToolError::engine(format!("The driver removal stopped: {e}")))??;
    elevated::report(&response, call.output.as_ref());
    if let Some(e) = &response.error {
        return Err(ToolError::engine(format!("The driver helper refused: {e}"))
            .with_details(response.details()));
    }
    // pnputil's exit codes: 0, or 3010 when Windows wants a restart.
    let failed: Vec<&str> = response
        .steps
        .iter()
        .filter(|s| s.command.contains("/delete-driver"))
        .filter(|s| !matches!(s.exit_code, Some(0) | Some(3010)))
        .map(|s| s.command.as_str())
        .collect();
    if !failed.is_empty() {
        return Err(ToolError::setup("Couldn't remove the WinUSB driver.")
            .with_details(response.details())
            .with_action(ActionCode::InstallDrivers, "Unplug the board and retry."));
    }

    phases.start("Checking driver");
    call.output
        .message("Waiting for Windows to restore the device's original driver.");
    let (custom, build, id) = (
        input.custom_boards_folder.clone(),
        input.build_folder.clone(),
        input.device.clone(),
    );
    let (device, now) = tokio::task::spawn_blocking(move || {
        let find = lookup();
        settle(&id, custom, build, false, &find)
    })
    .await
    .map_err(|e| ToolError::engine(format!("The driver check stopped: {e}")))??;
    Ok(UninstallDriverOutput {
        device: device.id,
        driver: now.unwrap_or(DriverStatus {
            current: None,
            ready: false,
            ..driver
        }),
        removed: packages,
    })
}

/// uninstall_driver on Linux: removes Logix Engine's udev rule for the
/// device's USB IDs, through pkexec. The device may stay open to the user
/// until it's plugged in again.
async fn run_linux(
    input: UninstallDriverInput,
    call: &Call,
) -> Result<UninstallDriverOutput, ToolError> {
    let find = lookup();
    let located = list_devices::locate(
        &input.device,
        input.custom_boards_folder.clone(),
        input.build_folder.clone(),
        devices::scan,
        &find,
    )?;
    let d = &located.device;
    let driver = located
        .driver
        .clone()
        .unwrap_or_else(|| drivers::udev_status(d.reachable, None));
    let Some(rule) = drivers::udev_rule(&d.vid, &d.pid) else {
        return Ok(UninstallDriverOutput {
            device: d.id.clone(),
            driver,
            removed: Vec::new(),
        });
    };
    let path = format!("{}/{rule}", drivers::UDEV_RULES_FOLDER);
    let phases = std::sync::Arc::new(Phases::new(call.progress.clone(), 3));
    phases.start("Waiting for permission");
    call.output.message(&format!(
        "Removing the udev rule for {} ({path}). Linux asks for your password: enter it to allow.",
        located.who
    ));
    let request = Request::RemoveUdevRule {
        vid: d.vid.clone(),
        pid: d.pid.clone(),
    };
    let (started, said) = (phases.clone(), call.output.clone());
    let response = tokio::task::spawn_blocking(move || {
        elevated::run_elevated(&request, || {
            said.message("Permission given. Removing the rule.");
            started.start("Removing rule")
        })
    })
    .await
    .map_err(|e| ToolError::engine(format!("Removing the rule stopped: {e}")))??;
    elevated::report(&response, call.output.as_ref());
    if let Some(e) = &response.error {
        return Err(
            ToolError::engine(format!("The helper refused: {e}")).with_details(response.details())
        );
    }
    if response.steps.iter().any(|s| s.exit_code != Some(0)) {
        return Err(ToolError::setup("Couldn't remove the udev rule.")
            .with_details(response.details())
            .with_action(ActionCode::FixDevicePermissions, "Retry."));
    }

    phases.start("Checking access");
    let (custom, build, id) = (
        input.custom_boards_folder.clone(),
        input.build_folder.clone(),
        input.device.clone(),
    );
    let now = tokio::task::spawn_blocking(move || {
        let find = lookup();
        list_devices::locate(&id, custom, build, devices::scan, &find)
    })
    .await
    .map_err(|e| ToolError::engine(format!("The access check stopped: {e}")))?;
    let (device, driver) = match now {
        Ok(l) => (
            l.device.id,
            l.driver.unwrap_or(DriverStatus {
                current: None,
                ..driver
            }),
        ),
        Err(_) => (
            input.device,
            DriverStatus {
                current: None,
                ..driver
            },
        ),
    };
    Ok(UninstallDriverOutput {
        device,
        driver,
        removed: vec![path],
    })
}
