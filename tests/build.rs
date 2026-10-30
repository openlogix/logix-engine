//! Build and clean_build tests with the real toolchain. They share one engine
//! folder, target/tmp/test-engine, so oss-cad-suite is downloaded once (about
//! 526 MB) and reused by later runs. CI caches it.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use logix_engine::errors::Category;
use logix_engine::packages::PackageManager;
use logix_engine::packages::manifest;
use logix_engine::progress::{NoProgress, Phases, ProgressSink};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, build, clean_build};
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
                .expect("install oss-cad-suite for the build tests");
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
    let src = tmp.path().canonicalize().unwrap().join("proj");
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
        "build_folder": src.join("_build/default"),
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

#[tokio::test]
async fn builds_then_is_up_to_date_until_an_input_changes() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out_dir = src.join("_build/default");

    let out = build_it(&state, &src, json!({})).await.unwrap();
    assert!(!out.up_to_date);
    assert_eq!(out.bitstream, "hardware.bin");
    let bin = std::fs::read(out_dir.join("hardware.bin")).unwrap();
    assert!(bin.len() > 100_000, "an hx8k bitstream is about 135 KB");
    let lc = &out.resources_usage["ICESTORM_LC"];
    assert!(lc.used > 0 && lc.available == 7680, "{lc:?}");
    assert_eq!(out.resources_usage["SB_IO"].used, 3);
    assert_eq!(out.timing.len(), 1);
    assert!(
        out.timing.values().all(|mhz| *mhz > 50.0),
        "{:?}",
        out.timing
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
    assert_eq!(again.resources_usage, out.resources_usage);

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

#[tokio::test]
async fn a_syntax_error_is_a_design_error_and_removes_the_old_bitstream() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out_dir = src.join("_build/default");
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
async fn a_full_board_definition_works() {
    let state = engine().await;
    let (_tmp, src) = project();
    let board = json!({
        "description": "My Alhambra II copy",
        "fpga": {
            "architecture": "ice40", "part_number": "ICE40HX4K-TQ144", "size": "8k",
            "nextpnr_device": "hx8k", "nextpnr_package": "tq144:4k"
        },
        "default_variant": "flash",
        "variants": {"flash": {
            "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
            "usb": {"vid": "0403", "pid": "6010"}
        }}
    });
    let out = build_it(&state, &src, json!({"board": board}))
        .await
        .unwrap();
    assert!(src.join("_build/default").join(out.bitstream).exists());
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

#[tokio::test]
async fn clean_build_removes_apio_style_env_folders() {
    let state = engine().await;
    let (_tmp, src) = project();
    let root = src.join("_build");
    build_it(&state, &src, json!({"build_folder": root.join("default")}))
        .await
        .unwrap();
    build_it(&state, &src, json!({"build_folder": root.join("env2")}))
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
            root.join("default").display().to_string(),
            root.join("env2").display().to_string(),
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

use logix_engine::devices::{Device, DeviceKind};
use logix_engine::errors::ActionCode;
use logix_engine::tools::{list_devices, program_board};

fn usb(product: &str, serial: Option<&str>, dev: u8) -> Device {
    Device {
        kind: DeviceKind::Usb,
        vid: "0403".into(),
        pid: "6010".into(),
        manufacturer: None,
        product: Some(product.into()),
        serial_number: serial.map(Into::into),
        bus: 250,
        device: dev,
        matches_board: None,
    }
}

fn program_input(build_folder: &Path, extra: Value) -> program_board::ProgramBoardInput {
    let mut v = json!({ "build_folder": build_folder });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).unwrap()
}

#[test]
fn list_devices_marks_the_matching_boards() {
    let devices = vec![
        usb("Alhambra II", Some("A1"), 3),
        usb("Dual RS232-HS", None, 4),
    ];
    let input: list_devices::ListDevicesInput =
        serde_json::from_value(json!({"board": "alhambra-ii"})).unwrap();
    let out = list_devices::run_with(input, || Ok(devices.clone())).unwrap();
    let flags: Vec<_> = out.devices.iter().map(|d| d.matches_board).collect();
    assert_eq!(flags, vec![Some(true), Some(false)]);

    let out = list_devices::run_with(Default::default(), || Ok(devices.clone())).unwrap();
    assert!(out.devices.iter().all(|d| d.matches_board.is_none()));

    let input: list_devices::ListDevicesInput =
        serde_json::from_value(json!({"board": "nope"})).unwrap();
    assert_eq!(
        list_devices::run_with(input, || Ok(vec![]))
            .unwrap_err()
            .category,
        Category::Request
    );
}

#[tokio::test]
async fn program_board_needs_a_successful_build() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build/default");
    let err = program_board::run_with(&state, program_input(&folder, json!({})), &call(), || {
        Ok(vec![])
    })
    .await
    .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(err.message.contains("Run build first"), "{}", err.message);
}

#[tokio::test]
async fn program_board_finds_exactly_one_device() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build/default");
    build_it(&state, &src, json!({})).await.unwrap();
    let alhambra = || usb("Alhambra II v2.0", Some("A1"), 3);
    let other = || usb("Dual RS232-HS", None, 4);

    let run = |devices: Vec<Device>, extra: Value| {
        let state = state.clone();
        let input = program_input(&folder, extra);
        async move { program_board::run_with(&state, input, &call(), move || Ok(devices)).await }
    };

    let err = run(vec![other()], json!({})).await.unwrap_err();
    assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);

    let err = run(
        vec![alhambra(), usb("Alhambra II", Some("B2"), 5)],
        json!({}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.action.unwrap().code, ActionCode::SelectDevice);
    assert_eq!(err.data.unwrap()["candidates"].as_array().unwrap().len(), 2);

    // The real openFPGALoader, pointed at a device that isn't there: a
    // setup error with connect_board and the programmer's output.
    let err = run(
        vec![alhambra(), usb("Alhambra II", Some("B2"), 5)],
        json!({"serial_number": "B2"}),
    )
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
    let folder = src.join("_build/default");
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
        program_input(&folder, json!({"programmer_command": cmd})),
        &c,
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap();
    assert_eq!(out.programmer, "custom");
    assert_eq!(out.device.device, 9);
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
        program_input(&folder, json!({"programmer_command": cmd})),
        &call(),
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.category, Category::Tool);
    assert!(err.details.contains("segfault here"));
}
