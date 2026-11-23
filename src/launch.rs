//! Starting a program that outlives the call, such as Logix Viewer's apps
//! (Logix Waveforms, Logix Diagrams), which show the waveform viewer's and
//! the diagram's pages.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// Starts `program` fully detached and returns at once: its own session, no
/// input or output, and not stopped when the operation is cancelled or the
/// engine exits. The environment is the same minimal one the tools get
/// (PATH is the system's own folders), plus what a window needs on Linux.
pub fn launch_detached(program: &Path, args: &[OsString], cwd: &Path) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", crate::commands::tool_path(Path::new("/usr/bin")))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The tools' variables, and a desktop session's on Linux.
    let display = [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
    ];
    for var in crate::commands::KEPT_VARS.iter().chain(display.iter()) {
        if let Some(v) = std::env::var_os(var) {
            cmd.env(var, v);
        }
    }
    // Windows: its own process group, with no console, so it outlives the
    // call and Stop's Ctrl+Break never reaches it.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid only changes the child's session, before exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    log::info!(
        "Starting {} {} (detached, in {})",
        program.display(),
        args.iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        cwd.display()
    );
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Can't start {}: {e}", program.display()))?;
    let pid = child.id();
    log::info!("Started {} (process {pid})", program.display());
    // Reap it when it exits, so it doesn't linger as a zombie, and log how
    // it ended: a program that dies at once, such as for a missing library,
    // says nothing else.
    let name = program.display().to_string();
    std::thread::spawn(move || match child.wait() {
        Ok(status) => log::info!("{name} (process {pid}) exited: {status}"),
        Err(e) => log::warn!("{name} (process {pid}): can't wait for it: {e}"),
    });
    Ok(())
}
