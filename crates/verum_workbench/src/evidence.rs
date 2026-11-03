//! A small handoff between the compiler's document evidence and an exact Studio run.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Studio, execution::OperationKind, jobs::Receipt};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    source_mode: verum_common::source_mode::SourceMode,
    path: String,
    expected_source_hash: String,
    expected_project_revision: Option<String>,
    declaration_id: Option<String>,
    operation_id: Option<String>,
    source: Option<String>,
}

impl Studio {
    pub(super) async fn evidence_packet(&self, params: Value) -> Result<Value> {
        let request: Request = serde_json::from_value(params)?;
        let (path, source) = self
            .workspace
            .lock()
            .await
            .source_input(&request.path, request.source.map(Into::into))?;
        let analysis = self
            .language
            .request_with_source_mode(
                path,
                source,
                "evidence",
                json!({
                    "expectedSourceHash":request.expected_source_hash,
                    "expectedProjectRevision":request.expected_project_revision,
                    "declarationId":request.declaration_id,
                }),
                request.source_mode,
            )
            .await?;
        let operation = if let Some(id) = request.operation_id {
            let jobs = self.jobs.lock().await;
            Some(jobs.evidence(&id, &request.path, &analysis).await?)
        } else {
            None
        };
        Ok(
            json!({"schema":"verum-evidence-packet/1","path":request.path,"analysis":analysis,"operation":operation}),
        )
    }
}

impl crate::jobs::Jobs {
    // Project the result under the receipt lock. Raw logs, runtime traces and proof terms
    // may be large and are already available via operation.read.
    async fn evidence(&self, id: &str, path: &str, analysis: &Value) -> Result<Value> {
        for job in &self.jobs {
            let receipt = job.receipt.lock().await;
            if receipt.id == id {
                return operation_evidence(&receipt, path, analysis);
            }
        }
        anyhow::bail!("Unknown operation")
    }
}

fn operation_evidence(receipt: &Receipt, path: &str, analysis: &Value) -> Result<Value> {
    let plan = &receipt.plan;
    ensure!(
        std::path::Path::new(plan.path.as_str()) == std::path::Path::new(path)
            && analysis["source_hash"] == plan.source_hash.as_str()
            && plan.source_mode.is_some_and(|mode| analysis["source_mode"] == mode.as_str()),
        "Operation belongs to another document, source revision or parsing mode"
    );
    let relation = if !analysis["declaration"].is_null() {
        let binding = receipt
            .proof
            .as_ref()
            .and_then(|p| p.get("source"))
            .context("Operation has no checker binding for this declaration")?;
        let name = analysis["declaration"]["name"]
            .as_str()
            .context("Missing declaration name")?;
        ensure!(
            plan.kind == OperationKind::Proof
                && plan.declaration.as_deref() == Some(name)
                && binding["declaration"] == name
                && binding["sha256"] == plan.source_hash.as_str()
                && !analysis["declaration_span"].is_null()
                && binding["span"] == analysis["declaration_span"],
            "Operation does not identify this exact declaration"
        );
        "checker-source-span"
    } else {
        "input-source"
    };
    Ok(json!({
        "id":receipt.id, "version":receipt.version, "kind":plan.kind, "status":receipt.status,
        "relation":relation, "path":plan.path, "source_hash":plan.source_hash,
        "declaration":plan.declaration, "proof_profile":plan.proof_profile,
        "termination":receipt.termination, "exit_code":receipt.exit_code,
        "project_revision":null
    }))
}
