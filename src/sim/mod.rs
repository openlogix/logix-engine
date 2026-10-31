//! Lint, test and simulate (docs/tool-api.md): Verilator and Icarus
//! Verilog, run with the same commands as Apio, on the design and its
//! testbenches.
//!
//! The tools run in the source folder, so they report files as given. Unlike
//! yosys, they don't look for an included file next to the file that
//! includes it, so the engine passes an -I for the folder of every listed
//! and included file. Includes are then found in the source folder, then in
//! those folders.

pub mod lint;
pub mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::board::ResolvedBoard;
use crate::catalog::Architecture;
use crate::errors::ToolError;
use crate::progress::Phases;
use crate::state::EngineState;
use crate::tools::Call;

/// The package with Verilator, Icarus Verilog and GTKWave.
pub const TOOLCHAIN: &str = crate::build::TOOLCHAIN;

/// What every lint, test and simulate request has.
#[derive(Debug, Clone)]
pub struct Sources {
    pub board: ResolvedBoard,
    /// The real path of the source folder.
    pub source_folder: PathBuf,
    /// The design files, relative to the source folder.
    pub source_files: Vec<String>,
    /// The testbench files, relative to the source folder.
    pub testbench_files: Vec<String>,
    pub defines: BTreeMap<String, String>,
    pub verbose: bool,
}

/// The architecture's simulation models: the FPGA's primitives, such as
/// SB_IO and SB_PLL40_CORE for iCE40, so designs that use them can be
/// linted and simulated.
pub struct SimLibrary {
    /// Folder to search for included files.
    pub include_dir: PathBuf,
    /// Model files, compiled with the design.
    pub files: Vec<PathBuf>,
    /// Macros the models need, as -D arguments.
    pub defines: Vec<String>,
}

impl SimLibrary {
    /// The models for the board's FPGA, from the toolchain package in
    /// `package_dir`. The same files and macros Apio uses.
    pub fn for_board(board: &ResolvedBoard, package_dir: &Path) -> SimLibrary {
        match board.fpga.architecture {
            Architecture::Ice40 => {
                let dir = package_dir.join("share/yosys/ice40");
                SimLibrary {
                    files: vec![dir.join("cells_sim.v")],
                    include_dir: dir,
                    defines: vec!["-DNO_ICE40_DEFAULT_ASSIGNMENTS".into()],
                }
            }
        }
    }
}

impl Sources {
    /// The design files, then the testbenches.
    pub fn all_files(&self) -> Vec<String> {
        self.source_files
            .iter()
            .chain(self.testbench_files.iter())
            .cloned()
            .collect()
    }

    /// The -D arguments for the request's defines, in name order, each with
    /// its value given explicitly.
    pub fn define_args(&self) -> Vec<OsString> {
        self.defines
            .iter()
            .map(|(n, v)| format!("-D{n}={v}").into())
            .collect()
    }

    /// The -I arguments: the folder of every listed and included file, in
    /// order, other than the source folder itself, which the tools search
    /// anyway. Also checks that included files are inside the source folder.
    pub fn include_args(&self) -> Result<Vec<OsString>, ToolError> {
        let files = self.all_files();
        let includes = crate::build::find_includes(&self.source_folder, &files)?;
        let dirs: BTreeSet<String> = files
            .iter()
            .chain(includes.iter())
            .filter_map(|f| {
                let parent = Path::new(f).parent()?.to_string_lossy().to_string();
                (!parent.is_empty()).then_some(parent)
            })
            .collect();
        Ok(dirs.into_iter().map(|d| format!("-I{d}").into()).collect())
    }
}

/// Makes sure the toolchain is installed and returns its bin folder and
/// package folder. `phases` already counts one phase per package to install.
pub async fn toolchain(
    state: &EngineState,
    needed: &[crate::packages::manifest::PackageSpec],
    phases: &Phases,
    call: &Call,
) -> Result<(PathBuf, PathBuf), ToolError> {
    let spec = state
        .packages
        .manifest()
        .get(TOOLCHAIN)
        .cloned()
        .ok_or_else(|| ToolError::engine(format!("{TOOLCHAIN} isn't in the manifest.")))?;
    state
        .packages
        .ensure(needed, state.auto_install, phases, &call.cancel)
        .await?;
    let dir = state.packages.package_dir(&spec);
    Ok((dir.join("bin"), dir))
}

/// Keeps the first of each diagnostic with the same file, line and text.
pub fn dedup(diagnostics: &mut Vec<crate::errors::Diagnostic>) {
    let mut seen = std::collections::HashSet::new();
    diagnostics.retain(|d| seen.insert((d.file.clone(), d.line, d.column, d.text.clone())));
}
