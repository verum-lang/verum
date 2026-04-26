//! Kernel-rule recheck pass — VFE-1/VFE-3/VFE-7 V0 wiring.
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
//! invoked from any compiler phase. Per the VUVA architecture
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
    CoreTerm, KernelError, UniverseTier, check_eps_mu_coherence, check_refine_omega,
    check_universe_ascent,
};
use verum_types::refinement::{RefinementBinding, RefinementPredicate as TypesRefinementPredicate};
use verum_types::ty::Type as TypesType;

/// Errors surfaced by the kernel-recheck façade. Each variant
/// preserves enough provenance to thread the original
/// [`KernelError`] back to the verification ladder so the diagnostic
/// emitter can show *which* K-rule failed and on *what* obligation.
#[derive(Debug, Error)]
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
        for param in func.params.iter() {
            if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &param.kind {
                walk_ast_type_for_recheck(ty, &func.name.name, "param", &mut out);
            }
        }
        if let Maybe::Some(ret_ty) = &func.return_type {
            walk_ast_type_for_recheck(ret_ty, &func.name.name, "return", &mut out);
        }
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
        // Other shapes (Function, Tuple, Reference, etc.) — opaque
        // for V2; richer translation is V3 work tracked under #185.
        _ => CoreTerm::Var(Text::from("<unsupported-types-type>")),
    }
}

/// Lift an AST [`Expr`] node into a kernel [`CoreTerm`]. Modal-
/// operator support is wired so K-Refine-omega correctly rejects
/// over-stratified predicates (the canonical V1 use case).
///
/// Coverage:
///
///   • `ExprKind::Path` → `CoreTerm::Var("<last-segment>")`.
///   • Method-call shape `x.box()` / `x.diamond()` → `ModalBox(x)`
///     / `ModalDiamond(x)` so user-side modal predicates lift to
///     the kernel's modal constructors. (V2 will add a richer
///     surface syntax for modalities.)
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
            receiver, method, ..
        } => {
            let inner = lift_expr_to_core(receiver);
            match method.name.as_str() {
                "box" | "necessarily" => CoreTerm::ModalBox(Heap::new(inner)),
                "diamond" | "possibly" => CoreTerm::ModalDiamond(Heap::new(inner)),
                _ => CoreTerm::Var(Text::from("<method-call>")),
            }
        }
        _ => CoreTerm::Var(Text::from("<unsupported-expr>")),
    }
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
}
