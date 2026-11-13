//! The commands an operation ran, recorded so they can be run again without
//! the engine, such as for a bug report to a tool's developers
//! (docs/tool-api.md, "Commands").

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One program the engine ran, with what it takes to run it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandRecord {
    /// The program's name, such as yosys.
    pub tool: String,
    /// The package the program came from, with its version and the core's
    /// tool, such as "logix-core 2026.11.10 (oss-cad-suite)". Omitted for a
    /// program from elsewhere, such as a custom programmer command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// The folder it ran in, as an absolute path.
    pub cwd: String,
    /// The program as an absolute path, then its arguments, exactly as
    /// passed.
    pub args: Vec<String>,
    /// One shell command line that runs it again the same way: in the same
    /// folder, with the same minimal environment (PATH, the home and
    /// temporary folders and, on Windows, the variables its programs need).
    pub shell: String,
}

/// The folders on the PATH the engine gives a program in `bin_dir`: when
/// `bin_dir` is a core component's program folder, every PATH folder
/// `logix-core.json` lists for that component (on Windows also its `lib`,
/// with the DLLs its programs need), else `bin_dir` alone; then the
/// system's (`/usr/bin` and `/bin`, or Windows' `System32` and its own
/// folder).
pub fn tool_path_dirs(bin_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = component_path(bin_dir).unwrap_or_else(|| vec![bin_dir.to_path_buf()]);
    dirs.extend(system_path());
    dirs
}

/// The PATH the engine gives a program in `bin_dir` (tool_path_dirs).
pub fn tool_path(bin_dir: &Path) -> OsString {
    std::env::join_paths(tool_path_dirs(bin_dir)).unwrap_or_default()
}

/// The PATH folders of the core component whose program folder is
/// `bin_dir`, from the core's `logix-core.json` (in a folder above it).
fn component_path(bin_dir: &Path) -> Option<Vec<PathBuf>> {
    let core = bin_dir
        .ancestors()
        .skip(1)
        .find(|a| a.join(crate::packages::CORE_METADATA).is_file())?;
    let components = crate::packages::read_core_components(core)?;
    components
        .values()
        .find(|c| c.run.path.first().map(|p| core.join(p)).as_deref() == Some(bin_dir))
        .map(|c| c.run.path.iter().map(|p| core.join(p)).collect())
}

#[cfg(windows)]
fn system_path() -> Vec<PathBuf> {
    let root =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()));
    vec![root.join("System32"), root]
}

#[cfg(not(windows))]
fn system_path() -> Vec<PathBuf> {
    vec!["/usr/bin".into(), "/bin".into()]
}

/// The environment variables the programs keep, besides PATH: the home and
/// temporary folders, and on Windows what its programs need to run
/// (`SystemRoot`, `COMSPEC` and `PATHEXT` for `.cmd` programs).
#[cfg(windows)]
pub const KEPT_VARS: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "COMSPEC",
    "PATHEXT",
];
#[cfg(not(windows))]
pub const KEPT_VARS: &[&str] = &["HOME", "TMPDIR"];

/// A program named `name` in `bin_dir`: as named on macOS and Linux; on
/// Windows the first of `name.exe`, `name.cmd` and `name.bat` that's there
/// (openXC7's `fasm2frames` is a `.cmd`), else `name.exe`.
pub fn program_in(bin_dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) && Path::new(name).extension().is_none() {
        for ext in ["exe", "cmd", "bat"] {
            let p = bin_dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return p;
            }
        }
        return bin_dir.join(format!("{name}.exe"));
    }
    bin_dir.join(name)
}

/// Quotes a word for a POSIX shell when it needs it.
pub fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=+,@%".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The package a program in `bin_dir` came from, from the engine folder
/// layout packages/<name>/<version>/..., with the core component it's in,
/// such as "logix-core 2026.11.10 (oss-cad-suite)" (the version as people see
/// it; the folder is named by the release tag).
fn package_of(program: &Path, bin_dir: &Path) -> Option<String> {
    if program.parent() != Some(bin_dir) {
        return None;
    }
    let version_dir = bin_dir.ancestors().find(|a| {
        a.parent().and_then(Path::parent).and_then(Path::file_name) == Some("packages".as_ref())
    })?;
    let name = version_dir.parent()?.file_name()?.to_string_lossy();
    let version =
        crate::packages::manifest::shown_version(&version_dir.file_name()?.to_string_lossy());
    match bin_dir.strip_prefix(version_dir).ok()?.components().next() {
        Some(c) => Some(format!(
            "{name} {version} ({})",
            c.as_os_str().to_string_lossy()
        )),
        None => Some(format!("{name} {version}")),
    }
}

/// Records a run of `program` with `args` in `cwd`, with the environment the
/// runner gives it.
pub fn record(program: &Path, args: &[OsString], cwd: &Path, bin_dir: &Path) -> CommandRecord {
    let mut all = vec![program.to_string_lossy().into_owned()];
    all.extend(args.iter().map(|a| a.to_string_lossy().into_owned()));
    let path = tool_path(bin_dir).to_string_lossy().into_owned();
    let shell = format!(
        "cd {} && env -i PATH={} HOME=\"$HOME\" TMPDIR=\"$TMPDIR\" {}",
        shell_quote(&cwd.to_string_lossy()),
        shell_quote(&path),
        all.iter()
            .map(|w| shell_quote(w))
            .collect::<Vec<_>>()
            .join(" ")
    );
    CommandRecord {
        tool: program
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        package: package_of(program, bin_dir),
        cwd: cwd.to_string_lossy().into_owned(),
        args: all,
        shell,
    }
}

/// The commands one tool call ran, in order.
#[derive(Debug, Default)]
pub struct CommandLog(Mutex<Vec<CommandRecord>>);

impl CommandLog {
    pub fn push(&self, record: CommandRecord) {
        self.0.lock().unwrap().push(record);
    }

    /// The commands so far, leaving the log empty.
    pub fn take(&self) -> Vec<CommandRecord> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_runnable_commands_with_their_package() {
        let bin = Path::new("/eng/packages/logix-core/2026-11-10/oss-cad-suite/bin");
        let args: Vec<OsString> =
            vec!["-p".into(), "synth_ice40 -top main".into(), "it's.v".into()];
        let r = record(
            &bin.join("yosys"),
            &args,
            Path::new("/home/user/my proj"),
            bin,
        );
        assert_eq!(r.tool, "yosys");
        assert_eq!(
            r.package.as_deref(),
            Some("logix-core 2026.11.10 (oss-cad-suite)")
        );
        assert_eq!(r.cwd, "/home/user/my proj");
        assert_eq!(
            r.args,
            [
                "/eng/packages/logix-core/2026-11-10/oss-cad-suite/bin/yosys",
                "-p",
                "synth_ice40 -top main",
                "it's.v"
            ]
        );
        assert_eq!(
            r.shell,
            "cd '/home/user/my proj' && env -i \
             PATH=/eng/packages/logix-core/2026-11-10/oss-cad-suite/bin:/usr/bin:/bin \
             HOME=\"$HOME\" TMPDIR=\"$TMPDIR\" \
             /eng/packages/logix-core/2026-11-10/oss-cad-suite/bin/yosys -p 'synth_ice40 -top main' 'it'\\''s.v'"
        );

        // A program from elsewhere, such as a custom programmer command.
        let r = record(Path::new("/opt/x/flash"), &[], Path::new("/b"), bin);
        assert_eq!(r.package, None);
    }

    #[test]
    fn the_shell_line_runs_the_command_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a dir");
        std::fs::create_dir(&dir).unwrap();
        let args: Vec<OsString> = vec![
            "-c".into(),
            "pwd; echo \"$PATH\"; echo \"$1\"".into(),
            "sh".into(),
            "x 'y'".into(),
        ];
        let r = record(Path::new("/bin/sh"), &args, &dir, Path::new("/pkg/bin"));
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&r.shell)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<_> = out.lines().collect();
        assert!(lines[0].ends_with("/a dir"), "{out}");
        assert_eq!(lines[1], "/pkg/bin:/usr/bin:/bin");
        assert_eq!(lines[2], "x 'y'");
    }
}
