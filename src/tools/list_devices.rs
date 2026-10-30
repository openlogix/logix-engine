//! The list_devices tool (docs/tool-api.md, Tables 11 to 13).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board::{self, BoardSpec};
use crate::devices::{self, Device};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct ListDevices;

impl EngineTool for ListDevices {
    const NAME: &'static str = "list_devices";
    const DESCRIPTION: &'static str = "Lists the USB devices connected to this computer, with \
        their USB IDs, names and serial numbers. Given a board, each device also says whether it \
        matches that board, which shows whether the board is connected and whether several \
        are. Use it before program_board, or to find a board's serial number when several \
        boards of the same kind are connected.";
    const READ_ONLY: bool = true;
    type Input = ListDevicesInput;
    type Output = ListDevicesOutput;
}

/// Optionally, a board to match the devices against.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesInput {
    /// A board, as a catalog ID such as alhambra-ii or a full definition.
    /// When given, each device reports whether it matches this board.
    #[serde(default)]
    pub board: Option<BoardSpec>,
}

/// The connected devices.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesOutput {
    /// The connected USB devices, sorted by vendor ID, product ID, bus and
    /// device number.
    pub devices: Vec<Device>,
}

pub fn run_with(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ListDevicesOutput, ToolError> {
    let filter = match &input.board {
        Some(spec) => {
            let b = board::resolve(spec)?;
            Some(b.variants[&b.default_variant].usb.clone())
        }
        None => None,
    };
    let mut list = scan()?;
    if let Some(f) = &filter {
        for d in &mut list {
            d.matches_board = Some(devices::matches(d, f));
        }
    }
    Ok(ListDevicesOutput { devices: list })
}

pub fn run(_state: &EngineState, input: ListDevicesInput) -> Result<ListDevicesOutput, ToolError> {
    run_with(input, devices::scan)
}
