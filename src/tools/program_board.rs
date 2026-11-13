//! The program_board tool (docs/tool-api.md, Tables 9 and 10).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Call, EngineTool};
use crate::build::{self, TOOLCHAIN};
use crate::devices::{self, Device};
use crate::errors::{ActionCode, Category, ToolError};
use crate::output_rules::ToolOutput;
use crate::paths;
use crate::program;
use crate::progress::Phases;
use crate::runner::{RunError, command_line, run_tool_with};
use crate::state::EngineState;

pub struct ProgramBoard;

impl EngineTool for ProgramBoard {
    const NAME: &'static str = "program_board";
    const DESCRIPTION: &'static str = "Programs the bitstream of a successful build into the \
        board it was built for. It doesn't build: call build first, and program_board only if \
        the build succeeded. The board comes from the build, so it takes the build folder and \
        the device to program: the id of a device from list_devices whose boards include the \
        build's board (ask the user when several do). A device that isn't connected anymore is a \
        setup error with connect_board; list the devices again. On ECP5, Gowin and Xilinx \
        boards it first reads the connected FPGA's ID code and refuses a different FPGA, such as \
        a board of another size than the build's, before writing anything. Most boards are \
        programmed to flash, so the design stays after power-off.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = ProgramBoardInput;
    type Output = ProgramBoardOutput;
}

/// Which build to program, and into which device.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramBoardInput {
    /// The build folder of a successful build, as an absolute path.
    pub build_folder: String,
    /// The device to program: the id of a device from list_devices, such as
    /// one whose boards include the build's board. It must still be
    /// connected and match the board's USB filter.
    pub device: String,
    /// A custom programmer command that replaces the board's default one,
    /// for a custom setup. Its first word is a program from the
    /// toolchain package, such as openFPGALoader, or an absolute path. It may
    /// use ${VID}, ${PID}, ${BUS}, ${DEV}, ${SERIAL_NUM}, ${USB_PATH} and must use
    /// ${BITSTREAM}.
    #[serde(default)]
    pub programmer_command: Option<String>,
    /// Let the programmer print more detail, shown in the live output and
    /// the logs. Not applied to a programmer_command. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
    /// The host's own boards: the absolute path of a folder of board files,
    /// one board each in the catalog's format, named by board ID, such as
    /// boards/my-board.json (a project's boards folder). board can name one
    /// of them; one with a catalog board's ID replaces that board in this
    /// request. Problems in any file come back as a request error with a
    /// diagnostic for each, at file, line and column. Send the same folder to
    /// every tool. Omitted means none.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// How to name paths in what people read (diagnostics, messages,
    /// testbench output, live tool output, diagram labels): files named as
    /// a whole, then folders whose files keep their path below the folder
    /// under the folder's new name ("" gives names relative to it); the
    /// first match in each list wins. Paths are compared by real location.
    /// Never applied to command lines or fields that are paths. Omitted
    /// means paths read as the request gave them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_paths: Option<crate::display_paths::DisplayPaths>,
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
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

/// The checked build folder and its build record, from a successful build.
/// A folder that breaks the keyword rules, has no record or a damaged one is
/// a request error.
pub fn read_build(
    build_folder: &str,
) -> Result<(std::path::PathBuf, build::BuildRecord), ToolError> {
    let folder = paths::check_build_folder(build_folder, paths::home_folder().as_deref(), None)
        .map_err(|e| ToolError::request(format!("build_folder: {e}.")))?;
    let record = build::read_record(&folder).ok_or_else(|| {
        ToolError::request(format!(
            "{build_folder} has no build record from a successful build. Run build first."
        ))
    })?;
    if record.board.checksum() != record.board_checksum {
        return Err(ToolError::request(
            "The build record is damaged. Run build again.",
        ));
    }
    Ok((folder, record))
}

pub async fn run_with(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ProgramBoardOutput, ToolError> {
    run_with_programmers(state, input, call, scan, None).await
}

/// As run_with, with the programmers taken from `programmers` instead of the
/// toolchain package when given, so tests can use fake programmers.
pub async fn run_with_programmers(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
    programmers: Option<&std::path::Path>,
) -> Result<ProgramBoardOutput, ToolError> {
    let (folder, record) = read_build(&input.build_folder)?;
    let bitstream = folder.join(&record.result.bitstream);
    if !bitstream.is_file() {
        return Err(ToolError::request(format!(
            "The bitstream {} is missing. Run build again.",
            bitstream.display()
        )));
    }
    let custom = crate::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let board = &upload_board(&record.board, &custom, &input.build_folder)?;
    let variant = &board.variant;
    let custom = input.programmer_command.as_deref();

    let needed = state
        .packages
        .needing_download(&[crate::packages::CORE])
        .await?;
    let checks_fpga = custom.is_none() && program::checks_fpga(variant, &board.fpga);
    let steps = if custom.is_some() { 2 } else { 3 } + usize::from(checks_fpga);
    let phases = Phases::new(call.progress.clone(), needed.len() + steps);
    state
        .packages
        .ensure(&needed, state.auto_download, &phases, &call.cancel)
        .await?;
    let bin = match programmers {
        Some(dir) => dir.to_path_buf(),
        None => state.packages.bin_dir(TOOLCHAIN)?,
    };

    // Say how the board will be programmed (not for a custom command, which
    // may do anything), and any note for users, such as holding a button.
    if custom.is_none() {
        if let Some(plan) = program::plan(&board.description, variant) {
            call.output.message(&plan);
        }
        if let Some(notes) = &variant.user_notes {
            call.output.message(&format!("Note: {notes}"));
        }
    }
    phases.start("Finding device");
    let connected = scan()?;
    let device = devices::find(&connected, &input.device, &board.name)?;
    if !devices::matches(&device, &variant.usb) {
        return Err(ToolError::request(format!(
            "device: {} isn't a {}: it doesn't match the board's USB filter. Choose a device whose boards in list_devices include the build's board.",
            device_name(&device),
            board.name
        )));
    }
    program::check_alone(
        custom.unwrap_or(&variant.args),
        &connected,
        &device,
        &board.name,
    )?;
    if checks_fpga {
        phases.start("Checking FPGA");
        check_fpga(board, &device, &folder, &bin, call).await?;
    }
    let (program, args) =
        program::command(variant, custom, &device, &bitstream, &bin, input.verbose)?;
    let programmer = match custom {
        None => variant.programmer.clone(),
        Some(_) => "custom".to_string(),
    };

    // Progress: write, then verify, from the output rules. A custom
    // command's progress isn't read.
    let phase = std::sync::Mutex::new(None::<&str>);
    let tool_name = program
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let shown = ToolOutput::new(&tool_name, call.output.as_ref());
    let on_line = |line: &str, redraw: bool| {
        let progress = shown.line(line, redraw);
        if custom.is_some() {
            return;
        }
        if let Some(name) = progress.phase {
            let mut current = phase.lock().unwrap();
            if *current != Some(name) {
                *current = Some(name);
                phases.start(name);
            }
        }
        if let Some(p) = progress.percent {
            phases.percent(p);
        }
    };
    phases.start(if custom.is_some() {
        "Programming"
    } else {
        program::writing_phase(variant)
    });
    call.output
        .command(&tool_name, &command_line(&program, &args));
    call.commands
        .push(crate::commands::record(&program, &args, &folder, &bin));
    let run = run_tool_with(&program, &args, &folder, &bin, &call.cancel, &on_line).await;
    match run {
        Ok(r) if r.status.success() => {
            if custom.is_none()
                && let Some(done) = program::done(&board.description, variant)
            {
                call.output.message(&done);
            }
            Ok(ProgramBoardOutput {
                device,
                programmer,
                commands: call.commands.take(),
            })
        }
        Ok(r) => Err(program::failure(
            &program.file_name().unwrap_or_default().to_string_lossy(),
            &board.name,
            &r.output,
            &r.status.to_string(),
        )),
        Err(RunError::Cancelled) => Err(ToolError::cancelled("Programming was cancelled.")),
        Err(RunError::Spawn(e)) => Err(ToolError::new(
            Category::Tool,
            format!("Can't start {}: {e}", program.display()),
        )
        .with_action(
            ActionCode::DownloadPackages,
            format!(
                "Download the {} package again, which may be damaged.",
                crate::packages::CORE
            ),
        )),
    }
}

/// The board to program a build with: the build's board ID and variant, as
/// this request defines them now (its custom boards folder, else the catalog), so
/// programming uses the current way to program the board, such as fixed
/// programmer arguments, without a rebuild. Refused only when the
/// bitstream doesn't fit it anymore: another FPGA build setting, or the
/// board or variant is gone.
pub fn upload_board(
    built: &crate::board::ResolvedBoard,
    custom: &crate::board::CustomBoards,
    build_folder: &str,
) -> Result<crate::board::ResolvedBoard, ToolError> {
    // Build records from before board IDs were kept: the record's board.
    let Some(id) = &built.board_id else {
        return Ok(built.clone());
    };
    match crate::board::resolve(id, &Some(built.variant.selector.clone().into()), custom) {
        Ok(now) if now.build_key() == built.build_key() => Ok(now),
        _ => Err(ToolError::request(format!(
            "The board {id} in this request (from custom_boards_folder, else the catalog) isn't the one {build_folder} was built for. Send the custom boards folder the build had, or run build again."
        ))),
    }
}

/// Reads the connected FPGA's ID code with openFPGALoader's --detect, using
/// only the variant's connection options, and refuses a different FPGA
/// before anything is written (docs/requirements.md, Wrong size).
async fn check_fpga(
    board: &crate::board::ResolvedBoard,
    device: &Device,
    folder: &std::path::Path,
    bin: &std::path::Path,
    call: &Call,
) -> Result<(), ToolError> {
    let exe = crate::commands::program_in(bin, "openFPGALoader");
    let args = program::detect_args(&board.variant, device)?;
    call.output
        .command("openFPGALoader", &command_line(&exe, &args));
    call.commands
        .push(crate::commands::record(&exe, &args, folder, bin));
    let shown = ToolOutput::new("openFPGALoader", call.output.as_ref());
    let on_line = |line: &str, redraw: bool| {
        shown.line(line, redraw);
    };
    let run = run_tool_with(&exe, &args, folder, bin, &call.cancel, &on_line).await;
    let r = match run {
        Ok(r) => r,
        Err(RunError::Cancelled) => {
            return Err(ToolError::cancelled("Programming was cancelled."));
        }
        Err(RunError::Spawn(e)) => {
            return Err(ToolError::new(
                Category::Tool,
                format!("Can't start {}: {e}", exe.display()),
            )
            .with_action(
                ActionCode::DownloadPackages,
                format!(
                    "Download the {} package again, which may be damaged.",
                    crate::packages::CORE
                ),
            ));
        }
    };
    let found = program::parse_detect(&r.output);
    if !r.status.success() && found.is_empty() {
        let mut err = program::failure(
            "openFPGALoader",
            &board.name,
            &r.output,
            &r.status.to_string(),
        );
        if err.category == Category::Tool {
            err.message = format!(
                "openFPGALoader couldn't read the FPGA's ID code on the {} ({}), so nothing was written.",
                board.name, r.status
            );
        }
        return Err(err);
    }
    program::check_fpga(&found, &board.fpga, &board.name, |code| {
        boards_with_fpga(code, device)
    })
}

/// The catalog boards with the FPGA of this ID code that the device most
/// likely is: of those it matches, the ones at its strongest match level,
/// as list_devices rates them.
pub fn boards_with_fpga(code: u32, device: &Device) -> Vec<String> {
    let catalog = crate::catalog::get();
    let candidates: Vec<devices::Candidate> = catalog
        .boards
        .iter()
        .map(|(id, b)| devices::Candidate {
            id,
            custom: false,
            variants: &b.variants,
        })
        .collect();
    let has_fpga = |board: &str| {
        catalog
            .boards
            .get(board)
            .and_then(|b| catalog.fpgas.get(&b.fpga_id))
            .and_then(|f| f.jtag_idcode.as_deref())
            .and_then(crate::catalog::parse_idcode)
            == Some(code)
    };
    let rated: Vec<_> = devices::board_matches(device, &candidates)
        .into_iter()
        .filter(|m| has_fpga(&m.board))
        .collect();
    let top = rated.iter().map(|m| m.best_level()).max();
    rated
        .iter()
        .filter(|m| Some(m.best_level()) == top)
        .map(|m| catalog.boards[&m.board].name.clone())
        .collect()
}

pub async fn run(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
) -> Result<ProgramBoardOutput, ToolError> {
    run_with(state, input, call, devices::scan).await
}

/// A device for messages, such as "Dual RS232-HS (0403:6010)".
fn device_name(d: &Device) -> String {
    let name = d.product.as_deref().unwrap_or("The device");
    format!("{name} ({}:{})", d.vid, d.pid)
}
