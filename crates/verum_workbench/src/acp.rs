//! Text conversations over Antigravity CLI's ACP v1 stdio interface.
use crate::{agent_tools::Role, codex::Events};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, timeout},
};
use uuid::Uuid;
use verum_common::{List, Map, Text};

const FRAME_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

type Reply = oneshot::Sender<Result<Value>>;
enum Action {
    Initialize,
    Probe,
    Account,
    Login,
    New(Role),
    Prompt { thread: Text, text: Text },
    Interrupt { thread: Text, turn: Text },
}
enum Pending {
    Init(Reply),
    Probe(Reply),
    Login,
    New(Role, Reply),
    Prompt {
        thread: Text,
        turn: Text,
        cancelling: bool,
        output: Text,
    },
}
struct Request {
    deadline: Instant,
    pending: Pending,
}
pub(crate) struct Acp {
    pub(crate) events: Arc<Mutex<Events>>,
    actions: mpsc::Sender<(Action, Reply)>,
    task: JoinHandle<()>,
}

impl Acp {
    pub(crate) async fn connect(executable: &Path) -> Result<Self> {
        let data = crate::data_home().context("OS application-data directory unavailable")?;
        std::fs::create_dir_all(&data)?;
        let home = crate::workspace::private_dir(&data, &["VerumStudio", "agy"])?;
        Self::connect_with_storage(executable, &home).await
    }

    pub(crate) async fn connect_with_storage(executable: &Path, home: &Path) -> Result<Self> {
        Self::connect_in(executable, home, TURN_TIMEOUT).await
    }

    async fn connect_in(executable: &Path, home: &Path, turn_timeout: Duration) -> Result<Self> {
        let runtime = tempfile::tempdir()?;
        let room = crate::workspace::private_dir(runtime.path(), &["conversation"])?;
        let mut command = crate::antigravity::command(executable, home, &room)?;
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("Start Antigravity CLI with ACP support")?;
        let group = crate::jobs::ProcessGroup(child.id());
        let stdout = child
            .stdout
            .take()
            .context("Antigravity stdout unavailable")?;
        let stdin = child
            .stdin
            .take()
            .context("Antigravity stdin unavailable")?;
        let (actions, mut commands) = mpsc::channel(32);
        let events = Arc::new(Mutex::new(Events::default()));
        let mut session = Session {
            stdin,
            room,
            events: events.clone(),
            next: 0,
            pending: Map::new(),
            turn_timeout,
            authenticated: false,
            prepared: None,
        };
        let task = tokio::spawn(async move {
            let (frames, mut received) = mpsc::channel(4);
            let reader = tokio::spawn(read_frames(stdout, frames));
            let mut clock = tokio::time::interval(Duration::from_millis(200));
            let reason = loop {
                let result = tokio::select! {
                    command = commands.recv() => match command {
                        Some((action, reply)) => session.act(action, reply).await,
                        None => break "Antigravity ACP closed".to_owned(),
                    },
                    message = received.recv() => match message {
                        Some(value) => session.receive(value).await,
                        None => break "Antigravity ACP disconnected".to_owned(),
                    },
                    _ = clock.tick() => {
                        if session.pending.values().any(|req| req.deadline <= Instant::now()) {
                            break "Antigravity ACP request timed out".to_owned();
                        }
                        continue;
                    }
                };
                if let Err(error) = result {
                    break error.to_string();
                }
            };
            commands.close();
            reader.abort();
            drop(group);
            let _ = child.kill().await;
            session.close(&reason).await;
            drop(runtime);
        });
        let client = Self {
            events,
            actions,
            task,
        };
        client.call(Action::Initialize).await?;
        client.call(Action::Probe).await?;
        Ok(client)
    }

    pub(crate) fn connected(&self) -> bool {
        !self.task.is_finished() && !self.actions.is_closed()
    }

    async fn call(&self, action: Action) -> Result<Value> {
        let (reply, result) = oneshot::channel();
        self.actions
            .try_send((action, reply))
            .map_err(|_| anyhow::anyhow!("Antigravity ACP disconnected or busy"))?;
        result.await.context("Antigravity ACP disconnected")?
    }

    pub(crate) async fn account(&self) -> Result<Value> {
        self.call(Action::Account).await
    }
    pub(crate) async fn login(&self) -> Result<Value> {
        self.call(Action::Login).await
    }

    pub(crate) async fn request(
        &self,
        method: &str,
        params: &Value,
        role: Option<&Value>,
    ) -> Result<Value> {
        ensure!(params.is_object(), "Agent params must be an object");
        if method != "thread/start" {
            ensure!(role.is_none(), "Conversation role is fixed at creation");
        }
        match method {
            "thread/start" => {
                ensure!(
                    params.as_object().is_some_and(|p| p.is_empty()),
                    "Antigravity uses the account default model"
                );
                let role = role
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()
                    .context("Expected author or reviewer role")?
                    .unwrap_or_default();
                self.call(Action::New(role)).await
            }
            "turn/start" => {
                let parts = params["input"]
                    .as_array()
                    .context("Expected a text prompt")?;
                let [part] = parts.as_slice() else {
                    bail!("Expected one text prompt");
                };
                ensure!(
                    part["type"] == "text" && part.as_object().is_some_and(|p| p.len() == 2),
                    "Antigravity accepts text only"
                );
                let text = crate::field(part, "text")?;
                ensure!(
                    !text.trim().is_empty() && text.len() <= 128 * 1024,
                    "Expected 1–131072 bytes of prompt text"
                );
                ensure!(
                    !text.trim_start().starts_with('/'),
                    "Antigravity slash commands are unavailable in a text conversation"
                );
                self.call(Action::Prompt {
                    thread: crate::field(params, "threadId")?.into(),
                    text: text.into(),
                })
                .await
            }
            "turn/interrupt" => {
                self.call(Action::Interrupt {
                    thread: crate::field(params, "threadId")?.into(),
                    turn: crate::field(params, "turnId")?.into(),
                })
                .await
            }
            "account/read" => self.account().await,
            "model/list" => Ok(json!({"data":[]})),
            _ => bail!("Unsupported Antigravity ACP operation"),
        }
    }
}

async fn read_frames(stdout: ChildStdout, frames: mpsc::Sender<Value>) -> Result<()> {
    let mut input = BufReader::new(stdout);
    loop {
        let mut line = List::new();
        loop {
            let buf = input.fill_buf().await?;
            if buf.is_empty() {
                return Ok(());
            }
            let newline = buf.iter().position(|b| *b == b'\n');
            let n = newline.map_or(buf.len(), |p| p + 1);
            ensure!(
                line.len() + n <= FRAME_BYTES,
                "Antigravity ACP frame too large"
            );
            line.extend_from_slice(&buf[..n]);
            input.consume(n);
            if newline.is_some() {
                break;
            }
        }
        let value = serde_json::from_slice(&line)?;
        if frames.send(value).await.is_err() {
            return Ok(());
        }
    }
}

struct Session {
    stdin: ChildStdin,
    room: PathBuf,
    events: Arc<Mutex<Events>>,
    next: u64,
    pending: Map<u64, Request>,
    authenticated: bool,
    prepared: Option<Text>,
    turn_timeout: Duration,
}

impl Session {
    async fn write(&mut self, frame: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(frame)?;
        bytes.push(b'\n');
        timeout(Duration::from_secs(5), async {
            self.stdin.write_all(&bytes).await?;
            self.stdin.flush().await
        })
        .await??;
        Ok(())
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        pending: Pending,
        limit: Duration,
    ) -> Result<()> {
        self.next += 1;
        self.write(&json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}))
            .await?;
        self.pending.insert(
            self.next,
            Request {
                deadline: Instant::now() + limit,
                pending,
            },
        );
        Ok(())
    }

    async fn act(&mut self, action: Action, reply: Reply) -> Result<()> {
        let reject = |reply: Reply, message: &str| {
            let _ = reply.send(Err(anyhow::anyhow!("{message}")));
            Ok(())
        };
        if reply.is_closed() {
            return Ok(());
        }
        if self.pending.len() >= 64 {
            return reject(reply, "Antigravity ACP concurrent request limit reached");
        }
        match action {
            Action::Initialize => {
                let params = json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": {"name": "verum-studio", "version": env!("CARGO_PKG_VERSION")},
                });
                self.request("initialize", params, Pending::Init(reply), REQUEST_TIMEOUT)
                    .await?;
            }
            Action::Probe => {
                // ACP has no account/read method. A new session verifies the private profile
                // without sending a prompt. Its first Studio conversation can reuse it.
                let params = json!({"cwd":self.room,"mcpServers":[]});
                self.request(
                    "session/new",
                    params,
                    Pending::Probe(reply),
                    REQUEST_TIMEOUT,
                )
                .await?;
            }
            Action::Account => {
                let account = self.authenticated.then(|| json!({"type":"google"}));
                let _ = reply.send(Ok(json!({"provider":"agy","account":account})));
            }
            Action::Login => {
                let waiting = self
                    .pending
                    .values()
                    .any(|r| matches!(r.pending, Pending::Login));
                if !self.authenticated && !waiting {
                    let params = json!({"methodId":crate::antigravity::AUTH_METHOD});
                    self.request("authenticate", params, Pending::Login, LOGIN_TIMEOUT)
                        .await?;
                }
                let _ = reply.send(Ok(json!({"provider":"agy","interactive":true})));
            }
            Action::New(role) => {
                if !self.authenticated {
                    return reject(reply, "Sign in to Antigravity with Google first");
                }
                if let Err(error) = self.events.lock().await.can_start_thread() {
                    let _ = reply.send(Err(error));
                    return Ok(());
                }
                if let Some(thread) = self.prepared.take() {
                    let registration = self
                        .events
                        .lock()
                        .await
                        .register_thread(&thread, role)
                        .map(|_| json!({"thread":{"id":thread}}));
                    let _ = reply.send(registration);
                    return Ok(());
                }
                let params = json!({"cwd":self.room,"mcpServers":[]});
                self.request(
                    "session/new",
                    params,
                    Pending::New(role, reply),
                    REQUEST_TIMEOUT,
                )
                .await?;
            }
            Action::Prompt { thread, text } => {
                if !self.authenticated {
                    return reject(reply, "Sign in to Antigravity with Google first");
                }
                if self.events.lock().await.role(&thread).is_none() {
                    return reject(reply, "Unknown or retired Antigravity conversation");
                }
                if self.pending.values().any(|r| {
                    matches!(
                        &r.pending, Pending::Prompt { thread: owner,.. } if owner == &thread
                    )
                }) {
                    return reject(reply, "Antigravity conversation already has an active turn");
                }
                if let Err(error) = crate::antigravity::qualify_turn(&self.room) {
                    let _ = reply.send(Err(error));
                    return Ok(());
                }
                let turn: Text = Uuid::new_v4().to_string().into();
                let params = json!({"sessionId":thread,"prompt":[{"type":"text","text":text}]});
                let pending = Pending::Prompt {
                    thread: thread.clone(),
                    turn: turn.clone(),
                    cancelling: false,
                    output: Text::new(),
                };
                self.request("session/prompt", params, pending, self.turn_timeout)
                    .await?;
                let started = json!({"id":turn,"status":"inProgress"});
                self.events.lock().await.push(json!({
                    "method":"turn/started","params":{"threadId":thread,"turn":started}
                }))?;
                let _ = reply.send(Ok(json!({"turn":started})));
            }
            Action::Interrupt { thread, turn } => {
                let request = self.pending.values_mut().find(|r| matches!(
                    &r.pending, Pending::Prompt { thread:owner, turn:live,.. } if owner==&thread && live==&turn
                ));
                let Some(Request {
                    deadline,
                    pending: Pending::Prompt { cancelling, .. },
                }) = request
                else {
                    return reject(reply, "Antigravity turn is no longer active");
                };
                if !*cancelling {
                    *cancelling = true;
                    *deadline = Instant::now() + REQUEST_TIMEOUT;
                    self.write(&json!({
                        "jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":thread}
                    }))
                    .await?;
                }
                let _ = reply.send(Ok(json!({})));
            }
        }
        Ok(())
    }

    async fn receive(&mut self, message: Value) -> Result<()> {
        ensure!(
            message["jsonrpc"] == "2.0",
            "Expected an ACP v1 JSON-RPC frame"
        );
        if let Some(method) = message["method"].as_str() {
            if let Some(id) = message.get("id") {
                ensure!(id.is_number() || id.is_string(), "Invalid ACP request ID");
                let response = if method == "session/request_permission" {
                    json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}})
                } else {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Studio exposes no ACP client tools"}})
                };
                return self.write(&response).await;
            }
            if method == "session/update" {
                let params = &message["params"];
                let thread = crate::field(params, "sessionId")?;
                let update = &params["update"];
                if matches!(
                    update["sessionUpdate"].as_str(),
                    Some("tool_call" | "tool_call_update")
                ) {
                    // A pending notification may precede the permission request that we cancel.
                    ensure!(
                        matches!(update["status"].as_str(), None | Some("pending" | "failed")),
                        "Antigravity reported native tool activity in a text-only conversation"
                    );
                    return Ok(());
                }
                if update["sessionUpdate"] == "agent_message_chunk"
                    && update["content"]["type"] == "text"
                    && let Some(text) = update["content"]["text"].as_str()
                    && let Some((turn, output)) =
                        self.pending
                            .values_mut()
                            .find_map(|r| match &mut r.pending {
                                Pending::Prompt {
                                    thread: owner,
                                    turn,
                                    cancelling: false,
                                    output,
                                } if owner.as_str() == thread => Some((turn, output)),
                                _ => None,
                            })
                {
                    ensure!(
                        output.len() + text.len() <= FRAME_BYTES,
                        "Antigravity turn output limit reached"
                    );
                    output.push_str(text);
                    self.events.lock().await.push(json!({"method":"item/agentMessage/delta","params":{"threadId":thread,"turnId":turn,"itemId":turn,"delta":text}}))?;
                }
            }
            return Ok(());
        }
        let id = message["id"].as_u64().context("Invalid ACP response ID")?;
        let Some(req) = self.pending.remove(&id) else {
            return Ok(());
        };
        let result = message.get("result");
        ensure!(
            result.is_some() != message.get("error").is_some(),
            "Malformed ACP response"
        );
        if message["error"]["code"] == -32000
            && matches!(req.pending, Pending::New(..) | Pending::Prompt { .. })
        {
            self.authenticated = false;
            self.events
                .lock()
                .await
                .push(json!({"method":"account/updated","params":{"account":null}}))?;
        }
        let error = |action: &str| {
            anyhow::anyhow!(
                "Antigravity {action} failed (ACP code {})",
                message["error"]["code"].as_i64().unwrap_or(-32603)
            )
        };
        match req.pending {
            Pending::Init(reply) => {
                let valid = result.is_some_and(|r| {
                    r["protocolVersion"] == 1
                        && r["agentInfo"]["name"] == "Antigravity ACP Server"
                        && r["authMethods"].as_array().is_some_and(|methods| {
                            methods
                                .iter()
                                .any(|m| m["id"] == crate::antigravity::AUTH_METHOD)
                        })
                });
                let _ = reply.send(if valid {
                    Ok(json!({}))
                } else {
                    Err(anyhow::anyhow!(
                        "Antigravity CLI must support ACP v1 and Google sign-in"
                    ))
                });
            }
            Pending::Probe(reply) => {
                let probe = if let Some(thread) = result
                    .and_then(|r| r["sessionId"].as_str())
                    .filter(|id| !id.is_empty() && id.len() <= 1024)
                {
                    self.prepared = Some(thread.into());
                    self.authenticated = true;
                    Ok(json!({}))
                } else if message["error"]["code"] == -32000 {
                    Ok(json!({}))
                } else {
                    Err(error("profile check"))
                };
                let _ = reply.send(probe);
            }
            Pending::Login => {
                self.authenticated = result.is_some();
                let reason = (!self.authenticated).then(|| error("Google sign-in").to_string());
                self.events.lock().await.push(json!({"method":"account/login/completed","params":{"success":self.authenticated,"error":reason}}))?;
            }
            Pending::New(role, reply) => {
                let registration = match result.and_then(|v| v["sessionId"].as_str()) {
                    Some(thread) => self
                        .events
                        .lock()
                        .await
                        .register_thread(thread, role)
                        .map(|_| json!({"thread":{"id":thread}})),
                    None => Err(error("session creation")),
                };
                let _ = reply.send(registration);
            }
            Pending::Prompt {
                thread,
                turn,
                cancelling,
                output,
            } => {
                let stop = result.and_then(|r| r["stopReason"].as_str());
                let (status, reason) = match (cancelling, stop) {
                    (true, _) | (_, Some("cancelled")) => ("interrupted", None),
                    (_, Some("end_turn")) => ("completed", None),
                    (_, Some("max_tokens" | "max_turn_requests" | "refusal")) => {
                        ("failed", stop.map(|s| format!("Antigravity stopped: {s}")))
                    }
                    _ => ("failed", Some(error("prompt").to_string())),
                };
                if status == "completed" && !output.is_empty() {
                    self.events.lock().await.push(json!({"method":"item/completed","params":{"threadId":thread,"turnId":turn,"item":{"id":turn,"type":"agentMessage","text":output}}}))?;
                }
                self.complete(&thread, &turn, status, reason.as_deref())
                    .await?;
            }
        }
        Ok(())
    }

    async fn complete(
        &self,
        thread: &str,
        turn: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.events.lock().await.push(json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":turn,"status":status,"error":error.map(|message|json!({"message":message}))}}}))
    }

    async fn close(&mut self, reason: &str) {
        let mut events = self.events.lock().await;
        for (_, request) in std::mem::take(&mut self.pending) {
            match request.pending {
                Pending::Init(reply) | Pending::Probe(reply) | Pending::New(_, reply) => {
                    let _ = reply.send(Err(anyhow::anyhow!("{reason}")));
                }
                Pending::Prompt { thread, turn, .. } => {
                    let _=events.push(json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":turn,"status":"failed","error":{"message":reason}}}}));
                }
                Pending::Login => {
                    let _=events.push(json!({"method":"account/login/completed","params":{"success":false,"error":reason}}));
                }
            }
        }
        events.disconnect(reason);
    }
}

#[cfg(all(test, unix))]
#[path = "../tests/acp/conversation.rs"]
mod tests;
