//! A workspace owns one serialized LSP client session on a dedicated worker.
//! UI and agent requests use the same synchronization and revision-bound results.
use anyhow::{Context, Result, bail, ensure};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tower::{Service, ServiceExt};
use tower_lsp::{
    LspService,
    jsonrpc::{Request, Response},
    lsp_types::Url,
};
use verum_common::{Map, Text};
use verum_lsp::Backend;

struct Command {
    path: PathBuf,
    source: Text,
    method: Text,
    position: Value,
    reply: oneshot::Sender<Result<Value>>,
}
pub struct Language {
    requests: mpsc::Sender<Command>,
}
impl Language {
    pub fn start(root: PathBuf) -> Result<Self> {
        let (requests, mut receiver) = mpsc::channel::<Command>(8);
        std::thread::Builder::new()
            .name("studio-language".into())
            // Parser/type-checker recursion needs the compiler's 16 MiB
            // analysis stack, including when launched outside Cargo's env.
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else {
                    return;
                };
                runtime.block_on(async move {
                    let mut session = Session::new(root).await;
                    while let Some(command) = receiver.recv().await {
                        if command.reply.is_closed() {
                            continue;
                        }
                        let result = match &mut session {
                            Ok(session) => session.call(&command).await,
                            Err(error) => {
                                Err(anyhow::anyhow!("Language service unavailable: {error}"))
                            }
                        };
                        let _ = command.reply.send(result);
                    }
                });
            })?;
        Ok(Self { requests })
    }
    pub async fn request(
        &self,
        path: PathBuf,
        source: Text,
        method: &str,
        position: Value,
    ) -> Result<Value> {
        ensure!(
            source.len() <= crate::workspace::SOURCE_LIMIT,
            "Source exceeds editor limit"
        );
        ensure!(
            matches!(
                method,
                "project"
                    | "analyze"
                    | "textDocument/hover"
                    | "textDocument/completion"
                    | "textDocument/definition"
            ),
            "Unsupported language operation"
        );
        let (reply, response) = oneshot::channel();
        self.requests
            .try_send(Command {
                path,
                source,
                method: method.into(),
                position,
                reply,
            })
            .map_err(|_| {
                anyhow::anyhow!(
                    "Language service busy or unavailable; retry after the current analysis"
                )
            })?;
        tokio::time::timeout(std::time::Duration::from_secs(30), response)
            .await
            .context("Language analysis timed out")?
            .context("Language worker stopped")?
    }
}

struct Session {
    service: LspService<Backend>,
    documents: Map<Text, (Text, i32)>,
    sequence: i64,
    capabilities: Value,
}
impl Session {
    async fn new(root: PathBuf) -> Result<Self> {
        let (service, socket) = verum_lsp::service::service();
        tokio::spawn(async move {
            let (mut incoming, mut outgoing) = socket.split();
            while let Some(request) = incoming.next().await {
                if let Some(id) = request.id() {
                    // The embedded client does not grant workspace edits or execute commands.
                    let response = Response::from_parts(
                        id.clone(),
                        Err(tower_lsp::jsonrpc::Error::method_not_found()),
                    );
                    if outgoing.send(response).await.is_err() {
                        break;
                    }
                }
            }
        });
        let mut session = Self {
            service,
            documents: Map::new(),
            sequence: 0,
            capabilities: Value::Null,
        };
        let uri = Url::from_directory_path(root).map_err(|_| anyhow::anyhow!("Workspace URI"))?;
        session.capabilities = session.rpc("initialize", json!({
            "processId":std::process::id(), "rootUri":uri,
            "capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"publishDiagnostics":{"versionSupport":true}}},
            "initializationOptions":{"lint":{"enabled":false}}
        }), true).await?;
        session.rpc("initialized", json!({}), false).await?;
        Ok(session)
    }
    async fn rpc(&mut self, method: &str, params: Value, response: bool) -> Result<Value> {
        let mut request = Request::build(method.to_owned()).params(params);
        if response {
            self.sequence += 1;
            request = request.id(self.sequence);
        }
        let reply = self.service.ready().await?.call(request.finish()).await?;
        match reply {
            Some(reply) => reply
                .into_parts()
                .1
                .map_err(|error| anyhow::anyhow!("{error}")),
            None if !response => Ok(Value::Null),
            None => bail!("Language server omitted the response"),
        }
    }
    async fn call(&mut self, command: &Command) -> Result<Value> {
        if command.method == "project" {
            return self.rpc("verum/projectSnapshot", json!({}), true).await;
        }
        let uri = Url::from_file_path(&command.path).map_err(|_| anyhow::anyhow!("Source URI"))?;
        let key = Text::from(uri.as_str());
        let hash = crate::workspace::digest(command.source.as_bytes());
        let previous = self.documents.get(&key).cloned();
        if previous.is_none() && self.documents.len() >= 128 {
            // Bound retained source/AST memory; reopening reconstructs from exact source.
            let old = self.documents.keys().next().cloned();
            if let Some(old) = old {
                self.rpc(
                    "textDocument/didClose",
                    json!({"textDocument":{"uri":old}}),
                    false,
                )
                .await?;
                self.documents.remove(&old);
            }
        }
        let version = match previous {
            Some((old, version)) if old == hash => version,
            Some((_, version)) => {
                let version = version
                    .checked_add(1)
                    .context("Document version exhausted")?;
                self.rpc("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":command.source}]}), false).await?;
                version
            }
            None => {
                self.rpc("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":"verum","version":1,"text":command.source}}), false).await?;
                1
            }
        };
        self.documents.insert(key, (hash.clone(), version));
        let result = if command.method == "analyze" {
            self.rpc(
                "verum/documentSnapshot",
                json!({"textDocument":{"uri":uri},"version":version}),
                true,
            )
            .await?
        } else {
            let position: tower_lsp::lsp_types::Position =
                serde_json::from_value(command.position.clone())
                    .context("Expected LSP position")?;
            ensure!(
                verum_common::span_utils::lsp::lsp_position_to_offset(position, &command.source)
                    .is_some(),
                "Position outside source"
            );
            self.rpc(
                &command.method,
                json!({"textDocument":{"uri":uri},"position":position}),
                true,
            )
            .await?
        };
        Ok(
            json!({"source_hash":hash,"version":version,"result":result,"server":self.capabilities["serverInfo"]}),
        )
    }
}
