//! Showing pages in Logix Viewer (docs/requirements.md, Logix Viewer):
//! logix-core's `logix-viewer` program, run as one app per kind of page
//! (Logix Waveforms, Logix Diagrams), one running copy per user however
//! many engines run.
//!
//! The engine connects to each kind's app over a per-user local socket,
//! starting the app first when nothing answers, and keeps the connection
//! while it runs: the windows it opens are its own, and close when it
//! exits (the connection drops) or on `close_viewers`. Each request is one
//! JSON line, answered by one (logix-core's `apps/viewer/src/protocol.rs`).
//!
//! There's no browser fallback (the user's decision, 2026-11-22): when the
//! app can't start, the caller gets the reason as `viewer_problem`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use interprocess::local_socket::{Stream, prelude::*};
use serde::Deserialize;
use serde_json::{Value, json};

/// Points the engine at a folder of viewer apps instead of logix-core's,
/// such as `bundle.py`'s output in a logix-core clone, for developing
/// them (docs/command-line.md, Diagnostic settings).
pub const DEV_VIEWER_VAR: &str = "LOGIX_ENGINE_DEV_VIEWER_FOLDER";

/// The logix-core component that holds the apps.
pub const COMPONENT: &str = "logix-viewer";

/// The page contract the engine's pages need (`window.logixHost`).
pub const CONTRACT: u32 = 1;

/// How long a starting app has to answer.
const START_WAIT: Duration = Duration::from_secs(10);

/// The kinds of page, as `apps.json` names them.
pub const WAVEFORMS: &str = "waveforms";
pub const DIAGRAM: &str = "diagram";

/// One app in `apps.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub slot: u32,
}

#[derive(Debug, Deserialize)]
struct AppsFile {
    apps: Vec<App>,
}

/// A page to show.
pub struct Page<'a> {
    pub kind: &'a str,
    pub page: &'a Path,
    pub title: String,
    /// Files the page may write, by id, such as `view`.
    pub declared: BTreeMap<String, PathBuf>,
    pub source_folder: Option<&'a Path>,
}

/// This engine's open connections, by kind.
fn connections() -> &'static Mutex<BTreeMap<String, Connection>> {
    static C: OnceLock<Mutex<BTreeMap<String, Connection>>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

struct Connection {
    stream: BufReader<Stream>,
    next_id: u64,
}

impl Connection {
    /// Sends one request and reads its answer.
    fn call(&mut self, mut request: Value) -> std::io::Result<Value> {
        self.next_id += 1;
        request["id"] = json!(self.next_id);
        let line = request.to_string() + "\n";
        self.stream.get_mut().write_all(line.as_bytes())?;
        self.stream.get_mut().flush()?;
        let mut answer = String::new();
        if self.stream.read_line(&mut answer)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&answer).map_err(std::io::Error::other)
    }
}

/// The folder with the apps and `apps.json`: the developer setting's, else
/// logix-core's component.
pub fn apps_folder(packages: &crate::packages::PackageManager) -> Result<PathBuf, String> {
    if let Some(dev) = std::env::var_os(DEV_VIEWER_VAR).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dev));
    }
    let dir = packages.component_dir(COMPONENT).map_err(|e| e.message)?;
    if dir.join("apps.json").is_file() {
        Ok(dir)
    } else {
        Err(
            "This logix-core has no Logix Viewer yet, so pages can't open in a window: \
             update the engine's packages (check_updates)."
                .into(),
        )
    }
}

/// The app for a kind of page; the generic one for kinds without their own.
pub fn app_for(folder: &Path, kind: &str) -> Result<App, String> {
    let text = std::fs::read_to_string(folder.join("apps.json"))
        .map_err(|e| format!("Can't read Logix Viewer's apps.json: {e}"))?;
    let file: AppsFile =
        serde_json::from_str(&text).map_err(|e| format!("Logix Viewer's apps.json: {e}"))?;
    let pick = |k: &str| file.apps.iter().find(|a| a.kind == k).cloned();
    pick(kind)
        .or_else(|| pick("viewer"))
        .ok_or_else(|| format!("Logix Viewer has no app for {kind} pages."))
}

/// The app's program: inside its bundle on macOS.
pub fn program(folder: &Path, app: &App) -> PathBuf {
    if cfg!(target_os = "macos") {
        folder
            .join(format!("{}.app", app.name))
            .join("Contents/MacOS/logix-viewer")
    } else if cfg!(windows) {
        folder.join("logix-viewer.exe")
    } else {
        folder.join("logix-viewer")
    }
}

/// The app's socket, per user: `~/.logix/viewer/<kind>.sock` (kept short:
/// a Unix socket path holds about 100 characters), or a named pipe's name
/// on Windows.
pub fn socket(kind: &str) -> Result<String, String> {
    if cfg!(windows) {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        return Ok(format!("logix-viewer-{kind}-{user}"));
    }
    let home = crate::paths::home_folder().ok_or("Can't find the home folder.")?;
    let path = socket_in(&home, kind);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// The socket file in a home folder.
fn socket_in(home: &Path, kind: &str) -> PathBuf {
    home.join(".logix")
        .join("viewer")
        .join(format!("{kind}.sock"))
}

fn connect(name: &str) -> std::io::Result<Stream> {
    #[cfg(windows)]
    let n = name.to_ns_name::<interprocess::local_socket::GenericNamespaced>()?;
    #[cfg(not(windows))]
    let n = name.to_fs_name::<interprocess::local_socket::GenericFilePath>()?;
    Stream::connect(n)
}

/// Connects to the kind's app, starting it when nothing answers, and says
/// hello. Blocking: call it off the async runtime.
fn open_connection(folder: &Path, app: &App) -> Result<Connection, String> {
    let name = socket(&app.kind)?;
    let stream = match connect(&name) {
        Ok(s) => s,
        Err(_) => {
            let exe = program(folder, app);
            if !exe.is_file() {
                return Err(format!(
                    "{} isn't installed ({} is missing).",
                    app.name,
                    exe.display()
                ));
            }
            let args: [std::ffi::OsString; 6] = [
                "--name".into(),
                app.name.clone().into(),
                "--socket".into(),
                name.clone().into(),
                "--slot".into(),
                app.slot.to_string().into(),
            ];
            let cwd = exe.parent().unwrap_or(folder);
            crate::launch::launch_detached(&exe, &args, cwd)
                .map_err(|e| format!("{} can't start: {e}", app.name))?;
            let started = Instant::now();
            loop {
                match connect(&name) {
                    Ok(s) => break s,
                    Err(e) if started.elapsed() > START_WAIT => {
                        return Err(not_started(&app.name, &exe, &e));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
    };
    let mut conn = Connection {
        stream: BufReader::new(stream),
        next_id: 0,
    };
    let client = format!(
        "logix-engine {} (process {})",
        crate::state::ENGINE_VERSION,
        std::process::id()
    );
    let answer = conn
        .call(json!({"type": "hello", "contract": CONTRACT, "client": client}))
        .map_err(|e| format!("{} doesn't answer: {e}", app.name))?;
    if answer["ok"].as_bool() != Some(true) {
        return Err(answer["problem"]
            .as_str()
            .unwrap_or("Logix Viewer refused")
            .to_string());
    }
    Ok(conn)
}

/// Why an app didn't start, in plain words. On Linux, the libraries its
/// program needs that aren't installed, from `ldd`, since a missing one
/// stops it before it can say anything (libxdo, found in the Linux test,
/// 2026-11-23).
fn not_started(name: &str, exe: &Path, error: &std::io::Error) -> String {
    if !cfg!(target_os = "linux") {
        return format!("{name} didn't start: {error}.");
    }
    let missing = std::process::Command::new("ldd")
        .arg(exe)
        .output()
        .map(|o| missing_libraries(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    linux_reason(name, &missing, error)
}

/// The Linux message: the missing libraries when `ldd` named some, else
/// what it usually needs.
fn linux_reason(name: &str, missing: &[String], error: &std::io::Error) -> String {
    if missing.is_empty() {
        format!(
            "{name} didn't start: {error}. On Linux it needs WebKitGTK and GTK (the libwebkit2gtk-4.1 and libgtk-3 packages); with no screen, such as over SSH, no window can open."
        )
    } else {
        format!(
            "{name} didn't start: it needs {}, which isn't installed on this computer. Install the package that has it, then try again.",
            missing.join(", ")
        )
    }
}

/// The libraries `ldd`'s output says are missing (`libxdo.so.3 => not found`).
fn missing_libraries(ldd: &str) -> Vec<String> {
    ldd.lines()
        .filter_map(|l| l.trim().strip_suffix("=> not found"))
        .map(|l| l.trim().to_string())
        .collect()
}

/// Shows a page in its kind's app: opens its window, or reloads the one
/// that shows it already. The reason, in plain words, when it can't.
/// Blocking: call it off the async runtime.
pub fn show(folder: &Path, page: &Page) -> Result<(), String> {
    let app = app_for(folder, page.kind)?;
    let request = json!({
        "type": "open",
        "page": page.page,
        "title": page.title,
        "declared": page.declared,
        "source_folder": page.source_folder,
    });
    let mut conns = connections().lock().unwrap();
    // A kept connection may be stale (the app quit when its last window
    // closed): one retry on a fresh one.
    for attempt in 0..2 {
        if attempt == 1 || !conns.contains_key(&app.kind) {
            let c = open_connection(folder, &app)?;
            conns.insert(app.kind.clone(), c);
        }
        match conns.get_mut(&app.kind).unwrap().call(request.clone()) {
            Ok(answer) if answer["ok"].as_bool() == Some(true) => return Ok(()),
            Ok(answer) => {
                return Err(answer["problem"]
                    .as_str()
                    .unwrap_or("Logix Viewer refused")
                    .into());
            }
            Err(_) => {
                conns.remove(&app.kind);
            }
        }
    }
    Err(format!("{} stopped answering.", app.name))
}

/// Closes this engine's windows: of one kind or all, one page or all of
/// them. Returns how many closed. Blocking: call it off the async runtime.
pub fn close(kind: Option<&str>, page: Option<&Path>) -> usize {
    let mut conns = connections().lock().unwrap();
    let mut closed = 0;
    let kinds: Vec<String> = conns
        .keys()
        .filter(|k| kind.is_none_or(|want| *k == want))
        .cloned()
        .collect();
    for k in kinds {
        let mut request = json!({"type": "close"});
        if let Some(p) = page {
            request["page"] = json!(p);
        }
        match conns.get_mut(&k).unwrap().call(request) {
            Ok(answer) => {
                closed += answer["done"]
                    .as_str()
                    .and_then(|d| d.strip_prefix("closed "))
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(0);
            }
            Err(_) => {
                conns.remove(&k);
            }
        }
    }
    closed
}

/// Shows a page for the person at the screen. None when it opened; else
/// why not, for the tool's `viewer_problem`. Blocks (starting an app can
/// take seconds), so on the engine's runtime it moves off the async
/// threads.
pub fn present(packages: &crate::packages::PackageManager, page: &Page) -> Option<String> {
    let work = || {
        let problem = apps_folder(packages)
            .and_then(|folder| show(&folder, page))
            .err();
        if let Some(p) = &problem {
            log::warn!("Didn't show {}: {p}", page.page.display());
        }
        problem
    };
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

/// The app's name for a kind, for window titles; the kind itself when the
/// apps can't be read.
pub fn app_name(packages: &crate::packages::PackageManager, kind: &str) -> String {
    apps_folder(packages)
        .and_then(|f| app_for(&f, kind))
        .map(|a| a.name)
        .unwrap_or_else(|_| kind.to_string())
}

/// A window title: "Logix Waveforms · blinky · main_tb".
pub fn title(app_name: &str, parts: &[&str]) -> String {
    std::iter::once(app_name)
        .chain(parts.iter().copied().filter(|p| !p.is_empty()))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn ldd_names_the_missing_libraries() {
        let ldd = "\tlinux-vdso.so.1 (0x00007ffd)\n\tlibxdo.so.3 => not found\n\tlibgtk-3.so.0 => /lib/x86_64-linux-gnu/libgtk-3.so.0 (0x7f00)\n\tlibfoo.so.1 => not found\n";
        assert_eq!(
            super::missing_libraries(ldd),
            ["libxdo.so.3", "libfoo.so.1"]
        );
        assert!(super::missing_libraries("\tlibc.so.6 => /lib/libc.so.6 (0x7f)\n").is_empty());
    }

    #[test]
    fn the_linux_reason_names_what_is_missing() {
        let e = std::io::Error::from(std::io::ErrorKind::NotFound);
        let named = super::linux_reason("Logix Waveforms", &["libxdo.so.3".into()], &e);
        assert!(
            named.contains("needs libxdo.so.3, which isn't installed"),
            "{named}"
        );
        let general = super::linux_reason("Logix Waveforms", &[], &e);
        assert!(general.contains("WebKitGTK"), "{general}");
    }

    use super::*;

    #[test]
    fn finds_each_kind_s_app_and_the_generic_one() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("apps.json"),
            r#"{"apps": [
                {"kind": "waveforms", "name": "Logix Waveforms", "slot": 0},
                {"kind": "diagram", "name": "Logix Diagrams", "slot": 1},
                {"kind": "viewer", "name": "Logix Viewer", "slot": 2}]}"#,
        )
        .unwrap();
        assert_eq!(
            app_for(tmp.path(), "diagram").unwrap().name,
            "Logix Diagrams"
        );
        assert_eq!(app_for(tmp.path(), "report").unwrap().name, "Logix Viewer");
        let app = app_for(tmp.path(), WAVEFORMS).unwrap();
        let exe = program(tmp.path(), &app);
        if cfg!(target_os = "macos") {
            assert!(exe.ends_with("Logix Waveforms.app/Contents/MacOS/logix-viewer"));
        }
    }

    #[test]
    fn titles_name_the_app_and_the_page() {
        assert_eq!(
            title("Logix Waveforms", &["blinky", "main_tb"]),
            "Logix Waveforms · blinky · main_tb"
        );
        assert_eq!(
            title("Logix Diagrams", &["", "main"]),
            "Logix Diagrams · main"
        );
    }

    #[test]
    fn sockets_are_short_enough_for_unix() {
        let s = socket_in(Path::new("/Users/someone-with-a-long-name"), WAVEFORMS);
        assert!(s.ends_with(".logix/viewer/waveforms.sock"));
        assert!(s.as_os_str().len() < 100, "{}", s.display());
    }
}
