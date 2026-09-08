use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};

use crate::browser::DetectedBrowser;

const DEVTOOLS_PROTOCOL_VERSION: &str = "2025-03-26";
const DEVTOOLS_CLIENT_NAME: &str = "catdesk-bridge";
const DEVTOOLS_CLIENT_VERSION: &str = "4.0.0";
const DEVTOOLS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

type PendingResponse = Result<Value, String>;
type PendingRequests = Arc<Mutex<HashMap<Value, oneshot::Sender<PendingResponse>>>>;

/// A running chrome-devtools-mcp child process with stdin/stdout JSON-RPC bridge.
pub struct DevtoolsBridge {
    child: Mutex<Child>,
    stdin: Mutex<tokio::io::BufWriter<tokio::process::ChildStdin>>,
    pending: PendingRequests,
}

impl DevtoolsBridge {
    /// Spawn `npx chrome-devtools-mcp@latest` and set up stdio bridge.
    pub async fn start(selected_browser: Option<&DetectedBrowser>) -> Result<Arc<Self>, String> {
        let mut command = Command::new("npx");
        command.args(["-y", "chrome-devtools-mcp@latest"]);

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
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn chrome-devtools-mcp: {e}"))?;

        let child_stdin = child.stdin.take().ok_or("No stdin")?;
        let child_stdout = child.stdout.take().ok_or("No stdout")?;
        let pending = Arc::new(Mutex::new(HashMap::new()));

        let bridge = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(tokio::io::BufWriter::new(child_stdin)),
            pending: pending.clone(),
        });

        Self::spawn_stdout_reader(child_stdout, pending);

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
        bridge.request(&init_req).await?;
        bridge
            .notify(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .await?;

        Ok(bridge)
    }

    fn spawn_stdout_reader(child_stdout: tokio::process::ChildStdout, pending: PendingRequests) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        Self::fail_all_pending(
                            &pending,
                            "chrome-devtools-mcp stdout closed".to_string(),
                        )
                        .await;
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(msg) = serde_json::from_str::<Value>(trimmed) {
                            if let Some(id) = msg.get("id").cloned() {
                                let sender = {
                                    let mut map = pending.lock().await;
                                    map.remove(&id)
                                };
                                if let Some(tx) = sender {
                                    let _ = tx.send(Ok(msg));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        Self::fail_all_pending(
                            &pending,
                            format!("chrome-devtools-mcp stdout read failed: {error}"),
                        )
                        .await;
                        break;
                    }
                }
            }
        });
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
                Err("Request timed out (120s)".into())
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
