//! Sends an operation's progress and live tool output to the host, during
//! the tool call:
//!
//! - Progress as MCP progress notifications, when the request has a progress
//!   token.
//! - Each tool output line as a `logix/output` notification, when the
//!   request's `_meta` has `"logix/output": true` (docs/tool-api.md, "Live
//!   tool output").
//!
//! Both go through one task, so they arrive in order, and all of them before
//! the tool result.

use std::sync::Mutex;

use rmcp::RoleServer;
use rmcp::model::{
    CustomNotification, ProgressNotificationParam, ProgressToken, RequestId, ServerNotification,
};
use rmcp::service::Peer;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::progress::{OutputLevel, OutputSink, ProgressSink};

/// The notification method for live tool output.
pub const OUTPUT_METHOD: &str = "logix/output";

enum Note {
    Progress(ProgressNotificationParam),
    Output(serde_json::Value),
}

pub struct McpNotifier {
    sender: Mutex<Option<mpsc::UnboundedSender<Note>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    progress_token: Option<ProgressToken>,
    /// The request ID, when the host asked for live output.
    output_for: Option<RequestId>,
}

impl McpNotifier {
    /// Starts sending notifications for one request to `peer`.
    pub fn new(
        peer: Peer<RoleServer>,
        progress_token: Option<ProgressToken>,
        output_for: Option<RequestId>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Note>();
        let task = tokio::spawn(async move {
            while let Some(note) = receiver.recv().await {
                let result = match note {
                    Note::Progress(p) => peer.notify_progress(p).await,
                    Note::Output(params) => {
                        let n = CustomNotification::new(OUTPUT_METHOD, Some(params));
                        peer.send_notification(ServerNotification::CustomNotification(n))
                            .await
                    }
                };
                if let Err(e) = result {
                    log::debug!("Can't send a notification: {e}");
                }
            }
        });
        McpNotifier {
            sender: Mutex::new(Some(sender)),
            task: Mutex::new(Some(task)),
            progress_token,
            output_for,
        }
    }

    fn send(&self, note: Note) {
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            let _ = sender.send(note);
        }
    }

    /// Sends what was reported so far and stops. Called before the tool
    /// result goes out, so nothing arrives after it.
    pub async fn finish(&self) {
        self.sender.lock().unwrap().take();
        let task = self.task.lock().unwrap().take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

impl ProgressSink for McpNotifier {
    fn report(&self, progress: f64, total: f64, message: Option<String>) {
        if let Some(m) = &message {
            log::info!("Progress: {m}");
        }
        let Some(token) = &self.progress_token else {
            return;
        };
        let mut param = ProgressNotificationParam::new(token.clone(), progress).with_total(total);
        if let Some(m) = message {
            param = param.with_message(m);
        }
        self.send(Note::Progress(param));
    }
}

impl OutputSink for McpNotifier {
    fn line(&self, tool: &str, level: OutputLevel, text: &str) {
        let Some(id) = &self.output_for else { return };
        self.send(Note::Output(json!({
            "request_id": id,
            "tool": tool,
            "kind": "output",
            "level": level,
            "text": text,
        })));
    }

    fn command(&self, tool: &str, text: &str) {
        let Some(id) = &self.output_for else { return };
        self.send(Note::Output(json!({
            "request_id": id,
            "tool": tool,
            "kind": "command",
            "level": OutputLevel::Info,
            "text": text,
        })));
    }
}
