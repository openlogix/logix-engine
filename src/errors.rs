//! The error model from docs/tool-api.md ("Results and errors"). A tool that
//! can't do its job returns a `ToolError`, sent to the host as the structured
//! content of an MCP tool result marked as an error.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whose problem the error is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    /// A problem in the user's design files, such as a syntax error.
    Design,
    /// Something missing or wrong on the computer, such as a package that
    /// isn't installed or a board that isn't connected.
    Setup,
    /// The host sent an invalid request, such as an unknown board or a
    /// missing field. Fix the request and retry.
    Request,
    /// A packaged tool crashed or misbehaved.
    Tool,
    /// A bug in the engine itself.
    Engine,
    /// The host cancelled the operation.
    Cancelled,
}

/// A problem reported by a tool, at a file and line when known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostic {
    /// File, relative to the source folder, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Line number, starting at 1, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub line: Option<u32>,
    /// Column number, starting at 1, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub column: Option<u32>,
    pub severity: Severity,
    /// The problem, in words.
    pub text: String,
    /// Name of the tool that reported it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A fix the host can offer the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionCode {
    /// Required packages are missing or broken and weren't installed
    /// automatically, because auto-install is off or the install failed.
    InstallPackages,
    /// No matching board is connected.
    ConnectBoard,
    /// Several devices match. Pick one and retry with its serial number.
    SelectDevice,
    /// A device was found but cannot be opened.
    FixDevicePermissions,
    /// A driver required by the board is missing.
    InstallDrivers,
}

/// A suggested action, with a code for the host and a description for people.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Action {
    pub code: ActionCode,
    /// The suggested action, in words.
    pub description: String,
}

/// Why a tool could not do its job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolError {
    pub category: Category,
    /// Short message for end users.
    pub message: String,
    /// Troubleshooting details, such as tool output. May span several lines.
    #[serde(default)]
    pub details: String,
    /// Problems with file and line, mainly for design errors.
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// A suggested action, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    /// Extra data, such as candidate devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Map<String, serde_json::Value>>,
}

impl ToolError {
    pub fn new(category: Category, message: impl Into<String>) -> Self {
        ToolError {
            category,
            message: message.into(),
            details: String::new(),
            diagnostics: Vec::new(),
            action: None,
            data: None,
        }
    }

    pub fn request(message: impl Into<String>) -> Self {
        Self::new(Category::Request, message)
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::new(Category::Engine, message)
    }

    pub fn setup(message: impl Into<String>) -> Self {
        Self::new(Category::Setup, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(Category::Cancelled, message)
    }

    pub fn with_action(mut self, code: ActionCode, description: impl Into<String>) -> Self {
        self.action = Some(Action {
            code,
            description: description.into(),
        });
        self
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = crate::text::to_multiline_text(&details.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_with_the_documented_field_names() {
        let mut err = ToolError::new(Category::Setup, "Found 2 matching devices.");
        err.action = Some(Action {
            code: ActionCode::SelectDevice,
            description: "Choose one device and retry with its serial number.".into(),
        });
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({
                "category": "setup",
                "message": "Found 2 matching devices.",
                "details": "",
                "diagnostics": [],
                "action": {
                    "code": "select_device",
                    "description": "Choose one device and retry with its serial number."
                }
            })
        );
    }

    #[test]
    fn all_categories_and_action_codes_use_the_documented_names() {
        let names: Vec<_> = [
            Category::Design,
            Category::Setup,
            Category::Request,
            Category::Tool,
            Category::Engine,
            Category::Cancelled,
        ]
        .iter()
        .map(|c| serde_json::to_value(c).unwrap())
        .collect();
        assert_eq!(
            names,
            json!(["design", "setup", "request", "tool", "engine", "cancelled"])
                .as_array()
                .unwrap()
                .clone()
        );
        let codes: Vec<_> = [
            ActionCode::InstallPackages,
            ActionCode::ConnectBoard,
            ActionCode::SelectDevice,
            ActionCode::FixDevicePermissions,
            ActionCode::InstallDrivers,
        ]
        .iter()
        .map(|c| serde_json::to_value(c).unwrap())
        .collect();
        assert_eq!(
            codes,
            json!([
                "install_packages",
                "connect_board",
                "select_device",
                "fix_device_permissions",
                "install_drivers"
            ])
            .as_array()
            .unwrap()
            .clone()
        );
    }

    #[test]
    fn details_are_normalized_to_multiline_text() {
        let err = ToolError::engine("x").with_details("a\r\nb\x1b[31mc\x1b[0m\n");
        assert_eq!(err.details, "a\nbc");
    }
}
