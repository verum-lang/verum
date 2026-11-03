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

use crate::agent_tools::Role;
use crate::agent_runtime::{End, Scope, Turns};

struct PendingRequest {
    reply: oneshot::Sender<Result<Value>>,
    starting: Option<Role>,
}
type Pending = Arc<Mutex<Map<u64, PendingRequest>>>;
pub struct Events {
    connection: uuid::Uuid,
    // Keep retired IDs until disconnect so they cannot acquire a different role.
    roles: std::collections::HashMap<String, Option<Role>>,
    next: u64,
    messages: std::collections::VecDeque<Value>,
    // Live provider wire IDs are unique within this transport. Their values are
    // host-issued envelopes identifying the exact occurrence shown for review.
    requests: Map<Text, Value>,
    scopes: Map<Text, Scope>,
    executing: Map<Text, Scope>,
    turns: Turns,
}
impl Default for Events {
    fn default() -> Self {
        let connection = uuid::Uuid::new_v4();
        Self {
            connection,
            roles: Default::default(),
            next: 0,
            messages: Default::default(),
            requests: Map::new(),
            scopes: Map::new(),
            executing: Map::new(),
            turns: Turns::new(connection),
        }
    }
}
impl Events {
    pub fn connection(&self) -> uuid::Uuid {
        self.connection
    }

    pub(crate) fn can_start_thread(&self) -> Result<()> {
        ensure!(
            self.roles.len() < 256,
            "Conversation limit reached; restart Studio to start a new provider session"
        );
        Ok(())
    }

    pub(crate) fn register_thread(&mut self, thread: &str, role: Role) -> Result<()> {
        ensure!(
            !thread.is_empty() && thread.len() <= 1024,
            "Invalid provider conversation ID"
        );
        // An occurrence received before registration never inherits a later grant.
        self.requests
            .retain(|_, entry| entry["message"]["params"]["threadId"] != thread);
        if let Some(previous) = self.roles.get_mut(thread) {
            *previous = None;
            self.turns.retire(thread);
            self.scopes.retain(|_, scope| scope.identity().conversation != thread);
            self.executing.retain(|_, scope| scope.identity().conversation != thread);
            bail!("Provider reused a conversation ID; its authorizations have been revoked");
        }
        self.can_start_thread()?;
        self.roles.insert(thread.to_owned(), Some(role));
        Ok(())
    }

    pub(crate) fn role(&self, thread: &str) -> Option<Role> {
        self.roles.get(thread).copied().flatten()
    }

    fn push(&mut self, message: Value) -> Result<()> {
        let params = &message["params"];
        if message["method"] == "serverRequest/resolved" {
            let key: Text = params["requestId"].to_string().into();
            if let Some(pending) = self.requests.get(&key)
                && (params.get("threadId").is_none()
                    || pending["message"]["params"]["threadId"] == params["threadId"])
            {
                self.requests.remove(&key);
                if let Some(scope) = self.scopes.remove(&key) { scope.end(End::Resolved); }
            }
            if let Some(scope) = self.executing.get(&key)
                && (params.get("threadId").is_none()
                    || params["threadId"] == scope.identity().conversation.as_str())
                && let Some(scope) = self.executing.remove(&key)
            { scope.end(End::Resolved); }
        } else if message["method"] == "turn/completed"
            && let (Some(thread), Some(turn)) =
                (params["threadId"].as_str(), params["turn"]["id"].as_str())
        {
            self.end_turn(thread, turn, End::Completed)?;
        }
        let seq = self.next + 1;
        let mut entry = json!({"seq":seq,"message":message});
        if let Some(id) = message.get("id") {
            ensure!(
                id.is_string() || id.is_i64() || id.is_u64(),
                "Invalid provider request ID"
            );
            ensure!(
                params["threadId"]
                    .as_str()
                    .is_some_and(|thread| !thread.is_empty()),
                "Unscoped provider request"
            );
            let key: Text = id.to_string().into();
            ensure!(
                self.requests.len() + self.executing.len() < 64
                    && !self.requests.contains_key(&key) && !self.executing.contains_key(&key),
                "Invalid pending provider request"
            );
            let mut reviewable = true;
            if message["method"] == "item/tool/call"
                && let Some(role) = params["threadId"]
                    .as_str()
                    .and_then(|thread| self.role(thread))
                && let Some(kind) = params["tool"]
                    .as_str()
                    .and_then(|tool| role.review_kind(tool))
            {
                let thread = params["threadId"].as_str().expect("scoped request");
                let turn = params["turnId"].as_str().context("Unscoped provider tool call")?;
                match self.turns.get(thread, turn)?.child() {
                    Ok(scope) => {
                        self.scopes.insert(key.clone(), scope);
                        entry["reviewKind"] = json!(kind);
                    }
                    Err(_) => {
                        // Calls already in the stdout pipe can arrive after a
                        // local interrupt. Retain the event, never its authority.
                        reviewable = false;
                        entry["reviewExpired"] = json!(true);
                    }
                }
            }
            if reviewable {
                entry["reviewId"] = json!(uuid::Uuid::new_v4().to_string());
                self.requests.insert(key, entry.clone());
            }
        }
        self.next = seq;
        if self.messages.len() == 512 {
            self.messages.pop_front();
        }
        self.messages.push_back(entry);
        Ok(())
    }

    pub fn read(&self, after: u64, thread: Option<&str>) -> Value {
        let gap = self
            .messages
            .front()
            .and_then(|e| e["seq"].as_u64())
            .is_some_and(|s| after.saturating_add(1) < s);
        let visible = |e: &Value| {
            let message = &e["message"];
            let params = &message["params"];
            if let Some(owner) = params["threadId"]
                .as_str()
                .or_else(|| params["thread"]["id"].as_str())
            {
                Some(owner) == thread
            } else {
                matches!(
                    message["method"].as_str(),
                    Some("account/updated" | "account/login/completed" | "studio/disconnected")
                )
            }
        };
        let events: List<_> = self
            .messages
            .iter()
            .filter(|e| e["seq"].as_u64().is_some_and(|seq| seq > after) && visible(e))
            .collect();
        let pending: List<_> = self.requests.values().filter(|e| visible(e)).collect();
        // A live review may outlast its position in the bounded journal.
        let first = self
            .messages
            .front()
            .and_then(|e| e["seq"].as_u64())
            .unwrap_or(0);
        let recovered: List<_> = pending
            .iter()
            .filter(|e| gap && e["seq"].as_u64().is_some_and(|seq| seq < first))
            .collect();
        let reviews: List<_> = pending.iter().map(|e| &e["reviewId"]).collect();
        json!({"connectionId":self.connection.to_string(),"cursor":self.next,"events":events,"gap":gap,"pendingReviews":reviews,"recoveredReviews":recovered})
    }

    pub(crate) fn take_review(&mut self, review: &str, thread: Option<&str>) -> Result<(Value, Option<Scope>)> {
        let key = self
            .requests
            .iter()
            .find_map(|(key, entry)| {
                (entry["reviewId"] == review
                    && entry["message"]["params"]["threadId"].as_str() == thread)
                    .then(|| key.clone())
            })
            .context("This review has expired or belongs to another conversation")?;
        let mut entry = self.requests.remove(&key).context("Review has expired")?;
        let scope = self.scopes.remove(&key);
        if let Some(scope) = &scope {
            scope.admit(|| Ok(()))?;
            self.executing.insert(key, scope.clone());
        }
        Ok((entry["message"].take(), scope))
    }

    pub(crate) fn finish_review(&mut self, id: &Value, scope: &Scope) -> bool {
        let key: Text = id.to_string().into();
        if self.executing.get(&key).is_some_and(|current| current.same(scope)) {
            self.executing.remove(&key);
            return scope.admit(|| Ok(())).is_ok();
        }
        false
    }

    pub(crate) fn end_turn(&mut self, thread: &str, turn: &str, reason: End) -> Result<()> {
        self.turns.end(thread, turn, reason)?;
        self.requests.retain(|_, entry| {
            let owner = &entry["message"]["params"];
            owner["threadId"] != thread || owner["turnId"] != turn
        });
        let other = |scope: &Scope| scope.identity().conversation != thread || scope.identity().turn != turn;
        self.scopes.retain(|_, scope| other(scope));
        self.executing.retain(|_, scope| other(scope));
        Ok(())
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
    turns: Turns,
}
struct Transmission<'a> {
    provider: &'a Codex,
    complete: bool,
}
impl Drop for Transmission<'_> {
    fn drop(&mut self) {
        if !self.complete {
            // Cancellation may have interrupted a frame between writes.
            self.provider.turns.disconnect();
            self.provider.reader.abort();
        }
    }
}
impl Drop for Codex {
    fn drop(&mut self) {
        self.turns.disconnect();
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
        let turns = events.lock().await.turns.clone();
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
                    // Release the pending-request lock before registration awaits
                    // the event journal. Response delivery takes the other order.
                    let request = if let Some(id) = message["id"].as_u64() {
                        responses.lock().await.remove(&id)
                    } else { None };
                    if let Some(request) = request {
                        // Install the role in wire order, before a following tool call
                        // can enter the approval journal. A cancelled start grants nothing.
                        let result = if let Some(role) = request.starting
                            && message.get("error").is_none()
                        {
                            let thread = message["result"]["thread"]["id"]
                                .as_str()
                                .or_else(|| message["result"]["id"].as_str());
                            match thread {
                                Some(id) => {
                                    let mut events = notifications.lock().await;
                                    let registration = events.register_thread(id, role);
                                    if request.reply.is_closed() {
                                        if let Some(grant) = events.roles.get_mut(id) {
                                            *grant = None;
                                        }
                                    }
                                    registration.map(|_| message)
                                }
                                None => Err(anyhow::anyhow!(
                                    "Provider did not return a conversation ID"
                                )),
                            }
                        } else {
                            Ok(message)
                        };
                        let _ = request.reply.send(result);
                    }
                } else {
                    let mut events = notifications.lock().await;
                    if events.push(message).is_err() {
                        break;
                    }
                }
            }
            responses.lock().await.clear();
            let mut events = notifications.lock().await;
            events.turns.disconnect();
            events.requests.clear();
            events.scopes.clear();
            events.executing.clear();
            let _ = events.push(json!({"method":"studio/disconnected","params":{"message":"Codex disconnected. Check CLI version and reconnect."}}));
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
            turns,
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
        let write = async {
            let mut writer = self.writer.lock().await;
            let mut transmission = Transmission { provider: self, complete: false };
            writer.write_all(&bytes).await?;
            writer.flush().await?;
            transmission.complete = true;
            Ok::<_, anyhow::Error>(())
        };
        let result = tokio::time::timeout(Duration::from_secs(5), write)
            .await.context("Provider stopped accepting input").and_then(|result| result);
        if result.is_err() {
            // A partial JSON frame cannot be retried on the same connection.
            self.turns.disconnect();
            self.reader.abort();
            self.pending.lock().await.clear();
        }
        result
    }
    pub(crate) async fn start_thread(&self, role: Role, params: Value) -> Result<Value> {
        self.events.lock().await.can_start_thread()?;
        self.request_inner("thread/start", params, Some(role)).await
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_inner(method, params, None).await
    }

    async fn request_inner(
        &self,
        method: &str,
        params: Value,
        starting: Option<Role>,
    ) -> Result<Value> {
        ensure!(
            !self.reader.is_finished(),
            "Codex is disconnected; reconnect first"
        );
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (send, receive) = oneshot::channel();
        self.pending.lock().await.insert(
            id,
            PendingRequest {
                reply: send,
                starting,
            },
        );
        let result = async {
            self.send(json!({"id":id,"method":method,"params":params}))
                .await?;
            let message = tokio::time::timeout(Duration::from_secs(30), receive)
                .await
                .context("Codex request timed out")?
                .context("Codex disconnected; update the CLI if app-server v2 is unavailable")??;
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
/// Tool definitions sent through Codex's dynamic tool transport.
pub use crate::agent_tools::catalog as tools;

#[cfg(test)]
mod review_tests {
    use super::*;

    fn call(thread: &str, path: &str) -> Value {
        json!({"id":7,"method":"item/tool/call","params":{
            "threadId":thread,"turnId":"turn","tool":"verum_propose","arguments":{"path":path}
        }})
    }
    fn ticket(events: &Events, thread: &str) -> String {
        events.read(0, Some(thread))["pendingReviews"][0]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn terminal_turns_expire_execution_and_cannot_be_resurrected() {
        let mut events = Events::default();
        events.register_thread("author", Role::Author).unwrap();
        events.push(call("author", "first.vr")).unwrap();
        let (_, scope) = events.take_review(&ticket(&events, "author"), Some("author")).unwrap();
        let scope = scope.unwrap();
        events.end_turn("author", "turn", End::Interrupted).unwrap();
        assert!(matches!(scope.cancelled().await, End::Interrupted));
        assert!(!events.finish_review(&json!(7), &scope));
        events.push(call("author", "late.vr")).unwrap();
        let expired = events.read(0, Some("author"));
        assert_eq!(expired["pendingReviews"], json!([]));
        let last = expired["events"].as_array().unwrap().last().unwrap();
        assert_eq!(last["reviewExpired"], true);
        assert!(last.get("reviewId").is_none());
        let mut fresh = call("author", "new.vr");
        fresh["params"]["turnId"] = json!("new-turn");
        events.push(fresh).unwrap();
        assert_eq!(events.read(0, Some("author"))["pendingReviews"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn withdrawing_one_request_cancels_only_its_execution() {
        let mut events = Events::default();
        events.register_thread("author", Role::Author).unwrap();
        events.push(call("author", "first.vr")).unwrap();
        let (_, first) = events.take_review(&ticket(&events, "author"), Some("author")).unwrap();
        let first = first.unwrap();
        let mut second = call("author", "second.vr");
        second["id"] = json!(8);
        events.push(second).unwrap();
        let (_, second) = events.take_review(&ticket(&events, "author"), Some("author")).unwrap();
        let second = second.unwrap();
        events.push(json!({"method":"serverRequest/resolved","params":{"threadId":"author","requestId":7}})).unwrap();
        assert!(matches!(first.cancelled().await, End::Resolved));
        assert!(!events.finish_review(&json!(7), &first));
        assert!(second.admit(|| Ok(())).is_ok());
        events.turns.disconnect();
        assert!(matches!(second.cancelled().await, End::Disconnected));
    }

    #[test]
    fn review_authorizes_one_exact_occurrence_in_one_conversation() {
        let mut events = Events::default();
        events
            .register_thread("author", crate::agent_tools::Role::Author)
            .unwrap();
        events.push(call("author", "first.vr")).unwrap();
        let first = ticket(&events, "author");
        assert!(events.take_review(&first, Some("critic")).is_err());
        assert!(
            events.read(0, Some("critic"))["events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            events.read(0, None)["pendingReviews"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            events.read(0, Some("author"))["events"][0]["reviewKind"],
            "source"
        );
        assert_eq!(
            events.take_review(&first, Some("author")).unwrap().0["params"]["arguments"]["path"],
            "first.vr"
        );
        let scope = events.executing.get(&Text::from("7")).unwrap().clone();
        assert!(events.finish_review(&json!(7), &scope));
        events.push(call("author", "second.vr")).unwrap();
        let second = ticket(&events, "author");
        assert_ne!(first, second);
        assert!(events.take_review(&first, Some("author")).is_err());
        assert!(events.push(call("critic", "replacement.vr")).is_err());
        let mut reconnected = Events::default();
        reconnected.push(call("author", "new-process.vr")).unwrap();
        assert_ne!(events.connection(), reconnected.connection());
        assert!(reconnected.take_review(&second, Some("author")).is_err());
        assert!(events.take_review(&second, Some("author")).is_ok());
    }

    #[test]
    fn later_registration_cannot_authorize_an_earlier_tool_call() {
        let mut events = Events::default();
        events.push(call("unregistered", "early.vr")).unwrap();
        let id = ticket(&events, "unregistered");
        events
            .register_thread("unregistered", crate::agent_tools::Role::Author)
            .unwrap();
        assert!(events.take_review(&id, Some("unregistered")).is_err());
        events.push(call("unregistered", "current.vr")).unwrap();
        let fresh = ticket(&events, "unregistered");
        assert_eq!(
            events.take_review(&fresh, Some("unregistered")).unwrap().0["params"]["arguments"]["path"],
            "current.vr"
        );
    }

    #[test]
    fn journal_eviction_preserves_live_reviews_and_terminal_events_retire_only_the_owner() {
        let mut events = Events::default();
        events.push(call("author", "lib.vr")).unwrap();
        let id = ticket(&events, "author");
        events.push(json!({"method":"turn/completed","params":{"threadId":"critic","turn":{"id":"turn"}}})).unwrap();
        events.push(json!({"method":"thread/started","params":{"thread":{"id":"author","preview":"private"}}})).unwrap();
        assert_eq!(
            events.read(0, Some("critic"))["events"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        for _ in 0..512 {
            events
                .push(json!({"method":"account/updated","params":{}}))
                .unwrap();
        }
        let page = events.read(0, Some("author"));
        assert_eq!(page["gap"], true);
        assert_eq!(page["recoveredReviews"][0]["reviewId"], id);
        assert_eq!(
            events.read(0, Some("critic"))["recoveredReviews"],
            json!([])
        );
        assert_eq!(events.read(u64::MAX, None)["events"], json!([]));
        events.push(json!({"method":"turn/completed","params":{"threadId":"author","turn":{"id":"turn"}}})).unwrap();
        assert!(events.take_review(&id, Some("author")).is_err());
        assert!(
            events
                .push(json!({"id":9,"method":"item/tool/call","params":{"tool":"verum_read"}}))
                .is_err()
        );
    }
}
