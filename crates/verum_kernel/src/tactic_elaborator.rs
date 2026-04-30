//! Tactic-to-proof-term elaboration — connects Verum proof bodies
//! to kernel-checkable [`Certificate`] values.
//!
//! # The de Bruijn criterion
//!
//! The architectural pattern that makes a proof assistant
//! trustworthy is:
//!
//!     trusted_kernel + tactic_as_proof_term_builder
//!
//! Hilbert-style proofs run inside the kernel itself; tactic-style
//! proofs are *productivity sugar* whose semantics IS proof-term
//! construction.  Without the second half, the trust base
//! ([`proof_checker`], `core/verify/kernel_v0/`) is theoretically
//! trustworthy but practically unused — no Verum theorem reduces
//! to a kernel-readable term.
//!
//! This module is the second half: it walks
//! `ProofBody::Tactic(TacticExpr)` (or `ProofBody::Term(Expr)`) and
//! emits a `Term` that the kernel re-checks against the theorem's
//! [`crate::verification_goal::VerificationGoal::to_term`].
//!
//! # Surface
//!
//!   - [`ElabContext`] — name → de-Bruijn-index map for local
//!     binders + global axiom registry.
//!   - [`elaborate_theorem`] — top-level entry: `TheoremDecl` →
//!     [`Certificate`].  Builds the certificate's claimed type via
//!     [`crate::verification_goal::from_theorem_decl`] so the
//!     verification surface stays unified across theorems / lemmas /
//!     corollaries / fn-contracts / refinements.
//!   - [`elaborate_proof_body`] / [`elaborate_tactic`] — walk the
//!     proof body / tactic expression to produce a `Term`.
//!   - [`expr_to_term`] / [`proposition_to_term`] — translate
//!     expressions and proposition shapes into kernel terms.
//!   - [`ElabError`] — structured error type for unsupported tactic
//!     forms, undeclared lemmas, unsupported expression shapes,
//!     and kernel-rejection contract violations.
//!
//! # Tactic coverage
//!
//! Tactics that emit kernel-readable terms:
//!
//!   - `Apply { lemma, args }` — `App` chain over an axiom or local
//!     binder resolved via [`resolve_apply_target`].
//!   - `Exact(expr)` — direct Curry-Howard term via [`expr_to_term`].
//!   - `Reflexivity` — innermost-binder reference (placeholder until
//!     `DefinitionalEquality::Refl` is wired into the kernel checker).
//!
//! Other tactic forms ([`TacticExpr::Seq`], `Intro`, `Rewrite`,
//! `Induction`, `Smt`, `Ring`, `Omega`, …) return
//! [`ElabError::UnsupportedTactic`] with the variant name so
//! downstream tooling can route around them.
//!
//! # Proposition coverage
//!
//! [`proposition_to_term`] translates the following Verum proposition
//! shapes into kernel terms:
//!
//!   - `Literal(Bool::*)` → `Universe(0)` (trivially-inhabited).
//!   - `Path` / `Field` / `Call` — axiom resolution + App chain.
//!   - `Binary` — opaque connective axiom application
//!     (`__verum_kernel_<Op>` registered via
//!     [`register_propositional_connectives`]).
//!   - `Unary { op: Not, .. }` — `Not` connective App.
//!
//! Quantifiers, pattern matches, blocks, and conditionals fall
//! through to [`ElabError::UnsupportedExpression`].
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
    /// The proof body has a tactic form the elaborator does not
    /// translate.  Carries the variant name so callers can route
    /// around it (e.g., dispatch to a different verification path).
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
                write!(f, "expression form not supported by the elaborator: {}", e)
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
///   - **Registered axiom**: produces `Var(local_depth + axiom_position)`.
///     The axiom slot encoding works because [`close_over_axioms`]
///     places all registered axioms in the outermost `Pi`-chain
///     before any local binders, so the axiom-table position lifts
///     correctly past the local-binder shift.
///   - **Unknown**: returns [`ElabError::UndeclaredApplyTarget`].
pub fn resolve_apply_target(
    ctx: &ElabContext,
    name: &str,
) -> Result<Term, ElabError> {
    if let Some(idx) = ctx.lookup_local(name) {
        return Ok(Term::Var(idx));
    }
    if let Some(_axiom) = ctx.get_axiom(name) {
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

/// Default theorem-conclusion term — `Universe(0)` — used as a
/// fallback when [`proposition_to_term`] can't translate the
/// theorem's proposition.  The trivial type is inhabited by every
/// closed term-of-Universe-0; the certificate produced is therefore
/// only weakly load-bearing.  Callers should record the fallback in
/// the certificate's metadata so reviewers can downgrade trust.
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
/// Coverage:
///
///   - `Literal(Bool::*)` → `Universe(0)` (trivially-inhabited).
///   - `Path` / `Field` / `Call` → axiom resolution + App chain via
///     [`expr_to_term`].
///   - `Binary` / `Unary` → opaque connective axiom application.
///     The connective name is mapped via [`binop_to_axiom_name`] /
///     [`unop_to_axiom_name`]; the surrounding context must register
///     the matching axiom (see [`register_propositional_connectives`]).
///
/// Quantifiers, blocks, conditionals, pattern matches return
/// [`ElabError::UnsupportedExpression`].
pub fn proposition_to_term(
    prop: &Expr,
    ctx: &ElabContext,
) -> Result<Term, ElabError> {
    use verum_ast::LiteralKind;
    match &prop.kind {
        ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Bool(_)) => {
            // Both `true` and `false` map to `Universe(0)` — the
            // trivially-inhabited type.  The kernel doesn't yet
            // distinguish bottom; that distinction is a future
            // primitive-encoding upgrade.
            Ok(Term::Universe(0))
        }
        ExprKind::Path(_) | ExprKind::Field { .. } | ExprKind::Call { .. } => {
            // Predicate names or function applications — direct
            // axiom-resolution + App chain.
            expr_to_term(prop, ctx)
        }
        ExprKind::Binary { op, left, right } => {
            // Opaque-axiom connective encoding.  The connective is
            // registered as a polymorphic operator (claimed type
            // `Universe(0)`); the proposition translates to an `App`
            // chain over the operand terms.  The kernel verifies the
            // App chain is type-correct under the connective's
            // declared type — it doesn't *understand* the connective
            // semantically (that is a future primitive-encoding
            // upgrade), but the structural check still rejects
            // malformed applications.  Mirrors mathlib-Lean's
            // forward-axiom mode for `Eq` / `And` / `Or`.
            let connective_name = binop_to_axiom_name(*op).ok_or_else(|| {
                ElabError::UnsupportedExpression(format!(
                    "Binary op {:?} is not a propositional connective",
                    op,
                ))
            })?;
            let head = resolve_apply_target(ctx, connective_name)?;
            let lhs = expr_to_term(left, ctx)?;
            let rhs = expr_to_term(right, ctx)?;
            Ok(build_app_chain(head, vec![lhs, rhs]))
        }
        ExprKind::Unary { op, expr: operand } => {
            let connective_name = unop_to_axiom_name(*op).ok_or_else(|| {
                ElabError::UnsupportedExpression(format!(
                    "Unary op {:?} is not a propositional connective",
                    op,
                ))
            })?;
            let head = resolve_apply_target(ctx, connective_name)?;
            let arg = expr_to_term(operand, ctx)?;
            Ok(build_app_chain(head, vec![arg]))
        }
        other => Err(ElabError::UnsupportedExpression(format!(
            "proposition translation: ExprKind::{} not handled",
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
/// elaboration that needs to translate `Binary` / `Unary`
/// propositions.
///
/// Each axiom is registered with claimed type `Universe(0)` — the
/// opaque-polymorphic form.  The kernel-side type-check is
/// structural: the connective is a value of `Universe(0)`,
/// applications produce `Universe(0)`, and the certificate's
/// `claimed_type` is a chain of `Universe(0)` values the kernel
/// verifies cleanly.
///
/// A future primitive-encoding upgrade replaces these opaque axioms
/// with explicit Leibniz / Church / Pi forms so the connectives are
/// *understood* by the kernel, not merely *applied*: e.g. `Eq`
/// unfolds to `Π(A:𝓤, Π(a b:A, Π(P:A→𝓤, Π(P(a), P(b)))))`.
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

/// **Register the kernel_v0 lemma stubs** as axioms in `ctx`.
///
/// The five lemma stubs in `core/verify/kernel_v0/lemmas/` carry
/// `@framework(<system>, "<path>")` citations that pin upstream
/// proofs of fundamental meta-theorems (substitution lemma,
/// Church-Rosser confluence, cartesian closure, function-extensionality,
/// universe cumulativity).  The kernel rules in
/// `core/verify/kernel_v0/rules/k_*.vr` discharge their soundness
/// IOUs by `apply`-ing these lemma names.
///
/// Registering them up-front lets the elaborator resolve those
/// `apply` chains without forcing every corpus theorem to register
/// the lemma names individually.  Each stub carries claimed type
/// `Universe(0)` — the opaque-axiom encoding (as with the
/// connectives) — so the kernel checker accepts the structural
/// shape; the load-bearing semantics is the upstream citation
/// recorded by the `@framework` attribute.
pub fn register_kernel_v0_lemmas(ctx: &mut ElabContext) {
    for name in [
        "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
        "core.verify.kernel_v0.lemmas.beta.church_rosser_confluence",
        "core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi",
        "core.verify.kernel_v0.lemmas.eta.function_extensionality",
        "core.verify.kernel_v0.lemmas.sub.cumulative_universe_inclusion",
    ] {
        if ctx.get_axiom(name).is_none() {
            ctx.register_axiom(name, Term::Universe(0));
        }
    }
}

/// **Register the canonical kernel-bridge dispatcher names** as
/// axioms in `ctx`.
///
/// Kernel rule files in `core/verify/kernel_v0/rules/` carry
/// `@kernel_discharge("kernel_<rule>_strict")` annotations.  The
/// proof body of each rule's soundness lemma reduces to
/// `apply kernel_<rule>_strict` — invoking the bridge to the
/// dispatcher.  Registering these names lets the elaborator
/// resolve those calls.
pub fn register_kernel_bridge_dispatchers(ctx: &mut ElabContext) {
    for name in [
        "kernel_var_strict",
        "kernel_universe_intro_strict",
        "kernel_forward_axiom_strict",
        "kernel_positivity_strict",
        "kernel_pi_form_strict",
        "kernel_lam_intro_strict",
        "kernel_app_elim_strict",
        "kernel_beta_strict",
        "kernel_eta_strict",
        "kernel_sub_strict",
    ] {
        if ctx.get_axiom(name).is_none() {
            ctx.register_axiom(name, Term::Universe(0));
        }
    }
}

/// **Close the body and its type over the registered axiom table.**
///
/// Wraps `body` in a `Lam`-chain and `body_type` in a matching
/// `Pi`-chain — one binder per registered axiom, in axiom-table
/// (BTreeMap key) order.  The result is a closed `Term` (no free
/// variables) where `Var(i)` inside the original body refers to the
/// i-th axiom.
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

/// **Elaborate one tactic expression to a kernel `Term`.**
///
/// Tactics that emit kernel-readable terms:
///
///   - `Apply { lemma, args }` — `App` chain: head is the resolved
///     lemma / axiom / local binder; arguments are the translated
///     argument expressions.
///   - `Exact(expr)` — direct Curry-Howard term via [`expr_to_term`].
///   - `Reflexivity` — `Var(0)`, the innermost binder.  This is a
///     placeholder until the kernel checker exposes
///     `DefinitionalEquality::Refl`; it works for goals where the
///     just-introduced binder is the witness.
///
/// All other [`TacticExpr`] variants return
/// [`ElabError::UnsupportedTactic`] carrying the variant name so
/// downstream tooling can route around them.  Adding support for
/// a new tactic means: (1) build its kernel term shape here,
/// (2) ensure the corresponding kernel rule is sound.
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
            // Stand-in: refl returns a reference to the innermost
            // binder.  Replacing this with a real
            // `DefinitionalEquality::Refl` witness requires extending
            // the kernel checker with a Refl-term form.
            if ctx.depth() == 0 {
                return Err(ElabError::UnsupportedTactic(
                    "Reflexivity in empty context — needs a \
                     DefinitionalEquality witness".into(),
                ));
            }
            Ok(Term::Var(0))
        }
        TacticExpr::Exact(expr) => expr_to_term(expr, ctx),
        TacticExpr::Intro(idents) => {
            // `intro x` (or `intro x y z`) peels Pi-binders off the
            // goal and introduces them as local hypotheses.  The
            // proof term is `λx.body` where `body` is `Var(0)` —
            // the just-introduced binder.
            //
            // Standalone Intro produces `λx_0. λx_1. ... Var(0)`
            // (the innermost binder).  This handles the common
            // `theorem id<A>(x: A): A { proof { intro a; } }`
            // shape where the witness is the introduced binder.
            //
            // To compose Intro with subsequent tactics, use
            // `TacticExpr::Seq([Intro(_), <continuation>])`.
            let mut depth = 0;
            for ident in idents.iter() {
                ctx.push_binder(ident.name.to_string());
                depth += 1;
            }
            let mut term = Term::Var(0);
            for _ in 0..depth {
                term = Term::Lam(
                    Box::new(Term::Universe(0)),
                    Box::new(term),
                );
            }
            for _ in 0..depth {
                ctx.pop_binder();
            }
            Ok(term)
        }
        TacticExpr::Seq(steps) => elaborate_tactic_seq(steps, ctx),
        TacticExpr::Trivial => Err(ElabError::UnsupportedTactic("Trivial".into())),
        TacticExpr::Assumption => Err(ElabError::UnsupportedTactic("Assumption".into())),
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

/// **Elaborate a sequenced tactic chain.**
///
/// `Seq(steps)` represents `tactic_1; tactic_2; ...; tactic_n` —
/// a sequence whose intermediate steps modify the elaboration
/// context (typically by introducing binders) and whose final
/// step produces the proof term.
///
/// Currently supports:
///
///   - Intermediate `Intro(idents)` steps — push binders onto the
///     local context.  Other intermediate-step forms return
///     [`ElabError::UnsupportedTactic`].
///   - A single final step (any tactic that
///     [`elaborate_tactic`] handles).
///
/// The result is wrapped in a `Lam`-chain matching the introduced
/// binders, and the binders are popped before returning so the
/// context remains balanced for the surrounding scope.
fn elaborate_tactic_seq(
    steps: &verum_common::List<TacticExpr>,
    ctx: &mut ElabContext,
) -> Result<Term, ElabError> {
    if steps.is_empty() {
        return Err(ElabError::UnsupportedTactic("Seq of length 0".into()));
    }
    // Collect intro'd binder names so we can pop them and wrap the
    // result in a matching Lam-chain.
    let mut introduced: Vec<String> = Vec::new();
    let last_index = steps.len() - 1;
    for (i, step) in steps.iter().enumerate() {
        if i == last_index {
            // Final step produces the term; intermediate intro
            // binders are still in scope.
            let body = elaborate_tactic(step, ctx)?;
            // Wrap from innermost outwards.
            let mut term = body;
            for _ in 0..introduced.len() {
                term = Term::Lam(
                    Box::new(Term::Universe(0)),
                    Box::new(term),
                );
            }
            // Restore the context for the caller.
            for _ in 0..introduced.len() {
                ctx.pop_binder();
            }
            return Ok(term);
        }
        // Intermediate step.
        match step {
            TacticExpr::Intro(idents) => {
                for ident in idents.iter() {
                    let name = ident.name.to_string();
                    ctx.push_binder(name.clone());
                    introduced.push(name);
                }
            }
            other => {
                // Roll back the introduced binders before propagating
                // the error so the caller's context is untouched.
                for _ in 0..introduced.len() {
                    ctx.pop_binder();
                }
                return Err(ElabError::UnsupportedTactic(format!(
                    "Seq intermediate step: {} (only Intro is currently supported as a non-final step)",
                    tactic_variant_name(other),
                )));
            }
        }
    }
    // Unreachable: the loop returns or errors on the last index.
    Err(ElabError::UnsupportedTactic("Seq fell through".into()))
}

/// Diagnostic-only tag for a `TacticExpr` variant.  Used by error
/// messages to name the unsupported tactic form.
fn tactic_variant_name(t: &TacticExpr) -> &'static str {
    match t {
        TacticExpr::Trivial => "Trivial",
        TacticExpr::Assumption => "Assumption",
        TacticExpr::Reflexivity => "Reflexivity",
        TacticExpr::Intro(_) => "Intro",
        TacticExpr::Apply { .. } => "Apply",
        TacticExpr::Rewrite { .. } => "Rewrite",
        TacticExpr::Simp { .. } => "Simp",
        TacticExpr::Ring => "Ring",
        TacticExpr::Field => "Field",
        TacticExpr::Omega => "Omega",
        TacticExpr::Auto { .. } => "Auto",
        TacticExpr::Blast => "Blast",
        TacticExpr::Smt { .. } => "Smt",
        TacticExpr::Split => "Split",
        TacticExpr::Left => "Left",
        TacticExpr::Right => "Right",
        TacticExpr::Exists(_) => "Exists",
        TacticExpr::CasesOn(_) => "CasesOn",
        TacticExpr::InductionOn(_) => "InductionOn",
        TacticExpr::Exact(_) => "Exact",
        TacticExpr::Unfold(_) => "Unfold",
        TacticExpr::Compute => "Compute",
        TacticExpr::Try(_) => "Try",
        TacticExpr::TryElse { .. } => "TryElse",
        TacticExpr::Repeat(_) => "Repeat",
        TacticExpr::Seq(_) => "Seq",
        TacticExpr::Alt(_) => "Alt",
        TacticExpr::AllGoals(_) => "AllGoals",
        TacticExpr::Focus(_) => "Focus",
        TacticExpr::Named { .. } => "Named",
        TacticExpr::Let { .. } => "Let",
        _ => "<unknown>",
    }
}

/// **Elaborate a proof body to a kernel `Term`.**
///
///   - `ProofBody::Tactic(t)` — delegates to [`elaborate_tactic`].
///   - `ProofBody::Term(e)` — direct Curry-Howard proof term;
///     delegates to [`expr_to_term`].  Handles the
///     `proof = lemma_name(args)` syntax where the user writes a
///     constructive witness directly without tactic wrapping.
///
/// `Structured` and `ByMethod` proof bodies are not yet handled
/// and return [`ElabError::UnsupportedTactic`].
pub fn elaborate_proof_body(
    body: &ProofBody,
    ctx: &mut ElabContext,
) -> Result<Term, ElabError> {
    match body {
        ProofBody::Tactic(t) => elaborate_tactic(t, ctx),
        ProofBody::Term(e) => expr_to_term(e, ctx),
        ProofBody::Structured(_) => Err(ElabError::UnsupportedTactic(
            "ProofBody::Structured".into(),
        )),
        ProofBody::ByMethod(_) => Err(ElabError::UnsupportedTactic(
            "ProofBody::ByMethod".into(),
        )),
    }
}

/// **Elaborate a complete theorem.**  Top-level entry point.
///
/// 1. Verify the theorem has a proof body (else
///    [`ElabError::NoProofBody`]).
/// 2. Elaborate the proof body to a `Term` via
///    [`elaborate_proof_body`].
/// 3. Build a [`crate::verification_goal::VerificationGoal`] from
///    the theorem.  The goal's `to_term()` is the Pi-chain over
///    hypotheses with the proposition as conclusion — that is the
///    certificate's claimed type.  On translation failure (e.g.,
///    a quantifier or pattern-match in the proposition), fall back
///    to [`placeholder_proposition`] so the elaborator still
///    produces a (weakly load-bearing) certificate.
/// 4. [`close_over_axioms`] wraps body and claimed type in a
///    matching `Lam` / `Pi` chain over the axiom registry.
/// 5. [`finalise_certificate`] re-verifies via the kernel checker —
///    the load-bearing step that enforces the de Bruijn criterion.
///
/// The certificate's metadata records `claimed_type_source:
/// verification_goal` when the unified converter handled the
/// proposition or `placeholder` when it fell back, so JSON
/// consumers can downgrade trust accordingly.
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

    let (body_type, prop_translation_status) =
        match from_theorem_decl(theorem, TheoremKind::Theorem, ctx) {
            Ok(goal) => (goal.to_term(), "verification_goal"),
            Err(_) => (placeholder_proposition(), "placeholder"),
        };

    let (closed_term, closed_type) = close_over_axioms(ctx, body_term, body_type);
    let mut metadata = BTreeMap::new();
    metadata.insert("theorem_name".to_string(), theorem.name.name.to_string());
    metadata.insert("kernel_version".to_string(), crate::VVA_VERSION.to_string());
    metadata.insert(
        "claimed_type_source".to_string(),
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

/// **Translate a Verum `Expr` to a kernel `Term`.**
///
///   - `Path(name)` — resolves via [`resolve_apply_target`].
///   - `Field(obj, field)` — composes path name then resolves.
///   - `Call(f, args)` — recursive translation + `App` chain.
///
/// Other expression forms (literals, conditionals, lambda
/// expressions, type-level constructs) return
/// [`ElabError::UnsupportedExpression`] with the variant tag.
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
            "expression form not supported: ExprKind::{}",
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
// Tests — contract pins for the elaborator surface
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

    // ----- AST-integration tests -----

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
        // ProofBody::Term(expr) — direct Curry-Howard term.
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
    fn elaborate_tactic_intro_single_binder_produces_lam() {
        // `intro x` with one binder produces `λx. Var(0)` —
        // identity-like at the witness level.
        let mut ctx = ElabContext::new();
        let span = Span::dummy();
        let mut idents = List::new();
        idents.push(Ident { name: "x".into(), span });
        let term = elaborate_tactic(&TacticExpr::Intro(idents), &mut ctx).unwrap();
        assert_eq!(
            term,
            Term::Lam(
                Box::new(Term::Universe(0)),
                Box::new(Term::Var(0)),
            ),
        );
        // Context restored — Intro doesn't leak binders.
        assert_eq!(ctx.depth(), 0);
    }

    #[test]
    fn elaborate_tactic_intro_multi_binder_produces_lam_chain() {
        // `intro x y z` produces `λ. λ. λ. Var(0)` (innermost).
        let mut ctx = ElabContext::new();
        let span = Span::dummy();
        let mut idents = List::new();
        for n in &["x", "y", "z"] {
            idents.push(Ident { name: (*n).into(), span });
        }
        let term = elaborate_tactic(&TacticExpr::Intro(idents), &mut ctx).unwrap();
        assert_eq!(
            term,
            Term::Lam(
                Box::new(Term::Universe(0)),
                Box::new(Term::Lam(
                    Box::new(Term::Universe(0)),
                    Box::new(Term::Lam(
                        Box::new(Term::Universe(0)),
                        Box::new(Term::Var(0)),
                    )),
                )),
            ),
        );
        assert_eq!(ctx.depth(), 0, "Intro pops all binders before returning");
    }

    #[test]
    fn elaborate_tactic_seq_intro_then_apply_uses_introduced_binder() {
        // `intro x; apply x` — the witness is the just-introduced
        // binder.  Term shape: `λx. Var(0)`.
        let mut ctx = ElabContext::new();
        let span = Span::dummy();
        let mut idents = List::new();
        idents.push(Ident { name: "x".into(), span });
        let mut steps = List::new();
        steps.push(TacticExpr::Intro(idents));
        steps.push(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("x")),
            args: List::new(),
        });
        let term = elaborate_tactic(&TacticExpr::Seq(steps), &mut ctx).unwrap();
        assert_eq!(
            term,
            Term::Lam(
                Box::new(Term::Universe(0)),
                Box::new(Term::Var(0)),
            ),
        );
        assert_eq!(ctx.depth(), 0, "Seq restores context after binder pop");
    }

    #[test]
    fn elaborate_tactic_seq_intermediate_non_intro_rejected() {
        // `apply foo; apply bar` — the intermediate step is Apply,
        // not Intro.  Reject with a diagnostic naming the offending
        // step.
        let mut ctx = ElabContext::new();
        ctx.register_axiom("foo", Term::Universe(0));
        ctx.register_axiom("bar", Term::Universe(0));
        let mut steps = List::new();
        steps.push(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("foo")),
            args: List::new(),
        });
        steps.push(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("bar")),
            args: List::new(),
        });
        match elaborate_tactic(&TacticExpr::Seq(steps), &mut ctx) {
            Err(ElabError::UnsupportedTactic(msg)) => {
                assert!(
                    msg.contains("Apply"),
                    "diagnostic should name the offending tactic: {}",
                    msg,
                );
            }
            other => panic!("expected UnsupportedTactic, got {:?}", other),
        }
    }

    #[test]
    fn elaborate_tactic_seq_empty_rejected() {
        let mut ctx = ElabContext::new();
        match elaborate_tactic(&TacticExpr::Seq(List::new()), &mut ctx) {
            Err(ElabError::UnsupportedTactic(msg)) => {
                assert!(msg.contains("length 0"), "got: {}", msg);
            }
            other => panic!("expected UnsupportedTactic, got {:?}", other),
        }
    }

    #[test]
    fn elaborate_tactic_seq_single_step_works() {
        // A single Apply wrapped in Seq behaves like the Apply alone.
        let mut ctx = ElabContext::new();
        ctx.register_axiom("witness", Term::Universe(0));
        let mut steps = List::new();
        steps.push(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr("witness")),
            args: List::new(),
        });
        let term = elaborate_tactic(&TacticExpr::Seq(steps), &mut ctx).unwrap();
        assert_eq!(term, Term::Var(0));
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
        // The boolean-literal proposition `true` translates to
        // Universe(0); the body `apply foo` produces Var(0) with
        // type Universe(0) — they match, so the kernel accepts.
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
        // Metadata records whether the claimed type came from the
        // unified VerificationGoal path or from the placeholder
        // fallback.
        assert_eq!(
            cert.metadata.get("claimed_type_source").map(|s| s.as_str()),
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
        // Binary `a == b` translates via the registered Eq
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
        // Block expressions aren't propositional connectives.
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
    fn register_kernel_v0_lemmas_covers_all_five_iou_stubs() {
        let mut ctx = ElabContext::new();
        register_kernel_v0_lemmas(&mut ctx);
        for name in [
            "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
            "core.verify.kernel_v0.lemmas.beta.church_rosser_confluence",
            "core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi",
            "core.verify.kernel_v0.lemmas.eta.function_extensionality",
            "core.verify.kernel_v0.lemmas.sub.cumulative_universe_inclusion",
        ] {
            assert!(
                ctx.get_axiom(name).is_some(),
                "kernel_v0 lemma stub `{}` must be registered",
                name,
            );
        }
    }

    #[test]
    fn register_kernel_bridge_dispatchers_covers_all_ten_rules() {
        let mut ctx = ElabContext::new();
        register_kernel_bridge_dispatchers(&mut ctx);
        for name in [
            "kernel_var_strict",
            "kernel_universe_intro_strict",
            "kernel_forward_axiom_strict",
            "kernel_positivity_strict",
            "kernel_pi_form_strict",
            "kernel_lam_intro_strict",
            "kernel_app_elim_strict",
            "kernel_beta_strict",
            "kernel_eta_strict",
            "kernel_sub_strict",
        ] {
            assert!(
                ctx.get_axiom(name).is_some(),
                "kernel bridge dispatcher `{}` must be registered",
                name,
            );
        }
    }

    #[test]
    fn elaborate_proof_with_kernel_v0_lemma_apply_succeeds() {
        // A theorem that applies one of the kernel_v0 lemma stubs
        // resolves cleanly when register_kernel_v0_lemmas is called.
        use verum_ast::decl::TheoremDecl;
        use verum_ast::Literal;
        let span = Span::dummy();
        let true_prop = Expr::new(
            ExprKind::Literal(Literal::bool(true, span)),
            span,
        );
        let mut theorem = TheoremDecl::new(
            Ident { name: "uses_church_rosser".into(), span },
            true_prop,
            span,
        );
        theorem.proof = verum_common::Maybe::Some(ProofBody::Tactic(TacticExpr::Apply {
            lemma: verum_common::Heap::new(path_expr_dotted(&[
                "core",
                "verify",
                "kernel_v0",
                "lemmas",
                "beta",
                "church_rosser_confluence",
            ])),
            args: List::new(),
        }));

        let mut ctx = ElabContext::new();
        register_kernel_v0_lemmas(&mut ctx);

        let cert = elaborate_theorem(&theorem, &mut ctx).unwrap();
        cert.verify().unwrap();
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
            cert.metadata.get("claimed_type_source").map(|s| s.as_str()),
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
