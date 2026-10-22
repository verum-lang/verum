//! Exact-source, opt-in logical elaboration using the Verum production parser.
//! This profile checks closed proof terms; it does not certify runtime functions.
//! Untrusted input must run through the bounded host process, not in a server.
//!
//! The wire protocol and binder IR deliberately use the portable native checker's
//! Rust ownership types. They are not language-runtime values or a second checker.
pub mod scoped;

pub use scoped::PROFILE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use verum_ast::decl::ItemKind;
use verum_proof::{
    admission::{self, Diagnostic, Judgment, Report},
    proof_checker::Term,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRequest {
    pub profile: String,
    pub source: String,
    pub declaration: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub start: u32,
    pub end: u32,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub sha256: String,
    pub declaration: String,
    pub span: Option<SourceSpan>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelescopeEntry {
    pub id: u64,
    pub name: Option<String>,
    pub role: String,
    pub domain: Term,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseBinding {
    pub source: String,
    pub kernel: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReport {
    pub profile: String,
    pub status: String,
    pub source: SourceBinding,
    pub elaborator: String,
    pub native: Option<Report>,
    pub judgment: Option<Judgment>,
    pub telescope: Vec<TelescopeEntry>,
    pub conclusion: Option<Term>,
    pub universe_bindings: Vec<UniverseBinding>,
    pub diagnostic: Option<Diagnostic>,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn elaborator_identity() -> &'static str {
    env!("VERUM_SOURCE_ELABORATOR")
}
impl SourceReport {
    pub fn failure(status: &str, stage: &str, code: &str, message: &str) -> Self {
        Self {
            profile: PROFILE.into(),
            status: status.into(),
            source: SourceBinding {
                sha256: String::new(),
                declaration: String::new(),
                span: None,
            },
            elaborator: elaborator_identity().into(),
            native: None,
            judgment: None,
            telescope: Vec::new(),
            conclusion: None,
            universe_bindings: Vec::new(),
            diagnostic: Some(Diagnostic {
                code: code.into(),
                stage: stage.into(),
                message: message.into(),
            }),
        }
    }
}

/// Strict JSON admission and exact statement extraction. Resource isolation is
/// required before calling this function for input not trusted by the caller.
pub fn replay(bytes: &[u8]) -> SourceReport {
    if bytes.len() > admission::MAX_BYTES {
        return SourceReport::failure(
            "inconclusive",
            "decode",
            "byte_limit",
            "Source request exceeds its byte budget.",
        );
    }
    let request: SourceRequest = match serde_json::from_slice(bytes) {
        Ok(request) => request,
        Err(_) => {
            return SourceReport::failure(
                "rejected",
                "decode",
                "invalid_source_request",
                "Expected one strict source request JSON object, without duplicate or unknown fields.",
            );
        }
    };
    let source = SourceBinding {
        sha256: sha256(request.source.as_bytes()),
        declaration: request.declaration.clone(),
        span: None,
    };
    let failure = |status: &str, stage: &str, code: &str, message: &str, source: &SourceBinding| {
        let mut report = SourceReport::failure(status, stage, code, message);
        report.source = source.clone();
        report
    };
    if request.profile != PROFILE {
        return failure(
            "unsupported",
            "profile",
            "unsupported_profile",
            "Select the explicit verum-source-pi-id/1 proof-only interpretation.",
            &source,
        );
    }
    let module = match verum_fast_parser::FastParser::new()
        .parse_module_str(&request.source, verum_common::span::FileId::new(0))
    {
        Ok(module) => module,
        Err(_) => {
            return failure(
                "rejected",
                "parse",
                "source_parse_error",
                "The production Verum parser rejected the source module.",
                &source,
            );
        }
    };
    let mut selected = module.items.iter().filter_map(|item| match &item.kind {
        ItemKind::Theorem(theorem) | ItemKind::Lemma(theorem) | ItemKind::Corollary(theorem)
            if theorem.name.as_str() == request.declaration =>
        {
            Some((item, theorem))
        }
        _ => None,
    });
    let Some((item, theorem)) = selected.next() else {
        return failure(
            "unsupported",
            "declaration",
            "declaration_not_found",
            "Select an exact top-level theorem name; runtime declarations are not proof declarations.",
            &source,
        );
    };
    if selected.next().is_some() {
        return failure(
            "rejected",
            "declaration",
            "ambiguous_declaration",
            "The source contains more than one selected theorem declaration.",
            &source,
        );
    }
    let span = item.span;
    let Some(exact) = request
        .source
        .get(span.start as usize..span.end as usize)
        .filter(|s| !s.is_empty())
    else {
        return failure(
            "rejected",
            "parse",
            "invalid_source_span",
            "The parser did not provide a valid nonempty UTF-8 declaration span.",
            &source,
        );
    };
    let source = SourceBinding {
        span: Some(SourceSpan {
            start: span.start,
            end: span.end,
            sha256: sha256(exact.as_bytes()),
        }),
        ..source
    };
    if !item.attributes.is_empty() {
        return failure(
            "unsupported",
            "elaboration",
            "unsupported_attributes",
            "Declaration attributes require explicit semantics and cannot be erased.",
            &source,
        );
    }
    let artifact = match scoped::elaborate(theorem, PROFILE) {
        Ok(artifact) => artifact,
        Err(error) => {
            let (status, stage, code, message) = match error {
                scoped::Error::Unsupported(m) => (
                    "unsupported",
                    "elaboration",
                    "unsupported_source_construct",
                    m,
                ),
                scoped::Error::Scope(m) => ("rejected", "scope", "unbound_or_duplicate_binder", m),
                scoped::Error::Rejected(m) => ("rejected", "elaboration", "ill_typed_source", m),
            };
            return failure(status, stage, code, &message, &source);
        }
    };
    let statement = match scoped::lower(&artifact.scoped_type, &mut Vec::new()) {
        Ok(term) => term,
        Err(_) => {
            return failure(
                "rejected",
                "scope",
                "escaped_statement",
                "The source statement is not closed.",
                &source,
            );
        }
    };
    let judgment = Judgment {
        profile: admission::PROFILE.into(),
        statement,
        certificate: artifact.certificate,
    };
    let encoded =
        serde_json::to_vec(&judgment).expect("native judgment serialization is infallible");
    let native = admission::replay(&encoded);
    if !native.accepted {
        let diagnostic = native
            .diagnostic
            .clone()
            .expect("rejected native report has diagnostic");
        let status = if matches!(diagnostic.code.as_str(), "structural_limit" | "byte_limit") {
            "inconclusive"
        } else {
            "rejected"
        };
        let mut report = failure(
            status,
            &diagnostic.stage,
            &diagnostic.code,
            &diagnostic.message,
            &source,
        );
        report.native = Some(native);
        return report;
    }
    let mut frame = Vec::new();
    let mut telescope = Vec::new();
    for binding in artifact.telescope {
        let domain = match scoped::lower(&binding.domain, &mut frame) {
            Ok(term) => term,
            Err(_) => {
                return failure(
                    "rejected",
                    "scope",
                    "escaped_telescope",
                    "An assumption refers outside its preceding telescope.",
                    &source,
                );
            }
        };
        frame.push(binding.id);
        let role = match binding.role {
            scoped::Role::Type => "type",
            scoped::Role::Value => "value",
            scoped::Role::Premise => "premise",
        };
        telescope.push(TelescopeEntry {
            id: binding.id.0,
            name: binding.name,
            role: role.into(),
            domain,
        });
    }
    SourceReport {
        profile: PROFILE.into(),
        status: "verified".into(),
        source,
        elaborator: elaborator_identity().into(),
        native: Some(native),
        judgment: Some(judgment),
        telescope,
        conclusion: Some(artifact.conclusion),
        universe_bindings: artifact
            .level_bindings
            .into_iter()
            .map(|(source, kernel)| UniverseBinding { source, kernel })
            .collect(),
        diagnostic: None,
    }
}
