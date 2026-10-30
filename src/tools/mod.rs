//! The engine's MCP tools, from docs/tool-api.md. Each tool has an input
//! type and an output type. Their JSON schemas are generated from the Rust
//! types, so the code, the validation and the exported schemas agree.

pub mod get_engine_info;

use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::errors::ToolError;
use crate::state::EngineState;

/// A tool's name, description and types.
pub trait EngineTool {
    const NAME: &'static str;
    /// What the tool does, for hosts and AI agents that learn the tools from
    /// their descriptions alone.
    const DESCRIPTION: &'static str;
    /// True if the tool changes nothing, on disk or on a board.
    const READ_ONLY: bool;
    type Input: DeserializeOwned + JsonSchema;
    type Output: Serialize + JsonSchema;
}

/// The JSON schema of a type, as an MCP schema object.
pub fn schema_of<T: JsonSchema>() -> JsonObject {
    match serde_json::to_value(schemars::schema_for!(T)) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => unreachable!("a derived schema is a JSON object"),
    }
}

/// The MCP definition of a tool, as `tools/list` returns it.
pub fn definition<T: EngineTool>() -> Tool {
    let mut annotations = ToolAnnotations::default();
    annotations.read_only_hint = Some(T::READ_ONLY);
    annotations.destructive_hint = Some(false);
    annotations.open_world_hint = Some(false);
    Tool::new(T::NAME, T::DESCRIPTION, Arc::new(schema_of::<T::Input>()))
        .with_raw_output_schema(Arc::new(schema_of::<T::Output>()))
        .with_annotations(annotations)
}

/// All tools, in the order `tools/list` returns them.
pub fn all_definitions() -> Vec<Tool> {
    vec![definition::<get_engine_info::GetEngineInfo>()]
}

/// Parses a tool's arguments. A request that doesn't match the input schema
/// is a request error that names the problem, so the caller can fix it.
pub fn parse_input<T: EngineTool>(arguments: Option<JsonObject>) -> Result<T::Input, ToolError> {
    let value = serde_json::Value::Object(arguments.unwrap_or_default());
    serde_json::from_value(value)
        .map_err(|e| ToolError::request(format!("Invalid input for the {} tool: {e}.", T::NAME)))
}

/// Turns a tool's result into an MCP tool result: the output or the error
/// object as structured content, with the error flag set for errors.
pub fn to_call_result<O: Serialize>(result: Result<O, ToolError>) -> CallToolResult {
    let (value, is_error) = match result {
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

/// Runs a tool by name. Returns None for an unknown tool.
pub async fn call(
    state: &EngineState,
    name: &str,
    arguments: Option<JsonObject>,
) -> Option<CallToolResult> {
    use get_engine_info::GetEngineInfo;
    let result = match name {
        GetEngineInfo::NAME => to_call_result(
            parse_input::<GetEngineInfo>(arguments).and_then(|i| get_engine_info::run(state, i)),
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
    fn unknown_input_fields_are_request_errors() {
        let mut args = JsonObject::new();
        args.insert("bogus".into(), 1.into());
        let err = parse_input::<get_engine_info::GetEngineInfo>(Some(args)).unwrap_err();
        assert_eq!(err.category, crate::errors::Category::Request);
        assert!(err.message.contains("bogus"), "{}", err.message);
    }
}
