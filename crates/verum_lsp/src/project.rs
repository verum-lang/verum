//! Revision-bound projections of the language server's actual analysis.
//! Architecture consumers share AST identities with diagnostics/navigation;
//! a declaration graph is not a proof of runtime implementation correspondence.
use crate::{
    Backend, DocumentState, diagnostics::convert_diagnostics, position_utils::ast_span_to_range,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower_lsp::{
    jsonrpc,
    lsp_types::{Diagnostic, Range, TextDocumentIdentifier, Url},
};
use verum_ast::{ItemKind, Span, decl::TypeDeclBody};
use verum_common::{List, Text};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotParams {
    pub text_document: TextDocumentIdentifier,
    pub version: i32,
}

#[derive(Debug, Serialize)]
pub struct Declaration {
    pub id: Text,
    pub name: Text,
    pub kind: &'static str,
    pub range: Range,
    pub selection_range: Range,
    pub requirements: List<Requirement>,
}
#[derive(Debug, Serialize)]
pub struct Requirement {
    pub source: Text,
    pub range: Range,
}
#[derive(Debug, Serialize)]
pub struct DocumentSnapshot {
    pub schema: &'static str,
    pub uri: Url,
    pub version: i32,
    pub source_hash: Text,
    pub parsed: bool,
    pub analysis_scope: &'static str,
    pub diagnostics: List<Diagnostic>,
    pub declarations: List<Declaration>,
    pub mounts: List<Requirement>,
}

impl DocumentSnapshot {
    pub fn from_document(uri: &Url, doc: &DocumentState) -> Self {
        let mut declarations = List::new();
        let mut mounts = List::new();
        let requirement = |span: Span| Requirement {
            source: doc
                .text
                .get(span.start as usize..span.end as usize)
                .unwrap_or("")
                .into(),
            range: ast_span_to_range(&span, &doc.text),
        };
        if let Some(module) = &doc.module {
            for item in &module.items {
                let (name, kind, requirements) = match &item.kind {
                    ItemKind::Function(decl) => (
                        &decl.name,
                        "function",
                        decl.contexts
                            .iter()
                            .map(|context| requirement(context.span))
                            .collect(),
                    ),
                    ItemKind::Type(decl) => (
                        &decl.name,
                        match &decl.body {
                            TypeDeclBody::Protocol(body) if body.is_context => "context",
                            TypeDeclBody::Protocol(_) => "protocol",
                            TypeDeclBody::Record(_) => "record",
                            TypeDeclBody::Variant(_) => "variant",
                            _ => "type",
                        },
                        List::new(),
                    ),
                    ItemKind::Context(decl) => (&decl.name, "context", List::new()),
                    ItemKind::Protocol(decl) => (&decl.name, "protocol", List::new()),
                    ItemKind::Const(decl) => (&decl.name, "constant", List::new()),
                    ItemKind::Module(decl) => (&decl.name, "module", List::new()),
                    ItemKind::Mount(_) => {
                        mounts.push(requirement(item.span));
                        continue;
                    }
                    _ => continue,
                };
                declarations.push(Declaration {
                    id: format!("{}#{}:{}:{}", uri, item.span.start, item.span.end, kind).into(),
                    name: name.as_str().into(),
                    kind,
                    range: ast_span_to_range(&item.span, &doc.text),
                    selection_range: ast_span_to_range(&name.span, &doc.text),
                    requirements,
                });
            }
        }
        Self {
            schema: "verum-document-analysis/1",
            uri: uri.clone(),
            version: doc.version,
            source_hash: hex::encode(Sha256::digest(doc.text.as_bytes())).into(),
            parsed: doc.module.is_some(),
            analysis_scope: "single-module-with-builtins",
            diagnostics: convert_diagnostics(doc.diagnostics.clone(), &doc.text, uri).into(),
            declarations,
            mounts,
        }
    }
}

impl Backend {
    pub async fn handle_project_snapshot(
        &self,
        _: serde_json::Value,
    ) -> jsonrpc::Result<serde_json::Value> {
        let mut documents = self.documents.flat_collect_from_documents(|uri, doc| {
            vec![DocumentSnapshot::from_document(uri, doc)]
        });
        documents.sort_by(|left, right| left.uri.cmp(&right.uri));
        serde_json::to_value(serde_json::json!({
            "schema":"verum-project-analysis/1", "coverage":"analyzed-documents-only", "documents":documents
        })).map_err(|_| jsonrpc::Error::internal_error())
    }
    pub async fn handle_document_snapshot(
        &self,
        params: SnapshotParams,
    ) -> jsonrpc::Result<serde_json::Value> {
        self.documents
            .with_document(&params.text_document.uri, |doc| {
                if doc.version != params.version {
                    return Err(jsonrpc::Error::content_modified());
                }
                serde_json::to_value(DocumentSnapshot::from_document(
                    &params.text_document.uri,
                    doc,
                ))
                .map_err(|_| jsonrpc::Error::internal_error())
            })
            .unwrap_or_else(|| Err(jsonrpc::Error::invalid_params("Document is not open")))
    }
}
