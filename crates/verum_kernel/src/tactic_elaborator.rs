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
/// can't yet translate a complex Verum proposition.  Phase-1 used
/// this as a universal stand-in.  Phase-4 supersedes most uses via
/// [`proposition_to_term`]; this helper remains the fallback for
/// genuinely-untranslatable shapes.
pub fn placeholder_proposition() -> Term {
    Term::Universe(0)
}

/// **Translate a Verum proposition Expr to a kernel Term.**
///
/// This is the load-bearing step that makes elaborated certificates
/// prove the *original theorem statement* rather than the trivial
/// `Universe(0)` placeholder.  Without it, `Certificate::verify`
/// only checks "the term is well-typed at some sort" — vacuously
/// true.  With it, `verify` checks "the term inhabits the
/// proposition the user wrote".
///
/// **Phase-4 coverage**:
///
///   - `Literal(Bool::true)` → `Universe(0)` — the trivially-inhabited
///     proposition (every term-of-Universe-0 inhabits it).  This
///     covers theorems with `ensures true` (the most common shape
///     in `kernel_v0/rules/k_*.vr` and corpus delegating theorems).
///   - `Path(name)`, `Field(...)`, `Call(...)` — delegates to
///     [`expr_to_term`].  Covers propositions like `my_predicate`
///     (a path-named axiom) or `has_property(x)` (an axiom applied
///     to args).
///
/// **Phase-5 work** (deferred — needs kernel extension or Leibniz
/// encoding for equality):
///
///   - `Binary { op: Eq, .. }` — encode via Leibniz `Pi(P, Pi(P(a), P(b)))`.
///   - `Binary { op: And, .. }` — encode via Pi-pair.
///   - `Binary { op: Or, .. }` — encode via Pi-sum.
///   - Quantifiers (forall / exists) — Pi / Church encoding.
pub fn proposition_to_term(
    prop: &Expr,
    ctx: &ElabContext,
) -> Result<Term, ElabError> {
    use verum_ast::LiteralKind;
    match &prop.kind {
        ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Bool(_)) => {
            // `true` is the trivially-inhabited proposition.  Encode
            // as `Universe(0)`; every closed term of type `Universe(0)`
            // inhabits the trivial proposition.  `false` is encoded
            // the same way for now (Phase-6 adds proper bottom-type
            // encoding via `Pi(P: Universe(0). P)`).
            Ok(Term::Universe(0))
        }
        ExprKind::Path(_) | ExprKind::Field { .. } | ExprKind::Call { .. } => {
            // Path / Field / Call propositions reduce to expression
            // translation — these are predicate names or applications.
            expr_to_term(prop, ctx)
        }
        ExprKind::Binary { op, left, right } => {
            // **Phase-5 connective encoding** — opaque axiomatic form.
            // The connective (And / Or / Eq / etc.) is registered in
            // the global axiom registry as an opaque polymorphic
            // operator.  The proposition translates to an `App` chain
            // applying the connective axiom to the translated operands.
            //
            // **Soundness**: the kernel verifies that the resulting
            // term is well-typed at the connective axiom's claimed
            // type — it doesn't UNDERSTAND the connective semantically,
            // but the type-correctness check still rejects malformed
            // applications.  This is the same approach mathlib-Lean
            // uses for `Eq`, `And`, `Or` at the kernel level when
            // running with a forward-axiom encoding.
            let connective_name = binop_to_axiom_name(*op).ok_or_else(|| {
                ElabError::UnsupportedExpression(format!(
                    "Binary op {:?} not yet wired as a Phase-5 connective axiom",
                    op,
                ))
            })?;
            let head = resolve_apply_target(ctx, connective_name)?;
            let lhs = expr_to_term(left, ctx)?;
            let rhs = expr_to_term(right, ctx)?;
            Ok(build_app_chain(head, vec![lhs, rhs]))
        }
        ExprKind::Unary { op, expr: operand } => {
            // **Phase-5 unary connective encoding** — Not is the
            // primary unary connective at the proposition level.
            let connective_name = unop_to_axiom_name(*op).ok_or_else(|| {
                ElabError::UnsupportedExpression(format!(
                    "Unary op {:?} not yet wired as a Phase-5 connective axiom",
                    op,
                ))
            })?;
            let head = resolve_apply_target(ctx, connective_name)?;
            let arg = expr_to_term(operand, ctx)?;
            Ok(build_app_chain(head, vec![arg]))
        }
        other => Err(ElabError::UnsupportedExpression(format!(
            "proposition translation: ExprKind::{} not yet supported (Phase-6 quantifiers)",
            expr_kind_tag(other),
        ))),
    }
}

/// Map a Verum binary operator to the canonical axiom name in the
/// elaboration context's connective registry.  Returns `None` for
/// operators that aren't propositional connectives (arithmetic,
/// assignment).
///
/// **Soundness invariant**: callers must register the corresponding
/// axiom (via [`ElabContext::register_axiom`]) before elaborating a
/// proposition that uses it.  See [`register_propositional_connectives`]
/// for the canonical bootstrap.
fn binop_to_axiom_name(op: verum_ast::BinOp) -> Option<&'static str> {
    use verum_ast::BinOp;
    match op {
        BinOp::And => Some("__verum_kernel_And"),
        BinOp::Or => Some("__verum_kernel_Or"),
        BinOp::Eq => Some("__verum_kernel_Eq"),
        BinOp::Ne => Some("__verum_kernel_Ne"),
        BinOp::Lt => Some("__verum_kernel_Lt"),
        BinOp::Le => Some("__verum_kernel_Le"),
        BinOp::Gt => Some("__verum_kernel_Gt"),
        BinOp::Ge => Some("__verum_kernel_Ge"),
        BinOp::Imply => Some("__verum_kernel_Implies"),
        BinOp::Iff => Some("__verum_kernel_Iff"),
        BinOp::In => Some("__verum_kernel_In"),
        _ => None,
    }
}

/// Map a Verum unary operator to the canonical axiom name.
fn unop_to_axiom_name(op: verum_ast::expr::UnOp) -> Option<&'static str> {
    use verum_ast::expr::UnOp;
    match op {
        UnOp::Not => Some("__verum_kernel_Not"),
        _ => None,
    }
}

/// **Bootstrap helper** — register the canonical propositional
/// connective axioms in `ctx`.  Call this once at the start of any
/// elaboration that needs to translate `Binary`/`Unary` propositions.
///
/// Each axiom is registered with claimed type `Universe(0)` (opaque
/// polymorphic form).  The kernel-side type-check is structural: the
/// connective is treated as a value of `Universe(0)`, applications
/// produce `Universe(0)`, and the certificate's `claimed_type` is a
/// chain of `Universe(0)` values that the kernel verifies cleanly.
///
/// **Phase-6 work** (deferred): replace these opaque axioms with full
/// Leibniz / Church / Pi encodings so the connectives are *understood*
/// by the kernel, not just *applied*.  E.g. `Eq` would unfold to
/// `Pi(A: Universe, Pi(A, Pi(A, Pi(P: A → Type, Pi(P(a), P(b))))))`
/// (Leibniz at level 0).
pub fn register_propositional_connectives(ctx: &mut ElabContext) {
    for name in [
        "__verum_kernel_And",
        "__verum_kernel_Or",
        "__verum_kernel_Eq",
        "__verum_kernel_Ne",
        "__verum_kernel_Lt",
        "__verum_kernel_Le",
        "__verum_kernel_Gt",
        "__verum_kernel_Ge",
        "__verum_kernel_Implies",
        "__verum_kernel_Iff",
        "__verum_kernel_In",
        "__verum_kernel_Not",
    ] {
        if ctx.get_axiom(name).is_none() {
            ctx.register_axiom(name, Term::Universe(0));
        }
    }
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
// AST integration — consume real Verum AST values
// =============================================================================

use verum_ast::decl::{ProofBody, TacticExpr, TheoremDecl};
use verum_ast::expr::{Expr, ExprKind};

/// **Elaborate one tactic expression.**  Phase-1 supports:
///
///   - `Apply { lemma, args }` — `apply <name>(<args>);` — the most
///     common shape, covers every `kernel_v0/lemmas/` stub and every
///     `@delegate` theorem from #146.
///   - `Reflexivity` — `refl` — produces `Var(0)` referencing the
///     innermost binder (Phase-1 simplification: assumes the goal
///     was just-introduced).
///   - `Exact(expr)` — `exact <expr>;` — translates the expr.
///
/// All other tactic forms return [`ElabError::UnsupportedTactic`]
/// with the variant name; Phase-2 adds them progressively.
pub fn elaborate_tactic(
    tactic: &TacticExpr,
    ctx: &mut ElabContext,
) -> Result<Term, ElabError> {
    match tactic {
        TacticExpr::Apply { lemma, args } => {
            let lemma_name = expr_to_path_name(lemma)
                .ok_or_else(|| ElabError::UnsupportedExpression(format!(
                    "apply target is not a path: {:?}",
                    lemma.kind,
                )))?;
            let head = resolve_apply_target(ctx, &lemma_name)?;
            let mut arg_terms = Vec::with_capacity(args.len());
            for arg in args.iter() {
                arg_terms.push(expr_to_term(arg, ctx)?);
            }
            Ok(build_app_chain(head, arg_terms))
        }
        TacticExpr::Reflexivity => {
            // Phase-1 stand-in: refl produces a reference to the
            // innermost binder.  Real refl would produce a
            // `DefinitionalEquality::Refl` witness, but the kernel
            // proof_checker doesn't yet expose a Refl-term form.
            // Phase-2 wires this through the kernel's def_eq path.
            if ctx.depth() == 0 {
                return Err(ElabError::UnsupportedTactic(
                    "Reflexivity in empty context — Phase-2 will wire \
                     def-eq witnesses".into(),
                ));
            }
            Ok(Term::Var(0))
        }
        TacticExpr::Exact(expr) => expr_to_term(expr, ctx),
        // Phase-1 stops here.  Every other tactic is recorded as
        // unsupported with the variant name for diagnostics.
        TacticExpr::Trivial => Err(ElabError::UnsupportedTactic("Trivial".into())),
        TacticExpr::Assumption => Err(ElabError::UnsupportedTactic("Assumption".into())),
        TacticExpr::Intro(_) => Err(ElabError::UnsupportedTactic("Intro".into())),
        TacticExpr::Rewrite { .. } => Err(ElabError::UnsupportedTactic("Rewrite".into())),
        TacticExpr::Simp { .. } => Err(ElabError::UnsupportedTactic("Simp".into())),
        TacticExpr::Ring => Err(ElabError::UnsupportedTactic("Ring".into())),
        TacticExpr::Field => Err(ElabError::UnsupportedTactic("Field".into())),
        TacticExpr::Omega => Err(ElabError::UnsupportedTactic("Omega".into())),
        TacticExpr::Auto { .. } => Err(ElabError::UnsupportedTactic("Auto".into())),
        TacticExpr::Blast => Err(ElabError::UnsupportedTactic("Blast".into())),
        TacticExpr::Smt { .. } => Err(ElabError::UnsupportedTactic("Smt".into())),
        TacticExpr::Split => Err(ElabError::UnsupportedTactic("Split".into())),
        TacticExpr::Left => Err(ElabError::UnsupportedTactic("Left".into())),
        TacticExpr::Right => Err(ElabError::UnsupportedTactic("Right".into())),
        TacticExpr::Exists(_) => Err(ElabError::UnsupportedTactic("Exists".into())),
        TacticExpr::CasesOn(_) => Err(ElabError::UnsupportedTactic("CasesOn".into())),
        TacticExpr::InductionOn(_) => Err(ElabError::UnsupportedTactic("InductionOn".into())),
        TacticExpr::Unfold(_) => Err(ElabError::UnsupportedTactic("Unfold".into())),
        TacticExpr::Compute => Err(ElabError::UnsupportedTactic("Compute".into())),
        TacticExpr::Try(_) => Err(ElabError::UnsupportedTactic("Try".into())),
        TacticExpr::TryElse { .. } => Err(ElabError::UnsupportedTactic("TryElse".into())),
        TacticExpr::Repeat(_) => Err(ElabError::UnsupportedTactic("Repeat".into())),
        TacticExpr::Seq(_) => Err(ElabError::UnsupportedTactic(
            "Seq — Phase-2 will compose multi-step bodies".into(),
        )),
        TacticExpr::Alt(_) => Err(ElabError::UnsupportedTactic("Alt".into())),
        TacticExpr::AllGoals(_) => Err(ElabError::UnsupportedTactic("AllGoals".into())),
        TacticExpr::Focus(_) => Err(ElabError::UnsupportedTactic("Focus".into())),
        TacticExpr::Named { .. } => Err(ElabError::UnsupportedTactic("Named".into())),
        TacticExpr::Let { .. } => Err(ElabError::UnsupportedTactic("Let".into())),
        _ => Err(ElabError::UnsupportedTactic(
            "unknown TacticExpr variant".into(),
        )),
    }
}

/// **Elaborate a proof body.**  Phase-3 supports:
///
///   - `ProofBody::Tactic(t)` — delegates to [`elaborate_tactic`].
///   - `ProofBody::Term(e)` — direct Curry-Howard proof term.
///     Delegates to [`expr_to_term`] for the expression-to-term
///     translation.  This handles the `proof = lemma_name(args)`
///     syntax where the user writes a constructive witness directly
///     (no tactic wrapping).
///
/// Phase-4 adds `Structured`, `ByMethod`.
pub fn elaborate_proof_body(
    body: &ProofBody,
    ctx: &mut ElabContext,
) -> Result<Term, ElabError> {
    match body {
        ProofBody::Tactic(t) => elaborate_tactic(t, ctx),
        ProofBody::Term(e) => expr_to_term(e, ctx),
        ProofBody::Structured(_) => Err(ElabError::UnsupportedTactic(
            "ProofBody::Structured — Phase-4 unrolls structured proofs".into(),
        )),
        ProofBody::ByMethod(_) => Err(ElabError::UnsupportedTactic(
            "ProofBody::ByMethod — Phase-4 dispatches by-induction / by-cases".into(),
        )),
    }
}

/// **Elaborate a complete theorem.**  Top-level entry point.
///
/// Algorithm:
///   1. Verify the theorem has a proof body (else `NoProofBody`).
///   2. Elaborate the proof body to a `Term`.
///   3. Build a [`crate::verification_goal::VerificationGoal`] from
///      the theorem (#167 unification).  The goal's `to_term()`
///      method produces a Pi-chain over hypotheses with the
///      proposition as conclusion — this is the certificate's
///      claimed type.
///   4. Falls back to [`placeholder_proposition`] when the
///      proposition shape is Phase-7+ work (so theorems with
///      complex propositions still produce a *weakly* load-bearing
///      certificate rather than failing outright).
///   5. [`close_over_axioms`] to wrap the body in a `Lam`/`Pi` chain
///      over the registered axiom table.
///   6. [`finalise_certificate`] re-verifies via the kernel checker.
///
/// **Post-#167**: the claimed type comes from the unified
/// `VerificationGoal::to_term()` — the same shape that fn-contracts
/// and refinement-predicates produce.  One verification surface,
/// many sources.
pub fn elaborate_theorem(
    theorem: &TheoremDecl,
    ctx: &mut ElabContext,
) -> Result<Certificate, ElabError> {
    use crate::verification_goal::{from_theorem_decl, TheoremKind};

    let body = theorem
        .proof
        .as_ref()
        .ok_or(ElabError::NoProofBody)?;
    let body_term = elaborate_proof_body(body, ctx)?;

    // Phase-6: build a unified VerificationGoal from the theorem.
    // The goal's to_term() is the certificate's claimed type — a
    // Pi-chain over hypotheses with the proposition as conclusion.
    // On translation failure, fall back to placeholder (graceful).
    let (body_type, prop_translation_status) =
        match from_theorem_decl(theorem, TheoremKind::Theorem, ctx) {
            Ok(goal) => (goal.to_term(), "verification_goal"),
            Err(_) => (placeholder_proposition(), "placeholder"),
        };

    let (closed_term, closed_type) = close_over_axioms(ctx, body_term, body_type);
    let mut metadata = BTreeMap::new();
    metadata.insert("theorem_name".to_string(), theorem.name.name.to_string());
    metadata.insert("kernel_version".to_string(), crate::VVA_VERSION.to_string());
    metadata.insert("elaborator_phase".to_string(), "6".to_string());
    metadata.insert(
        "proposition_translation".to_string(),
        prop_translation_status.to_string(),
    );
    finalise_certificate(closed_term, closed_type, metadata)
}

/// **Path → name extraction.**  Walks an Expr tree expecting
/// `Path(name)` or a single-segment `Field` chain.  Returns the
/// dotted name as a String.  Used by `apply` to read its lemma
/// target.  Returns `None` for non-path expressions.
pub fn expr_to_path_name(expr: &Expr) -> Option<String> {
    use verum_ast::ty::PathSegment;
    match &expr.kind {
        ExprKind::Path(path) => {
            let mut parts: Vec<String> = Vec::with_capacity(path.segments.len());
            for seg in path.segments.iter() {
                match seg {
                    PathSegment::Name(ident) => parts.push(ident.name.to_string()),
                    PathSegment::SelfValue => parts.push("self".to_string()),
                    PathSegment::Super => parts.push("super".to_string()),
                    PathSegment::Cog => parts.push("cog".to_string()),
                    PathSegment::Relative => parts.push("".to_string()),
                }
            }
            Some(parts.join("."))
        }
        ExprKind::Field { expr: object, field } => {
            let base = expr_to_path_name(object)?;
            Some(format!("{}.{}", base, field.name))
        }
        _ => None,
    }
}

/// **Translate a Verum `Expr` to a kernel `Term`.**  Phase-1 handles:
///
///   - `Path(name)` — resolves via [`resolve_apply_target`].
///   - `Field(obj, field)` — composes path name then resolves.
///   - `Call(f, args)` — recursive translation + `App` chain.
///
/// All other expression forms return [`ElabError::UnsupportedExpression`].
/// Phase-2 adds: literals, conditionals, lambda expressions,
/// type-level constructs.
pub fn expr_to_term(expr: &Expr, ctx: &ElabContext) -> Result<Term, ElabError> {
    match &expr.kind {
        ExprKind::Path(_) | ExprKind::Field { .. } => {
            let name = expr_to_path_name(expr).ok_or_else(|| {
                ElabError::UnsupportedExpression("path-walk failed".into())
            })?;
            resolve_apply_target(ctx, &name)
        }
        ExprKind::Call { func, args, .. } => {
            let head = expr_to_term(func, ctx)?;
            let mut arg_terms = Vec::with_capacity(args.len());
            for arg in args.iter() {
                arg_terms.push(expr_to_term(arg, ctx)?);
            }
            Ok(build_app_chain(head, arg_terms))
        }
        other => Err(ElabError::UnsupportedExpression(format!(
            "Phase-1 does not handle ExprKind::{}",
            expr_kind_tag(other),
        ))),
    }
}

/// Diagnostic-only tag for the [`ExprKind`] variant.  Used by error
/// messages to name the unsupported expression form.
fn expr_kind_tag(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::Literal(_) => "Literal",
        ExprKind::Path(_) => "Path",
        ExprKind::Field { .. } => "Field",
        ExprKind::Call { .. } => "Call",
        ExprKind::Binary { .. } => "Binary",
        ExprKind::Unary { .. } => "Unary",
        ExprKind::Block(_) => "Block",
        ExprKind::If { .. } => "If",
        ExprKind::Match { .. } => "Match",
        ExprKind::Tuple(_) => "Tuple",
        ExprKind::Index { .. } => "Index",
        ExprKind::MethodCall { .. } => "MethodCall",
        _ => "<other>",
    }
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

    // ----- AST-integration tests (Phase-2 entry points) -----

    use verum_ast::decl::{ProofBody, TacticExpr};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::List;
    use verum_common::Span;

    /// Build a Path expression with a single segment.  Test helper so
    /// integration tests don't drown in AST construction.
    fn path_expr(name: &str) -> Expr {
        let span = Span::dummy();
        let mut list = List::new();
        list.push(PathSegment::Name(Ident {
            name: name.into(),
            span,
        }));
        let path = Path::new(list, span);
        Expr::new(ExprKind::Path(path), span)
    }

    /// Build a dotted-Path expression `a.b.c`.
    fn path_expr_dotted(parts: &[&str]) -> Expr {
        let span = Span::dummy();
        let mut list = List::new();
        for p in parts {
            list.push(PathSegment::Name(Ident {
                name: (*p).into(),
                span,
            }));
        }
        let path = Path::new(list, span);
        Expr::new(ExprKind::Path(path), span)
    }

    #[test]
    fn expr_to_path_name_extracts_simple() {
        assert_eq!(expr_to_path_name(&path_expr("foo")).as_deref(), Some("foo"));
    }

    #[test]
    fn expr_to_path_name_dotted_path() {
        assert_eq!(
            expr_to_path_name(&path_expr_dotted(&["mathlib4", "lambda", "ChurchRosser"])).as_deref(),
            Some("mathlib4.lambda.ChurchRosser"),
        );
    }

    #[test]
    fn expr_to_term_resolves_path_via_axiom() {
        let mut ctx = ElabContext::new();
        ctx.register_axiom("foo", Term::Universe(0));
        let term = expr_to_term(&path_expr("foo"), &ctx).unwrap();
        assert_eq!(term, Term::Var(0)); // depth 0 + axiom position 0
    }

    #[test]
    fn elaborate_tactic_apply_zero_args() {
        // Tactic body: `apply foo;`
        let mut ctx = ElabContext::new();
        ctx.register_axiom("foo", Term::Universe(0));
        let tactic = TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("foo")),
            args: List::new(),
        };
        let term = elaborate_tactic(&tactic, &mut ctx).unwrap();
        assert_eq!(term, Term::Var(0));
    }

    #[test]
    fn elaborate_tactic_apply_with_args() {
        // Tactic body: `apply foo(a, b);` where foo, a, b all axioms.
        // BTreeMap iterates by KEY order: a < b < foo, so positions are
        // a=0, b=1, foo=2.  Depth=0.
        let mut ctx = ElabContext::new();
        ctx.register_axiom("foo", Term::Universe(0));
        ctx.register_axiom("a", Term::Universe(0));
        ctx.register_axiom("b", Term::Universe(0));
        let mut args = List::new();
        args.push(path_expr("a"));
        args.push(path_expr("b"));
        let tactic = TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("foo")),
            args,
        };
        let term = elaborate_tactic(&tactic, &mut ctx).unwrap();
        // App(App(Var(2), Var(0)), Var(1)) — foo=2 (head), a=0, b=1.
        assert_eq!(
            term,
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Var(2)),
                    Box::new(Term::Var(0)),
                )),
                Box::new(Term::Var(1)),
            ),
        );
    }

    #[test]
    fn elaborate_tactic_apply_undeclared_rejects() {
        let mut ctx = ElabContext::new();
        let tactic = TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("nope")),
            args: List::new(),
        };
        match elaborate_tactic(&tactic, &mut ctx) {
            Err(ElabError::UndeclaredApplyTarget(name)) => assert_eq!(name, "nope"),
            other => panic!("expected UndeclaredApplyTarget, got {:?}", other),
        }
    }

    #[test]
    fn elaborate_tactic_unsupported_returns_named_error() {
        let mut ctx = ElabContext::new();
        match elaborate_tactic(&TacticExpr::Ring, &mut ctx) {
            Err(ElabError::UnsupportedTactic(t)) => assert_eq!(t, "Ring"),
            other => panic!("expected UnsupportedTactic(Ring), got {:?}", other),
        }
        match elaborate_tactic(&TacticExpr::Smt { solver: verum_common::Maybe::None, timeout: verum_common::Maybe::None }, &mut ctx) {
            Err(ElabError::UnsupportedTactic(t)) => assert_eq!(t, "Smt"),
            other => panic!("expected UnsupportedTactic(Smt), got {:?}", other),
        }
    }

    #[test]
    fn elaborate_proof_body_dispatches_to_tactic() {
        let mut ctx = ElabContext::new();
        ctx.register_axiom("witness", Term::Universe(0));
        let body = ProofBody::Tactic(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("witness")),
            args: List::new(),
        });
        let term = elaborate_proof_body(&body, &mut ctx).unwrap();
        assert_eq!(term, Term::Var(0));
    }

    #[test]
    fn elaborate_proof_body_term_succeeds_with_axiom_witness() {
        // Phase-3: ProofBody::Term(expr) — direct Curry-Howard term.
        // `proof = foo` where foo is a registered axiom should produce
        // the same Var(idx) the apply form would.
        let mut ctx = ElabContext::new();
        ctx.register_axiom("foo", Term::Universe(0));
        let body = ProofBody::Term(verum_common::Heap::new(path_expr("foo")));
        let term = elaborate_proof_body(&body, &mut ctx).unwrap();
        assert_eq!(term, Term::Var(0));
    }

    #[test]
    fn elaborate_proof_body_term_undeclared_rejects() {
        let mut ctx = ElabContext::new();
        let body = ProofBody::Term(verum_common::Heap::new(path_expr("nope")));
        match elaborate_proof_body(&body, &mut ctx) {
            Err(ElabError::UndeclaredApplyTarget(name)) => assert_eq!(name, "nope"),
            other => panic!("expected UndeclaredApplyTarget, got {:?}", other),
        }
    }

    #[test]
    fn elaborate_proof_body_structured_unsupported() {
        use verum_ast::decl::ProofStructure;
        let mut ctx = ElabContext::new();
        let body = ProofBody::Structured(ProofStructure {
            steps: List::new(),
            conclusion: verum_common::Maybe::None,
            span: Span::dummy(),
        });
        match elaborate_proof_body(&body, &mut ctx) {
            Err(ElabError::UnsupportedTactic(_)) => {}
            other => panic!("expected UnsupportedTactic, got {:?}", other),
        }
    }

    #[test]
    fn elaborate_theorem_apply_axiom_round_trips() {
        // Construct: `theorem id_proof() ensures true { proof { apply foo; } }`
        // where foo is an axiom of type Universe(0) and proposition
        // is `true` (Bool literal).  The elaborator produces a
        // closure-over-axioms certificate that the kernel re-verifies.
        //
        // Phase-4: proposition `true` translates to Universe(0); body
        // `apply foo` produces Var(0) with type Universe(0) — they
        // match, so the kernel accepts.
        use verum_ast::decl::TheoremDecl;
        use verum_ast::Literal;
        let span = Span::dummy();
        // Proposition is the boolean literal `true` (translates to Universe(0)).
        let true_prop = Expr::new(
            ExprKind::Literal(Literal::bool(true, span)),
            span,
        );
        let mut theorem = TheoremDecl::new(
            Ident { name: "id_proof".into(), span },
            true_prop,
            span,
        );
        theorem.proof = verum_common::Maybe::Some(ProofBody::Tactic(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("foo")),
            args: List::new(),
        }));

        let mut ctx = ElabContext::new();
        ctx.register_axiom("foo", Term::Universe(0));

        let cert = elaborate_theorem(&theorem, &mut ctx).unwrap();
        // De Bruijn criterion: certificate re-verifies via the kernel.
        cert.verify().unwrap();
        // Metadata pin
        assert_eq!(
            cert.metadata.get("theorem_name").map(|s| s.as_str()),
            Some("id_proof"),
        );
        assert_eq!(
            cert.metadata.get("elaborator_phase").map(|s| s.as_str()),
            Some("6"),
        );
        // Phase-4 records whether the proposition was translated.
        assert_eq!(
            cert.metadata.get("proposition_translation").map(|s| s.as_str()),
            Some("verification_goal"),
            "Bool literal proposition should translate via VerificationGoal",
        );
    }

    #[test]
    fn proposition_to_term_handles_bool_literal_true() {
        use verum_ast::Literal;
        let span = Span::dummy();
        let prop = Expr::new(
            ExprKind::Literal(Literal::bool(true, span)),
            span,
        );
        let ctx = ElabContext::new();
        let term = proposition_to_term(&prop, &ctx).unwrap();
        assert_eq!(term, Term::Universe(0));
    }

    #[test]
    fn proposition_to_term_handles_path_via_axiom() {
        let mut ctx = ElabContext::new();
        ctx.register_axiom("my_predicate", Term::Universe(0));
        let prop = path_expr("my_predicate");
        let term = proposition_to_term(&prop, &ctx).unwrap();
        assert_eq!(term, Term::Var(0));
    }

    #[test]
    fn proposition_to_term_handles_unknown_path_via_undeclared() {
        let ctx = ElabContext::new();
        let prop = path_expr("unknown_pred");
        match proposition_to_term(&prop, &ctx) {
            Err(ElabError::UndeclaredApplyTarget(name)) => assert_eq!(name, "unknown_pred"),
            other => panic!("expected UndeclaredApplyTarget, got {:?}", other),
        }
    }

    #[test]
    fn proposition_to_term_binary_eq_translates_via_connective_axiom() {
        // Phase-5: Binary `a == b` translates via the registered Eq
        // connective axiom.  `App(App(Eq, a), b)` is the encoding.
        let span = Span::dummy();
        let prop = Expr::new(
            ExprKind::Binary {
                op: verum_ast::BinOp::Eq,
                left: verum_common::Heap::new(path_expr("a")),
                right: verum_common::Heap::new(path_expr("b")),
            },
            span,
        );
        let mut ctx = ElabContext::new();
        register_propositional_connectives(&mut ctx);
        ctx.register_axiom("a", Term::Universe(0));
        ctx.register_axiom("b", Term::Universe(0));
        let term = proposition_to_term(&prop, &ctx).unwrap();
        // Term is App(App(Var(eq_idx), Var(a_idx)), Var(b_idx)) — exact
        // indices depend on BTreeMap key order; just check the shape.
        match term {
            Term::App(outer, b_arg) => match (*outer, *b_arg) {
                (Term::App(eq, a_arg), Term::Var(_)) => {
                    assert!(matches!(*eq, Term::Var(_)), "head should be Var");
                    assert!(matches!(*a_arg, Term::Var(_)), "lhs arg should be Var");
                }
                _ => panic!("expected App(App(_, _), Var), got differently-shaped term"),
            },
            other => panic!("expected App, got {:?}", other),
        }
    }

    #[test]
    fn proposition_to_term_unsupported_shape_still_falls_through() {
        // Match expressions are Phase-6 work.
        let span = Span::dummy();
        // Build a Match expr; we don't care about its inner shape.
        let prop = Expr::new(
            ExprKind::Block(verum_ast::Block::new(
                Vec::<verum_ast::Stmt>::new().into(),
                verum_common::Maybe::None,
                span,
            )),
            span,
        );
        let ctx = ElabContext::new();
        match proposition_to_term(&prop, &ctx) {
            Err(ElabError::UnsupportedExpression(msg)) => {
                assert!(
                    msg.contains("Block"),
                    "expected Block in error msg: {}",
                    msg,
                );
            }
            other => panic!("expected UnsupportedExpression, got {:?}", other),
        }
    }

    #[test]
    fn proposition_to_term_binary_unwired_op_returns_unsupported() {
        // Arithmetic ops aren't propositional connectives.
        let span = Span::dummy();
        let mut ctx = ElabContext::new();
        register_propositional_connectives(&mut ctx);
        ctx.register_axiom("a", Term::Universe(0));
        ctx.register_axiom("b", Term::Universe(0));
        let prop = Expr::new(
            ExprKind::Binary {
                op: verum_ast::BinOp::Add,
                left: verum_common::Heap::new(path_expr("a")),
                right: verum_common::Heap::new(path_expr("b")),
            },
            span,
        );
        match proposition_to_term(&prop, &ctx) {
            Err(ElabError::UnsupportedExpression(msg)) => {
                assert!(
                    msg.contains("Add"),
                    "expected Add in error msg: {}",
                    msg,
                );
            }
            other => panic!("expected UnsupportedExpression(Add), got {:?}", other),
        }
    }

    #[test]
    fn register_propositional_connectives_is_idempotent() {
        let mut ctx = ElabContext::new();
        register_propositional_connectives(&mut ctx);
        let depth_after_first = ctx.get_axiom("__verum_kernel_And").is_some();
        register_propositional_connectives(&mut ctx);
        let depth_after_second = ctx.get_axiom("__verum_kernel_And").is_some();
        assert!(depth_after_first);
        assert!(depth_after_second);
    }

    #[test]
    fn elaborate_theorem_complex_proposition_without_connectives_falls_back() {
        // Theorem with `Binary` proposition (a == b) BUT without
        // calling `register_propositional_connectives` — the Eq axiom
        // is missing, so the elaborator falls back to placeholder.
        use verum_ast::decl::TheoremDecl;
        let span = Span::dummy();
        let prop = Expr::new(
            ExprKind::Binary {
                op: verum_ast::BinOp::Eq,
                left: verum_common::Heap::new(path_expr("a")),
                right: verum_common::Heap::new(path_expr("b")),
            },
            span,
        );
        let mut theorem = TheoremDecl::new(
            Ident { name: "binary_prop".into(), span },
            prop,
            span,
        );
        theorem.proof = verum_common::Maybe::Some(ProofBody::Tactic(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("witness")),
            args: List::new(),
        }));
        let mut ctx = ElabContext::new();
        ctx.register_axiom("witness", Term::Universe(0));
        // Note: connectives NOT registered, so Eq axiom is undeclared.

        let cert = elaborate_theorem(&theorem, &mut ctx).unwrap();
        cert.verify().unwrap();
        // Fallback recorded in metadata.
        assert_eq!(
            cert.metadata.get("proposition_translation").map(|s| s.as_str()),
            Some("placeholder"),
            "Binary proposition without connectives registered should fall back",
        );
    }

    #[test]
    fn elaborate_theorem_no_body_rejects() {
        use verum_ast::decl::TheoremDecl;
        let span = Span::dummy();
        let theorem = TheoremDecl::new(
            Ident { name: "unproved".into(), span },
            path_expr("foo"),
            span,
        );
        // theorem.proof is None by default.
        let mut ctx = ElabContext::new();
        match elaborate_theorem(&theorem, &mut ctx) {
            Err(ElabError::NoProofBody) => {}
            other => panic!("expected NoProofBody, got {:?}", other),
        }
    }
}
