//! The MCP server: one session over stdin/stdout, with the engine's tools.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::mcp_progress::{McpNotifier, OUTPUT_METHOD};
use crate::progress::{NoOutput, NoProgress};
use crate::state::{self, EngineState};
use crate::tools;

pub struct EngineServer {
    state: EngineState,
}

impl EngineServer {
    pub fn new(state: EngineState) -> Self {
        EngineServer { state }
    }
}

/// The server instructions, sent to every client at `initialize`. The
/// default project model in it is only a suggestion for AI sessions with no
/// other requirements, not a standard for frontends and hosts; the engine
/// needs no project file in any format. Full text: docs/tool-api.md,
/// Default project model.
const INSTRUCTIONS: &str = "Logix Engine builds and tests FPGA designs and programs FPGA boards, using open source \
    tools that it downloads into its own folder and runs itself. Every request is self-contained: it names its own \
    board, source folder, source files and build folder, as absolute paths. Errors come back as \
    data with a category (design, setup, request, tool, engine or cancelled), a message and, \
    when there is one, a suggested action. Before writing a design for a board, read the \
    board in get_catalog: its resources (clock, LEDs, buttons and their pins), what designs \
    must drive (requires) and its ai_hints. To show a build's resource use to people, prefer \
    build's report and report_summary text, and each resource's display_percent, over \
    numbers of your own: display_percent is \"0%\" only when none is used and \"100%\" only \
    when all are used, and anything else is rounded up. On Windows, a board whose \
    list_devices driver isn't ready needs WinUSB first: ask the user before calling \
    install_driver (Windows then asks whether Logix Engine may make changes, and other \
    connected devices with the same USB IDs get it too); uninstall_driver undoes it. \
    The engine needs no project file. If your session \
    has no other requirements and the project has no layout of its own, a suggested default \
    project model is: one folder with logix.json (board, board_variant, top_module), \
    main.v, a pinout constraint file, <name>_tb.v testbenches and a _build build folder, as \
    https://github.com/openlogix/logix-engine/blob/main/docs/tool-api.md#default-project-model \
    describes. Full reference, including how to launch the engine and \
    write your own client: tool API https://github.com/openlogix/logix-engine/blob/main/docs/tool-api.md, \
    command line https://github.com/openlogix/logix-engine/blob/main/docs/command-line.md.";

impl ServerHandler for EngineServer {
    fn get_info(&self) -> ServerConfig {
        let mut implementation = Implementation::new("logix-engine", state::ENGINE_VERSION);
        implementation.title = Some("Logix Engine".into());
        implementation.website_url = Some("https://www.openlogix.org/engine/".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(implementation)
            .with_instructions(INSTRUCTIONS)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(state::PROTOCOL_VERSIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tools::all_definitions()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        // Returning None makes rmcp skip its own validation; the engine
        // validates inputs itself, so bad input is a request error the
        // caller can read, not a protocol error.
        let _ = name;
        None
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        log::info!("Tool call: {name}");
        let token = context
            .meta
            .get_progress_token()
            .or_else(|| request.meta.as_ref().and_then(|m| m.get_progress_token()));
        // Live tool output only when the request asks for it.
        let wants_output = [Some(&context.meta), request.meta.as_ref()]
            .into_iter()
            .flatten()
            .any(|m| m.0.0.get(OUTPUT_METHOD) == Some(&serde_json::Value::Bool(true)));
        let notifier = (token.is_some() || wants_output).then(|| {
            Arc::new(McpNotifier::new(
                context.peer.clone(),
                token,
                wants_output.then(|| context.id.clone()),
            ))
        });
        let call = match &notifier {
            Some(n) => tools::Call {
                display: Default::default(),
                progress: n.clone(),
                output: n.clone(),
                cancel: context.ct.clone(),
                commands: Default::default(),
            },
            None => tools::Call {
                display: Default::default(),
                progress: Arc::new(NoProgress),
                output: Arc::new(NoOutput),
                cancel: context.ct.clone(),
                commands: Default::default(),
            },
        };
        let result = tools::call(&self.state, &name, request.arguments, &call).await;
        if let Some(n) = notifier {
            n.finish().await;
        }
        match result {
            Some(result) => Ok(result.into()),
            None => Err(McpError::invalid_params(
                format!("Unknown tool: {name}"),
                None,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::INSTRUCTIONS;

    /// Every docs link in the instructions points to a file, and heading, in this repo's docs.
    #[test]
    fn instructions_link_to_existing_docs() {
        let prefix = "https://github.com/openlogix/logix-engine/blob/main/";
        let links: Vec<&str> = INSTRUCTIONS
            .split_whitespace()
            .filter_map(|word| word.trim_end_matches([',', '.']).strip_prefix(prefix))
            .collect();
        assert_eq!(
            links,
            [
                "docs/tool-api.md#default-project-model",
                "docs/tool-api.md",
                "docs/command-line.md"
            ]
        );
        for link in links {
            let (file, anchor) = link.split_once('#').unwrap_or((link, ""));
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{file} doesn't exist"));
            if !anchor.is_empty() {
                // GitHub's anchor for a heading: lowercase, spaces to dashes.
                let found = text.lines().any(|line| {
                    line.strip_prefix("## ")
                        .is_some_and(|h| h.to_lowercase().replace(' ', "-") == anchor)
                });
                assert!(found, "{file} has no heading for #{anchor}");
            }
        }
    }
}
