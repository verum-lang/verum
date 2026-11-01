//! Exact-source, opt-in logical elaboration using the Verum production parser.
//! Profiles check logical proof terms or contracts of a total finite source
//! fragment. Neither profile certifies compiler lowering or native execution.
//! Untrusted input must run through the bounded host process, not in a server.
//!
//! The wire protocol and binder IR deliberately use the portable native checker's
//! Rust ownership types. They are not language-runtime values or a second checker.
pub mod finite;
mod profile;
pub mod scoped;

pub use profile::{SourceOutcome, SourceProfile};
pub use scoped::PROFILE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use verum_ast::decl::ItemKind;
use verum_proof::{
    admission::{self, Diagnostic, Judgment, ReplayOutcome, Report},
    limits::{AdmissionLimits, ProcessLimits, ReplayLimits},
    proof_checker::Term,
};

/// A source producer returns the judgment, its displayed conclusion and source
/// implementation, whereas a native replay worker returns only a verdict.
/// Bound this envelope separately; input, proof and execution budgets stay at
/// their native defaults. The child and supervising caller use this policy.
pub const WORKER_LIMITS: ReplayLimits = ReplayLimits {
    admission: AdmissionLimits::DEFAULT,
    process: ProcessLimits {
        output_bytes: 4 * AdmissionLimits::DEFAULT.max_bytes as u64,
        ..ProcessLimits::DEFAULT
    },
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
    /// Half-open UTF-8 source range; an empty range denotes an insertion point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_span: Option<SourceSpan>,
    /// Present only for a successfully checked source-function contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<finite::ImplementationBinding>,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn elaborator_identity() -> &'static str {
    env!("VERUM_SOURCE_ELABORATOR")
}
impl SourceReport {
    fn at_source(mut self, source: &str, span: verum_common::span::Span) -> Self {
        if let Some(text) = source.get(span.start as usize..span.end as usize) {
            self.diagnostic_span = Some(SourceSpan {
                start: span.start,
                end: span.end,
                sha256: sha256(text.as_bytes()),
            });
        } else {
            self.status = "inconclusive".into();
            self.diagnostic = Some(Diagnostic {
                stage: "protocol".into(),
                code: "invalid_diagnostic_span".into(),
                message: "The diagnostic range does not address exact UTF-8 source bytes.".into(),
            });
        }
        self
    }
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
            diagnostic_span: None,
            implementation: None,
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
        report.profile = request.profile.clone();
        report.source = source.clone();
        report
    };
    if request.profile != PROFILE && request.profile != finite::PROFILE {
        return failure(
            "unsupported",
            "profile",
            "unsupported_profile",
            "Select an explicit supported source interpretation: verum-source-pi-id/1 or verum-source-finite/1.",
            &source,
        );
    }
    let module = match verum_fast_parser::FastParser::new()
        .parse_module_str(&request.source, verum_common::span::FileId::new(0))
    {
        Ok(module) => module,
        Err(errors) => {
            let Some(error) = errors.first() else {
                return failure(
                    "inconclusive",
                    "parse",
                    "empty_parser_failure",
                    "The parser failed without a diagnostic.",
                    &source,
                );
            };
            return failure(
                "rejected",
                "parse",
                "source_parse_error",
                &format!(
                    "First of {} parser diagnostics: {}",
                    errors.len(),
                    error.kind
                ),
                &source,
            )
            .at_source(&request.source, error.span);
        }
    };
    if request.profile == finite::PROFILE {
        return replay_finite(&request, &module, source);
    }
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
    let artifact = match scoped::elaborate_diagnostic(theorem, PROFILE) {
        Ok(artifact) => artifact,
        Err(error) => {
            let location = error.span.unwrap_or(item.span);
            let (status, stage, code, message) = match error.error {
                scoped::Error::Unsupported(m) => (
                    "unsupported",
                    "elaboration",
                    "unsupported_source_construct",
                    m,
                ),
                scoped::Error::Scope(m) => ("rejected", "scope", "unbound_or_duplicate_binder", m),
                scoped::Error::Rejected(m) => ("rejected", "elaboration", "ill_typed_source", m),
                scoped::Error::Inconclusive { code, message } => {
                    ("inconclusive", "elaboration", code, message)
                }
            };
            return failure(status, stage, code, &message, &source)
                .at_source(&request.source, location);
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
        return native_refusal(native, &source);
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
        diagnostic_span: None,
        implementation: None,
    }
}

fn replay_finite(
    request: &SourceRequest,
    module: &verum_ast::Module,
    source: SourceBinding,
) -> SourceReport {
    let artifact = match finite::elaborate(&request.source, module, &request.declaration) {
        Ok(artifact) => artifact,
        Err(error) => {
            let (status, code, message) = match error {
                scoped::Error::Unsupported(message) => {
                    ("unsupported", "unsupported_source_construct", message)
                }
                scoped::Error::Scope(message) => {
                    ("rejected", "unbound_or_duplicate_binder", message)
                }
                scoped::Error::Rejected(message) => ("rejected", "ill_typed_source", message),
                scoped::Error::Inconclusive { code, message } => ("inconclusive", code, message),
            };
            let mut report = SourceReport::failure(status, "elaboration", code, &message);
            report.profile = finite::PROFILE.into();
            report.source = source;
            return report;
        }
    };
    let judgment = Judgment {
        profile: admission::PROFILE.into(),
        statement: artifact.certificate.claimed_type.clone(),
        certificate: artifact.certificate,
    };
    let native = admission::replay(&serde_json::to_vec(&judgment).expect("serializable judgment"));
    if !native.accepted {
        // A failed conjunction can be localized without rechecking every
        // component on the successful path. Resource refusals stay distinct.
        if native.outcome == ReplayOutcome::Rejected && native.is_consistent() {
            for candidate in &artifact.candidates {
                let component = Judgment {
                    profile: admission::PROFILE.into(),
                    statement: candidate.certificate.claimed_type.clone(),
                    certificate: candidate.certificate.clone(),
                };
                let checked = admission::replay(
                    &serde_json::to_vec(&component).expect("serializable component"),
                );
                if checked.outcome == ReplayOutcome::Rejected && checked.is_consistent() {
                    let mut report = native_refusal(checked, &artifact.source);
                    report.profile = finite::PROFILE.into();
                    report.diagnostic_span = candidate
                        .binding
                        .call
                        .clone()
                        .or_else(|| candidate.binding.owner.span.clone());
                    if let Some(diagnostic) = &mut report.diagnostic {
                        diagnostic.message = format!(
                            "{:?} obligation in {}: {}",
                            candidate.binding.kind,
                            candidate.binding.owner.declaration,
                            diagnostic.message
                        );
                    }
                    return report;
                }
                if !checked.accepted {
                    break;
                }
            }
        }
        let mut report = native_refusal(native, &artifact.source);
        report.profile = finite::PROFILE.into();
        return report;
    }
    SourceReport {
        profile: finite::PROFILE.into(),
        status: "verified".into(),
        source: artifact.source,
        elaborator: elaborator_identity().into(),
        native: Some(native),
        conclusion: Some(judgment.statement.clone()),
        judgment: Some(judgment),
        telescope: vec![],
        universe_bindings: vec![],
        diagnostic: None,
        diagnostic_span: None,
        implementation: Some(artifact.implementation),
    }
}

// Preserve the native semantic category rather than recognizing a diagnostic
// string whitelist. New resource kinds must not become invalid source proofs.
fn native_refusal(native: Report, source: &SourceBinding) -> SourceReport {
    let consistent = native.is_consistent() && native.outcome != ReplayOutcome::Accepted;
    let status = if !consistent {
        "inconclusive"
    } else {
        match native.outcome {
            ReplayOutcome::Rejected => "rejected",
            ReplayOutcome::Unsupported => "unsupported",
            ReplayOutcome::ResourceExhausted | ReplayOutcome::Inconclusive => "inconclusive",
            ReplayOutcome::Accepted => "inconclusive",
        }
    };
    let diagnostic = if consistent {
        native
            .diagnostic
            .clone()
            .expect("non-accepted consistent report has diagnostic")
    } else {
        Diagnostic {
            stage: "protocol".into(),
            code: "invalid_native_report".into(),
            message: "The native checker returned an inconsistent refusal.".into(),
        }
    };
    let mut report = SourceReport::failure(
        status,
        &diagnostic.stage,
        &diagnostic.code,
        &diagnostic.message,
    );
    report.source = source.clone();
    report.native = Some(native);
    report
}

#[cfg(test)]
#[path = "../tests/unit/native_outcomes.rs"]
mod native_outcomes;
