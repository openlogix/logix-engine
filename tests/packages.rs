//! Package download and repair, against a local HTTP server serving small
//! test packages. The real oss-cad-suite download is in the ignored test
//! `real_oss_cad_suite_download` (run: cargo test --test packages -- --ignored).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use logix_engine::errors::{ActionCode, Category};
use logix_engine::packages::manifest::{self, Manifest};
use logix_engine::packages::remote_config;
use logix_engine::packages::{PackageManager, RECORD_FILE};
use logix_engine::progress::{NoProgress, Phases, ProgressSink};
use logix_engine::state::EngineState;
use logix_engine::tools::get_engine_info::{PackageStatus, package_info};
use logix_engine::tools::{Call, download_packages};
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
    /// The files it serves, by path; tests may change them.
    files: Arc<Mutex<HashMap<String, Served>>>,
}

impl Server {
    fn put(&self, path: &str, body: &[u8]) {
        self.files.lock().unwrap().insert(
            path.to_string(),
            Served {
                body: body.to_vec(),
                stall_after: None,
            },
        );
    }

    fn remove(&self, path: &str) {
        self.files.lock().unwrap().remove(path);
    }

    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

fn serve(files: HashMap<String, Served>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let files = Arc::new(Mutex::new(files));
    let shared = files.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let files = shared.clone();
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || handle(stream, &files));
        }
    });
    Server {
        base,
        requests,
        files,
    }
}

fn handle(mut stream: TcpStream, files: &Mutex<HashMap<String, Served>>) {
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
    let file = files.lock().unwrap().get(&path).cloned();
    let Some(file) = file else {
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

/// A good package like flat, with a tool, a symlink and a big file
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
    p.start("Downloading pkg");
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
async fn downloads_a_missing_package() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    assert_eq!(s.pm.status(&spec), PackageStatus::Missing);
    assert!(package_info(&s.pm, &spec).components.is_empty());
    assert_eq!(s.pm.needing_download(&["pkg"]).await.unwrap().len(), 1);

    s.pm.download(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(s.pm.status(&spec), PackageStatus::Ready);
    assert!(s.pm.needing_download(&["pkg"]).await.unwrap().is_empty());
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
    assert_eq!(record["release"], "2026-01-02");
    assert_eq!(record["platform"], PLATFORM);
    assert_eq!(record["layout"], 1);
    assert!(
        record["downloaded_by"]
            .as_str()
            .unwrap()
            .starts_with("logix-engine ")
    );
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
    let spec = s.pm.needing_download(&["pkg"]).await.unwrap();
    s.pm.ensure(&spec, true, &phases, &CancellationToken::new())
        .await
        .unwrap();
    let values = rec.0.lock().unwrap().clone();
    assert!(values.len() >= 3, "{values:?}");
    assert!(values.windows(2).all(|w| w[1] > w[0]), "{values:?}");
    assert!(*values.last().unwrap() <= 99.0);
}

#[tokio::test]
async fn a_checksum_mismatch_is_a_setup_error_and_downloads_nothing() {
    let wrong = "0".repeat(64);
    let s = setup(good_package(), Some(&wrong), None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let err =
        s.pm.download(&spec, &phases(), &CancellationToken::new())
            .await
            .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert_eq!(err.action.unwrap().code, ActionCode::DownloadPackages);
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

    s.pm.download(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Ready);
    assert!(!dir.join("bin/leftover").exists());
    assert!(dir.join("bin/tool").exists());
}

#[tokio::test]
async fn a_record_from_another_release_is_broken() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    s.pm.download(&spec, &phases(), &CancellationToken::new())
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
async fn auto_download_off_is_a_setup_error_listing_the_packages() {
    let s = setup(good_package(), None, None);
    let missing = s.pm.needing_download(&["pkg"]).await.unwrap();
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
    assert_eq!(err.action.unwrap().code, ActionCode::DownloadPackages);
    assert!(
        err.message.contains("pkg 2026.01.02 (missing)"),
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
        .download(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert!(err.details.contains("404"), "{}", err.details);

    // No server at all.
    let pm = PackageManager::new(&s.engine, pm.manifest(), PLATFORM, "http://127.0.0.1:9");
    let err = pm
        .download(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert_eq!(err.action.unwrap().code, ActionCode::DownloadPackages);
}

#[tokio::test]
async fn cancelling_a_download_downloads_nothing() {
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
    let err = s.pm.download(&spec, &phases(), &cancel).await.unwrap_err();
    assert_eq!(err.category, Category::Cancelled);
    assert_eq!(s.pm.status(&spec), PackageStatus::Missing);
    assert!(entries(&s.engine.join("downloads")).is_empty());
}

#[tokio::test]
async fn concurrent_downloads_download_once() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let (a, b) = (s.pm.clone(), s.pm.clone());
    let (sa, sb) = (spec.clone(), spec.clone());
    let ta =
        tokio::spawn(async move { a.download(&sa, &phases(), &CancellationToken::new()).await });
    let tb =
        tokio::spawn(async move { b.download(&sb, &phases(), &CancellationToken::new()).await });
    ta.await.unwrap().unwrap();
    tb.await.unwrap().unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Ready);
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
            s.pm.download(&spec, &phases(), &CancellationToken::new())
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

// ----- The download_packages tool.

fn state_for(s: &Setup, auto_download: bool) -> EngineState {
    EngineState {
        engine_folder: s.engine.display().to_string(),
        engine_folder_resolved: s.engine.clone(),
        auto_download,
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
async fn download_packages_tool_downloads_once_even_with_auto_download_off() {
    let s = setup(good_package(), None, None);
    let state = state_for(&s, false);
    let rec = Arc::new(Messages::default());
    let call = Call::new(rec.clone(), CancellationToken::new());

    let out = download_packages::run(&state, Default::default(), &call)
        .await
        .unwrap();
    assert_eq!(out.downloaded, vec!["pkg"]);
    assert_eq!(out.packages.len(), 1);
    assert_eq!(out.packages[0].status, PackageStatus::Ready);
    let reports = rec.0.lock().unwrap().clone();
    assert_eq!(
        reports[0],
        (0.0, 100.0, Some("Downloading pkg 2026.01.02".into()))
    );

    // Again: nothing to do, no download, no progress.
    let rec2 = Arc::new(Messages::default());
    let call = Call::new(rec2.clone(), CancellationToken::new());
    let input: download_packages::DownloadPackagesInput =
        serde_json::from_value(serde_json::json!({"packages": ["pkg", "pkg"]})).unwrap();
    let out = download_packages::run(&state, input, &call).await.unwrap();
    assert!(out.downloaded.is_empty());
    assert!(rec2.0.lock().unwrap().is_empty());
    assert_eq!(s.server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn force_downloads_again_and_restores_damaged_files() {
    let s = setup(good_package(), None, None);
    let state = state_for(&s, true);
    let call = Call::new(Arc::new(NoProgress), CancellationToken::new());
    download_packages::run(&state, Default::default(), &call)
        .await
        .unwrap();

    // A tool deleted after the download: the record still says downloaded,
    // so a normal download does nothing.
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    let tool = s.pm.package_dir(&spec).join("bin/tool");
    std::fs::remove_file(&tool).unwrap();
    assert_eq!(s.pm.status(&spec), PackageStatus::Ready);
    let out = download_packages::run(&state, Default::default(), &call)
        .await
        .unwrap();
    assert!(out.downloaded.is_empty());
    assert!(!tool.exists());

    // Force downloads it again.
    let rec = Arc::new(Messages::default());
    let call = Call::new(rec.clone(), CancellationToken::new());
    let input: download_packages::DownloadPackagesInput =
        serde_json::from_value(serde_json::json!({"force": true})).unwrap();
    let out = download_packages::run(&state, input, &call).await.unwrap();
    assert_eq!(out.downloaded, vec!["pkg"]);
    assert!(tool.exists());
    assert_eq!(s.pm.status(&spec), PackageStatus::Ready);
    assert_eq!(s.server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        rec.0.lock().unwrap()[0].2.as_deref(),
        Some("Downloading again pkg")
    );
    assert_eq!(entries(&s.engine.join("packages/pkg")), vec!["2026-01-02"]);
}

#[tokio::test]
async fn a_failed_redownload_keeps_the_current_copy() {
    let s = setup(good_package(), None, None);
    let spec = s.pm.manifest().get("pkg").unwrap().clone();
    s.pm.download(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap();
    // The server now has no file: the second download fails.
    let pm = PackageManager::new(&s.engine, s.pm.manifest(), PLATFORM, "http://127.0.0.1:9");
    let err = pm
        .redownload(&spec, &phases(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup);
    // The current copy stays (its receipt names the first server's URL).
    assert_eq!(s.pm.status(&spec), PackageStatus::Ready);
    assert!(pm.package_dir(&spec).join("bin/tool").exists());
}

#[tokio::test]
async fn download_packages_tool_refuses_unknown_packages() {
    let s = setup(good_package(), None, None);
    let state = state_for(&s, true);
    let call = Call::new(Arc::new(NoProgress), CancellationToken::new());
    let input: download_packages::DownloadPackagesInput =
        serde_json::from_value(serde_json::json!({"packages": ["nope"]})).unwrap();
    let err = download_packages::run(&state, input, &call)
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

/// Downloads the real logix-core (about 640 MB) into a temporary engine
/// folder. Run: cargo test --test packages -- --ignored
#[tokio::test]
#[ignore]
async fn real_core_download() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = tmp.path().canonicalize().unwrap().join("engine");
    std::fs::create_dir(&engine).unwrap();
    let pm = PackageManager::with_remote_config(
        &engine,
        remote_config::repo_file_source(),
        true,
        "darwin-arm64",
        manifest::GITHUB,
    );
    let missing = pm.needing_download(&["logix-core"]).await.unwrap();
    let phases = Phases::new(Arc::new(NoProgress), missing.len());
    pm.ensure(&missing, true, &phases, &CancellationToken::new())
        .await
        .unwrap();
    let spec = &pm.spec("logix-core").unwrap();
    assert_eq!(pm.status(spec), PackageStatus::Ready);
    // get_engine_info reports where each component's programs are.
    let info = package_info(&pm, spec);
    let names: Vec<&str> = info.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["openxc7", "oss-cad-suite", "verible"]);
    let bin = pm.bin_dir("oss-cad-suite").unwrap();
    assert_eq!(bin, pm.package_dir(spec).join("oss-cad-suite/bin"));
    assert!(
        pm.bin_dir("verible")
            .unwrap()
            .join("verible-verilog-ls")
            .exists()
    );
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

// ----- The remote config: read, cached for 24 hours, switched, kept.

/// A test release: its file and its SHA256SUMS.
fn put_release(server: &Server, repo: &str, tag: &str, file: &str, data: &[u8]) {
    let dir = format!("/test/{repo}/releases/download/{tag}");
    server.put(&format!("{dir}/{file}"), data);
    server.put(
        &format!("{dir}/SHA256SUMS"),
        format!("{}  {file}\n", sha256(data)).as_bytes(),
    );
}

fn config_text(catalog: &str) -> String {
    format!(
        r#"// A test config.
{{
  "$schema": "https://www.openlogix.org/schemas/logix-remote-config-v1.json",
  /* The tools. */
  "logix-core": {{
    "repo": "test/core",
    "release": "2026-01-02",
    "file": "core-${{PLATFORM}}-${{YYYYMMDD}}.tgz"
  }},
  "logix-catalog": {{
    "repo": "test/catalog",
    "release": "{catalog}",
    "file": "catalog-${{YYYYMMDD}}.tgz"
  }}
}}
"#
    )
}

struct RemoteSetup {
    _tmp: tempfile::TempDir,
    engine: std::path::PathBuf,
    server: Server,
}

/// The channel these tests run on: a local build's `dev`, or the one CI's
/// pass sets in LOGIX_ENGINE_CHANNEL (once per config file in the repo).
fn test_channel() -> String {
    std::env::var(remote_config::CHANNEL_VAR).unwrap_or_else(|_| "dev".into())
}

/// The test server's config file for that channel, such as /001/dev.jsonc.
fn config_path() -> String {
    format!("/001/{}.jsonc", test_channel())
}

/// A server with core 2026-01-02, catalogs 2026-01-02 and 2026-01-03, and a
/// version folder at /001/ (channels.jsonc, and the test channel's config
/// file naming catalog 2026-01-02).
fn remote_setup() -> RemoteSetup {
    let tmp = tempfile::tempdir().unwrap();
    let engine = tmp.path().canonicalize().unwrap().join("engine");
    std::fs::create_dir(&engine).unwrap();
    let server = serve(HashMap::new());
    put_release(
        &server,
        "core",
        "2026-01-02",
        &format!("core-{PLATFORM}-20260102.tgz"),
        &good_package(),
    );
    put_release(
        &server,
        "catalog",
        "2026-01-02",
        "catalog-20260102.tgz",
        &good_package(),
    );
    put_release(
        &server,
        "catalog",
        "2026-01-03",
        "catalog-20260103.tgz",
        &good_package(),
    );
    server.put(&config_path(), config_text("2026-01-02").as_bytes());
    // The version folder's channel lists: nothing promoted, so the tests read
    // their channel's file.
    server.put(
        "/001/channels.jsonc",
        br#"{"$schema": "https://www.openlogix.org/schemas/logix-channels-v1.json"}"#,
    );
    RemoteSetup {
        _tmp: tmp,
        engine,
        server,
    }
}

impl RemoteSetup {
    fn pm(&self, network: bool, is_override: bool) -> PackageManager {
        PackageManager::with_remote_config(
            &self.engine,
            remote_config::ConfigSource {
                location: format!("{}/001/", self.server.base),
                is_override,
            },
            network,
            PLATFORM,
            &self.server.base,
        )
    }
}

const BOTH: [&str; 2] = ["logix-catalog", "logix-core"];

async fn download_all(pm: &PackageManager) -> Vec<String> {
    let needed = pm.needing_download(&BOTH).await.unwrap();
    pm.ensure(&needed, true, &phases(), &CancellationToken::new())
        .await
        .unwrap();
    needed
        .into_iter()
        .map(|s| format!("{} {}", s.name, s.version))
        .collect()
}

#[tokio::test]
async fn reads_the_remote_config_then_downloads_what_it_names() {
    let r = remote_setup();
    let pm = r.pm(true, false);
    assert!(
        pm.manifest().packages.is_empty(),
        "nothing before the first read"
    );
    assert_eq!(pm.remote_info().unwrap().fetched, None);
    assert_eq!(
        download_all(&pm).await,
        ["logix-catalog 2026-01-02", "logix-core 2026-01-02"]
    );
    for name in BOTH {
        assert_eq!(pm.status(&pm.spec(name).unwrap()), PackageStatus::Ready);
    }
    assert!(pm.remote_info().unwrap().fetched.is_some());
    let record: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            r.engine
                .join("packages/logix-core/2026-01-02")
                .join(RECORD_FILE),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["repo"], "test/core");
    assert_eq!(
        record["url"],
        format!(
            "{}/test/core/releases/download/2026-01-02/core-{PLATFORM}-20260102.tgz",
            r.server.base
        )
    );
    assert_eq!(record["size"], good_package().len());
    // channels.jsonc, the config, the two SHA256SUMS and the two packages.
    assert_eq!(r.server.count(), 6);
    let channel = pm.remote_info().unwrap().channel.unwrap();
    // A local build is on dev; CI's passes set LOGIX_ENGINE_CHANNEL.
    assert_eq!(
        (channel.name.clone(), channel.file.clone()),
        (test_channel(), format!("{}.jsonc", test_channel()))
    );
    assert!(
        ["local build", remote_config::CHANNEL_VAR].contains(&channel.how.as_str()),
        "{}",
        channel.how
    );
}

#[tokio::test]
async fn without_its_own_file_dev_reads_the_prod_config() {
    if test_channel() == "prod" {
        return; // Only another channel falls back to prod.jsonc.
    }
    let r = remote_setup();
    r.server.remove(&config_path());
    r.server
        .put("/001/prod.jsonc", config_text("2026-01-02").as_bytes());
    let pm = r.pm(true, false);
    assert_eq!(download_all(&pm).await.len(), 2);
    let channel = pm.remote_info().unwrap().channel.unwrap();
    assert_eq!(
        (channel.name.clone(), channel.file.as_str()),
        (test_channel(), "prod.jsonc")
    );
}

#[tokio::test]
async fn the_cached_config_is_used_for_24_hours() {
    let r = remote_setup();
    download_all(&r.pm(true, false)).await;
    let before = r.server.count();
    // A new engine process with the same engine folder.
    let pm = r.pm(true, false);
    assert!(pm.remote_info().unwrap().fetched.is_some());
    assert!(pm.needing_download(&BOTH).await.unwrap().is_empty());
    assert_eq!(r.server.count(), before, "no network use");
}

#[tokio::test]
async fn a_new_config_switches_releases_and_deletes_the_old_ones() {
    let r = remote_setup();
    let pm = r.pm(true, false);
    download_all(&pm).await;
    r.server
        .put(&config_path(), config_text("2026-01-03").as_bytes());
    let updates = pm.refresh(true, false).await.unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].name, "logix-catalog");
    assert_eq!(updates[0].from.as_deref(), Some("2026.01.02"));
    assert_eq!(updates[0].to, "2026.01.03");
    assert_eq!(download_all(&pm).await, ["logix-catalog 2026-01-03"]);
    assert_eq!(
        entries(&r.engine.join("packages/logix-catalog")),
        vec!["2026-01-03"]
    );
    assert_eq!(
        entries(&r.engine.join("packages/logix-core")),
        vec!["2026-01-02"]
    );
    // Nothing changed since: no updates.
    assert!(pm.refresh(true, false).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_package_rebuilt_under_the_same_tag_is_downloaded_again() {
    let r = remote_setup();
    let pm = r.pm(true, false);
    download_all(&pm).await;
    let rebuilt = tgz(|b| add_file(b, "./rebuilt.txt", b"a rebuilt package", 0o644));
    put_release(
        &r.server,
        "catalog",
        "2026-01-02",
        "catalog-20260102.tgz",
        &rebuilt,
    );
    pm.refresh(true, false).await.unwrap();
    assert_eq!(download_all(&pm).await, ["logix-catalog 2026-01-02"]);
}

#[tokio::test]
async fn the_config_in_use_stays_when_a_new_one_cant_be_read() {
    let r = remote_setup();
    let pm = r.pm(true, false);
    download_all(&pm).await;
    r.server.remove(&config_path());
    assert!(pm.refresh(true, false).await.unwrap().is_empty());
    assert!(pm.needing_download(&BOTH).await.unwrap().is_empty());
    // A bad config is refused the same way.
    r.server.put(&config_path(), b"{ \"nope\": 1 }");
    assert!(pm.refresh(true, false).await.unwrap().is_empty());
    assert_eq!(pm.spec("logix-catalog").unwrap().version, "2026-01-02");
}

#[tokio::test]
async fn without_a_config_to_use_it_is_a_setup_error() {
    let r = remote_setup();
    r.server.remove(&config_path());
    let err = r.pm(true, false).needing_download(&BOTH).await.unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert!(
        err.message
            .contains("Can't read the engine's remote config"),
        "{}",
        err.message
    );
    let missing = match test_channel().as_str() {
        "prod" => "there's no prod.jsonc".to_string(),
        c => format!("there's neither {c}.jsonc nor prod.jsonc"),
    };
    assert!(err.message.contains(&missing), "{}", err.message);
}

#[tokio::test]
async fn with_automatic_download_off_only_a_request_reads_the_config() {
    let r = remote_setup();
    let pm = r.pm(false, false);
    let err = pm.needing_download(&BOTH).await.unwrap_err();
    assert!(
        err.message.contains("automatic download is turned off"),
        "{}",
        err.message
    );
    assert_eq!(r.server.count(), 0);
    pm.refresh(false, true).await.unwrap();
    assert_eq!(pm.needing_download(&BOTH).await.unwrap().len(), 2);
}

#[tokio::test]
async fn the_override_never_falls_back() {
    let r = remote_setup();
    let pm = r.pm(true, true);
    download_all(&pm).await;
    r.server.remove(&config_path());
    let err = pm.refresh(true, false).await.unwrap_err();
    assert!(
        err.message.contains(remote_config::DEV_REMOTE_CONFIG_VAR),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn a_package_the_config_drops_is_deleted() {
    let r = remote_setup();
    let pm = r.pm(true, false);
    download_all(&pm).await;
    std::fs::create_dir_all(r.engine.join("packages/logix-old/2025-01-01")).unwrap();
    pm.refresh(true, false).await.unwrap();
    assert_eq!(
        entries(&r.engine.join("packages")),
        vec!["logix-catalog", "logix-core"]
    );
}
