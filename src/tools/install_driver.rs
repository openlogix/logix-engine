//! The install_driver tool (docs/tool-api.md, install_driver): installs
//! WinUSB on the interface a board's programmer opens, on Windows, with
//! logix-core's wdi-simple, run as an administrator after Windows asks the
//! user (src/elevated.rs).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::list_devices::{self, DriverLookup, Located};
use super::{Call, EngineTool};
use crate::devices::{self, Device};
use crate::drivers::{self, DriverStatus};
use crate::elevated::{self, Request};
use crate::errors::{ActionCode, ToolError};
use crate::progress::Phases;
use crate::state::EngineState;

pub struct InstallDriver;

impl EngineTool for InstallDriver {
    const NAME: &'static str = "install_driver";
    const DESCRIPTION: &'static str = "Windows only. Installs WinUSB, the driver the board's \
        programmer needs, on the USB interface list_devices' driver names, for a device from \
        list_devices (in devices or unreachable) whose driver isn't ready. Ask the user first: \
        Windows then asks whether Logix Engine may make changes, and the user must answer Yes. \
        Windows gives drivers by USB IDs and interface, not per board, so other connected \
        devices with the same USB IDs get WinUSB too: the tool refuses while there are any, \
        unless allow_same_ids is true (ask the user to unplug them, or to confirm). Vendor \
        tools that use the interface (such as Vivado for a Digilent board) can't until \
        uninstall_driver. Does nothing when the driver is ready already.";
    const READ_ONLY: bool = false;
    type Input = InstallDriverInput;
    type Output = InstallDriverOutput;
}

/// The device, and how to tell its board.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallDriverInput {
    /// The device's id from list_devices, in devices or unreachable.
    pub device: String,
    /// The host's own boards, as for list_devices. Omitted means none.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// A successful build's folder, as an absolute path: the driver is the
    /// one the build's board and variant need, as list_devices' driver says
    /// with the same build_folder. Omitted means the device's best match.
    #[serde(default)]
    pub build_folder: Option<String>,
    /// Install even when other connected devices have the same USB IDs,
    /// which then get WinUSB too. Default false: refused, naming them.
    #[serde(default)]
    pub allow_same_ids: bool,
    /// How to name paths in what people read. Omitted means as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_paths: Option<crate::display_paths::DisplayPaths>,
}

/// What was done.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallDriverOutput {
    /// The device's id now: it can change once the driver lets the engine
    /// read its serial number. Use it, or list the devices again.
    pub device: String,
    /// The device's driver now (ready true).
    pub driver: DriverStatus,
    /// False when the driver was ready already, so nothing was done.
    pub installed: bool,
    /// Other connected devices with the same USB IDs, which got WinUSB too
    /// (only with allow_same_ids).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub same_ids: Vec<String>,
}

/// The driver lookup on this system.
pub fn lookup() -> impl Fn(&str, &str, u8, bool) -> Vec<Option<String>> {
    |v: &str, p: &str, i: u8, c: bool| drivers::current_drivers(v, p, i, c)
}

/// The refusal while other devices have the same USB IDs.
pub fn same_ids_error(located: &Located) -> ToolError {
    ToolError::request(format!(
        "Other connected devices have the same USB IDs ({}:{}) and would get WinUSB too: {}.",
        located.device.vid,
        located.device.pid,
        located.same_ids.join(", ")
    ))
    .with_action(
        ActionCode::DisconnectDevices,
        "Unplug the other devices and retry, or retry with allow_same_ids if the user agrees that they get WinUSB too.",
    )
}

/// The device's driver after a change, as Windows reports it once it has
/// settled (it may take a moment to bind the new driver).
pub fn settle(
    id: &str,
    input_custom: Option<String>,
    input_build: Option<String>,
    want_ready: bool,
    lookup: DriverLookup,
) -> Result<(Device, Option<DriverStatus>), ToolError> {
    let mut last = None;
    for attempt in 0..20 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        match list_devices::locate(
            id,
            input_custom.clone(),
            input_build.clone(),
            devices::scan,
            lookup,
        ) {
            Ok(l) => {
                let done = l.driver.as_ref().is_some_and(|d| d.ready == want_ready);
                last = Some((l.device, l.driver));
                if done {
                    break;
                }
            }
            Err(e) if attempt == 19 => return Err(e),
            Err(_) => {}
        }
    }
    last.ok_or_else(|| ToolError::setup("The device isn't connected anymore."))
}

pub async fn run(
    state: &EngineState,
    input: InstallDriverInput,
    call: &Call,
) -> Result<InstallDriverOutput, ToolError> {
    if !cfg!(windows) {
        return Err(not_windows());
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
            "The {} needs no driver: its programmer uses Windows' own.",
            located.board_name
        )));
    };
    if driver.ready {
        return Ok(InstallDriverOutput {
            device: located.device.id.clone(),
            driver,
            installed: false,
            same_ids: Vec::new(),
        });
    }
    if !located.same_ids.is_empty() && !input.allow_same_ids {
        return Err(same_ids_error(&located));
    }

    let needed = state
        .packages
        .needing_download(&[crate::packages::CORE])
        .await?;
    let phases = std::sync::Arc::new(Phases::new(call.progress.clone(), needed.len() + 3));
    state
        .packages
        .ensure(&needed, state.auto_download, &phases, &call.cancel)
        .await?;
    let wdi_simple = state
        .packages
        .bin_dir("wdi-simple")
        .map_err(|_| {
            ToolError::setup("The FPGA tools package has no driver installer (wdi-simple).")
                .with_action(
                    ActionCode::DownloadPackages,
                    "Check for updates to get a newer package, then retry.",
                )
        })?
        .join("wdi-simple.exe");

    phases.start("Waiting for permission");
    call.output.message(&format!(
        "Installing WinUSB for the {} (USB interface {}). Windows will ask whether Logix Engine may make changes: answer Yes. Installing can take a minute or two.",
        located.board_name, driver.interface
    ));
    let d = &located.device;
    // wdi-simple writes the driver package here; Windows keeps its own copy.
    let dest = std::env::temp_dir().join(format!("logix-engine-winusb-{}", std::process::id()));
    let request = Request::InstallDriver {
        wdi_simple,
        vid: d.vid.clone(),
        pid: d.pid.clone(),
        interface: d.composite.then_some(driver.interface),
        name: located.board_name.clone(),
        dest: dest.clone(),
    };
    let started = phases.clone();
    let response = tokio::task::spawn_blocking(move || {
        elevated::run_elevated(&request, || started.start("Installing driver"))
    })
    .await
    .map_err(|e| ToolError::engine(format!("The driver installation stopped: {e}")))??;
    elevated::report(&response, call.output.as_ref());
    // Best effort: files the administrator wrote may stay until Windows
    // cleans its temporary folder.
    let _ = std::fs::remove_dir_all(&dest);
    if let Some(e) = &response.error {
        return Err(ToolError::engine(format!("The driver helper refused: {e}"))
            .with_details(response.details()));
    }
    let code = response.steps.first().and_then(|s| s.exit_code);
    if code != Some(0) {
        let why = code.map_or("it couldn't start", drivers::wdi_error);
        return Err(ToolError::setup(format!("Couldn't install WinUSB: {why}."))
            .with_details(response.details())
            .with_action(
                ActionCode::InstallDrivers,
                "Unplug the board, plug it in again and retry.",
            ));
    }

    phases.start("Checking driver");
    call.output
        .message("Waiting for Windows to give the device WinUSB.");
    let (custom, build, id) = (
        input.custom_boards_folder.clone(),
        input.build_folder.clone(),
        input.device.clone(),
    );
    let (device, driver) = tokio::task::spawn_blocking(move || {
        let find = lookup();
        settle(&id, custom, build, true, &find)
    })
    .await
    .map_err(|e| ToolError::engine(format!("The driver check stopped: {e}")))??;
    match driver {
        Some(driver) if driver.ready => Ok(InstallDriverOutput {
            device: device.id,
            driver,
            installed: true,
            same_ids: located.same_ids,
        }),
        other => Err(ToolError::setup(format!(
            "WinUSB was installed, but Windows still gives the device {}.",
            other
                .and_then(|d| d.current)
                .unwrap_or_else(|| "no driver".into())
        ))
        .with_details(response.details())
        .with_action(
            ActionCode::InstallDrivers,
            "Unplug the board, plug it in again and list the devices; if it still isn't ready, retry.",
        )),
    }
}

/// The refusal on systems that need no drivers.
pub fn not_windows() -> ToolError {
    ToolError::request(
        "Drivers are needed only on Windows: on this system the programmers use the board as it is.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_ids_are_named_in_the_refusal() {
        let located = Located {
            device: crate::devices::tests::device(
                "0403",
                "6010",
                Some("Alhambra II v1.0A"),
                None,
                2,
            ),
            board_name: "Alhambra II".into(),
            driver: None,
            same_ids: vec!["Dual RS232-HS (0403:6010)".into()],
        };
        let err = same_ids_error(&located);
        assert_eq!(err.category, crate::errors::Category::Request);
        assert!(err.message.contains("0403:6010"), "{}", err.message);
        assert!(err.message.contains("Dual RS232-HS"), "{}", err.message);
        assert_eq!(err.action.unwrap().code, ActionCode::DisconnectDevices);
    }

    // install_driver and uninstall_driver refuse at once elsewhere.
    #[test]
    fn elsewhere_it_is_refused() {
        let err = not_windows();
        assert_eq!(err.category, crate::errors::Category::Request);
        assert!(err.message.contains("only on Windows"), "{}", err.message);
    }
}
