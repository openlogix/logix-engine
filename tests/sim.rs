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
use logix_engine::tools::{Call, lint, test};
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

fn test_input(src: &Path, extra: Value) -> test::TestInput {
    let v = json!({
        "board": "alhambra-ii",
        "source_folder": src,
        "source_files": ["main.v", "rtl/counter.v"],
        "testbench_files": ["main_tb.v"],
        "build_folder": src.join("_build/default"),
    });
    serde_json::from_value(with(v, extra)).expect("a valid test input")
}

/// A testbench in a subfolder that reads a data file relative to the source
/// folder, checks APIO_SIM, and fails with $fatal when `fail` is set.
fn sub_tb(fail: bool) -> String {
    let expected = if fail { "8'h99" } else { "8'h5a" };
    format!(
        r#"module sub_tb;
  reg [7:0] mem [0:0];
  initial begin
    $readmemh("data/mem.hex", mem);
`ifndef APIO_SIM
    $fatal(1, "APIO_SIM isn't defined");
`endif
    if (`APIO_SIM != 0) $fatal(1, "APIO_SIM isn't 0");
    $display("mem is %h", mem[0]);
    if (mem[0] !== {expected}) $fatal(1, "mem is %h", mem[0]);
    $finish;
  end
endmodule
"#
    )
}

#[tokio::test]
async fn test_runs_a_passing_testbench_and_writes_its_waveform() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(out.passed, "{out:#?}");
    let tb = &out.testbenches[0];
    assert_eq!(tb.file, "main_tb.v");
    assert!(tb.passed);
    assert!(tb.output.contains("main_tb passed"), "{}", tb.output);
    assert!(!tb.output.contains("VCD info"), "{}", tb.output);
    assert_eq!(tb.waveform_file.as_deref(), Some("main_tb.vcd"));
    let build = src.join("_build/default");
    assert!(build.join("main_tb.vcd").is_file());
    assert!(build.join("main_tb.out").is_file());
    assert!(build.join(".logix-build").is_file());
}

#[tokio::test]
async fn test_runs_every_testbench_and_reports_failures_as_results() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::create_dir_all(src.join("tb")).unwrap();
    std::fs::create_dir_all(src.join("data")).unwrap();
    std::fs::write(src.join("data/mem.hex"), "5a\n").unwrap();
    std::fs::write(src.join("tb/sub_tb.v"), sub_tb(true)).unwrap();
    std::fs::write(
        src.join("tb/ok_tb.v"),
        sub_tb(false).replace("sub_tb", "ok_tb"),
    )
    .unwrap();
    let input = test_input(
        &src,
        json!({"testbench_files": ["tb/sub_tb.v", "main_tb.v", "tb/ok_tb.v"]}),
    );
    let out = test::run(&state, input, &call()).await.unwrap();
    assert!(!out.passed);
    let passed: Vec<_> = out.testbenches.iter().map(|t| t.passed).collect();
    assert_eq!(passed, [false, true, true], "{out:#?}");
    let failed = &out.testbenches[0];
    assert!(failed.output.contains("mem is 5a"), "{}", failed.output);
    let d = &failed.diagnostics[0];
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(d.file.as_deref(), Some("tb/sub_tb.v"));
    assert_eq!(d.line, Some(10));
    assert_eq!(d.text, "mem is 5a");
    // No $dumpvars, so no waveform.
    assert_eq!(failed.waveform_file, None);
    assert!(src.join("_build/default/tb/sub_tb.out").is_file());
}

#[tokio::test]
async fn test_compile_errors_are_design_errors() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("rtl/counter.v"),
        COUNTER.replace("cnt + 1'b1;", "cnt +;"),
    )
    .unwrap();
    let err = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design, "{err:#?}");
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("rtl/counter.v"));
    assert_eq!(err.diagnostics[0].line, Some(4));
    assert_eq!(err.diagnostics[0].tool.as_deref(), Some("iverilog"));
}

#[tokio::test]
async fn test_refuses_dumpfile_and_bad_requests() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main_tb.v"),
        MAIN_TB.replace(
            "$dumpvars(0, main_tb);",
            "$dumpfile(\"x.vcd\");\n    $dumpvars(0, main_tb);",
        ),
    )
    .unwrap();
    let err = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert_eq!(err.diagnostics[0].line, Some(8));

    for (extra, needle) in [
        (json!({"testbench_files": []}), "testbench_files is empty"),
        (json!({"build_folder": src.join("out")}), "build_folder"),
    ] {
        let err = test::run(&state, test_input(&src, extra.clone()), &call())
            .await
            .unwrap_err();
        assert_eq!(err.category, Category::Request, "{extra}");
        assert!(err.message.contains(needle), "{extra}: {}", err.message);
    }
}

#[tokio::test]
async fn test_can_be_cancelled() {
    let state = engine().await;
    let (_tmp, src) = project();
    // A testbench that never calls $finish.
    std::fs::write(src.join("main_tb.v"), MAIN_TB.replace("$finish;", "")).unwrap();
    let c = call();
    let cancel = c.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        cancel.cancel();
    });
    let start = std::time::Instant::now();
    let err = test::run(&state, test_input(&src, json!({})), &c)
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Cancelled);
    assert!(
        err.message.contains("test was cancelled"),
        "{}",
        err.message
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
}
