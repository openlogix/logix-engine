//! End-to-end tests: launch the real engine binary as a host would, and talk
//! MCP with it over stdin/stdout.

use std::path::Path;

use rmcp::ServiceExt;
use std::sync::{Arc, Mutex};

use rmcp::ClientHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, JsonObject, NumberOrString, ProgressNotificationParam,
    ProgressToken, RequestMetaObject,
};
use rmcp::service::NotificationContext;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::process::Command;

const ENGINE: &str = env!("CARGO_BIN_EXE_logix-engine");

async fn start(engine_folder: &Path, extra: &[&str]) -> RunningService<RoleClient, ()> {
    let mut cmd = Command::new(ENGINE);
    cmd.arg("--engine-folder").arg(engine_folder).args(extra);
    let transport = TokioChildProcess::new(cmd).expect("launch the engine");
    ().serve(transport).await.expect("start an MCP session")
}

async fn call(client: &RunningService<RoleClient, ()>, name: &str, args: Value) -> CallToolResult {
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Value::Object(map) = args {
        params = params.with_arguments(map);
    }
    client.call_tool(params).await.expect("tool call")
}

fn assert_matches_schema(schema: &JsonObject, value: &Value) {
    let schema = Value::Object(schema.clone());
    let validator = jsonschema::validator_for(&schema).expect("valid schema");
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "{value} doesn't match its schema: {errors:?}"
    );
}

#[tokio::test]
async fn get_engine_info_reports_the_launch_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let engine_folder = tmp.path().join("engine");
    let client = start(
        &engine_folder,
        &["--no-auto-install", "--log-level", "debug"],
    )
    .await;

    let tools = client.list_all_tools().await.unwrap();
    let tool = tools
        .iter()
        .find(|t| t.name == "get_engine_info")
        .expect("listed");

    let result = call(&client, "get_engine_info", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let out = result
        .structured_content
        .clone()
        .expect("structured content");
    assert_matches_schema(tool.output_schema.as_ref().unwrap(), &out);

    let folder = engine_folder.to_str().unwrap();
    assert_eq!(out["engine_folder"], folder);
    assert_eq!(out["auto_install"], false);
    assert_eq!(
        out["command_line"],
        json!([
            "--engine-folder",
            folder,
            "--no-auto-install",
            "--log-level",
            "debug"
        ])
    );
    assert_eq!(
        out["protocol_versions"],
        json!(["2026-07-28", "2025-11-25"])
    );
    // A fresh engine folder has the manifest's packages, not yet installed.
    assert_eq!(
        out["packages"],
        json!([
            {"name": "oss-cad-suite", "version": "2026-10-22", "status": "missing"},
            {"name": "verible", "version": "2026-10-22", "status": "missing"}
        ])
    );

    // The log file is in the engine folder's logs subfolder, named
    // yyyymmdd-<pid>.log, and has the startup line.
    let log_file = out["log_file"].as_str().unwrap();
    assert!(
        log_file.starts_with(&format!("{folder}/logs/")),
        "{log_file}"
    );
    assert!(log_file.ends_with(".log"));
    let log = std::fs::read_to_string(log_file).expect("log file exists");
    assert!(
        log.contains("Logix Engine") && log.contains("started"),
        "{log}"
    );
    assert!(log.contains("Tool call: get_engine_info"), "{log}");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn auto_install_is_on_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let client = start(&tmp.path().join("engine"), &[]).await;
    let out = call(&client, "get_engine_info", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(out["auto_install"], true);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn bad_input_is_a_request_error_the_caller_can_read() {
    let tmp = tempfile::tempdir().unwrap();
    let client = start(&tmp.path().join("engine"), &[]).await;
    let result = call(&client, "get_engine_info", json!({"bogus": 1})).await;
    assert_eq!(result.is_error, Some(true));
    let err = result.structured_content.unwrap();
    assert_eq!(err["category"], "request");
    assert!(err["message"].as_str().unwrap().contains("bogus"), "{err}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_unknown_tool_is_a_protocol_error() {
    let tmp = tempfile::tempdir().unwrap();
    let client = start(&tmp.path().join("engine"), &[]).await;
    let params = CallToolRequestParams::new("no_such_tool");
    assert!(client.call_tool(params).await.is_err());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tools_list_matches_the_published_schemas() {
    let tmp = tempfile::tempdir().unwrap();
    let client = start(&tmp.path().join("engine"), &[]).await;
    let tools = client.list_all_tools().await.unwrap();
    let listed = serde_json::to_value(&tools).unwrap();
    let published: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/tools.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        listed, published,
        "run `scripts/update-schemas.sh` and review the change"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn install_packages_refuses_unknown_packages() {
    let tmp = tempfile::tempdir().unwrap();
    let client = start(&tmp.path().join("engine"), &[]).await;
    let result = call(&client, "install_packages", json!({"packages": ["nope"]})).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.structured_content.unwrap()["category"], "request");
    client.cancel().await.unwrap();
}

/// A client that records progress notifications.
#[derive(Clone, Default)]
struct ProgressClient(Arc<Mutex<Vec<ProgressNotificationParam>>>);

impl ClientHandler for ProgressClient {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.0.lock().unwrap().push(params);
    }
}

/// Installs the real packages (oss-cad-suite is about 526 MB) through MCP,
/// with progress. Run: cargo test --test mcp -- --ignored
#[tokio::test]
#[ignore]
async fn real_install_packages_with_progress() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(ENGINE);
    cmd.arg("--engine-folder").arg(tmp.path().join("engine"));
    let handler = ProgressClient::default();
    let client = handler
        .clone()
        .serve(TokioChildProcess::new(cmd).unwrap())
        .await
        .unwrap();

    let mut params = CallToolRequestParams::new("install_packages");
    let mut meta = RequestMetaObject::new();
    meta.set_progress_token(ProgressToken(NumberOrString::Number(7)));
    params.meta = Some(meta);
    let result = client.call_tool(params).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let out = result.structured_content.unwrap();
    assert_eq!(out["installed"], json!(["oss-cad-suite", "verible"]));
    for p in out["packages"].as_array().unwrap() {
        assert_eq!(p["status"], "installed", "{p}");
    }

    let progress = handler.0.lock().unwrap().clone();
    assert!(progress.len() > 10, "{}", progress.len());
    assert_eq!(
        progress[0].message.as_deref(),
        Some("Installing oss-cad-suite")
    );
    assert!(
        progress
            .iter()
            .any(|p| p.message.as_deref() == Some("Installing verible"))
    );
    // One phase per package, 100 each.
    assert!(progress.iter().all(|p| p.total == Some(200.0)));
    assert!(progress.windows(2).all(|w| w[1].progress > w[0].progress));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn get_catalog_returns_the_requested_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let client = start(&tmp.path().join("engine"), &[]).await;
    let tools = client.list_all_tools().await.unwrap();
    let tool = tools.iter().find(|t| t.name == "get_catalog").unwrap();

    // Nothing asked: empty sections.
    let out = call(&client, "get_catalog", json!({}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(out, json!({"boards": {}, "fpgas": {}}));

    // One board and all FPGAs.
    let result = call(
        &client,
        "get_catalog",
        json!({"boards": {"fetch": true, "ids": ["alhambra-ii"]}, "fpgas": {"fetch": true}}),
    )
    .await;
    let out = result.structured_content.unwrap();
    assert_matches_schema(tool.output_schema.as_ref().unwrap(), &out);
    let boards = out["boards"].as_object().unwrap();
    assert_eq!(boards.len(), 1);
    let alhambra = &boards["alhambra-ii"];
    assert_eq!(alhambra["description"], "Alhambra II");
    assert_eq!(alhambra["fpga_id"], "ice40hx4k-tq144-8k");
    assert_eq!(
        alhambra["variants"]["flash"]["usb"]["product_regex"],
        "^Alhambra II.*"
    );
    assert_eq!(out["fpgas"].as_object().unwrap().len(), 40);
    assert_eq!(out["fpgas"]["ice40hx4k-tq144-8k"]["architecture"], "ice40");

    // An unknown ID is a request error.
    let result = call(
        &client,
        "get_catalog",
        json!({"boards": {"fetch": true, "ids": ["nope"]}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.structured_content.unwrap()["category"], "request");
    client.cancel().await.unwrap();
}

/// A client that records logix/output notifications.
#[derive(Clone, Default)]
struct OutputClient(Arc<Mutex<Vec<Value>>>);

impl ClientHandler for OutputClient {
    async fn on_custom_notification(
        &self,
        notification: rmcp::model::CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) {
        if notification.method == "logix/output" {
            self.0
                .lock()
                .unwrap()
                .push(notification.params.unwrap_or_default());
        }
    }
}

#[tokio::test]
async fn build_streams_tool_output_when_asked() {
    // The toolchain from the build tests' shared engine folder.
    let engine = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine");
    std::fs::create_dir_all(&engine).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap().join("proj");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0);\n  assign LED0 = undeclared;\nendmodule\n",
    )
    .unwrap();
    std::fs::write(src.join("pinout.pcf"), "set_io LED0 45\nset_io CLK 49\n").unwrap();

    let mut cmd = Command::new(ENGINE);
    cmd.arg("--engine-folder").arg(&engine);
    let handler = OutputClient::default();
    let client = handler
        .clone()
        .serve(TokioChildProcess::new(cmd).unwrap())
        .await
        .unwrap();

    let args = |folder: &str| {
        json!({
            "board": "alhambra-ii", "top_module": "main", "source_folder": src,
            "source_files": ["main.v"], "constraint_file": "pinout.pcf",
            "build_folder": src.join(folder),
        })
    };
    let request = |folder: &str, live: bool| {
        let mut p = CallToolRequestParams::new("build");
        if let Value::Object(m) = args(folder) {
            p = p.with_arguments(m);
        }
        if live {
            let mut meta = RequestMetaObject::new();
            meta.0.0.insert("logix/output".into(), json!(true));
            p.meta = Some(meta);
        }
        p
    };

    // Without the flag: nothing streamed.
    let r = client.call_tool(request("_build/a", false)).await.unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    assert!(handler.0.lock().unwrap().is_empty());

    // With it: yosys's warning arrives as a warning line, before the result.
    let r = client.call_tool(request("_build/b", true)).await.unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let lines = handler.0.lock().unwrap().clone();
    let warning = lines
        .iter()
        .find(|l| {
            l["text"]
                .as_str()
                .unwrap_or("")
                .contains("implicitly declared")
        })
        .unwrap_or_else(|| panic!("no warning line in {lines:?}"));
    assert_eq!(warning["tool"], "yosys");
    assert_eq!(warning["kind"], "output");
    assert_eq!(warning["level"], "warning");
    // Each tool's block starts with its command line.
    let commands: Vec<&Value> = lines.iter().filter(|l| l["kind"] == "command").collect();
    let tools: Vec<&str> = commands
        .iter()
        .map(|c| c["tool"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["yosys", "nextpnr-ice40", "icepack"]);
    assert!(
        commands[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("yosys -q -l "),
        "{:?}",
        commands[0]
    );
    assert_eq!(
        lines[0]["kind"], "command",
        "the command comes before the tool's lines"
    );
    assert!(warning["request_id"].is_number() || warning["request_id"].is_string());
    client.cancel().await.unwrap();
}
