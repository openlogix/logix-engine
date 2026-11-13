//! diagram with the real toolchain, on the designs in tests/designs. It uses
//! the same engine folder as tests/build.rs, target/tmp/test-engine, so the
//! package is downloaded once and reused.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use logix_engine::errors::Category;
use logix_engine::packages::PackageManager;
use logix_engine::packages::{manifest, remote_config};
use logix_engine::progress::{NoProgress, Phases};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, diagram, test};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

/// An engine state with the toolchain downloaded.
async fn engine() -> EngineState {
    static READY: OnceCell<()> = OnceCell::const_new();
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = dunce::canonicalize(folder).unwrap();
    let packages = PackageManager::with_remote_config(
        &folder,
        remote_config::repo_file_source(),
        true,
        &state::platform(),
        manifest::GITHUB,
    );
    READY
        .get_or_init(|| async {
            let missing = packages.needing_download(&["logix-core"]).await.unwrap();
            let phases = Phases::new(Arc::new(NoProgress), missing.len());
            packages
                .ensure(&missing, true, &phases, &CancellationToken::new())
                .await
                .expect("download logix-core for the diagram tests");
        })
        .await;
    logix_engine::catalog::use_test_copy();
    // Layout workers are the engine's own binary, not this test's.
    logix_engine::diagram::layout::set_worker_program(env!("CARGO_BIN_EXE_logix-engine").into());
    EngineState {
        engine_folder: folder.display().to_string(),
        engine_folder_resolved: folder.clone(),
        auto_download: true,
        command_line: vec![],
        log_file: folder.join("logs/test.log"),
        packages,
    }
}

fn call() -> Call {
    Call::new(Arc::new(NoProgress), CancellationToken::new())
}

/// A copy of a design from tests/designs in a folder named `name`, so the
/// originals are never changed.
fn design(design: &str, name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap().join(name);
    std::fs::create_dir_all(&src).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/designs")
        .join(design);
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), src.join(entry.file_name())).unwrap();
    }
    (tmp, src)
}

fn input(src: &Path, extra: Value) -> diagram::DiagramInput {
    let mut v = json!({
        "board": "icesugar-1-5",
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v"],
        "build_folder": src.join("_build"),
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).expect("a valid diagram input")
}

fn read(src: &Path, file: &str) -> String {
    std::fs::read_to_string(src.join("_build").join(file)).unwrap()
}

fn model(src: &Path) -> Value {
    serde_json::from_str(&read(src, "diagram/diagram.json")).unwrap()
}

#[tokio::test]
async fn diagram_draws_the_design() {
    let state = engine().await;
    // A space in the path, which yosys's script must quote.
    let (_tmp, src) = design("blinky", "my proj");
    let c = call();
    let out = diagram::run(&state, input(&src, json!({})), &c)
        .await
        .unwrap();
    assert_eq!(out.page_file, "diagram/diagram.html");
    assert_eq!(out.model_file, "diagram/diagram.json");
    assert!(!out.opened);
    assert_eq!(out.modules.len(), 1);
    let m = &out.modules[0];
    assert!(m.top && m.key == "main" && m.note.is_none(), "{m:?}");
    assert_eq!(m.svg_file.as_deref(), Some("diagram/main.svg"));
    let svg = read(&src, "diagram/main.svg");
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"logix-diagram\""));
    // No script in the SVG; its default style uses the variables.
    assert!(!svg.contains("<script"));
    assert!(svg.contains("var(--logix-diagram-register,"));
    // The pins, the counter's register and its adder, and the wires.
    assert!(
        svg.contains(r#"data-pin="CLK""#) && svg.contains(r#"data-pin="LED0""#),
        "{svg}"
    );
    assert!(
        svg.contains("cat-reg") && svg.contains("cat-arith"),
        "{svg}"
    );
    assert!(
        svg.contains(r#"class="wire"#) && svg.contains("portbox"),
        "{svg}"
    );

    // No folder in what yosys names: cells are named after their file as
    // the request gave it.
    assert!(!svg.contains(&src.display().to_string()), "{svg}");
    assert!(svg.contains(r#"data-block="$main.v:"#), "{svg}");

    // The model: every block with its source line.
    let model = model(&src);
    assert_eq!(model["top"], "main");
    let blocks = model["modules"]["main"]["blocks"].as_object().unwrap();
    let reg = blocks.values().find(|b| b["category"] == "reg").unwrap();
    assert_eq!(reg["kind"], "logic");
    assert_eq!(reg["source"]["file"], "main.v");
    assert!(reg["source"]["line"].as_u64().unwrap() > 0);
    assert!(reg["name"].is_string(), "{reg}");

    // The page: self-contained, with the data and no outside resources.
    let page = read(&src, "diagram/diagram.html");
    assert!(page.contains(r#""top":"main""#));
    assert!(!page.contains("/*__DATA__*/"));
    assert!(!page.contains("<link"));
    assert!(!page.contains("src=\"http"));
    assert!(src.join("_build/diagram/netlist.json").is_file());

    // The command is ready to run again.
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys"]);
    let script = &out.commands[0].args[2];
    assert!(script.starts_with("read_verilog -lib \""), "{script}");
    assert!(script.contains("read_verilog -sv \"main.v\";"), "{script}");
    assert!(
        script.ends_with("/_build/diagram/netlist.json\""),
        "{script}"
    );
    std::fs::remove_file(src.join("_build/diagram/netlist.json")).unwrap();
    let rerun = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&out.commands[0].shell)
        .status()
        .unwrap();
    assert!(rerun.success());
    assert!(src.join("_build/diagram/netlist.json").is_file());
}

/// A top module with a clock and a reset to several registers, a fixed
/// input, a memory loaded from a file, and two versions of a module.
const HIERARCHY: &str = r#"module counter #(parameter N = 4) (input clk, input rst, output reg [N-1:0] q);
  always @(posedge clk)
    if (rst) q <= 0;
    else q <= q + 1;
endmodule

module main(input CLK, input RST, input [1:0] SEL, output [3:0] LEDS, output [7:0] DATA);
  wire [3:0] a;
  wire [5:0] b;
  counter #(.N(4)) small (.clk(CLK), .rst(RST), .q(a));
  counter #(.N(6)) big (.clk(CLK), .rst(RST), .q(b));
  reg [7:0] rom [0:3];
  initial $readmemh("rom.hex", rom);
  reg [7:0] data;
  always @(posedge CLK) data <= rom[SEL];
  assign DATA = data;
  assign LEDS = a ^ b[5:2];
endmodule
"#;

#[tokio::test]
async fn diagram_draws_every_module_with_the_prototype_rules() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    std::fs::write(src.join("main.v"), HIERARCHY).unwrap();
    std::fs::write(src.join("rom.hex"), "01\n02\n03\n04\n").unwrap();
    let out = diagram::run(&state, input(&src, json!({})), &call())
        .await
        .unwrap();
    let keys: Vec<&str> = out.modules.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys[0], "main");
    assert_eq!(keys.len(), 3, "{keys:?}");
    assert!(
        keys.contains(&"counter") && keys.contains(&"counter-2"),
        "{keys:?}"
    );
    assert!(out.modules.iter().all(|m| m.svg_file.is_some()));

    let model = model(&src);
    let main = &model["modules"]["main"];
    // The instances link to their modules, for drill-down.
    let instances: Vec<&Value> = main["blocks"]
        .as_object()
        .unwrap()
        .values()
        .filter(|b| b["kind"] == "module")
        .collect();
    assert_eq!(instances.len(), 2);
    for i in &instances {
        assert!(
            model["modules"][i["module"].as_str().unwrap()].is_object(),
            "{i}"
        );
        assert_eq!(i["title"], "counter");
    }
    // The clock and the reset go to several places: labels, not wires.
    let labels = main["labels"].as_object().unwrap();
    assert!(
        labels
            .values()
            .any(|l| l["text"] == "CLK" && l["direction"] == "in"),
        "{labels:?}"
    );
    assert!(labels.values().any(|l| l["text"] == "RST"), "{labels:?}");
    // The memory: its initialization isn't drawn, and it says where it's
    // loaded.
    let blocks = main["blocks"].as_object().unwrap();
    assert!(
        !blocks
            .values()
            .any(|b| b["type"].as_str().unwrap().starts_with("$meminit"))
    );
    assert!(
        blocks
            .values()
            .any(|b| b["memory_init"]["file"] == "main.v"),
        "{blocks:?}"
    );
    // In the counter, the reset value is a fixed input.
    let counter = &model["modules"]["counter"];
    assert!(
        counter["blocks"]
            .as_object()
            .unwrap()
            .values()
            .any(|b| b["category"] == "reg" && b["name"] == "q")
    );
    let svg = read(&src, "diagram/main.svg");
    assert!(
        svg.contains("cat-module") && svg.contains("lbltext signal"),
        "{svg}"
    );
    // A bus split: part of b goes to the XOR.
    assert!(svg.contains(r#"data-kind="split""#), "{svg}");
    let page = read(&src, "diagram/diagram.html");
    assert!(page.contains(r#""counter-2""#));
}

#[tokio::test]
async fn diagram_names_fpga_blocks_for_every_family() {
    let state = engine().await;
    for (design, board, file) in [
        ("blinky", "icesugar-1-5", "main.v"),
        ("blinky-ecp5", "icesugar-pro", "main.v"),
        ("blinky-gowin", "sipeed-tang-nano-9k", "main.v"),
        ("blinky-xilinx", "arty-a7-35t", "main.v"),
    ] {
        let (_tmp, src) = design_any(design);
        let out = diagram::run(
            &state,
            input(&src, json!({"board": board, "source_files": [file]})),
            &call(),
        )
        .await
        .unwrap_or_else(|e| panic!("{design}: {e:?}"));
        assert!(out.modules[0].svg_file.is_some(), "{design}");
    }
    // An FPGA primitive gets its plain name and its ports' directions.
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n\
         `ifdef SYNTHESIZE\n  SB_LUT4 #(.LUT_INIT(16'h8000)) lut (.O(LED0), .I0(CLK), .I1(1'b1), \
         .I2(1'b1), .I3(1'b1));\n`endif\n\
         `ifdef MARK\n  assign LED1 = ~CLK;\n`endif\nendmodule\n",
    )
    .unwrap();
    diagram::run(
        &state,
        input(&src, json!({"defines": {"MARK": ""}})),
        &call(),
    )
    .await
    .unwrap();
    let model = model(&src);
    let main = &model["modules"]["main"];
    let lut = &main["blocks"]["lut"];
    assert_eq!(lut["kind"], "fpga");
    assert_eq!(lut["title"], "Logic cell (4-input LUT)");
    assert_eq!(lut["ports"]["O"]["direction"], "output");
    assert_eq!(lut["fixed"]["I1"], "1");
    // The define reached yosys: its inverter is drawn.
    assert!(
        main["blocks"]
            .as_object()
            .unwrap()
            .values()
            .any(|b| b["type"] == "$not")
    );
}

/// A layout that fails or runs too long stops only its worker: the next
/// one starts a new worker, and the engine goes on.
#[tokio::test]
async fn layout_workers_fail_alone() {
    let state = engine().await;
    let bundle = state
        .packages
        .component_dir(logix_engine::diagram::ELKJS)
        .unwrap()
        .join("lib/elk.bundled.js");
    use logix_engine::diagram::layout::{Layouts, Stop};
    use std::time::Duration;
    let graph = r#"{"id":"root","children":[{"id":"a","width":10,"height":10}],"edges":[]}"#;
    // No ELK: the worker can't start its layout.
    let mut broken = Layouts::new(
        Path::new("/nonexistent/elk.bundled.js"),
        CancellationToken::new(),
    );
    assert!(matches!(
        broken.layout(graph, Duration::from_secs(30)).await,
        Err(Stop::Failed(_))
    ));
    assert!(matches!(
        broken.layout(graph, Duration::from_secs(30)).await,
        Err(Stop::Failed(_))
    ));
    // Too long: stopped, then a new worker lays out the next graph.
    let mut layouts = Layouts::new(&bundle, CancellationToken::new());
    assert_eq!(
        layouts.layout(graph, Duration::from_millis(1)).await,
        Err(Stop::TooLong)
    );
    let laid = layouts
        .layout(graph, Duration::from_secs(30))
        .await
        .unwrap();
    assert!(laid.contains(r#""x""#), "{laid}");
    // Cancelled.
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut stopped = Layouts::new(&bundle, cancel);
    assert_eq!(
        stopped.layout(graph, Duration::from_secs(30)).await,
        Err(Stop::Cancelled)
    );
}

/// Modules deeper than `levels` are in the model, not drawn.
#[tokio::test]
async fn diagram_draws_only_levels_below_the_top() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    std::fs::write(
        src.join("main.v"),
        "module leaf(input a, output y); assign y = ~a; endmodule\n\
         module mid(input a, output y); leaf l(.a(a), .y(y)); endmodule\n\
         module main(input A, output Y); mid m(.a(A), .y(Y)); endmodule\n",
    )
    .unwrap();
    let out = diagram::run(&state, input(&src, json!({"levels": 1})), &call())
        .await
        .unwrap();
    let by_key = |k: &str| out.modules.iter().find(|m| m.key == k).unwrap();
    assert!(by_key("main").svg_file.is_some() && by_key("mid").svg_file.is_some());
    let leaf = by_key("leaf");
    assert!(leaf.svg_file.is_none());
    assert!(
        leaf.note
            .as_deref()
            .unwrap()
            .contains("more than 1 levels below main"),
        "{leaf:?}"
    );
    // A diagram that starts at a module: it's the top.
    let out = diagram::run(&state, input(&src, json!({"top_module": "mid"})), &call())
        .await
        .unwrap();
    assert_eq!(out.modules[0].key, "mid");
    assert!(out.modules[0].top);
    assert_eq!(out.modules.len(), 2);
}

/// A copy of a test design, in a folder of its own.
fn design_any(name: &str) -> (tempfile::TempDir, PathBuf) {
    design(name, "proj")
}

#[tokio::test]
async fn diagram_source_places_follow_display_paths() {
    let state = engine().await;
    let (_tmp, src) = design("blinky", "my proj");
    let mut args = json!({
        "board": "icesugar-1-5",
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v"],
        "build_folder": src.join("_build"),
        "display_paths": {"folders": [{"path": src, "as": "PROJECT"}]},
    });
    let result = logix_engine::tools::call(&state, "diagram", args.as_object().cloned(), &call())
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text = read(&src, "diagram/diagram.json");
    assert!(text.contains(r#""file": "PROJECT/main.v""#), "{text}");
    // Without display_paths: relative to the source folder.
    args.as_object_mut().unwrap().remove("display_paths");
    logix_engine::tools::call(&state, "diagram", args.as_object().cloned(), &call())
        .await
        .unwrap();
    let text = read(&src, "diagram/diagram.json");
    assert!(text.contains(r#""file": "main.v""#), "{text}");
}

#[tokio::test]
async fn diagram_design_errors_and_no_stale_diagram() {
    let state = engine().await;
    let (_tmp, src) = design("syntax-error", "proj");
    // An old diagram is removed, so a failure never leaves it looking current.
    std::fs::create_dir_all(src.join("_build/diagram")).unwrap();
    std::fs::write(src.join("_build/diagram/diagram.html"), "old").unwrap();
    let err = diagram::run(&state, input(&src, json!({})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design, "{err:?}");
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("main.v"));
    assert_eq!(err.diagnostics[0].line, Some(3));
    assert!(!src.join("_build/diagram/diagram.html").exists());

    let (_tmp, src) = design("blinky", "proj");
    let err = diagram::run(&state, input(&src, json!({"top_module": "nope"})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert_eq!(
        err.message,
        "The design has errors. The top module nope isn't in the design files."
    );
}

/// The counter design's testbenches pass and fail as their names say.
#[tokio::test]
async fn the_counter_test_design_has_a_passing_and_a_failing_testbench() {
    let state = engine().await;
    let (_tmp, src) = design("counter", "proj");
    let v = json!({
        "board": "alhambra-ii",
        "source_folder": src,
        "source_files": ["counter.v"],
        "testbench_files": ["counter_tb.v", "counter_fail_tb.v"],
        "build_folder": src.join("_build"),
    });
    let out = test::run(&state, serde_json::from_value(v).unwrap(), &call())
        .await
        .unwrap();
    let passed: Vec<_> = out.testbenches.iter().map(|t| t.passed).collect();
    assert_eq!(passed, [true, false]);
    assert!(
        out.testbenches[1].diagnostics[0]
            .text
            .contains("q is 9, expected 8"),
        "{:?}",
        out.testbenches[1].diagnostics
    );
}
