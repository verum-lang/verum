//! Verum Studio: one local operation service, with browser and provider adapters.
pub mod codex;
pub mod jobs;
pub mod server;
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
    agent: Mutex<Option<Arc<codex::Codex>>>,
    pub config: Config,
}
impl Studio {
    pub fn open(mut config: Config) -> Result<Self> {
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
            "workspace.list" => self.workspace.lock().await.list(),
            "workspace.read" => self.workspace.lock().await.read(field(&params, "path")?),
            "workspace.write" => {
                ensure!(
                    params.get("expected_hash").is_some(),
                    "expected_hash is required (null only for a new file)"
                );
                self.workspace.lock().await.write(
                    field(&params, "path")?,
                    field(&params, "source")?,
                    params["expected_hash"].as_str(),
                )
            }
            "operation.start" => {
                let path = self.workspace.lock().await.path(field(&params, "path")?)?;
                let mut jobs = self.jobs.lock().await;
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
                jobs.start(&params, path)
            }
            "operation.list" => Ok(self.jobs.lock().await.changes(&params["known"]).await),
            "operation.cancel" => self.jobs.lock().await.cancel(field(&params, "id")?).await,
            "agent.connect" => {
                // A new connection never borrows another application's auth files.
                let mut provider = self.agent.lock().await;
                if provider.as_ref().is_none_or(|p| !p.connected()) {
                    *provider = Some(Arc::new(
                        codex::Codex::connect(&self.config.codex, &self.config.workspace).await?,
                    ));
                }
                provider
                    .as_ref()
                    .context("Codex")?
                    .request("account/read", json!({"refreshToken":false}))
                    .await
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
                .read(params["after"].as_u64().unwrap_or(0))),
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
                if method == "thread/start" {
                    let model = args.get("model").cloned();
                    args = json!({"cwd":self.config.workspace,"sandbox":"read-only","approvalPolicy":"untrusted",
                        "baseInstructions":"You are the Verum Studio research assistant. Work only in Verum. Use verum_read and propose complete .vr edits through verum_propose for human review. Execute and check only through verum_run, verum_check and verum_proof. Never substitute Python, Rust, JavaScript, a shell, or another language for a Verum experiment. Never treat passing tests as a theorem or logical proof as runtime implementation correspondence. Cite source hashes and exact evidence, and preserve rejected/exhausted/inconclusive outcomes. Read files before proposing changes. Ask the user to select a file if no path is known. Tools require human approval.",
                        "dynamicTools":codex::tools()});
                    if let Some(model) = model {
                        args["model"] = model;
                    }
                } else if method == "turn/start" {
                    // Client controls conversation input, not sandbox or execution policy.
                    args = json!({"threadId":args["threadId"],"input":args["input"]});
                }
                self.provider().await?.request(method, args).await
            }
            "agent.respond" => self.respond(params).await,
            _ => bail!("Unknown Studio method"),
        }
    }
    async fn respond(&self, params: Value) -> Result<Value> {
        let provider = self.provider().await?;
        let id = params.get("id").context("Missing request id")?;
        let request = provider
            .events
            .lock()
            .await
            .requests
            .remove(&Text::from(id.to_string()))
            .context("This approval has expired or was already answered")?;
        if request["method"] != "item/tool/call" {
            // No generic shell/permission escalation in a Verum-only research workspace.
            provider.send(json!({"id":id,"error":{"code":-32601,"message":"Studio exposes only reviewed Verum tools"}})).await?;
            return Ok(json!({"declined":true}));
        }
        let result = if params["result"]["approved"].as_bool() != Some(true) {
            Err(anyhow::anyhow!("User declined this operation"))
        } else {
            self.tool(&request["params"]).await
        };
        let (success, tool_result) = match result {
            Ok(value) => (true, value),
            Err(error) => (false, json!({"error":error.to_string()})),
        };
        let content = tool_result.to_string();
        provider.send(json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":content}]}})).await?;
        Ok(json!({"delivered":true,"success":success,"tool_result":tool_result}))
    }
    async fn tool(&self, params: &Value) -> Result<Value> {
        let tool = field(params, "tool")?;
        let mut args = params["arguments"].clone();
        match tool {
            "verum_read" => self.workspace.lock().await.read(field(&args, "path")?),
            "verum_propose" => {
                ensure!(
                    args.get("expected_hash").is_some(),
                    "Proposal requires expected_hash"
                );
                self.workspace.lock().await.write(
                    field(&args, "path")?,
                    field(&args, "source")?,
                    args["expected_hash"].as_str(),
                )
            }
            "verum_run" | "verum_check" | "verum_proof" => {
                args["kind"] = json!(tool.strip_prefix("verum_").context("Tool prefix")?);
                let path = self.workspace.lock().await.path(field(&args, "path")?)?;
                let id = {
                    let mut jobs = self.jobs.lock().await;
                    ensure!(
                        jobs.list()
                            .await
                            .as_array()
                            .context("jobs")?
                            .iter()
                            .filter(|r| r["status"] == "running")
                            .count()
                            < 2,
                        "Two runs already active"
                    );
                    jobs.start(&args, path)?["id"].clone()
                };
                loop {
                    if let Some(receipt) = self.jobs.lock().await.read(&id).await
                        && receipt.status != "running"
                    {
                        return Ok(serde_json::to_value(receipt)?);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            _ => bail!("Unknown Verum tool"),
        }
    }
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .with_context(|| format!("Expected {name}"))
}
