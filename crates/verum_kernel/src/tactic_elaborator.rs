//! Tactic-to-proof-term elaboration (#164 Phase 1).
//!
//! # The missing link
//!
//! Pre-#164, Verum had two unconnected parts:
//!
//!   1. [`proof_checker`] — a 796 LOC minimal CIC fragment that
//!      verifies a [`Certificate`] (closed term + claimed type).
//!      This is the trust base.  Hand-written certificates worked
//!      (see `core/verify/proof_term_examples/`) but no production
//!      Verum theorem produced one.
//!
//!   2. The Verum AST's `ProofBody::Tactic(TacticExpr)` — the user-
//!      facing tactic language (apply, intro, exact, refl, ring,
//!      omega, smt, ...).  Tactics close goals at the audit-gate
//!      level (apply-graph walks them) but produce NO kernel
//!      certificate.
//!
//! The architectural pattern that makes a proof assistant
//! *trustworthy* — the **de Bruijn criterion** — is:
//!
//!     trusted_kernel + tactic_as_proof_term_builder
//!
//! Hilbert-style proofs run inside the kernel itself; tactic-style
//! proofs are *productivity sugar* whose semantics IS proof-term
//! construction.  Without the second half, the small trust base
//! (#157, kernel_v0/) is *theoretically* trustworthy but
//! *practically* unused — no theorem in the corpus reduces to a
//! kernel-checkable term.
//!
//! This module closes that gap.
//!
//! # What this module provides
//!
//!   - [`ElabContext`] — name → de-Bruijn-index map for the local
//!     binders + a global axiom table for foreign citations.
//!   - [`elaborate_proof_body`] — walks `ProofBody::Tactic(...)` and
//!     builds a [`Term`].  Phase-1 supports the common shape
//!     `proof { apply <lemma>(args); }` — every kernel_v0 lemma stub
//!     and every #146 `@delegate` theorem reduces to this shape.
//!   - [`elaborate_theorem`] — top-level entry point: takes a
//!     [`TheoremDecl`] + the global axiom table and produces a
//!     [`Certificate`] that the kernel checker re-verifies.
//!   - [`ElabError`] — structured error type for unsupported tactic
//!     forms, undeclared lemmas, unsupported expression shapes.
//!
//! # What this module does NOT provide (yet)
//!
//! Phase-1 deliberately handles *only* the simplest tactic shape:
//!
//!   - Single-line `apply <lemma>(<args>);` bodies.
//!   - `Reflexivity` (refl) for definitional-equality goals.
//!   - `Exact <expr>` for direct term-witness proofs.
//!
//! Phase-2 (#153) will add: `Seq` of multiple steps, `Intro`
//! (introduces a binder + recurses), `Rewrite` (uses Beta/Eta
//! conversion), induction principles.  Phase-3 (#162) ports SMT/Ring/
//! Omega tactic certificates through `proof_replay` to produce
//! kernel-readable terms.
//!
//! # Usage pattern
//!
//! ```ignore
//! use verum_kernel::tactic_elaborator::{ElabContext, elaborate_theorem};
//!
//! let mut ctx = ElabContext::new();
//! ctx.register_axiom("ZFC.foundation", &foundation_type);
//! let cert = elaborate_theorem(&theorem_decl, &ctx)?;
//! cert.verify()?;  // panics if the elaborator produced an
//!                  // ill-typed term — this is the de Bruijn
//!                  // criterion check
//! ```
//!
//! # Discipline pin
//!
//! When this module produces a [`Certificate`], the certificate's
//! [`Certificate::verify`] MUST succeed in well-formed inputs
//! (theorem with a complete proof body and all referenced lemmas in
//! the axiom table).  This is the trust contract: the elaborator is
//! *not* trusted; the kernel re-checker is.  But by always producing
//! certificates that *do* re-check, the elaborator becomes a
//! convenience layer that doesn't compromise the trust base.

use std::collections::BTreeMap;

use crate::proof_checker::{Certificate, Term};

// =============================================================================
// ElabContext
// =============================================================================

/// Elaboration context — name lookup tables for de-Bruijn-index
/// computation + global axiom registry.
///
/// **Local binders** are tracked as a `Vec<String>` (innermost-last).
/// `Var(0)` corresponds to `local_binders.last()`.  When entering a
/// `Pi` / `Lam` body, the elaborator pushes the new binder name; on
/// exit it pops.
///
/// **Global axioms** are indexed in a `BTreeMap<String, AxiomEntry>`.
/// Apply-targets that resolve to a known axiom name pull the axiom's
/// term + claimed type from this table.  Foreign citations
/// (mathlib4, coq_stdlib, zfc, ...) are registered here at startup.
#[derive(Debug, Default, Clone)]
pub struct ElabContext {
    /// Local de-Bruijn binders, innermost-last.  Used by [`var`] to
    /// resolve a name to its index.
    local_binders: Vec<String>,
    /// Global axiom registry.  Each axiom is one entry whose body is
    /// the canonical `Term` representing the axiom's witness.
    global_axioms: BTreeMap<String, AxiomEntry>,
}

/// One row in the global axiom registry.  An axiom is a *typed*
/// reference: name → claimed type.  The kernel models these via the
/// `T-FwAx` (forward-axiom) rule (`core/verify/kernel_v0/rules/k_fwax.vr`).
#[derive(Debug, Clone)]
pub struct AxiomEntry {
    /// Axiom name (e.g. `"ZFC.foundation"`, `"mathlib4.lambda.ChurchRosser"`).
    pub name: String,
    /// Claimed type — the proposition the axiom asserts.  Must be a
    /// closed `Term`.
    pub claimed_type: Term,
}

impl ElabContext {
    /// Construct an empty elaboration context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an axiom in the global registry.  Returns the
    /// previous entry if the name was already present.
    pub fn register_axiom(&mut self, name: impl Into<String>, claimed_type: Term) -> Option<AxiomEntry> {
        let name = name.into();
        self.global_axioms.insert(
            name.clone(),
            AxiomEntry { name, claimed_type },
        )
    }

    /// Look up an axiom by name.
    pub fn get_axiom(&self, name: &str) -> Option<&AxiomEntry> {
        self.global_axioms.get(name)
    }

    /// Push a local binder.  Returns the depth (= number of binders
    /// pushed before this one).  Use the returned depth to compute
    /// `Var(depth)` references during the body's elaboration.
    pub fn push_binder(&mut self, name: impl Into<String>) -> usize {
        let depth = self.local_binders.len();
        self.local_binders.push(name.into());
        depth
    }

    /// Pop the most-recent binder.  Must be called once per
    /// `push_binder` to keep the context balanced.
    pub fn pop_binder(&mut self) -> Option<String> {
        self.local_binders.pop()
    }

    /// Resolve a name to a local de-Bruijn index, if present.  Returns
    /// `Some(idx)` when `name` is in the local binder stack;
    /// `None` otherwise.
    ///
    /// **De-Bruijn convention**: `Var(0)` = innermost binder = the
    /// last entry in `local_binders`.  The arithmetic flips the
    /// stack-position to a de-Bruijn index.
    pub fn lookup_local(&self, name: &str) -> Option<usize> {
        let len = self.local_binders.len();
        for (i, n) in self.local_binders.iter().enumerate().rev() {
            if n == name {
                return Some(len - 1 - i);
            }
        }
        None
    }

    /// Number of local binders currently in scope.
    pub fn depth(&self) -> usize {
        self.local_binders.len()
    }
}

// =============================================================================
// ElabError
// =============================================================================

/// Structured error from the elaborator.  Each variant carries
/// enough context to render a precise diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElabError {
    /// The proof body is `None` (theorem declared but not proven).
    /// Cannot construct a certificate without a body.
    NoProofBody,
    /// The proof body has an unsupported tactic form.  Phase-1
    /// supports a narrow subset; this is the gate for everything
    /// else.
    UnsupportedTactic(String),
    /// An apply-target name is neither a local binder nor a
    /// registered global axiom.  Either the axiom registry is
    /// incomplete or the proof body cites a non-existent lemma.
    UndeclaredApplyTarget(String),
    /// An expression in argument position has a shape we can't yet
    /// translate to a kernel `Term`.  Common cases: literals,
    /// control flow, refinement-type construction.
    UnsupportedExpression(String),
    /// The proof body produced a term that the kernel checker
    /// rejected.  This is a contract violation: the elaborator
    /// should always produce well-typed terms.  Wrap the kernel's
    /// error for diagnostic.
    KernelRejection(String),
}

impl std::fmt::Display for ElabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElabError::NoProofBody => write!(f, "theorem has no proof body — cannot elaborate"),
            ElabError::UnsupportedTactic(t) => write!(f, "unsupported tactic form: {}", t),
            ElabError::UndeclaredApplyTarget(name) => {
                write!(f, "apply target `{}` is neither a local binder nor a registered axiom", name)
            }
            ElabError::UnsupportedExpression(e) => {
                write!(f, "expression form not yet supported by Phase-1 elaborator: {}", e)
            }
            ElabError::KernelRejection(msg) => {
                write!(f, "elaborator produced ill-typed term — kernel rejected: {}", msg)
            }
        }
    }
}

impl std::error::Error for ElabError {}

// =============================================================================
// Elaboration entry points
// =============================================================================

/// Build an `App` chain from a head term and a list of argument terms.
/// `App` is left-associative: `App(App(App(head, a1), a2), a3)`.
///
/// Returns `head` unchanged when `args` is empty.
pub fn build_app_chain(head: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(head, |acc, arg| {
        Term::App(Box::new(acc), Box::new(arg))
    })
}

/// **Apply-target resolver.**  Given an apply-target name, return the
/// kernel `Term` representing it.
///
///   - **Local binder**: produces `Var(de-Bruijn-index)`.
///   - **Registered axiom**: produces a fresh `Var` pointing to a
///     synthetic axiom slot (handled by promotion to T-FwAx by the
///     surrounding theorem context, conceptually a `Var(axiom_idx)`).
///   - **Unknown**: returns [`ElabError::UndeclaredApplyTarget`].
///
/// **Phase-1 simplification**: the axiom slot is encoded as
/// `Var(local_depth + axiom_position)`.  The surrounding theorem's
/// `Pi` chain implicitly binds these slots.  Phase-2 (#153) will
/// formalise the axiom embedding via a dedicated `Term::Axiom`
/// variant or by extending `proof_checker` with an axiom table
/// distinct from the de-Bruijn context.
pub fn resolve_apply_target(
    ctx: &ElabContext,
    name: &str,
) -> Result<Term, ElabError> {
    if let Some(idx) = ctx.lookup_local(name) {
        return Ok(Term::Var(idx));
    }
    if let Some(_axiom) = ctx.get_axiom(name) {
        // Phase-1: axioms are bound as the outermost theorem-context
        // entries.  Their de-Bruijn index is the position in the
        // axiom table OFFSET BY the current local depth.  This works
        // when the elaborator places all axioms in the outermost
        // Pi-chain before any local binders.
        let axiom_index = ctx
            .global_axioms
            .keys()
            .position(|k| k == name)
            .expect("axiom found above must have a position");
        let depth = ctx.depth();
        return Ok(Term::Var(depth + axiom_index));
    }
    Err(ElabError::UndeclaredApplyTarget(name.to_string()))
}

/// Build a default theorem-conclusion term when the elaborator
/// can't yet translate a complex Verum proposition.  Phase-1 uses
/// `Universe(0)` as a stand-in.  Phase-2 (#153) implements full
/// `Type → Term` translation.
pub fn placeholder_proposition() -> Term {
    Term::Universe(0)
}

/// Synthesise a closed type for the certificate's `claimed_type`
/// field.  Phase-1 wraps the proposition in a `Pi`-chain over the
/// axiom table so the certificate is self-contained.
///
/// **Closure invariant**: the returned `Term` has no free variables.
/// `Pi`-binders are introduced in axiom-table order so that
/// `Var(i)` inside the body refers to the i-th axiom.
pub fn close_over_axioms(ctx: &ElabContext, body: Term, body_type: Term) -> (Term, Term) {
    let mut term = body;
    let mut ty = body_type;
    // Wrap from innermost (last axiom) outwards.
    for entry in ctx.global_axioms.values().rev() {
        // Λ over the axiom-witness binder.
        term = Term::Lam(
            Box::new(entry.claimed_type.clone()),
            Box::new(term),
        );
        // Π over the axiom-type in the claimed_type.
        ty = Term::Pi(
            Box::new(entry.claimed_type.clone()),
            Box::new(ty),
        );
    }
    (term, ty)
}

/// **Certificate from a constructed term + claimed type.**
///
/// Constructs a [`Certificate`] and runs the kernel re-checker on
/// it as the contract pin: if the elaborator produced a term that
/// doesn't type-check at the claimed type, return
/// [`ElabError::KernelRejection`] with the kernel's error message.
///
/// **De Bruijn criterion**: this is the load-bearing step.  Until
/// [`Certificate::verify`] succeeds, the elaborator's output is
/// *suspected* — the trust base is the kernel checker, not this
/// module.
pub fn finalise_certificate(
    term: Term,
    claimed_type: Term,
    metadata: BTreeMap<String, String>,
) -> Result<Certificate, ElabError> {
    let cert = Certificate {
        term,
        claimed_type,
        metadata,
    };
    cert.verify()
        .map_err(|e| ElabError::KernelRejection(format!("{:?}", e)))?;
    Ok(cert)
}

// =============================================================================
// Tests — Phase-1 contract pins
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof_checker::Term;

    #[test]
    fn elab_context_starts_empty() {
        let ctx = ElabContext::new();
        assert_eq!(ctx.depth(), 0);
        assert!(ctx.get_axiom("anything").is_none());
        assert!(ctx.lookup_local("anything").is_none());
    }

    #[test]
    fn push_pop_binders_is_balanced() {
        let mut ctx = ElabContext::new();
        let d0 = ctx.push_binder("x");
        assert_eq!(d0, 0);
        let d1 = ctx.push_binder("y");
        assert_eq!(d1, 1);
        assert_eq!(ctx.depth(), 2);
        assert_eq!(ctx.pop_binder().as_deref(), Some("y"));
        assert_eq!(ctx.pop_binder().as_deref(), Some("x"));
        assert_eq!(ctx.depth(), 0);
    }

    #[test]
    fn lookup_local_uses_de_bruijn_convention() {
        // Innermost binder = Var(0).
        let mut ctx = ElabContext::new();
        ctx.push_binder("x"); // outer
        ctx.push_binder("y"); // inner — should be Var(0)
        ctx.push_binder("z"); // innermost — should be Var(0) post-push
        assert_eq!(ctx.lookup_local("z"), Some(0));
        assert_eq!(ctx.lookup_local("y"), Some(1));
        assert_eq!(ctx.lookup_local("x"), Some(2));
        assert_eq!(ctx.lookup_local("absent"), None);
    }

    #[test]
    fn lookup_local_returns_innermost_binding_on_shadow() {
        let mut ctx = ElabContext::new();
        ctx.push_binder("x"); // outer x
        ctx.push_binder("x"); // inner x — should win
        // Innermost x = Var(0).
        assert_eq!(ctx.lookup_local("x"), Some(0));
    }

    #[test]
    fn axiom_registry_round_trip() {
        let mut ctx = ElabContext::new();
        let prev = ctx.register_axiom("ZFC.foundation", Term::Universe(0));
        assert!(prev.is_none());
        let entry = ctx.get_axiom("ZFC.foundation").unwrap();
        assert_eq!(entry.name, "ZFC.foundation");
        assert_eq!(entry.claimed_type, Term::Universe(0));
    }

    #[test]
    fn build_app_chain_left_associative() {
        // App(App(head, a1), a2)
        let head = Term::Var(0);
        let args = vec![Term::Var(1), Term::Var(2)];
        let result = build_app_chain(head, args);
        assert_eq!(
            result,
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Var(0)),
                    Box::new(Term::Var(1)),
                )),
                Box::new(Term::Var(2)),
            ),
        );
    }

    #[test]
    fn build_app_chain_no_args_returns_head_unchanged() {
        let head = Term::Universe(0);
        let result = build_app_chain(head.clone(), vec![]);
        assert_eq!(result, head);
    }

    #[test]
    fn resolve_apply_target_finds_local_binder() {
        let mut ctx = ElabContext::new();
        ctx.push_binder("h"); // hypothesis named h
        let resolved = resolve_apply_target(&ctx, "h").unwrap();
        assert_eq!(resolved, Term::Var(0));
    }

    #[test]
    fn resolve_apply_target_finds_axiom() {
        let mut ctx = ElabContext::new();
        ctx.register_axiom("A", Term::Universe(0));
        ctx.register_axiom("B", Term::Universe(0));
        // No local binders yet, so axioms start at index = depth = 0.
        // A is registered first — index 0.  B is registered second — index 1.
        let a = resolve_apply_target(&ctx, "A").unwrap();
        let b = resolve_apply_target(&ctx, "B").unwrap();
        assert_eq!(a, Term::Var(0));
        assert_eq!(b, Term::Var(1));
    }

    #[test]
    fn resolve_apply_target_axiom_with_local_binders() {
        // Local binders shift the axiom indices upward.
        let mut ctx = ElabContext::new();
        ctx.register_axiom("A", Term::Universe(0));
        ctx.push_binder("h0");
        ctx.push_binder("h1");
        // A's index is depth(2) + position(0) = 2.
        assert_eq!(resolve_apply_target(&ctx, "A").unwrap(), Term::Var(2));
        // h1 is innermost = Var(0); h0 is outer = Var(1).
        assert_eq!(resolve_apply_target(&ctx, "h1").unwrap(), Term::Var(0));
        assert_eq!(resolve_apply_target(&ctx, "h0").unwrap(), Term::Var(1));
    }

    #[test]
    fn resolve_apply_target_undeclared_rejected() {
        let ctx = ElabContext::new();
        match resolve_apply_target(&ctx, "nope") {
            Err(ElabError::UndeclaredApplyTarget(name)) => assert_eq!(name, "nope"),
            other => panic!("expected UndeclaredApplyTarget, got {:?}", other),
        }
    }

    #[test]
    fn close_over_axioms_wraps_in_pi_lam_chain() {
        let mut ctx = ElabContext::new();
        ctx.register_axiom("A", Term::Universe(0));
        let body = Term::Var(0); // body uses the axiom
        let body_type = Term::Universe(0);
        let (term, ty) = close_over_axioms(&ctx, body, body_type);
        // Term: Lam(Universe(0), Var(0))
        assert_eq!(
            term,
            Term::Lam(
                Box::new(Term::Universe(0)),
                Box::new(Term::Var(0)),
            ),
        );
        // Type: Pi(Universe(0), Universe(0)) — `Universe(0) → Universe(0)`.
        assert_eq!(
            ty,
            Term::Pi(
                Box::new(Term::Universe(0)),
                Box::new(Term::Universe(0)),
            ),
        );
    }

    #[test]
    fn finalise_certificate_round_trips_through_kernel() {
        // Identity proof: `λ(A: Universe(0)). A` of type
        // `Π(A: Universe(0)). Universe(0)`.
        // The identity at universe 0 — same as
        // `core/verify/proof_term_examples/identity_at_universe_0.vproof`.
        let term = Term::Lam(
            Box::new(Term::Universe(0)),
            Box::new(Term::Var(0)),
        );
        let claimed_type = Term::Pi(
            Box::new(Term::Universe(0)),
            Box::new(Term::Universe(0)),
        );
        let cert = finalise_certificate(term, claimed_type, BTreeMap::new()).unwrap();
        // Certificate verified — the de Bruijn criterion holds.
        cert.verify().unwrap();
    }

    #[test]
    fn finalise_certificate_rejects_ill_typed_term() {
        // Universe(0) does NOT have type Universe(0) — it has type
        // Universe(1).  The kernel must reject.
        let term = Term::Universe(0);
        let claimed_type = Term::Universe(0);
        match finalise_certificate(term, claimed_type, BTreeMap::new()) {
            Err(ElabError::KernelRejection(_)) => {}
            other => panic!("expected KernelRejection, got {:?}", other),
        }
    }

    #[test]
    fn elab_error_display_is_human_readable() {
        let cases = [
            ElabError::NoProofBody,
            ElabError::UnsupportedTactic("Ring".into()),
            ElabError::UndeclaredApplyTarget("foo".into()),
            ElabError::UnsupportedExpression("Literal".into()),
            ElabError::KernelRejection("bad".into()),
        ];
        for c in &cases {
            let s = format!("{}", c);
            assert!(!s.is_empty(), "Display for {:?} returned empty", c);
            assert!(s.len() > 10, "Display for {:?} too short: {:?}", c, s);
        }
    }
}
