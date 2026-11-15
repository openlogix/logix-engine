//! Logging (docs/command-line.md, "Logs"), by the org's log rule (the
//! website repo's `CLAUDE.md`, Log files): every run of an Open Logix
//! program at the top makes a *session folder* in its own `logs` folder,
//! named by its start time, `<yyyymmdd>-<hhmmss>-<pid>` (local time), and
//! every program it starts, and each one those start, writes into that
//! folder, one file per program, named by its chain of programs joined by
//! dots (`logix-ide.logix-engine.log`). Each run appends to its file after a
//! separator line, so nothing is ever renamed. Only the top program deletes
//! old session folders (`clean_sessions`, also the `clean_logs` tool for
//! hosts).
//!
//! The engine is at the top when its host gives no `--log-folder`: it then
//! makes its own session folder in `<engine folder>/logs` and cleans that
//! folder at startup. Under a host, it writes `<log name>.log` in the host's
//! session folder and deletes nothing there. Warnings and errors also go to
//! stderr. Nothing goes to stdout, which is only for MCP.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use log::{Level, LevelFilter, Log, Metadata, Record};
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

/// The engine's own name in log file names.
pub const PROGRAM: &str = "logix-engine";
/// Session folders kept by `clean_sessions`, the current ones included.
pub const MAX_SESSIONS: usize = 10;
/// The most the kept session folders may use in all.
pub const MAX_SESSION_BYTES: u64 = 100 * 1024 * 1024;
/// Session folders older than this are deleted whatever the counts.
pub const MAX_SESSION_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// A session folder whose files changed more recently than this may still be
/// in use (a program in it still running), so it's never deleted.
pub const IN_USE: Duration = Duration::from_secs(24 * 60 * 60);
/// Above this size, a log file gets no more debug lines (the tools' full
/// output); messages still go in. The same limit as Logix IDE's.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// The local time's offset from UTC, read once at startup (`init_local_offset`).
static LOCAL_OFFSET: OnceLock<UtcOffset> = OnceLock::new();

/// Reads the local time's offset from UTC. Call it first in `main`, before
/// any thread starts: the time crate reads it reliably only then on macOS
/// and Linux. Falls back to UTC when it can't be read.
pub fn init_local_offset() {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let _ = LOCAL_OFFSET.set(offset);
}

/// Now, in local time (UTC until `init_local_offset` ran).
pub fn now_local() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(*LOCAL_OFFSET.get().unwrap_or(&UtcOffset::UTC))
}

/// A session folder's name: `<yyyymmdd>-<hhmmss>-<pid>`, the top program's
/// start (local time) and process ID, so names sort by start.
pub fn session_name(at: OffsetDateTime, pid: u32) -> String {
    let started = at
        .format(format_description!(
            "[year][month][day]-[hour][minute][second]"
        ))
        .unwrap_or_default();
    format!("{started}-{pid}")
}

/// Whether `name` is a session folder's name.
pub fn is_session_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('-').collect();
    matches!(parts.as_slice(), [d, t, p]
        if d.len() == 8 && t.len() == 6 && !p.is_empty()
            && [d, t, p].iter().all(|s| s.bytes().all(|c| c.is_ascii_digit())))
}

/// A separator line, written before each run's lines in a log file:
/// `=== 2026-11-15 19:12:05 +02:00  Logix Engine 2026.11.15 started (process 98223) ===`.
pub fn separator(at: OffsetDateTime, what: &str) -> String {
    let when = at
        .format(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second] [offset_hour sign:mandatory]:[offset_minute]"
        ))
        .unwrap_or_default();
    format!("=== {when}  {what} ===\n")
}

/// Whether `name` can be a log name: lowercase letters, digits, `-` and
/// `.` (a chain of program names), not starting or ending with a dot.
pub fn is_log_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.')
}

/// Where this process logs: its session folder and its name in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPlace {
    pub folder: PathBuf,
    /// The chain of program names, such as `logix-ide.logix-engine`.
    pub name: String,
}

impl LogPlace {
    /// This program's log file.
    pub fn file(&self) -> PathBuf {
        self.folder.join(format!("{}.log", self.name))
    }

    /// The log file of a program this one starts, such as `gtkwave`:
    /// `<name>.<child>.log` in the same session folder.
    pub fn child_file(&self, child: &str) -> PathBuf {
        self.folder.join(format!("{}.{child}.log", self.name))
    }
}

static PLACE: OnceLock<LogPlace> = OnceLock::new();

/// Where this process logs, once `init` ran.
pub fn place() -> Option<&'static LogPlace> {
    PLACE.get()
}

/// Opens a log file for appending, creating it and its folder when needed.
pub fn open_append(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

struct Logger {
    file: Mutex<(File, u64, bool)>,
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
        let now = now_local()
            .format(format_description!(
                "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
            ))
            .unwrap_or_default();
        let line = format!("{now} {:<5} {}\n", record.level(), record.args());
        if record.level() <= self.file_level
            && let Ok(mut guard) = self.file.lock()
        {
            let (file, bytes, capped) = &mut *guard;
            write_capped(file, bytes, capped, record.level(), &line);
        }
        if record.level() <= Level::Warn {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            let _ = guard.0.flush();
        }
    }
}

/// Writes a line, leaving out debug lines (the tools' full output) once the
/// file is over MAX_FILE_BYTES, with one note saying so.
fn write_capped(
    file: &mut impl Write,
    bytes: &mut u64,
    capped: &mut bool,
    level: Level,
    line: &str,
) {
    if level >= Level::Debug && *bytes > MAX_FILE_BYTES {
        if !*capped {
            *capped = true;
            let note = format!(
                "Debug lines (the tools' full output) are left out from here: the log is over {} MB.\n",
                MAX_FILE_BYTES / 1024 / 1024
            );
            if file.write_all(note.as_bytes()).is_ok() {
                *bytes += note.len() as u64;
            }
        }
        return;
    }
    if file.write_all(line.as_bytes()).is_ok() {
        *bytes += line.len() as u64;
    }
}

/// Where the engine logs, from its command line: the host's session folder
/// and name, or none (the engine is at the top: its own session folder in
/// `<engine folder>/logs`).
pub fn place_for(
    engine_folder: &Path,
    log_folder: Option<&Path>,
    log_name: Option<&str>,
) -> Result<(LogPlace, bool), String> {
    if let Some(name) = log_name
        && !is_log_name(name)
    {
        return Err(format!(
            "--log-name must be lowercase letters, digits, - and . (such as my-ide.logix-engine), not {name:?}"
        ));
    }
    match log_folder {
        Some(folder) => {
            if !folder.is_absolute() {
                return Err(format!(
                    "--log-folder must be an absolute path, not {}",
                    folder.display()
                ));
            }
            let name = log_name.unwrap_or(PROGRAM).to_string();
            Ok((
                LogPlace {
                    folder: folder.to_path_buf(),
                    name,
                },
                false,
            ))
        }
        None => {
            if log_name.is_some() {
                return Err("--log-name needs --log-folder".to_string());
            }
            let folder = engine_folder
                .join("logs")
                .join(session_name(now_local(), std::process::id()));
            Ok((
                LogPlace {
                    folder,
                    name: PROGRAM.to_string(),
                },
                true,
            ))
        }
    }
}

/// Opens the log file, writes this run's separator, installs the logger
/// and, when the engine is at the top, deletes old session folders in its
/// `logs` folder. Returns the log file path.
pub fn init(place: LogPlace, at_top: bool, file_level: LevelFilter) -> std::io::Result<PathBuf> {
    let path = place.file();
    let mut file = open_append(&path)?;
    let start = separator(
        now_local(),
        &format!(
            "Logix Engine {} started (process {})",
            crate::state::ENGINE_VERSION,
            std::process::id()
        ),
    );
    file.write_all(start.as_bytes())?;
    let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    let logger = Logger {
        file: Mutex::new((file, bytes, false)),
        file_level,
    };
    log::set_boxed_logger(Box::new(logger)).map_err(|e| std::io::Error::other(e.to_string()))?;
    log::set_max_level(file_level.max(LevelFilter::Warn));
    let _ = PLACE.set(place.clone());
    if at_top && let Some(logs) = place.folder.parent() {
        match clean_sessions(logs, &[place.folder.as_path()], SystemTime::now()) {
            Ok(deleted) if !deleted.is_empty() => {
                log::info!("Deleted {} old log session folders", deleted.len())
            }
            Ok(_) => {}
            Err(e) => log::warn!("Can't delete old log folders in {}: {e}", logs.display()),
        }
    }
    Ok(path)
}

/// A session folder and what `clean_sessions` needs to know about it.
struct Session {
    path: PathBuf,
    changed: SystemTime,
    bytes: u64,
}

/// Reads a session folder: its newest change (its files', or the folder's
/// own when it's empty) and size. None when it holds anything but `.log`
/// files, so it's never deleted.
fn read_session(path: &Path) -> std::io::Result<Option<Session>> {
    let mut changed: Option<SystemTime> = None;
    let mut bytes = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        let is_log = entry.path().extension().is_some_and(|e| e == "log");
        if !meta.is_file() || !is_log {
            return Ok(None);
        }
        bytes += meta.len();
        let modified = meta.modified()?;
        changed = Some(changed.map_or(modified, |c| c.max(modified)));
    }
    let changed = match changed {
        Some(c) => c,
        None => fs::metadata(path)?.modified()?,
    };
    Ok(Some(Session {
        path: path.to_path_buf(),
        changed,
        bytes,
    }))
}

/// Deletes old session folders in a `logs` folder, by the org's rule:
/// newest first, keep at most MAX_SESSIONS and MAX_SESSION_BYTES, and
/// delete any older than MAX_SESSION_AGE. Never deleted: the folders in
/// `keep` (the current sessions), folders changed in the last IN_USE (a
/// program in them may still run), and anything that isn't a session
/// folder holding only `.log` files. Returns the deleted folders.
pub fn clean_sessions(
    logs: &Path,
    keep: &[&Path],
    now: SystemTime,
) -> std::io::Result<Vec<PathBuf>> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(logs)? {
        let entry = entry?;
        let name = entry.file_name();
        if !entry.file_type()?.is_dir() || !is_session_name(&name.to_string_lossy()) {
            continue;
        }
        if let Some(s) = read_session(&entry.path())? {
            sessions.push(s);
        }
    }
    // Newest first.
    sessions.sort_by(|a, b| b.changed.cmp(&a.changed).then_with(|| b.path.cmp(&a.path)));
    let age = |s: &Session| now.duration_since(s.changed).unwrap_or_default();
    let mut count = 0;
    let mut bytes = 0;
    let mut deleted = Vec::new();
    for s in sessions {
        let protected = keep.iter().any(|k| *k == s.path) || age(&s) < IN_USE;
        let fits = count < MAX_SESSIONS && bytes + s.bytes <= MAX_SESSION_BYTES;
        if protected || (fits && age(&s) <= MAX_SESSION_AGE) {
            count += 1;
            bytes += s.bytes;
        } else {
            fs::remove_dir_all(&s.path)?;
            deleted.push(s.path);
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    const NOW: u64 = 2_000_000_000;
    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn session_names_and_separators() {
        let t = OffsetDateTime::from_unix_timestamp(1_793_145_600 + 3_723)
            .unwrap()
            .to_offset(UtcOffset::from_hms(2, 0, 0).unwrap()); // 2026-10-28 03:02:03 +02:00
        assert_eq!(session_name(t, 4123), "20261028-030203-4123");
        assert!(is_session_name("20261028-030203-4123"));
        for bad in [
            "notes",
            "2026-10-28",
            "20261028-030203",
            "20261028-0302-1",
            "x0261028-030203-1",
        ] {
            assert!(!is_session_name(bad), "{bad}");
        }
        assert_eq!(
            separator(t, "GTKWave started for hardware.vcd"),
            "=== 2026-10-28 03:02:03 +02:00  GTKWave started for hardware.vcd ===\n"
        );
    }

    #[test]
    fn log_names_are_chains_of_program_names() {
        for good in [
            "logix-engine",
            "logix-ide.logix-engine",
            "my-ide2.logix-engine",
        ] {
            assert!(is_log_name(good), "{good}");
        }
        for bad in ["", ".x", "x.", "a..b", "Logix", "a/b", "a b"] {
            assert!(!is_log_name(bad), "{bad}");
        }
        let place = LogPlace {
            folder: PathBuf::from("/s"),
            name: "logix-ide.logix-engine".into(),
        };
        assert_eq!(place.file(), Path::new("/s/logix-ide.logix-engine.log"));
        assert_eq!(
            place.child_file("gtkwave"),
            Path::new("/s/logix-ide.logix-engine.gtkwave.log")
        );
    }

    #[test]
    fn the_engine_is_at_the_top_without_a_log_folder() {
        // Absolute on every OS (on Windows, /home/... isn't).
        let base = std::env::temp_dir();
        let engine = &base.join(".logix/claude-code/engine");
        let (place, top) = place_for(engine, None, None).unwrap();
        assert!(top);
        assert_eq!(place.name, "logix-engine");
        assert!(place.folder.starts_with(engine.join("logs")));
        assert!(is_session_name(
            &place.folder.file_name().unwrap().to_string_lossy()
        ));
        let host = &base.join(".logix/ide/logs/20261115-171205-4412");
        let (place, top) = place_for(engine, Some(host), Some("logix-ide.logix-engine")).unwrap();
        assert!(!top);
        assert_eq!(place.file(), host.join("logix-ide.logix-engine.log"));
        assert!(place_for(engine, None, Some("x")).is_err());
        assert!(place_for(engine, Some(Path::new("rel")), None).is_err());
        assert!(place_for(engine, Some(host), Some("Bad Name")).is_err());
    }

    #[test]
    fn debug_lines_stop_at_the_size_cap() {
        let mut out = Vec::new();
        let (mut bytes, mut capped) = (MAX_FILE_BYTES, false);
        write_capped(
            &mut out,
            &mut bytes,
            &mut capped,
            Level::Debug,
            "tool line\n",
        );
        assert_eq!(out, b"tool line\n");
        write_capped(&mut out, &mut bytes, &mut capped, Level::Debug, "dropped\n");
        write_capped(&mut out, &mut bytes, &mut capped, Level::Debug, "dropped\n");
        write_capped(&mut out, &mut bytes, &mut capped, Level::Info, "message\n");
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("dropped"));
        assert_eq!(text.matches("left out from here").count(), 1);
        assert!(text.ends_with("message\n"));
    }

    /// A session folder whose one log file is `size` bytes, changed
    /// `age_days` days before NOW.
    fn session(logs: &Path, name: &str, size: usize, age_days: u64) -> PathBuf {
        let dir = logs.join(name);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("logix-ide.log");
        fs::write(&file, vec![b'x'; size]).unwrap();
        let t = UNIX_EPOCH + Duration::from_secs(NOW - age_days * DAY);
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(t)
            .unwrap();
        dir
    }

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(NOW)
    }

    #[test]
    fn cleanup_keeps_the_newest_10_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let current = session(tmp.path(), "20261115-120000-1", 1, 0);
        let old: Vec<_> = (0..15)
            .map(|i| session(tmp.path(), &format!("20261101-1200{i:02}-2"), 1, 2 + i))
            .collect();
        let notes = tmp.path().join("notes.txt");
        fs::write(&notes, "mine").unwrap();
        let deleted = clean_sessions(tmp.path(), &[current.as_path()], now()).unwrap();
        assert_eq!(deleted.len(), 6);
        assert!(current.exists() && notes.exists());
        assert!(old[8].exists() && !old[9].exists());
    }

    #[test]
    fn cleanup_keeps_at_most_100_mb_and_30_days() {
        let tmp = tempfile::tempdir().unwrap();
        let mb = 1024 * 1024;
        let a = session(tmp.path(), "20261115-120000-1", 60 * mb, 2);
        let b = session(tmp.path(), "20261114-120000-2", 30 * mb, 3);
        let c = session(tmp.path(), "20261113-120000-3", 20 * mb, 4);
        let ancient = session(tmp.path(), "20261001-120000-4", 1, 31);
        clean_sessions(tmp.path(), &[], now()).unwrap();
        assert!(a.exists() && b.exists());
        assert!(!c.exists() && !ancient.exists());
    }

    #[test]
    fn cleanup_never_deletes_sessions_in_use_or_folders_with_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        // Over every limit, but changed in the last day: maybe in use.
        let recent: Vec<_> = (0..12)
            .map(|i| session(tmp.path(), &format!("20261115-1200{i:02}-1"), 1, 0))
            .collect();
        let ancient = session(tmp.path(), "20261001-120000-2", 1, 40);
        fs::write(ancient.join("keep-me.txt"), "mine").unwrap();
        let deleted = clean_sessions(tmp.path(), &[], now()).unwrap();
        assert!(deleted.is_empty());
        assert!(recent.iter().all(|p| p.exists()) && ancient.exists());
    }
}
