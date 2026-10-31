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
use logix_engine::sim::testbench::Ending;
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, lint, simulate, test};
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
async fn lint_without_a_top_module_ignores_warnings_in_the_fpga_models() {
    // Without a top module, Verilator checks every unused primitive in the
    // models too, which has warnings that aren't the user's.
    let state = engine().await;
    let (_tmp, src) = project();
    for synthesize in [true, false] {
        let input = lint_input(&src, json!({"top_module": null, "synthesize": synthesize}));
        let out = lint::run(&state, input, &call()).await.unwrap();
        assert!(out.passed, "{:#?}", out.diagnostics);
    }
    // The user's own warnings still count.
    std::fs::write(src.join("main.v"), MAIN.replace(".I3(q[3])", ".I3(q)")).unwrap();
    let input = lint_input(&src, json!({"top_module": null}));
    let out = lint::run(&state, input, &call()).await.unwrap();
    assert!(!out.passed);
    assert!(
        out.diagnostics
            .iter()
            .all(|d| d.file.as_deref() == Some("main.v")),
        "{:#?}",
        out.diagnostics
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
    // synthesize false lints the simulation side, like apio lint --nosynth.
    let out = lint::run(
        &state,
        lint_input(&src, json!({"synthesize": false})),
        &call(),
    )
    .await
    .unwrap();
    assert!(!out.passed);
    // Verilator reports the broken line 5 at the next token, on line 7.
    assert_eq!(out.diagnostics[0].line, Some(7), "{:#?}", out.diagnostics);
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

fn simulate_input(src: &Path, extra: Value) -> simulate::SimulateInput {
    let v = json!({
        "board": "alhambra-ii",
        "source_folder": src,
        "source_files": ["main.v", "rtl/counter.v"],
        "testbench_file": "main_tb.v",
        "build_folder": src.join("_build/default"),
        "layout_file": "main_tb.gtkw",
        "open_viewer": false,
    });
    serde_json::from_value(with(v, extra)).expect("a valid simulate input")
}

#[tokio::test]
async fn simulate_writes_the_waveform_and_a_default_layout_then_keeps_the_users() {
    let state = engine().await;
    let (_tmp, src) = project();
    // APIO_SIM is 1 when simulating for a viewer.
    std::fs::write(
        src.join("main_tb.v"),
        MAIN_TB.replace(
            "$finish;",
            "if (`APIO_SIM != 1) $fatal(1, \"APIO_SIM\");\n    $finish;",
        ),
    )
    .unwrap();
    let out = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(out.passed, "{out:#?}");
    assert_eq!(out.waveform_file, "main_tb.vcd");
    assert!(src.join("_build/default/main_tb.vcd").is_file());
    assert_eq!(out.layout_file, src.join("main_tb.gtkw").to_string_lossy());
    assert!(out.layout_created);
    assert!(!out.viewer_opened);
    let layout = std::fs::read_to_string(src.join("main_tb.gtkw")).unwrap();
    assert!(layout.contains("GENERATED AUTOMATICALLY BY LOGIX ENGINE"));
    let signals: Vec<_> = layout
        .lines()
        .filter(|l| !l.starts_with('*') && !l.starts_with('['))
        .collect();
    assert_eq!(signals, ["main_tb.clk", "main_tb.q[3:0]"]);
    // A passing run: a zoom that fits it, and no marker.
    assert!(
        layout
            .lines()
            .any(|l| l.starts_with("*-") && l.ends_with(" -1")),
        "{layout}"
    );

    // A layout the user saved from GTKWave is kept.
    std::fs::write(src.join("main_tb.gtkw"), "[*] mine\nmain_tb.q[3:0]\n").unwrap();
    let out = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.layout_created);
    assert_eq!(
        std::fs::read_to_string(src.join("main_tb.gtkw")).unwrap(),
        "[*] mine\nmain_tb.q[3:0]\n"
    );

    // Without a layout file, the default goes into the build folder.
    let input = simulate_input(&src, json!({"layout_file": null}));
    let out = simulate::run(&state, input, &call()).await.unwrap();
    let in_build = src.join("_build/default/main_tb.gtkw");
    assert_eq!(out.layout_file, in_build.to_string_lossy());
    assert!(in_build.is_file());
}

#[tokio::test]
async fn simulate_shows_a_failing_testbench_but_needs_a_waveform() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(src.join("main_tb.v"), MAIN_TB.replace("4'd9", "4'd3")).unwrap();
    let out = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.passed);
    // The comparison changed, not the message.
    assert_eq!(out.diagnostics[0].text, "q is  9, expected 9");
    assert_eq!(out.diagnostics[0].line, Some(10));
    assert!(src.join("_build/default/main_tb.vcd").is_file());

    std::fs::write(
        src.join("main_tb.v"),
        MAIN_TB.replace("$dumpvars(0, main_tb);", ""),
    )
    .unwrap();
    let err = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert!(err.message.contains("$dumpvars"), "{}", err.message);
}

#[tokio::test]
async fn simulate_request_errors() {
    let state = engine().await;
    let (_tmp, src) = project();
    for (extra, needle) in [
        (json!({"testbench_file": "nope_tb.v"}), "testbench_file"),
        (json!({"testbench_file": "main.v"}), "listed more than once"),
        (json!({"layout_file": "main_tb.txt"}), "must end with .gtkw"),
        (
            json!({"layout_file": "missing/main_tb.gtkw"}),
            "layout_file",
        ),
    ] {
        let err = simulate::run(&state, simulate_input(&src, extra.clone()), &call())
            .await
            .unwrap_err();
        assert_eq!(err.category, Category::Request, "{extra}");
        assert!(err.message.contains(needle), "{extra}: {}", err.message);
    }
}

/// A testbench with Apio's assertion macro: it prints the failure, and calls
/// $fatal only when APIO_SIM is 0, so simulate keeps going.
const APIO_MACRO_TB: &str = r#"`define EXPECT(signal, value) \
    if (signal !== (value)) begin \
        $display("*** ASSERTION FAILED in %m (clk_num=%0d): expected (signal == value), actual: 'h%h", 0, (signal)); \
        if (!`APIO_SIM) $fatal; \
    end
`timescale 1ns/1ps
module main_tb;
  reg clk = 0;
  wire [3:0] q;
  counter dut(.clk(clk), .q(q));
  always #5 clk = ~clk;
  initial begin
    $dumpvars(0, main_tb);
    #95;
    `EXPECT(q, 4'd3)
    $display("End of simulation");
    $finish;
  end
endmodule
"#;

#[tokio::test]
async fn apio_assertion_macros_fail_both_test_and_simulate() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(src.join("main_tb.v"), APIO_MACRO_TB).unwrap();

    // simulate: the macro skips $fatal, so the run ends cleanly, but the
    // printed failure still fails it.
    let out = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.passed, "{out:#?}");
    assert!(out.output.contains("End of simulation"));
    assert!(
        out.diagnostics[0]
            .text
            .starts_with("Assertion failed in main_tb (clk_num=0): expected (q == 4'd3)"),
        "{:#?}",
        out.diagnostics
    );

    // test: the macro calls $fatal, so the run stops at the failure.
    let out = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.passed);
    let tb = &out.testbenches[0];
    assert!(!tb.output.contains("End of simulation"), "{}", tb.output);
    assert!(
        tb.diagnostics
            .iter()
            .any(|d| d.text.starts_with("Assertion failed"))
    );
}

/// A testbench that fails at 10 ns, then keeps busy for a few seconds and
/// fails again: the guidelines' style, with $error and no mode checks.
const SLOW_FAILING_TB: &str = r#"`timescale 1ns/1ps
module main_tb;
  integer i;
  reg [31:0] x = 0;
  initial begin
    $dumpvars(0, main_tb);
    #10 $error("first failure");
    for (i = 0; i < 10000000; i = i + 1) x = x + i;
    $display("still running");
    #5 $error("second failure");
    $finish;
  end
endmodule
"#;

#[tokio::test]
async fn test_stops_at_the_first_failure_unless_asked_not_to() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(src.join("main_tb.v"), SLOW_FAILING_TB).unwrap();

    let start = std::time::Instant::now();
    let out = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap();
    let fast = start.elapsed();
    let tb = &out.testbenches[0];
    assert!(!tb.passed);
    assert_eq!(tb.ending, Ending::FirstFailure);
    assert!(!tb.output.contains("still running"), "{}", tb.output);
    assert_eq!(tb.diagnostics.len(), 1, "{:#?}", tb.diagnostics);
    assert_eq!(tb.diagnostics[0].text, "first failure");
    assert_eq!(tb.diagnostics[0].line, Some(7));
    assert_eq!(tb.diagnostics[0].time_ns, Some(10.0));

    let input = test_input(&src, json!({"stop_at_first_failure": false}));
    let start = std::time::Instant::now();
    let out = test::run(&state, input, &call()).await.unwrap();
    let full = start.elapsed();
    let tb = &out.testbenches[0];
    assert!(!tb.passed);
    assert_eq!(tb.ending, Ending::Finished);
    assert!(tb.output.contains("still running"), "{}", tb.output);
    let times: Vec<_> = tb.diagnostics.iter().map(|d| d.time_ns).collect();
    assert_eq!(times, [Some(10.0), Some(15.0)]);
    assert!(
        fast < full,
        "stopping early is faster: {fast:?} vs {full:?}"
    );
}

#[tokio::test]
async fn simulate_runs_to_the_end_and_marks_the_first_failure() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main_tb.v"),
        MAIN_TB.replace("$fatal(1,", "$error("),
    )
    .unwrap();
    std::fs::write(
        src.join("main_tb.v"),
        MAIN_TB.replace(
            "if (q !== 4'd9) $fatal(1, \"q is %d, expected 9\", q);",
            "if (q !== 4'd3) $error(\"q is %0d, expected 3\", q);",
        ),
    )
    .unwrap();
    let out = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(!out.passed);
    assert!(
        out.output.contains("main_tb passed"),
        "ran to the end: {}",
        out.output
    );
    assert_eq!(out.diagnostics[0].text, "q is 9, expected 3");
    assert_eq!(out.diagnostics[0].time_ns, Some(95.0));
    let layout = std::fs::read_to_string(src.join("main_tb.gtkw")).unwrap();
    // A zoom that fits the run, and the marker at the failure (95 ns in ps).
    let star = layout
        .lines()
        .find(|l| l.starts_with('*') && !l.starts_with("[*]"))
        .unwrap();
    assert!(
        star.starts_with("*-") && star.ends_with(" 95000"),
        "{layout}"
    );
    assert!(layout.contains("[timestart] "), "{layout}");
}

#[tokio::test]
async fn repeated_failures_are_each_kept_up_to_a_limit_and_counted() {
    let state = engine().await;
    let (_tmp, src) = project();
    // The same check fails 150 times, 1 ns apart.
    std::fs::write(
        src.join("main_tb.v"),
        r#"`timescale 1ns/1ps
module main_tb;
  integer i;
  initial begin
    $dumpvars(0, main_tb);
    for (i = 0; i < 150; i = i + 1) #1 $error("same failure");
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let input = test_input(&src, json!({"stop_at_first_failure": false}));
    let out = test::run(&state, input, &call()).await.unwrap();
    let tb = &out.testbenches[0];
    assert_eq!(tb.failure_count, 150);
    assert_eq!(tb.diagnostics.len(), 100, "the first 100 are listed");
    assert_eq!(tb.diagnostics[0].time_ns, Some(1.0));
    assert_eq!(tb.diagnostics[99].time_ns, Some(100.0));

    let out = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert_eq!(out.testbenches[0].failure_count, 1);

    let out = simulate::run(&state, simulate_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert_eq!(out.failure_count, 150);
    assert_eq!(out.diagnostics.len(), 100);
}

#[tokio::test]
async fn runs_that_dont_end_are_stopped_by_the_limits() {
    let state = engine().await;
    let (_tmp, src) = project();
    // A clock forever, with no `timescale and no $finish.
    let forever = "module main_tb;\n  reg clk = 0;\n  initial $dumpvars(0, main_tb);\n  always #5 clk = ~clk;\nendmodule\n";
    std::fs::write(src.join("main_tb.v"), forever).unwrap();

    // Simulated time limit.
    let input = test_input(&src, json!({"max_sim_time_ns": 250}));
    let out = test::run(&state, input, &call()).await.unwrap();
    let tb = &out.testbenches[0];
    assert_eq!(tb.ending, Ending::SimTimeLimit, "{tb:#?}");
    assert!(!tb.passed);
    assert!(
        tb.diagnostics.is_empty(),
        "no timescale warning: {:#?}",
        tb.diagnostics
    );
    assert!(src.join("_build/default/main_tb.vcd").is_file());

    // Real time limit.
    let start = std::time::Instant::now();
    let input = test_input(&src, json!({"timeout_seconds": 1}));
    let out = test::run(&state, input, &call()).await.unwrap();
    assert_eq!(out.testbenches[0].ending, Ending::Timeout);
    assert!(!out.passed);
    assert!(start.elapsed() < std::time::Duration::from_secs(8));

    let input = simulate_input(&src, json!({"max_sim_time_ns": 100}));
    let out = simulate::run(&state, input, &call()).await.unwrap();
    assert_eq!(out.ending, Ending::SimTimeLimit);
    let input = simulate_input(&src, json!({"timeout_seconds": 1}));
    let out = simulate::run(&state, input, &call()).await.unwrap();
    assert_eq!(out.ending, Ending::Timeout);

    for (extra, needle) in [
        (json!({"max_sim_time_ns": 0}), "max_sim_time_ns"),
        (json!({"timeout_seconds": 0}), "timeout_seconds"),
    ] {
        let err = simulate::run(&state, simulate_input(&src, extra), &call())
            .await
            .unwrap_err();
        assert!(err.message.contains(needle), "{}", err.message);
    }
}

#[tokio::test]
async fn a_testbench_without_finish_fails_and_long_output_is_shortened() {
    let state = engine().await;
    let (_tmp, src) = project();
    // Prints 6000 lines, then runs out of events without $finish.
    std::fs::write(
        src.join("main_tb.v"),
        "module main_tb;\n  integer i;\n  initial for (i = 0; i < 6000; i = i + 1) $display(\"line %0d\", i);\nendmodule\n",
    )
    .unwrap();
    let out = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap();
    let tb = &out.testbenches[0];
    assert_eq!(tb.ending, Ending::NoFinish);
    assert!(!tb.passed);
    assert!(tb.output.starts_with("line 0\n"));
    assert!(
        tb.output.contains("\n[... 1000 lines left out ...]\n"),
        "{}",
        &tb.output[..100]
    );
    assert!(tb.output.ends_with("line 5999"));
    assert_eq!(tb.output.lines().count(), 5001);
}

#[tokio::test]
async fn results_list_commands_that_run_again_without_the_engine() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out = lint::run(&state, lint_input(&src, json!({})), &call())
        .await
        .unwrap();
    assert!(out.passed);
    assert_eq!(out.commands.len(), 1);
    let cmd = &out.commands[0];
    assert_eq!(cmd.tool, "verilator_bin");
    assert!(
        cmd.package
            .as_deref()
            .unwrap()
            .starts_with("oss-cad-suite ")
    );
    // The shell line reproduces the result: clean, then a warning.
    let rerun = |shell: &str| {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(shell)
            .output()
            .unwrap()
    };
    assert!(rerun(&cmd.shell).status.success());
    std::fs::write(src.join("main.v"), MAIN.replace(".I3(q[3])", ".I3(q)")).unwrap();
    let again = rerun(&cmd.shell);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("WIDTHTRUNC"));

    let out = test::run(&state, test_input(&src, json!({})), &call())
        .await
        .unwrap();
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["iverilog", "vvp"]);
}

#[tokio::test]
async fn failed_calls_carry_their_commands_in_the_error() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("rtl/counter.v"),
        COUNTER.replace("cnt + 1'b1;", "cnt +;"),
    )
    .unwrap();
    let args = serde_json::to_value(json!({
        "board": "alhambra-ii",
        "source_folder": src,
        "source_files": ["main.v", "rtl/counter.v"],
        "testbench_files": ["main_tb.v"],
        "build_folder": src.join("_build/default"),
    }))
    .unwrap();
    let result = logix_engine::tools::call(&state, "test", args.as_object().cloned(), &call())
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let err = result.structured_content.unwrap();
    assert_eq!(err["category"], "design");
    let commands = err["data"]["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1, "{err:#}");
    assert_eq!(commands[0]["tool"], "iverilog");
    assert!(commands[0]["shell"].as_str().unwrap().contains("iverilog"));
}
