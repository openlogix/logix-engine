//! Debug session locks across processes. A debug session holds its port's
//! lock file for as long as it is open, so that other Logix Engine processes
//! can tell the board is in a session even while the session has the port
//! closed, such as after the board restarted, until it opens it again.
//! program_board and open_debug in another process then refuse that board
//! (docs/debugger.md, Sessions in other programs).

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;

/// A held lock: released when dropped (the OS releases it also when the
/// process ends).
pub struct PortLock {
    _file: File,
}

/// The lock file of a port: the same for the names a port may go by
/// (on macOS /dev/tty.x and /dev/cu.x; on Windows COM8 and com8).
fn lock_path(port: &str) -> PathBuf {
    file_path(port, "lock")
}

/// The file beside the lock that names its holder's process. Not the lock
/// file itself: on Windows a locked file can't be read by other processes.
fn pid_path(port: &str) -> PathBuf {
    file_path(port, "pid")
}

fn file_path(port: &str, extension: &str) -> PathBuf {
    let port = port.replace("/dev/tty.", "/dev/cu.").to_lowercase();
    let name: String = port
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::temp_dir()
        .join("logix-debug-sessions")
        .join(format!("{name}.{extension}"))
}

/// Whether two port names name the same port.
pub fn same_port(a: &str, b: &str) -> bool {
    lock_path(a) == lock_path(b)
}

fn open_file(port: &str) -> std::io::Result<File> {
    let path = lock_path(port);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// What another process holding a port's lock is known by.
#[derive(Debug, PartialEq)]
pub struct Holder {
    /// The holder's process ID, when it wrote one.
    pub pid: Option<u32>,
}

impl Holder {
    /// "(process 1234)", or "" when the ID isn't known.
    pub fn process(&self) -> String {
        self.pid
            .map(|p| format!(" (process {p})"))
            .unwrap_or_default()
    }
}

/// Takes the lock of `port`: Ok(Some) when taken, Err with the holder when
/// another process has it, and Ok(None) when the lock file can't be used
/// (then the session goes on without one).
pub fn take(port: &str) -> Result<Option<PortLock>, Holder> {
    let Ok(file) = open_file(port) else {
        return Ok(None);
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = std::fs::write(pid_path(port), std::process::id().to_string());
            Ok(Some(PortLock { _file: file }))
        }
        Err(TryLockError::WouldBlock) => Err(holder(port)),
        Err(TryLockError::Error(_)) => Ok(None),
    }
}

/// The other process holding the lock of `port`, if any. Call it only for
/// ports this process has no session on: within one process, a session's
/// own lock reads as held on some systems.
pub fn holder_of(port: &str) -> Option<Holder> {
    let file = open_file(port).ok()?;
    match file.try_lock() {
        Ok(()) => None, // Dropping the file releases it.
        Err(TryLockError::WouldBlock) => Some(holder(port)),
        Err(TryLockError::Error(_)) => None,
    }
}

/// The holder of a held lock. Its pid file may be from an earlier holder
/// for a moment, while a new one is writing it, or missing.
fn holder(port: &str) -> Holder {
    let text = std::fs::read_to_string(pid_path(port)).unwrap_or_default();
    Holder {
        pid: text.trim().parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_of_one_port_share_a_lock() {
        assert_eq!(
            lock_path("/dev/tty.usbmodem1102"),
            lock_path("/dev/cu.usbmodem1102")
        );
        assert_eq!(lock_path("COM8"), lock_path("com8"));
        assert_ne!(lock_path("COM8"), lock_path("COM9"));
        let name = lock_path("/dev/ttyACM0");
        assert_eq!(
            name.file_name().unwrap().to_str().unwrap(),
            "_dev_ttyacm0.lock"
        );
    }

    #[test]
    fn a_held_lock_is_seen_until_dropped() {
        let port = format!("test-port-{}", std::process::id());
        let lock = take(&port).unwrap().expect("lock file usable");
        // A second handle in this process stands in for another process:
        // both flock and LockFileEx lock per handle.
        let h = holder_of(&port).expect("held");
        assert_eq!(h.pid, Some(std::process::id()));
        assert!(take(&port).is_err());
        drop(lock);
        assert_eq!(holder_of(&port), None);
        let _ = std::fs::remove_file(lock_path(&port));
        let _ = std::fs::remove_file(pid_path(&port));
    }
}
