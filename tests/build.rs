//! Build and clean_build tests with the real toolchain. They share one engine
//! folder, target/tmp/test-engine, so oss-cad-suite (about 526 MB) and
//! openxc7 (about 124 MB) are downloaded once and reused by later runs. CI
//! caches them.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use logix_engine::errors::Category;
use logix_engine::packages::PackageManager;
use logix_engine::packages::{manifest, remote_config};
use logix_engine::progress::{NoProgress, Phases, ProgressSink};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, build, clean_build};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

fn engine_folder() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine")
}

/// An engine state with the toolchain downloaded.
async fn engine() -> EngineState {
    static READY: OnceCell<()> = OnceCell::const_new();
    let folder = engine_folder();
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
                .expect("download logix-core for the build tests");
        })
        .await;
    logix_engine::catalog::use_test_copy();
    EngineState {
        engine_folder: folder.display().to_string(),
        engine_folder_resolved: folder.clone(),
        auto_download: true,
        command_line: vec![],
        log_file: folder.join("logs/test.log"),
        packages,
    }
}

/// The plain names of a build's clocks, each with its net and a speed.
fn clock_names(out: &build::BuildOutput) -> Vec<String> {
    for c in &out.clocks_summary {
        assert!(!c.net.is_empty() && c.max_mhz > 0.0, "{c:?}");
    }
    out.clocks_summary.iter().map(|c| c.name.clone()).collect()
}

/// A build's resource by its toolchain name.
fn res<'a>(out: &'a build::BuildOutput, name: &str) -> &'a logix_engine::build::summary::Resource {
    out.resources
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no {name}: {:?}", out.resources))
}

fn call() -> Call {
    Call::new(Arc::new(NoProgress), CancellationToken::new())
}

const BLINK: &str = r#"`include "defs.vh"
module main(input CLK, output LED0, output LED1);
  reg [23:0] cnt = 0;
  always @(posedge CLK) cnt <= cnt + 1;
  assign LED0 = cnt[23];
`ifdef SYNTHESIZE
  assign LED1 = `LED1_VALUE;
`endif
endmodule
"#;

const PCF: &str = "set_io LED0 45\nset_io LED1 44\nset_io CLK 49\n";

/// A project folder with a blink design for the Alhambra II.
fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap().join("proj");
    std::fs::create_dir_all(src.join("inc")).unwrap();
    std::fs::write(src.join("main.v"), BLINK).unwrap();
    std::fs::write(src.join("defs.vh"), "`define LED1_VALUE 1'b1\n").unwrap();
    std::fs::write(src.join("pinout.pcf"), PCF).unwrap();
    (tmp, src)
}

fn request(src: &Path, extra: Value) -> build::BuildInput {
    let mut v = json!({
        "board": "alhambra-ii",
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v"],
        "constraint_file": "pinout.pcf",
        "build_folder": src.join("_build"),
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).expect("a valid build input")
}

async fn build_it(
    state: &EngineState,
    src: &Path,
    extra: Value,
) -> Result<build::BuildOutput, logix_engine::errors::ToolError> {
    build::run(state, request(src, extra), &call()).await
}

/// Writes custom boards to `dir/name` and returns the file's absolute path,
/// for custom_boards_folder.
/// Writes `boards` (ID to board) as a custom boards folder `dir/name`, a
/// file per board, and gives its path.
fn boards_file(dir: &Path, name: &str, boards: &Value) -> String {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    for (id, b) in boards.as_object().unwrap() {
        std::fs::write(
            path.join(format!("{id}.json")),
            serde_json::to_string_pretty(b).unwrap(),
        )
        .unwrap();
    }
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn builds_then_is_up_to_date_until_an_input_changes() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out_dir = src.join("_build");

    let out = build_it(&state, &src, json!({})).await.unwrap();
    assert!(!out.up_to_date);
    // The variant the build is for, in full, though the request named none.
    assert_eq!(
        out.board_variant,
        logix_engine::catalog::Selector::new("usb", "flash")
    );
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "nextpnr-ice40", "icepack"]);
    assert!(
        out.commands[0]
            .package
            .as_deref()
            .unwrap()
            .ends_with("(oss-cad-suite)")
    );
    assert_eq!(out.commands[0].cwd, src.to_string_lossy());
    assert_eq!(out.bitstream, "hardware.bin");
    // The plain-language summary: logic cells first, as the build used them.
    let main = &out.resources_summary[0];
    assert_eq!(
        (main.kind.as_str(), main.label.as_str()),
        ("logic_cells", "logic cells")
    );
    assert_eq!(main.used, res(&out, "ICESTORM_LC").used);
    assert!(out.resources_summary.iter().any(|r| r.kind == "io_pins"));
    // Each resource described, with its share; the report's text for people.
    assert_eq!(
        res(&out, "ICESTORM_LC").percent,
        out.resources_summary[0].percent
    );
    assert!(res(&out, "SB_IO").description.is_some());
    assert!(out.report.starts_with("The design uses "), "{}", out.report);
    assert!(
        out.report.contains("Maximum clock speed: "),
        "{}",
        out.report
    );
    assert!(
        out.report_summary.starts_with("Uses "),
        "{}",
        out.report_summary
    );
    let bin = std::fs::read(out_dir.join("hardware.bin")).unwrap();
    assert!(bin.len() > 100_000, "an hx8k bitstream is about 135 KB");
    let lc = &res(&out, "ICESTORM_LC");
    assert!(lc.used > 0 && lc.available == 7680, "{lc:?}");
    assert_eq!(res(&out, "SB_IO").used, 3);
    assert_eq!(out.clocks_summary.len(), 1);
    assert_eq!(clock_names(&out), ["CLK"], "{:?}", out.clocks_summary);
    assert!(
        out.clocks_summary.iter().all(|c| c.max_mhz > 50.0),
        "{:?}",
        out.clocks_summary
    );
    for f in [
        ".logix-build",
        "build-record.json",
        "synth.log",
        "pnr.log",
        "hardware.json",
    ] {
        assert!(out_dir.join(f).exists(), "{f}");
    }

    // Same inputs: up to date, same result.
    let again = build_it(&state, &src, json!({})).await.unwrap();
    assert!(again.up_to_date);
    assert!(
        again.commands.is_empty(),
        "an up-to-date build runs nothing"
    );
    assert_eq!(again.resources, out.resources);

    // An included file changes: rebuilds.
    std::fs::write(src.join("defs.vh"), "`define LED1_VALUE 1'b0\n").unwrap();
    assert!(!build_it(&state, &src, json!({})).await.unwrap().up_to_date);
    // A define changes: rebuilds.
    assert!(
        !build_it(&state, &src, json!({"defines": {"X": "1"}}))
            .await
            .unwrap()
            .up_to_date
    );
    assert!(
        build_it(&state, &src, json!({"defines": {"X": "1"}}))
            .await
            .unwrap()
            .up_to_date
    );
}

/// An ECP5 build with the standard commands, for the iCESugar-Pro.
#[tokio::test]
async fn builds_an_ecp5_design() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-ecp5");
    for f in ["main.v", "main.lpf"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let out = build_it(
        &state,
        &src,
        json!({"board": "icesugar-pro", "constraint_file": "main.lpf"}),
    )
    .await
    .unwrap();
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "nextpnr-ecp5", "ecppack"]);
    assert!(
        out.commands[0]
            .args
            .iter()
            .any(|a| a.contains("; synth_ecp5 -top main"))
    );
    // nextpnr gets the top module first.
    assert_eq!(out.commands[1].args[1..3], ["--top", "main"]);
    let pnr = &out.commands[1].args[3..];
    assert_eq!(
        &pnr[..6],
        ["--25k", "--package", "CABGA256", "--speed", "6", "--json"]
    );
    assert!(pnr.contains(&"--timing-allow-fail".to_string()));
    assert_eq!(out.bitstream, "hardware.bit");
    let bit = std::fs::read(src.join("_build/hardware.bit")).unwrap();
    assert!(bit.len() > 10_000, "{}", bit.len());
    assert!(res(&out, "TRELLIS_COMB").used > 0, "{:?}", out.resources);
    assert_eq!(out.clocks_summary.len(), 1, "{:?}", out.clocks_summary);
    assert_eq!(clock_names(&out), ["CLK"], "{:?}", out.clocks_summary);

    // Another variant changes only how the board is programmed, not the
    // bitstream: up to date, and the build record keeps the new variant for
    // program_board.
    let ram = json!({"board": "icesugar-pro", "constraint_file": "main.lpf",
                     "board_variant": {"connection": "usb", "memory": "ram"}});
    assert!(
        build_it(&state, &src, ram.clone())
            .await
            .unwrap()
            .up_to_date
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(src.join("_build/build-record.json")).unwrap())
            .unwrap();
    assert_eq!(
        record["board"]["variant"]["selector"],
        json!({"connection": "usb", "memory": "ram"})
    );
    assert_eq!(record["board"]["name"], "Icesugar Pro (RAM)");
    assert!(build_it(&state, &src, ram).await.unwrap().up_to_date);
    let err = build_it(
        &state,
        &src,
        json!({"board": "icesugar-pro", "constraint_file": "main.lpf",
               "board_variant": {"connection": "usb", "memory": "eeprom"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.category, logix_engine::errors::Category::Request);
    assert!(
        err.message.contains(
            "no variant with connection usb, memory eeprom. Its variants: (connection usb, memory flash), (connection usb, memory ram)"
        ),
        "{}",
        err.message
    );

    // A pin that doesn't exist is a design error.
    std::fs::write(
        src.join("main.lpf"),
        "LOCATE COMP \"CLK\" SITE \"P6\";\nLOCATE COMP \"LED0\" SITE \"ZZ99\";\n",
    )
    .unwrap();
    let err = build_it(
        &state,
        &src,
        json!({"board": "icesugar-pro", "constraint_file": "main.lpf"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err.category,
        logix_engine::errors::Category::Design,
        "{err:?}"
    );
    assert!(!src.join("_build/hardware.bit").exists());
}

/// A Gowin build with the standard commands, for the Tang Nano 9K.
#[tokio::test]
async fn builds_a_gowin_design() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-gowin");
    for f in ["main.v", "main.cst"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let input = json!({"board": "sipeed-tang-nano-9k", "constraint_file": "main.cst"});
    // gowin_pack's warnings about missing Python speedups are hidden (an
    // output rule), and nothing else of it.
    #[derive(Default)]
    struct Lines(std::sync::Mutex<Vec<String>>);
    impl logix_engine::progress::OutputSink for Lines {
        fn line(&self, line: &logix_engine::progress::ToolLine) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{}: {}", line.tool, line.text));
        }
    }
    let lines = Arc::new(Lines::default());
    let mut c = call();
    c.output = lines.clone();
    let out = build::run(&state, request(&src, input.clone()), &c)
        .await
        .unwrap();
    let shown = lines.0.lock().unwrap().clone();
    assert!(
        !shown.iter().any(|l| l.contains("is not available")),
        "{shown:?}"
    );
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "nextpnr-himbaechel", "gowin_pack"]);
    assert!(
        out.commands[0]
            .args
            .iter()
            .any(|a| a.contains("; synth_gowin -top main; write_json "))
    );
    // nextpnr gets the top module first.
    assert_eq!(out.commands[1].args[1..3], ["--top", "main"]);
    let pnr = &out.commands[1].args[3..];
    assert_eq!(
        &pnr[..10],
        [
            "--device",
            "GW1NR-LV9QN88PC6/I5",
            "--json",
            "hardware.json",
            "--write",
            "hardware.pnr.json",
            "--report",
            "hardware.pnr",
            "--vopt",
            "family=GW1N-9C"
        ]
    );
    assert_eq!(pnr[10], "--vopt");
    assert!(pnr[11].starts_with("cst=") && pnr[11].ends_with("main.cst"));
    assert_eq!(
        out.commands[2].args[1..],
        ["-d", "GW1N-9C", "-o", "hardware.fs", "hardware.pnr.json"]
    );
    assert_eq!(out.bitstream, "hardware.fs");
    let fs = std::fs::read(src.join("_build/hardware.fs")).unwrap();
    assert!(fs.len() > 10_000, "{}", fs.len());
    assert!(
        res(&out, "LUT4").used > 0 && res(&out, "DFF").used > 0,
        "{:?}",
        out.resources
    );
    assert_eq!(out.clocks_summary.len(), 1, "{:?}", out.clocks_summary);
    assert_eq!(clock_names(&out), ["CLK"], "{:?}", out.clocks_summary);

    // A pin that doesn't exist is a design error.
    std::fs::write(
        src.join("main.cst"),
        "IO_LOC \"CLK\" 52;\nIO_LOC \"LED0\" 999;\n",
    )
    .unwrap();
    let err = build_it(&state, &src, input).await.unwrap_err();
    assert_eq!(
        err.category,
        logix_engine::errors::Category::Design,
        "{err:?}"
    );
    assert!(!src.join("_build/hardware.fs").exists());
}

/// A Xilinx build with the standard commands, for the Arty A7-35T: yosys from
/// oss-cad-suite, the rest from openxc7.
#[tokio::test]
async fn builds_a_xilinx_design() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-xilinx");
    for f in ["main.v", "main.xdc"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let input = json!({"board": "arty-a7-35t", "constraint_file": "main.xdc"});
    let out = build_it(&state, &src, input.clone()).await.unwrap();
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(
        tools,
        ["yosys", "nextpnr-xilinx", "fasm2frames", "xc7frames2bit"]
    );
    let packages: Vec<_> = out
        .commands
        .iter()
        .map(|c| c.package.clone().unwrap_or_default())
        .collect();
    assert!(packages[0].ends_with("(oss-cad-suite)"), "{packages:?}");
    assert!(
        packages[1..].iter().all(|p| p.ends_with("(openxc7)")),
        "{packages:?}"
    );
    assert!(
        out.commands[0]
            .args
            .iter()
            .any(|a| a.contains("; synth_xilinx -arch xc7 -top main; write_json "))
    );
    // nextpnr gets the top module first.
    assert_eq!(out.commands[1].args[1..3], ["--top", "main"]);
    let pnr = &out.commands[1].args[3..];
    assert_eq!(&pnr[..3], ["--device", "xc7a35tcsg324-1", "-o"]);
    assert!(pnr[3].starts_with("xdc=") && pnr[3].ends_with("main.xdc"));
    assert_eq!(
        &pnr[4..10],
        [
            "-o",
            "fasm=hardware.fasm",
            "--json",
            "hardware.json",
            "--report",
            "hardware.pnr"
        ]
    );
    let frames = &out.commands[2].args[1..];
    assert!(frames[3].ends_with("share/nextpnr/external/prjxray-db/artix7"));
    assert_eq!(&frames[4..], ["hardware.fasm", "hardware.frames"]);
    assert!(out.commands[3].args[2].ends_with("prjxray-db/artix7/xc7a35tcsg324-1/part.yaml"));
    assert_eq!(out.bitstream, "hardware.bit");
    let bit = std::fs::read(src.join("_build/hardware.bit")).unwrap();
    assert!(bit.len() > 100_000, "{}", bit.len());
    assert!(!out.resources.is_empty(), "{:?}", out.resources);
    assert_eq!(out.clocks_summary.len(), 1, "{:?}", out.clocks_summary);
    // The design's clock signal, as for the other families (openXC7
    // 2026-11-09 and later name the clock net after it).
    assert_eq!(clock_names(&out), ["CLK"], "{:?}", out.clocks_summary);

    // A pin that doesn't exist is a design error.
    std::fs::write(
        src.join("main.xdc"),
        "set_property PACKAGE_PIN E3 [get_ports CLK]\nset_property PACKAGE_PIN ZZ99 [get_ports LED0]\n",
    )
    .unwrap();
    let err = build_it(&state, &src, input).await.unwrap_err();
    assert_eq!(
        err.category,
        logix_engine::errors::Category::Design,
        "{err:?}"
    );
    assert!(!src.join("_build/hardware.bit").exists());
}

/// An empty module other than the top, such as a primitive's stub written for
/// a simulator, stays a black box: synthesis keeps the primitive.
#[tokio::test]
async fn an_empty_primitive_stub_keeps_the_primitive() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap();
    std::fs::write(
        src.join("main.v"),
        "module SB_HFOSC (input CLKHFPU, input CLKHFEN, output CLKHF);\nendmodule\n\n\
         module main (output led);\n  wire clk;\n  SB_HFOSC osc (.CLKHFPU(1'b1), .CLKHFEN(1'b1), .CLKHF(clk));\n\
         \x20 reg [3:0] n = 0;\n  always @(posedge clk) n <= n + 1;\n  assign led = n[3];\nendmodule\n",
    )
    .unwrap();
    std::fs::write(src.join("pinout.pcf"), "set_io led 11\n").unwrap();
    let input = json!({"board": "icebreaker", "constraint_file": "pinout.pcf"});
    let out = build_it(&state, &src, input).await;
    assert!(out.is_ok(), "{:?}", out.err());
    let netlist = std::fs::read_to_string(src.join("_build/hardware.json")).unwrap();
    assert!(
        netlist.contains("\"type\": \"SB_HFOSC\""),
        "the oscillator is gone"
    );
}

/// Designs without logic build on every family: one with no ports and a pin
/// file without pins, and one whose only port is the clock input. yosys
/// makes such an empty module a black box, unless the synthesis script
/// unmarks the top module: nextpnr couldn't find it, and on Xilinx the input
/// got no I/O buffer ("IO port 'sys_clk' is missing an IO buffer").
#[tokio::test]
async fn designs_without_logic_build() {
    let state = engine().await;
    for (board, file, clock) in [
        ("alhambra-ii", "pinout.pcf", "set_io sys_clk 49\n"),
        (
            "icesugar-pro",
            "pinout.lpf",
            "LOCATE COMP \"sys_clk\" SITE \"P6\";\n",
        ),
        (
            "sipeed-tang-primer-20k-lite",
            "pinout.cst",
            "IO_LOC \"sys_clk\" H11;\n",
        ),
        (
            "arty-a7-35t",
            "pinout.xdc",
            "set_property -dict { PACKAGE_PIN E3 IOSTANDARD LVCMOS33 } [get_ports {sys_clk}]\n",
        ),
    ] {
        for (main, pins) in [
            ("module main;\nendmodule\n", "# No pins yet.\n"),
            ("module main (input sys_clk);\nendmodule\n", clock),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let src = dunce::canonicalize(tmp.path()).unwrap();
            std::fs::write(src.join("main.v"), main).unwrap();
            let pins = if file.ends_with(".cst") {
                pins.replace('#', "//")
            } else {
                pins.to_string()
            };
            std::fs::write(src.join(file), pins).unwrap();
            let input = json!({"board": board, "constraint_file": file});
            let out = build_it(&state, &src, input).await;
            assert!(out.is_ok(), "{board}, {main:?}: {:?}", out.err());
        }
    }
}

#[tokio::test]
async fn a_syntax_error_is_a_design_error_and_removes_the_old_bitstream() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out_dir = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();
    std::fs::write(out_dir.join("my-notes.txt"), "keep").unwrap();

    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n  reg x\n  assign LED0 = 1;\nendmodule\n",
    )
    .unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Design, "{err:?}");
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("main.v"));
    assert_eq!(err.diagnostics[0].line, Some(3));
    assert!(err.message.contains("main.v:3"), "{}", err.message);
    assert!(!out_dir.join("hardware.bin").exists(), "no stale bitstream");
    assert!(
        !out_dir.join("build-record.json").exists(),
        "no stale record"
    );
    assert!(
        out_dir.join("my-notes.txt").exists(),
        "other files are kept"
    );
}

#[tokio::test]
async fn design_errors_from_each_tool() {
    let state = engine().await;

    // Unknown top module: no location.
    let (_t1, src) = project();
    let err = build_it(&state, &src, json!({"top_module": "nosuch"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert!(err.message.contains("nosuch"), "{}", err.message);

    // A pin the package doesn't have: in the constraint file.
    let (_t2, src) = project();
    std::fs::write(
        src.join("pinout.pcf"),
        "set_io LED0 45\nset_io LED1 999\nset_io CLK 49\n",
    )
    .unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("pinout.pcf"));
    assert_eq!(err.diagnostics[0].line, Some(2));
    assert_eq!(err.diagnostics[0].tool.as_deref(), Some("nextpnr"));

    // An unconstrained port.
    let (_t3, src) = project();
    std::fs::write(src.join("pinout.pcf"), "set_io LED0 45\nset_io CLK 49\n").unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert!(err.message.contains("LED1"), "{}", err.message);
}

#[tokio::test]
async fn warnings_are_reported() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n  assign LED0 = undeclared;\n  assign LED1 = 0;\nendmodule\n",
    )
    .unwrap();
    let out = build_it(&state, &src, json!({})).await.unwrap();
    let w = out
        .warnings
        .iter()
        .find(|w| w.text.contains("undeclared"))
        .expect("the implicit declaration warning");
    assert_eq!(w.file.as_deref(), Some("main.v"));
    assert_eq!(w.line, Some(2));
}

#[tokio::test]
async fn request_errors() {
    let state = engine().await;
    let (tmp, src) = project();
    let cases = [
        json!({"build_folder": src.join("out")}),
        json!({"build_folder": src}),
        json!({"source_files": ["../proj/main.v"]}),
        json!({"source_files": ["missing.v"]}),
        json!({"source_files": []}),
        json!({"constraint_file": "/abs.pcf"}),
        json!({"board": "no-such-board"}),
        json!({"defines": {"1BAD": "x"}}),
        json!({"top_module": "two words"}),
        json!({"source_folder": "relative/path"}),
    ];
    for extra in cases {
        let err = build_it(&state, &src, extra.clone()).await.unwrap_err();
        assert_eq!(err.category, Category::Request, "{extra}: {err:?}");
    }
    // A PCF file named .lpf, for an iCE40 board: nextpnr would read it as PCF
    // anyway, but the name is the wrong family's format.
    std::fs::copy(src.join("pinout.pcf"), src.join("pinout.lpf")).unwrap();
    let err = build_it(&state, &src, json!({"constraint_file": "pinout.lpf"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request, "{err:?}");
    assert!(err.message.contains("is the wrong kind"), "{}", err.message);
    assert!(err.action.is_none(), "{err:?}");
    std::fs::remove_file(src.join("pinout.lpf")).unwrap();
    // An include from outside the source folder.
    std::fs::write(tmp.path().join("outside.vh"), "").unwrap();
    std::fs::write(
        src.join("main.v"),
        format!("`include \"../outside.vh\"\n{BLINK}"),
    )
    .unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(
        !src.join("_build").exists(),
        "nothing written on a request error"
    );
}

#[tokio::test]
async fn a_custom_board_works() {
    let state = engine().await;
    let (_tmp, src) = project();
    let mine = json!({
        "name": "My Alhambra II copy",
        "fpga_id": "ice40hx4k-tq144-8k",
        "variants": [{
            "selector": {"connection": "usb", "memory": "flash"},
            "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
            "usb": {"vid": "0403", "pid": "6010"}, "tested": false
        }]
    });
    let out = build_it(
        &state,
        &src,
        json!({"board": "my-alhambra",
               "custom_boards_folder": boards_file(&src, "boards", &json!({"my-alhambra": mine}))}),
    )
    .await
    .unwrap();
    assert!(src.join("_build").join(out.bitstream).exists());
    // Without the file, the ID means nothing.
    let err = build_it(&state, &src, json!({"board": "my-alhambra"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    // A file with a problem: a diagnostic at its line and column, named
    // from the source folder, as for a source file; nothing is built.
    std::fs::write(
        src.join("boards").join("my-alhambra.json"),
        "{\n  \"name\": \"x\",\n  \"fpga_id\": \"x\",\n}\n",
    )
    .unwrap();
    let file = src.join("boards").to_string_lossy().into_owned();
    let err = build_it(
        &state,
        &src,
        json!({"board": "my-alhambra", "custom_boards_folder": file}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.category, Category::Request);
    // As the request gave it, even inside the source folder.
    let d = &err.diagnostics[0];
    assert_eq!(
        (d.file.as_deref(), d.line, d.column),
        (
            Some(format!("{file}/my-alhambra.json").as_str()),
            Some(4),
            Some(1)
        ),
        "{d:?}"
    );
}

#[tokio::test]
async fn progress_and_cancel() {
    #[derive(Default)]
    struct Messages(std::sync::Mutex<Vec<String>>);
    impl ProgressSink for Messages {
        fn report(&self, _p: f64, _t: f64, m: Option<String>) {
            if let Some(m) = m {
                self.0.lock().unwrap().push(m);
            }
        }
    }
    let state = engine().await;
    let (_tmp, src) = project();
    let rec = Arc::new(Messages::default());
    let c = Call::new(rec.clone(), CancellationToken::new());
    build::run(&state, request(&src, json!({})), &c)
        .await
        .unwrap();
    assert_eq!(
        *rec.0.lock().unwrap(),
        vec!["Synthesizing", "Placing and routing", "Packing bitstream"]
    );

    let (_tmp2, src) = project();
    let c = Call::new(Arc::new(NoProgress), CancellationToken::new());
    c.cancel.cancel();
    let err = build::run(&state, request(&src, json!({})), &c)
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Cancelled);
}

/// Build folders are disposable: deleting one loses nothing, and the next
/// build creates it again with the same result.
#[tokio::test]
async fn a_deleted_build_folder_is_built_again_with_the_same_result() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();
    let bin = std::fs::read(folder.join("hardware.bin")).unwrap();
    let record = std::fs::read(folder.join("build-record.json")).unwrap();

    std::fs::remove_dir_all(&folder).unwrap();
    let out = build_it(&state, &src, json!({})).await.unwrap();
    assert!(!out.up_to_date, "a deleted folder is built again");
    assert!(folder.join(".logix-build").is_file());
    assert_eq!(std::fs::read(folder.join("hardware.bin")).unwrap(), bin);
    assert_eq!(
        std::fs::read(folder.join("build-record.json")).unwrap(),
        record
    );
}

#[tokio::test]
async fn clean_build_removes_a_folder_of_build_folders() {
    let state = engine().await;
    let (_tmp, src) = project();
    let root = src.join("_build");
    build_it(
        &state,
        &src,
        json!({"build_folder": root.join("alhambra-ii")}),
    )
    .await
    .unwrap();
    build_it(&state, &src, json!({"build_folder": root.join("icezum")}))
        .await
        .unwrap();

    let out = clean_build::run(
        &state,
        serde_json::from_value(json!({"folder": root})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        out.deleted,
        vec![
            root.join("alhambra-ii").display().to_string(),
            root.join("icezum").display().to_string(),
            root.display().to_string(),
        ]
    );
    assert!(!root.exists());
    assert!(src.join("main.v").exists());

    // The project folder itself fails the path rules.
    let err = clean_build::run(
        &state,
        serde_json::from_value(json!({"folder": src})).unwrap(),
    )
    .unwrap_err();
    assert_eq!(err.category, Category::Request);
}

// ----- list_devices and program_board, with fake device lists. Programming
// real boards is checked by hand on hardware.

use logix_engine::devices::{Device, MatchLevel};
use logix_engine::errors::ActionCode;
use logix_engine::tools::{list_devices, program_board};

fn usb(product: &str, serial: Option<&str>, dev: u8) -> Device {
    Device::usb(
        "0403".into(),
        "6010".into(),
        None,
        Some(product.into()),
        serial.map(Into::into),
        250,
        dev,
        format!("250-{dev}"),
        true,
    )
}

fn program_input(build_folder: &Path, extra: Value) -> program_board::ProgramBoardInput {
    let mut v = json!({ "build_folder": build_folder });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).unwrap()
}

#[test]
fn list_devices_names_the_boards_each_device_may_be() {
    let devices = vec![
        usb("Alhambra II", Some("A1"), 3),
        usb("Dual RS232-HS", None, 4),
    ];
    let out = list_devices::run_with(Default::default(), || Ok(devices.clone())).unwrap();
    let best = &out.devices[0].boards[0];
    assert_eq!(
        (best.board.as_str(), best.variants[0].level),
        ("alhambra-ii", MatchLevel::Strong)
    );
    assert!(
        out.devices[1]
            .boards
            .iter()
            .all(|m| m.board != "alhambra-ii"),
        "another product name"
    );
    assert_eq!(out.devices[0].id, devices[0].id);
}

#[tokio::test]
async fn list_devices_for_a_build_lists_only_the_devices_that_fit() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    let input = |folder: &Path| -> list_devices::ListDevicesInput {
        serde_json::from_value(json!({"build_folder": folder})).unwrap()
    };
    // No build yet: a request error.
    let err = list_devices::run_with(input(&folder), || Ok(vec![])).unwrap_err();
    assert_eq!(err.category, Category::Request);

    build_it(&state, &src, json!({})).await.unwrap();
    let mut locked = usb("", None, 7);
    locked.product = None;
    locked.reachable = false;
    let devices = vec![
        usb("Dual RS232-HS", None, 4),
        usb("Alhambra II v1.0A", Some("A1"), 3),
        locked,
    ];
    let out = list_devices::run_with(input(&folder), || Ok(devices)).unwrap();
    let fit: Vec<_> = out
        .devices
        .iter()
        .map(|d| (d.product.as_deref().unwrap(), d.build_match))
        .collect();
    assert_eq!(fit, [("Alhambra II v1.0A", Some(MatchLevel::Strong))]);
    assert!(!out.devices[0].boards.is_empty(), "still all its boards");
    assert_eq!(out.unreachable.len(), 1);
    assert_eq!(out.unreachable[0].build_match, Some(MatchLevel::Weak));
}

#[tokio::test]
async fn program_board_needs_a_successful_build() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    let input = program_input(&folder, json!({"device": "usb:250-3:0403:6010:"}));
    let err = program_board::run_with(&state, input, &call(), || Ok(vec![]))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(err.message.contains("Run build first"), "{}", err.message);
}

#[tokio::test]
async fn program_board_checks_the_chosen_device() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();
    let alhambra = || usb("Alhambra II v2.0", Some("A1"), 3);
    let second = || usb("Alhambra II", Some("B2"), 5);
    let other = || usb("Dual RS232-HS", None, 4);

    let run = |devices: Vec<Device>, extra: Value| {
        let state = state.clone();
        let input = program_input(&folder, extra);
        async move { program_board::run_with(&state, input, &call(), move || Ok(devices)).await }
    };

    // Not connected anymore.
    let err = run(vec![other()], json!({"device": alhambra().id}))
        .await
        .unwrap_err();
    assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);

    // Not an ID, and a device that isn't the board.
    let err = run(vec![alhambra()], json!({"device": "alhambra"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    let err = run(vec![alhambra(), other()], json!({"device": other().id}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(err.message.contains("Dual RS232-HS"), "{}", err.message);

    // A command that doesn't name the device runs only when no other device
    // has its USB IDs.
    let err = run(
        vec![alhambra(), other()],
        json!({"device": alhambra().id, "programmer_command": "openFPGALoader ${BITSTREAM}"}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.action.unwrap().code, ActionCode::DisconnectDevices);

    // The real openFPGALoader, pointed at a device that isn't there: a
    // setup error with connect_board and the programmer's output.
    let err = run(vec![alhambra(), second()], json!({"device": second().id}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup, "{err:?}");
    assert_eq!(err.action.as_ref().unwrap().code, ActionCode::ConnectBoard);
    assert!(err.details.contains("device not found"), "{err:?}");
}

#[tokio::test]
async fn program_board_runs_the_command_with_the_device_and_bitstream() {
    #[derive(Default)]
    struct Messages(std::sync::Mutex<Vec<String>>);
    impl ProgressSink for Messages {
        fn report(&self, _p: f64, _t: f64, m: Option<String>) {
            if let Some(m) = m {
                self.0.lock().unwrap().push(m);
            }
        }
    }
    let state = engine().await;
    let (tmp, src) = project();
    let folder = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();

    // A fake programmer that records its arguments.
    let log = tmp.path().join("args.txt");
    let script = tmp.path().join("fake-prog.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" > {}\necho 'Writing: 50.00%'\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cmd = format!(
        "{} --busdev ${{BUS}}:${{DEV}} ${{BITSTREAM}}",
        script.display()
    );
    let rec = Arc::new(Messages::default());
    let c = Call::new(rec.clone(), CancellationToken::new());
    let out = program_board::run_with(
        &state,
        program_input(
            &folder,
            json!({"programmer_command": cmd, "device": usb("Alhambra II", None, 9).id}),
        ),
        &c,
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap();
    assert_eq!(out.programmer, "custom");
    assert_eq!(out.device.device, 9);
    assert_eq!(out.commands.len(), 1);
    assert_eq!(out.commands[0].args[0], script.to_string_lossy());
    assert_eq!(out.commands[0].package, None, "a custom programmer");
    let args = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        args.trim(),
        format!("--busdev 250:9 {}", folder.join("hardware.bin").display())
    );
    assert_eq!(
        *rec.0.lock().unwrap(),
        vec!["Finding device", "Programming"]
    );

    // A failing programmer is an error with its output.
    std::fs::write(&script, "#!/bin/sh\necho 'segfault here' >&2\nexit 3\n").unwrap();
    let err = program_board::run_with(
        &state,
        program_input(
            &folder,
            json!({"programmer_command": cmd, "device": usb("Alhambra II", None, 9).id}),
        ),
        &call(),
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.category, Category::Tool);
    assert!(err.details.contains("segfault here"));
}

#[tokio::test]
async fn verbose_prints_more_but_gives_the_same_result() {
    use logix_engine::progress::{OutputSink, ToolLine};
    #[derive(Default)]
    struct Lines(std::sync::Mutex<Vec<String>>);
    impl OutputSink for Lines {
        fn line(&self, line: &ToolLine) {
            self.0.lock().unwrap().push(line.text.to_string());
        }
    }
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n  assign LED0 = undeclared;\n  assign LED1 = 0;\nendmodule\n",
    )
    .unwrap();
    let mut results = Vec::new();
    let mut counts = Vec::new();
    for (folder, verbose) in [("_build/quiet", false), ("_build/verbose", true)] {
        let lines = Arc::new(Lines::default());
        let mut c = Call::new(Arc::new(NoProgress), CancellationToken::new());
        c.output = lines.clone();
        let input = request(
            &src,
            json!({"build_folder": src.join(folder), "verbose": verbose}),
        );
        results.push(build::run(&state, input, &c).await.unwrap());
        counts.push(lines.0.lock().unwrap().len());
    }
    assert!(
        counts[1] > counts[0] * 5 + 50,
        "verbose printed more: {counts:?}"
    );
    assert_eq!(
        results[0].warnings, results[1].warnings,
        "same warnings, no duplicates"
    );
    assert_eq!(results[0].resources, results[1].resources);
}

/// A programmer's progress redrawn with carriage returns, as iceprog prints
/// it: the live output marks each redraw, and leaves out the blank erasers.
#[tokio::test]
async fn live_output_marks_progress_redraws() {
    use logix_engine::progress::{OutputSink, ToolLine};
    #[derive(Default)]
    struct Lines(std::sync::Mutex<Vec<(String, bool)>>);
    impl OutputSink for Lines {
        fn line(&self, line: &ToolLine) {
            self.0
                .lock()
                .unwrap()
                .push((line.text.to_string(), line.redraw));
        }
    }
    let state = engine().await;
    let (tmp, src) = project();
    let folder = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();
    let script = tmp.path().join("fake-iceprog.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf 'programming..\\n                      \\raddr 0x000000   0%%\\r                      \\raddr 0x000100  50%%\\r                      \\rdone.\\n' >&2\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let lines = Arc::new(Lines::default());
    let mut c = Call::new(Arc::new(NoProgress), CancellationToken::new());
    c.output = lines.clone();
    program_board::run_with(
        &state,
        program_input(
            &folder,
            json!({"programmer_command": format!("{} ${{BITSTREAM}}", script.display()), "device": usb("Alhambra II", None, 9).id}),
        ),
        &c,
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap();
    assert_eq!(
        *lines.0.lock().unwrap(),
        vec![
            ("programming..".to_string(), false),
            ("addr 0x000000   0%".to_string(), true),
            ("addr 0x000100  50%".to_string(), true),
            ("done.".to_string(), false),
        ]
    );
}

/// Before programming over JTAG, program_board reads the connected FPGA's ID
/// code with openFPGALoader --detect, and refuses another FPGA before
/// anything is written. Uses a fake openFPGALoader.
#[tokio::test]
async fn program_board_refuses_a_board_with_another_fpga() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = dunce::canonicalize(tmp.path()).unwrap().join("proj");
    std::fs::create_dir(&src).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-xilinx");
    for f in ["main.v", "main.xdc"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let ram = json!({"board": "arty-a7-35t", "board_variant": {"connection": "usb", "memory": "ram"}, "constraint_file": "main.xdc"});
    build_it(&state, &src, ram).await.unwrap();
    let folder = src.join("_build");

    // The fake prints detect.txt for --detect, and logs every run.
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let log = tmp.path().join("runs.txt");
    let detect = tmp.path().join("detect.txt");
    let script = bin.join("openFPGALoader");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> {log}\ncase \"$*\" in *--detect*) cat {detect};; esac\n",
            log = log.display(),
            detect = detect.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let arty = usb("Digilent USB Device", Some("210319AFEF8F"), 4);
    let program = |fpga: &str| {
        std::fs::write(&detect, fpga).unwrap();
        let _ = std::fs::remove_file(&log);
        let state = &state;
        let input = program_input(&folder, json!({"device": arty.id}));
        let devices = vec![arty.clone()];
        let bin = bin.clone();
        async move {
            program_board::run_with_programmers(
                state,
                input,
                &call(),
                move || Ok(devices),
                Some(&bin),
            )
            .await
        }
    };
    let a35 = "index 0:\n\tidcode 0x362d093\n\tmanufacturer xilinx\n\tfamily artix a7 35t\n\tmodel  xc7a35\n\tirlength 6\n";

    // The right FPGA: detect, then program.
    let out = program(a35).await.unwrap();
    let runs = std::fs::read_to_string(&log).unwrap();
    let runs: Vec<&str> = runs.lines().collect();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(
        runs[0],
        "--force-terminal-mode --board arty_a7_35t --busdev-num 250:4 --detect"
    );
    assert!(runs[1].ends_with("hardware.bit"), "{}", runs[1]);
    assert_eq!(out.commands.len(), 2, "both runs are in commands");

    // An Arty A7-100T: refused after detect, nothing written.
    let a100 = a35
        .replace("0x362d093", "0x3631093")
        .replace("xc7a35", "xc7a100");
    let err = program(&a100).await.unwrap_err();
    assert_eq!(err.category, Category::Setup);
    assert!(
        err.message.contains("has XC7A100 (ID code 0x03631093)"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("Arty A7-100T"),
        "suggests the matching board: {}",
        err.message
    );
    let runs = std::fs::read_to_string(&log).unwrap();
    assert_eq!(runs.lines().count(), 1, "only detect ran: {runs}");

    // Custom boards: the upload uses the board the build was made with,
    // never another definition with its ID.
    // The catalog's entry, as is: a custom board under any ID.
    let my_arty =
        serde_json::to_value(&logix_engine::catalog::get().boards["arty-a7-35t"]).unwrap();
    let boards_dir = tmp.path().join("boards");
    std::fs::create_dir(&boards_dir).unwrap();
    let uploads = std::sync::atomic::AtomicUsize::new(0);
    // custom: the custom boards file's boards, or None for no file.
    let upload = |folder: PathBuf, custom: Option<Value>| {
        let _ = std::fs::remove_file(&log);
        let state = &state;
        let mut args = json!({"device": arty.id});
        if let Some(boards) = custom {
            let n = uploads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            args["custom_boards_folder"] =
                json!(boards_file(&boards_dir, &format!("boards-{n}"), &boards));
        }
        let input = program_input(&folder, args);
        let devices = vec![arty.clone()];
        let bin = bin.clone();
        async move {
            program_board::run_with_programmers(
                state,
                input,
                &call(),
                move || Ok(devices),
                Some(&bin),
            )
            .await
        }
    };
    std::fs::write(&detect, a35).unwrap();
    let mine = src.join("_build_mine");
    let custom_build = json!({"board": "my-arty", "board_variant": {"connection": "usb", "memory": "ram"},
        "custom_boards_folder": boards_file(&boards_dir, "build-mine", &json!({"my-arty": my_arty.clone()})),
        "constraint_file": "main.xdc", "build_folder": mine});
    build_it(&state, &src, custom_build).await.unwrap();
    upload(mine.clone(), Some(json!({"my-arty": my_arty.clone()})))
        .await
        .unwrap();
    // What doesn't change the bitstream may change after the build: the
    // upload uses the board as the request has it now, such as new
    // programmer arguments.
    let mut renamed = my_arty.clone();
    renamed["name"] = json!("Renamed");
    renamed["variants"][1]["user_notes"] = json!("A new note.");
    renamed["variants"][1]["args"] = json!(format!(
        "{} --skip-reset",
        my_arty["variants"][1]["args"].as_str().unwrap()
    ));
    assert_eq!(
        renamed["variants"][1]["selector"],
        json!({"connection": "usb", "memory": "ram"})
    );
    upload(mine.clone(), Some(json!({"my-arty": renamed})))
        .await
        .unwrap();
    let runs = std::fs::read_to_string(&log).unwrap();
    assert!(
        runs.lines().last().unwrap().contains("--skip-reset"),
        "{runs}"
    );
    // The list left out, or the board's FPGA changed since the build:
    // refused, nothing run.
    for custom in [None, {
        let mut changed = my_arty.clone();
        changed["fpga_id"] = json!("xc7a100tcsg324-1");
        Some(json!({"my-arty": changed}))
    }] {
        let err = upload(mine.clone(), custom).await.unwrap_err();
        assert_eq!(err.category, Category::Request);
        assert!(err.message.contains("run build again"), "{}", err.message);
        assert!(!log.exists());
    }
    // Built with an exact copy of the catalog board under its ID, then
    // uploaded without the list: the same board, so it goes ahead.
    let same = my_arty.clone();
    let same_folder = src.join("_build_same");
    let same_build = json!({"board": "arty-a7-35t", "board_variant": {"connection": "usb", "memory": "ram"},
        "custom_boards_folder": boards_file(&boards_dir, "build-same", &json!({"arty-a7-35t": same})),
        "constraint_file": "main.xdc", "build_folder": same_folder});
    build_it(&state, &src, same_build).await.unwrap();
    upload(same_folder, None).await.unwrap();
    // A catalog build, with a custom board of another FPGA now replacing
    // its ID: refused.
    let mut over = my_arty.clone();
    over["fpga_id"] = json!("xc7a100tcsg324-1");
    let err = upload(folder.clone(), Some(json!({"arty-a7-35t": over})))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
}

/// The wrong-FPGA error suggests only the boards the device most likely is,
/// as list_devices rates them: for a Digilent device with an XC7A35T, the
/// boards whose filters name Digilent's chip, not the Cmod A7, whose
/// filter matches any FTDI chip with those USB IDs.
#[test]
fn the_wrong_fpga_error_suggests_the_most_likely_boards() {
    let digilent = usb("Digilent USB Device", Some("210319AFEF8F"), 4);
    let boards = program_board::boards_with_fpga(0x0362d093, &digilent);
    assert!(
        boards.iter().any(|b| b.starts_with("Arty A7-35T")),
        "{boards:?}"
    );
    assert!(
        !boards.iter().any(|b| b.starts_with("Cmod A7")),
        "{boards:?}"
    );
    // A plain FTDI chip: the boards whose filters have only the USB IDs,
    // not the Alchitry Au, which needs its own name.
    let plain = usb("Dual RS232-HS", None, 4);
    let boards = program_board::boards_with_fpga(0x0362d093, &plain);
    assert!(
        boards.iter().any(|b| b.starts_with("Cmod A7")),
        "{boards:?}"
    );
    assert!(
        !boards.iter().any(|b| b.starts_with("Alchitry Au")),
        "{boards:?}"
    );
}

/// The configuration this engine release pins: every FPGA in its
/// logix-catalog is one its logix-core's tools can build, by the list the
/// core generates from their databases (logix-core.json `fpgas`).
#[tokio::test]
async fn the_pinned_catalog_fits_the_pinned_core() {
    use logix_engine::catalog;
    use logix_engine::packages::CATALOG;
    let state = engine().await;
    let missing = state.packages.needing_download(&[CATALOG]).await.unwrap();
    let phases = Phases::new(Arc::new(NoProgress), missing.len());
    state
        .packages
        .ensure(&missing, true, &phases, &CancellationToken::new())
        .await
        .expect("download logix-catalog");
    let spec = state.packages.spec(CATALOG).unwrap().clone();
    let pinned = catalog::read_folder(&state.packages.package_dir(&spec)).unwrap();
    let core = state
        .packages
        .core_fpgas()
        .expect("the core's logix-core.json");
    for toolchain in ["ice40", "ecp5", "gowin", "openxc7"] {
        assert!(
            core.get(toolchain).is_some_and(|l| !l.is_empty()),
            "{toolchain}"
        );
    }
    assert_eq!(catalog::not_in_core(&pinned, &core), Vec::<String>::new());
    // The repo's catalog/ copy, the tests' data, too.
    let copy = catalog::read_folder(&catalog::test_copy_folder()).unwrap();
    assert_eq!(catalog::not_in_core(&copy, &core), Vec::<String>::new());
}
