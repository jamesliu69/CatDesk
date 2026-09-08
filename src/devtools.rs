use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};

use crate::browser::DetectedBrowser;

const DEVTOOLS_PROTOCOL_VERSION: &str = "2025-03-26";
const CHROME_DEVTOOLS_MCP_VERSION: &str = "1.8.0";
const STDERR_DIAGNOSTIC_MAX_LINES: usize = 32;
const STDERR_DIAGNOSTIC_MAX_CHARS_PER_LINE: usize = 512;
const DEVTOOLS_CLIENT_NAME: &str = "catdesk-bridge";
const DEVTOOLS_CLIENT_VERSION: &str = "4.0.0";
const DEVTOOLS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

type PendingResponse = Result<Value, String>;
type PendingRequests = Arc<Mutex<HashMap<Value, oneshot::Sender<PendingResponse>>>>;
type DiagnosticBuffer = Arc<Mutex<VecDeque<String>>>;

/// A running chrome-devtools-mcp child process with stdin/stdout JSON-RPC bridge.
pub struct DevtoolsBridge {
    child: Mutex<Child>,
    stdin: Mutex<tokio::io::BufWriter<tokio::process::ChildStdin>>,
    pending: PendingRequests,
    diagnostics: DiagnosticBuffer,
}

impl DevtoolsBridge {
    /// Spawn the pinned `chrome-devtools-mcp` package and set up the stdio bridge.
    pub async fn start(selected_browser: Option<&DetectedBrowser>) -> Result<Arc<Self>, String> {
        let mut command = Command::new("npx");
        let package = format!("chrome-devtools-mcp@{CHROME_DEVTOOLS_MCP_VERSION}");
        command.args(["-y", &package]);

        if let Some(browser) = selected_browser {
            if browser.remote_debug_active {
                if let Some(target) = browser.remote_debug_target.as_deref() {
                    if target == "pipe" {
                        command.args(["--executablePath", &browser.path]);
                    } else {
                        command.args(["--browserUrl", &format!("http://{target}")]);
                    }
                } else {
                    command.args(["--executablePath", &browser.path]);
                }
            } else {
                command.args(["--executablePath", &browser.path]);
            }
        }

        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn chrome-devtools-mcp: {e}"))?;

        let child_stdin = child.stdin.take().ok_or("No stdin")?;
        let child_stdout = child.stdout.take().ok_or("No stdout")?;
        let child_stderr = child.stderr.take().ok_or("No stderr")?;
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics = Arc::new(Mutex::new(VecDeque::new()));

        let bridge = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(tokio::io::BufWriter::new(child_stdin)),
            pending: pending.clone(),
            diagnostics: diagnostics.clone(),
        });

        Self::spawn_stdout_reader(child_stdout, pending, diagnostics.clone());
        Self::spawn_stderr_reader(child_stderr, diagnostics);

        let init_req = json!({
            "jsonrpc": "2.0",
            "id": "dt-init",
            "method": "initialize",
            "params": {
                "protocolVersion": DEVTOOLS_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": DEVTOOLS_CLIENT_NAME,
                    "version": DEVTOOLS_CLIENT_VERSION
                }
            }
        });
        if let Err(error) = bridge.request(&init_req).await {
            return Err(bridge.with_diagnostics(error).await);
        }
        bridge
            .notify(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .await?;

        Ok(bridge)
    }

    fn spawn_stdout_reader(
        child_stdout: tokio::process::ChildStdout,
        pending: PendingRequests,
        diagnostics: DiagnosticBuffer,
    ) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        let error = Self::diagnostic_message(
                            "chrome-devtools-mcp stdout closed",
                            &diagnostics,
                        )
                        .await;
                        Self::fail_all_pending(&pending, error).await;
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(msg) = serde_json::from_str::<Value>(trimmed)
                            && let Some(id) = msg.get("id").cloned()
                        {
                            let sender = {
                                let mut map = pending.lock().await;
                                map.remove(&id)
                            };
                            if let Some(tx) = sender {
                                let _ = tx.send(Ok(msg));
                            }
                        }
                    }
                    Err(error) => {
                        let error = Self::diagnostic_message(
                            &format!("chrome-devtools-mcp stdout read failed: {error}"),
                            &diagnostics,
                        )
                        .await;
                        Self::fail_all_pending(&pending, error).await;
                        break;
                    }
                }
            }
        });
    }

    fn spawn_stderr_reader(
        child_stderr: tokio::process::ChildStderr,
        diagnostics: DiagnosticBuffer,
    ) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => Self::push_diagnostic(&diagnostics, line.trim_end()).await,
                }
            }
        });
    }

    async fn push_diagnostic(diagnostics: &DiagnosticBuffer, line: &str) {
        if line.is_empty() {
            return;
        }
        let truncated = line
            .chars()
            .take(STDERR_DIAGNOSTIC_MAX_CHARS_PER_LINE)
            .collect::<String>();
        let mut lines = diagnostics.lock().await;
        lines.push_back(truncated);
        while lines.len() > STDERR_DIAGNOSTIC_MAX_LINES {
            lines.pop_front();
        }
    }

    async fn diagnostic_message(base: &str, diagnostics: &DiagnosticBuffer) -> String {
        let lines = diagnostics.lock().await;
        if lines.is_empty() {
            base.to_string()
        } else {
            format!(
                "{base}\nchrome-devtools-mcp stderr:\n{}",
                lines.iter().cloned().collect::<Vec<_>>().join("\n")
            )
        }
    }

    async fn with_diagnostics(&self, base: String) -> String {
        Self::diagnostic_message(&base, &self.diagnostics).await
    }

    async fn fail_all_pending(pending: &PendingRequests, error: String) {
        let senders = {
            let mut map = pending.lock().await;
            map.drain().map(|(_, sender)| sender).collect::<Vec<_>>()
        };
        for sender in senders {
            let _ = sender.send(Err(error.clone()));
        }
    }

    async fn write_message(&self, req: &Value) -> Result<(), String> {
        let line = serde_json::to_string(req).map_err(|e| e.to_string())?;
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("stdin write: {e}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| format!("stdin write newline: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("stdin flush: {e}"))?;
        Ok(())
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn request(&self, req: &Value) -> Result<Value, String> {
        let Some(id) = req.get("id").cloned() else {
            self.write_message(req).await?;
            return Ok(Value::Null);
        };

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            if map.contains_key(&id) {
                return Err(format!("Duplicate pending DevTools request id: {id}"));
            }
            map.insert(id.clone(), tx);
        }

        if let Err(error) = self.write_message(req).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        match tokio::time::timeout(DEVTOOLS_REQUEST_TIMEOUT, rx).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err("Response channel closed".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(self
                    .with_diagnostics("Request timed out (120s)".into())
                    .await)
            }
        }
    }

    /// Send a notification (no id, no response expected).
    pub async fn notify(&self, req: &Value) -> Result<(), String> {
        self.write_message(req).await
    }

    /// Kill the child process.
    #[allow(dead_code)]
    pub async fn stop(&self) {
        let _ = self.child.lock().await.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_registers_pending_before_writing_to_child() {
        let source = include_str!("devtools.rs");
        let request_start = source
            .find("pub async fn request")
            .expect("request implementation");
        let request_end = source[request_start..]
            .find("pub async fn notify")
            .map(|offset| request_start + offset)
            .expect("notify implementation");
        let request = &source[request_start..request_end];

        let insert = request
            .find("map.insert")
            .expect("pending request registration");
        let write = request
            .find("if let Err(error) = self.write_message")
            .expect("request child stdin write");

        assert!(
            insert < write,
            "request id must be registered before bytes are sent to the child"
        );
    }

    #[test]
    fn chrome_devtools_mcp_launch_is_version_pinned() {
        let package = format!("chrome-devtools-mcp@{CHROME_DEVTOOLS_MCP_VERSION}");
        assert_eq!(package, "chrome-devtools-mcp@1.8.0");
        assert!(!package.contains("@latest"));
    }

    #[tokio::test]
    async fn stderr_diagnostics_are_bounded() {
        let diagnostics: DiagnosticBuffer = Arc::new(Mutex::new(VecDeque::new()));
        for index in 0..(STDERR_DIAGNOSTIC_MAX_LINES + 5) {
            DevtoolsBridge::push_diagnostic(&diagnostics, &format!("line-{index}")).await;
        }
        let lines = diagnostics.lock().await;
        assert_eq!(lines.len(), STDERR_DIAGNOSTIC_MAX_LINES);
        assert_eq!(lines.front().map(String::as_str), Some("line-5"));
    }

    #[tokio::test]
    async fn failing_bridge_drains_pending_requests_immediately() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(json!(7), tx);

        DevtoolsBridge::fail_all_pending(&pending, "bridge closed".to_string()).await;

        let response = rx.await.expect("pending sender must resolve");
        assert_eq!(response.unwrap_err(), "bridge closed");
        assert!(pending.lock().await.is_empty());
    }
}
