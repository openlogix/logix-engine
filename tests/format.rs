//! format with the real Verible package. It uses the same engine folder as
//! tests/build.rs, target/tmp/test-engine, so the package is downloaded once
//! (8 MB) and reused.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use logix_engine::errors::Severity;
use logix_engine::format::FileStatus;
use logix_engine::packages::PackageManager;
use logix_engine::packages::manifest;
use logix_engine::progress::{NoProgress, OutputLevel, OutputSink};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, format};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn engine() -> EngineState {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let packages = PackageManager::new(
        &folder,
        Arc::new(manifest::builtin().clone()),
        &state::platform(),
        manifest::GITHUB,
    );
    EngineState {
        engine_folder: folder.display().to_string(),
        engine_folder_resolved: folder.clone(),
        auto_install: true,
        command_line: vec![],
        log_file: folder.join("logs/test.log"),
        packages,
    }
}

/// Live output, kept as (kind, tool, text).
#[derive(Default)]
struct Output(Mutex<Vec<(String, String, String)>>);

impl OutputSink for Output {
    fn line(&self, tool: &str, _level: OutputLevel, text: &str) {
        let line = ("output".into(), tool.into(), text.into());
        self.0.lock().unwrap().push(line);
    }
    fn command(&self, tool: &str, text: &str) {
        let line = ("command".into(), tool.into(), text.into());
        self.0.lock().unwrap().push(line);
    }
}

const MESSY: &str = "module  main(input a,output   b);\nassign b=a;\nendmodule\n";
const TIDY: &str = "module main (\n    input  a,\n    output b\n);\n  assign b = a;\nendmodule\n";
const BROKEN: &str = "module main;\n  wire = ;\n  initial begin\n end end\nendmodule\n";

fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap();
    std::fs::create_dir_all(src.join("rtl")).unwrap();
    std::fs::write(src.join("main.v"), MESSY).unwrap();
    std::fs::write(src.join("rtl/tidy.v"), TIDY).unwrap();
    std::fs::write(src.join("broken.v"), BROKEN).unwrap();
    (tmp, src)
}

async fn run(src: &Path, extra: serde_json::Value, output: Arc<Output>) -> format::FormatOutput {
    let mut v = json!({ "source_folder": src });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    let mut call = Call::new(Arc::new(NoProgress), CancellationToken::new());
    call.output = output;
    format::run(&engine(), serde_json::from_value(v).unwrap(), &call)
        .await
        .unwrap()
}

#[tokio::test]
async fn formats_each_file_and_reports_syntax_errors() {
    let (_tmp, src) = project();
    let output = Arc::new(Output::default());
    let out = run(
        &src,
        json!({"source_files": ["main.v", "rtl/tidy.v", "broken.v"]}),
        output.clone(),
    )
    .await;

    let statuses: Vec<(&str, FileStatus)> = out
        .files
        .iter()
        .map(|f| (f.file.as_str(), f.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("main.v", FileStatus::Formatted),
            ("rtl/tidy.v", FileStatus::Unchanged),
            ("broken.v", FileStatus::Failed),
        ]
    );
    assert!(!out.passed);
    assert_eq!(std::fs::read_to_string(src.join("main.v")).unwrap(), TIDY);
    assert_eq!(
        std::fs::read_to_string(src.join("rtl/tidy.v")).unwrap(),
        TIDY
    );
    assert_eq!(
        std::fs::read_to_string(src.join("broken.v")).unwrap(),
        BROKEN
    );

    let lines: Vec<(Option<&str>, Option<u32>)> = out
        .diagnostics
        .iter()
        .map(|d| (d.file.as_deref(), d.line))
        .collect();
    assert_eq!(
        lines,
        [(Some("broken.v"), Some(2)), (Some("broken.v"), Some(4))]
    );
    assert!(
        out.diagnostics
            .iter()
            .all(|d| d.severity == Severity::Error)
    );

    assert_eq!(out.commands.len(), 3);
    assert!(
        out.commands[0]
            .shell
            .contains("verible-verilog-format --nofailsafe_success --inplace main.v"),
        "{}",
        out.commands[0].shell
    );
    let output = output.0.lock().unwrap();
    assert!(
        output
            .iter()
            .any(|(k, t, _)| k == "command" && t == "verible-verilog-format")
    );
}

#[tokio::test]
async fn options_change_the_style() {
    let (_tmp, src) = project();
    let out = run(
        &src,
        json!({"source_files": ["main.v"], "verible_options": ["--indentation_spaces=4"]}),
        Arc::default(),
    )
    .await;
    assert!(out.passed);
    let text = std::fs::read_to_string(src.join("main.v")).unwrap();
    assert!(text.contains("\n    assign b = a;\n"), "{text}");
}

#[tokio::test]
async fn a_linked_file_is_formatted_through_the_link() {
    let (_tmp, src) = project();
    std::os::unix::fs::symlink("main.v", src.join("link.v")).unwrap();
    let out = run(&src, json!({"source_files": ["link.v"]}), Arc::default()).await;
    assert_eq!(out.files[0].status, FileStatus::Formatted);
    assert!(
        std::fs::symlink_metadata(src.join("link.v"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(src.join("main.v")).unwrap(), TIDY);
}
