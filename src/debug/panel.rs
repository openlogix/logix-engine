//! open_debug_panel: an interactive page for a debug session, served by
//! Logix Engine on 127.0.0.1 and opened in the browser. It shows the watched
//! nets live and drives and releases the driven ones, through the same sessions as
//! the debug tools.
//!
//! Only this computer can reach the server (it listens on 127.0.0.1), and
//! every request must carry the server's random token and a 127.0.0.1 or
//! localhost Host header, so other web pages can't drive the board.

use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;

use crate::errors::ToolError;
use crate::tools::EngineTool;

use super::session;

const PAGE: &str = include_str!("panel.html");
/// The server's name, which the page shows if it loses the server.
const SERVER_NAME: &str = "Logix Engine";
/// The largest request the server reads.
const MAX_REQUEST: usize = 64 * 1024;

struct Server {
    port: u16,
    token: String,
}

static SERVER: OnceCell<Server> = OnceCell::const_new();

async fn server() -> Result<&'static Server, ToolError> {
    SERVER
        .get_or_try_init(|| async {
            let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| {
                ToolError::setup(format!("Can't start the debug panel's server: {e}"))
            })?;
            let port = listener
                .local_addr()
                .map_err(|e| ToolError::engine(format!("The panel's server has no port: {e}")))?
                .port();
            let token = new_token();
            let t = token.clone();
            tokio::spawn(async move {
                loop {
                    if let Ok((stream, _)) = listener.accept().await {
                        let t = t.clone();
                        tokio::spawn(async move {
                            let _ = handle(stream, port, &t).await;
                        });
                    }
                }
            });
            log::info!("Debug panel server on 127.0.0.1:{port}");
            Ok(Server { port, token })
        })
        .await
}

fn new_token() -> String {
    let mut h = Sha256::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    h.update(now.as_nanos().to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let local = 0u8;
    h.update((&local as *const u8 as usize).to_le_bytes());
    hex::encode(&h.finalize()[..16])
}

// --- HTTP -------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    host: String,
    body: Vec<u8>,
}

impl Request {
    fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_REQUEST {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let mut host = String::new();
    let mut length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "host" => host = v.trim().to_string(),
                "content-length" => length = v.trim().parse().ok()?,
                _ => {}
            }
        }
    }
    if length > MAX_REQUEST {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_string(), v.to_string())
        })
        .collect();
    Some(Request {
        method,
        path: path.to_string(),
        query,
        host,
        body,
    })
}

async fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.shutdown().await;
}

async fn respond_json(stream: &mut TcpStream, result: Result<Value, ToolError>) {
    match result {
        Ok(v) => {
            respond(
                stream,
                "200 OK",
                "application/json",
                v.to_string().as_bytes(),
            )
            .await
        }
        Err(e) => {
            let v = serde_json::to_value(&e).unwrap_or_default();
            respond(
                stream,
                "400 Bad Request",
                "application/json",
                json!({ "error": v }).to_string().as_bytes(),
            )
            .await
        }
    }
}

async fn handle(mut stream: TcpStream, port: u16, token: &str) -> Option<()> {
    let req = read_request(&mut stream).await?;
    let host_ok = [format!("127.0.0.1:{port}"), format!("localhost:{port}")].contains(&req.host);
    if !host_ok || req.param("t") != Some(token) {
        respond(&mut stream, "403 Forbidden", "text/plain", b"Forbidden").await;
        return Some(());
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            respond(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                PAGE.as_bytes(),
            )
            .await
        }
        ("GET", "/api/about") => {
            // For the page to name its server once it can't reach it.
            respond_json(&mut stream, Ok(json!({ "name": SERVER_NAME }))).await
        }
        ("GET", "/api/sessions") => {
            respond_json(&mut stream, Ok(json!({ "sessions": session::ids() }))).await
        }
        ("GET", "/api/state") => {
            let id = req.param("session").unwrap_or("").to_string();
            let read_values = req.param("read") != Some("0");
            let result = tokio::task::spawn_blocking(move || state(&id, read_values))
                .await
                .unwrap_or_else(|e| {
                    Err(ToolError::engine(format!("The panel's read failed: {e}")))
                });
            respond_json(&mut stream, result).await
        }
        ("POST", "/api/write") => {
            let parsed: Result<session::DebugWriteInput, _> = serde_json::from_slice(&req.body);
            let result = match parsed {
                Ok(input) => tokio::task::spawn_blocking(move || {
                    session::write(input).map(|o| serde_json::to_value(o).unwrap_or_default())
                })
                .await
                .unwrap_or_else(|e| {
                    Err(ToolError::engine(format!("The panel's write failed: {e}")))
                }),
                Err(e) => Err(ToolError::request(format!("Bad write request: {e}"))),
            };
            respond_json(&mut stream, result).await
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"Not found").await,
    }
    Some(())
}

/// The session's build and driven nets, and a fresh snapshot of the
/// watched ones.
fn state(id: &str, read_values: bool) -> Result<Value, ToolError> {
    let info = session::info(id)?;
    let values = if read_values && !info.watch.is_empty() {
        session::read(session::DebugReadInput {
            session: id.to_string(),
            names: None,
        })?
        .values
    } else {
        Vec::new()
    };
    Ok(json!({ "info": info, "values": values }))
}

// --- The tool ---------------------------------------------------------------

pub struct DebugPanel;

impl EngineTool for DebugPanel {
    const NAME: &'static str = "open_debug_panel";
    const DESCRIPTION: &'static str = "Opens an interactive panel for a debug session in the \
        browser: it shows the watched nets live (as hex, decimal and bits) and has controls \
        to drive each driven net and release it. Logix Engine serves it on this computer only, at \
        127.0.0.1, with a random token in its address. The panel uses the same session as \
        read_debug and write_debug, so both can be used at once. Returns the address; with \
        open false it only returns it, such as for a host that shows pages itself.";
    const READ_ONLY: bool = false;
    type Input = DebugPanelInput;
    type Output = DebugPanelOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugPanelInput {
    /// The session, from open_debug.
    pub session: String,
    /// Open the panel in the default browser. Defaults to true.
    #[serde(default)]
    pub open: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugPanelOutput {
    /// The panel's address. It works while Logix Engine runs, for any open
    /// session (the page can switch between them).
    pub url: String,
    /// True when the browser was asked to open it.
    pub opened: bool,
}

pub async fn run(input: DebugPanelInput) -> Result<DebugPanelOutput, ToolError> {
    session::info(&input.session)?;
    let s = server().await?;
    let url = format!(
        "http://127.0.0.1:{}/?t={}&session={}",
        s.port, s.token, input.session
    );
    let opened = input.open.unwrap_or(true) && open_browser(&url);
    Ok(DebugPanelOutput { url, opened })
}

fn open_browser(url: &str) -> bool {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        // Not cmd's start, which splits the address at its &.
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}
