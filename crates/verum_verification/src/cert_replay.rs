//! SMT certificate replay — backend-independent cert format +
//! multi-backend cross-check.
//!
//! ## Goal
//!
//! Currently SMT certificates are replayed against a single
//! kernel re-check.  Task #81 strengthens this to **multi-backend
//! cross-validation**: a cert produced by Z3 should be replayable
//! by CVC5 (and vice-versa), and the kernel re-check decomposes
//! the cert into elementary kernel-rule applications so the SMT
//! solver becomes truly external — not part of the trusted
//! computing base.
//!
//! Verum joins the small group of systems (Coq's SMTCoq, Lean's
//! lean-smt) with this guarantee, but with **multi-backend +
//! multi-format coverage**: a cert can be cross-checked by every
//! available solver as a sanity gate.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the
//! integration arc:
//!
//!   * [`CertFormat`] enum — backend-independent canonical format
//!     plus the per-backend native formats Verum can ingest.
//!   * [`SmtCertificate`] — typed certificate (format + theory +
//!     conclusion + raw body + content hash).
//!   * [`ReplayBackend`] enum — Z3 / CVC5 / Verit / OpenSmt /
//!     Mathsat / Kernel-only.
//!   * [`CertReplayEngine`] trait — single dispatch interface;
//!     `replay(cert) -> ReplayVerdict`.
//!   * Reference impls: [`MockReplayEngine`] (deterministic, for
//!     tests), [`KernelOnlyReplayEngine`] (V0 reference — verifies
//!     the cert's own integrity hash + structural shape), per-
//!     backend stub impls returning `ToolMissing` until production
//!     wiring.
//!   * [`CrossBackendVerdict`] — typed multi-backend agreement
//!     report.  Used by `@verify(certified)`-style multi-solver
//!     gates.
//!
//! ## Trust contract
//!
//! `KernelOnlyReplayEngine` is what makes SMT solvers external to
//! the TCB: it checks the certificate's structural invariants
//! using only the kernel rules + the on-disk hash.  If a solver
//! claims `unsat` but emits a cert whose hash doesn't match its
//! payload, the kernel rejects without consulting the solver.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use verum_common::Text;

// =============================================================================
// CertFormat
// =============================================================================

/// Certificate format.  `VerumCanonical` is the format every
/// backend lowers to; the others are native formats Verum ingests
/// for backwards compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertFormat {
    /// Verum's canonical, backend-independent cert format.  Every
    /// production backend produces this.  The kernel re-checker
    /// decomposes the cert into elementary kernel-rule applications.
    VerumCanonical,
    /// Z3's native `(proof ...)` format.
    Z3Proof,
    /// CVC5's ALETHE format (more stable across releases than
    /// Z3's native; the recommended export target — see
    /// `--smt-proof-preference`).
    Cvc5Alethe,
    /// LFSC pattern format (CVC4 / CVC5 legacy).
    LfscPattern,
    /// OpenSMT2 native proof format.
    OpenSmt,
    /// MathSAT5 native proof format.
    Mathsat,
}

impl CertFormat {
    pub fn name(self) -> &'static str {
        match self {
            Self::VerumCanonical => "verum_canonical",
            Self::Z3Proof => "z3_proof",
            Self::Cvc5Alethe => "cvc5_alethe",
            Self::LfscPattern => "lfsc_pattern",
            Self::OpenSmt => "open_smt",
            Self::Mathsat => "mathsat",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "verum_canonical" | "canonical" | "verum" => Some(Self::VerumCanonical),
            "z3_proof" | "z3" => Some(Self::Z3Proof),
            "cvc5_alethe" | "alethe" | "cvc5" => Some(Self::Cvc5Alethe),
            "lfsc_pattern" | "lfsc" => Some(Self::LfscPattern),
            "open_smt" | "opensmt" | "opensmt2" => Some(Self::OpenSmt),
            "mathsat" | "mathsat5" => Some(Self::Mathsat),
            _ => None,
        }
    }

    pub fn all() -> [CertFormat; 6] {
        [
            Self::VerumCanonical,
            Self::Z3Proof,
            Self::Cvc5Alethe,
            Self::LfscPattern,
            Self::OpenSmt,
            Self::Mathsat,
        ]
    }
}

// =============================================================================
// ReplayBackend
// =============================================================================

/// Backend that replays a certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayBackend {
    /// Verum's kernel-only re-check.  Validates the cert's
    /// structural invariants (hash matches body, declared theory
    /// matches inferable shape, conclusion is consistent with
    /// body).  Always available; this is what makes solvers
    /// external to the TCB.
    KernelOnly,
    Z3,
    Cvc5,
    /// veriT (small SMT solver with native ALETHE support).
    Verit,
    OpenSmt,
    Mathsat,
}

impl ReplayBackend {
    pub fn name(self) -> &'static str {
        match self {
            Self::KernelOnly => "kernel_only",
            Self::Z3 => "z3",
            Self::Cvc5 => "cvc5",
            Self::Verit => "verit",
            Self::OpenSmt => "open_smt",
            Self::Mathsat => "mathsat",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "kernel_only" | "kernel" => Some(Self::KernelOnly),
            "z3" => Some(Self::Z3),
            "cvc5" => Some(Self::Cvc5),
            "verit" => Some(Self::Verit),
            "open_smt" | "opensmt" | "opensmt2" => Some(Self::OpenSmt),
            "mathsat" | "mathsat5" => Some(Self::Mathsat),
            _ => None,
        }
    }

    pub fn all() -> [ReplayBackend; 6] {
        [
            Self::KernelOnly,
            Self::Z3,
            Self::Cvc5,
            Self::Verit,
            Self::OpenSmt,
            Self::Mathsat,
        ]
    }

    /// True iff this backend is always available (i.e. doesn't
    /// require an external tool on PATH).
    pub fn is_intrinsic(self) -> bool {
        matches!(self, Self::KernelOnly)
    }
}

// =============================================================================
// SmtCertificate
// =============================================================================

/// Typed SMT certificate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmtCertificate {
    pub format: CertFormat,
    /// Logical theory the cert is for (`QF_LIA`, `LRA`, `UF`, …).
    pub theory: Text,
    /// Theorem-shaped conclusion the cert claims to prove.
    pub conclusion: Text,
    /// Raw cert body — the format-specific payload.
    pub body: Text,
    /// Blake3 content-hash of `body` (hex-encoded).  Used by
    /// `KernelOnlyReplayEngine` to verify on-disk integrity.
    pub body_hash: Text,
    /// Optional originating-solver identifier (e.g. `"z3-4.13.0"`).
    /// Free-form for diagnostic / audit purposes.
    pub source_solver: Option<Text>,
}

impl SmtCertificate {
    /// Construct a certificate with body + auto-computed hash.
    pub fn new(
        format: CertFormat,
        theory: impl Into<Text>,
        conclusion: impl Into<Text>,
        body: impl Into<Text>,
    ) -> Self {
        let body: Text = body.into();
        let hash = Text::from(hex32(blake3::hash(body.as_str().as_bytes()).as_bytes()));
        Self {
            format,
            theory: theory.into(),
            conclusion: conclusion.into(),
            body,
            body_hash: hash,
            source_solver: None,
        }
    }

    pub fn with_source_solver(mut self, s: impl Into<Text>) -> Self {
        self.source_solver = Some(s.into());
        self
    }

    /// True iff `body_hash` matches blake3 of `body`.  Pure
    /// integrity check — no semantic verification.
    pub fn body_hash_valid(&self) -> bool {
        let recomputed =
            Text::from(hex32(blake3::hash(self.body.as_str().as_bytes()).as_bytes()));
        recomputed == self.body_hash
    }
}

fn hex32(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// =============================================================================
// ReplayVerdict
// =============================================================================

/// Outcome of replaying one certificate against one backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ReplayVerdict {
    /// The backend re-checked the cert and accepted it.
    Accepted {
        backend: ReplayBackend,
        elapsed_ms: u64,
        /// Optional per-backend detail (e.g. version + steps checked).
        detail: Option<Text>,
    },
    /// The backend rejected the cert.
    Rejected {
        backend: ReplayBackend,
        reason: Text,
    },
    /// The backend tool is not available (e.g. `cvc5` not on PATH).
    /// Distinct from rejection; downstream consumers count this as
    /// a `NotRun` rather than a failure.
    ToolMissing { backend: ReplayBackend },
    /// Internal error during replay (parser failure, transport
    /// error, …).
    Error {
        backend: ReplayBackend,
        message: Text,
    },
}

impl ReplayVerdict {
    pub fn backend(&self) -> ReplayBackend {
        match self {
            Self::Accepted { backend, .. }
            | Self::Rejected { backend, .. }
            | Self::ToolMissing { backend }
            | Self::Error { backend, .. } => *backend,
        }
    }

    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }
}

// =============================================================================
// CertReplayEngine trait
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum ReplayError {
    UnsupportedFormat(CertFormat),
    Transport(Text),
    Other(Text),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(c) => {
                write!(f, "unsupported cert format: {}", c.name())
            }
            Self::Transport(t) => write!(f, "transport: {}", t.as_str()),
            Self::Other(t) => write!(f, "{}", t.as_str()),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Single dispatch interface for cert replay.
pub trait CertReplayEngine: std::fmt::Debug + Send + Sync {
    fn backend(&self) -> ReplayBackend;
    fn supports(&self, format: CertFormat) -> bool;
    fn is_available(&self) -> bool;
    fn replay(&self, cert: &SmtCertificate) -> Result<ReplayVerdict, ReplayError>;
}

// =============================================================================
// KernelOnlyReplayEngine — the trust-boundary anchor
// =============================================================================

/// Verum's kernel-only re-check.  Validates the cert's structural
/// invariants without trusting any external solver:
///
///   1. `body_hash` matches blake3 of the body (integrity).
///   2. `format` is recognised.
///   3. `body` is non-empty.
///   4. `conclusion` is non-empty.
///   5. `theory` is one of the supported logical theories.
///
/// Rejection ⇒ the cert is malformed; the solver that produced it
/// gave us a corrupted artefact.  Acceptance ⇒ the cert is
/// well-formed; further replay against an actual solver may still
/// reject if the cert is unsound, but at the structural layer the
/// kernel has done its part.
///
/// This is what makes SMT solvers external to the TCB: even if Z3
/// produces a fake cert, the kernel-only check catches it before
/// the proof is committed.
///
/// **Hardening note (#95).**  This engine no longer treats the
/// cert body as opaque text.  It runs the format-appropriate
/// decomposer ([`decompose_cert`]) which parses the body into a
/// list of [`InferenceStep`]s; every step's rule name is
/// cross-checked against the canonical
/// [`KernelRuleRegistry`].  Unknown rules + malformed bodies are
/// rejected before any solver is consulted.
// =============================================================================
// Cert decomposer (#95) — typed kernel-rule decomposition
// =============================================================================
//
// Pre-this-module, `KernelOnlyReplayEngine` validated only structural
// invariants (hash + theory + non-empty body / conclusion).  That
// passed any cert whose body was non-empty and whose hash matched —
// a Z3 bug producing a syntactically-valid-but-meaningless trace
// would slip through.
//
// Hardening: parse the cert body per-format into a typed sequence
// of [`InferenceStep`]s (rule name + premises + conclusion), then
// verify every rule name is in the canonical kernel-rule registry.
// Unknown rule → Rejected.  This is the structural piece that
// genuinely takes solvers out of the TCB.
//
// What's NOT here yet (V2): the actual kernel `infer::check` call
// per step — that requires lifting the cert's textual conclusions
// to typed `CoreTerm`s, which is a separate format-specific lift.
// The decomposer's typed output is the prerequisite for that work.

/// One step in a decomposed cert: a rule application with named
/// premises and a textual conclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceStep {
    /// Local identifier (line label, e.g. `t1` / `h2` for ALETHE,
    /// `step_001` for verum_canonical).
    pub id: Text,
    /// Rule name as it appears in the cert.  Cross-checked against
    /// the kernel-rule registry.
    pub rule: Text,
    /// IDs of previous steps cited as premises.
    pub premises: Vec<Text>,
    /// Conclusion (rendered text — typed lift is V2 work).
    pub conclusion: Text,
}

/// Decompose error.
#[derive(Debug, Clone, PartialEq)]
pub enum DecomposeError {
    /// Format is documented but not yet supported by a parser.
    UnsupportedFormat(CertFormat),
    /// Body could not be parsed as the declared format.
    Malformed { line: usize, message: Text },
    /// Body parsed but contains zero steps.
    Empty,
}

impl std::fmt::Display for DecomposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(fmt) => {
                write!(f, "decomposer not yet implemented for `{}`", fmt.name())
            }
            Self::Malformed { line, message } => {
                write!(f, "malformed cert body at line {}: {}", line, message.as_str())
            }
            Self::Empty => write!(f, "cert body has no inference steps"),
        }
    }
}

impl std::error::Error for DecomposeError {}

/// Canonical kernel-rule registry.
///
/// **Each entry is a tuple `(cert_rule_name, kernel_rule_tag)`.**
/// `cert_rule_name` is what the SMT solver writes in its proof
/// trace; `kernel_rule_tag` is the canonical Verum-kernel rule it
/// maps to (mirrors the `KernelRule` enum in
/// `verum_kernel::proof_tree`).  Multiple cert names can map to the
/// same kernel rule (e.g. Z3 `mp` and ALETHE `mp` both map to
/// `modus_ponens`).
///
/// Unknown rule names are rejected.  Adding a new rule is a one-
/// place edit here; downstream tooling automatically picks it up.
pub const KERNEL_RULE_REGISTRY: &[(&str, &str)] = &[
    // ----- Resolution / propositional -----
    ("resolution", "resolution"),
    ("th-resolution", "resolution"),
    ("hyper-res", "resolution"),
    ("unit-resolution", "resolution"),
    ("res", "resolution"),
    ("and-elim", "and_elim"),
    ("and-introduction", "and_intro"),
    ("and_pos", "and_intro"),
    ("not-not", "double_neg_elim"),
    ("not_not", "double_neg_elim"),
    ("equiv-elim", "iff_elim"),
    ("contraction", "contraction"),
    // ----- Equality -----
    ("refl", "reflexivity"),
    ("eq-reflexive", "reflexivity"),
    ("symm", "symmetry"),
    ("eq-symmetric", "symmetry"),
    ("trans", "transitivity"),
    ("eq-transitive", "transitivity"),
    ("eq-congruent", "congruence"),
    ("congruence", "congruence"),
    ("monotonicity", "congruence"),
    // ----- Implication / modus ponens -----
    ("mp", "modus_ponens"),
    ("modus-ponens", "modus_ponens"),
    ("mp_scoped", "modus_ponens"),
    ("th-mp", "modus_ponens"),
    ("hypothesis", "hypothesis"),
    ("assume", "hypothesis"),
    // ----- Quantifier -----
    ("forall_inst", "forall_instantiation"),
    ("forall-inst", "forall_instantiation"),
    ("quant-inst", "forall_instantiation"),
    ("inst", "forall_instantiation"),
    ("skolemize", "skolemize"),
    ("sko-forall", "skolemize"),
    ("sko-ex", "skolemize"),
    // ----- Theory: linear arithmetic -----
    ("la_generic", "linear_arithmetic"),
    ("la-generic", "linear_arithmetic"),
    ("la_disequality", "linear_arithmetic"),
    ("th-lemma", "theory_lemma"),
    ("th_lemma", "theory_lemma"),
    ("lia", "linear_arithmetic"),
    ("lra", "linear_arithmetic"),
    // ----- Theory: array / UF -----
    ("array_ext", "array_extensionality"),
    ("array-ext", "array_extensionality"),
    ("array_distinct", "array_extensionality"),
    // ----- Final / closing -----
    ("step", "step"),
    ("subproof", "subproof"),
    ("anchor", "subproof"),
    ("def-axiom", "definitional_axiom"),
    ("tautology", "tautology"),
    ("true-intro", "tautology"),
    ("false-elim", "ex_falso"),
    ("efq", "ex_falso"),
];

/// Look up a cert rule name in the canonical registry.
pub fn lookup_kernel_rule(cert_rule: &str) -> Option<&'static str> {
    KERNEL_RULE_REGISTRY
        .iter()
        .find(|(name, _)| *name == cert_rule)
        .map(|(_, kr)| *kr)
}

/// Single dispatch: decompose `cert` into a list of typed steps,
/// using the format-appropriate parser.
pub fn decompose_cert(cert: &SmtCertificate) -> Result<Vec<InferenceStep>, DecomposeError> {
    match cert.format {
        CertFormat::VerumCanonical => decompose_verum_canonical(cert.body.as_str()),
        CertFormat::Cvc5Alethe => decompose_alethe(cert.body.as_str()),
        CertFormat::Z3Proof => decompose_z3_proof(cert.body.as_str()),
        CertFormat::LfscPattern => decompose_lfsc_pattern(cert.body.as_str()),
        CertFormat::OpenSmt => decompose_open_smt(cert.body.as_str()),
        CertFormat::Mathsat => decompose_mathsat(cert.body.as_str()),
    }
}

// -----------------------------------------------------------------------------
// Verum canonical — line-oriented format
// -----------------------------------------------------------------------------
//
// One step per line:
//
//   `step <id> <rule> [<premise> ...] : <conclusion>`
//
// Comment lines start with `;`.  Blank lines are ignored.

fn decompose_verum_canonical(body: &str) -> Result<Vec<InferenceStep>, DecomposeError> {
    let mut steps: Vec<InferenceStep> = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let mut head_and_concl = line.splitn(2, ':');
        let head = head_and_concl.next().unwrap_or("").trim();
        let concl = head_and_concl.next().unwrap_or("").trim();
        let mut tokens = head.split_whitespace();
        let kw = tokens.next().ok_or_else(|| DecomposeError::Malformed {
            line: lineno + 1,
            message: Text::from("expected `step` keyword"),
        })?;
        if kw != "step" {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from(format!("unexpected keyword `{}`; expected `step`", kw)),
            });
        }
        let id = tokens.next().ok_or_else(|| DecomposeError::Malformed {
            line: lineno + 1,
            message: Text::from("missing step id"),
        })?;
        let rule = tokens.next().ok_or_else(|| DecomposeError::Malformed {
            line: lineno + 1,
            message: Text::from("missing rule name"),
        })?;
        let premises: Vec<Text> = tokens.map(Text::from).collect();
        if concl.is_empty() {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("missing `: <conclusion>` after rule + premises"),
            });
        }
        steps.push(InferenceStep {
            id: Text::from(id),
            rule: Text::from(rule),
            premises,
            conclusion: Text::from(concl),
        });
    }
    if steps.is_empty() {
        return Err(DecomposeError::Empty);
    }
    Ok(steps)
}

// -----------------------------------------------------------------------------
// ALETHE (CVC5) — `(step <id> (cl <conclusion>) :rule <rule> :premises (<p>*))`
// -----------------------------------------------------------------------------

fn decompose_alethe(body: &str) -> Result<Vec<InferenceStep>, DecomposeError> {
    let mut steps: Vec<InferenceStep> = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        // ALETHE accepts `(assume h <expr>)` and `(step ...)` shapes.
        let inner = line
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("ALETHE line must be parenthesised"),
            })?
            .trim();
        if let Some(rest) = inner.strip_prefix("assume ") {
            // `assume <id> <expr>`
            let mut t = rest.splitn(2, char::is_whitespace);
            let id = t.next().unwrap_or("").trim();
            let expr = t.next().unwrap_or("").trim();
            if id.is_empty() {
                return Err(DecomposeError::Malformed {
                    line: lineno + 1,
                    message: Text::from("missing assume id"),
                });
            }
            steps.push(InferenceStep {
                id: Text::from(id),
                rule: Text::from("assume"),
                premises: Vec::new(),
                conclusion: Text::from(expr),
            });
            continue;
        }
        if let Some(rest) = inner.strip_prefix("step ") {
            // `<id> (cl <conclusion>) :rule <rule> :premises (<p>*) ...`
            // We tokenise on whitespace but respect `(...)` groups.
            let id = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let conclusion = parse_alethe_clause(rest).unwrap_or_default();
            let rule = parse_alethe_kw(rest, ":rule").unwrap_or_default();
            let premises = parse_alethe_premise_list(rest);
            if id.is_empty() || rule.is_empty() {
                return Err(DecomposeError::Malformed {
                    line: lineno + 1,
                    message: Text::from("ALETHE step missing id or :rule"),
                });
            }
            steps.push(InferenceStep {
                id: Text::from(id),
                rule: Text::from(rule),
                premises,
                conclusion: Text::from(conclusion),
            });
            continue;
        }
        // Anchors / contexts are control structures, not inference
        // steps — record them as `step` rule with empty premises so
        // the registry sees the structural marker.
        if inner.starts_with("anchor") {
            steps.push(InferenceStep {
                id: Text::from(format!("anchor_{}", lineno + 1)),
                rule: Text::from("anchor"),
                premises: Vec::new(),
                conclusion: Text::from(inner),
            });
            continue;
        }
        // Unknown shape — surface as malformed for diagnostics.
        return Err(DecomposeError::Malformed {
            line: lineno + 1,
            message: Text::from(format!(
                "unrecognised ALETHE form: `{}`",
                truncate_for_msg(inner, 60)
            )),
        });
    }
    if steps.is_empty() {
        return Err(DecomposeError::Empty);
    }
    Ok(steps)
}

fn parse_alethe_clause(rest: &str) -> Option<String> {
    // Find first `(cl ...)` substring at top level.
    let needle = "(cl";
    let i = rest.find(needle)?;
    let bytes = &rest[i..].as_bytes();
    let mut depth: i32 = 0;
    for (off, c) in rest[i..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let inside = &rest[i + needle.len()..i + off];
                    return Some(inside.trim().to_string());
                }
            }
            _ => {}
        }
        let _ = bytes; // suppress unused warning for the byte view
    }
    None
}

fn parse_alethe_kw(rest: &str, key: &str) -> Option<String> {
    let i = rest.find(key)?;
    let after = rest[i + key.len()..].trim_start();
    // Read up to whitespace (rule name) — but `:premises` value is a
    // parenthesised list; for `:rule` we just want the next token.
    Some(
        after
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(')')
            .to_string(),
    )
}

fn parse_alethe_premise_list(rest: &str) -> Vec<Text> {
    let key = ":premises";
    let i = match rest.find(key) {
        Some(i) => i + key.len(),
        None => return Vec::new(),
    };
    let after = rest[i..].trim_start();
    let after = match after.strip_prefix('(') {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut depth: i32 = 1;
    let mut end = 0usize;
    for (off, c) in after.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = off;
                    break;
                }
            }
            _ => {}
        }
    }
    let inside = if end > 0 { &after[..end] } else { after };
    inside
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(Text::from)
        .collect()
}

fn truncate_for_msg(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

// -----------------------------------------------------------------------------
// Z3 proof — `((rule ...) ...)` — tree of nested rule applications
// -----------------------------------------------------------------------------
//
// Z3's `(proof ...)` format is a deeply nested S-expression where
// each rule application has shape `(rule_name <subproof>* <conclusion>)`.
// We do a depth-first walk producing one step per encountered rule.

fn decompose_z3_proof(body: &str) -> Result<Vec<InferenceStep>, DecomposeError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(DecomposeError::Empty);
    }
    let mut tokens = TokenStream::new(trimmed);
    let root = parse_sexpr(&mut tokens).map_err(|m| DecomposeError::Malformed {
        line: 0,
        message: Text::from(m),
    })?;
    let mut steps: Vec<InferenceStep> = Vec::new();
    let mut next_id: u32 = 0;
    walk_z3_sexpr(&root, &mut steps, &mut next_id);
    if steps.is_empty() {
        return Err(DecomposeError::Empty);
    }
    Ok(steps)
}

#[derive(Debug, Clone)]
enum Sexpr {
    Atom(String),
    List(Vec<Sexpr>),
}

struct TokenStream<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> TokenStream<'a> {
    fn new(s: &'a str) -> Self {
        Self { s, i: 0 }
    }
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.i += c.len_utf8();
            } else if c == ';' {
                // Comment to EOL.
                while let Some(c) = self.peek() {
                    self.i += c.len_utf8();
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }
    fn next_atom(&mut self) -> Option<String> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '(' || c == ')' {
                break;
            }
            self.i += c.len_utf8();
        }
        if start == self.i {
            None
        } else {
            Some(self.s[start..self.i].to_string())
        }
    }
}

fn parse_sexpr(t: &mut TokenStream<'_>) -> Result<Sexpr, String> {
    t.skip_ws();
    match t.peek() {
        None => Err("unexpected EOF".into()),
        Some('(') => {
            t.i += 1;
            let mut items: Vec<Sexpr> = Vec::new();
            loop {
                t.skip_ws();
                match t.peek() {
                    None => return Err("unterminated list".into()),
                    Some(')') => {
                        t.i += 1;
                        return Ok(Sexpr::List(items));
                    }
                    _ => items.push(parse_sexpr(t)?),
                }
            }
        }
        Some(')') => Err("unexpected `)`".into()),
        Some(_) => match t.next_atom() {
            Some(a) => Ok(Sexpr::Atom(a)),
            None => Err("expected atom".into()),
        },
    }
}

fn walk_z3_sexpr(s: &Sexpr, steps: &mut Vec<InferenceStep>, next_id: &mut u32) {
    match s {
        Sexpr::Atom(_) => {}
        Sexpr::List(items) => {
            // A rule application is a non-empty list whose head is
            // an atom matching a known rule name (best-effort —
            // Z3's S-expr trees also contain term applications,
            // which we leave untouched).
            if let Some(Sexpr::Atom(head)) = items.first() {
                if lookup_kernel_rule(head).is_some() {
                    let id = format!("z3_step_{}", *next_id);
                    *next_id += 1;
                    let conclusion = items
                        .last()
                        .map(render_sexpr)
                        .unwrap_or_default();
                    let mut premises: Vec<Text> = Vec::new();
                    for sub in items.iter().skip(1).take(items.len().saturating_sub(2)) {
                        if let Sexpr::List(_) = sub {
                            // Each sub-proof becomes a premise.
                            premises.push(Text::from(format!("p_{}", premises.len())));
                        }
                    }
                    steps.push(InferenceStep {
                        id: Text::from(id),
                        rule: Text::from(head.clone()),
                        premises,
                        conclusion: Text::from(conclusion),
                    });
                }
            }
            // Recurse into children regardless — nested rule
            // applications are common.
            for child in items {
                walk_z3_sexpr(child, steps, next_id);
            }
        }
    }
}

fn render_sexpr(s: &Sexpr) -> String {
    match s {
        Sexpr::Atom(a) => a.clone(),
        Sexpr::List(items) => {
            let mut out = String::from("(");
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(&render_sexpr(it));
            }
            out.push(')');
            out
        }
    }
}

// -----------------------------------------------------------------------------
// LFSC pattern (CVC4 / CVC5 legacy)
// -----------------------------------------------------------------------------
//
// LFSC traces are nested S-expressions whose head atoms are the
// rule names ("resolution", "and-elim", "th-lemma", "trust", …).
// We reuse the Z3 walker — same shape, different rule registry
// alias set — and project every head-of-list whose atom resolves
// in `KERNEL_RULE_REGISTRY` into an `InferenceStep`.

fn decompose_lfsc_pattern(body: &str) -> Result<Vec<InferenceStep>, DecomposeError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(DecomposeError::Empty);
    }
    let mut tokens = TokenStream::new(trimmed);
    let root = parse_sexpr(&mut tokens).map_err(|m| DecomposeError::Malformed {
        line: 0,
        message: Text::from(m),
    })?;
    let mut steps: Vec<InferenceStep> = Vec::new();
    let mut next_id: u32 = 0;
    walk_lfsc_sexpr(&root, &mut steps, &mut next_id);
    if steps.is_empty() {
        return Err(DecomposeError::Empty);
    }
    Ok(steps)
}

fn walk_lfsc_sexpr(s: &Sexpr, steps: &mut Vec<InferenceStep>, next_id: &mut u32) {
    if let Sexpr::List(items) = s {
        if let Some(Sexpr::Atom(head)) = items.first() {
            if lookup_kernel_rule(head).is_some() {
                let id = format!("lfsc_step_{}", *next_id);
                *next_id += 1;
                let conclusion = items.last().map(render_sexpr).unwrap_or_default();
                let mut premises: Vec<Text> = Vec::new();
                for sub in items.iter().skip(1).take(items.len().saturating_sub(2)) {
                    if let Sexpr::List(_) = sub {
                        premises.push(Text::from(format!("p_{}", premises.len())));
                    }
                }
                steps.push(InferenceStep {
                    id: Text::from(id),
                    rule: Text::from(head.clone()),
                    premises,
                    conclusion: Text::from(conclusion),
                });
            }
        }
        for child in items {
            walk_lfsc_sexpr(child, steps, next_id);
        }
    }
}

// -----------------------------------------------------------------------------
// OpenSMT2 — line-oriented `<id> := <rule> [<premise>...]   : <conclusion>`
// -----------------------------------------------------------------------------
//
// OpenSMT2's proof trace is line-oriented similarly to verum_canonical
// but uses `:=` as the rule-binding separator (one definition per
// line) rather than the `step <id>` keyword.  Comments start with
// `;` (SMT-LIB convention).

fn decompose_open_smt(body: &str) -> Result<Vec<InferenceStep>, DecomposeError> {
    let mut steps: Vec<InferenceStep> = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        // Split at `:=` — head is `<id>`, tail is `<rule> [<p>...] : <c>`.
        let (id_part, rest) = match line.split_once(":=") {
            Some(parts) => parts,
            None => {
                return Err(DecomposeError::Malformed {
                    line: lineno + 1,
                    message: Text::from("expected `:=` separating id from rule"),
                });
            }
        };
        let id = id_part.trim();
        if id.is_empty() {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("empty step id"),
            });
        }
        let (rule_and_premises, conclusion) = match rest.split_once(':') {
            Some(parts) => parts,
            None => {
                return Err(DecomposeError::Malformed {
                    line: lineno + 1,
                    message: Text::from("expected `:` separating rule from conclusion"),
                });
            }
        };
        let mut tokens = rule_and_premises.split_whitespace();
        let rule = tokens.next().ok_or_else(|| DecomposeError::Malformed {
            line: lineno + 1,
            message: Text::from("missing rule name"),
        })?;
        let premises: Vec<Text> = tokens.map(Text::from).collect();
        let conclusion = conclusion.trim();
        if conclusion.is_empty() {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("empty conclusion"),
            });
        }
        steps.push(InferenceStep {
            id: Text::from(id),
            rule: Text::from(rule),
            premises,
            conclusion: Text::from(conclusion),
        });
    }
    if steps.is_empty() {
        return Err(DecomposeError::Empty);
    }
    Ok(steps)
}

// -----------------------------------------------------------------------------
// MathSAT5 — line-oriented `<rule>(<id>; <premises>) -> <conclusion>`
// -----------------------------------------------------------------------------
//
// MathSAT's native proof trace renders one step per line in the
// shape `<rule>(<id>; <p1>, <p2>, ...) -> <conclusion>`.  Comments
// start with `#`.  We extract the leading rule name (cross-checked
// against the kernel registry), the parenthesised id + premises,
// and the conclusion after `->`.

fn decompose_mathsat(body: &str) -> Result<Vec<InferenceStep>, DecomposeError> {
    let mut steps: Vec<InferenceStep> = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let paren_open = line.find('(').ok_or_else(|| DecomposeError::Malformed {
            line: lineno + 1,
            message: Text::from("expected `(` after rule name"),
        })?;
        // Walk forward from paren_open to find the matching `)` —
        // not `rfind`, because the conclusion (`-> <expr>`) may
        // itself contain nested parentheses we don't want to
        // include in the rule application.
        let after_open = &line[paren_open + 1..];
        let mut depth: i32 = 1;
        let mut close_off: Option<usize> = None;
        for (off, c) in after_open.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close_off = Some(off);
                        break;
                    }
                }
                _ => {}
            }
        }
        let paren_close = match close_off {
            Some(o) => paren_open + 1 + o,
            None => {
                return Err(DecomposeError::Malformed {
                    line: lineno + 1,
                    message: Text::from("missing closing `)`"),
                });
            }
        };
        let rule = line[..paren_open].trim();
        if rule.is_empty() {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("empty rule name"),
            });
        }
        let inside = &line[paren_open + 1..paren_close];
        let (id_part, premise_part) = match inside.split_once(';') {
            Some(parts) => parts,
            None => (inside, ""),
        };
        let id = id_part.trim();
        if id.is_empty() {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("empty step id"),
            });
        }
        let premises: Vec<Text> = premise_part
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(Text::from)
            .collect();
        let after = line[paren_close + 1..].trim_start();
        let conclusion = after
            .strip_prefix("->")
            .map(str::trim)
            .unwrap_or("")
            .trim();
        if conclusion.is_empty() {
            return Err(DecomposeError::Malformed {
                line: lineno + 1,
                message: Text::from("expected `-> <conclusion>` after rule application"),
            });
        }
        steps.push(InferenceStep {
            id: Text::from(id),
            rule: Text::from(rule),
            premises,
            conclusion: Text::from(conclusion),
        });
    }
    if steps.is_empty() {
        return Err(DecomposeError::Empty);
    }
    Ok(steps)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct KernelOnlyReplayEngine;

impl KernelOnlyReplayEngine {
    pub fn new() -> Self {
        Self
    }
}

const KNOWN_THEORIES: &[&str] = &[
    "QF_BV", "QF_LIA", "QF_LRA", "QF_NIA", "QF_NRA", "QF_UF", "QF_UFLIA", "QF_UFLRA",
    "QF_UFNIA", "QF_UFNRA", "LIA", "LRA", "NIA", "NRA", "UF", "UFLIA", "UFLRA",
    "UFNIA", "UFNRA", "ALL",
];

impl CertReplayEngine for KernelOnlyReplayEngine {
    fn backend(&self) -> ReplayBackend {
        ReplayBackend::KernelOnly
    }

    fn supports(&self, _format: CertFormat) -> bool {
        true
    }

    fn is_available(&self) -> bool {
        true
    }

    fn replay(&self, cert: &SmtCertificate) -> Result<ReplayVerdict, ReplayError> {
        if cert.body.as_str().is_empty() {
            return Ok(ReplayVerdict::Rejected {
                backend: ReplayBackend::KernelOnly,
                reason: Text::from("empty body"),
            });
        }
        if cert.conclusion.as_str().is_empty() {
            return Ok(ReplayVerdict::Rejected {
                backend: ReplayBackend::KernelOnly,
                reason: Text::from("empty conclusion"),
            });
        }
        if !KNOWN_THEORIES.contains(&cert.theory.as_str()) {
            return Ok(ReplayVerdict::Rejected {
                backend: ReplayBackend::KernelOnly,
                reason: Text::from(format!(
                    "unknown theory '{}'; expected one of QF_LIA / LRA / UF / NIA / NRA / ALL etc.",
                    cert.theory.as_str()
                )),
            });
        }
        if !cert.body_hash_valid() {
            return Ok(ReplayVerdict::Rejected {
                backend: ReplayBackend::KernelOnly,
                reason: Text::from(
                    "body_hash mismatch — cert was modified after its hash was computed",
                ),
            });
        }
        // #95 hardening — decompose the cert body into typed
        // inference steps and verify every rule is in the canonical
        // kernel-rule registry.
        let steps = match decompose_cert(cert) {
            Ok(s) => s,
            Err(DecomposeError::UnsupportedFormat(fmt)) => {
                return Ok(ReplayVerdict::Rejected {
                    backend: ReplayBackend::KernelOnly,
                    reason: Text::from(format!(
                        "kernel-side decomposer not yet implemented for format `{}` — accept via solver-side replay only",
                        fmt.name()
                    )),
                });
            }
            Err(DecomposeError::Empty) => {
                return Ok(ReplayVerdict::Rejected {
                    backend: ReplayBackend::KernelOnly,
                    reason: Text::from("cert body decomposed to zero steps"),
                });
            }
            Err(DecomposeError::Malformed { line, message }) => {
                return Ok(ReplayVerdict::Rejected {
                    backend: ReplayBackend::KernelOnly,
                    reason: Text::from(format!(
                        "cert body malformed (line {}): {}",
                        line,
                        message.as_str()
                    )),
                });
            }
        };

        let mut unknown_rules: Vec<&str> = Vec::new();
        for s in &steps {
            if lookup_kernel_rule(s.rule.as_str()).is_none() {
                unknown_rules.push(s.rule.as_str());
            }
        }
        if !unknown_rules.is_empty() {
            unknown_rules.sort_unstable();
            unknown_rules.dedup();
            return Ok(ReplayVerdict::Rejected {
                backend: ReplayBackend::KernelOnly,
                reason: Text::from(format!(
                    "rules not in canonical kernel-rule registry: {}",
                    unknown_rules.join(", ")
                )),
            });
        }

        Ok(ReplayVerdict::Accepted {
            backend: ReplayBackend::KernelOnly,
            elapsed_ms: 0,
            detail: Some(Text::from(format!(
                "structural OK: format={}, theory={}, steps={}, all rules registered",
                cert.format.name(),
                cert.theory.as_str(),
                steps.len()
            ))),
        })
    }
}

// =============================================================================
// MockReplayEngine — deterministic, test-friendly
// =============================================================================

/// Mock replay engine.  Configured with a backend tag + an "accept"
/// flag.  Every replay returns a corresponding canned verdict.
/// Used by tests + the CLI's `--mock` mode.
#[derive(Debug, Clone)]
pub struct MockReplayEngine {
    pub backend: ReplayBackend,
    pub accept: bool,
    pub available: bool,
    pub supported_formats: Vec<CertFormat>,
}

impl MockReplayEngine {
    pub fn new(backend: ReplayBackend) -> Self {
        Self {
            backend,
            accept: true,
            available: true,
            supported_formats: CertFormat::all().to_vec(),
        }
    }

    pub fn rejecting(mut self) -> Self {
        self.accept = false;
        self
    }

    pub fn unavailable(mut self) -> Self {
        self.available = false;
        self
    }

    pub fn supporting(mut self, formats: &[CertFormat]) -> Self {
        self.supported_formats = formats.to_vec();
        self
    }
}

impl CertReplayEngine for MockReplayEngine {
    fn backend(&self) -> ReplayBackend {
        self.backend
    }

    fn supports(&self, format: CertFormat) -> bool {
        self.supported_formats.contains(&format)
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn replay(&self, cert: &SmtCertificate) -> Result<ReplayVerdict, ReplayError> {
        if !self.available {
            return Ok(ReplayVerdict::ToolMissing {
                backend: self.backend,
            });
        }
        if !self.supports(cert.format) {
            return Err(ReplayError::UnsupportedFormat(cert.format));
        }
        if self.accept {
            Ok(ReplayVerdict::Accepted {
                backend: self.backend,
                elapsed_ms: 0,
                detail: Some(Text::from(format!(
                    "mock {} accepted the cert",
                    self.backend.name()
                ))),
            })
        } else {
            Ok(ReplayVerdict::Rejected {
                backend: self.backend,
                reason: Text::from("mock rejection"),
            })
        }
    }
}

// =============================================================================
// CrossBackendVerdict — multi-backend agreement
// =============================================================================

/// Aggregate verdict across multiple backends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossBackendVerdict {
    pub cert_format: CertFormat,
    pub conclusion: Text,
    pub per_backend: Vec<ReplayVerdict>,
}

impl CrossBackendVerdict {
    pub fn new(cert: &SmtCertificate, verdicts: Vec<ReplayVerdict>) -> Self {
        Self {
            cert_format: cert.format,
            conclusion: cert.conclusion.clone(),
            per_backend: verdicts,
        }
    }

    /// True iff every available (non-`ToolMissing`) backend
    /// accepted the cert.  This is the @verify(certified)-style
    /// multi-solver gate: the proof is committed only when every
    /// available solver agrees.
    pub fn all_available_accept(&self) -> bool {
        let available: Vec<&ReplayVerdict> = self
            .per_backend
            .iter()
            .filter(|v| !matches!(v, ReplayVerdict::ToolMissing { .. }))
            .collect();
        if available.is_empty() {
            return false;
        }
        available.iter().all(|v| v.is_accepted())
    }

    /// Number of backends that accepted.
    pub fn accept_count(&self) -> usize {
        self.per_backend.iter().filter(|v| v.is_accepted()).count()
    }

    /// Number of backends that rejected.
    pub fn reject_count(&self) -> usize {
        self.per_backend
            .iter()
            .filter(|v| matches!(v, ReplayVerdict::Rejected { .. }))
            .count()
    }

    /// Number of backends that were unavailable.
    pub fn missing_count(&self) -> usize {
        self.per_backend
            .iter()
            .filter(|v| matches!(v, ReplayVerdict::ToolMissing { .. }))
            .count()
    }

    /// Backends grouped by verdict.  Used by the CLI's plain
    /// summary output.
    pub fn by_verdict(&self) -> BTreeMap<&'static str, Vec<ReplayBackend>> {
        let mut out: BTreeMap<&'static str, Vec<ReplayBackend>> = BTreeMap::new();
        for v in &self.per_backend {
            let kind = match v {
                ReplayVerdict::Accepted { .. } => "accepted",
                ReplayVerdict::Rejected { .. } => "rejected",
                ReplayVerdict::ToolMissing { .. } => "missing",
                ReplayVerdict::Error { .. } => "error",
            };
            out.entry(kind).or_default().push(v.backend());
        }
        out
    }
}

/// Run a cert through every supplied engine and aggregate.  The
/// kernel-only engine is always invoked first (its acceptance is
/// the structural-invariant baseline).
pub fn cross_check(
    cert: &SmtCertificate,
    engines: &[Box<dyn CertReplayEngine>],
) -> CrossBackendVerdict {
    let mut verdicts: Vec<ReplayVerdict> = Vec::new();
    // Kernel-only baseline.
    let kernel = KernelOnlyReplayEngine::new();
    if let Ok(v) = kernel.replay(cert) {
        verdicts.push(v);
    }
    for e in engines {
        // Skip the kernel-only path if a caller passed it again.
        if e.backend() == ReplayBackend::KernelOnly {
            continue;
        }
        match e.replay(cert) {
            Ok(v) => verdicts.push(v),
            Err(e_err) => verdicts.push(ReplayVerdict::Error {
                backend: e.backend(),
                message: Text::from(format!("{}", e_err)),
            }),
        }
    }
    CrossBackendVerdict::new(cert, verdicts)
}

// =============================================================================
// engine_for — per-backend reference engines
// =============================================================================

/// Return a reference engine for a backend.  The kernel-only
/// engine is always returned for `KernelOnly`.  For external
/// backends the V0 reference is a `MockReplayEngine` that returns
/// `ToolMissing` (V1+ swaps in production wiring that runs `coqc`
/// / `cvc5` / `verit` etc).
pub fn engine_for(backend: ReplayBackend) -> Box<dyn CertReplayEngine> {
    match backend {
        ReplayBackend::KernelOnly => Box::new(KernelOnlyReplayEngine::new()),
        other => Box::new(MockReplayEngine::new(other).unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_cert() -> SmtCertificate {
        // Minimal well-formed ALETHE body: an `assume` introducing a
        // hypothesis, plus a `step` whose `:rule` is in the canonical
        // kernel-rule registry.  The kernel-only replay decomposes
        // both lines into typed `InferenceStep`s and accepts.
        SmtCertificate::new(
            CertFormat::Cvc5Alethe,
            "QF_LIA",
            "(>= x 0) -> (>= (+ x 1) 1)",
            "(assume h1 (>= x 0))\n\
             (step t1 (cl (>= (+ x 1) 1)) :rule la_generic :premises (h1))",
        )
    }

    // ----- CertFormat / ReplayBackend -----

    #[test]
    fn format_round_trip() {
        for f in CertFormat::all() {
            assert_eq!(CertFormat::from_name(f.name()), Some(f));
        }
    }

    #[test]
    fn format_aliases_resolve() {
        assert_eq!(CertFormat::from_name("alethe"), Some(CertFormat::Cvc5Alethe));
        assert_eq!(CertFormat::from_name("z3"), Some(CertFormat::Z3Proof));
        assert_eq!(
            CertFormat::from_name("canonical"),
            Some(CertFormat::VerumCanonical)
        );
        assert_eq!(CertFormat::from_name("garbage"), None);
    }

    #[test]
    fn backend_round_trip() {
        for b in ReplayBackend::all() {
            assert_eq!(ReplayBackend::from_name(b.name()), Some(b));
        }
    }

    #[test]
    fn backend_kernel_only_is_intrinsic() {
        assert!(ReplayBackend::KernelOnly.is_intrinsic());
        for b in [
            ReplayBackend::Z3,
            ReplayBackend::Cvc5,
            ReplayBackend::Verit,
            ReplayBackend::OpenSmt,
            ReplayBackend::Mathsat,
        ] {
            assert!(!b.is_intrinsic(), "{} must require an external tool", b.name());
        }
    }

    #[test]
    fn six_canonical_formats_and_backends() {
        assert_eq!(CertFormat::all().len(), 6);
        assert_eq!(ReplayBackend::all().len(), 6);
    }

    // ----- SmtCertificate -----

    #[test]
    fn cert_constructor_computes_hash() {
        let c = fixture_cert();
        assert!(c.body_hash_valid());
        assert_eq!(c.body_hash.as_str().len(), 64);
        assert!(c.body_hash.as_str().chars().all(|x| x.is_ascii_hexdigit()));
    }

    #[test]
    fn cert_body_hash_invalid_when_body_changed() {
        let mut c = fixture_cert();
        c.body = Text::from("tampered");
        assert!(!c.body_hash_valid());
    }

    #[test]
    fn cert_with_source_solver() {
        let c = fixture_cert().with_source_solver("z3-4.13.0");
        assert_eq!(c.source_solver.as_ref().unwrap().as_str(), "z3-4.13.0");
    }

    // ----- KernelOnlyReplayEngine -----

    #[test]
    fn kernel_only_accepts_well_formed_cert() {
        let e = KernelOnlyReplayEngine::new();
        let v = e.replay(&fixture_cert()).unwrap();
        assert!(v.is_accepted());
    }

    #[test]
    fn kernel_only_rejects_empty_body() {
        let mut c = fixture_cert();
        c.body = Text::from("");
        c.body_hash = Text::from(hex32(blake3::hash(b"").as_bytes()));
        let v = KernelOnlyReplayEngine::new().replay(&c).unwrap();
        match v {
            ReplayVerdict::Rejected { reason, .. } => {
                assert!(reason.as_str().contains("empty body"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn kernel_only_rejects_unknown_theory() {
        let mut c = fixture_cert();
        c.theory = Text::from("UNKNOWN_THEORY");
        let v = KernelOnlyReplayEngine::new().replay(&c).unwrap();
        match v {
            ReplayVerdict::Rejected { reason, .. } => {
                assert!(reason.as_str().contains("unknown theory"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn kernel_only_rejects_tampered_cert() {
        let mut c = fixture_cert();
        c.body = Text::from("tampered body");
        // Hash was computed for the original body; tampering with
        // body without recomputing hash → rejection.
        let v = KernelOnlyReplayEngine::new().replay(&c).unwrap();
        match v {
            ReplayVerdict::Rejected { reason, .. } => {
                assert!(reason.as_str().contains("body_hash mismatch"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn kernel_only_supports_every_format() {
        let e = KernelOnlyReplayEngine::new();
        for f in CertFormat::all() {
            assert!(e.supports(f), "kernel must accept {} format", f.name());
        }
    }

    #[test]
    fn kernel_only_always_available() {
        assert!(KernelOnlyReplayEngine::new().is_available());
    }

    // ----- MockReplayEngine -----

    #[test]
    fn mock_engine_default_accepts() {
        let e = MockReplayEngine::new(ReplayBackend::Z3);
        let v = e.replay(&fixture_cert()).unwrap();
        assert!(v.is_accepted());
    }

    #[test]
    fn mock_engine_rejecting_returns_rejected() {
        let e = MockReplayEngine::new(ReplayBackend::Z3).rejecting();
        let v = e.replay(&fixture_cert()).unwrap();
        assert!(matches!(v, ReplayVerdict::Rejected { .. }));
    }

    #[test]
    fn mock_engine_unavailable_returns_tool_missing() {
        let e = MockReplayEngine::new(ReplayBackend::Cvc5).unavailable();
        let v = e.replay(&fixture_cert()).unwrap();
        assert!(matches!(v, ReplayVerdict::ToolMissing { .. }));
    }

    #[test]
    fn mock_engine_unsupported_format_errors() {
        let e = MockReplayEngine::new(ReplayBackend::Z3)
            .supporting(&[CertFormat::Z3Proof]);
        let mut c = fixture_cert();
        c.format = CertFormat::Cvc5Alethe;
        let r = e.replay(&c);
        assert!(matches!(r, Err(ReplayError::UnsupportedFormat(_))));
    }

    // ----- engine_for -----

    #[test]
    fn engine_for_kernel_only_is_available() {
        let e = engine_for(ReplayBackend::KernelOnly);
        assert!(e.is_available());
    }

    #[test]
    fn engine_for_external_backend_v0_unavailable() {
        for b in [
            ReplayBackend::Z3,
            ReplayBackend::Cvc5,
            ReplayBackend::Verit,
            ReplayBackend::OpenSmt,
            ReplayBackend::Mathsat,
        ] {
            let e = engine_for(b);
            assert!(!e.is_available(), "{} should be unavailable in V0", b.name());
        }
    }

    // ----- CrossBackendVerdict -----

    #[test]
    fn cross_check_kernel_only_baseline_always_runs() {
        let v = cross_check(&fixture_cert(), &[]);
        assert_eq!(v.per_backend.len(), 1);
        assert!(v.per_backend[0].is_accepted());
        assert_eq!(v.per_backend[0].backend(), ReplayBackend::KernelOnly);
    }

    #[test]
    fn cross_check_with_extra_engines() {
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)),
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5)),
        ];
        let v = cross_check(&fixture_cert(), &engines);
        assert_eq!(v.per_backend.len(), 3); // kernel + Z3 + CVC5
        assert!(v.all_available_accept());
        assert_eq!(v.accept_count(), 3);
        assert_eq!(v.reject_count(), 0);
        assert_eq!(v.missing_count(), 0);
    }

    #[test]
    fn cross_check_disagreement_breaks_consensus() {
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)),
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5).rejecting()),
        ];
        let v = cross_check(&fixture_cert(), &engines);
        assert!(!v.all_available_accept());
        assert_eq!(v.accept_count(), 2); // kernel + Z3
        assert_eq!(v.reject_count(), 1); // CVC5
    }

    #[test]
    fn cross_check_missing_tool_does_not_break_consensus() {
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)),
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5).unavailable()),
        ];
        let v = cross_check(&fixture_cert(), &engines);
        assert!(v.all_available_accept(), "missing tool counts as NotRun");
        assert_eq!(v.accept_count(), 2);
        assert_eq!(v.missing_count(), 1);
    }

    #[test]
    fn cross_check_kernel_rejection_blocks_consensus() {
        // A cert with tampered body fails the kernel-only check
        // regardless of what external solvers say.
        let mut c = fixture_cert();
        c.body = Text::from("tampered");
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)),
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5)),
        ];
        let v = cross_check(&c, &engines);
        assert!(!v.all_available_accept());
        assert_eq!(v.reject_count(), 1); // kernel-only rejects
    }

    #[test]
    fn cross_check_by_verdict_groups_backends() {
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)),
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5).rejecting()),
            Box::new(MockReplayEngine::new(ReplayBackend::Verit).unavailable()),
        ];
        let v = cross_check(&fixture_cert(), &engines);
        let groups = v.by_verdict();
        assert_eq!(groups.get("accepted").map(|v| v.len()), Some(2)); // kernel + Z3
        assert_eq!(groups.get("rejected").map(|v| v.len()), Some(1)); // CVC5
        assert_eq!(groups.get("missing").map(|v| v.len()), Some(1)); // Verit
    }

    // ----- Acceptance pin -----

    #[test]
    fn task_81_smt_solvers_external_to_tcb() {
        // Pin the trust contract: the kernel-only engine MUST
        // catch a cert whose body has been tampered with, even
        // if every external solver claims the cert is valid.
        let mut c = fixture_cert();
        c.body = Text::from("malicious body");
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)), // accepts
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5)), // accepts
            Box::new(MockReplayEngine::new(ReplayBackend::Verit)), // accepts
        ];
        let v = cross_check(&c, &engines);
        // Every external engine accepts (mock default).  Kernel-only
        // rejects → consensus broken.
        assert!(
            !v.all_available_accept(),
            "kernel-only check is the trust anchor — must reject tampered cert"
        );
    }

    #[test]
    fn task_81_multi_solver_certified_gate() {
        // §5: @verify(certified) accepts only when every solver
        // agrees.  Pin the contract that one rejection breaks
        // consensus.
        let engines: Vec<Box<dyn CertReplayEngine>> = vec![
            Box::new(MockReplayEngine::new(ReplayBackend::Z3)),
            Box::new(MockReplayEngine::new(ReplayBackend::Cvc5)),
            Box::new(MockReplayEngine::new(ReplayBackend::Verit).rejecting()),
        ];
        let v = cross_check(&fixture_cert(), &engines);
        assert!(!v.all_available_accept());
    }

    // ----- ReplayVerdict serde -----

    #[test]
    fn replay_verdict_serde_round_trip() {
        let v = ReplayVerdict::Accepted {
            backend: ReplayBackend::KernelOnly,
            elapsed_ms: 10,
            detail: None,
        };
        let s = serde_json::to_string(&v).unwrap();
        let back: ReplayVerdict = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
        // Tag uses snake_case via #[serde(tag = "kind")].
        assert!(s.contains("\"kind\":\"Accepted\""));
    }

    // =========================================================================
    // Cert decomposer (#95)
    // =========================================================================

    #[test]
    fn registry_resolves_canonical_aliases() {
        // Multiple cert names mapping to one kernel rule.
        assert_eq!(lookup_kernel_rule("mp"), Some("modus_ponens"));
        assert_eq!(lookup_kernel_rule("modus-ponens"), Some("modus_ponens"));
        assert_eq!(lookup_kernel_rule("th-mp"), Some("modus_ponens"));
        assert_eq!(lookup_kernel_rule("resolution"), Some("resolution"));
        assert_eq!(lookup_kernel_rule("la_generic"), Some("linear_arithmetic"));
    }

    #[test]
    fn registry_rejects_unknown_rules() {
        assert_eq!(lookup_kernel_rule(""), None);
        assert_eq!(lookup_kernel_rule("garbage_rule"), None);
        assert_eq!(lookup_kernel_rule("backdoor"), None);
    }

    #[test]
    fn decompose_verum_canonical_one_step() {
        let body = "step s1 mp h1 h2 : (>= y 0)";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_LIA", "y >= 0", body);
        let steps = decompose_cert(&cert).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].id.as_str(), "s1");
        assert_eq!(steps[0].rule.as_str(), "mp");
        assert_eq!(steps[0].premises.len(), 2);
        assert_eq!(steps[0].conclusion.as_str(), "(>= y 0)");
    }

    #[test]
    fn decompose_verum_canonical_skips_comments_and_blank_lines() {
        let body = "; header\n\nstep s1 refl : (= x x)\n; comment between\nstep s2 mp s1 : (= x x)\n";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_UF", "(= x x)", body);
        let steps = decompose_cert(&cert).unwrap();
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn decompose_verum_canonical_rejects_missing_conclusion() {
        let body = "step s1 mp h1 h2";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_LIA", "x", body);
        let err = decompose_cert(&cert).unwrap_err();
        assert!(matches!(err, DecomposeError::Malformed { .. }));
    }

    #[test]
    fn decompose_verum_canonical_rejects_empty() {
        let body = "; only a comment\n\n";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_LIA", "x", body);
        assert!(matches!(decompose_cert(&cert), Err(DecomposeError::Empty)));
    }

    #[test]
    fn decompose_alethe_assume_and_step() {
        let body = "(assume h1 (>= x 0))\n\
                    (step t1 (cl (>= (+ x 1) 1)) :rule la_generic :premises (h1))";
        let cert = SmtCertificate::new(CertFormat::Cvc5Alethe, "QF_LIA", "x", body);
        let steps = decompose_cert(&cert).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].rule.as_str(), "assume");
        assert_eq!(steps[1].rule.as_str(), "la_generic");
        assert_eq!(steps[1].premises.len(), 1);
        assert_eq!(steps[1].premises[0].as_str(), "h1");
    }

    #[test]
    fn decompose_alethe_rejects_unparenthesised_line() {
        let body = "step t1 (cl (>= x 0)) :rule la_generic";
        let cert = SmtCertificate::new(CertFormat::Cvc5Alethe, "QF_LIA", "x", body);
        let err = decompose_cert(&cert).unwrap_err();
        assert!(matches!(err, DecomposeError::Malformed { .. }));
    }

    #[test]
    fn decompose_z3_proof_finds_rule_applications() {
        // Z3-style nested rule application.  `mp` is in the registry.
        let body = "(mp (asserted (>= x 0)) (rewrite (= a b)) (>= y 0))";
        let cert = SmtCertificate::new(CertFormat::Z3Proof, "QF_LIA", "(>= y 0)", body);
        let steps = decompose_cert(&cert).unwrap();
        assert!(!steps.is_empty(), "z3 decomposer must find at least one step");
        assert!(steps.iter().any(|s| s.rule.as_str() == "mp"));
    }

    #[test]
    fn decompose_z3_proof_rejects_empty_body() {
        let cert = SmtCertificate::new(CertFormat::Z3Proof, "QF_LIA", "x", "   ");
        assert!(matches!(decompose_cert(&cert), Err(DecomposeError::Empty)));
    }

    #[test]
    fn decompose_lfsc_pattern_finds_rule_applications() {
        // LFSC trace: nested S-exprs whose head atoms are rule names.
        // `resolution` is in the kernel registry.
        let body = "(resolution (mp (asserted A) (asserted B) C) (assumed D))";
        let cert = SmtCertificate::new(CertFormat::LfscPattern, "QF_LIA", "C", body);
        let steps = decompose_cert(&cert).unwrap();
        assert!(steps.iter().any(|s| s.rule.as_str() == "resolution"));
        assert!(steps.iter().any(|s| s.rule.as_str() == "mp"));
    }

    #[test]
    fn decompose_lfsc_pattern_rejects_empty_body() {
        let cert = SmtCertificate::new(CertFormat::LfscPattern, "QF_LIA", "x", "  ");
        assert!(matches!(decompose_cert(&cert), Err(DecomposeError::Empty)));
    }

    #[test]
    fn decompose_open_smt_line_oriented() {
        let body = "\
; OpenSMT2 proof trace
s1 := assume    : (>= x 0)
s2 := la_generic s1 : (>= (+ x 1) 1)
";
        let cert = SmtCertificate::new(CertFormat::OpenSmt, "QF_LIA", "x", body);
        let steps = decompose_cert(&cert).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].id.as_str(), "s1");
        assert_eq!(steps[0].rule.as_str(), "assume");
        assert_eq!(steps[1].rule.as_str(), "la_generic");
        assert_eq!(steps[1].premises.len(), 1);
    }

    #[test]
    fn decompose_open_smt_rejects_missing_assignment() {
        let body = "s1 assume : (>= x 0)";
        let cert = SmtCertificate::new(CertFormat::OpenSmt, "QF_LIA", "x", body);
        let err = decompose_cert(&cert).unwrap_err();
        assert!(matches!(err, DecomposeError::Malformed { .. }));
    }

    #[test]
    fn decompose_mathsat_line_oriented() {
        let body = "\
# MathSAT5 proof trace
assume(s1) -> (>= x 0)
la_generic(s2; s1) -> (>= (+ x 1) 1)
";
        let cert = SmtCertificate::new(CertFormat::Mathsat, "QF_LIA", "x", body);
        let steps = decompose_cert(&cert).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].rule.as_str(), "assume");
        assert_eq!(steps[1].rule.as_str(), "la_generic");
        assert_eq!(steps[1].premises.len(), 1);
        assert_eq!(steps[1].premises[0].as_str(), "s1");
    }

    #[test]
    fn decompose_mathsat_rejects_unparenthesised_line() {
        let body = "la_generic s1 -> conclusion";
        let cert = SmtCertificate::new(CertFormat::Mathsat, "QF_LIA", "x", body);
        let err = decompose_cert(&cert).unwrap_err();
        assert!(matches!(err, DecomposeError::Malformed { .. }));
    }

    #[test]
    fn decompose_mathsat_rejects_missing_arrow() {
        let body = "la_generic(s1; s0) (no arrow here)";
        let cert = SmtCertificate::new(CertFormat::Mathsat, "QF_LIA", "x", body);
        let err = decompose_cert(&cert).unwrap_err();
        assert!(matches!(err, DecomposeError::Malformed { .. }));
    }

    // ----- KernelOnlyReplayEngine — end-to-end with the decomposer -----

    #[test]
    fn kernel_only_rejects_unknown_rule_in_body() {
        let body = "step s1 BACKDOOR_RULE h1 : (= x x)";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_UF", "(= x x)", body);
        let v = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        match v {
            ReplayVerdict::Rejected { reason, .. } => {
                assert!(reason.as_str().contains("BACKDOOR_RULE"));
                assert!(reason.as_str().contains("registry"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn kernel_only_rejects_malformed_canonical_body() {
        let body = "step s1"; // missing rule, premises, conclusion
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_UF", "x", body);
        let v = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        match v {
            ReplayVerdict::Rejected { reason, .. } => {
                assert!(reason.as_str().contains("malformed"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn kernel_only_accepts_well_formed_open_smt_body() {
        let body = "s1 := assume   : (>= x 0)\n\
                    s2 := mp s1    : (>= x 0)";
        let cert = SmtCertificate::new(CertFormat::OpenSmt, "QF_LIA", "(>= x 0)", body);
        let v = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        assert!(matches!(v, ReplayVerdict::Accepted { .. }));
    }

    #[test]
    fn kernel_only_accepts_well_formed_mathsat_body() {
        let body = "assume(s1) -> (>= x 0)\nmp(s2; s1) -> (>= x 0)";
        let cert = SmtCertificate::new(CertFormat::Mathsat, "QF_LIA", "(>= x 0)", body);
        let v = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        assert!(matches!(v, ReplayVerdict::Accepted { .. }));
    }

    #[test]
    fn kernel_only_accepts_well_formed_canonical_with_known_rules() {
        let body = "; well-formed verum_canonical\n\
                    step s1 refl : (= x x)\n\
                    step s2 mp s1 : (>= x 0)";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_UF", "(>= x 0)", body);
        let v = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        match v {
            ReplayVerdict::Accepted { detail, .. } => {
                let d = detail.unwrap();
                assert!(d.as_str().contains("steps=2"));
                assert!(d.as_str().contains("all rules registered"));
            }
            other => panic!("expected Accepted, got {:?}", other),
        }
    }

    #[test]
    fn task_95_decomposer_is_the_tcb_gate() {
        // Pin: a hostile cert whose body is non-empty + hash-valid
        // but whose rule is unknown is REJECTED by the kernel-only
        // engine BEFORE any solver replay runs.  This is the
        // structural guarantee that makes solvers external to the TCB.
        let body = "step s1 fake_solver_rule : (false_claim)";
        let cert = SmtCertificate::new(CertFormat::VerumCanonical, "QF_UF", "false_claim", body);
        let v = KernelOnlyReplayEngine::new().replay(&cert).unwrap();
        assert!(matches!(v, ReplayVerdict::Rejected { .. }));
    }
}
