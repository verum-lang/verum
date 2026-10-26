//! A closed native judgment, with an independently supplied exact statement.
//! Source-language elaboration, prose interpretation and SMT traces are not admitted here.
//! Execute untrusted replay in a resource-limited worker; structural limits do not bound
//! normalization cost or replace a wall-clock/process memory budget.
use crate::proof_checker::{Certificate, CheckError, DataField, Level, Term};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "verum-native/1";
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_NODES: usize = 4096;
pub const MAX_DEPTH: usize = 64;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub profile: String,
    pub checker: String,
    pub accepted: bool,
    pub diagnostic: Option<Diagnostic>,
}

/// Identity of this admission policy and the exact native checker implementations.
/// This is reproducibility metadata, not a signature or a soundness proof.
pub fn checker_identity() -> String {
    let mut h = blake3::Hasher::new();
    for part in [
        PROFILE,
        env!("CARGO_PKG_VERSION"),
        include_str!("admission.rs"),
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
fn admit_structure(judgment: &Judgment) -> Result<(), &'static str> {
    enum Node<'a> {
        Term(&'a Term, usize),
        Level(&'a Level),
    }
    let mut queue = vec![
        (Node::Term(&judgment.statement, 0), 0),
        (Node::Term(&judgment.certificate.term, 0), 0),
        (Node::Term(&judgment.certificate.claimed_type, 0), 0),
    ];
    let mut count = 0;
    while let Some((node, depth)) = queue.pop() {
        count += 1;
        if count > MAX_NODES || depth > MAX_DEPTH {
            return Err("structural_limit");
        }
        match node {
            Node::Term(t, binders) => match t {
                Term::Data {
                    level,
                    constructors,
                    ..
                } => {
                    queue.push((Node::Level(level), depth + 1));
                    count += constructors.len();
                    for fields in constructors {
                        for (index, field) in fields.iter().enumerate() {
                            count += 1;
                            match field {
                                DataField::Value(ty) => {
                                    queue.push((Node::Term(ty, binders), depth + 1))
                                }
                                DataField::Dependent(ty) => {
                                    queue.push((Node::Term(ty, binders + index), depth + 1))
                                }
                                DataField::Recursive => {}
                            }
                        }
                    }
                    if count > MAX_NODES {
                        return Err("structural_limit");
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
                        return Err("unbound_variable");
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

// The wire representation can add five JSON containers between consecutive
// logical nodes (Data -> payload -> constructors -> fields -> Value). The
// judgment envelope adds fewer than eight. Bound those containers before
// recursive deserialization; the unchanged logical limits are checked next.
const MAX_JSON_NESTING: usize = 5 * (MAX_DEPTH + 1) + 8;

fn json_envelope(bytes: &[u8]) -> Result<(), &'static str> {
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
                if containers.len() == MAX_JSON_NESTING {
                    return Err("json_depth_limit");
                }
                containers.push(*byte);
            }
            b'}' | b']' => {
                let expected = if *byte == b'}' { b'{' } else { b'[' };
                if containers.pop() != Some(expected) {
                    return Err("invalid_certificate");
                }
            }
            _ => {}
        }
    }
    if quoted || !containers.is_empty() {
        return Err("invalid_certificate");
    }
    Ok(())
}

/// Replay exactly the statement carried by a native judgment. A metadata name,
/// filename, hash or citation cannot change the judgment being checked.
pub fn replay(bytes: &[u8]) -> Report {
    let rejected = |stage: &str, code: &str, message: &str| Report {
        profile: PROFILE.into(),
        checker: checker_identity(),
        accepted: false,
        diagnostic: Some(Diagnostic {
            code: code.into(),
            stage: stage.into(),
            message: message.into(),
        }),
    };
    if bytes.len() > MAX_BYTES {
        return rejected(
            "decode",
            "byte_limit",
            "Native judgment exceeds the input byte budget.",
        );
    }
    if let Err(code) = json_envelope(bytes) {
        return rejected(
            "decode",
            code,
            "Native judgment has malformed or excessive JSON nesting.",
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
            return rejected(
                "decode",
                "invalid_certificate",
                "Expected one strict native judgment JSON object; duplicate or unknown fields and malformed terms are rejected.",
            );
        }
    };
    if judgment.profile != PROFILE {
        return rejected(
            "profile",
            "unsupported_profile",
            "The requested logical profile is not supported by this checker.",
        );
    }
    if let Err(code) = admit_structure(&judgment) {
        return rejected(
            "admission",
            code,
            if code == "unbound_variable" {
                "A closed native judgment contains a variable outside its binder scope."
            } else {
                "Native judgment exceeds the node or depth budget."
            },
        );
    }
    if judgment.statement != judgment.certificate.claimed_type {
        return rejected(
            "statement",
            "statement_mismatch",
            "The certificate claims a different formal statement.",
        );
    }
    if let Err(e) = judgment.certificate.verify() {
        return rejected(
            "substitution",
            code(&e),
            "The native substitution checker rejected the judgment.",
        );
    }
    if let Err(e) = crate::proof_checker_nbe::verify_certificate(&judgment.certificate) {
        return rejected(
            "normalization",
            code(&e),
            "The normalization checker rejected the judgment; no proof authority is granted.",
        );
    }
    Report {
        profile: PROFILE.into(),
        checker: checker_identity(),
        accepted: true,
        diagnostic: None,
    }
}
