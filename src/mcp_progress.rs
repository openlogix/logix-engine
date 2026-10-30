//! Sends an operation's progress to the host as MCP progress notifications,
//! when the host asked for progress with a progress token. Notifications go
//! through one task, so they arrive in order.

use std::sync::Mutex;

use rmcp::RoleServer;
use rmcp::model::{ProgressNotificationParam, ProgressToken};
use rmcp::service::Peer;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::progress::ProgressSink;

pub struct McpProgress {
    sender: Mutex<Option<mpsc::UnboundedSender<ProgressNotificationParam>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    token: ProgressToken,
}

impl McpProgress {
    /// Starts forwarding progress for one request to `peer`.
    pub fn new(peer: Peer<RoleServer>, token: ProgressToken) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<ProgressNotificationParam>();
        let task = tokio::spawn(async move {
            while let Some(param) = receiver.recv().await {
                if let Err(e) = peer.notify_progress(param).await {
                    log::debug!("Can't send progress: {e}");
                }
            }
        });
        McpProgress {
            sender: Mutex::new(Some(sender)),
            task: Mutex::new(Some(task)),
            token,
        }
    }

    /// Sends the progress reported so far and stops. Called before the tool
    /// result goes out, so no progress arrives after it.
    pub async fn finish(&self) {
        self.sender.lock().unwrap().take();
        let task = self.task.lock().unwrap().take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

impl ProgressSink for McpProgress {
    fn report(&self, progress: f64, total: f64, message: Option<String>) {
        if let Some(m) = &message {
            log::info!("Progress: {m}");
        }
        let mut param =
            ProgressNotificationParam::new(self.token.clone(), progress).with_total(total);
        if let Some(m) = message {
            param = param.with_message(m);
        }
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            let _ = sender.send(param);
        }
    }
}
