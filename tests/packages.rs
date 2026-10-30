//! Package install and repair, against a local HTTP server serving small
//! test packages. The real oss-cad-suite download is in the ignored test
//! `real_oss_cad_suite_install` (run: cargo test --test packages -- --ignored).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use logix_engine::errors::{ActionCode, Category};
use logix_engine::packages::manifest::{self, Manifest};
use logix_engine::packages::{PackageManager, RECORD_FILE};
use logix_engine::progress::{NoProgress, Phases, ProgressSink};
use logix_engine::state::EngineState;
use logix_engine::tools::get_engine_info::PackageStatus;
use logix_engine::tools::{Call, install_packages};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const PLATFORM: &str = "test-platform";

// ----- A small HTTP server.

#[derive(Clone)]
struct Served {
    body: Vec<u8>,
    /// Stop after this many bytes and wait, to test cancelling.
    stall_after: Option<usize>,
}

struct Server {
    base: String,
    requests: Arc<AtomicUsize>,
}

fn serve(files: HashMap<String, Served>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let files = Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let files = files.clone();
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || handle(stream, &files));
        }
    });
    Server { base, requests }
}

fn handle(mut stream: TcpStream, files: &HashMap<String, Served>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
    }
    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
    let Some(file) = files.get(&path) else {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        file.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    match file.stall_after {
        Some(n) => {
            let _ = stream.write_all(&file.body[..n]);
            let _ = stream.flush();
            std::thread::sleep(Duration::from_secs(30));
        }
        None => {
            let _ = stream.write_all(&file.body);
        }
    }
}

// ----- Test packages.

fn tgz(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    build(&mut builder);
    let tar = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

fn add_file(b: &mut tar::Builder<Vec<u8>>, path: &str, data: &[u8], mode: u32) {
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(mode);
    h.set_entry_type(tar::EntryType::Regular);
    b.append_data(&mut h, path, data).unwrap();
}

fn add_symlink(b: &mut tar::Builder<Vec<u8>>, path: &str, target: &str) {
    let mut h = tar::Header::new_gnu();
    h.set_size(0);
    h.set_mode(0o777);
    h.set_entry_type(tar::EntryType::Symlink);
    b.append_link(&mut h, path, target).unwrap();
}

/// A good package like Apio's: flat, with a tool, a symlink and a big file
/// (for progress and cancel tests).
fn good_package() -> Vec<u8> {
    tgz(|b| {
        add_file(b, "./BUILD-INFO.json", b"{}", 0o644);
        add_file(b, "./bin/tool", b"#!/bin/sh\necho hi\n", 0o755);
        add_symlink(b, "./bin/tool-link", "tool");
        let big: Vec<u8> = (0..400_000u32).flat_map(|i| i.to_le_bytes()).collect();
        add_file(b, "./share/data.bin", &big, 0o644);
    })
}

fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn manifest_for(name: &str, version: &str, file: &str, data: &[u8], sha: Option<&str>) -> Manifest {
    let json = serde_json::json!({
        "packages": [{
            "name": name,
            "version": version,
            "description": "test package",
            "files": { PLATFORM: {
                "owner": "test", "repo": "pkg", "tag": version, "file": file,
                "size": data.len(), "sha256": sha.map(str::to_string).unwrap_or_else(|| sha256(data)),
            }}
        }]
    });
    Manifest::parse(&json.to_string()).unwrap()
}

fn url_path(version: &str, file: &str) -> String {
    format!("/test/pkg/releases/download/{version}/{file}")
}

struct Setup {
    _tmp: tempfile::TempDir,
    engine: std::path::PathBuf,
    server: Server,
    pm: PackageManager,
}

fn setup(data: Vec<u8>, sha: Option<&str>, stall_after: Option<usize>) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let engine = tmp.path().canonicalize().unwrap().join("engine");
    std::fs::create_dir(&engine).unwrap();
    let (version, file) = ("2026-01-02", "pkg-test-20260102.tgz");
    let manifest = manifest_for("pkg", version, file, &data, sha);
    let mut files = HashMap::new();
    files.insert(
        url_path(version, file),
        Served {
            body: data,
            stall_after,
        },
    );
    let server = serve(files);
    let pm = PackageManager::new(&engine, Arc::new(manifest), PLATFORM, &server.base);
    Setup {
        _tmp: tmp,
        engine,
        server,
        pm,
    }
}

fn phases() -> Phases {
    let p = Phases::new(Arc::new(NoProgress), 1);
    p.start("Installing pkg");
    p
}

fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

// ----- Tests.

#[tokio::test]
async fn installs_a_missing_package() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    assert_eq!(s.pm.status(&spec), PackageStatus::Missing);
    assert_eq!(s.pm.needing_install(&["pkg"]).unwrap().len(), 1);

    s.pm.install(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(s.pm.status(&spec), PackageStatus::Installed);
    assert!(s.pm.needing_install(&["pkg"]).unwrap().is_empty());
    let dir = s.pm.package_dir(&spec);
    assert_eq!(dir, s.engine.join("packages/pkg/2026-01-02"));
    assert_eq!(
        std::fs::read_to_string(dir.join("bin/tool-link")).unwrap(),
        "#!/bin/sh\necho hi\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("bin/tool"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the tool stays executable");
    }
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(RECORD_FILE)).unwrap()).unwrap();
    assert_eq!(record["name"], "pkg");
    assert_eq!(record["version"], "2026-01-02");
    // Nothing left over: no partial download, no unpack folder.
    assert!(entries(&s.engine.join("downloads")).is_empty());
    assert_eq!(entries(&s.engine.join("packages/pkg")), vec!["2026-01-02"]);
}

#[tokio::test]
async fn reports_increasing_progress() {
    #[derive(Default)]
    struct Rec(Mutex<Vec<f64>>);
    impl ProgressSink for Rec {
        fn report(&self, progress: f64, _total: f64, _m: Option<String>) {
            self.0.lock().unwrap().push(progress);
        }
    }
    let s = setup(good_package(), None, None);
    let rec = Arc::new(Rec::default());
    let phases = Phases::new(rec.clone(), 1);
    let spec = s.pm.needing_install(&["pkg"]).unwrap();
    s.pm.ensure(&spec, true, &phases, &CancellationToken::new())
        .await
        .unwrap();
    let values = rec.0.lock().unwrap().clone();
    assert!(values.len() >= 3, "{values:?}");
    assert!(values.windows(2).all(|w| w[1] > w[0]), "{values:?}");
    assert!(*values.last().unwrap() <= 99.0);
}

#[tokio::test]
async fn a_checksum_mismatch_is_a_setup_error_and_installs_nothing() {
    let wrong = "0".repeat(64);
    let s = setup(good_package(), Some(&wrong), None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let err =
        s.pm.install(&spec, &phases(), &CancellationToken::new())
            .await
            .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert_eq!(err.action.unwrap().code, ActionCode::InstallPackages);
    assert!(err.message.contains("checksum"), "{}", err.message);
    assert_eq!(s.pm.status(&spec), PackageStatus::Missing);
    assert!(entries(&s.engine.join("downloads")).is_empty());
    assert!(entries(&s.engine.join("packages/pkg")).is_empty());
}

#[tokio::test]
async fn a_broken_package_is_repaired() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let dir = s.pm.package_dir(&spec);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin/leftover"), "x").unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Broken);

    s.pm.install(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Installed);
    assert!(!dir.join("bin/leftover").exists());
    assert!(dir.join("bin/tool").exists());
}

#[tokio::test]
async fn a_record_from_another_release_is_broken() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    s.pm.install(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();
    let record = s.pm.package_dir(&spec).join(RECORD_FILE);
    let text = std::fs::read_to_string(&record).unwrap();
    let sha = sha256(&good_package());
    std::fs::write(&record, text.replace(&sha, &"1".repeat(64))).unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Broken);
    std::fs::remove_file(&record).unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Broken);
}

#[tokio::test]
async fn auto_install_off_is_a_setup_error_listing_the_packages() {
    let s = setup(good_package(), None, None);
    let missing = s.pm.needing_install(&["pkg"]).unwrap();
    let err =
        s.pm.ensure(
            &missing,
            false,
            &Phases::new(Arc::new(NoProgress), 1),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert_eq!(err.action.unwrap().code, ActionCode::InstallPackages);
    assert!(
        err.message.contains("pkg 2026-01-02 (missing)"),
        "{}",
        err.message
    );
    assert_eq!(
        s.server.requests.load(Ordering::SeqCst),
        0,
        "no network use"
    );
}

#[tokio::test]
async fn a_failed_download_is_a_setup_error() {
    let s = setup(good_package(), None, None);
    // A version the server doesn't have: 404.
    let manifest = manifest_for(
        "pkg",
        "2026-01-03",
        "pkg-test-20260103.tgz",
        &good_package(),
        None,
    );
    let pm = PackageManager::new(&s.engine, Arc::new(manifest), PLATFORM, &s.server.base);
    let spec = pm.manifest().get("pkg").unwrap().clone();
    let err = pm
        .install(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert!(err.details.contains("404"), "{}", err.details);

    // No server at all.
    let pm = PackageManager::new(
        &s.engine,
        Arc::new(pm.manifest().clone()),
        PLATFORM,
        "http://127.0.0.1:9",
    );
    let err = pm
        .install(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert_eq!(err.action.unwrap().code, ActionCode::InstallPackages);
}

#[tokio::test]
async fn cancelling_a_download_installs_nothing() {
    let data = good_package();
    let half = data.len() / 2;
    let s = setup(data, None, Some(half));
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c.cancel();
    });
    let err = s.pm.install(&spec, &phases(), &cancel).await.unwrap_err();
    assert_eq!(err.category, Category::Cancelled);
    assert_eq!(s.pm.status(&spec), PackageStatus::Missing);
    assert!(entries(&s.engine.join("downloads")).is_empty());
}

#[tokio::test]
async fn concurrent_installs_download_once() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let (a, b) = (s.pm.clone(), s.pm.clone());
    let (sa, sb) = (spec.clone(), spec.clone());
    let ta =
        tokio::spawn(async move { a.install(&sa, &phases(), &CancellationToken::new()).await });
    let tb =
        tokio::spawn(async move { b.install(&sb, &phases(), &CancellationToken::new()).await });
    ta.await.unwrap().unwrap();
    tb.await.unwrap().unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Installed);
    assert_eq!(s.server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn entries_outside_the_package_folder_are_refused() {
    // An entry with "..", written into the header directly since tar's
    // builder refuses it.
    let dotdot = tgz(|b| {
        let mut h = tar::Header::new_gnu();
        let name = b"../evil.txt";
        h.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
        h.set_size(4);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        b.append(&h, &b"evil"[..]).unwrap();
    });
    // A symlink out of the folder, then a file written through it.
    let through_link = tgz(|b| {
        add_symlink(b, "./out", "/tmp");
        add_file(b, "./out/logix-engine-evil.txt", b"evil", 0o644);
    });
    for (what, data) in [("dotdot", dotdot), ("symlink", through_link)] {
        let s = setup(data, None, None);
        let spec = s.pm.manifest().get("pkg").unwrap().clone();
        let err =
            s.pm.install(&spec, &phases(), &CancellationToken::new())
                .await
                .unwrap_err();
        assert_eq!(err.category, Category::Setup, "{what}");
        assert_eq!(s.pm.status(&spec), PackageStatus::Missing, "{what}");
        assert!(!s.engine.join("packages/evil.txt").exists(), "{what}");
        assert!(!s.engine.join("packages/pkg/evil.txt").exists(), "{what}");
        assert!(!Path::new("/tmp/logix-engine-evil.txt").exists(), "{what}");
        assert!(entries(&s.engine.join("packages/pkg")).is_empty(), "{what}");
    }
}

// ----- The install_packages tool.

fn state_for(s: &Setup, auto_install: bool) -> EngineState {
    EngineState {
        engine_folder: s.engine.display().to_string(),
        engine_folder_resolved: s.engine.clone(),
        auto_install,
        command_line: vec![],
        log_file: s.engine.join("logs/test.log"),
        packages: s.pm.clone(),
    }
}

#[derive(Default)]
struct Messages(Mutex<Vec<(f64, f64, Option<String>)>>);
impl ProgressSink for Messages {
    fn report(&self, progress: f64, total: f64, message: Option<String>) {
        self.0.lock().unwrap().push((progress, total, message));
    }
}

#[tokio::test]
async fn install_packages_tool_installs_once_even_with_auto_install_off() {
    let s = setup(good_package(), None, None);
    let state = state_for(&s, false);
    let rec = Arc::new(Messages::default());
    let call = Call::new(rec.clone(), CancellationToken::new());

    let out = install_packages::run(&state, Default::default(), &call)
        .await
        .unwrap();
    assert_eq!(out.installed, vec!["pkg"]);
    assert_eq!(out.packages.len(), 1);
    assert_eq!(out.packages[0].status, PackageStatus::Installed);
    let reports = rec.0.lock().unwrap().clone();
    assert_eq!(reports[0], (0.0, 100.0, Some("Installing pkg".into())));

    // Again: nothing to do, no download, no progress.
    let rec2 = Arc::new(Messages::default());
    let call = Call::new(rec2.clone(), CancellationToken::new());
    let input: install_packages::InstallPackagesInput =
        serde_json::from_value(serde_json::json!({"packages": ["pkg", "pkg"]})).unwrap();
    let out = install_packages::run(&state, input, &call).await.unwrap();
    assert!(out.installed.is_empty());
    assert!(rec2.0.lock().unwrap().is_empty());
    assert_eq!(s.server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn force_reinstalls_and_restores_damaged_files() {
    let s = setup(good_package(), None, None);
    let state = state_for(&s, true);
    let call = Call::new(Arc::new(NoProgress), CancellationToken::new());
    install_packages::run(&state, Default::default(), &call)
        .await
        .unwrap();

    // A tool deleted after the install: the record still says installed,
    // so a normal install does nothing.
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let tool = s.pm.package_dir(&spec).join("bin/tool");
    std::fs::remove_file(&tool).unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Installed);
    let out = install_packages::run(&state, Default::default(), &call)
        .await
        .unwrap();
    assert!(out.installed.is_empty());
    assert!(!tool.exists());

    // Force downloads and installs it again.
    let rec = Arc::new(Messages::default());
    let call = Call::new(rec.clone(), CancellationToken::new());
    let input: install_packages::InstallPackagesInput =
        serde_json::from_value(serde_json::json!({"force": true})).unwrap();
    let out = install_packages::run(&state, input, &call).await.unwrap();
    assert_eq!(out.installed, vec!["pkg"]);
    assert!(tool.exists());
    assert_eq!(s.pm.status(&spec), PackageStatus::Installed);
    assert_eq!(s.server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        rec.0.lock().unwrap()[0].2.as_deref(),
        Some("Reinstalling pkg")
    );
    assert_eq!(entries(&s.engine.join("packages/pkg")), vec!["2026-01-02"]);
}

#[tokio::test]
async fn a_failed_reinstall_keeps_the_installed_copy() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    s.pm.install(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();
    // The server now has no file: the reinstall fails.
    let pm = PackageManager::new(
        &s.engine,
        Arc::new(s.pm.manifest().clone()),
        PLATFORM,
        "http://127.0.0.1:9",
    );
    let err = pm
        .reinstall(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert_eq!(pm.status(&spec), PackageStatus::Installed);
    assert!(pm.package_dir(&spec).join("bin/tool").exists());
}

#[tokio::test]
async fn install_packages_tool_refuses_unknown_packages() {
    let s = setup(good_package(), None, None);
    let state = state_for(&s, true);
    let call = Call::new(Arc::new(NoProgress), CancellationToken::new());
    let input: install_packages::InstallPackagesInput =
        serde_json::from_value(serde_json::json!({"packages": ["nope"]})).unwrap();
    let err = install_packages::run(&state, input, &call)
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(
        err.message.contains("\"nope\"") && err.message.contains("pkg"),
        "{}",
        err.message
    );
    assert_eq!(s.server.requests.load(Ordering::SeqCst), 0);
}

/// Downloads and installs the real oss-cad-suite (about 526 MB) into a
/// temporary engine folder. Run: cargo test --test packages -- --ignored
#[tokio::test]
#[ignore]
async fn real_oss_cad_suite_install() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = tmp.path().canonicalize().unwrap().join("engine");
    std::fs::create_dir(&engine).unwrap();
    let pm = PackageManager::new(
        &engine,
        Arc::new(manifest::builtin().clone()),
        "darwin-arm64",
        manifest::GITHUB,
    );
    let missing = pm.needing_install(&["oss-cad-suite"]).unwrap();
    let phases = Phases::new(Arc::new(NoProgress), missing.len());
    pm.ensure(&missing, true, &phases, &CancellationToken::new())
        .await
        .unwrap();
    let spec = manifest::builtin().get("oss-cad-suite").unwrap();
    assert_eq!(pm.status(spec), PackageStatus::Installed);
    let bin = pm.package_dir(spec).join("bin");
    for tool in ["yosys", "nextpnr-ice40", "icepack", "openFPGALoader"] {
        assert!(bin.join(tool).exists(), "{tool}");
    }
    // The tools start straight from the package folder, with no extra
    // environment.
    for (tool, arg, expect) in [
        ("yosys", "-V", "Yosys"),
        ("nextpnr-ice40", "--version", "nextpnr"),
        ("openFPGALoader", "--Version", "openFPGALoader"),
    ] {
        let out = std::process::Command::new(bin.join(tool))
            .arg(arg)
            .env_clear()
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(text.contains(expect), "{tool} {arg}: {text}");
    }
}
