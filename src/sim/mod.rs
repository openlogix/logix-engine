//! Lint, test and simulate (docs/tool-api.md): Verilator and Icarus
//! Verilog, run with the standard commands, on the design and its
//! testbenches.
//!
//! The tools run in the source folder, so they report files as given. Unlike
//! yosys, they don't look for an included file next to the file that
//! includes it, so the engine passes an -I for the folder of every listed
//! and included file. Includes are then found in the source folder, then in
//! those folders.

pub mod lint;
pub mod parse;
pub mod testbench;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::board::ResolvedBoard;
use crate::catalog::Toolchain;
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

/// The toolchain's simulation models: the FPGA's primitives, such as
/// SB_IO and SB_PLL40_CORE for iCE40, so designs that use them can be
/// linted and simulated.
pub struct SimLibrary {
    /// Folder to search for included files.
    pub include_dir: PathBuf,
    /// Model files Icarus compiles with the design.
    pub files: Vec<PathBuf>,
    /// Model files Verilator reads with the design: the simulation models,
    /// plus black boxes for primitives that have none.
    pub lint_files: Vec<PathBuf>,
    /// Macros Icarus needs for the models, as -D arguments.
    pub defines: Vec<String>,
    /// Macros Verilator needs for the models, as -D arguments.
    pub lint_defines: Vec<String>,
    /// Verilator warnings to turn off in the model files besides its lint
    /// group, which is always off there. Warnings in the models aren't the
    /// user's to fix.
    pub verilator_rules_off: Vec<&'static str>,
}

impl SimLibrary {
    /// The models for the board's FPGA, from the toolchain package in
    /// `package_dir`. The FPGA's model files and macros.
    pub fn for_board(board: &ResolvedBoard, package_dir: &Path) -> SimLibrary {
        match board.fpga.toolchain() {
            Toolchain::Ice40 => {
                let dir = package_dir.join("share/yosys/ice40");
                let defines = vec!["-DNO_ICE40_DEFAULT_ASSIGNMENTS".to_string()];
                SimLibrary {
                    files: vec![dir.join("cells_sim.v")],
                    lint_files: vec![dir.join("cells_sim.v")],
                    include_dir: dir,
                    lint_defines: defines.clone(),
                    defines,
                    verilator_rules_off: vec!["COMBDLY"],
                }
            }
            Toolchain::Ecp5 => {
                let dir = package_dir.join("share/yosys/ecp5");
                SimLibrary {
                    files: vec![dir.join("cells_sim.v")],
                    lint_files: vec![dir.join("cells_sim.v"), dir.join("cells_bb.v")],
                    include_dir: dir,
                    // Per https://github.com/YosysHQ/yosys/issues/5668.
                    defines: vec!["-DNO_INCLUDES".into()],
                    lint_defines: vec![],
                    // duplicate modules from the models' includes, a
                    // width mismatch in a model, and the models' partial pin
                    // lists.
                    verilator_rules_off: vec!["MODDUP", "WIDTHEXPAND", "PINMISSING"],
                }
            }
            Toolchain::Gowin => {
                let dir = package_dir.join("share/yosys/gowin");
                let mut lint_files = vec![dir.join("cells_sim.v")];
                // Black boxes for the primitives without a model, per family.
                let family = board.fpga.family.id();
                lint_files.push(dir.join(format!("cells_xtra_{family}.v")));
                SimLibrary {
                    files: vec![dir.join("cells_sim.v")],
                    lint_files,
                    include_dir: dir,
                    defines: vec![],
                    lint_defines: vec![],
                    // timing blocks the models carry, and modules
                    // the black boxes declare again.
                    verilator_rules_off: vec!["SPECIFYIGN", "MODDUP"],
                }
            }
            Toolchain::Openxc7 => {
                let dir = package_dir.join("share/yosys/xilinx");
                SimLibrary {
                    files: vec![dir.join("cells_sim.v")],
                    // Black boxes for the primitives without a model, such
                    // as PLLE2_BASE.
                    lint_files: vec![dir.join("cells_sim.v"), dir.join("cells_xtra.v")],
                    include_dir: dir,
                    defines: vec![],
                    lint_defines: vec![],
                    // timing blocks the models carry.
                    verilator_rules_off: vec!["SPECIFYIGN"],
                }
            }
        }
    }
}

impl SimLibrary {
    /// A Verilator config (.vlt) that turns off warnings in the model files,
    /// a hardware.vlt. Without it, Verilator warns about the
    /// models' unused primitives when no top module is given.
    pub fn verilator_config(&self) -> String {
        let glob = format!("{}/*", self.include_dir.display()).replace('\\', "\\\\");
        let mut text = format!("`verilator_config\nlint_off -file \"{glob}\"\n");
        for rule in &self.verilator_rules_off {
            text.push_str(&format!("lint_off -rule {rule} -file \"{glob}\"\n"));
        }
        text
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

/// Makes sure the core is downloaded and returns the suite's bin folder and
/// its component folder. `phases` already counts one phase per package to
/// download.
pub async fn toolchain(
    state: &EngineState,
    needed: &[crate::packages::manifest::PackageSpec],
    phases: &Phases,
    call: &Call,
) -> Result<(PathBuf, PathBuf), ToolError> {
    state
        .packages
        .ensure(needed, state.auto_download, phases, &call.cancel)
        .await?;
    Ok((
        state.packages.bin_dir(TOOLCHAIN)?,
        state.packages.component_dir(TOOLCHAIN)?,
    ))
}

/// Keeps the first of each diagnostic with the same file, line, text and
/// simulation time. A testbench check that fails again at another time is
/// another failure and is kept.
pub fn dedup(diagnostics: &mut Vec<crate::errors::Diagnostic>) {
    let mut seen = std::collections::HashSet::new();
    diagnostics.retain(|d| {
        seen.insert((
            d.file.clone(),
            d.line,
            d.column,
            d.text.clone(),
            d.time_ns.map(f64::to_bits),
        ))
    });
}

/// The most testbench failures a result lists in its diagnostics. A result
/// also gives the total, failure_count.
pub const MAX_FAILURES_LISTED: usize = 100;
