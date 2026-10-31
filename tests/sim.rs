//! lint, test and simulate with the real toolchain. They use the same engine
//! folder as tests/build.rs, target/tmp/test-engine, so oss-cad-suite is
//! downloaded once and reused.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use logix_engine::errors::{Category, Severity};
use logix_engine::packages::PackageManager;
use logix_engine::packages::manifest;
use logix_engine::progress::{NoProgress, OutputLevel, OutputSink, Phases};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, lint};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

fn engine_folder() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine")
}

/// An engine state with the toolchain installed.
async fn engine() -> EngineState {
    static READY: OnceCell<()> = OnceCell::const_new();
    let folder = engine_folder();
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let packages = PackageManager::new(
        &folder,
        Arc::new(manifest::builtin().clone()),
        &state::platform(),
        manifest::GITHUB,
    );
    READY
        .get_or_init(|| async {
            let missing = packages.needing_install(&["oss-cad-suite"]).unwrap();
            let phases = Phases::new(Arc::new(NoProgress), missing.len());
            packages
                .ensure(&missing, true, &phases, &CancellationToken::new())
                .await
                .expect("install oss-cad-suite for the simulation tests");
        })
        .await;
    EngineState {
        engine_folder: folder.display().to_string(),
        engine_folder_resolved: folder.clone(),
        auto_install: true,
        command_line: vec![],
        log_file: folder.join("logs/test.log"),
        packages,
    }
}

fn call() -> Call {
    Call::new(Arc::new(NoProgress), CancellationToken::new())
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

/// A counter that uses an iCE40 primitive (SB_LUT4) and an include from a
/// subfolder, which only the engine's -I arguments find.
const COUNTER: &str = r#"`include "width.vh"
module counter(input clk, output [`WIDTH-1:0] q);
  reg [`WIDTH-1:0] cnt = 0;
  always @(posedge clk) cnt <= cnt + 1'b1;
  assign q = cnt;
endmodule
"#;

const MAIN: &str = r#"module main(input CLK, output LED0);
  wire [3:0] q;
  counter c(.clk(CLK), .q(q));
  SB_LUT4 #(.LUT_INIT(16'h8000)) all_ones (.O(LED0), .I0(q[0]), .I1(q[1]), .I2(q[2]), .I3(q[3]));
endmodule
"#;

const MAIN_TB: &str = r#"`timescale 1ns/1ps
module main_tb;
  reg clk = 0;
  wire [3:0] q;
  counter dut(.clk(clk), .q(q));
  always #5 clk = ~clk;
  initial begin
    $dumpvars(0, main_tb);
    #95;
    if (q !== 4'd9) $fatal(1, "q is %d, expected 9", q);
    $display("main_tb passed");
    $finish;
  end
endmodule
"#;

/// A project with a counter design and a passing testbench.
fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap().join("proj");
    std::fs::create_dir_all(src.join("rtl")).unwrap();
    std::fs::write(src.join("main.v"), MAIN).unwrap();
    std::fs::write(src.join("rtl/counter.v"), COUNTER).unwrap();
    std::fs::write(src.join("rtl/width.vh"), "`define WIDTH 4\n").unwrap();
    std::fs::write(src.join("main_tb.v"), MAIN_TB).unwrap();
    (tmp, src)
}

fn with(mut v: Value, extra: Value) -> Value {
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    v
}

fn lint_input(src: &Path, extra: Value) -> lint::LintInput {
    let v = json!({
        "board": "alhambra-ii",
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v", "rtl/counter.v"],
        "testbench_files": ["main_tb.v"],
    });
    serde_json::from_value(with(v, extra)).expect("a valid lint input")
}

#[tokio::test]
async fn lint_passes_a_clean_design_with_fpga_primitives_and_includes() {
    let state = engine().await;
    let (_tmp, src) = project();
    let output = Arc::new(Output::default());
    let mut c = call();
    c.output = output.clone();
    let out = lint::run(&state, lint_input(&src, json!({})), &c)
        .await
        .unwrap();
    assert!(out.passed, "{:#?}", out.diagnostics);
    assert!(out.diagnostics.is_empty());
    let lines = output.0.lock().unwrap();
    assert_eq!(lines[0].0, "command");
    assert!(
        lines[0].2.starts_with("verilator_bin --lint-only"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn lint_reports_warnings_and_errors_as_its_result() {
    let state = engine().await;
    let (_tmp, src) = project();

    // A width mismatch: a warning, which fails the lint as in Apio.
    std::fs::write(src.join("main.v"), MAIN.replace(".I3(q[3])", ".I3(q)")).unwrap();
    let out = lint::run(&state, lint_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.passed);
    let d = &out.diagnostics[0];
    assert_eq!(d.severity, Severity::Warning, "{d:?}");
    assert_eq!(d.file.as_deref(), Some("main.v"));
    assert_eq!(d.line, Some(4));
    assert!(d.text.ends_with("(WIDTHTRUNC)"), "{d:?}");

    // A syntax error in a testbench, which is linted too.
    std::fs::write(src.join("main.v"), MAIN).unwrap();
    std::fs::write(
        src.join("main_tb.v"),
        "module main_tb;\n  wire x\nendmodule\n",
    )
    .unwrap();
    let out = lint::run(&state, lint_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.passed);
    let errors: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors[0].file.as_deref(), Some("main_tb.v"), "{errors:?}");
    assert_eq!(errors[0].line, Some(3));
    assert_eq!(errors[0].tool.as_deref(), Some("verilator"));

    // Without the testbench, the design alone passes.
    let out = lint::run(
        &state,
        lint_input(&src, json!({"testbench_files": []})),
        &call(),
    )
    .await
    .unwrap();
    assert!(out.passed, "{:#?}", out.diagnostics);
}

#[tokio::test]
async fn lint_defines_reach_the_sources() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main.v"),
        format!("`ifdef BREAK_IT\n  syntax error here\n`endif\n`ifndef SYNTHESIZE\n  also broken\n`endif\n{MAIN}"),
    )
    .unwrap();
    let out = lint::run(&state, lint_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(out.passed, "SYNTHESIZE is defined: {:#?}", out.diagnostics);
    let out = lint::run(
        &state,
        lint_input(&src, json!({"defines": {"BREAK_IT": ""}})),
        &call(),
    )
    .await
    .unwrap();
    assert!(!out.passed);
    assert_eq!(out.diagnostics[0].line, Some(2), "{:#?}", out.diagnostics);
}

#[tokio::test]
async fn lint_request_errors() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(src.parent().unwrap().join("outside.vh"), "").unwrap();
    std::fs::write(
        src.join("main.v"),
        format!("`include \"../outside.vh\"\n{MAIN}"),
    )
    .unwrap();
    let err = lint::run(&state, lint_input(&src, json!({})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(
        err.message.contains("outside the source folder"),
        "{}",
        err.message
    );
}
