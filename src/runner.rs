//! Runs a packaged tool as a subprocess (docs/architecture.md, "Process
//! isolation"): its own working folder and a minimal environment, output
//! captured, and on cancel its whole process tree stopped: asked to exit
//! first (SIGTERM), so tools can finish writing files, then killed after a
//! short grace period.

use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A finished tool run.
#[derive(Debug)]
pub struct ToolRun {
    pub status: ExitStatus,
    /// stdout and stderr lines, in the order they arrived. A carriage
    /// return also ends a line, so progress bars become separate lines.
    /// Empty lines are left out.
    pub output: String,
}

#[derive(Debug)]
pub enum RunError {
    Cancelled,
    /// The tool couldn't be started.
    Spawn(std::io::Error),
}

/// Runs `program` with `args` in `cwd`. The environment has only PATH (the
/// package's bin folder first), HOME and TMPDIR, so results don't depend on
/// the user's shell setup.
pub async fn run_tool(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    bin_dir: &Path,
    cancel: &CancellationToken,
) -> Result<ToolRun, RunError> {
    run_tool_with(program, args, cwd, bin_dir, cancel, &|_, _| {}).await
}

/// Like run_tool, calling `on_line` with each output line as it arrives,
/// such as to read progress or show it, and whether the tool will print
/// over it (`redraw`: a bare carriage return ended it, as progress bars
/// do; `\r\n` counts as a newline). How lines are shown is up to the
/// output rules (src/output_rules.rs).
pub async fn run_tool_with(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    bin_dir: &Path,
    cancel: &CancellationToken,
    on_line: &(dyn Fn(&str, bool) + Send + Sync),
) -> Result<ToolRun, RunError> {
    if cancel.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    let path = crate::commands::tool_path(bin_dir);
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for var in crate::commands::KEPT_VARS {
        if let Some(v) = std::env::var_os(var) {
            cmd.env(var, v);
        }
    }
    #[cfg(unix)]
    cmd.process_group(0);

    log::info!(
        "Running {} {} (in {})",
        program.display(),
        args.iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        cwd.display()
    );
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let pid = child.id();

    let (tx, mut rx) = mpsc::unbounded_channel::<(String, bool)>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let mut lines = LineSplitter::default();
            loop {
                let n = match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for &b in &buf[..n] {
                    if let Some(line) = lines.push(b) {
                        let _ = tx.send(line);
                    }
                }
            }
            if let Some(line) = lines.finish() {
                let _ = tx.send(line);
            }
        });
    }
    drop(tx);

    let mut output = Vec::new();
    let status = loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some((l, redraw)) => {
                    on_line(&l, redraw);
                    output.push(l);
                }
                None => break child.wait().await.map_err(RunError::Spawn)?,
            },
            _ = cancel.cancelled() => {
                // Ask first, so tools can finish writing their files, such
                // as a waveform; then force whatever is left.
                signal_tree(pid, Signal::Terminate);
                let exited = tokio::time::timeout(STOP_GRACE, child.wait()).await;
                signal_tree(pid, Signal::Kill);
                if exited.is_err() {
                    let _ = child.kill().await;
                }
                log::info!("Stopped {}", program.display());
                return Err(RunError::Cancelled);
            }
        }
    };
    let output = output.join("\n");
    log::debug!("{} exited with {status}:\n{output}", program.display());
    Ok(ToolRun { status, output })
}

/// Splits a tool's output into lines, ended by `\n`, `\r\n` or a bare
/// `\r`, and tells which ones a bare `\r` ended (the tool prints over
/// them). Empty lines are left out.
#[derive(Default)]
struct LineSplitter {
    line: Vec<u8>,
    /// A `\r` was the last byte: it ends the line as a redraw unless a
    /// `\n` follows.
    pending_cr: bool,
}

impl LineSplitter {
    /// Takes the next byte; returns a line it ends, with its redraw flag.
    fn push(&mut self, b: u8) -> Option<(String, bool)> {
        if self.pending_cr {
            self.pending_cr = false;
            if b == b'\n' {
                return self.take(false);
            }
            let ended = self.take(true);
            self.accept(b);
            return ended;
        }
        match b {
            b'\n' => self.take(false),
            b'\r' => {
                self.pending_cr = true;
                None
            }
            _ => {
                self.line.push(b);
                None
            }
        }
    }

    /// A byte after a carriage return: the start of the next line.
    fn accept(&mut self, b: u8) {
        match b {
            b'\r' => self.pending_cr = true,
            b'\n' => {}
            _ => self.line.push(b),
        }
    }

    /// The line left at the end of the output. Nothing prints over it, so
    /// it isn't a redraw even if a carriage return ended it.
    fn finish(&mut self) -> Option<(String, bool)> {
        self.pending_cr = false;
        self.take(false)
    }

    fn take(&mut self, redraw: bool) -> Option<(String, bool)> {
        if self.line.is_empty() {
            return None;
        }
        let text = String::from_utf8_lossy(&self.line).into_owned();
        self.line.clear();
        Some((text, redraw))
    }
}

/// A command line for people to read: the program's file name and its
/// arguments, quoting arguments that have spaces or quotes.
pub fn command_line(program: &Path, args: &[OsString]) -> String {
    let mut words = vec![crate::commands::tool_name(program)];
    for a in args {
        let a = a.to_string_lossy();
        if a.is_empty() || a.contains([' ', '"', '\'', '\t']) {
            words.push(format!("'{}'", a.replace('\'', "'\\''")));
        } else {
            words.push(a.into_owned());
        }
    }
    words.join(" ")
}

/// How long a stopped tool gets to exit before it is killed.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Copy)]
enum Signal {
    /// Ask to exit (SIGTERM). vvp, for one, then writes out its waveform.
    Terminate,
    /// Force (SIGKILL).
    Kill,
}

/// Signals the tool and the processes it started, which share its process
/// group.
fn signal_tree(pid: Option<u32>, signal: Signal) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        let sig = match signal {
            Signal::Terminate => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        // SAFETY: killpg only sends a signal; a stale group ID fails harmlessly.
        unsafe {
            libc::killpg(pid as libc::pid_t, sig);
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, signal);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn captures_output_and_status() {
        let tmp = tempfile::tempdir().unwrap();
        let args: Vec<OsString> = vec!["-c".into(), "echo out; echo err >&2; exit 3".into()];
        let run = run_tool(
            Path::new("/bin/sh"),
            &args,
            tmp.path(),
            tmp.path(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(run.status.code(), Some(3));
        assert!(run.output.contains("out") && run.output.contains("err"));
    }

    #[test]
    fn command_lines_quote_spaces() {
        let args: Vec<OsString> = vec!["-p".into(), "synth_ice40 -top main".into(), "it's".into()];
        assert_eq!(
            command_line(Path::new("/pkg/bin/yosys"), &args),
            "yosys -p 'synth_ice40 -top main' 'it'\\''s'"
        );
    }

    #[tokio::test]
    async fn carriage_returns_split_lines_and_mark_redraws() {
        let tmp = tempfile::tempdir().unwrap();
        let args: Vec<OsString> = vec!["-c".into(), r"printf 'a 10%%\rb 50%%\r\nc\n'".into()];
        let seen = std::sync::Mutex::new(Vec::new());
        let run = run_tool_with(
            Path::new("/bin/sh"),
            &args,
            tmp.path(),
            tmp.path(),
            &CancellationToken::new(),
            &|l, redraw| seen.lock().unwrap().push((l.to_string(), redraw)),
        )
        .await
        .unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                ("a 10%".to_string(), true),
                ("b 50%".to_string(), false),
                ("c".to_string(), false)
            ]
        );
        assert_eq!(run.output, "a 10%\nb 50%\nc");
    }

    fn split(bytes: &[u8]) -> Vec<(String, bool)> {
        let mut s = LineSplitter::default();
        let mut lines: Vec<_> = bytes.iter().filter_map(|&b| s.push(b)).collect();
        lines.extend(s.finish());
        lines
    }

    #[test]
    fn iceprog_progress_is_redraws_with_blank_erasers() {
        // iceprog's output while writing, byte for byte (icestorm
        // iceprog/iceprog.c, the write loop): a blank eraser, then each
        // address, both ended by a carriage return.
        let out = b"programming..\n                      \raddr 0x000000   0%\r                      \raddr 0x000100   0%\r                      \rdone.\n";
        assert_eq!(
            split(out),
            vec![
                ("programming..".into(), false),
                ("                      ".into(), true),
                ("addr 0x000000   0%".into(), true),
                ("                      ".into(), true),
                ("addr 0x000100   0%".into(), true),
                ("                      ".into(), true),
                ("done.".into(), false),
            ]
        );
    }

    #[test]
    fn line_ends() {
        // \r\n is a newline; a final \r isn't a redraw; empty lines go.
        assert_eq!(
            split(b"a\r\n\nb\r\rc\r"),
            vec![("a".into(), false), ("b".into(), true), ("c".into(), false)]
        );
        assert_eq!(split(b"x"), vec![("x".into(), false)]);
    }

    #[tokio::test]
    async fn the_environment_is_minimal() {
        let tmp = tempfile::tempdir().unwrap();
        let args: Vec<OsString> = vec!["-c".into(), "env".into()];
        let run = run_tool(
            Path::new("/bin/sh"),
            &args,
            tmp.path(),
            Path::new("/pkg/bin"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(
            run.output.contains("PATH=/pkg/bin:/usr/bin:/bin"),
            "{}",
            run.output
        );
        assert!(!run.output.contains("CARGO"), "{}", run.output);
    }

    #[tokio::test]
    async fn cancel_kills_a_tool_that_ignores_the_request_to_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let args: Vec<OsString> = vec!["-c".into(), "trap '' TERM; sleep 30".into()];
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c.cancel();
        });
        let start = Instant::now();
        let r = run_tool(Path::new("/bin/sh"), &args, tmp.path(), tmp.path(), &cancel).await;
        assert!(matches!(r, Err(RunError::Cancelled)));
        let took = start.elapsed();
        assert!(
            took >= STOP_GRACE && took < STOP_GRACE + Duration::from_secs(3),
            "{took:?}"
        );
    }

    #[tokio::test]
    async fn cancel_stops_the_process_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("child-ran");
        // The child sleeps, then would write a file. Cancelling must stop it.
        let script = format!("(sleep 2; touch {}) & sleep 30", marker.display());
        let args: Vec<OsString> = vec!["-c".into(), script.into()];
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c.cancel();
        });
        let start = Instant::now();
        let r = run_tool(Path::new("/bin/sh"), &args, tmp.path(), tmp.path(), &cancel).await;
        assert!(matches!(r, Err(RunError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!marker.exists(), "the background child was stopped too");
    }
}
