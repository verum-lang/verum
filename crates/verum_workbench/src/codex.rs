//! Codex app-server transport. Authentication stays in the provider process.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, oneshot},
};
use verum_common::{List, Map, Text};

type Pending = Arc<Mutex<Map<u64, oneshot::Sender<Value>>>>;
#[derive(Default)]
pub struct Events {
    pub next: u64,
    pub messages: List<Value>,
    pub requests: Map<Text, Value>,
}
impl Events {
    fn push(&mut self, message: Value) {
        self.next += 1;
        if self.messages.len() == 512 {
            self.messages.remove(0);
        }
        self.messages
            .push(json!({"seq":self.next,"message":message}));
    }
    pub fn read(&self, after: u64) -> Value {
        let events: List<Value> = self
            .messages
            .iter()
            .filter(|e| e["seq"].as_u64().unwrap_or(0) > after)
            .cloned()
            .collect();
        json!({"cursor":self.next,"events":events,"gap":self.messages.first().and_then(|e|e["seq"].as_u64()).is_some_and(|s|after+1<s)})
    }
}
pub struct Codex {
    child: Child,
    writer: Mutex<ChildStdin>,
    pending: Pending,
    pub events: Arc<Mutex<Events>>,
    next: AtomicU64,
    reader: tokio::task::JoinHandle<()>,
    stderr: tokio::task::JoinHandle<()>,
}
impl Drop for Codex {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.reader.abort();
        self.stderr.abort();
    }
}
impl Codex {
    pub async fn connect(executable: &Path, root: &Path) -> Result<Self> {
        let data = dirs::data_local_dir().context("OS application-data directory unavailable")?;
        std::fs::create_dir_all(&data)?;
        let home = crate::workspace::private_dir(&data, &["VerumStudio", "codex"])?;
        Self::connect_with_storage(executable, root, &home).await
    }
    /// Explicit storage also lets protocol tests use disposable provider homes.
    pub async fn connect_with_storage(executable: &Path, root: &Path, home: &Path) -> Result<Self> {
        let mut child = Command::new(executable)
            .args([
                "app-server",
                "--listen",
                "stdio://",
                "-c",
                "forced_login_method=\"chatgpt\"",
                "-c",
                "features.shell_tool=false",
                "-c",
                "features.hooks=false",
                "-c",
                "features.apps=false",
                "-c",
                "features.multi_agent=false",
                "-c",
                "web_search=\"disabled\"",
            ])
            .env("CODEX_HOME", home)
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Start a current Codex CLI with app-server support")?;
        let writer = Mutex::new(child.stdin.take().context("Codex stdin")?);
        let stdout = child.stdout.take().context("Codex stdout")?;
        let stderr = child.stderr.take().context("Codex stderr")?;
        let pending: Pending = Arc::new(Mutex::new(Map::new()));
        let events = Arc::new(Mutex::new(Events::default()));
        let responses = pending.clone();
        let notifications = events.clone();
        let reader = tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = List::<u8>::new();
                // Inspect buffered fragments before allocation; a provider cannot grow
                // an unbounded line while the host waits for its newline.
                let read = async {
                    loop {
                        let buf = stdout.fill_buf().await?;
                        if buf.is_empty() {
                            return Ok::<bool, std::io::Error>(false);
                        }
                        let n = buf
                            .iter()
                            .position(|b| *b == b'\n')
                            .map_or(buf.len(), |i| i + 1);
                        ensure_io(line.len() + n <= 1024 * 1024)?;
                        let end = buf[n - 1] == b'\n';
                        line.extend_from_slice(&buf[..n]);
                        stdout.consume(n);
                        if end {
                            return Ok(true);
                        }
                    }
                }
                .await;
                if !matches!(read, Ok(true)) {
                    break;
                }
                let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                if message.get("method").is_none() {
                    if let Some(id) = message["id"].as_u64()
                        && let Some(sender) = responses.lock().await.remove(&id)
                    {
                        let _ = sender.send(message);
                    }
                } else {
                    let mut events = notifications.lock().await;
                    if message["method"] == "serverRequest/resolved" {
                        events
                            .requests
                            .remove(&Text::from(message["params"]["requestId"].to_string()));
                    }
                    if message["method"] == "turn/completed" {
                        let turn = &message["params"]["turn"]["id"];
                        events
                            .requests
                            .retain(|_, r| &r["params"]["turnId"] != turn);
                    }
                    if let Some(id) = message.get("id") {
                        if events.requests.len() >= 64 {
                            break;
                        }
                        events
                            .requests
                            .insert(id.to_string().into(), message.clone());
                    }
                    events.push(message);
                }
            }
            responses.lock().await.clear();
            let mut events = notifications.lock().await;
            events.requests.clear();
            events.push(json!({"method":"studio/disconnected","params":{"message":"Codex disconnected. Check CLI version and reconnect."}}));
        });
        let stderr_task = tokio::spawn(async move {
            // Drain without persisting provider diagnostics, which may contain account data.
            let _ = tokio::io::copy(&mut BufReader::new(stderr), &mut tokio::io::sink()).await;
        });
        let client = Self {
            child,
            writer,
            pending,
            events,
            next: AtomicU64::new(1),
            reader,
            stderr: stderr_task,
        };
        client.request("initialize",json!({"clientInfo":{"name":"verum_studio","title":"Verum Studio","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        client
            .send(json!({"method":"initialized","params":{}}))
            .await?;
        Ok(client)
    }
    pub async fn send(&self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(&bytes).await?;
        writer.flush().await?;
        Ok(())
    }
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        ensure!(
            !self.reader.is_finished(),
            "Codex is disconnected; reconnect first"
        );
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (send, receive) = oneshot::channel();
        self.pending.lock().await.insert(id, send);
        let result = async {
            self.send(json!({"id":id,"method":method,"params":params}))
                .await?;
            let message = tokio::time::timeout(Duration::from_secs(30), receive)
                .await
                .context("Codex request timed out")?
                .context("Codex disconnected; update the CLI if app-server v2 is unavailable")?;
            if let Some(error) = message.get("error") {
                bail!(
                    "Codex: {}",
                    error["message"].as_str().unwrap_or("protocol error")
                );
            }
            message
                .get("result")
                .cloned()
                .context("Codex response missing result")
        }
        .await;
        self.pending.lock().await.remove(&id);
        result
    }
    pub fn connected(&self) -> bool {
        !self.reader.is_finished()
    }
}
fn ensure_io(valid: bool) -> std::io::Result<()> {
    if valid {
        Ok(())
    } else {
        Err(std::io::Error::other("Codex frame exceeds limit"))
    }
}
pub fn tools() -> Value {
    let source = json!({"type":"string","description":"Complete Verum source for this revision"});
    let permissions = json!({"type":"string","enum":["source","deny-all","allow-all"],"description":"Script permission mode: source follows CLI/frontmatter defaults; deny-all clears grants; allow-all grants all kinds. Not an OS sandbox."});
    let timeout = json!({"type":"integer","minimum":1,"maximum":300000,"description":"Wall time budget in milliseconds; default 60000"});
    let mut tools = List::new();
    for (name, description, properties, required) in [
        (
            "verum_files",
            "Browse a project directory lazily (path defaults to the root), or search project file paths with query. Includes source and supporting files; hidden, generated and symlink paths are excluded. Search reports truncation explicitly.",
            json!({"path":{"type":"string"},"query":{"type":"string"}}),
            json!([]),
        ),
        (
            "verum_changes",
            "Inspect read-only Git status with separate index and working-tree states. To review a patch, supply path and side (index or worktree). Does not stage, commit, reset or change files.",
            json!({"path":{"type":"string"},"side":{"type":"string","enum":["index","worktree"]}}),
            json!([]),
        ),
        (
            "verum_project",
            "Inspect workspace source inventory and revision-bound analyzed documents shared with Studio. Coverage is analyzed documents only, not a whole-project build or proof.",
            json!({}),
            json!([]),
        ),
        (
            "verum_analyze",
            "Inspect exact source through the same LSP analysis as Studio: versioned diagnostics, declarations and context requirements. Single-module analysis with builtins, not a whole-project or runtime proof. Omit source to read the saved file.",
            json!({"path":{"type":"string"},"source":source}),
            json!(["path"]),
        ),
        (
            "verum_read",
            "Read a .vr source in the open workspace",
            json!({"path":{"type":"string"}}),
            json!(["path"]),
        ),
        (
            "verum_propose",
            "Propose a source replacement for human review; compare-and-swap against expected_hash",
            json!({"path":{"type":"string"},"source":source,"expected_hash":{"type":["string","null"]}}),
            json!(["path", "source", "expected_hash"]),
        ),
        (
            "verum_plan",
            "Inspect the exact execution contract without launching a process or writing run evidence. Run permission overrides apply only to scripts.",
            json!({"kind":{"type":"string","enum":["run","check","proof"]},"path":{"type":"string"},"source":source,"permissions":permissions,"timeout_ms":timeout,"declaration":{"type":"string"}}),
            json!(["kind", "path", "source"]),
        ),
        (
            "verum_run",
            "Run exact source with Verum interpreter after user approval",
            json!({"path":{"type":"string"},"source":source,"permissions":permissions,"timeout_ms":timeout}),
            json!(["path", "source"]),
        ),
        (
            "verum_check",
            "Type-check exact Verum source after user approval",
            json!({"path":{"type":"string"},"source":source,"timeout_ms":timeout}),
            json!(["path", "source"]),
        ),
        (
            "verum_proof",
            "Check one closed logical theorem using verum-source-pi-id/1; not a runtime refinement proof",
            json!({"path":{"type":"string"},"source":source,"declaration":{"type":"string"},"timeout_ms":timeout}),
            json!(["path", "source", "declaration"]),
        ),
    ] {
        tools.push(json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}));
    }
    json!(tools)
}
