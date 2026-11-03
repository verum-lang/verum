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
use verum_common::source_mode::SourceMode;
use verum_lsp::Backend;

pub(super) const DOCUMENT_QUERIES: &[&str] = &[
    "analyze",
    "textDocument/hover",
    "textDocument/definition",
    "textDocument/completion",
];

struct Command {
    source_mode: SourceMode,
    control: verum_lsp::AnalysisControl,
    path: PathBuf,
    source: Text,
    method: Text,
    argument: Value,
    reply: oneshot::Sender<Result<Value>>,
}
// Dropping a timed-out or disconnected caller also cancels synchronous work
// on the analysis thread. The session itself finishes synchronization so its
// document-version ledger cannot diverge from the language server.
struct CancelOnDrop(verum_lsp::AnalysisControl);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub struct Language {
    requests: mpsc::Sender<Command>,
}
impl Language {
    pub fn start(root: PathBuf) -> Result<Self> {
        Self::start_with_library(root, None, None)
    }
    pub fn start_with_library(
        root: PathBuf,
        library: Option<std::sync::Arc<verum_lsp::library_metadata::LibraryMetadata>>,
        formatter: Option<PathBuf>,
    ) -> Result<Self> {
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
                    let mut session = Session::new(root, library, formatter).await;
                    while let Some(command) = receiver.recv().await {
                        if command.reply.is_closed() || command.control.is_cancelled() {
                            continue;
                        }
                        let result = match &mut session {
                            Ok(session) => {
                                session
                                    .service
                                    .inner()
                                    .set_analysis_control(Some(command.control.clone()));
                                let result = session.call(&command).await;
                                session.service.inner().set_analysis_control(None);
                                result
                            }
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
        argument: Value,
    ) -> Result<Value> {
        self.request_with_source_mode(path, source, method, argument, SourceMode::Source).await
    }
    pub async fn request_with_source_mode(
        &self, path: PathBuf, source: Text, method: &str, argument: Value, source_mode: SourceMode,
    ) -> Result<Value> {
        ensure!(
            source.len() <= crate::workspace::SOURCE_LIMIT,
            "Source exceeds editor limit"
        );
        ensure!(
            DOCUMENT_QUERIES.contains(&method) || matches!(method, "project" | "evidence" | "textDocument/formatting"),
            "Unsupported language operation"
        );
        let control = verum_lsp::AnalysisControl::with_deadline(
            std::time::Instant::now() + verum_lsp::ANALYSIS_TIMEOUT,
        );
        let _cancel_on_drop = CancelOnDrop(control.clone());
        let (reply, response) = oneshot::channel();
        self.requests
            .try_send(Command {
                source_mode,
                control,
                path,
                source,
                method: method.into(),
                argument,
                reply,
            })
            .map_err(|_| {
                anyhow::anyhow!(
                    "Language service busy or unavailable; retry after the current analysis"
                )
            })?;
        tokio::time::timeout(verum_lsp::ANALYSIS_TIMEOUT, response)
            .await
            .context("Language analysis timed out")?
            .context("Language worker stopped")?
    }
}

struct Session {
    service: LspService<Backend>,
    documents: Map<Text, (Text, i32, SourceMode)>,
    sequence: i64,
    capabilities: Value,
}
impl Session {
    async fn new(
        root: PathBuf,
        library: Option<std::sync::Arc<verum_lsp::library_metadata::LibraryMetadata>>,
        formatter: Option<PathBuf>,
    ) -> Result<Self> {
        let (service, socket) = verum_lsp::service::service_with_library(library);
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
            "initializationOptions":{"lint":{"enabled":false},"fmt":{"enabled":true,"binary":formatter}}
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
    async fn open_document(&mut self, uri: &Url, source: &str, source_mode: SourceMode) -> Result<()> {
        self.rpc("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":source_mode.language_id(),"version":1,"text":source}}), false).await?;
        Ok(())
    }

    async fn evidence(&mut self, command: &Command, uri: &Url, hash: &Text, active: Option<(Text, i32, SourceMode)>) -> Result<Value> {
        let expected = command.argument["expectedSourceHash"].as_str().context("Evidence requires a SHA-256 source hash")?;
        ensure!(expected.eq_ignore_ascii_case(hash.as_str()), "Evidence source hash differs from the requested revision");
        let transient = active.is_none();
        let version = if let Some((active_hash, version, mode)) = active {
            ensure!(active_hash == *hash && mode == command.source_mode, "Another source revision or mode is active in Studio; refresh analysis before requesting evidence");
            version
        } else {
            // A cold evidence read must not evict an editor buffer or keep a stale disk overlay.
            self.open_document(uri, &command.source, command.source_mode).await?;
            1
        };
        let mut params = command.argument.clone();
        params["textDocument"] = json!({"uri":uri});
        params["version"] = json!(version);
        let result = self.rpc("verum/documentEvidence", params, true).await;
        if transient {
            if let Err(error) = self.rpc("textDocument/didClose", json!({"textDocument":{"uri":uri}}), false).await {
                self.documents.insert(uri.as_str().into(), (hash.clone(), version, command.source_mode));
                return Err(error);
            }
        }
        command.control.check()?;
        result
    }
    async fn call(&mut self, command: &Command) -> Result<Value> {
        command.control.check()?;
        if command.method == "project" {
            let result = self.rpc("verum/projectSnapshot", json!({}), true).await?;
            command.control.check()?;
            return Ok(result);
        }
        let uri = Url::from_file_path(&command.path).map_err(|_| anyhow::anyhow!("Source URI"))?;
        let key = Text::from(uri.as_str());
        let hash = crate::workspace::digest(command.source.as_bytes());
        let mut previous = self.documents.get(&key).cloned();
        if command.method == "evidence" {
            return self.evidence(command, &uri, &hash, previous).await;
        }
        if previous.as_ref().is_some_and(|entry| entry.2 != command.source_mode) {
            self.rpc("textDocument/didClose", json!({"textDocument":{"uri":uri}}), false).await?;
            self.documents.remove(&key);
            previous = None;
        }
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
            Some((old, version, _)) if old == hash => version,
            Some((_, version, _)) => {
                let version = version
                    .checked_add(1)
                    .context("Document version exhausted")?;
                self.rpc("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":command.source}]}), false).await?;
                version
            }
            None => {
                self.open_document(&uri, &command.source, command.source_mode).await?;
                1
            }
        };
        self.documents.insert(key, (hash.clone(), version, command.source_mode));
        command.control.check()?;
        let result = if command.method == "analyze" {
            self.rpc(
                "verum/documentSnapshot",
                json!({"textDocument":{"uri":uri},"version":version}),
                true,
            )
            .await?
        } else if command.method == "textDocument/formatting" {
            let options: tower_lsp::lsp_types::FormattingOptions =
                serde_json::from_value(command.argument.clone())
                    .context("Expected formatting options")?;
            ensure!(
                (1..=16).contains(&options.tab_size),
                "Tab size must be between 1 and 16"
            );
            self.rpc(
                &command.method,
                json!({"textDocument":{"uri":uri},"options":options}),
                true,
            )
            .await?
        } else {
            let position: tower_lsp::lsp_types::Position =
                serde_json::from_value(command.argument.clone())
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
        command.control.check()?;
        Ok(
            json!({"source_hash":hash,"source_mode":command.source_mode,"version":version,"result":result,"server":self.capabilities["serverInfo"]}),
        )
    }
}

#[cfg(test)]
#[path = "../tests/language/cancellation.rs"]
mod cancellation_tests;
