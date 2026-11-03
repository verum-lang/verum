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
use verum_ast::{ItemKind, Span, decl::TypeDeclBody, expr::{BinOp, ExprKind}, ty::PathSegment};
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
    pub type_display: Option<Text>,
    pub range: Range,
    pub selection_range: Range,
    pub requirements: List<Requirement>,
    pub preconditions: List<Requirement>,
    pub postconditions: List<Requirement>,
    pub architecture: List<ArchitectureAnnotation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_fields: Option<Vec<RecordMember>>,
}
#[derive(Debug, Serialize)]
pub struct RecordMember {
    pub name: Text,
    pub selection_range: Range,
    pub annotation: Requirement,
    /// Earlier field ordinals from the compiler. None means analysis could
    /// not establish the dependencies for this revision.
    pub depends_on: Option<Vec<usize>>,
}
#[derive(Debug, Serialize)]
pub struct Requirement {
    pub source: Text,
    pub range: Range,
}
#[derive(Debug, Serialize)]
pub struct ArchitectureArgument {
    pub name: Option<Text>,
    pub value: Requirement,
}
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArchitectureInterpretation {
    /// A parsed declaration, including kernel defaults. No body inference or
    /// architecture proof has been run by this projection.
    Declared { canonical: verum_kernel::arch::Shape },
    Invalid { message: Text },
}
#[derive(Debug, Serialize)]
pub struct ArchitectureAnnotation {
    pub attribute: Requirement,
    /// Preserve the actual supplied arguments separately from normalized
    /// values: defaulted fields are not claims made explicitly by the author.
    pub supplied: List<ArchitectureArgument>,
    #[serde(flatten)]
    pub interpretation: ArchitectureInterpretation,
}
#[derive(Debug, Serialize)]
pub struct MountSnapshot {
    #[serde(flatten)]
    pub declaration: Requirement,
    pub resolution: Option<crate::project_context::MountResolution>,
}
#[derive(Debug, Serialize)]
pub struct DocumentSnapshot {
    pub source_mode: verum_common::source_mode::SourceMode,
    pub schema: &'static str,
    pub uri: Url,
    pub version: i32,
    pub source_hash: Text,
    pub parsed: bool,
    pub project_revision: Option<Text>,
    pub library_metadata: Option<crate::library_metadata::LibraryIdentity>,
    pub bindings: List<crate::project_context::ProjectBinding>,
    pub analysis_scope: &'static str,
    pub diagnostics: List<Diagnostic>,
    pub declarations: List<Declaration>,
    pub mounts: List<MountSnapshot>,
    /// None means parsing is unavailable; an empty list means no root attribute.
    pub architecture: Option<List<ArchitectureAnnotation>>,
}

fn requirement(doc: &DocumentState, span: Span) -> Requirement {
    Requirement {
        source: doc
            .text
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
            .into(),
        range: ast_span_to_range(&span, &doc.text),
    }
}

fn architecture_annotations(doc: &DocumentState, attributes: &[verum_ast::Attribute]) -> List<ArchitectureAnnotation> {
    attributes.iter().filter(|attribute| attribute.is_named("arch_module")).map(|attribute| {
        let args = attribute.args.as_ref().map(|args| args.as_slice()).unwrap_or(&[]);
        let supplied = args.iter().map(|argument| {
            // Project syntax from AST nodes, not from spelling searches in text.
            let (name, value) = match &argument.kind {
                ExprKind::NamedArg { name, value } => (Some(name.name.clone()), value.as_ref()),
                ExprKind::Binary { op: BinOp::Assign, left, right } => {
                    let name = match &left.kind {
                        ExprKind::Path(path) => match path.segments.as_slice() {
                            [PathSegment::Name(name)] => Some(name.name.clone()),
                            _ => None,
                        },
                        _ => None,
                    };
                    (name, right.as_ref())
                }
                _ => (None, argument),
            };
            ArchitectureArgument { name, value: requirement(doc, value.span) }
        }).collect();
        let interpretation = match verum_kernel::arch_parse::parse_arch_module(args) {
            Ok(canonical) => ArchitectureInterpretation::Declared { canonical },
            Err(error) => ArchitectureInterpretation::Invalid { message: error.human_message().into() },
        };
        ArchitectureAnnotation { attribute: requirement(doc, attribute.span), supplied, interpretation }
    }).collect()
}

fn declaration_heading(item: &verum_ast::Item) -> Option<(&verum_ast::Ident, &'static str)> {
    Some(match &item.kind {
        ItemKind::Function(decl) => (&decl.name, "function"),
        ItemKind::Type(decl) => (
            &decl.name,
            match &decl.body {
                TypeDeclBody::Protocol(body) if body.is_context => "context",
                TypeDeclBody::Protocol(_) => "protocol",
                TypeDeclBody::Record(_) => "record",
                TypeDeclBody::Variant(_) => "variant",
                _ => "type",
            },
        ),
        ItemKind::Context(decl) => (&decl.name, "context"),
        ItemKind::Protocol(decl) => (&decl.name, "protocol"),
        ItemKind::Const(decl) => (&decl.name, "constant"),
        ItemKind::Theorem(decl) => (&decl.name, "theorem"),
        ItemKind::Lemma(decl) => (&decl.name, "lemma"),
        ItemKind::Corollary(decl) => (&decl.name, "corollary"),
        ItemKind::Axiom(decl) => (&decl.name, "axiom"),
        ItemKind::Module(decl) => (&decl.name, "module"),
        _ => return None,
    })
}

fn declaration_id(uri: &Url, item: &verum_ast::Item, kind: &str) -> String {
    format!("{}#{}:{}:{}", uri, item.span.start, item.span.end, kind)
}

fn analysis_scope(doc: &DocumentState) -> &'static str {
    match (&doc.analysis_revision, &doc.library_metadata) {
        (Some(_), Some(_)) => "project-sources-with-library-metadata",
        (Some(_), None) => "project-sources-with-builtins",
        (None, _) => "single-module-with-builtins",
    }
}

fn project_declaration(
    uri: &Url,
    doc: &DocumentState,
    item: &verum_ast::Item,
    (name, kind): (&verum_ast::Ident, &'static str),
    semantic_fields_available: bool,
) -> Declaration {
    let requirement = |span| requirement(doc, span);
    let requirements = match &item.kind {
        ItemKind::Function(decl) => decl
            .contexts
            .iter()
            .map(|context| requirement(context.span))
            .collect(),
        _ => List::new(),
    };
    let (preconditions, postconditions) = match &item.kind {
        ItemKind::Function(decl) => (
            decl.requires
                .iter()
                .map(|expr| requirement(expr.span))
                .collect(),
            decl.ensures
                .iter()
                .map(|expr| requirement(expr.span))
                .collect(),
        ),
        ItemKind::Theorem(decl) | ItemKind::Lemma(decl) | ItemKind::Corollary(decl) => (
            decl.requires
                .iter()
                .map(|expr| requirement(expr.span))
                .collect(),
            decl.ensures
                .iter()
                .map(|expr| requirement(expr.span))
                .collect(),
        ),
        _ => (List::new(), List::new()),
    };
    let record_fields = match &item.kind {
        ItemKind::Type(decl) => match &decl.body {
            TypeDeclBody::Record(fields) => Some(
                fields
                    .iter()
                    .map(|field| {
                        let depends_on = doc
                            .get_symbol(&format!("{}.{}", name, field.name))
                            .filter(|symbol| {
                                symbol.def_span == field.span && semantic_fields_available
                            })
                            .and_then(|symbol| symbol.ty.as_ref())
                            .map(|ty| {
                                let mut slots: Vec<_> = ty
                                    .record_field_slots()
                                    .into_iter()
                                    .map(|slot| slot.ordinal)
                                    .collect();
                                slots.sort_unstable();
                                slots.dedup();
                                slots
                            });
                        RecordMember {
                            name: field.name.name.clone(),
                            selection_range: ast_span_to_range(&field.name.span, &doc.text),
                            annotation: requirement(field.ty.span),
                            depends_on,
                        }
                    })
                    .collect(),
            ),
            _ => None,
        },
        _ => None,
    };
    Declaration {
        id: declaration_id(uri, item, kind).into(),
        name: name.as_str().into(),
        kind,
        type_display: doc
            .get_symbol(name.as_str())
            .and_then(|symbol| symbol.ty.as_ref())
            .map(|ty| ty.to_string().into()),
        range: ast_span_to_range(&item.span, &doc.text),
        selection_range: ast_span_to_range(&name.span, &doc.text),
        requirements,
        preconditions,
        postconditions,
        architecture: architecture_annotations(doc, &item.attributes),
        record_fields,
    }
}

impl DocumentSnapshot {
    pub fn from_document(uri: &Url, doc: &DocumentState) -> Self {
        let mut declarations = List::new();
        let mut mounts = List::new();
        let semantic_fields_available = !doc.diagnostics.iter().any(|d| d.is_error());
        if let Some(module) = &doc.module {
            for item in &module.items {
                if let Some(heading) = declaration_heading(item) {
                    declarations.push(project_declaration(
                        uri,
                        doc,
                        item,
                        heading,
                        semantic_fields_available,
                    ));
                } else if matches!(item.kind, ItemKind::Mount(_)) {
                    mounts.push(MountSnapshot {
                        declaration: requirement(doc, item.span),
                        resolution: doc.mount_resolutions.get(&item.span.start).cloned(),
                    });
                }
            }
        }
        Self {
            schema: "verum-document-analysis/1",
            uri: uri.clone(),
            version: doc.version,
            source_hash: hex::encode(Sha256::digest(doc.text.as_bytes())).into(),
            source_mode: doc.source_mode,
            parsed: doc.module.is_some(),
            project_revision: doc.analysis_revision.clone(),
            library_metadata: doc.library_metadata.clone(),
            bindings: doc.project_bindings.clone(),
            analysis_scope: analysis_scope(doc),
            diagnostics: convert_diagnostics(doc.diagnostics.clone(), &doc.text, uri).into(),
            declarations,
            mounts,
            architecture: doc.module.as_ref().map(|module| architecture_annotations(doc, &module.attributes)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceParams {
    #[serde(flatten)]
    pub snapshot: SnapshotParams,
    pub expected_source_hash: String,
    pub expected_project_revision: Option<String>,
    pub declaration_id: Option<String>,
}

/// Project only the requested declaration; the full snapshot has a separate transport budget.
fn document_evidence(
    params: &EvidenceParams,
    doc: &DocumentState,
) -> jsonrpc::Result<serde_json::Value> {
    let uri = &params.snapshot.text_document.uri;
    let hash = hex::encode(Sha256::digest(doc.text.as_bytes()));
    if doc.version != params.snapshot.version
        || !params.expected_source_hash.eq_ignore_ascii_case(&hash)
        || params
            .expected_project_revision
            .as_deref()
            .is_some_and(|pin| Some(pin) != doc.analysis_revision.as_deref())
    {
        return Err(jsonrpc::Error::content_modified());
    }
    let errors = doc.diagnostics.iter().filter(|d| d.is_error()).count();
    let selected = params.declaration_id.as_ref().map(|id| -> jsonrpc::Result<_> {
        // The suffix encodes the UTF-8 extent and kind. Check the complete opaque ID,
        // including its URI, on the candidate before returning any declaration.
        let start = id.strip_prefix(uri.as_str()).and_then(|s| s.strip_prefix('#'))
            .and_then(|suffix| suffix.split_once(':')).and_then(|(start, _)| start.parse::<u32>().ok());
        let (item, heading) = doc.module.iter().flat_map(|m| &m.items)
            .filter(|item| Some(item.span.start) == start)
            .filter_map(|item| Some((item, declaration_heading(item)?)))
            .find(|(item, (_, kind))| declaration_id(uri, item, kind) == *id)
            .ok_or_else(|| jsonrpc::Error::invalid_params("Unknown declaration in this revision"))?;
        let span = item.span;
        let bytes = doc.text.get(span.start as usize..span.end as usize).ok_or_else(jsonrpc::Error::internal_error)?;
        Ok((project_declaration(uri, doc, item, heading, errors == 0),
            serde_json::json!({"start":span.start,"end":span.end,"sha256":hex::encode(Sha256::digest(bytes.as_bytes()))})))
    }).transpose()?;
    let (declaration, extent) = selected.unzip();
    Ok(serde_json::json!({
        "schema":"verum-document-evidence/1", "uri":uri, "source_hash":hash, "source_mode":doc.source_mode, "version":doc.version,
        "project_revision":doc.analysis_revision, "library_metadata":doc.library_metadata, "analysis_scope":analysis_scope(doc),
        "parsed":doc.module.is_some(), "document_diagnostics":{"total":doc.diagnostics.len(),"errors":errors},
        "declaration":declaration, "declaration_span":extent,
        "architecture":doc.module.as_ref().map(|module| architecture_annotations(doc, &module.attributes))
    }))
}

impl Backend {
    pub async fn handle_document_evidence(
        &self,
        params: EvidenceParams,
    ) -> jsonrpc::Result<serde_json::Value> {
        let _guard = self.project_gate.lock().await;
        self.refresh_project();
        self.documents
            .with_document(&params.snapshot.text_document.uri, |doc| {
                document_evidence(&params, doc)
            })
            .unwrap_or_else(|| Err(jsonrpc::Error::invalid_params("Document is not open")))
    }
    pub async fn handle_project_snapshot(
        &self,
        _: serde_json::Value,
    ) -> jsonrpc::Result<serde_json::Value> {
        let _guard = self.project_gate.lock().await;
        self.refresh_project();
        let mut project = self.project.lock();
        let mut documents = self.documents.flat_collect_from_documents(|uri, doc| {
            vec![DocumentSnapshot::from_document(uri, doc)]
        });
        documents.sort_by(|left, right| left.uri.cmp(&right.uri));
        let topology = serde_json::to_value(project.import_topology()
            .map_err(|error| jsonrpc::Error::invalid_params(error.to_string()))?)
            .map_err(|_| jsonrpc::Error::internal_error())?;
        let mut snapshot = serde_json::json!({
            "schema": "verum-project-analysis/1",
            "coverage": "open-documents-and-source-dependencies",
            "project_revision": project.revision(),
            "library_metadata": project.library_identity(),
            "sources": project.identities,
            "issues": project.issues,
            "documents": documents,
        });
        snapshot["import_topology"] = topology;
        Ok(snapshot)
    }
    pub async fn handle_document_snapshot(
        &self,
        params: SnapshotParams,
    ) -> jsonrpc::Result<serde_json::Value> {
        let _guard = self.project_gate.lock().await;
        self.refresh_project();
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
