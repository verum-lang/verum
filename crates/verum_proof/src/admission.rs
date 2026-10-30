//! A closed native judgment, with an independently supplied exact statement.
//! Source-language elaboration, prose interpretation and SMT traces are not admitted here.
//! Execute untrusted replay in a resource-limited worker; structural limits do not bound
//! normalization cost or replace a wall-clock/process memory budget.
pub use crate::limits::AdmissionLimits;
use crate::limits::WorkerReceipt;
use crate::proof_checker::{Certificate, CheckError, DataField, Foundation, Level, Term};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "verum-native/1";
pub const EXTENSIONAL_PROFILE: &str = "verum-native-funext/1";

pub fn profile_for(foundation: Foundation) -> &'static str {
    match foundation {
        Foundation::Intensional => PROFILE,
        Foundation::FunctionExtensionality => EXTENSIONAL_PROFILE,
    }
}
pub const MAX_BYTES: usize = AdmissionLimits::DEFAULT.max_bytes;
// Structural bounds protect decoding and traversal, not normalization. General
// dependent program proofs contain thousands of nodes even with shared local
// definitions. The isolated worker independently retains its CPU, memory and
// wall-clock ceilings; reaching those ceilings never counts as a proof verdict.
pub const MAX_NODES: usize = AdmissionLimits::DEFAULT.max_nodes;
pub const MAX_DEPTH: usize = AdmissionLimits::DEFAULT.max_depth;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Judgment {
    pub profile: String,
    pub statement: Term,
    pub certificate: Certificate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub stage: String,
    pub message: String,
}
/// Operational exhaustion and unsupported evidence are not invalid proofs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOutcome {
    Accepted,
    Rejected,
    Unsupported,
    ResourceExhausted,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub profile: String,
    pub checker: String,
    /// Compatibility flag; true exactly when outcome is Accepted.
    pub accepted: bool,
    pub outcome: ReplayOutcome,
    pub limits: AdmissionLimits,
    pub worker: Option<WorkerReceipt>,
    pub diagnostic: Option<Diagnostic>,
}

impl Report {
    pub(crate) fn refusal(
        foundation: Foundation,
        limits: AdmissionLimits,
        outcome: ReplayOutcome,
        stage: &str,
        code: &str,
        message: &str,
    ) -> Self {
        Self {
            profile: profile_for(foundation).into(),
            checker: checker_identity(),
            accepted: false,
            outcome,
            limits,
            worker: None,
            diagnostic: Some(Diagnostic {
                code: code.into(),
                stage: stage.into(),
                message: message.into(),
            }),
        }
    }

    pub fn is_consistent(&self) -> bool {
        self.accepted == (self.outcome == ReplayOutcome::Accepted)
            && self.accepted == self.diagnostic.is_none()
    }
}

#[derive(Clone, Copy)]
enum AdmissionFailure {
    Exhausted(&'static str),
    Invalid(&'static str),
}

impl AdmissionFailure {
    fn outcome(self) -> ReplayOutcome {
        match self {
            Self::Exhausted(_) => ReplayOutcome::ResourceExhausted,
            Self::Invalid(_) => ReplayOutcome::Rejected,
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Exhausted(code) | Self::Invalid(code) => code,
        }
    }
}

/// Identity of this admission policy and the exact native checker implementations.
/// This is reproducibility metadata, not a signature or a soundness proof.
pub fn checker_identity() -> String {
    let mut h = blake3::Hasher::new();
    for part in [
        PROFILE,
        env!("CARGO_PKG_VERSION"),
        include_str!("admission.rs"),
        include_str!("limits.rs"),
        include_str!("process.rs"),
        include_str!("main.rs"),
        include_str!("proof_checker.rs"),
        include_str!("proof_checker_nbe.rs"),
    ] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}
fn code(error: &CheckError) -> &'static str {
    match error {
        CheckError::FoundationRequired(_) => "foundation_required",
        CheckError::InvalidInductive(_) => "invalid_inductive",
        CheckError::UnboundVariable(_) => "unbound_variable",
        CheckError::NotAType(_) | CheckError::ClaimedTypeNotAType { .. } => "not_a_type",
        CheckError::NotAFunction(_) => "not_a_function",
        CheckError::NotASigma(_) => "not_a_pair_type",
        CheckError::NotAnIdentity(_) => "not_an_identity",
        CheckError::ReflEndpointMismatch { .. } => "identity_endpoints_differ",
        CheckError::NotAValidJMotive(_) => "invalid_identity_motive",
        CheckError::DomainMismatch { .. } => "domain_mismatch",
        CheckError::TypeMismatch { .. } => "type_mismatch",
        CheckError::UniverseOverflow { .. } => "universe_limit",
        CheckError::KernelV0ManifestUnclean { .. }
        | CheckError::KernelV0MetaSoundnessExceeded
        | CheckError::KernelV0StrictIntrinsicDisagreement { .. } => "unsupported_kernel_extension",
    }
}
fn admit_structure(judgment: &Judgment, limits: AdmissionLimits) -> Result<(), AdmissionFailure> {
    enum Node<'a> {
        Term(&'a Term, usize),
        Level(&'a Level),
    }
    let mut queue = vec![
        (Node::Term(&judgment.statement, 0), 0),
        (Node::Term(&judgment.certificate.term, 0), 0),
        (Node::Term(&judgment.certificate.claimed_type, 0), 0),
    ];
    let mut count = 0usize;
    while let Some((node, depth)) = queue.pop() {
        count = count
            .checked_add(1)
            .ok_or(AdmissionFailure::Exhausted("structural_limit"))?;
        if count > limits.max_nodes || depth > limits.max_depth {
            return Err(AdmissionFailure::Exhausted("structural_limit"));
        }
        match node {
            Node::Term(t, binders) => match t {
                Term::FunExt {
                    function_type,
                    lhs,
                    rhs,
                    pointwise,
                } => {
                    for term in [function_type, lhs, rhs, pointwise] {
                        queue.push((Node::Term(term, binders), depth + 1));
                    }
                }
                Term::Data {
                    level,
                    indices,
                    constructors,
                    ..
                } => {
                    queue.push((Node::Level(level), depth + 1));
                    for (index, ty) in indices.iter().enumerate() {
                        queue.push((Node::Term(ty, binders + index), depth + 1));
                    }
                    count = count
                        .checked_add(constructors.len())
                        .ok_or(AdmissionFailure::Exhausted("structural_limit"))?;
                    for fields in constructors {
                        for term in &fields.indices {
                            queue.push((Node::Term(term, binders + fields.len()), depth + 1));
                        }
                        for (index, field) in fields.iter().enumerate() {
                            count = count
                                .checked_add(1)
                                .ok_or(AdmissionFailure::Exhausted("structural_limit"))?;
                            match field {
                                DataField::Value(ty) => {
                                    queue.push((Node::Term(ty, binders), depth + 1))
                                }
                                DataField::Dependent(ty) => {
                                    queue.push((Node::Term(ty, binders + index), depth + 1))
                                }
                                DataField::Recursive => {}
                                DataField::RecursiveAt(indices) => {
                                    for term in indices {
                                        queue.push((Node::Term(term, binders + index), depth + 1));
                                    }
                                }
                            }
                        }
                    }
                    if count > limits.max_nodes {
                        return Err(AdmissionFailure::Exhausted("structural_limit"));
                    }
                }
                Term::Construct { data, args, .. } => {
                    queue.push((Node::Term(data, binders), depth + 1));
                    for term in args {
                        queue.push((Node::Term(term, binders), depth + 1));
                    }
                }
                Term::Induct {
                    scrutinee,
                    motive,
                    cases,
                } => {
                    queue.push((Node::Term(scrutinee, binders), depth + 1));
                    queue.push((Node::Term(motive, binders), depth + 1));
                    for term in cases {
                        queue.push((Node::Term(term, binders), depth + 1));
                    }
                }
                Term::Universe(l) => queue.push((Node::Level(l), depth + 1)),
                Term::Var(index) => {
                    if *index >= binders {
                        return Err(AdmissionFailure::Invalid("unbound_variable"));
                    }
                }
                Term::Let { value: a, body: b }
                | Term::Pi(a, b)
                | Term::Lam(a, b)
                | Term::Sigma(a, b) => {
                    queue.push((Node::Term(a, binders), depth + 1));
                    queue.push((Node::Term(b, binders + 1), depth + 1));
                }
                Term::App(a, b) | Term::Pair(a, b) => {
                    queue.push((Node::Term(a, binders), depth + 1));
                    queue.push((Node::Term(b, binders), depth + 1));
                }
                Term::Fst(a) | Term::Snd(a) | Term::Refl(a) => {
                    queue.push((Node::Term(a, binders), depth + 1))
                }
                Term::Id {
                    ty: a,
                    lhs: b,
                    rhs: c,
                }
                | Term::J {
                    motive: a,
                    base: b,
                    scrutinee: c,
                }
                | Term::IdElim {
                    motive: a,
                    base: b,
                    scrutinee: c,
                } => {
                    for t in [a, b, c] {
                        queue.push((Node::Term(t, binders), depth + 1));
                    }
                }
            },
            Node::Level(l) => match l {
                Level::Concrete(_) | Level::Var(_) => {}
                Level::Succ(a) | Level::Offset(a, _) => queue.push((Node::Level(a), depth + 1)),
                Level::Max(a, b) => {
                    queue.push((Node::Level(a), depth + 1));
                    queue.push((Node::Level(b), depth + 1));
                }
            },
        }
    }
    Ok(())
}

// Indexed constructor declarations can add eight JSON containers between
// consecutive logical nodes, including field and index arrays. The
// judgment envelope adds fewer than eight. Bound those containers before
// recursive deserialization; the unchanged logical limits are checked next.
fn json_envelope(bytes: &[u8], max_nesting: usize) -> Result<(), AdmissionFailure> {
    let mut containers = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                if containers.len() == max_nesting {
                    return Err(AdmissionFailure::Exhausted("json_depth_limit"));
                }
                containers.push(*byte);
            }
            b'}' | b']' => {
                let expected = if *byte == b'}' { b'{' } else { b'[' };
                if containers.pop() != Some(expected) {
                    return Err(AdmissionFailure::Invalid("invalid_certificate"));
                }
            }
            _ => {}
        }
    }
    if quoted || !containers.is_empty() {
        return Err(AdmissionFailure::Invalid("invalid_certificate"));
    }
    Ok(())
}

/// Replay exactly the statement carried by a native judgment. A metadata name,
/// filename, hash or citation cannot change the judgment being checked.
pub fn replay(bytes: &[u8]) -> Report {
    replay_with_foundation(bytes, Foundation::Intensional)
}

/// The verifier selects the foundation, and the judgment must name exactly
/// that profile. A submitted artifact cannot silently strengthen its verifier.
pub fn replay_with_foundation(bytes: &[u8], foundation: Foundation) -> Report {
    replay_with_limits(bytes, foundation, AdmissionLimits::default())
}

/// Select structural admission resources independently of the submitted proof.
/// This in-process entry point does not sandbox evaluation; untrusted inputs
/// should use the isolated process API as well.
pub fn replay_with_limits(bytes: &[u8], foundation: Foundation, limits: AdmissionLimits) -> Report {
    let refusal = |outcome: ReplayOutcome, stage: &str, code: &str, message: &str| {
        Report::refusal(foundation, limits, outcome, stage, code, message)
    };
    if !limits.is_valid() {
        return refusal(
            ReplayOutcome::Inconclusive,
            "policy",
            "invalid_limits",
            "Admission limits cannot be represented safely.",
        );
    }
    if bytes.len() > limits.max_bytes {
        return refusal(
            ReplayOutcome::ResourceExhausted,
            "decode",
            "byte_limit",
            "Native judgment exceeds the input byte budget.",
        );
    }
    if let Err(error) = json_envelope(bytes, limits.json_nesting_limit().unwrap()) {
        return refusal(
            error.outcome(),
            "decode",
            error.code(),
            match error {
                AdmissionFailure::Exhausted(_) => {
                    "Native judgment exceeds the JSON nesting budget."
                }
                AdmissionFailure::Invalid(_) => "Native judgment has malformed JSON nesting.",
            },
        );
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    // The iterative envelope check above owns the physical nesting bound.
    // serde's unrelated default 128 counts wrappers rather than logical nodes.
    decoder.disable_recursion_limit();
    let decoded =
        Judgment::deserialize(&mut decoder).and_then(|judgment| decoder.end().map(|_| judgment));
    let judgment: Judgment = match decoded {
        Ok(j) => j,
        Err(_) => {
            return refusal(
                ReplayOutcome::Rejected,
                "decode",
                "invalid_certificate",
                "Expected one strict native judgment JSON object; duplicate or unknown fields and malformed terms are rejected.",
            );
        }
    };
    if judgment.profile != profile_for(foundation) {
        return refusal(
            ReplayOutcome::Unsupported,
            "profile",
            "unsupported_profile",
            "The requested logical profile is not supported by this checker.",
        );
    }
    if let Err(error) = admit_structure(&judgment, limits) {
        return refusal(
            error.outcome(),
            "admission",
            error.code(),
            match error {
                AdmissionFailure::Invalid(_) => {
                    "A closed native judgment contains a variable outside its binder scope."
                }
                AdmissionFailure::Exhausted(_) => {
                    "Native judgment exceeds the node or depth budget."
                }
            },
        );
    }
    if judgment.statement != judgment.certificate.claimed_type {
        return refusal(
            ReplayOutcome::Rejected,
            "statement",
            "statement_mismatch",
            "The certificate claims a different formal statement.",
        );
    }
    let check_failure = |stage: &str, error: &CheckError| {
        let outcome = match error {
            CheckError::FoundationRequired(_)
            | CheckError::UniverseOverflow { .. }
            | CheckError::KernelV0ManifestUnclean { .. }
            | CheckError::KernelV0MetaSoundnessExceeded
            | CheckError::KernelV0StrictIntrinsicDisagreement { .. } => ReplayOutcome::Unsupported,
            _ => ReplayOutcome::Rejected,
        };
        refusal(
            outcome,
            stage,
            code(error),
            "The native checker did not establish the judgment.",
        )
    };
    if let Err(e) = judgment.certificate.verify_with_foundation(foundation) {
        return check_failure("substitution", &e);
    }
    if let Err(e) = crate::proof_checker_nbe::verify_certificate_with_foundation(
        &judgment.certificate,
        foundation,
    ) {
        return check_failure("normalization", &e);
    }
    Report {
        profile: profile_for(foundation).into(),
        checker: checker_identity(),
        accepted: true,
        outcome: ReplayOutcome::Accepted,
        limits,
        worker: None,
        diagnostic: None,
    }
}
