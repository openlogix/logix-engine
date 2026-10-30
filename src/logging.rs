//! Logging from docs/command-line.md ("Logs"). Each process writes its own
//! file in the engine folder's logs subfolder, named with the UTC date and
//! the process ID. Warnings and errors also go to stderr. Nothing goes to
//! stdout, which is only for MCP.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use log::{Level, LevelFilter, Log, Metadata, Record};
use time::OffsetDateTime;
use time::macros::format_description;

/// Old log files are deleted so at most this many remain, the current one
/// included.
pub const MAX_LOG_FILES: usize = 20;
/// Old log files are deleted so they use at most this many bytes in total.
pub const MAX_LOG_BYTES: u64 = 50 * 1024 * 1024;

struct Logger {
    file: Mutex<File>,
    file_level: LevelFilter,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.file_level || metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let now = OffsetDateTime::now_utc()
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
            ))
            .unwrap_or_default();
        let line = format!("{now} {:<5} {}\n", record.level(), record.args());
        if record.level() <= self.file_level
            && let Ok(mut f) = self.file.lock()
        {
            let _ = f.write_all(line.as_bytes());
        }
        if record.level() <= Level::Warn {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// The log file name for a process started now: `yyyymmdd-<pid>.log`.
pub fn log_file_name(now: OffsetDateTime, pid: u32) -> String {
    let date = now
        .format(format_description!("[year][month][day]"))
        .unwrap_or_default();
    format!("{date}-{pid}.log")
}

/// Creates the log file in `<engine folder>/logs`, installs the logger,
/// and deletes old log files. Returns the log file path.
pub fn init(engine_folder: &Path, file_level: LevelFilter) -> std::io::Result<PathBuf> {
    let dir = engine_folder.join("logs");
    fs::create_dir_all(&dir)?;
    let path = dir.join(log_file_name(OffsetDateTime::now_utc(), std::process::id()));
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let logger = Logger {
        file: Mutex::new(file),
        file_level,
    };
    log::set_boxed_logger(Box::new(logger)).map_err(|e| std::io::Error::other(e.to_string()))?;
    log::set_max_level(file_level.max(LevelFilter::Warn));
    if let Err(e) = cleanup(&dir, &path) {
        log::warn!("Can't delete old log files in {}: {e}", dir.display());
    }
    Ok(path)
}

/// Deletes old `.log` files in `dir`, newest kept first, so at most
/// MAX_LOG_FILES remain and they use at most MAX_LOG_BYTES. `current` is
/// always kept.
pub fn cleanup(dir: &Path, current: &Path) -> std::io::Result<()> {
    let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path == current || path.extension().is_none_or(|e| e != "log") {
            continue;
        }
        let meta = entry.metadata()?;
        if meta.is_file() {
            files.push((path, meta.modified()?, meta.len()));
        }
    }
    // Newest first.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    let mut count = 1;
    let mut bytes = fs::metadata(current).map(|m| m.len()).unwrap_or(0);
    for (path, _, size) in files {
        if count < MAX_LOG_FILES && bytes + size <= MAX_LOG_BYTES {
            count += 1;
            bytes += size;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn file_name_has_date_and_pid() {
        let t = OffsetDateTime::from_unix_timestamp(1_793_145_600).unwrap(); // 2026-10-28
        assert_eq!(log_file_name(t, 4123), "20261028-4123.log");
    }

    fn make(dir: &Path, name: &str, size: usize, age_secs: u64) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, vec![b'x'; size]).unwrap();
        let t = UNIX_EPOCH + Duration::from_secs(2_000_000_000 - age_secs);
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
        p
    }

    #[test]
    fn cleanup_keeps_the_newest_20_files() {
        let tmp = tempfile::tempdir().unwrap();
        let current = make(tmp.path(), "current.log", 1, 0);
        for i in 0..25 {
            make(tmp.path(), &format!("old-{i:02}.log"), 1, 10 + i);
        }
        let other = make(tmp.path(), "notes.txt", 1, 1000);
        cleanup(tmp.path(), &current).unwrap();
        let mut left: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), 21); // 20 logs plus notes.txt
        assert!(current.exists() && other.exists());
        assert!(left.contains(&"old-18.log".to_string()));
        assert!(!left.contains(&"old-19.log".to_string()));
    }

    #[test]
    fn cleanup_keeps_at_most_50_mb() {
        let tmp = tempfile::tempdir().unwrap();
        let current = make(tmp.path(), "current.log", 1, 0);
        let mb = 1024 * 1024;
        let a = make(tmp.path(), "a.log", 30 * mb, 10);
        let b = make(tmp.path(), "b.log", 15 * mb, 20);
        let c = make(tmp.path(), "c.log", 10 * mb, 30);
        cleanup(tmp.path(), &current).unwrap();
        assert!(a.exists() && b.exists());
        assert!(!c.exists());
    }
}
