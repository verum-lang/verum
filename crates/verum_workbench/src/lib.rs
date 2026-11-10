//! Verum Studio: one local operation service, with browser and provider adapters.
pub mod agent_tools;
pub mod agent_runtime;
pub mod codex;
mod evidence;
pub mod execution;
pub mod jobs;
pub mod language;
pub mod project_files;
pub mod project_git;
pub mod server;
pub mod runtime_diagnostics;
pub mod workspace;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use verum_common::{List, Text};

pub struct Config {
    pub workspace: PathBuf,
    pub verum: PathBuf,
    pub proof: PathBuf,
    pub codex: PathBuf,
    pub port: u16,
}
pub struct Studio {
    pub workspace: Mutex<workspace::Workspace>,
    pub jobs: Mutex<jobs::Jobs>,
    language: language::Language,
    agent: Mutex<Option<Arc<codex::Codex>>>,
    pub config: Config,
}
impl Studio {
    pub fn open(config: Config) -> Result<Self> {
        Self::open_with_library(config, None)
    }
    pub fn open_with_library(
        mut config: Config,
        library: Option<Arc<verum_lsp::library_metadata::LibraryMetadata>>,
    ) -> Result<Self> {
        for executable in [&mut config.verum, &mut config.proof, &mut config.codex] {
            if executable.is_relative() && executable.components().count() > 1 {
                *executable = std::env::current_dir()?.join(&*executable);
            }
        }
        let workspace = workspace::Workspace::open(&config.workspace)?;
        config.workspace = workspace.root.clone();
        let mut jobs = jobs::Jobs {
            jobs: List::new(),
            verum: config.verum.clone(),
            proof: config.proof.clone(),
            root: workspace.root.clone(),
        };
        jobs.restore()?;
        Ok(Self {
            language: language::Language::start_with_library(
                workspace.root.clone(),
                library,
                Some(config.verum.clone()),
            )?,
            workspace: Mutex::new(workspace),
            jobs: Mutex::new(jobs),
            agent: Mutex::new(None),
            config,
        })
    }
    async fn provider(&self) -> Result<Arc<codex::Codex>> {
        self.agent
            .lock()
            .await
            .clone()
            .context("Connect Codex first")
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "agent.tools" => Ok(agent_tools::catalog()),
            "agent.connect" => {
                // A new connection never borrows another application's auth files.
                let mut provider = self.agent.lock().await;
                if provider.as_ref().is_none_or(|p| !p.connected()) {
                    *provider = Some(Arc::new(
                        codex::Codex::connect(&self.config.codex, &self.config.workspace).await?,
                    ));
                }
                let provider = provider.as_ref().context("Codex")?;
                let mut account = provider.request("account/read", json!({"refreshToken":false})).await?;
                account["connectionId"] = json!(provider.events.lock().await.connection().to_string());
                Ok(account)
            }
            "agent.login" => {
                self.provider()
                    .await?
                    .request("account/login/start", json!({"type":"chatgpt"}))
                    .await
            }
            "agent.events" => Ok(self
                .provider()
                .await?
                .events
                .lock()
                .await
                .read(params["after"].as_u64().unwrap_or(0), params["threadId"].as_str())),
            "agent.request" => {
                let method = field(&params, "method")?;
                let mut args = params["params"].clone();
                ensure!(args.is_object(), "Agent params must be an object");
                ensure!(
                    matches!(
                        method,
                        "thread/start"
                            | "turn/start"
                            | "turn/interrupt"
                            | "model/list"
                            | "account/read"
                    ),
                    "Unsupported agent operation"
                );
                let provider = self.provider().await?;
                if let Some(expected) = params.get("connectionId") {
                    ensure!(
                        expected == &json!(provider.events.lock().await.connection().to_string()),
                        "Provider connection changed; reconnect before sending a request"
                    );
                }
                if method == "thread/start" {
                    let role: agent_tools::Role = params.get("role").map(|value| serde_json::from_value(value.clone()))
                        .transpose().context("Expected author or reviewer role")?.unwrap_or_default();
                    let instructions = match role {
                        agent_tools::Role::Author => "You are the Verum Studio research assistant. Work only in Verum. Use verum_analyze for revision-bound diagnostics, declarations and context requirements from the same language service as the editor. Read the reported analysis_scope and library_metadata identity: captured dependency declarations and available release metadata inform open-document checks; unopened dependency bodies are not whole-program verification. Use verum_read and propose complete .vr edits through verum_propose for human review. Execute and check only through verum_run, verum_check and verum_proof. Never substitute Python, Rust, JavaScript, a shell, or another language for a Verum experiment. Never treat passing tests as a theorem or logical proof as runtime implementation correspondence. Cite source hashes and exact evidence, and preserve rejected/exhausted/inconclusive outcomes. Read files before proposing changes. Ask the user to select a file if no path is known. Tools require human approval.",
                        agent_tools::Role::Reviewer => "Independently review Verum source and explicitly identified Studio receipts using only the available read tools. Check the source hash, analysis scope and proof outcome. Distinguish a verified theorem from implementation correspondence and a passing execution. Report reproducible findings and missing evidence. Request the relevant paths and receipt IDs from the user when needed.",
                    };
                    let model = args.get("model").cloned();
                    args = json!({"cwd":self.config.workspace,"sandbox":"read-only","approvalPolicy":"untrusted",
                        "baseInstructions":instructions,
                        "dynamicTools":role.catalog()});
                    if let Some(model) = model {
                        args["model"] = model;
                    }
                    return provider.start_thread(role, args).await;
                }
                ensure!(params.get("role").is_none(), "Conversation role is fixed at creation");
                if matches!(method, "turn/start" | "turn/interrupt") {
                    ensure!(provider.events.lock().await.role(field(&args, "threadId")?).is_some(), "Unknown or retired conversation");
                }
                if method == "turn/start" {
                    // Client controls conversation input, not sandbox or execution policy.
                    args = json!({"threadId":args["threadId"],"input":args["input"]});
                }
                if method == "turn/interrupt" {
                    // Revoke local authority before waiting for a provider acknowledgement.
                    provider.events.lock().await.end_turn(
                        field(&args, "threadId")?, field(&args, "turnId")?, agent_runtime::End::Interrupted,
                    )?;
                }
                provider.request(method, args).await
            }
            "agent.respond" => self.respond(params).await,
            _ => self.workspace_operation(method, params).await,
        }
    }
    async fn respond(&self, params: Value) -> Result<Value> {
        let provider = self.provider().await?;
        let thread = params["threadId"].as_str();
        let (request, role, scope) = {
            let mut events = provider.events.lock().await;
            let (request, scope) = events.take_review(field(&params, "reviewId")?, thread)?;
            (request, thread.and_then(|id| events.role(id)), scope)
        };
        let _guard = scope.as_ref().map(agent_runtime::Scope::guard);
        let id = &request["id"];
        if request["method"] != "item/tool/call" {
            // No generic shell/permission escalation in a Verum-only research workspace.
            provider.send(json!({"id":id,"error":{"code":-32601,"message":"Studio exposes only reviewed Verum tools"}})).await?;
            return Ok(json!({"declined":true}));
        }
        let result = if params["result"]["approved"].as_bool() != Some(true) {
            Err(anyhow::anyhow!("User declined this operation"))
        } else if role.and_then(|role| role.review_kind(request["params"]["tool"].as_str()?)).is_none() {
            Err(anyhow::anyhow!("This conversation is not authorized to invoke the requested tool"))
        } else {
            match &scope {
                Some(scope) => self.tool_owned(&request["params"], Some(scope)).await,
                None => Err(anyhow::anyhow!("Agent operation has no live turn owner")),
            }
        };
        let (success, tool_result) = match result {
            Ok(value) => (true, value),
            Err(error) => (false, json!({"error":error.to_string()})),
        };
        let content = tool_result.to_string();
        // Keep the provider's wire ID reserved until its response is written.
        let mut events = provider.events.lock().await;
        if let Some(scope) = &scope
            && !events.finish_review(id, scope)
        {
            return Ok(json!({"delivered":false,"success":false,"cancelled":true,"tool_result":tool_result}));
        }
        provider.send(json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":content}]}})).await?;
        Ok(json!({"delivered":true,"success":success,"tool_result":tool_result}))
    }
    #[cfg(test)]
    async fn tool(&self, params: &Value) -> Result<Value> {
        self.tool_owned(params, None).await
    }
    async fn tool_owned(&self, params: &Value, owner: Option<&agent_runtime::Scope>) -> Result<Value> {
        let (method, args) = agent_tools::operation(params)?;
        if let Some(owner) = owner { owner.admit(|| Ok(()))?; }
        let operation = self.workspace_operation_owned(method, args, owner);
        let result = if let Some(owner) = owner {
            tokio::select! {
                biased;
                reason = owner.cancelled() => bail!("{}", reason.reason()),
                result = operation => result?,
            }
        } else { operation.await? };
        if method != "operation.start" {
            return Ok(result);
        }
        loop {
            let receipt = self.jobs.lock().await.read(&result["id"]).await
                .context("Operation receipt is unavailable")?;
            if receipt.status != "running" {
                return Ok(serde_json::to_value(receipt)?);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn workspace_operation(&self, method: &str, params: Value) -> Result<Value> {
        self.workspace_operation_owned(method, params, None).await
    }
    async fn workspace_operation_owned(&self, method: &str, params: Value, owner: Option<&agent_runtime::Scope>) -> Result<Value> {
        match method {
            "workspace.browse" | "workspace.search" | "workspace.changes" | "workspace.diff" => {
                self.project_files(method, params).await
            }
            "document.read" => self
                .workspace
                .lock()
                .await
                .read_document(field(&params, "path")?),
            "document.write" => {
                self.workspace.lock().await.write_document(
                    field(&params, "path")?,
                    field(&params, "source")?,
                    expected_hash(&params)?,
                )
            }
            "workspace.list" => self.workspace.lock().await.list(),
            "workspace.lint" => {
                let report = verum_lsp::lint_report::inspect(&self.config.verum, &self.config.workspace, None).await?;
                for document in &report.files {
                    ensure!(document.path.to_str().is_some_and(project_files::exposed)
                        && document.path.extension().is_some_and(|ext| ext == "vr"), "Lint reported a non-workspace source");
                }
                Ok(serde_json::to_value(report.as_ref())?)
            }
            "language.request" => self.language_request(&params).await,
            "project.inspect" => self.project().await,
            "evidence.packet" => self.evidence_packet(params).await,
            "workspace.read" => self.workspace.lock().await.read(field(&params, "path")?),
            "workspace.write" => {
                let workspace = self.workspace.lock().await;
                let write = || workspace.write(
                    field(&params, "path")?,
                    field(&params, "source")?,
                    expected_hash(&params)?,
                );
                match owner {
                    Some(owner) => owner.admit(write),
                    None => write(),
                }
            }
            "operation.plan" | "operation.start" => {
                let path = self.workspace.lock().await.path(field(&params, "path")?)?;
                let mut jobs = self.jobs.lock().await;
                if method == "operation.plan" {
                    return Ok(serde_json::to_value(jobs.plan(&params)?)?);
                }
                let running = jobs
                    .list()
                    .await
                    .as_array()
                    .context("jobs")?
                    .iter()
                    .filter(|r| r["status"] == "running")
                    .count();
                ensure!(
                    running < 2,
                    "Two operations are already running; wait or cancel one"
                );
                match owner {
                    Some(owner) => jobs.start_owned(&params, path, owner),
                    None => jobs.start(&params, path),
                }
            }
            "operation.read" => {
                let id = &params["id"];
                ensure!(id.is_string(), "Expected operation ID");
                let receipt = self.jobs.lock().await.read(id).await.context("Unknown operation")?;
                if let Some(expected) = params.get("expected_source_hash") {
                    let expected = expected.as_str().context("Expected a SHA-256 source hash")?;
                    ensure!(expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()), "Invalid SHA-256 source hash");
                    ensure!(receipt.plan.source_hash.eq_ignore_ascii_case(expected), "Receipt source hash does not match the expected source");
                }
                Ok(serde_json::to_value(receipt)?)
            }
            "operation.list" => Ok(self.jobs.lock().await.changes(&params["known"]).await),
            "operation.cancel" => self.jobs.lock().await.cancel(field(&params, "id")?).await,
            _ => bail!("Unknown Studio method"),
        }
    }
    async fn language_request(&self, params: &Value) -> Result<Value> {
        let draft = params.get("source").map(|value| value.as_str().map(Text::from).context("Expected source text")).transpose()?;
        let (path, source) = self.workspace.lock().await.source_input(field(params, "path")?, draft)?;
        self.language
            .request_with_source_mode(
                path,
                source,
                params["method"].as_str().unwrap_or("analyze"),
                params[if params["method"] == "textDocument/formatting" {
                    "options"
                } else {
                    "position"
                }]
                .clone(),
                params.get("source_mode").map(|value| serde_json::from_value(value.clone())).transpose()?.unwrap_or_default(),
            )
            .await
    }
    async fn project(&self) -> Result<Value> {
        let files = self.workspace.lock().await.list()?;
        let analysis = self
            .language
            .request(
                self.config.workspace.clone(),
                Text::new(),
                "project",
                Value::Null,
            )
            .await?;
        Ok(json!({"workspace":files,"analysis":analysis}))
    }
    async fn project_files(&self, method: &str, params: Value) -> Result<Value> {
        let root = self.workspace.lock().await.root.clone();
        match method {
            "workspace.changes" => Ok(serde_json::to_value(project_git::changes(&root).await)?),
            "workspace.diff" => Ok(
                json!({"path":field(&params,"path")?,"side":field(&params,"side")?,"patch":project_git::diff(&root, field(&params,"path")?, field(&params,"side")?).await?}),
            ),
            _ => {
                let search = method == "workspace.search";
                tokio::task::spawn_blocking(move || {
                    let workspace = workspace::Workspace::open(&root)?;
                    let result = if search {
                        workspace.search_files(field(&params, "query")?)?
                    } else {
                        workspace.browse(
                            params
                                .get("path")
                                .map(|_| field(&params, "path"))
                                .transpose()?
                                .unwrap_or(""),
                        )?
                    };
                    Ok(serde_json::to_value(result)?)
                })
                .await?
            }
        }
    }
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .with_context(|| format!("Expected {name}"))
}
fn expected_hash(value: &Value) -> Result<Option<&str>> {
    let expected = value.get("expected_hash").context("expected_hash is required (null only for a new file)")?;
    if expected.is_null() { Ok(None) }
    else { Ok(Some(expected.as_str().context("expected_hash must be a string or null")?)) }
}
