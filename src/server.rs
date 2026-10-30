//! The MCP server: one session over stdin/stdout, with the engine's tools.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::mcp_progress::McpProgress;
use crate::progress::{NoProgress, ProgressSink};
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

const INSTRUCTIONS: &str = "Logix Engine builds, tests and programs FPGAs, using open source \
    tools that it installs and runs itself. Every request is self-contained: it names its own \
    board, source folder, source files and build folder, as absolute paths. Errors come back as \
    data with a category (design, setup, request, tool, engine or cancelled), a message and, \
    when there is one, a suggested action.";

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
        let mcp_progress =
            token.map(|token| Arc::new(McpProgress::new(context.peer.clone(), token)));
        let progress: Arc<dyn ProgressSink> = match &mcp_progress {
            Some(p) => p.clone(),
            None => Arc::new(NoProgress),
        };
        let call = tools::Call {
            progress,
            cancel: context.ct.clone(),
        };
        let result = tools::call(&self.state, &name, request.arguments, &call).await;
        if let Some(p) = mcp_progress {
            p.finish().await;
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
