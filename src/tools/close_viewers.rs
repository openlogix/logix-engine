//! The close_viewers tool (docs/tool-api.md, close_viewers): closes the
//! windows this engine opened in Logix Viewer's apps (Logix Waveforms,
//! Logix Diagrams), never another engine's. The windows also close when
//! the engine exits.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct CloseViewers;

impl EngineTool for CloseViewers {
    const NAME: &'static str = "close_viewers";
    const DESCRIPTION: &'static str = "Closes windows this engine opened for the person at \
        the screen (simulate, open_waveform and diagram, in Logix Waveforms and Logix \
        Diagrams): one page's window, every window of one kind, or all of them. Never \
        touches windows another engine opened. Views are saved as they change, so nothing \
        is lost. The windows also close when the engine exits.";
    const READ_ONLY: bool = false;
    type Input = CloseViewersInput;
    type Output = CloseViewersOutput;
}

/// Which windows to close; omitted fields mean all.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseViewersInput {
    /// The kind of page: "waveforms" or "diagram". Omitted means every kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// One page's window, by the page's path as the tool that wrote it
    /// returned it (absolute, such as /home/user/proj/_build/main_tb.html).
    /// Omitted means every page.
    #[serde(default)]
    pub page: Option<String>,
}

/// What closed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseViewersOutput {
    /// How many windows closed.
    pub closed: usize,
}

pub fn run(
    _state: &EngineState,
    input: CloseViewersInput,
) -> Result<CloseViewersOutput, ToolError> {
    if let Some(p) = &input.page
        && !Path::new(p).is_absolute()
    {
        return Err(ToolError::request(format!(
            "page: {p:?} must be an absolute path."
        )));
    }
    let close =
        || crate::viewer::close(input.kind.as_deref(), input.page.as_deref().map(Path::new));
    let closed = match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(close)
        }
        _ => close(),
    };
    Ok(CloseViewersOutput { closed })
}
