//! The engine's MCP tools, from docs/tool-api.md. Each tool has an input
//! type and an output type. Their JSON schemas are generated from the Rust
//! types, so the code, the validation and the exported schemas agree.

pub mod build;
pub mod check_updates;
pub mod clean_build;
pub mod compare_boards;
pub mod diagram;
pub mod download_packages;
pub mod format;
pub mod get_catalog;
pub mod get_engine_info;
pub mod install_driver;
pub mod lint;
pub mod list_devices;
pub mod program_board;
pub mod simulate;
pub mod test;
pub mod uninstall_driver;

use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use tokio_util::sync::CancellationToken;

use crate::errors::ToolError;
use crate::progress::{OutputSink, ProgressSink};
use crate::state::EngineState;

/// What a tool call gets besides its input: where to send progress, and a
/// token that is cancelled when the host cancels the call.
pub struct Call {
    pub progress: Arc<dyn ProgressSink>,
    /// Live tool output, line by line.
    pub output: Arc<dyn OutputSink>,
    pub cancel: CancellationToken,
    /// The programs the call ran, for the result's commands list.
    pub commands: Arc<crate::commands::CommandLog>,
    /// How the request names paths for people (`display_paths`). The live
    /// output and the result are named by it already; a tool uses it only
    /// for text it writes elsewhere, such as the diagram's labels.
    pub display: Arc<crate::display_paths::Display>,
}

impl Call {
    /// A call with the given progress sink and no live output.
    pub fn new(progress: Arc<dyn ProgressSink>, cancel: CancellationToken) -> Self {
        Call {
            progress,
            output: Arc::new(crate::progress::NoOutput),
            cancel,
            commands: Default::default(),
            display: Default::default(),
        }
    }
}

/// A tool's name, description and types.
pub trait EngineTool {
    const NAME: &'static str;
    /// What the tool does, for hosts and AI agents that learn the tools from
    /// their descriptions alone.
    const DESCRIPTION: &'static str;
    /// True if the tool changes nothing, on disk or on a board.
    const READ_ONLY: bool;
    /// True if the tool may use the network, such as to download packages.
    const OPEN_WORLD: bool = false;
    type Input: DeserializeOwned + JsonSchema;
    type Output: Serialize + JsonSchema;
}

/// The JSON schema of a type, as an MCP schema object.
pub fn schema_of<T: JsonSchema>() -> JsonObject {
    let mut value = serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default();
    remove_number_formats(&mut value);
    match value {
        serde_json::Value::Object(map) => map,
        _ => unreachable!("a derived schema is a JSON object"),
    }
}

/// Removes the number formats schemars adds, such as "uint32" and "double".
/// They aren't standard JSON Schema formats, so clients warn about them, and
/// the ranges are already given by minimum and maximum.
fn remove_number_formats(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let numeric = map.get("format").and_then(|f| f.as_str()).is_some_and(|f| {
                f.starts_with("int") || f.starts_with("uint") || f == "float" || f == "double"
            });
            if numeric {
                map.remove("format");
            }
            for v in map.values_mut() {
                remove_number_formats(v);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(remove_number_formats),
        _ => {}
    }
}

/// The MCP definition of a tool, as `tools/list` returns it.
pub fn definition<T: EngineTool>() -> Tool {
    let mut annotations = ToolAnnotations::default();
    annotations.read_only_hint = Some(T::READ_ONLY);
    annotations.destructive_hint = Some(false);
    annotations.open_world_hint = Some(T::OPEN_WORLD);
    Tool::new(T::NAME, T::DESCRIPTION, Arc::new(schema_of::<T::Input>()))
        .with_raw_output_schema(Arc::new(schema_of::<T::Output>()))
        .with_annotations(annotations)
}

/// All tools, in the order `tools/list` returns them.
pub fn all_definitions() -> Vec<Tool> {
    vec![
        definition::<get_engine_info::GetEngineInfo>(),
        definition::<download_packages::DownloadPackages>(),
        definition::<check_updates::CheckUpdates>(),
        definition::<get_catalog::GetCatalog>(),
        definition::<compare_boards::CompareBoards>(),
        definition::<lint::Lint>(),
        definition::<test::Test>(),
        definition::<simulate::Simulate>(),
        definition::<format::Format>(),
        definition::<diagram::Diagram>(),
        definition::<build::Build>(),
        definition::<clean_build::CleanBuild>(),
        definition::<list_devices::ListDevices>(),
        definition::<program_board::ProgramBoard>(),
        definition::<install_driver::InstallDriver>(),
        definition::<uninstall_driver::UninstallDriver>(),
    ]
}

/// Parses a tool's arguments. A request that doesn't match the input schema
/// is a request error that names the problem, so the caller can fix it.
pub fn parse_input<T: EngineTool>(arguments: Option<JsonObject>) -> Result<T::Input, ToolError> {
    let value = serde_json::Value::Object(arguments.unwrap_or_default());
    serde_json::from_value(value)
        .map_err(|e| ToolError::request(format!("Invalid input for the {} tool: {e}.", T::NAME)))
}

/// Turns a tool's result into an MCP tool result: the output or the error
/// object as structured content, with the error flag set for errors, and
/// paths named as the request's display paths say.
pub fn to_call_result<O: Serialize>(
    display: &crate::display_paths::Display,
    result: Result<O, ToolError>,
) -> CallToolResult {
    let (mut value, is_error) = match result {
        Ok(output) => match serde_json::to_value(output) {
            Ok(v) => (v, false),
            Err(e) => (
                to_value(&ToolError::engine(format!(
                    "Can't encode the tool output: {e}"
                ))),
                true,
            ),
        },
        Err(err) => (to_value(&err), true),
    };
    display.json(&mut value);
    if is_error {
        log::info!("Tool error: {value}");
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}

fn to_value(err: &ToolError) -> serde_json::Value {
    serde_json::to_value(err).unwrap_or_else(|_| serde_json::json!({"category": "engine"}))
}

/// Puts the commands a failed call ran into its error's data, as
/// data.commands, so a failure can be reproduced without the engine.
fn with_commands<O>(result: Result<O, ToolError>, call: &Call) -> Result<O, ToolError> {
    result.map_err(|mut err| {
        let commands = call.commands.take();
        if !commands.is_empty() {
            err.data.get_or_insert_with(Default::default).insert(
                "commands".into(),
                serde_json::to_value(commands).unwrap_or_default(),
            );
        }
        err
    })
}

/// Runs a tool by name. Returns None for an unknown tool.
pub async fn call(
    state: &EngineState,
    name: &str,
    arguments: Option<JsonObject>,
    call: &Call,
) -> Option<CallToolResult> {
    use build::Build;
    use check_updates::CheckUpdates;
    use clean_build::CleanBuild;
    use compare_boards::CompareBoards;
    use diagram::Diagram;
    use download_packages::DownloadPackages;
    use format::Format;
    use get_catalog::GetCatalog;
    use get_engine_info::GetEngineInfo;
    use install_driver::InstallDriver;
    use lint::Lint;
    use list_devices::ListDevices;
    use program_board::ProgramBoard;
    use simulate::Simulate;
    use test::Test;
    use uninstall_driver::UninstallDriver;
    // Paths named as the request says: in the live output now, in the
    // result below.
    let display = match crate::display_paths::Display::from_arguments(arguments.as_ref()) {
        Ok(d) => Arc::new(d),
        Err(e) => return Some(to_call_result::<()>(&Default::default(), Err(e))),
    };
    let named = Call {
        progress: call.progress.clone(),
        output: Arc::new(crate::display_paths::DisplayOutput {
            inner: call.output.clone(),
            display: display.clone(),
        }),
        cancel: call.cancel.clone(),
        commands: call.commands.clone(),
        display: display.clone(),
    };
    let call = &named;
    let display = &*display;
    // Every tool that may use packages reads the remote config first when
    // it's due (24 hours), so it works with the releases the config names.
    // Best effort: a tool that needs a package and has none to use gets the
    // error from needing_download; one that doesn't, such as get_catalog
    // with a local catalog folder, works anyway. download_packages and
    // check_updates read it themselves (they may use the network even with
    // automatic download off), and get_engine_info and clean_build use no
    // packages.
    if ![
        GetEngineInfo::NAME,
        CleanBuild::NAME,
        DownloadPackages::NAME,
        CheckUpdates::NAME,
    ]
    .contains(&name)
        && all_definitions().iter().any(|t| t.name == name)
        && let Err(e) = state.packages.refresh(false, false).await
    {
        log::info!("Before {name}: {}", e.message);
    }
    // The tools that read boards load the catalog first.
    if [
        GetCatalog::NAME,
        CompareBoards::NAME,
        Build::NAME,
        Lint::NAME,
        Diagram::NAME,
        ListDevices::NAME,
        ProgramBoard::NAME,
        InstallDriver::NAME,
        UninstallDriver::NAME,
    ]
    .contains(&name)
        && let Err(e) = state.ensure_catalog(call).await
    {
        return Some(to_call_result::<()>(display, with_commands(Err(e), call)));
    }
    let result = match name {
        GetEngineInfo::NAME => to_call_result(
            display,
            with_commands(
                parse_input::<GetEngineInfo>(arguments)
                    .and_then(|i| get_engine_info::run(state, i)),
                call,
            ),
        ),
        GetCatalog::NAME => to_call_result(
            display,
            with_commands(
                parse_input::<GetCatalog>(arguments).and_then(|i| get_catalog::run(state, i)),
                call,
            ),
        ),
        CompareBoards::NAME => to_call_result(
            display,
            with_commands(
                parse_input::<CompareBoards>(arguments).and_then(compare_boards::run),
                call,
            ),
        ),
        Build::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<Build>(arguments) {
                    Ok(input) => build::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        Lint::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<Lint>(arguments) {
                    Ok(input) => lint::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        Test::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<Test>(arguments) {
                    Ok(input) => test::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        Simulate::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<Simulate>(arguments) {
                    Ok(input) => simulate::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        Format::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<Format>(arguments) {
                    Ok(input) => format::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        Diagram::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<Diagram>(arguments) {
                    Ok(input) => diagram::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        CleanBuild::NAME => to_call_result(
            display,
            with_commands(
                parse_input::<CleanBuild>(arguments).and_then(|i| clean_build::run(state, i)),
                call,
            ),
        ),
        ListDevices::NAME => to_call_result(
            display,
            with_commands(
                parse_input::<ListDevices>(arguments).and_then(|i| list_devices::run(state, i)),
                call,
            ),
        ),
        ProgramBoard::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<ProgramBoard>(arguments) {
                    Ok(input) => program_board::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        InstallDriver::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<InstallDriver>(arguments) {
                    Ok(input) => install_driver::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        UninstallDriver::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<UninstallDriver>(arguments) {
                    Ok(input) => uninstall_driver::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        DownloadPackages::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<DownloadPackages>(arguments) {
                    Ok(input) => download_packages::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        CheckUpdates::NAME => to_call_result(
            display,
            with_commands(
                match parse_input::<CheckUpdates>(arguments) {
                    Ok(input) => check_updates::run(state, input, call).await,
                    Err(e) => Err(e),
                },
                call,
            ),
        ),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_object_schemas_and_a_description() {
        for tool in all_definitions() {
            assert_eq!(
                tool.input_schema.get("type"),
                Some(&"object".into()),
                "{}",
                tool.name
            );
            let out = tool.output_schema.as_ref().expect("output schema");
            assert_eq!(out.get("type"), Some(&"object".into()), "{}", tool.name);
            assert!(tool.description.as_deref().is_some_and(|d| d.len() > 40));
        }
    }

    #[test]
    fn schemas_use_only_standard_formats() {
        let all = serde_json::to_string(&all_definitions()).unwrap();
        for f in ["uint", "int8", "int16", "int32", "int64", "double", "float"] {
            assert!(!all.contains(&format!("\"format\":\"{f}")), "format {f}");
        }
    }

    #[test]
    fn unknown_input_fields_are_request_errors() {
        let mut args = JsonObject::new();
        args.insert("bogus".into(), 1.into());
        let err = parse_input::<get_engine_info::GetEngineInfo>(Some(args)).unwrap_err();
        assert_eq!(err.category, crate::errors::Category::Request);
        assert!(err.message.contains("bogus"), "{}", err.message);
    }
}
