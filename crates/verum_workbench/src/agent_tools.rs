//! Reviewed Verum operations advertised to providers and the Studio client.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::sync::OnceLock;

use crate::language::DOCUMENT_QUERIES;

enum Route {
    Project,
    ReadSource,
    Propose,
    Receipt,
    Evidence,
    Plan,
    Files,
    Changes,
    Analyze,
    Lint,
    Format,
    Start(&'static str),
}

struct Tool {
    name: &'static str,
    route: Route,
    definition: Value,
}

fn tools() -> &'static [Tool] {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(define_tools)
}

fn find(name: &str) -> Option<&'static Tool> {
    tools().iter().find(|tool| tool.name == name)
}

impl Tool {
    fn review_kind(&self) -> &'static str {
        match self.route {
            Route::Propose => "source",
            Route::Start(_) => "execution",
            Route::Project
            | Route::ReadSource
            | Route::Evidence
            | Route::Receipt
            | Route::Plan
            | Route::Files
            | Route::Changes
            | Route::Analyze
            | Route::Lint
            | Route::Format => "read",
        }
    }
}

pub(crate) fn review_kind(name: &str) -> Option<&'static str> {
    find(name).map(Tool::review_kind)
}

#[derive(Clone, Copy, Debug, serde::Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Role {
    #[default]
    Author,
    Reviewer,
}

impl Role {
    fn permits(self, kind: &str) -> bool {
        self == Self::Author || kind == "read"
    }

    pub(super) fn review_kind(self, name: &str) -> Option<&'static str> {
        review_kind(name).filter(|kind| self.permits(kind))
    }

    pub(super) fn catalog(self) -> Value {
        Value::Array(
            tools()
                .iter()
                .filter(|tool| self.permits(tool.review_kind()))
                .map(|tool| tool.definition.clone())
                .collect(),
        )
    }
}

/// Host capabilities; individual conversations receive only their role's tools.
pub fn catalog() -> Value {
    Role::Author.catalog()
}

/// Translate an approved request into an existing Studio operation. The caller retains
/// the approval boundary; a provider cannot select an arbitrary Studio method.
pub(super) fn operation(params: &Value) -> Result<(&'static str, Value)> {
    let name = super::field(params, "tool")?;
    let tool = find(name).context("Unknown Verum tool")?;
    let mut args = params["arguments"].clone();
    let fields = args.as_object().context("Expected tool arguments object")?;
    let schema = &tool.definition["inputSchema"];
    for field in fields.keys() {
        ensure!(
            schema["properties"].get(field).is_some(),
            "Unexpected {name} argument: {field}"
        );
    }
    for required in schema["required"]
        .as_array()
        .expect("tool schema required list")
    {
        let key = required.as_str().expect("tool schema field name");
        ensure!(fields.contains_key(key), "{name} requires {key}");
    }
    let method = match tool.route {
        Route::Project => "project.inspect",
        Route::Lint => "workspace.lint",
        Route::ReadSource => "workspace.read",
        Route::Propose => "workspace.write",
        Route::Receipt => "operation.read",
        Route::Evidence => "evidence.packet",
        Route::Plan => "operation.plan",
        Route::Files => {
            ensure!(
                args.get("query").is_none() || args.get("path").is_none(),
                "Choose a directory path or a project-wide query"
            );
            if args.get("query").is_some() {
                "workspace.search"
            } else {
                "workspace.browse"
            }
        }
        Route::Changes => {
            ensure!(
                args.get("path").is_some() == args.get("side").is_some(),
                "A Git patch requires both path and side"
            );
            if args.get("path").is_some() {
                "workspace.diff"
            } else {
                "workspace.changes"
            }
        }
        Route::Analyze => {
            let method = args
                .get("method")
                .map(|value| value.as_str().context("Expected language method"))
                .transpose()?
                .unwrap_or("analyze");
            ensure!(
                DOCUMENT_QUERIES.contains(&method),
                "Unsupported analysis method"
            );
            ensure!(
                args.get("position").is_some() == (method != "analyze"),
                "A position is required only for hover, definition and completion"
            );
            "language.request"
        }
        Route::Format => {
            args["method"] = json!("textDocument/formatting");
            if args.get("options").is_none() {
                args["options"] = json!({"tabSize":4,"insertSpaces":true});
            }
            "language.request"
        }
        Route::Start(kind) => {
            args["kind"] = json!(kind);
            "operation.start"
        }
    };
    Ok((method, args))
}

fn define_tools() -> Vec<Tool> {
    let source = json!({"type":"string","description":"Complete Verum source for this revision"});
    let permissions = json!({"type":"string","enum":["source","deny-all","allow-all"],"description":"Script permission mode: source follows CLI/frontmatter defaults; deny-all clears grants; allow-all grants all kinds. Not an OS sandbox."});
    let timeout = json!({"type":"integer","minimum":1,"maximum":300000,"description":"Wall time budget in milliseconds; default 60000"});
    let runtime_diagnostics = json!({"type":"boolean","description":"Record bounded CBGR errors, VBC method dispatch counts and inclusive handler times, and recent cooperative task outcomes including detached failures, bound to this invocation and source. Capture is limited to interpreter execution; nursery jobs and native worker threads are excluded. Default false."});
    let proof_profile = json!({"type":"string","enum":verum_source_proof::SourceProfile::ALL,"description":"Source proof scope: logical theorem (default), or closed finite function contracts with proved helper-call obligations. Neither proves native compiler correspondence."});
    [
        (
            "verum_files", Route::Files,
            "Browse a project directory lazily (path defaults to the root), or search project file paths with query. Includes source and supporting files; hidden, generated and symlink paths are excluded. Search reports truncation explicitly.",
            json!({"path":{"type":"string"},"query":{"type":"string"}}),
            json!([]),
        ),
        (
            "verum_changes", Route::Changes,
            "Inspect read-only Git status with separate index and working-tree states. To review a patch, supply path and side (index or worktree). Does not stage, commit, reset or change files.",
            json!({"path":{"type":"string"},"side":{"type":"string","enum":["index","worktree"]}}),
            json!([]),
        ),
        (
            "verum_project", Route::Project,
            "Inspect workspace source inventory and revision-bound analyzed documents shared with Studio. Coverage is analyzed documents only, not a whole-project build or proof.",
            json!({}),
            json!([]),
        ),
        (
            "verum_lint", Route::Lint,
            "Inspect saved Verum sources across workspace packages. Each file carries its SHA-256, parser coverage, findings and suggested UTF-16 edits. Review edits against that revision before proposing changes.",
            json!({}), json!([]),
        ),
        (
            "verum_analyze", Route::Analyze,
            "Inspect a source revision through Studio's language service. The default analysis reports diagnostics, declarations, context requirements and coverage. Hover, definition and completion require a zero-based LSP UTF-16 position. Omit source to use the saved file.",
            json!({"path":{"type":"string"},"source":source,"method":{"type":"string","enum":DOCUMENT_QUERIES},"position":{"type":"object","properties":{"line":{"type":"integer","minimum":0},"character":{"type":"integer","minimum":0}},"required":["line","character"],"additionalProperties":false}}),
            json!(["path"]),
        ),
        (
            "verum_format", Route::Format,
            "Return revision-bound formatting edits from Studio's shared Verum LSP. Does not write files. Omit source to format the saved file; propose the resulting source separately for review.",
            json!({"path":{"type":"string"},"source":source,"options":{"type":"object","properties":{"tabSize":{"type":"integer","minimum":1,"maximum":16},"insertSpaces":{"type":"boolean"}},"required":["tabSize","insertSpaces"],"additionalProperties":false}}),
            json!(["path"]),
        ),
        (
            "verum_read", Route::ReadSource,
            "Read a .vr source in the open workspace",
            json!({"path":{"type":"string"}}),
            json!(["path"]),
        ),
        (
            "verum_propose", Route::Propose,
            "Propose a source replacement for human review; compare-and-swap against expected_hash",
            json!({"path":{"type":"string"},"source":source,"expected_hash":{"type":["string","null"]}}),
            json!(["path", "source", "expected_hash"]),
        ),
        (
            "verum_evidence", Route::Evidence,
            "Inspect compact compiler evidence at a pinned source revision. Take declaration_id and project revision from verum_analyze. Attach an exact operation ID for input-source correspondence, or a checker-bound span for a selected declaration. The operation does not attest to the compiler project revision.",
            json!({"path":{"type":"string"},"expected_source_hash":{"type":"string","pattern":"^[A-Fa-f0-9]{64}$"},"expected_project_revision":{"type":"string"},"declaration_id":{"type":"string"},"operation_id":{"type":"string"}}),
            json!(["path", "expected_source_hash"]),
        ),
        (
            "verum_receipt", Route::Receipt,
            "Read one retained Studio operation by its exact ID, including status, bounded output and recorded proof or CBGR evidence. Supply expected_source_hash to require the recorded input source bytes to have that SHA-256 digest.",
            json!({"id":{"type":"string"},"expected_source_hash":{"type":"string","pattern":"^[0-9A-Fa-f]{64}$"}}),
            json!(["id"]),
        ),
        (
            "verum_plan", Route::Plan,
            "Inspect the exact execution contract without launching a process or writing run evidence. Run permission overrides apply only to scripts.",
            json!({"kind":{"type":"string","enum":["run","check","proof"]},"path":{"type":"string"},"source":source,"permissions":permissions,"runtime_diagnostics":runtime_diagnostics,"timeout_ms":timeout,"declaration":{"type":"string"},"proof_profile":proof_profile}),
            json!(["kind", "path", "source"]),
        ),
        (
            "verum_run", Route::Start("run"),
            "Run exact source with Verum interpreter after user approval",
            json!({"path":{"type":"string"},"source":source,"permissions":permissions,"runtime_diagnostics":runtime_diagnostics,"timeout_ms":timeout}),
            json!(["path", "source"]),
        ),
        (
            "verum_check", Route::Start("check"),
            "Type-check exact Verum source after user approval",
            json!({"path":{"type":"string"},"source":source,"timeout_ms":timeout}),
            json!(["path", "source"]),
        ),
        (
            "verum_proof", Route::Start("proof"),
            "Check a logical theorem or finite source function contract using the selected proof_profile; not native implementation correspondence",
            json!({"path":{"type":"string"},"source":source,"declaration":{"type":"string"},"timeout_ms":timeout,"proof_profile":proof_profile}),
            json!(["path", "source", "declaration"]),
        ),
    ].into_iter().map(|(name, route, description, mut properties, required)| {
        if matches!(&route, Route::Analyze | Route::Format | Route::Evidence | Route::Plan | Route::Start("run" | "check")) {
            properties["source_mode"] = json!({"type":"string","enum":["source","script"],"description":"Parsing policy shared with execution: source (default) honors modules and shebang scripts; script permits top-level statements only in this entry."});
        }
        Tool {
            name, route,
            definition: json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}),
        }
    }).collect()
}

#[cfg(test)]
mod tests;
