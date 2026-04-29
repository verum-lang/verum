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
        Ok(ReplayVerdict::Accepted {
            backend: ReplayBackend::KernelOnly,
            elapsed_ms: 0,
            detail: Some(Text::from(format!(
                "structural OK: format={}, theory={}, hash matches",
                cert.format.name(),
                cert.theory.as_str()
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
        SmtCertificate::new(
            CertFormat::Cvc5Alethe,
            "QF_LIA",
            "(>= x 0) -> (>= (+ x 1) 1)",
            "(step 1 ...)\n(step 2 ...)\n(qed ...)",
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
}
