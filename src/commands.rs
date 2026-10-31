//! The commands an operation ran, recorded so they can be run again without
//! the engine, such as for a bug report to a tool's developers
//! (docs/tool-api.md, "Commands").

use std::ffi::OsString;
use std::path::Path;
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One program the engine ran, with what it takes to run it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandRecord {
    /// The program's name, such as yosys.
    pub tool: String,
    /// The package the program came from, with its version, such as
    /// "oss-cad-suite 2026-10-22". Omitted for a program from elsewhere, such
    /// as a custom programmer command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// The folder it ran in, as an absolute path.
    pub cwd: String,
    /// The program as an absolute path, then its arguments, exactly as
    /// passed.
    pub args: Vec<String>,
    /// One shell command line that runs it again the same way: in the same
    /// folder, with the same minimal environment (PATH, HOME and TMPDIR).
    pub shell: String,
}

/// The PATH the engine gives the programs it runs: the package's bin folder,
/// then the system's.
pub fn tool_path(bin_dir: &Path) -> OsString {
    let mut path = OsString::from(bin_dir);
    path.push(":/usr/bin:/bin");
    path
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
/// layout packages/<name>/<version>/bin.
fn package_of(program: &Path, bin_dir: &Path) -> Option<String> {
    if program.parent() != Some(bin_dir) {
        return None;
    }
    let version_dir = bin_dir.parent()?;
    let name_dir = version_dir.parent()?;
    if name_dir.parent()?.file_name()? != "packages" {
        return None;
    }
    Some(format!(
        "{} {}",
        name_dir.file_name()?.to_string_lossy(),
        version_dir.file_name()?.to_string_lossy()
    ))
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
        let bin = Path::new("/eng/packages/oss-cad-suite/2026-10-22/bin");
        let args: Vec<OsString> =
            vec!["-p".into(), "synth_ice40 -top main".into(), "it's.v".into()];
        let r = record(&bin.join("yosys"), &args, Path::new("/home/u/my proj"), bin);
        assert_eq!(r.tool, "yosys");
        assert_eq!(r.package.as_deref(), Some("oss-cad-suite 2026-10-22"));
        assert_eq!(r.cwd, "/home/u/my proj");
        assert_eq!(
            r.args,
            [
                "/eng/packages/oss-cad-suite/2026-10-22/bin/yosys",
                "-p",
                "synth_ice40 -top main",
                "it's.v"
            ]
        );
        assert_eq!(
            r.shell,
            "cd '/home/u/my proj' && env -i \
             PATH=/eng/packages/oss-cad-suite/2026-10-22/bin:/usr/bin:/bin \
             HOME=\"$HOME\" TMPDIR=\"$TMPDIR\" \
             /eng/packages/oss-cad-suite/2026-10-22/bin/yosys -p 'synth_ice40 -top main' 'it'\\''s.v'"
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
