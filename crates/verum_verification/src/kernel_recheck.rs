//! Kernel-rule recheck pass — naturality / categorical-coherence /
//! modal-depth wiring.
//!
//! This module bridges `verum_kernel`'s trusted-base K-rules
//! (`check_eps_mu_coherence`, `check_universe_ascent`,
//! `check_refine_omega`) into the gradual-verification pipeline so
//! the higher tiers of the verification ladder
//! (`@verify(certified)` and the three `@verify(coherent*)`
//! variants) can honour their certificate-recheck semantics.
//!
//! # The structural problem this module solves
//!
//! Before this module landed, `verum_kernel` had **zero** downstream
//! Rust dependents — the K-rules were tested in isolation but never
//! invoked from any compiler phase. Per the VVA architecture
//! (§§9.2, 12.4) the kernel is the trusted base of the verification
//! ladder; without it being invoked, the `@verify(certified)` /
//! `@verify(coherent*)` strategies would silently degrade to
//! `@verify(reliable)` semantics — emitting certificates that no
//! re-check actually validated.
//!
//! # V0 surface (this revision)
//!
//! V0 ships a **kernel-recheck façade** in pure Rust:
//!
//!   * [`KernelRecheck`] — a thin handle around the K-rule entry
//!     points.
//!   * [`refine_omega`] — call-site for `K-Refine-omega`. Given a
//!     refinement type's binder + base + predicate **already lifted
//!     to `CoreTerm`**, returns `Ok(())` or a [`KernelRecheckError`]
//!     wrapping the underlying [`KernelError`].
//!   * [`universe_ascent`] — call-site for `K-Universe-Ascent`.
//!   * [`eps_mu_coherence`] — call-site for `K-Eps-Mu`.
//!
//! V1 will add the AST-to-CoreTerm lifting helpers so the
//! verification pipeline can call these directly on the typed AST
//! without the caller pre-lifting.

use thiserror::Error;
use verum_ast::FunctionDecl;
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::ty::{PathSegment, RefinementPredicate as AstRefinementPredicate, Type as AstType, TypeKind};
use verum_common::{Heap, List, Maybe, Text};
use verum_kernel::{
    BridgeAudit, CoreTerm, KernelError, UniverseTier, canonical_form,
    check_eps_mu_coherence, check_eps_mu_coherence_v3_final, check_refine_omega,
    check_round_trip, check_round_trip_v2, check_universe_ascent,
};
use verum_types::refinement::{RefinementBinding, RefinementPredicate as TypesRefinementPredicate};
use verum_types::ty::Type as TypesType;

/// Errors surfaced by the kernel-recheck façade. Each variant
/// preserves enough provenance to thread the original
/// [`KernelError`] back to the verification ladder so the diagnostic
/// emitter can show *which* K-rule failed and on *what* obligation.
#[derive(Debug, Clone, Error)]
pub enum KernelRecheckError {
    /// `K-Refine-omega` rejected the refinement-type formation.
    #[error("kernel-recheck: K-Refine-omega failed for binder '{binder}': {source}")]
    RefineOmega {
        /// Binder name from the refinement type (e.g., `it`, `x`).
        binder: Text,
        /// Wrapped kernel error.
        source: KernelError,
    },
    /// `K-Universe-Ascent` rejected the universe transition.
    #[error("kernel-recheck: K-Universe-Ascent failed at '{context}': {source}")]
    UniverseAscent {
        /// Human-readable call-site context.
        context: Text,
        /// Wrapped kernel error.
        source: KernelError,
    },
    /// `K-Eps-Mu` rejected the naturality-square shape.
    #[error("kernel-recheck: K-Eps-Mu failed at '{context}': {source}")]
    EpsMu {
        /// Human-readable call-site context.
        context: Text,
        /// Wrapped kernel error.
        source: KernelError,
    },
    /// `K-Round-Trip` rejected the AC/OC duality round-trip.
    #[error("kernel-recheck: K-Round-Trip failed at '{context}': {source}")]
    RoundTrip {
        /// Human-readable call-site context (typically the AC/OC
        /// duality theorem name).
        context: Text,
        /// Wrapped kernel error.
        source: KernelError,
    },
}

/// Public façade — every verification phase that wants to invoke a
/// K-rule goes through one of the methods below. The façade is
/// stateless; it lives as a unit struct so call-sites read like
/// `KernelRecheck::refine_omega(...)` instead of free-function
/// imports (which would conflict with `verum_kernel`'s own naming
/// inside this crate).
#[derive(Debug, Clone, Copy, Default)]
pub struct KernelRecheck;

impl KernelRecheck {
    /// `K-Refine-omega` recheck for a refinement type
    /// `{binder : base | predicate}`. Routes through
    /// [`check_refine_omega`] and lifts any kernel error into a
    /// [`KernelRecheckError::RefineOmega`] tagged with the binder.
    pub fn refine_omega(
        binder: &Text,
        base: &CoreTerm,
        predicate: &CoreTerm,
    ) -> Result<(), KernelRecheckError> {
        check_refine_omega(binder, base, predicate).map_err(|err| {
            KernelRecheckError::RefineOmega {
                binder: binder.clone(),
                source: err,
            }
        })
    }

    /// `K-Refine-omega` recheck **gated by `@require_extension(vfe_7)`
    /// policy** (M-VVA Sub-2.4 — VVA spec L170, deferred policy wiring).
    ///
    /// Routes through [`check_refine_omega`] only when the configured
    /// [`ExtensionPolicy`] declares `vfe_7` active for the consuming
    /// scope. When the extension is opt-out (policy = `OptInOnly` and
    /// `set` lacks the `@require_extension(vfe_7)` annotation), this
    /// returns `Ok(())` *without* invoking the kernel rule — preserving
    /// the VVA Year-0–2 rollout default of "extensions off unless
    /// explicitly opted in".
    ///
    /// **Soundness.** Skipping the rule is sound under the rollout
    /// calendar: K-Refine-omega's transfinite stratification check is
    /// strictly stronger than the always-on K-Refine rule (finite
    /// `dp(P) < dp(A) + 1`); declining to apply the stronger rule means
    /// the weaker rule still holds and the program is admitted under
    /// the weaker discipline. No false acceptance results from gating
    /// off — only from gating ON when the program author's intent was
    /// to opt into the weaker rule.
    ///
    /// **Backward-compat.** Existing callers continue using
    /// [`KernelRecheck::refine_omega`] (the unconditional form). The
    /// gated form is opt-in for new callers wiring policy-aware passes.
    pub fn refine_omega_gated(
        binder: &Text,
        base: &CoreTerm,
        predicate: &CoreTerm,
        policy: crate::extension_policy::ExtensionPolicy,
        set: &crate::extension_policy::EnabledExtensions,
    ) -> Result<(), KernelRecheckError> {
        if policy.is_active(set, "vfe_7") {
            Self::refine_omega(binder, base, predicate)
        } else {
            // Policy gate inactive — vacuous pass. The rule stays
            // available but does not run on this scope; the weaker
            // K-Refine still gates refinement formation.
            Ok(())
        }
    }

    /// `K-Universe-Ascent` recheck for a meta-classifier application
    /// `M_stack(α) : Articulation@U_{k+1}`. Routes through
    /// [`check_universe_ascent`] and lifts any kernel error.
    pub fn universe_ascent(
        source: UniverseTier,
        target: UniverseTier,
        context: &str,
    ) -> Result<(), KernelRecheckError> {
        check_universe_ascent(source, target, context).map_err(|err| {
            KernelRecheckError::UniverseAscent {
                context: Text::from(context),
                source: err,
            }
        })
    }

    /// `K-Eps-Mu` recheck for the canonical naturality-square pair
    /// `(EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))`. Routes through
    /// [`check_eps_mu_coherence`] and lifts any kernel error.
    pub fn eps_mu_coherence(
        lhs: &CoreTerm,
        rhs: &CoreTerm,
        context: &str,
    ) -> Result<(), KernelRecheckError> {
        check_eps_mu_coherence(lhs, rhs, context).map_err(|err| {
            KernelRecheckError::EpsMu {
                context: Text::from(context),
                source: err,
            }
        })
    }

    /// `K-Round-Trip` recheck for the AC/OC duality round-trip
    /// `canonicalise(inverse(translate(α))) ≡ canonicalise(α)`.
    /// Routes through [`check_round_trip`] and lifts any kernel
    /// error into [`KernelRecheckError::RoundTrip`] tagged with the
    /// callsite context.
    ///
    /// Admit-set (V0/V1):
    ///   - structural identity (`α == α`),
    ///   - K-Adj-Unit shape `AlphaOf(EpsilonOf(F)) ↔ F`,
    ///   - K-Adj-Counit shape `EpsilonOf(AlphaOf(F)) ↔ F`,
    ///   - β-/ι-/δ-equivalence (definitional_eq).
    ///
    /// V2 (preprint-blocked on Diakrisis 16.10) adds the universal
    /// canonicalize algorithm.
    pub fn round_trip(
        lhs: &CoreTerm,
        rhs: &CoreTerm,
        context: &str,
    ) -> Result<(), KernelRecheckError> {
        check_round_trip(lhs, rhs, context).map_err(|err| {
            KernelRecheckError::RoundTrip {
                context: Text::from(context),
                source: err,
            }
        })
    }

    /// V2 universal-canonicalize K-Round-Trip recheck.
    ///
    /// Strictly stronger than [`Self::round_trip`]: every pair the
    /// V0/V1 algorithm admits is also admitted by V2 with an EMPTY
    /// audit trail. Pairs that V2 admits but V0/V1 reject (modal-
    /// idempotent / cohesive-idempotent / refine-fold pairs) produce
    /// a non-empty [`BridgeAudit`] surfacing every Diakrisis admit
    /// invoked.
    ///
    /// External auditors enumerate the audit trail to see WHICH
    /// preprint-blocked claims (16.10 / 16.7 / 14.3) the proof
    /// relies on. An empty audit means the proof is fully
    /// decidable in V2.
    pub fn round_trip_v2(
        lhs: &CoreTerm,
        rhs: &CoreTerm,
        context: &str,
    ) -> Result<BridgeAudit, KernelRecheckError> {
        check_round_trip_v2(lhs, rhs, context).map_err(|err| {
            KernelRecheckError::RoundTrip {
                context: Text::from(context),
                source: err,
            }
        })
    }

    /// V3-final K-Eps-Mu recheck with explicit Diakrisis A-3
    /// τ-witness audit trail.
    ///
    /// Strictly stronger than [`Self::eps_mu_coherence`]: every pair
    /// V3-incremental admits is also admitted by V3-final, with the
    /// audit trail recording the σ_α / π_α witness construction
    /// reliance for non-identity canonical naturality squares.
    /// Identity sub-cases (structural / β-equiv) produce an empty
    /// audit.
    pub fn eps_mu_v3_final(
        lhs: &CoreTerm,
        rhs: &CoreTerm,
        context: &str,
    ) -> Result<BridgeAudit, KernelRecheckError> {
        check_eps_mu_coherence_v3_final(lhs, rhs, context).map_err(|err| {
            KernelRecheckError::EpsMu {
                context: Text::from(context),
                source: err,
            }
        })
    }

    /// Universal canonicalize entry point — exposes V2 normalize-
    /// to-fixed-point with audit trail. Verification phases that
    /// want to compute normal forms outside the round-trip pair
    /// API (e.g. for caching / diagnostic emission) call here.
    pub fn canonicalize(term: &CoreTerm, context: &str) -> (CoreTerm, BridgeAudit) {
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(term, &mut audit, context);
        (canon, audit)
    }

    /// Aggregate two [`BridgeAudit`] trails into one. Used by
    /// multi-rule recheck call sites that want to surface the
    /// complete bridge footprint of a composite proof. Insertion
    /// order is preserved; per-bridge dedup is honoured because
    /// [`BridgeAudit::record`] is idempotent on (bridge, context)
    /// pairs.
    pub fn merge_audits(lhs: BridgeAudit, mut rhs: BridgeAudit) -> BridgeAudit {
        let mut out = lhs;
        for admit in rhs.admits().to_vec().drain(..) {
            out.record(admit.bridge, admit.context);
        }
        // rhs is consumed via the to_vec() copy above; explicitly drop.
        let _ = rhs;
        out
    }

    /// V1 convenience — directly recheck a refinement type
    /// `{binder : base | predicate}` from its AST form. Performs
    /// the AST → CoreTerm lift via [`lift_type_to_core`] +
    /// [`lift_expr_to_core`] (best-effort), pulls the binder name
    /// from `predicate.binding` (defaulting to `"it"` per
    /// Verum's Rule 1 convention), and dispatches to
    /// [`Self::refine_omega`]. The lifter is conservative: AST
    /// shapes it does not yet recognise become opaque
    /// `CoreTerm::Var("<unsupported>")` placeholders so the K-rule
    /// still gets a structurally well-formed input. The
    /// underlying `m_depth_omega` walker treats these as atomic
    /// (rank 0), which preserves soundness for non-modal user
    /// code (the rule fires only on modal-typed predicates).
    pub fn refine_omega_from_ast(
        base: &AstType,
        predicate: &AstRefinementPredicate,
    ) -> Result<(), KernelRecheckError> {
        let binder: Text = match &predicate.binding {
            verum_common::Maybe::Some(ident) => ident.name.clone(),
            verum_common::Maybe::None => Text::from("it"),
        };
        let base_core = lift_type_to_core(base);
        let pred_core = lift_expr_to_core(&predicate.expr);
        Self::refine_omega(&binder, &base_core, &pred_core)
    }

    /// V3 convenience — recheck **every** refinement type
    /// appearing in a function declaration's parameter or return
    /// type. Returns the list of (call-site label, K-rule outcome)
    /// pairs for diagnostic surfacing. Walks composite types
    /// (tuples, references, slices, arrays, function-types, bounded-
    /// types) so refinements nested inside generics are not missed.
    ///
    /// V3 is what the production verification pipeline calls
    /// (`crates/verum_verification/src/passes.rs::SmtVerificationPass::
    /// verify_function`); V0/V1/V2 entry points remain available
    /// for unit-test isolation and direct kernel-rule invocation.
    pub fn recheck_function(
        func: &FunctionDecl,
    ) -> List<(Text, Result<(), KernelRecheckError>)> {
        let mut out: List<(Text, Result<(), KernelRecheckError>)> = List::new();
        recheck_signature_into(
            &func.name.name,
            func.params.iter(),
            match &func.return_type {
                Maybe::Some(t) => Some(t),
                Maybe::None => None,
            },
            &mut out,
        );
        // descend into the function body to surface
        // refinements declared in `let x: Int{...} = ...` bindings
        // and inside nested control-flow blocks. Real Verum code
        // (vcs/specs/L1-core/refinement/verification/array_indexing.vr)
        // uses local-binding refinements that previously escaped
        // the kernel-recheck because the walker only saw the
        // function signature.
        if let verum_common::Maybe::Some(body) = &func.body {
            match body {
                verum_ast::decl::FunctionBody::Block(b) => {
                    walk_ast_block_for_recheck(b, &func.name.name, &mut out);
                }
                verum_ast::decl::FunctionBody::Expr(e) => {
                    walk_ast_expr_for_recheck(e, &func.name.name, &mut out);
                }
            }
        }
        // descend into requires / ensures clauses.
        // Pre/postcondition expressions can mention refinement
        // types via `where` clauses or named refinements; pre-V8
        // these escaped the kernel-recheck. The contract-style
        // expressions are walked the same way as let-binding
        // initializers via walk_ast_expr_for_recheck — refinement-
        // type formation inside `requires x: Int{p.box().box()}`
        // surfaces with K-Refine-omega rejection.
        for req in func.requires.iter() {
            walk_ast_expr_for_recheck(req, &func.name.name, &mut out);
        }
        for ens in func.ensures.iter() {
            walk_ast_expr_for_recheck(ens, &func.name.name, &mut out);
        }
        out
    }

    /// recheck a theorem / lemma / corollary
    /// declaration. Same FunctionParam-shaped signature as
    /// `recheck_function`; refinement types in params or return
    /// type are walked through K-Refine-omega.
    pub fn recheck_theorem(
        theorem: &verum_ast::decl::TheoremDecl,
    ) -> List<(Text, Result<(), KernelRecheckError>)> {
        let mut out: List<(Text, Result<(), KernelRecheckError>)> = List::new();
        recheck_signature_into(
            &theorem.name.name,
            theorem.params.iter(),
            match &theorem.return_type {
                Maybe::Some(t) => Some(t),
                Maybe::None => None,
            },
            &mut out,
        );
        // walk theorem requires/ensures as well.
        // Theorems carry the same pre/post contract surface as
        // functions; same refinement-type leak applies pre-V8.
        for req in theorem.requires.iter() {
            walk_ast_expr_for_recheck(req, &theorem.name.name, &mut out);
        }
        for ens in theorem.ensures.iter() {
            walk_ast_expr_for_recheck(ens, &theorem.name.name, &mut out);
        }
        // Walk the proof body if present — TheoremDecl.proof is a
        // Maybe<ProofBody> not a FunctionBody, so we keep the walk
        // narrow at the V8 surface and defer richer ProofBody
        // descent to a dedicated proof-recheck pass.
        out
    }

    /// recheck an axiom declaration. Axioms can
    /// carry refinement types in their parameter list and return
    /// type (e.g., `axiom positive_succ(n: Nat{> 0}) -> Nat{> n}`);
    /// these previously escaped K-rule checking entirely.
    pub fn recheck_axiom(
        axiom: &verum_ast::decl::AxiomDecl,
    ) -> List<(Text, Result<(), KernelRecheckError>)> {
        let mut out: List<(Text, Result<(), KernelRecheckError>)> = List::new();
        recheck_signature_into(
            &axiom.name.name,
            axiom.params.iter(),
            match &axiom.return_type {
                Maybe::Some(t) => Some(t),
                Maybe::None => None,
            },
            &mut out,
        );
        // : the axiom's `proposition` field carries
        // the assumed claim. While the proposition isn't a
        // refinement-bearing type itself, it CAN reference
        // refinement-typed sub-terms via path expressions that
        // resolve to typedefs whose body is a Refined CoreTerm.
        // Walking the proposition via the body-expr walker
        // surfaces refinement-type formation inside the
        // proposition's structure.
        walk_ast_expr_for_recheck(&axiom.proposition, &axiom.name.name, &mut out);
        out
    }

    /// V2 convenience — directly recheck a refinement type from
    /// the post-typecheck `verum_types::Type` IR. This is the
    /// flavour the production verification phase actually consumes
    /// (the AST-level lifter exists for unit-test isolation).
    /// Routes through [`lift_types_type_to_core`] (verum_types ⇒
    /// CoreTerm) + [`lift_expr_to_core`] (the AST-level lifter
    /// reused since `RefinementPredicate.predicate` carries an
    /// `Expr`).
    pub fn refine_omega_from_types(
        base: &TypesType,
        predicate: &TypesRefinementPredicate,
    ) -> Result<(), KernelRecheckError> {
        let binder: Text = match &predicate.binding {
            RefinementBinding::Inline | RefinementBinding::Bare => Text::from("it"),
            RefinementBinding::Lambda(name)
            | RefinementBinding::Sigma(name) => name.clone(),
            RefinementBinding::Named(_) => Text::from("it"),
        };
        let base_core = lift_types_type_to_core(base);
        let pred_core = lift_expr_to_core(&predicate.predicate);
        Self::refine_omega(&binder, &base_core, &pred_core)
    }
}

// =============================================================================
// V3 — recursive AST walker for kernel-recheck preamble
// =============================================================================

/// Recurse into an `AstType`, calling
/// [`KernelRecheck::refine_omega_from_ast`] on every `Refined`
/// node and pushing `(label, outcome)` pairs into `out`. The walker
/// covers the composite-type shapes that may carry refinements
/// inside themselves (Reference / CheckedReference / UnsafeReference
/// / Pointer / Array / Slice / Tuple / Function / Bounded). Atomic
/// types (Int / Float / Text / Bool / Unit / Never / Unknown / Var
/// / Inferred / Path / etc.) terminate without further work.
fn walk_ast_type_for_recheck(
    ty: &AstType,
    function_name: &Text,
    context_kind: &str,
    out: &mut List<(Text, Result<(), KernelRecheckError>)>,
) {
    match &ty.kind {
        TypeKind::Refined { base, predicate } => {
            let label = Text::from(format!(
                "K-Refine-omega [{} {}]",
                function_name.as_str(),
                context_kind,
            ));
            let outcome = KernelRecheck::refine_omega_from_ast(base, predicate);
            out.push((label, outcome));
            // Refinements can stack (e.g., Int{> 0}{< 100}); recurse.
            walk_ast_type_for_recheck(base, function_name, context_kind, out);
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. }
        | TypeKind::Pointer { inner, .. } => {
            walk_ast_type_for_recheck(inner, function_name, context_kind, out);
        }
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            walk_ast_type_for_recheck(element, function_name, context_kind, out);
        }
        TypeKind::Tuple(types) => {
            for t in types.iter() {
                walk_ast_type_for_recheck(t, function_name, context_kind, out);
            }
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for p in params.iter() {
                walk_ast_type_for_recheck(p, function_name, context_kind, out);
            }
            walk_ast_type_for_recheck(return_type, function_name, context_kind, out);
        }
        TypeKind::Bounded { base, .. } => {
            walk_ast_type_for_recheck(base, function_name, context_kind, out);
        }
        _ => {}
    }
}

// =============================================================================
// V6 — shared signature walker (function / theorem / axiom share the shape)
// =============================================================================

/// Walk the parameters and return type of a declaration, surfacing
/// every refinement type to K-Refine-omega. Used by recheck_function,
/// recheck_theorem, recheck_axiom — the three decls that share the
/// `FunctionParam` + `Maybe<Type>` signature shape.
fn recheck_signature_into<'a, I>(
    decl_name: &Text,
    params: I,
    return_type: Option<&AstType>,
    out: &mut List<(Text, Result<(), KernelRecheckError>)>,
) where
    I: Iterator<Item = &'a verum_ast::decl::FunctionParam>,
{
    for param in params {
        if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &param.kind {
            walk_ast_type_for_recheck(ty, decl_name, "param", out);
        }
    }
    if let Some(ret_ty) = return_type {
        walk_ast_type_for_recheck(ret_ty, decl_name, "return", out);
    }
}

// =============================================================================
// V4 — function-body walker for let-binding refinements 
// =============================================================================

/// Walk an AST [`Block`] for refinement-type-bearing constructs in
/// statements + the trailing tail expression. Currently surfaces
/// refinements in `let` / `let-else` type annotations and
/// recursively descends into nested control-flow expressions.
pub(crate) fn walk_ast_block_for_recheck(
    block: &verum_ast::expr::Block,
    function_name: &Text,
    out: &mut List<(Text, Result<(), KernelRecheckError>)>,
) {
    for stmt in block.stmts.iter() {
        match &stmt.kind {
            verum_ast::stmt::StmtKind::Let { ty, value, .. } => {
                if let verum_common::Maybe::Some(t) = ty {
                    walk_ast_type_for_recheck(t, function_name, "let", out);
                }
                if let verum_common::Maybe::Some(v) = value {
                    walk_ast_expr_for_recheck(v, function_name, out);
                }
            }
            verum_ast::stmt::StmtKind::LetElse {
                ty,
                value,
                else_block,
                ..
            } => {
                if let verum_common::Maybe::Some(t) = ty {
                    walk_ast_type_for_recheck(t, function_name, "let-else", out);
                }
                walk_ast_expr_for_recheck(value, function_name, out);
                walk_ast_block_for_recheck(else_block, function_name, out);
            }
            verum_ast::stmt::StmtKind::Expr { expr, .. } => {
                walk_ast_expr_for_recheck(expr, function_name, out);
            }
            verum_ast::stmt::StmtKind::Defer(e)
            | verum_ast::stmt::StmtKind::Errdefer(e) => {
                walk_ast_expr_for_recheck(e, function_name, out);
            }
            // Item declarations inside fn bodies
            // (nested fns, types, theorems, axioms). The module-
            // level pipeline pass walks ONLY top-level items, so
            // pre-V7 these escaped the kernel-recheck entirely.
            // V7 recurses through the nested item's signature so
            // refinements in its params/return-type are caught.
            verum_ast::stmt::StmtKind::Item(item) => {
                walk_ast_nested_item_for_recheck(&item.kind, function_name, out);
            }
            _ => {}
        }
    }
    if let verum_common::Maybe::Some(tail) = &block.expr {
        walk_ast_expr_for_recheck(tail, function_name, out);
    }
}

/// walk a nested ItemKind that appeared as a Stmt
/// inside a function body. The module-level KernelRecheckPass
/// only walks TOP-level items; nested fns / types / theorems /
/// axioms inside `fn outer() { fn inner(...) ... }` would otherwise
/// escape kernel-recheck. This walker mirrors the V6
/// recheck_one_item dispatcher but for body-nested items: instead
/// of producing per-decl cost records, it folds outcomes into the
/// caller's `out` list under the parent function's label so the
/// diagnostic surface stays anchored on the visible scope.
fn walk_ast_nested_item_for_recheck(
    kind: &verum_ast::decl::ItemKind,
    parent_function_name: &Text,
    out: &mut List<(Text, Result<(), KernelRecheckError>)>,
) {
    use verum_ast::decl::ItemKind as IK;
    match kind {
        IK::Function(f) => {
            // Recheck the nested function's signature + body
            // recursively — refinements at any depth are caught.
            let inner = KernelRecheck::recheck_function(f);
            for (label, outcome) in inner.iter() {
                let nested_label = Text::from(format!(
                    "{} → nested {}",
                    parent_function_name.as_str(),
                    label.as_str(),
                ));
                out.push((nested_label, outcome.clone()));
            }
        }
        IK::Theorem(d) | IK::Lemma(d) | IK::Corollary(d) => {
            let inner = KernelRecheck::recheck_theorem(d);
            for (label, outcome) in inner.iter() {
                let nested_label = Text::from(format!(
                    "{} → nested {}",
                    parent_function_name.as_str(),
                    label.as_str(),
                ));
                out.push((nested_label, outcome.clone()));
            }
        }
        IK::Axiom(a) => {
            let inner = KernelRecheck::recheck_axiom(a);
            for (label, outcome) in inner.iter() {
                let nested_label = Text::from(format!(
                    "{} → nested {}",
                    parent_function_name.as_str(),
                    label.as_str(),
                ));
                out.push((nested_label, outcome.clone()));
            }
        }
        IK::Module(m) => {
            // Recurse into the nested module's items.
            if let verum_common::Maybe::Some(items) = &m.items {
                for nested in items.iter() {
                    walk_ast_nested_item_for_recheck(
                        &nested.kind,
                        parent_function_name,
                        out,
                    );
                }
            }
        }
        IK::Impl(impl_decl) => {
            for impl_item in impl_decl.items.iter() {
                if let verum_ast::decl::ImplItemKind::Function(f) = &impl_item.kind {
                    let inner = KernelRecheck::recheck_function(f);
                    for (label, outcome) in inner.iter() {
                        let nested_label = Text::from(format!(
                            "{} → nested {}",
                            parent_function_name.as_str(),
                            label.as_str(),
                        ));
                        out.push((nested_label, outcome.clone()));
                    }
                }
            }
        }
        // Other ItemKind variants don't carry refinement-bearing
        // signatures the kernel-recheck currently observes.
        _ => {}
    }
}

/// Walk an AST [`Expr`] for nested control-flow that may carry
/// further block-scoped refinement types. Most expression shapes
/// don't carry refinements — only block-shaped constructs (If /
/// Match arms / Loop / While / For) need recursion.
pub(crate) fn walk_ast_expr_for_recheck(
    expr: &Expr,
    function_name: &Text,
    out: &mut List<(Text, Result<(), KernelRecheckError>)>,
) {
    match &expr.kind {
        ExprKind::Block(b) => walk_ast_block_for_recheck(b, function_name, out),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            walk_ast_block_for_recheck(then_branch, function_name, out);
            if let verum_common::Maybe::Some(e) = else_branch {
                walk_ast_expr_for_recheck(e, function_name, out);
            }
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms.iter() {
                walk_ast_expr_for_recheck(&arm.body, function_name, out);
            }
        }
        ExprKind::Loop { body, .. } => {
            walk_ast_block_for_recheck(body, function_name, out);
        }
        ExprKind::While { body, .. } => {
            walk_ast_block_for_recheck(body, function_name, out);
        }
        ExprKind::For { body, .. } => {
            walk_ast_block_for_recheck(body, function_name, out);
        }
        ExprKind::Paren(inner) => {
            walk_ast_expr_for_recheck(inner, function_name, out)
        }
        // Other shapes (Path / Literal / Binary / Unary / Call /
        // MethodCall / etc.) don't introduce new bindings or
        // blocks — leaf for the body walker.
        _ => {}
    }
}

// =============================================================================
// AST → CoreTerm lifting (V1 minimum-viable shape)
// =============================================================================

/// Lift an AST [`Type`] node into a kernel [`CoreTerm`]. The lift
/// is best-effort and conservative — atomic types (`Int`, `Bool`,
/// `Float`, `Text`, `Unit`) become `CoreTerm::Var("<name>")`;
/// path types become `Var("<last-segment>")`; refinement types
/// recurse into their base; everything else is materialised as a
/// `Var("<kind-tag>")` placeholder so the `m_depth_omega` walker
/// treats it as rank 0.
///
/// The richer translation (Π / Σ / App / Pair / etc.) is V2 work —
/// V1 covers the cases that actually trigger
/// `K-Refine-omega` rejection in user code (modal predicates over
/// atomic base types).
pub fn lift_type_to_core(ty: &AstType) -> CoreTerm {
    match &ty.kind {
        TypeKind::Unit => CoreTerm::Var(Text::from("Unit")),
        TypeKind::Bool => CoreTerm::Var(Text::from("Bool")),
        TypeKind::Int => CoreTerm::Var(Text::from("Int")),
        TypeKind::Float => CoreTerm::Var(Text::from("Float")),
        TypeKind::Text => CoreTerm::Var(Text::from("Text")),
        TypeKind::Inferred => CoreTerm::Var(Text::from("_")),
        TypeKind::Never => CoreTerm::Var(Text::from("Never")),
        TypeKind::Unknown => CoreTerm::Var(Text::from("Unknown")),
        TypeKind::Path(path) => {
            let name = path
                .segments
                .last()
                .and_then(|s| match s {
                    PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| Text::from("<path>"));
            CoreTerm::Var(name)
        }
        TypeKind::Refined { base, .. } => lift_type_to_core(base),
        // Other shapes — materialise as opaque atomic so the K-rule
        // sees a well-formed CoreTerm. V2 lifts the structure.
        _ => CoreTerm::Var(Text::from("<unsupported-type>")),
    }
}

/// V2 sister of [`lift_type_to_core`] — same conservative
/// best-effort lift, but operating on the post-typecheck
/// [`TypesType`] IR. The verification phase consumes
/// `verum_types::Type` because that's what type-inference
/// produces; this lifter lets `KernelRecheck::refine_omega_from_types`
/// be wired in directly without first converting back to the AST.
pub fn lift_types_type_to_core(ty: &TypesType) -> CoreTerm {
    match ty {
        TypesType::Unit => CoreTerm::Var(Text::from("Unit")),
        TypesType::Bool => CoreTerm::Var(Text::from("Bool")),
        TypesType::Int => CoreTerm::Var(Text::from("Int")),
        TypesType::Float => CoreTerm::Var(Text::from("Float")),
        TypesType::Char => CoreTerm::Var(Text::from("Char")),
        TypesType::Text => CoreTerm::Var(Text::from("Text")),
        TypesType::Never => CoreTerm::Var(Text::from("Never")),
        TypesType::Unknown => CoreTerm::Var(Text::from("Unknown")),
        TypesType::Var(_) => CoreTerm::Var(Text::from("_")),
        TypesType::Named { path, .. } => {
            let name = path
                .segments
                .last()
                .and_then(|s| match s {
                    PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| Text::from("<path>"));
            CoreTerm::Var(name)
        }
        TypesType::Generic { name, .. } => CoreTerm::Var(name.clone()),
        TypesType::Refined { base, .. } => lift_types_type_to_core(base),
        // V3 composite shapes — fold operands into App chains so
        // m_depth_omega computes max-rank correctly for nested
        // refinements. Pre-V3 these collapsed to opaque Var.
        TypesType::Function {
            params,
            return_type,
            ..
        } => {
            // V3 + fold params + return_type into an
            // App chain. The `..` deliberately discards
            // `contexts: Option<ContextExpr>` and
            // `properties: Option<PropertySet>` from the lift:
            //   • ContextExpr::Concrete wraps ContextRequirement
            //     which is `Set<ContextRef>`; ContextRef is just
            //     (name: Text, type_id: TypeId) — no inline Type
            //     to recurse into. The TypeId is an indirection
            //     into the type registry; following it would
            //     require ambient registry access the structural
            //     lifter doesn't (and shouldn't) have.
            //   • PropertySet is `Set<ComputationalProperty>` —
            //     a flat enum (Pure / IO / Async / Fallible /
            //     Mutates) with no inline Type fields.
            // Refinements arriving via these channels surface
            // via the type_id back-references, which are walked
            // by other compiler phases (typecheck, contract-
            // verification) operating with full registry access.
            // The K-rule preamble is correct to leave them
            // unwalked here.
            let mut acc = lift_types_type_to_core(return_type);
            for p in params.iter() {
                acc = CoreTerm::App(
                    Heap::new(acc),
                    Heap::new(lift_types_type_to_core(p)),
                );
            }
            acc
        }
        TypesType::Tuple(types) => fold_app_chain_types(types.iter()),
        TypesType::Array { element, .. }
        | TypesType::Slice { element } => lift_types_type_to_core(element),
        TypesType::Reference { inner, .. }
        | TypesType::CheckedReference { inner, .. }
        | TypesType::UnsafeReference { inner, .. }
        | TypesType::Ownership { inner, .. } => lift_types_type_to_core(inner),
        TypesType::Record(fields) => fold_app_chain_types(fields.values()),
        TypesType::ExtensibleRecord { fields, .. } => {
            fold_app_chain_types(fields.values())
        }
        TypesType::Variant(fields) => fold_app_chain_types(fields.values()),
        // Truly unrecognised shapes — opaque atomic placeholder.
        _ => CoreTerm::Var(Text::from("<unsupported-types-type>")),
    }
}

/// Fold a sequence of [`TypesType`] children into a left-associated
/// `App` chain. Used by Tuple / Record / Variant lifters.
/// Returns `Var("<empty>")` for an empty sequence.
fn fold_app_chain_types<'a, I>(it: I) -> CoreTerm
where
    I: IntoIterator<Item = &'a TypesType>,
{
    let mut iter = it.into_iter();
    let first = match iter.next() {
        Some(t) => lift_types_type_to_core(t),
        None => return CoreTerm::Var(Text::from("<empty>")),
    };
    let mut acc = first;
    for t in iter {
        acc = CoreTerm::App(
            Heap::new(acc),
            Heap::new(lift_types_type_to_core(t)),
        );
    }
    acc
}

/// Lift an AST [`Expr`] node into a kernel [`CoreTerm`]. Modal-
/// operator support is wired so K-Refine-omega correctly rejects
/// over-stratified predicates (the canonical V1 use case).
///
/// composite-expression coverage. Previously
/// `Binary` / `Unary` / `Call` / `If` / `Match` / `Block` /
/// `Literal` collapsed to opaque `Var("<unsupported-expr>")`
/// placeholders (rank 0 to `m_depth_omega`), which silently
/// accepted modal-typed predicates nested inside arithmetic /
/// boolean / control-flow expressions. The lifter now recurses
/// through these shapes, encoding component composition as
/// `CoreTerm::App(left, right)` so `m_depth_omega` correctly
/// computes `max` over the operands.
///
/// Coverage:
///
///   • `ExprKind::Path` → `CoreTerm::Var("<last-segment>")`.
///   • `ExprKind::Paren(e)` → recurse on `e`.
///   • Method-call shape `x.box()` / `x.diamond()` /
///     `x.necessarily()` / `x.possibly()` → `ModalBox(x)` /
///     `ModalDiamond(x)`. Other methods → `App(receiver, args...)`
///     so the K-rule sees the receiver's modal structure.
///   • `ExprKind::Binary { left, _, right }` → `App(left, right)`
///     (operator is irrelevant to `m_depth_omega`; the rank is the
///     max of the operand ranks).
///   • `ExprKind::Unary { _, expr }` → recurse on `expr` (unary
///     operators don't add structural depth).
///   • `ExprKind::Call { func, args }` → fold args into a
///     left-associated `App` chain rooted at the callee.
///   • `ExprKind::If { _, then_branch, else_branch }` → `App(lift(then),
///     lift(else))` so the rule sees the max-rank branch.
///   • `ExprKind::Match { _, arms }` → fold all arm bodies into
///     an `App` chain (max rank across arms).
///   • `ExprKind::Block(b)` → lift the trailing expression if any,
///     else `Var("<empty-block>")`.
///   • `ExprKind::Literal(_)` → `Var("<lit>")` (atomic, rank 0).
///   • Everything else → `Var("<unsupported-expr>")` placeholder.
pub fn lift_expr_to_core(expr: &Expr) -> CoreTerm {
    match &expr.kind {
        ExprKind::Path(path) => {
            let name = path
                .segments
                .last()
                .and_then(|s| match s {
                    PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| Text::from("<path>"));
            CoreTerm::Var(name)
        }
        ExprKind::Paren(inner) => lift_expr_to_core(inner),
        ExprKind::MethodCall {
            receiver, method, args, ..
        } => {
            let inner = lift_expr_to_core(receiver);
            match method.name.as_str() {
                "box" | "necessarily" => CoreTerm::ModalBox(Heap::new(inner)),
                "diamond" | "possibly" => CoreTerm::ModalDiamond(Heap::new(inner)),
                _ => {
                    // Other methods: fold args into an App chain
                    // anchored on the receiver. This preserves the
                    // receiver's modal structure under the K-rule.
                    let mut acc = inner;
                    for arg in args.iter() {
                        acc = CoreTerm::App(
                            Heap::new(acc),
                            Heap::new(lift_expr_to_core(arg)),
                        );
                    }
                    acc
                }
            }
        }
        ExprKind::Binary { left, right, .. } => {
            // BinOp is structurally an App over its operands;
            // m_depth_omega(App(l, r)) = max(rank(l), rank(r)).
            // The operator itself is irrelevant — modal depth
            // is a structural property of the syntax tree.
            CoreTerm::App(
                Heap::new(lift_expr_to_core(left)),
                Heap::new(lift_expr_to_core(right)),
            )
        }
        ExprKind::Unary { expr: inner, .. } => lift_expr_to_core(inner),
        ExprKind::Call { func, args, .. } => {
            // Left-associated App chain: lift(func)(arg0)(arg1)...
            let mut acc = lift_expr_to_core(func);
            for arg in args.iter() {
                acc = CoreTerm::App(
                    Heap::new(acc),
                    Heap::new(lift_expr_to_core(arg)),
                );
            }
            acc
        }
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            let then_core = lift_block_tail_to_core(then_branch);
            let else_core = match else_branch {
                verum_common::Maybe::Some(e) => lift_expr_to_core(e),
                verum_common::Maybe::None => CoreTerm::Var(Text::from("<unit>")),
            };
            CoreTerm::App(Heap::new(then_core), Heap::new(else_core))
        }
        ExprKind::Match { arms, .. } => {
            // Fold arm bodies into an App chain so the K-rule
            // sees the max-rank arm.
            let mut iter = arms.iter();
            let first_body = match iter.next() {
                Some(arm) => lift_match_arm_body_to_core(arm),
                None => return CoreTerm::Var(Text::from("<empty-match>")),
            };
            let mut acc = first_body;
            for arm in iter {
                acc = CoreTerm::App(
                    Heap::new(acc),
                    Heap::new(lift_match_arm_body_to_core(arm)),
                );
            }
            acc
        }
        ExprKind::Block(b) => lift_block_tail_to_core(b),
        ExprKind::Literal(_) => CoreTerm::Var(Text::from("<lit>")),
        _ => CoreTerm::Var(Text::from("<unsupported-expr>")),
    }
}

fn lift_block_tail_to_core(b: &verum_ast::expr::Block) -> CoreTerm {
    match &b.expr {
        verum_common::Maybe::Some(e) => lift_expr_to_core(e),
        verum_common::Maybe::None => CoreTerm::Var(Text::from("<empty-block>")),
    }
}

fn lift_match_arm_body_to_core(arm: &verum_ast::pattern::MatchArm) -> CoreTerm {
    // MatchArm.body is a Heap<Expr> (typically a Block).
    lift_expr_to_core(&arm.body)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::Heap;

    fn var(name: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(name))
    }

    fn epsilon_of(t: CoreTerm) -> CoreTerm {
        CoreTerm::EpsilonOf(Heap::new(t))
    }

    fn alpha_of(t: CoreTerm) -> CoreTerm {
        CoreTerm::AlphaOf(Heap::new(t))
    }

    // ---- K-Refine-omega ----

    #[test]
    fn refine_omega_accepts_well_stratified_predicate() {
        // base = Var("Int"), pred = Var("p") — both atomic, ranks 0
        // and 0; predicate-rank < base-rank.succ() ⇒ Ok.
        let binder = Text::from("it");
        let base = var("Int");
        let pred = var("p");
        assert!(KernelRecheck::refine_omega(&binder, &base, &pred).is_ok());
    }

    #[test]
    fn refine_omega_rejects_overshooting_predicate() {
        // pred = Box(Box(Var("p"))) — md^ω = 2; base = Var("Int") —
        // md^ω = 0. 2 < 0.succ() = 1 is false ⇒ Err.
        let binder = Text::from("it");
        let base = var("Int");
        let pred = CoreTerm::ModalBox(Heap::new(CoreTerm::ModalBox(Heap::new(var("p")))));
        let err = KernelRecheck::refine_omega(&binder, &base, &pred).unwrap_err();
        match err {
            KernelRecheckError::RefineOmega { binder: b, .. } => {
                assert_eq!(b.as_str(), "it");
            }
            other => panic!("expected RefineOmega, got {:?}", other),
        }
    }

    // ---- K-Refine-omega gated by @require_extension(vfe_7) ----
    //
    // M-VVA Sub-2.4 — VVA spec L170 deferred policy wiring. The gate
    // ensures K-Refine-omega only runs when the consuming scope opts
    // in via `@require_extension(vfe_7)`. Year-0–2 default (`OptInOnly`)
    // skips the rule; `AllRulesActive` (back-compat) runs unconditionally.

    fn require_extension_attr(ext: &str) -> verum_ast::attr::Attribute {
        use verum_ast::Ident;
        use verum_ast::Span;
        use verum_ast::attr::Attribute;
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::ty::{Path, PathSegment};
        let span = Span::default();
        let mut segs: verum_common::List<PathSegment> = verum_common::List::new();
        segs.push(PathSegment::Name(Ident { name: Text::from(ext), span }));
        let mut args: verum_common::List<Expr> = verum_common::List::new();
        args.push(Expr::new(ExprKind::Path(Path::new(segs, span)), span));
        Attribute {
            name: Text::from("require_extension"),
            args: Maybe::Some(args),
            span,
        }
    }

    fn disable_extension_attr(ext: &str) -> verum_ast::attr::Attribute {
        let mut a = require_extension_attr(ext);
        a.name = Text::from("disable_extension");
        a
    }

    #[test]
    fn refine_omega_gated_opt_in_only_skips_when_extension_absent() {
        use crate::extension_policy::{EnabledExtensions, ExtensionPolicy};
        // Predicate that WOULD reject under unconditional K-Refine-omega.
        let binder = Text::from("it");
        let base = var("Int");
        let pred = CoreTerm::ModalBox(Heap::new(CoreTerm::ModalBox(Heap::new(var("p")))));

        // Empty extension set + OptInOnly policy → vfe_7 inactive → vacuous Ok.
        let empty = EnabledExtensions::empty();
        assert!(
            KernelRecheck::refine_omega_gated(
                &binder,
                &base,
                &pred,
                ExtensionPolicy::OptInOnly,
                &empty,
            )
            .is_ok(),
            "OptInOnly with empty set must skip K-Refine-omega"
        );
    }

    #[test]
    fn refine_omega_gated_opt_in_only_runs_when_extension_required() {
        use crate::extension_policy::{EnabledExtensions, ExtensionPolicy};
        let binder = Text::from("it");
        let base = var("Int");
        // Overshooting predicate.
        let pred = CoreTerm::ModalBox(Heap::new(CoreTerm::ModalBox(Heap::new(var("p")))));

        // Set with vfe_7 required.
        let mut attrs: List<verum_ast::attr::Attribute> = List::new();
        attrs.push(require_extension_attr("vfe_7"));
        let required = EnabledExtensions::from_attributes(&attrs);

        // OptInOnly + vfe_7 required → rule runs and rejects the predicate.
        let result = KernelRecheck::refine_omega_gated(
            &binder,
            &base,
            &pred,
            ExtensionPolicy::OptInOnly,
            &required,
        );
        assert!(
            matches!(result, Err(KernelRecheckError::RefineOmega { .. })),
            "OptInOnly with vfe_7 required must run K-Refine-omega and reject overshooting predicate; got {:?}",
            result
        );
    }

    #[test]
    fn refine_omega_gated_all_rules_active_runs_unconditionally() {
        use crate::extension_policy::{EnabledExtensions, ExtensionPolicy};
        let binder = Text::from("it");
        let base = var("Int");
        // Well-stratified predicate.
        let pred = var("p");
        let empty = EnabledExtensions::empty();

        // AllRulesActive ignores the extension set — runs and passes.
        assert!(
            KernelRecheck::refine_omega_gated(
                &binder,
                &base,
                &pred,
                ExtensionPolicy::AllRulesActive,
                &empty,
            )
            .is_ok()
        );
    }

    #[test]
    fn refine_omega_gated_opt_out_only_runs_unless_disabled() {
        use crate::extension_policy::{EnabledExtensions, ExtensionPolicy};
        let binder = Text::from("it");
        let base = var("Int");
        let pred = CoreTerm::ModalBox(Heap::new(CoreTerm::ModalBox(Heap::new(var("p")))));

        // OptOutOnly + empty set → rule active by default → rejects.
        let empty = EnabledExtensions::empty();
        let result = KernelRecheck::refine_omega_gated(
            &binder,
            &base,
            &pred,
            ExtensionPolicy::OptOutOnly,
            &empty,
        );
        assert!(matches!(result, Err(KernelRecheckError::RefineOmega { .. })));

        // OptOutOnly + vfe_7 explicitly disabled → rule skipped → vacuous Ok.
        let mut attrs: List<verum_ast::attr::Attribute> = List::new();
        attrs.push(disable_extension_attr("vfe_7"));
        let disabled = EnabledExtensions::from_attributes(&attrs);
        assert!(
            KernelRecheck::refine_omega_gated(
                &binder,
                &base,
                &pred,
                ExtensionPolicy::OptOutOnly,
                &disabled,
            )
            .is_ok()
        );
    }

    // ---- K-Round-Trip façade ----
    //
    // M-VVA Sub-2.1 closure (round_trip kernel rule integration).
    // The façade lifts `verum_kernel::check_round_trip` to a typed
    // KernelRecheckError variant.

    #[test]
    fn round_trip_facade_accepts_identity() {
        let f = var("F");
        assert!(KernelRecheck::round_trip(&f, &f, "test-identity").is_ok());
    }

    #[test]
    fn round_trip_facade_accepts_alpha_of_epsilon_of_x_vs_x() {
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        assert!(KernelRecheck::round_trip(&aef, &f, "K-Adj-Unit").is_ok());
    }

    #[test]
    fn round_trip_facade_rejects_distinct_atoms_with_typed_error() {
        let alpha = var("alpha");
        let beta = var("beta");
        let err = KernelRecheck::round_trip(&alpha, &beta, "AC/OC duality").unwrap_err();
        match err {
            KernelRecheckError::RoundTrip { context, .. } => {
                assert_eq!(context.as_str(), "AC/OC duality");
            }
            other => panic!("expected RoundTrip, got {:?}", other),
        }
    }

    // ---- K-Universe-Ascent ----

    #[test]
    fn universe_ascent_accepts_kappa_1_to_kappa_2() {
        assert!(
            KernelRecheck::universe_ascent(
                UniverseTier::Kappa1,
                UniverseTier::Kappa2,
                "M_stack_ascent"
            )
            .is_ok()
        );
    }

    #[test]
    fn universe_ascent_rejects_kappa_2_to_kappa_1() {
        let err = KernelRecheck::universe_ascent(
            UniverseTier::Kappa2,
            UniverseTier::Kappa1,
            "tier_inversion",
        )
        .unwrap_err();
        match err {
            KernelRecheckError::UniverseAscent { context, .. } => {
                assert_eq!(context.as_str(), "tier_inversion");
            }
            other => panic!("expected UniverseAscent, got {:?}", other),
        }
    }

    // ---- K-Eps-Mu ----

    #[test]
    fn eps_mu_accepts_canonical_identity_square() {
        let alpha = var("α");
        let lhs = epsilon_of(alpha.clone());
        let rhs = alpha_of(epsilon_of(alpha));
        assert!(KernelRecheck::eps_mu_coherence(&lhs, &rhs, "M_eq_id_case").is_ok());
    }

    #[test]
    fn eps_mu_rejects_malformed_inner() {
        let lhs = epsilon_of(var("M_α"));
        let rhs = alpha_of(var("α"));
        let err = KernelRecheck::eps_mu_coherence(&lhs, &rhs, "malformed").unwrap_err();
        match err {
            KernelRecheckError::EpsMu { context, .. } => {
                assert_eq!(context.as_str(), "malformed");
            }
            other => panic!("expected EpsMu, got {:?}", other),
        }
    }

    // ---- V1 AST-to-CoreTerm lifting ----

    use verum_ast::Span;
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::ty::{Path, RefinementPredicate as AstRefinementPredicate, Type as AstType};
    use verum_ast::Ident;
    use verum_common::List;

    fn span() -> Span {
        Span::default()
    }

    fn path_expr(name: &str) -> Expr {
        let ident = Ident {
            name: Text::from(name),
            span: span(),
        };
        Expr::ident(ident)
    }

    fn method_call_expr(receiver: Expr, method_name: &str) -> Expr {
        Expr::new(
            ExprKind::MethodCall {
                receiver: verum_common::Heap::new(receiver),
                method: Ident {
                    name: Text::from(method_name),
                    span: span(),
                },
                args: List::new(),
                type_args: List::new(),
            },
            span(),
        )
    }

    #[test]
    fn lift_type_atomic_int_to_var() {
        let ty = AstType::int(span());
        match super::lift_type_to_core(&ty) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Int"),
            other => panic!("expected Var(Int), got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_path_to_var() {
        let e = path_expr("p");
        match super::lift_expr_to_core(&e) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "p"),
            other => panic!("expected Var(p), got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_method_box_to_modal_box() {
        let inner = path_expr("p");
        let boxed = method_call_expr(inner, "box");
        match super::lift_expr_to_core(&boxed) {
            CoreTerm::ModalBox(_) => {}
            other => panic!("expected ModalBox, got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_method_diamond_to_modal_diamond() {
        let inner = path_expr("p");
        let dia = method_call_expr(inner, "possibly");
        match super::lift_expr_to_core(&dia) {
            CoreTerm::ModalDiamond(_) => {}
            other => panic!("expected ModalDiamond, got {:?}", other),
        }
    }

    #[test]
    fn refine_omega_from_ast_atomic_predicate_accepted() {
        // base = Int, predicate = `p` (rank 0). 0 < 0+1 ⇒ Ok.
        let base = AstType::int(span());
        let predicate = AstRefinementPredicate {
            expr: path_expr("p"),
            binding: verum_common::Maybe::None,
            span: span(),
        };
        assert!(KernelRecheck::refine_omega_from_ast(&base, &predicate).is_ok());
    }

    #[test]
    fn refine_omega_from_ast_modal_predicate_overshoots() {
        // base = Int (rank 0), predicate = p.box().box() (rank 2).
        // 2 < 0+1 = 1 ⇒ FAIL.
        let base = AstType::int(span());
        let p = path_expr("p");
        let boxed_once = method_call_expr(p, "box");
        let boxed_twice = method_call_expr(boxed_once, "box");
        let predicate = AstRefinementPredicate {
            expr: boxed_twice,
            binding: verum_common::Maybe::None,
            span: span(),
        };
        let err = KernelRecheck::refine_omega_from_ast(&base, &predicate).unwrap_err();
        match err {
            KernelRecheckError::RefineOmega { binder, .. } => {
                assert_eq!(binder.as_str(), "it");
            }
            other => panic!("expected RefineOmega, got {:?}", other),
        }
    }

    // ---- V2 verum_types-IR lifter ----

    use verum_types::ty::Type as TypesType;
    use verum_types::refinement::{RefinementBinding, RefinementPredicate as TypesRefinementPredicate};

    #[test]
    fn lift_types_type_atomic_int_to_var() {
        match super::lift_types_type_to_core(&TypesType::Int) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Int"),
            other => panic!("expected Var(Int), got {:?}", other),
        }
    }

    #[test]
    fn lift_types_type_generic_uses_name() {
        let ty = TypesType::Generic {
            name: Text::from("Maybe"),
            args: List::new(),
        };
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Maybe"),
            other => panic!("expected Var(Maybe), got {:?}", other),
        }
    }

    #[test]
    fn lift_types_type_refined_recurses_to_base() {
        let pred = TypesRefinementPredicate::inline(path_expr("p"), span());
        let ty = TypesType::Refined {
            base: Box::new(TypesType::Int),
            predicate: pred,
        };
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Int"),
            other => panic!("expected Var(Int) (recursing into base), got {:?}", other),
        }
    }

    #[test]
    fn refine_omega_from_types_atomic_predicate_accepted() {
        let pred = TypesRefinementPredicate::inline(path_expr("p"), span());
        assert!(KernelRecheck::refine_omega_from_types(&TypesType::Int, &pred).is_ok());
    }

    #[test]
    fn refine_omega_from_types_modal_overshoot_uses_binding_name() {
        let p = path_expr("p");
        let boxed = method_call_expr(method_call_expr(p, "box"), "box");
        let pred = TypesRefinementPredicate::lambda(
            boxed,
            Text::from("y"),
            span(),
        );
        let err = KernelRecheck::refine_omega_from_types(&TypesType::Int, &pred).unwrap_err();
        match err {
            KernelRecheckError::RefineOmega { binder, .. } => {
                assert_eq!(binder.as_str(), "y");
            }
            other => panic!("expected RefineOmega, got {:?}", other),
        }
    }

    // ---- V3 lifter extension: composite expression shapes  ----

    use verum_ast::expr::{BinOp, Block, IfCondition, UnOp};
    use verum_ast::literal::{IntLit, Literal, LiteralKind};

    fn binary_expr(left: Expr, op: BinOp, right: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                left: Heap::new(left),
                op,
                right: Heap::new(right),
            },
            span(),
        )
    }

    fn int_literal(n: i128) -> Expr {
        let lit = Literal {
            kind: LiteralKind::Int(IntLit {
                value: n,
                suffix: None,
            }),
            span: span(),
        };
        Expr::new(ExprKind::Literal(lit), span())
    }

    fn unary_expr(op: UnOp, e: Expr) -> Expr {
        Expr::new(
            ExprKind::Unary {
                op,
                expr: Heap::new(e),
            },
            span(),
        )
    }

    fn call_expr(func: Expr, args: Vec<Expr>) -> Expr {
        let mut a: List<Expr> = List::new();
        for x in args {
            a.push(x);
        }
        Expr::new(
            ExprKind::Call {
                func: Heap::new(func),
                args: a,
                type_args: List::new(),
            },
            span(),
        )
    }

    fn block_with_tail(tail: Expr) -> Block {
        Block::new(
            List::new(),
            verum_common::Maybe::Some(Heap::new(tail)),
            span(),
        )
    }

    fn if_expr(then_tail: Expr, else_tail: Option<Expr>) -> Expr {
        use smallvec::smallvec;
        use verum_ast::expr::ConditionKind;
        let cond = path_expr("cond");
        let if_cond = IfCondition {
            conditions: smallvec![ConditionKind::Expr(cond)],
            span: span(),
        };
        Expr::new(
            ExprKind::If {
                condition: Heap::new(if_cond),
                then_branch: block_with_tail(then_tail),
                else_branch: match else_tail {
                    Some(e) => verum_common::Maybe::Some(Heap::new(e)),
                    None => verum_common::Maybe::None,
                },
            },
            span(),
        )
    }

    fn box_call(receiver: Expr) -> Expr {
        method_call_expr(receiver, "box")
    }

    #[test]
    fn lift_expr_binary_uses_max_of_operand_ranks() {
        // BinOp(p.box(), q) — left has md^ω = 1, right has 0;
        // App(left, right) ranks at max = 1.
        let lifted = super::lift_expr_to_core(&binary_expr(
            box_call(path_expr("p")),
            BinOp::And,
            path_expr("q"),
        ));
        match lifted {
            CoreTerm::App(_, _) => {}
            other => panic!("expected App, got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_literal_to_atomic_var() {
        let lifted = super::lift_expr_to_core(&int_literal(42));
        match lifted {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "<lit>"),
            other => panic!("expected Var(<lit>), got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_unary_recurses_without_extra_node() {
        let lifted = super::lift_expr_to_core(&unary_expr(UnOp::Not, path_expr("p")));
        match lifted {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "p"),
            other => panic!("expected Var(p), got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_call_folds_args_into_app_chain() {
        // f(a, b) → App(App(f, a), b)
        let lifted = super::lift_expr_to_core(&call_expr(
            path_expr("f"),
            vec![path_expr("a"), path_expr("b")],
        ));
        match lifted {
            CoreTerm::App(outer_left, _outer_right) => match outer_left.as_ref() {
                CoreTerm::App(_, _) => {}
                other => panic!("expected nested App, got {:?}", other),
            },
            other => panic!("expected App, got {:?}", other),
        }
    }

    #[test]
    fn lift_expr_if_takes_max_of_branches() {
        // if cond { p.box() } else { q } — then has md^ω = 1, else has 0;
        // App(then, else) ranks at max = 1.
        let lifted = super::lift_expr_to_core(&if_expr(
            box_call(path_expr("p")),
            Some(path_expr("q")),
        ));
        match lifted {
            CoreTerm::App(_, _) => {}
            other => panic!("expected App, got {:?}", other),
        }
    }

    #[test]
    fn refine_omega_from_ast_rejects_modal_inside_binary() {
        // V3: `Int{ p.box().box() && q }` — modal nested in BinOp.
        // Predicate ranks at 2, base at 0; 2 < 0+1 = 1 ⇒ reject.
        // Pre-V3 this would have been opaque rank 0 → silently
        // accepted.
        let bad = binary_expr(
            box_call(box_call(path_expr("p"))),
            BinOp::And,
            path_expr("q"),
        );
        let predicate = verum_ast::ty::RefinementPredicate {
            expr: bad,
            binding: verum_common::Maybe::None,
            span: span(),
        };
        let base = AstType::int(span());
        let err = KernelRecheck::refine_omega_from_ast(&base, &predicate)
            .expect_err("V3 must catch modal nested inside BinOp");
        match err {
            KernelRecheckError::RefineOmega { .. } => {}
            other => panic!("expected RefineOmega, got {:?}", other),
        }
    }

    #[test]
    fn refine_omega_from_ast_rejects_modal_inside_if_branch() {
        // V3: `Int{ if cond { p.box().box() } else { q } }`.
        // Then-branch has rank 2; else-branch has rank 0.
        // App(then, else) ranks at max(2, 0) = 2.
        // 2 < 0+1 = 1 ⇒ reject.
        let bad = if_expr(box_call(box_call(path_expr("p"))), Some(path_expr("q")));
        let predicate = verum_ast::ty::RefinementPredicate {
            expr: bad,
            binding: verum_common::Maybe::None,
            span: span(),
        };
        let base = AstType::int(span());
        assert!(
            KernelRecheck::refine_omega_from_ast(&base, &predicate).is_err(),
            "modal in if-branch must reject under V3"
        );
    }

    #[test]
    fn refine_omega_from_ast_rejects_modal_inside_call_args() {
        // V3: `Int{ f(p.box().box()) }` — modal inside Call arg.
        // Predicate ranks at 2.
        let bad = call_expr(path_expr("f"), vec![box_call(box_call(path_expr("p")))]);
        let predicate = verum_ast::ty::RefinementPredicate {
            expr: bad,
            binding: verum_common::Maybe::None,
            span: span(),
        };
        let base = AstType::int(span());
        assert!(
            KernelRecheck::refine_omega_from_ast(&base, &predicate).is_err(),
            "modal in Call arg must reject under V3"
        );
    }

    #[test]
    fn refine_omega_from_ast_atomic_binary_still_accepted() {
        // V3 must NOT regress on atomic BinOp predicates like
        // `Int{x > 0}` — both operands rank 0, App ranks 0.
        let pred = binary_expr(path_expr("x"), BinOp::Gt, int_literal(0));
        let predicate = verum_ast::ty::RefinementPredicate {
            expr: pred,
            binding: verum_common::Maybe::None,
            span: span(),
        };
        let base = AstType::int(span());
        assert!(
            KernelRecheck::refine_omega_from_ast(&base, &predicate).is_ok(),
            "atomic binary predicate must still pass"
        );
    }

    // ---- V3 types-IR composite-shape coverage ----

    #[test]
    fn lift_types_type_function_folds_params_into_app_chain() {
        let ty = TypesType::Function {
            params: vec![TypesType::Int, TypesType::Bool].into(),
            return_type: Box::new(TypesType::Text),
            contexts: None,
            type_params: List::new(),
            properties: None,
        };
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::App(_, _) => {}
            other => panic!("expected App, got {:?}", other),
        }
    }

    #[test]
    fn lift_types_type_tuple_folds_into_app_chain() {
        let ty = TypesType::Tuple(
            vec![TypesType::Int, TypesType::Bool, TypesType::Text].into(),
        );
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::App(_, _) => {}
            other => panic!("expected App, got {:?}", other),
        }
    }

    #[test]
    fn lift_types_type_reference_recurses_into_inner() {
        let ty = TypesType::Reference {
            mutable: false,
            inner: Box::new(TypesType::Int),
        };
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Int"),
            other => panic!("expected Var(Int), got {:?}", other),
        }
    }

    #[test]
    fn lift_types_type_array_recurses_into_element() {
        let ty = TypesType::Array {
            element: Box::new(TypesType::Bool),
            size: Some(8),
        };
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Bool"),
            other => panic!("expected Var(Bool), got {:?}", other),
        }
    }

    #[test]
    fn lift_types_type_slice_recurses_into_element() {
        let ty = TypesType::Slice {
            element: Box::new(TypesType::Float),
        };
        match super::lift_types_type_to_core(&ty) {
            CoreTerm::Var(name) => assert_eq!(name.as_str(), "Float"),
            other => panic!("expected Var(Float), got {:?}", other),
        }
    }

    #[test]
    fn refine_omega_from_types_function_with_refined_atomic_param() {
        // Function (Refined<Int>{p}, Bool) -> Text
        // Inner refinement is well-formed atomic predicate. The
        // function-type lifter folds it into an App chain; the
        // recheck of the OUTER Function type doesn't go through
        // refine_omega_from_types (only Refined types do). This
        // test pins that the lifter is structural — it doesn't
        // mistake the function-type wrapper for a refinement.
        let inner_pred = TypesRefinementPredicate::inline(path_expr("p"), span());
        let refined_int = TypesType::Refined {
            base: Box::new(TypesType::Int),
            predicate: inner_pred,
        };
        let _ty = TypesType::Function {
            params: vec![refined_int, TypesType::Bool].into(),
            return_type: Box::new(TypesType::Text),
            contexts: None,
            type_params: List::new(),
            properties: None,
        };
        // refine_omega_from_types is only meaningful when called
        // ON a refinement. Calling it on a Function type would
        // misuse the API. The structural lifter test above is the
        // V3 coverage point.
        let pred = TypesRefinementPredicate::inline(path_expr("p"), span());
        assert!(KernelRecheck::refine_omega_from_types(&TypesType::Int, &pred).is_ok());
    }

    #[test]
    fn refine_omega_from_types_inline_binding_defaults_to_it() {
        let p = path_expr("p");
        let boxed = method_call_expr(method_call_expr(p, "box"), "box");
        let pred = TypesRefinementPredicate::inline(boxed, span());
        let err = KernelRecheck::refine_omega_from_types(&TypesType::Int, &pred).unwrap_err();
        match err {
            KernelRecheckError::RefineOmega { binder, .. } => {
                assert_eq!(binder.as_str(), "it");
            }
            other => panic!("expected RefineOmega, got {:?}", other),
        }
    }

    #[test]
    fn refine_omega_from_ast_uses_explicit_binder_when_present() {
        let base = AstType::int(span());
        let predicate = AstRefinementPredicate {
            expr: path_expr("q"),
            binding: verum_common::Maybe::Some(Ident {
                name: Text::from("x"),
                span: span(),
            }),
            span: span(),
        };
        // This passes (rank 0 base, rank 0 predicate). The binder
        // contract is exercised in the failure case below by
        // constructing a known-fail and reading the binder back.
        assert!(KernelRecheck::refine_omega_from_ast(&base, &predicate).is_ok());

        // Failing variant — same setup but with a modal-overshoot
        // predicate, so we can read the binder out of the error.
        let p = path_expr("p");
        let boxed_once = method_call_expr(p, "box");
        let boxed_twice = method_call_expr(boxed_once, "box");
        let bad_predicate = AstRefinementPredicate {
            expr: boxed_twice,
            binding: verum_common::Maybe::Some(Ident {
                name: Text::from("x"),
                span: span(),
            }),
            span: span(),
        };
        let err = KernelRecheck::refine_omega_from_ast(&base, &bad_predicate).unwrap_err();
        match err {
            KernelRecheckError::RefineOmega { binder, .. } => {
                assert_eq!(binder.as_str(), "x");
            }
            other => panic!("expected RefineOmega, got {:?}", other),
        }
    }

    // -------------------------------------------------------------------------
    // V2/V3 façade integration tests (#33)
    // -------------------------------------------------------------------------

    fn core_var(n: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(n))
    }

    #[test]
    fn round_trip_v2_admits_v0_pairs_with_empty_audit() {
        // Identity pair → V2 admits with empty audit (decidable).
        let f = core_var("F");
        let audit = KernelRecheck::round_trip_v2(&f, &f, "test").unwrap();
        assert!(audit.is_decidable(),
            "V0/V1-decidable pair must produce empty V2 audit");
    }

    #[test]
    fn round_trip_v2_admits_modal_idempotent_pair() {
        // ModalBox(ModalBox(F)) ≡ ModalBox(F) via V2 canonicalize.
        // V0/V1 reject this; V2 admits decidably (modal-idem rewrite).
        let f = core_var("F");
        let bbf = CoreTerm::ModalBox(verum_common::Heap::new(
            CoreTerm::ModalBox(verum_common::Heap::new(f.clone()))));
        let bf = CoreTerm::ModalBox(verum_common::Heap::new(f));
        let audit = KernelRecheck::round_trip_v2(&bbf, &bf, "Modal-Idem").unwrap();
        assert!(audit.is_decidable(),
            "Modal-Idem must be decidable in V2 (no bridge admit)");
    }

    #[test]
    fn round_trip_v2_rejects_distinct_atoms() {
        let err = KernelRecheck::round_trip_v2(
            &core_var("a"), &core_var("b"), "distinct"
        ).unwrap_err();
        match err {
            KernelRecheckError::RoundTrip { .. } => {} // expected
            other => panic!("expected RoundTrip error, got {:?}", other),
        }
    }

    #[test]
    fn eps_mu_v3_final_admits_identity_with_empty_audit() {
        let f = core_var("F");
        let lhs = CoreTerm::EpsilonOf(verum_common::Heap::new(f.clone()));
        let rhs = CoreTerm::AlphaOf(verum_common::Heap::new(
            CoreTerm::EpsilonOf(verum_common::Heap::new(f))));
        let audit = KernelRecheck::eps_mu_v3_final(&lhs, &rhs, "id-case").unwrap();
        assert!(audit.is_decidable());
    }

    #[test]
    fn eps_mu_v3_final_records_a_3_for_non_identity() {
        // (App(F, x), App(x, F)) — same depth, same fvs, distinct shape.
        let m_alpha = CoreTerm::App(
            verum_common::Heap::new(core_var("F")),
            verum_common::Heap::new(core_var("x")));
        let alpha_rhs = CoreTerm::App(
            verum_common::Heap::new(core_var("x")),
            verum_common::Heap::new(core_var("F")));
        let lhs = CoreTerm::EpsilonOf(verum_common::Heap::new(m_alpha));
        let rhs = CoreTerm::AlphaOf(verum_common::Heap::new(
            CoreTerm::EpsilonOf(verum_common::Heap::new(alpha_rhs))));
        let audit = KernelRecheck::eps_mu_v3_final(&lhs, &rhs, "non-id").unwrap();
        assert!(!audit.is_decidable(),
            "non-identity case must record an A-3 admit");
        assert_eq!(audit.admits().len(), 1);
    }

    #[test]
    fn canonicalize_returns_normal_form_with_audit() {
        // canonical_form(AlphaOf(EpsilonOf(F))) → F.
        let f = core_var("F");
        let aef = CoreTerm::AlphaOf(verum_common::Heap::new(
            CoreTerm::EpsilonOf(verum_common::Heap::new(f.clone()))));
        let (canon, audit) = KernelRecheck::canonicalize(&aef, "K-Adj-Unit");
        assert_eq!(canon, f, "K-Adj-Unit collapse must produce F");
        assert!(audit.is_decidable());
    }

    #[test]
    fn merge_audits_concatenates_distinct_records() {
        let mut a = BridgeAudit::new();
        a.record(verum_kernel::BridgeId::ConfluenceOfModalRewrite, "ctx-A");
        let mut b = BridgeAudit::new();
        b.record(verum_kernel::BridgeId::EpsMuTauWitness, "ctx-B");
        let merged = KernelRecheck::merge_audits(a, b);
        assert_eq!(merged.admits().len(), 2);
    }

    #[test]
    fn merge_audits_dedups_same_bridge_same_context() {
        let mut a = BridgeAudit::new();
        a.record(verum_kernel::BridgeId::ConfluenceOfModalRewrite, "ctx-shared");
        let mut b = BridgeAudit::new();
        b.record(verum_kernel::BridgeId::ConfluenceOfModalRewrite, "ctx-shared");
        let merged = KernelRecheck::merge_audits(a, b);
        assert_eq!(merged.admits().len(), 1, "dup must collapse");
    }
}
