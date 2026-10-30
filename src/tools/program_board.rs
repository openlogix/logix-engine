//! The program_board tool (docs/tool-api.md, Tables 9 and 10).

use std::sync::atomic::{AtomicBool, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Call, EngineTool};
use crate::build::{self, TOOLCHAIN};
use crate::devices::{self, Device};
use crate::errors::{ActionCode, Category, ToolError};
use crate::paths;
use crate::program::{self, ProgressLine};
use crate::progress::Phases;
use crate::runner::{RunError, run_tool_with};
use crate::state::EngineState;

pub struct ProgramBoard;

impl EngineTool for ProgramBoard {
    const NAME: &'static str = "program_board";
    const DESCRIPTION: &'static str = "Programs the bitstream of a successful build into the \
        board it was built for. It doesn't build: call build first, and program_board only if \
        the build succeeded. The board comes from the build, so it takes only the build folder. \
        It finds the connected board by USB; if none is connected it returns a setup error with \
        connect_board, and if several are, a setup error with select_device that lists them, \
        so retry with one's serial_number. Most boards are programmed to flash, so the design \
        stays after power-off.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = ProgramBoardInput;
    type Output = ProgramBoardOutput;
}

/// Which build to program, and optionally which device.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramBoardInput {
    /// The build folder of a successful build, as an absolute path.
    pub build_folder: String,
    /// Program only the device with this serial number, when several
    /// matching boards are connected.
    #[serde(default)]
    pub serial_number: Option<String>,
    /// A custom programmer command that replaces the board's default one,
    /// like Apio's programmer_cmd. Its first word is a program from the
    /// toolchain package, such as openFPGALoader, or an absolute path. It may
    /// use ${VID}, ${PID}, ${BUS}, ${DEV}, ${SERIAL} and must use
    /// ${BITSTREAM}.
    #[serde(default)]
    pub programmer_command: Option<String>,
}

/// What was programmed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramBoardOutput {
    /// The device that was programmed.
    pub device: Device,
    /// The programmer that was used, such as openfpgaloader, or custom for a
    /// programmer_command.
    pub programmer: String,
}

pub async fn run_with(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ProgramBoardOutput, ToolError> {
    let folder =
        paths::check_build_folder(&input.build_folder, paths::home_folder().as_deref(), None)
            .map_err(|e| ToolError::request(format!("build_folder: {e}.")))?;
    let record = build::read_record(&folder).ok_or_else(|| {
        ToolError::request(format!(
            "{} has no build record from a successful build. Run build first.",
            input.build_folder
        ))
    })?;
    if record.board.checksum() != record.board_checksum {
        return Err(ToolError::request(
            "The build record is damaged. Run build again.",
        ));
    }
    let bitstream = folder.join(&record.result.bitstream);
    if !bitstream.is_file() {
        return Err(ToolError::request(format!(
            "The bitstream {} is missing. Run build again.",
            bitstream.display()
        )));
    }
    if let Some(s) = &input.serial_number
        && (s.is_empty() || s.chars().any(char::is_control))
    {
        return Err(ToolError::request(
            "serial_number must be a non-empty single line.",
        ));
    }
    let board = &record.board;
    let variant = board.variants.get(&board.default_variant).ok_or_else(|| {
        ToolError::request("The build's board has no default programming variant.")
    })?;
    let custom = input.programmer_command.as_deref();

    let needed = state.packages.needing_install(&[TOOLCHAIN])?;
    let steps = if custom.is_some() { 2 } else { 3 };
    let phases = Phases::new(call.progress.clone(), needed.len() + steps);
    state
        .packages
        .ensure(&needed, state.auto_install, &phases, &call.cancel)
        .await?;
    let spec = state
        .packages
        .manifest()
        .get(TOOLCHAIN)
        .cloned()
        .ok_or_else(|| ToolError::engine(format!("{TOOLCHAIN} isn't in the manifest.")))?;
    let bin = state.packages.package_dir(&spec).join("bin");

    phases.start("Finding device");
    let device = devices::select(
        &scan()?,
        &variant.usb,
        input.serial_number.as_deref(),
        &board.description,
    )?;
    let (program, args) = program::command(variant, custom, &device, &bitstream, &bin)?;
    let programmer = match custom {
        None => variant.programmer.clone(),
        Some(_) => "custom".to_string(),
    };

    // Progress: write, then verify. A custom command's output isn't read.
    let verifying = AtomicBool::new(false);
    let on_line = |line: &str| {
        if custom.is_some() {
            return;
        }
        match program::progress_line(line) {
            ProgressLine::Percent(p) => phases.percent(p),
            ProgressLine::Verifying(p) => {
                if !verifying.swap(true, Ordering::SeqCst) {
                    phases.start("Verifying");
                }
                if let Some(p) = p {
                    phases.percent(p);
                }
            }
            ProgressLine::Other => {}
        }
    };
    phases.start(if custom.is_some() {
        "Programming"
    } else {
        "Writing flash"
    });
    let run = run_tool_with(&program, &args, &folder, &bin, &call.cancel, &on_line).await;
    match run {
        Ok(r) if r.status.success() => Ok(ProgramBoardOutput { device, programmer }),
        Ok(r) => Err(program::failure(
            &program.file_name().unwrap_or_default().to_string_lossy(),
            &board.description,
            &r.output,
            &r.status.to_string(),
        )),
        Err(RunError::Cancelled) => Err(ToolError::cancelled("Programming was cancelled.")),
        Err(RunError::Spawn(e)) => Err(ToolError::new(
            Category::Tool,
            format!("Can't start {}: {e}", program.display()),
        )
        .with_action(
            ActionCode::InstallPackages,
            format!("Reinstall the {TOOLCHAIN} package, which may be damaged."),
        )),
    }
}

pub async fn run(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
) -> Result<ProgramBoardOutput, ToolError> {
    run_with(state, input, call, devices::scan).await
}
