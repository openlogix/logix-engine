//! Runs a packaged tool as a subprocess (docs/architecture.md, "Process
//! isolation"): its own working folder and a minimal environment, output
//! captured, and on cancel its whole process tree stopped.

use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A finished tool run.
#[derive(Debug)]
pub struct ToolRun {
    pub status: ExitStatus,
    /// stdout and stderr lines, in the order they arrived.
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
    if cancel.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    let mut path = OsString::from(bin_dir);
    path.push(":/usr/bin:/bin");
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for var in ["HOME", "TMPDIR"] {
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

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
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
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(line);
            }
        });
    }
    drop(tx);

    let mut output = Vec::new();
    let status = loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(l) => output.push(l),
                None => break child.wait().await.map_err(RunError::Spawn)?,
            },
            _ = cancel.cancelled() => {
                kill_tree(pid);
                let _ = child.kill().await;
                log::info!("Cancelled {}", program.display());
                return Err(RunError::Cancelled);
            }
        }
    };
    let output = output.join("\n");
    log::debug!("{} exited with {status}:\n{output}", program.display());
    Ok(ToolRun { status, output })
}

/// Stops the tool and the processes it started, which share its process
/// group.
fn kill_tree(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        // SAFETY: killpg only sends a signal; a stale group ID fails harmlessly.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
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
