//! Cross-target
//! proof-replay framework.
//!
//! The certificate-export pipeline (`verum_cli::commands::export`)
//! ships **statement-only** scaffolds today: every theorem is emitted
//! with its target-language Pi/forall signature and an admitted /
//! sorry / `?` proof body. Per the V2 contract requires
//! **proof-term replay** — every theorem's `SmtCertificate` lowered
//! into the target language's tactic / proof-term shape so the
//! exported file actually proves what it claims.
//!
//! This module ships the **architecture** for that lowering:
//!
//!   • [`ProofReplayBackend`] — the trait every per-target replayer
//!     implements. Given a [`SmtCertificate`] and the surrounding
//!     [`DeclarationHeader`], the backend produces a [`TargetTactic`]
//!     in its native language.
//!   • [`TargetTactic`] — common-format proof representation, opaque
//!     `String` source plus optional dependency / admitted markers.
//!   • [`ProofReplayRegistry`] — lookup table that maps target name
//!     (`"coq"` / `"lean"` / `"agda"` / `"dedukti"` / `"metamath"`)
//!     to the registered backend.
//!   • [`AdmittedReplay`] — fallback backend that produces a
//!     target-correct `Admitted` / `sorry` / `?` placeholder. This
//!     is the V1 default; it preserves the existing statement-only
//!     export contract when no real lowering is wired.
//!
//! V4.1+ work attaches actual SmtCertificate→target lowering for
//! each backend; this module provides the shape both sides commit
//! to so the integration is plug-and-play.

use std::collections::BTreeMap;

use verum_common::Text;
use verum_kernel::SmtCertificate;

/// error surface for proof-replay failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// The target backend isn't registered or doesn't recognise the
    /// certificate's source backend (`SmtCertificate.backend`).
    UnsupportedBackend {
        target: Text,
        cert_backend: Text,
    },
    /// The certificate's envelope schema is too new for this
    /// backend's lowering rules.
    UnsupportedSchema {
        target: Text,
        found: u32,
        max_supported: u32,
    },
    /// The backend's proof-step lowering hit a trace shape it doesn't
    /// know how to handle. Carries a free-form diagnostic so future
    /// rules can extend coverage incrementally.
    UnsupportedTrace { target: Text, reason: Text },
    /// Free-form fallback for backend-specific failures (parse error
    /// in the trace, missing hypothesis, etc.).
    Custom(Text),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedBackend { target, cert_backend } => write!(
                f,
                "proof-replay: target `{}` does not know how to lower certificates from backend `{}`",
                target, cert_backend
            ),
            Self::UnsupportedSchema { target, found, max_supported } => write!(
                f,
                "proof-replay: target `{}` supports envelope schema ≤ {}, certificate is at {}",
                target, max_supported, found
            ),
            Self::UnsupportedTrace { target, reason } => write!(
                f,
                "proof-replay: target `{}` cannot lower trace ({})",
                target, reason
            ),
            Self::Custom(msg) => write!(f, "proof-replay: {}", msg),
        }
    }
}

impl std::error::Error for ReplayError {}

/// the kind of declaration being replayed.
/// Backends use this to pick the right header keyword
/// (`Theorem` / `Lemma` / `Axiom` / `Corollary`) where their language
/// distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Axiom,
    Theorem,
    Lemma,
    Corollary,
}

impl DeclKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Axiom => "axiom",
            Self::Theorem => "theorem",
            Self::Lemma => "lemma",
            Self::Corollary => "corollary",
        }
    }

    /// Map a verum-AST keyword string back to a [`DeclKind`].
    /// Returns `None` for unrecognised inputs so callers can fall
    /// back to a sensible default.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "axiom" => Some(Self::Axiom),
            "theorem" => Some(Self::Theorem),
            "lemma" => Some(Self::Lemma),
            "corollary" => Some(Self::Corollary),
            _ => None,
        }
    }
}

/// minimal context the replay backend needs.
///
/// Carries the declaration's name, kind, and optional framework
/// attribution. Intentionally minimal so the export pipeline can
/// construct it from the AST without dragging additional state.
#[derive(Debug, Clone)]
pub struct DeclarationHeader {
    pub name: Text,
    pub kind: DeclKind,
    /// Framework lineage, if the source declaration carried
    /// `@framework(name, "citation")`.
    pub framework: Option<FrameworkRef>,
}

/// framework attribution carried into the
/// replay context. Mirrors `verum_kernel::FrameworkId`'s shape so
/// callers can pass either through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkRef {
    pub name: Text,
    pub citation: Text,
}

/// the lowered proof in the target language.
///
/// `source` is opaque per-target text; the export emitter splices it
/// into the target file verbatim. `depends_on` lists axiom / lemma
/// names the proof cites so the emitter can ensure they're imported
/// or declared earlier in the file. `admitted = true` marks proofs
/// where the backend gracefully fell back to a placeholder (the
/// emitter then reports "N admitted of M" in the export summary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTactic {
    /// Target language identifier (`"coq"`, `"lean"`, `"agda"`,
    /// `"dedukti"`, `"metamath"`).
    pub language: Text,
    /// Target-language source text — splice point in the emitter.
    pub source: String,
    /// Names of axioms / lemmas the proof depends on. Used by the
    /// emitter to verify import order or detect missing dependencies.
    pub depends_on: Vec<Text>,
    /// `true` when the backend produced an Admitted / sorry / `?`
    /// placeholder rather than a fully-replayed proof.
    pub admitted: bool,
}

impl TargetTactic {
    pub fn new(language: Text, source: String) -> Self {
        Self {
            language,
            source,
            depends_on: Vec::new(),
            admitted: false,
        }
    }

    pub fn admitted(language: Text, source: String) -> Self {
        Self {
            language,
            source,
            depends_on: Vec::new(),
            admitted: true,
        }
    }

    pub fn with_dependencies(mut self, deps: Vec<Text>) -> Self {
        self.depends_on = deps;
        self
    }
}

/// the trait every target backend implements.
///
/// Implementors live in a per-target module. The framework wires
/// them into the [`ProofReplayRegistry`] at startup (or lazily on
/// first lookup).
pub trait ProofReplayBackend: Send + Sync {
    /// Target language identifier — used as the registry key. Stable.
    fn target_name(&self) -> &'static str;

    /// Lower `cert` (a backend-neutral `SmtCertificate`) into a
    /// target-language proof. The default contract: on inability to
    /// lower the trace fully, return a `TargetTactic::admitted` so
    /// the export still produces a syntactically valid file.
    fn lower(
        &self,
        cert: &SmtCertificate,
        decl: &DeclarationHeader,
    ) -> Result<TargetTactic, ReplayError>;

    /// Canonical foreign-system handle.  Default implementation
    /// resolves [`target_name`](Self::target_name) via
    /// [`ForeignSystem::from_name`]; override when the backend's
    /// name doesn't match the canonical alias set.  Lets consumers
    /// dispatch by typed enum rather than string comparison.
    fn foreign_system(&self) -> Option<verum_kernel::foreign_system::ForeignSystem> {
        verum_kernel::foreign_system::ForeignSystem::from_name(self.target_name())
    }
}

/// per-target lookup. The registry is keyed
/// by `target_name`; lookup returns a borrowed dyn-trait reference.
#[derive(Default)]
pub struct ProofReplayRegistry {
    backends: BTreeMap<&'static str, Box<dyn ProofReplayBackend>>,
}

impl ProofReplayRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a backend. Backends keyed by `target_name`; later
    /// registrations under the same key replace the earlier entry
    /// (allows test scaffolding without lifecycle ceremony).
    pub fn register(&mut self, backend: Box<dyn ProofReplayBackend>) {
        let key = backend.target_name();
        self.backends.insert(key, backend);
    }

    /// Look up a backend by target name. Returns `None` when no
    /// backend is registered for the target — callers typically
    /// fall back to the [`AdmittedReplay`] sentinel.
    pub fn get(&self, target: &str) -> Option<&dyn ProofReplayBackend> {
        self.backends.get(target).map(|b| b.as_ref())
    }

    /// Enumerate every registered target name. Used by `verum
    /// export --list-replay-backends` (V4.1 CLI surface).
    pub fn target_names(&self) -> Vec<&'static str> {
        self.backends.keys().copied().collect()
    }
}

/// fallback backend. Produces a
/// target-language Admitted / sorry / `?` placeholder for every
/// supported target. Always succeeds, always sets `admitted = true`.
///
/// This is the contract that lets V4 ship today without breaking
/// statement-only exports: when no per-target replayer is
/// registered, [`AdmittedReplay`] is the default and the existing
/// V1 emit shape is preserved exactly.
pub struct AdmittedReplay {
    target: &'static str,
}

impl AdmittedReplay {
    pub const COQ: AdmittedReplay = AdmittedReplay { target: "coq" };
    pub const LEAN: AdmittedReplay = AdmittedReplay { target: "lean" };
    pub const AGDA: AdmittedReplay = AdmittedReplay { target: "agda" };
    pub const DEDUKTI: AdmittedReplay = AdmittedReplay { target: "dedukti" };
    pub const METAMATH: AdmittedReplay = AdmittedReplay { target: "metamath" };

    pub fn new(target: &'static str) -> Self {
        Self { target }
    }
}

impl ProofReplayBackend for AdmittedReplay {
    fn target_name(&self) -> &'static str {
        self.target
    }

    fn lower(
        &self,
        _cert: &SmtCertificate,
        _decl: &DeclarationHeader,
    ) -> Result<TargetTactic, ReplayError> {
        let body = match self.target {
            "coq" => "Proof. Admitted.".to_string(),
            "lean" => "sorry".to_string(),
            "agda" => "{!!}".to_string(), // Agda's hole syntax
            "dedukti" => "(; admitted ;)".to_string(),
            "metamath" => "$= ? $.".to_string(),
            other => format!("(* {} admitted *)", other),
        };
        Ok(TargetTactic::admitted(Text::from(self.target), body))
    }
}

/// Coq backend. Lowers SmtCertificate
/// traces into Coq tactic chains; recognises Z3 `(proof ...)` and
/// CVC5 ALETHE step shapes. See [`coq::CoqProofReplay`].
pub mod coq;
pub use coq::CoqProofReplay;

/// Lean 4 backend. Tactic-block style
/// (`by ...`); same Z3+CVC5 dispatch as Coq.
pub mod lean;
pub use lean::LeanProofReplay;

/// Agda backend. Term-style proofs
/// (refl / cong / sym / trans / λ-binding).
pub mod agda;
pub use agda::AgdaProofReplay;

/// Dedukti backend. λΠ-modulo
/// rewrite-rule style.
pub mod dedukti;
pub use dedukti::DeduktiProofReplay;

/// Metamath backend. `$= ... $.`
/// proof-step language with named axioms from set.mm.
pub mod metamath;
pub use metamath::MetamathProofReplay;

/// convenience constructor that
/// pre-registers every shipping target backend with its concrete
/// proof-replay implementation. All five targets now have real
/// trace-aware lowering; [`AdmittedReplay`] remains available as
/// a fallback for callers that explicitly want the V1 statement-
/// only contract.
pub fn default_registry() -> ProofReplayRegistry {
    let mut r = ProofReplayRegistry::new();
    r.register(Box::new(CoqProofReplay::new()));
    r.register(Box::new(LeanProofReplay::new()));
    r.register(Box::new(AgdaProofReplay::new()));
    r.register(Box::new(DeduktiProofReplay::new()));
    r.register(Box::new(MetamathProofReplay::new()));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::List;

    fn dummy_cert() -> SmtCertificate {
        SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.12.0"),
            List::new(),
            Text::from("blake3:abcdef"),
        )
    }

    fn dummy_decl(name: &str, kind: DeclKind) -> DeclarationHeader {
        DeclarationHeader {
            name: Text::from(name),
            kind,
            framework: None,
        }
    }

    #[test]
    fn admitted_replay_coq_emits_admitted_keyword() {
        let backend = AdmittedReplay::new("coq");
        let tactic = backend
            .lower(&dummy_cert(), &dummy_decl("plus_comm", DeclKind::Theorem))
            .expect("admitted replay always succeeds");
        assert!(tactic.admitted);
        assert_eq!(tactic.language.as_str(), "coq");
        assert!(tactic.source.contains("Admitted"));
    }

    #[test]
    fn admitted_replay_lean_emits_sorry() {
        let backend = AdmittedReplay::new("lean");
        let tactic = backend
            .lower(&dummy_cert(), &dummy_decl("foo", DeclKind::Lemma))
            .expect("ok");
        assert_eq!(tactic.source, "sorry");
        assert!(tactic.admitted);
    }

    #[test]
    fn admitted_replay_agda_emits_hole() {
        let t = AdmittedReplay::new("agda")
            .lower(&dummy_cert(), &dummy_decl("g", DeclKind::Theorem))
            .unwrap();
        assert_eq!(t.source, "{!!}");
    }

    #[test]
    fn admitted_replay_dedukti_emits_comment_marker() {
        let t = AdmittedReplay::new("dedukti")
            .lower(&dummy_cert(), &dummy_decl("h", DeclKind::Axiom))
            .unwrap();
        assert!(t.source.contains("admitted"));
    }

    #[test]
    fn admitted_replay_metamath_emits_question_mark() {
        let t = AdmittedReplay::new("metamath")
            .lower(&dummy_cert(), &dummy_decl("ax", DeclKind::Axiom))
            .unwrap();
        assert!(t.source.contains("?"));
    }

    #[test]
    fn registry_round_trip() {
        let mut r = ProofReplayRegistry::new();
        r.register(Box::new(AdmittedReplay::new("coq")));
        let backend = r.get("coq").expect("coq must be registered");
        let t = backend
            .lower(&dummy_cert(), &dummy_decl("t", DeclKind::Theorem))
            .unwrap();
        assert_eq!(t.language.as_str(), "coq");
    }

    #[test]
    fn registry_returns_none_for_unknown_target() {
        let r = ProofReplayRegistry::new();
        assert!(r.get("haskell").is_none());
    }

    #[test]
    fn default_registry_has_all_five_shipping_targets() {
        let r = default_registry();
        let names: Vec<&str> = r.target_names();
        assert!(names.contains(&"coq"));
        assert!(names.contains(&"lean"));
        assert!(names.contains(&"agda"));
        assert!(names.contains(&"dedukti"));
        assert!(names.contains(&"metamath"));
    }

    #[test]
    fn decl_kind_from_str_maps_known_keywords() {
        assert_eq!(DeclKind::from_str("axiom"), Some(DeclKind::Axiom));
        assert_eq!(DeclKind::from_str("theorem"), Some(DeclKind::Theorem));
        assert_eq!(DeclKind::from_str("lemma"), Some(DeclKind::Lemma));
        assert_eq!(DeclKind::from_str("corollary"), Some(DeclKind::Corollary));
        assert_eq!(DeclKind::from_str("foo"), None);
    }

    #[test]
    fn target_tactic_with_dependencies_carries_deps() {
        let t = TargetTactic::new(Text::from("coq"), "exact H.".to_string())
            .with_dependencies(vec![Text::from("plus_assoc"), Text::from("zero_l")]);
        assert_eq!(t.depends_on.len(), 2);
        assert!(!t.admitted);
    }

    #[test]
    fn replay_error_display_messages_are_distinct() {
        let e1 = ReplayError::UnsupportedBackend {
            target: Text::from("coq"),
            cert_backend: Text::from("vampire"),
        };
        let e2 = ReplayError::UnsupportedSchema {
            target: Text::from("lean"),
            found: 99,
            max_supported: 1,
        };
        let e3 = ReplayError::UnsupportedTrace {
            target: Text::from("agda"),
            reason: Text::from("unknown rule"),
        };
        let e4 = ReplayError::Custom(Text::from("parse failure"));
        assert_ne!(format!("{}", e1), format!("{}", e2));
        assert_ne!(format!("{}", e2), format!("{}", e3));
        assert_ne!(format!("{}", e3), format!("{}", e4));
    }
}
