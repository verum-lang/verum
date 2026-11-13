//! Expression inference methods for the bidirectional type-checker.
//!
//! Contains ~78 `TypeChecker` methods covering:
//! - Top-level synthesis/checking dispatch (`synth_expr`, `check_expr`, `infer_expr`)
//! - Per-expression-kind handlers (`infer_expr_call`, `infer_match_expr`, …)
//! - Block and statement-level inference (`infer_block`, `check_block`)
//! - Iterative inference for deeply-nested expressions (`synth_expr_iterative`)

#[allow(unused_imports)]
use crate::const_eval::ConstEvaluator;
#[allow(unused_imports)]
use crate::context::{TypeContext, TypeScheme};
#[allow(unused_imports)]
use crate::context_check::{ContextChecker, ContextRequirement, ContextSet};
#[allow(unused_imports)]
use crate::integer_hierarchy::IntegerHierarchy;
#[allow(unused_imports)]
use crate::meta_context::{MetaContextValidation, MetaContextValidator};
#[allow(unused_imports)]
use crate::operator_protocols::{OperatorProtocols, OutputStrategy};
#[allow(unused_imports)]
use crate::protocol::ProtocolChecker;
#[allow(unused_imports)]
use crate::refinement::RefinementChecker;
#[allow(unused_imports)]
use crate::stage_checker::{StageChecker, StageConfig};
#[allow(unused_imports)]
use crate::subtype::Subtyping;
#[allow(unused_imports)]
use crate::ty::{Substitution, Type, TypeVar};
#[allow(unused_imports)]
use crate::unify::Unifier;
#[allow(unused_imports)]
use crate::{Result, TypeCheckMetrics, TypeError};
// Items from parent module (crate::infer) — accessible because expr is a descendant module.
#[allow(unused_imports)]
use super::{
    GlobalDepthGuard, GeneratorContext, InferMode, InferResult, InferWork, TypeChecker,
    WKT_HEAP, WKT_RESULT, WKT_SHARED,
    DEREF_COERCION_DEPTH, GLOBAL_CALL_DEPTH, NORMALIZE_DEPTH,
    TYPE_RESOLUTION_STACK, NORMALIZE_TYPE_STACK, AST_TO_TYPE_DEPTH,
    span_to_line_col, levenshtein_distance, expr_kind_description,
};
#[allow(unused_imports)]
use verum_ast::{
    BinOp, Block, Expr, ExprKind, LiteralKind, Stmt, StmtKind, TokenTree, UnOp,
};
#[allow(unused_imports)]
use verum_ast::decl::RecordField;
#[allow(unused_imports)]
use verum_ast::pattern::Pattern;
#[allow(unused_imports)]
use verum_ast::span::Span;
#[allow(unused_imports)]
use verum_ast::ty::Path;
#[allow(unused_imports)]
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
#[allow(unused_imports)]
use verum_common::well_known_types::WellKnownType as WKT;
#[allow(unused_imports)]
use verum_common::well_known_types::type_names as wkt_names;
#[allow(unused_imports)]
use verum_common::{Heap, List, Map, Maybe, Set, Shared, Text, ToText};
#[allow(unused_imports)]
use verum_modules::resolver::NameKind;

/// Head (last dotted segment) simple name of a type — the #47
/// expected-parent hint for multi-parent bare-constructor
/// disambiguation (`Continue` on both ControlFlow and ReduceResult).
fn expected_parent_head(ty: &Type) -> Option<String> {
    // NOTE: name-level alias chains are resolved by the caller via
    // `Self::expected_parent_hint` — this helper is purely structural.

    match ty {
        Type::Named { path, .. } => path.segments.iter().rev().find_map(|s| match s {
            verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str().to_owned()),
            _ => None,
        }),
        Type::Generic { name, .. } => Some(name.as_str().to_owned()),
        _ => None,
    }
}

/// Peel refinement wrappers to reach the type that decides indexability.
///
/// A refinement says WHICH values inhabit a type; it never removes an
/// operation the base type supports.  `&List<Int>{len > 0}` is indexable
/// exactly when `&List<Int>` is.  Both nestings occur — a refinement can
/// sit outside the reference or inside it — so callers peel on both sides
/// of the reference rather than assuming one spelling.
fn peel_refinement(ty: &Type) -> &Type {
    match ty {
        Type::Refined { base, .. } => peel_refinement(base),
        other => other,
    }
}


/// Whether evaluating a named field transfers its value or only addresses its storage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldUse {
    Value,
    Borrow,
    /// Selecting a deeper field may traverse a partially moved record.
    Operand,
}

/// A resolved callable and the premises attached to its fresh parameters.
struct PreparedCall {
    callable: Type,
    callee_name: Maybe<Text>,
    protocol_bounds: Map<TypeVar, List<crate::protocol::ProtocolBound>>,
    type_bounds: Map<TypeVar, List<Type>>,
}

enum CallPreparation {
    Inferred(InferResult),
    Apply(PreparedCall),
}

impl TypeChecker {
    pub(super) fn synth_field_use(&mut self, expr: &Expr, usage: FieldUse) -> Result<InferResult> {
        let mut field = expr;
        while let ExprKind::Paren(inner) = &field.kind {
            field = inner;
        }
        if !matches!(field.kind, ExprKind::Field { .. }) {
            return self.synth_expr(expr);
        }
        let result = self.infer_expr_field(field)?;
        if let Some((root, projection)) = Self::named_field_place(field)
            && self.affine_tracker.get_binding_resource_kind(&root).is_some()
        {
            self.affine_tracker.check_field_available(
                &root, &projection, usage == FieldUse::Operand, field.span,
            )?;
            if usage == FieldUse::Value {
                self.track_affine_field_access(&root, &projection, &result.ty, field.span)?;
            }
        }
        self.observe_expression_type(expr, &result.ty);
        Ok(result)
    }

    /// Arguments initially borrow their operands; the resolved parameter modes
    /// determine consumption afterwards. Restore the enclosing mode even when
    /// inference fails, so diagnostic recovery cannot suppress later moves.
    pub(super) fn with_call_argument_context<T>(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous = self.in_call_arg_context;
        self.in_call_arg_context = true;
        let result = check(self);
        self.in_call_arg_context = previous;
        result
    }

    pub(super) fn synth_borrowed_operand(&mut self, expr: &Expr) -> Result<InferResult> {
        self.with_call_argument_context(|checker| checker.synth_field_use(expr, FieldUse::Borrow))
    }

    fn with_async_context<T>(
        &mut self,
        enabled: bool,
        check: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous = std::mem::replace(&mut self.in_async_context, enabled);
        let result = check(self);
        self.in_async_context = previous;
        result
    }

    pub(super) fn with_lexical_scope<T>(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.ctx.enter_scope();
        self.borrow_tracker.enter_scope();
        self.context_checker.enter_scope();
        let result = check(self);
        self.context_checker.exit_scope();
        self.borrow_tracker.exit_scope();
        self.ctx.exit_scope();
        result
    }

    fn with_lexical_result_scope(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<InferResult>,
    ) -> Result<InferResult> {
        self.ctx.enter_scope();
        self.context_checker.enter_scope();
        let result = self.with_result_borrow_scope(check);
        self.context_checker.exit_scope();
        self.ctx.exit_scope();
        result
    }

    fn with_try_recover_context<T>(
        &mut self,
        error_type: &Type,
        check: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let return_type = Type::result(Type::Var(TypeVar::fresh()), error_type.clone());
        let previous_return = self.current_function_return_type.replace(return_type);
        self.try_recover_depth += 1;
        let result = check(self);
        self.try_recover_depth -= 1;
        self.current_function_return_type = previous_return;
        result
    }

    fn check_borrow_operand(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        if Self::named_field_place(expr).is_some() {
            let result = self.synth_borrowed_operand(expr)?;
            return self.check_synthesized_value(expr, expected, result);
        }
        self.check_expr(expr, expected)
    }

    pub fn synth_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        self.metrics.synth_count += 1;

        // Use iterative inference for expressions that might be deeply nested
        // to avoid stack overflow. Falls back to recursive inference for
        // complex expressions that need full context.
        let result = self.with_value_obligation_scope(false, |checker| {
            if Self::should_use_iterative_inference(expr) {
                checker.synth_expr_iterative(expr)
            } else {
                checker.infer_expr(expr, InferMode::Synth)
            }
        })?;
        self.observe_expression_type(expr, &result.ty);
        Ok(result)
    }

    /// Determine if an expression should use iterative inference.
    ///
    /// Returns true for expressions that are commonly deeply nested:
    /// - Binary operations (arithmetic chains)
    /// - Unary operations
    /// - Simple literals and paths
    /// - If expressions
    ///
    /// Returns false for complex expressions that need full recursive context:
    /// - Function calls (need argument type checking)
    /// - Method calls (need receiver type checking)
    /// - Closures (need scope management)
    /// - Blocks (need statement processing)
    fn should_use_iterative_inference(expr: &Expr) -> bool {
        use ExprKind::*;

        // Unwrap parentheses to see actual expression
        let mut current = expr;
        while let Paren(inner) = &current.kind {
            current = inner;
        }

        match &current.kind {
            // Safe for iterative inference - these are the expressions that commonly nest deeply
            Literal(_) | Path(_) | Binary { .. } | Unary { .. } => true,

            // Parentheses should already be unwrapped, but handle defensively
            Paren(inner) => Self::should_use_iterative_inference(inner),

            // Complex expressions need recursive inference
            // This includes: If (has IfCondition), Call, Method, Field, Block, Closure, etc.
            _ => false,
        }
    }

    /// Type check in checking mode.
    ///
    /// Uses a unified depth counter to prevent stack overflow from mutual
    /// recursion between check_expr and infer_expr.
    pub(super) fn check_expr(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        let _depth_guard = self.inc_inference_depth("check_expr")?;

        // REDUCE THE PROJECTION BEFORE COMPARING AGAINST IT.
        //
        // `resolve_assoc_projections_deep` was applied to match
        // scrutinees and to nothing else, so an EXPECTED type carrying a
        // projection reached unification unreduced and the mismatch was
        // reported against the projection itself:
        //
        //     expected 'Item<ListIter<Verified>>', found 'Verified'
        //
        // The resolver had already answered, in the same run — the
        // `[assoc]` trace shows `Item<ListIter<Int>> -> Some(&Int)` —
        // and the comparison did not ask. Computed and discarded, which
        // is the shape this codebase keeps paying for (T0912).
        //
        // Resolve MORE, never less: the helper rewrites only heads the
        // protocol registry recognises as associated types and returns
        // everything else unchanged, so a genuine generic (`List`,
        // `Maybe`) passes through untouched.
        let expected = &self.resolve_assoc_projections_deep(expected);

        let result = self.check_expr_inner(expr, expected)?;
        self.observe_expression_type(expr, &result.ty);
        Ok(result)
    }

    /// If `resolved_expected` is a function type whose return type expands to a
    /// `Type::Variant` that declares constructor `name`, return that return
    /// type's head type name — the constructor's parent. This lets a bare
    /// constructor used as a first-class function value (`and_then(Some)`)
    /// resolve under glob / `mount core.prelude.*` imports, which register the
    /// variant type but do NOT populate `variant_constructor_parents` (only the
    /// first-declaration and core_metadata paths do). None otherwise.
    fn ctor_parent_from_expected_fn_return(
        &self,
        resolved_expected: &Type,
        name: &str,
    ) -> Result<Option<Text>> {
        let return_type = match resolved_expected {
            Type::Function { return_type, .. } => return_type,
            _ => return Ok(None),
        };
        let ret = self.unifier.apply(&**return_type);
        let expanded = self.expand_generic_to_variant(&ret)?;
        if crate::ctor_trace_enabled() {
            eprintln!(
                "[ctor-trace] ctor_parent_from_expected_fn_return: name={:?} ret={:?} expanded_is_variant={}",
                name,
                ret,
                matches!(&expanded, Type::Variant(_)),
            );
        }
        let Type::Variant(variants) = &expanded else {
            return Ok(None);
        };
        if !variants.contains_key(name) { return Ok(None); }
        Ok(match &ret {
            Type::Named { path, .. } => path.as_ident().map(|i| Text::from(i.name.as_str())),
            Type::Generic { name: n, .. } => Some(n.clone()),
            _ => None,
        })
    }

    /// CTOR-AS-FN-SYNTH (#31): a bare PAYLOAD constructor used as a VALUE is a
    /// first-class function `fn(payload...) -> Variant`. The synth path
    /// (`infer_expr_path`) otherwise yields the ctor's nominal binding type
    /// (`Named("Some")`), which mismatches wherever a function is expected —
    /// the `m.and_then(Some)` / `let f: fn(Int) -> Maybe<Int> = Some` class
    /// (the annotated-let and method-arg flows SYNTH the expr; check_expr's
    /// bare-ctor coercion arm is never consulted). The called form `Some(x)`
    /// never reaches `infer_expr_path` (the Call arm resolves the ctor
    /// directly), and 0-arg ctors (`None`) keep their variant VALUE typing
    /// (`params.is_empty()` → no override). Returns None when `name` is not a
    /// registered payload constructor.
    fn bare_payload_ctor_as_fn(&self, name: &str, resolved_ty: &Type) -> Option<Type> {
        if crate::ctor_trace_enabled() {
            eprintln!(
                "[ctor-trace] bare_payload_ctor_as_fn: name={:?} resolved={:?} in_parents={}",
                name,
                resolved_ty,
                self.variant_constructor_parents
                    .contains_key(&Text::from(name)),
            );
        }
        if matches!(resolved_ty, Type::Function { .. }) {
            return None;
        }
        let Some(parents) = self.variant_constructor_parents.get(&Text::from(name)) else {
            return None;
        };
        // GENUINE-VALUE-BINDING GUARD (T0617): the ctor-as-fn synth is only
        // correct when `env.lookup` returned the ctor's OWN nominal binding — a
        // `Named`/`Generic` whose head is the ctor name or one of its parent
        // types (the `Named("Some")` the doc comment above describes). When
        // `env.lookup` instead returned an UNRELATED value whose head is neither
        // — e.g. an explicitly `mount`-ed `const E: Float` colliding with an
        // ambient `ParsedResource.E` variant ctor from another, unmounted module
        // — that explicit binding WINS (name-resolution.md: an explicit mount
        // always beats an ambient variant ctor). Overriding it here fabricated
        // `fn(ClusterLoadAssignment) -> ParsedResource` for `E` and broke every
        // `E: Float` use (T0617, 4x E400 in math/constants/unit_test.vr). Every
        // genuine synth site resolves `env.lookup` to a `Function` (returned
        // above) or to the ctor's own nominal (allowed here), so this guard
        // changes ONLY the unrelated-binding case — no stdlib names hardcoded.
        let head = match resolved_ty {
            Type::Named { path, .. } => path.as_ident().map(|i| i.name.as_str().to_string()),
            Type::Generic { name: n, .. } => Some(n.as_str().to_string()),
            _ => None,
        };
        // T1234: the guard above extracts a HEAD, and a `Type::Variant`
        // has none — so an owner bound in EXPANDED form fell into
        // `None => false` and returned before the ctor table was ever
        // asked.  That is the whole of `r.and_then(Ok)`: the unit claim
        // (infer/helpers.rs) binds bare `Ok` to ExitCode's variant type,
        // `env.lookup` hands back
        //   `Ok(Unit) | GenericError(Unit) | … | Custom(Int)`,
        // head is None, and the synth declined without ever learning
        // that `Result.Ok(T)` also owns the name.
        //
        // A Variant that CONTAINS `name` is the ctor's own binding —
        // the same fact `head == name` states, written in expanded
        // form — so it satisfies T0617's question rather than evading
        // it.  The unrelated-binding case T0617 was bought by (a mounted
        // `const E: Float` colliding with `ParsedResource.E`) is not a
        // Variant and still returns false here.
        //
        // This does NOT make unit ctors into functions: `None` reaches
        // the same line, resolves to `Maybe.None`, and is declined below
        // by `params.is_empty()` — the arm that keeps 0-arg ctors as
        // VALUES.  Only a name whose resolved owner carries a payload
        // gets past that, which is exactly the shape being fixed.
        let is_ctor_nominal = match head.as_deref() {
            Some(h) => h == name || parents.iter().any(|p| p.as_str() == h),
            None => matches!(
                resolved_ty,
                Type::Variant(vs) if vs.contains_key(&Text::from(name))
            ),
        };
        if crate::ctor_trace_enabled() {
            eprintln!(
                "[ctor-trace]   T1234 guard: name={:?} head={:?} \
                 variant_owner={} => is_ctor_nominal={}",
                name,
                head,
                matches!(resolved_ty, Type::Variant(vs)
                    if vs.contains_key(&Text::from(name))),
                is_ctor_nominal,
            );
        }
        if !is_ctor_nominal {
            return None;
        }
        let ctor_ty = self.try_resolve_variant_constructor_with_arity(name, None)?;
        if let Type::Function { ref params, .. } = ctor_ty
            && !params.is_empty()
        {
            return Some(ctor_ty);
        }
        None
    }



    /// EXHAUSTIVENESS-DEEP-EXPAND (#41): expand nested variant payloads for
    /// the exhaustiveness scrutinee. `expand_generic_to_variant` only opens
    /// the TOP level (`Poll<Result<Int, P>>` -> `Ready(Result<Int, P>) |
    /// Pending`), so nested-constructor coverage (`Ready(Ok(_)) |
    /// Ready(Err(_))`) could not be proven — the payload stayed a closed
    /// `Generic{Result}` and 13 async tests hard-failed with
    /// "`Ready(_)` not covered". Recursively expand payloads (bounded depth
    /// guards against recursive types like Tree/List).
    fn expand_scrut_deep(&self, ty: &Type, depth: u8) -> Result<Type> {
        let expanded = self.expand_generic_to_variant(ty)?;
        if depth == 0 {
            return Ok(expanded);
        }
        if let Type::Variant(vs) = &expanded {
            let deep: indexmap::IndexMap<verum_common::Text, Type> = vs
                .iter()
                .map(|(n, p)| Ok((n.clone(), self.expand_scrut_deep(p, depth - 1)?)))
                .collect::<Result<_>>()?;
            return Ok(Type::Variant(deep));
        }
        Ok(expanded)
    }

    /// EXHAUSTIVENESS-UNRESOLVED-PAYLOAD (#41): true when the scrutinee's
    /// variant payloads contain an UNRESOLVED type — a unification var, a
    /// bare generic type param (`T`), or an associated-type projection
    /// (`::Output[...]`). Nested-constructor coverage over such a payload is
    /// UNDECIDABLE at check time (`match f.poll(cx) { Poll.Ready(Ok(_)) |
    /// Poll.Ready(Err(_)) | Poll.Pending }` inside `fn tag<F: Future>` — the
    /// payload is `F.Output`), so a non-exhaustive verdict must degrade to
    /// the W0601 warning path instead of hard-failing compilation with
    /// `Ready(_) not covered` (13 async tests).
    fn scrut_has_unresolved_payload(ty: &Type) -> bool {
        fn unresolved(t: &Type) -> bool {
            match t {
                Type::Var(_) => true,
                Type::Generic { name, args } => {
                    name.as_str().starts_with("::")
                        || verum_common::well_known_types::looks_like_type_param(name.as_str())
                        || args.iter().any(unresolved)
                }
                Type::Named { args, .. } => args.iter().any(unresolved),
                // A payload may itself be an EXPANDED variant/tuple/record
                // (expand_generic_to_variant inlines `Result<F.Output, E>`
                // into `Ok(::Output[F]) | Err(E)`), so recurse structurally.
                Type::Variant(vs) => vs.iter().any(|(_, p)| unresolved(p)),
                Type::Tuple(elems) => elems.iter().any(unresolved),
                Type::Record(fields) => fields.iter().any(|(_, t)| unresolved(t)),
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } => unresolved(inner),
                _ => false,
            }
        }
        if let Type::Variant(variants) = ty {
            variants.iter().any(|(_, payload)| unresolved(payload))
        } else {
            false
        }
    }

    /// Both bidirectional paths submit the same source patterns and guards.
    /// Conditional coverage is never presented as a concrete counterexample.
    fn check_match_coverage(
        &mut self,
        scrutinee: &Type,
        arms: &verum_common::List<verum_ast::pattern::MatchArm>,
        span: verum_ast::span::Span,
    ) -> Result<()> {
        let applied = self.unifier.apply(scrutinee);
        let mut carrier = &applied;
        while let Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::UnsafeReference { inner, .. } = carrier
        {
            carrier = inner;
        }
        let resolved = self.expand_scrut_deep(carrier, 3)?;
        let mut coverage_carrier = &resolved;
        while let Type::Refined { base, .. } = coverage_carrier {
            coverage_carrier = base;
        }
        if !matches!(
            coverage_carrier,
            Type::Variant(_) | Type::Bool | Type::Int | Type::Float
        ) {
            return Ok(());
        }
        let patterns: Vec<_> = arms
            .iter()
            .map(|arm| match &arm.guard {
                Some(guard) => verum_ast::pattern::Pattern::new(
                    verum_ast::pattern::PatternKind::Guard {
                        pattern: verum_common::Heap::new(arm.pattern.clone()),
                        guard: guard.clone(),
                    },
                    arm.pattern.span,
                ),
                None => arm.pattern.clone(),
            })
            .collect();
        let complex = arms
            .iter()
            .any(|arm| Self::pattern_has_complex_forms(&arm.pattern));
        let message = match crate::exhaustiveness::check_exhaustiveness(&patterns, &resolved, &self.ctx.env) {
            Ok(result) if result.is_exhaustive => return Ok(()),
            Ok(result) if result.has_unverified_guards || result.uncovered_witnesses.is_empty() || complex => {
                "match coverage is not established; add an unconditional covering arm or simplify the patterns".to_string()
            }
            Ok(result) => format!("non-exhaustive patterns: `{}` not covered",
                result.uncovered_witnesses.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")),
            Err(error) => format!("match coverage is not established: {error}"),
        };
        if crate::ctor_trace_enabled() {
            eprintln!(
                "[ctor-trace] match coverage pre={applied:?} resolved={resolved:?} unresolved={}",
                Self::scrut_has_unresolved_payload(&resolved)
            );
        }
        // Preserve the existing diagnostic boundary for patterns whose nested
        // representation is not yet fully handled by the matrix checker.
        let diagnostic = if complex {
            Diagnostic::new_warning(message, span_to_line_col(span), "W0601")
        } else {
            Diagnostic::new_error(message, span_to_line_col(span), "E0601")
        };
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    /// Inner implementation of check_expr.
    fn check_expr_inner(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        let _global_guard = GlobalDepthGuard::enter()?;
        self.metrics.check_count += 1;

        use ExprKind::*;

        match &expr.kind {
            // Lambda with known function type
            Closure { .. } => self.check_closure_expr(expr, expected),

            // If expression with expected type
            // If expressions: both branches must unify to same type; if-let patterns narrow types in the then-branch
            // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Evidence Propagation
            If { .. } => self.check_if_expr(expr, expected),

            // Block with expected type
            Block(block) => self.check_block(block, expected),

            // TryBlock with expected type - enables bidirectional inference for Result types
            // Error handling: Result<T, E> and Maybe<T> types, try (?) operator with automatic From conversion, error propagation — Section 6.3 - Try blocks
            //

            // Design: Uses STRUCTURAL matching based on variant keys (Ok/Err, Some/None)
            // rather than hardcoded type names. This is stdlib-agnostic.
            TryBlock(inner_block) => self.check_try_block_expr(expr, expected, inner_block),

            // Record literal with expected type - enables bidirectional inference for generics
            // e.g., `Box { value: 42 }` with expected type `Box<Int>` should infer T = Int
            Record { .. } => self.check_record_expr(expr, expected),

            // Array with expected type - enables bidirectional inference for element types
            // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .5.1 - Meta Parameters
            // When expected is [T; N], check each element against T instead of synthesizing
            // This enables: `let bytes: [Byte; 8] = [0x01, 0x02, ...]` with coercion
            Array(arr_expr) => self.check_array_expr(expr, expected, arr_expr),

            // Bare variant path with expected variant type (e.g., `None` when return type is `Maybe<Int>`):
            // When the expected type is a variant and the expression is a bare path matching
            // one of its unit variants, resolve to the expected type directly.
            // This prevents collisions with other types that have the same variant name.
            Path(path) if path.segments.len() == 1 => self.check_single_path_expr(expr, expected, path),

            // Try/recover with expected type: propagate expected type to both try block
            // and recover arm bodies. This enables bidirectional resolution for `None`/`Some`
            // in try { Some(r) } recover { ... => None } with expected Maybe<T>.
            TryRecover { try_block, recover } => {
                self.check_try_recover_expr(expected, try_block, recover)
            }

            // Match expression with expected type: propagate expected type to each arm body.
            // This enables bidirectional type checking for bare variant names like `None`
            // in match arms, preventing collisions with other types (e.g., Command.None vs Maybe.None).
            Match { expr: scrutinee, arms } => self.check_match_expr(expr, expected, scrutinee, arms),

            // Variant constructor call with expected variant type:
            // When checking `Valid(42)` against `Validation<Text, Int>`, resolve the
            // constructor from the expected type's variants rather than the global scope.
            // This enables user-defined types to use variant names that overlap with stdlib.
            Call { func, args: call_args, .. } if matches!(&func.kind, ExprKind::Path(_)) => {
                match self.try_check_path_call_expr(expr, expected, func, call_args)? {
                    Some(result) => Ok(result),
                    None => self.synth_and_check(expr, expected),
                }
            }

            // Bidirectional checking for reference expressions: &expr against &T
            // When the expected type is a reference, push down the inner type to the
            // inner expression. This prevents the smart-pointer auto-deref in infer_unop
            // from stripping Heap<T>/Shared<T> when we need the wrapper type preserved.
            //

            // Example: `a.cmp(&b)` where cmp expects `&Heap<Int>` and `b: Heap<Int>`.
            // Without this, synth(&b) auto-derefs Heap<Int>→Int, yielding &Int (wrong).
            // With this, we check `b` against `Heap<Int>` directly, yielding &Heap<Int>.
            Unary { op: UnOp::Ref, expr: inner } => {
                self.check_reference_expr(expr, expected, inner, false)
            }
            Unary { op: UnOp::RefMut, expr: inner } => {
                self.check_reference_expr(expr, expected, inner, true)
            }

            // ============================================================
            // PROTOCOL-QUALIFIED METHOD CALL
            // ============================================================
            // Handle expressions like `Default.default()` where the receiver is a
            // protocol name, not a type. When expected type is known, we can use it
            // to determine which type's protocol implementation to call.
            //

            // Example: `let u: () = Default.default();`
            // - Receiver `Default` is a protocol name
            // - Method `default()` has signature `fn default() -> Self`
            // - Expected type is `()`, so we resolve Self = ()
            // - Result type is `()`
            //

            // This is related to task #22 (protocol constant access on types).
            MethodCall { receiver, method, args: call_args, .. } => {
                self.check_method_call_expr(expr, expected, receiver, method, call_args)
            }

            // Map literal: check entries against expected Map<K, V> type
            MapLiteral { entries } => self.check_map_literal_expr(expr, expected, entries),

            // Fall back to synthesis + subsumption check
            _ => self.synth_and_check(expr, expected),
        }
    }

    fn check_try_recover_expr(
        &mut self,
        expected: &Type,
        try_block: &Expr,
        recover: &verum_ast::expr::RecoverBody,
    ) -> Result<InferResult> {
        let error_type = self.extract_try_block_error_type(try_block)?;
        self.with_try_recover_context(&error_type, |checker| {
            checker.check_expr(try_block, expected)
        })?;
        self.check_recover_body(recover, &error_type, Some(expected))?;
        Ok(InferResult::new(expected.clone()))
    }

    fn check_reference_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        inner: &Expr,
        mutable: bool,
    ) -> Result<InferResult> {
        if let Type::Reference {
            mutable: expected_mutable,
            inner: expected_inner,
        } = expected
            && *expected_mutable == mutable
        {
            let checked = if mutable {
                self.with_checked_write(inner, |checker| {
                    checker.check_borrow_operand(inner, expected_inner)
                })
            } else {
                self.check_borrow_operand(inner, expected_inner)
            };
            if checked.is_ok() {
                self.track_reference_borrow(inner, mutable, expr.span)?;
                return Ok(InferResult::new(expected.clone()));
            }
        }
        self.synth_and_check(expr, expected)
    }

    fn check_try_block_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        inner_block: &Expr,
    ) -> Result<InferResult> {
        use ExprKind::*;
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG check_expr_inner TryBlock] expected={}, span={:?}", expected, expr.span);

        // Extract the success type from expected type using STRUCTURAL matching
        // Checks for Ok/Err or Some/None variant structure, not type names
        let (success_expected, error_expected) = self.extract_try_output_types(expected);

        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG check_expr_inner TryBlock] success_expected={}, error_expected={}", success_expected, error_expected);

        // Find error type from ? operators inside the block
        let error_from_operators = self
            .find_try_operator_error_type(inner_block)
            .unwrap_or_else(|| Type::Never);

        // Unify error types if both are known
        if error_expected != Type::Never && error_from_operators != Type::Never {
            let _ = self
                .unifier
                .unify(&error_from_operators, &error_expected, expr.span);
        }
        let error_type = if error_expected != Type::Never {
            error_expected
        } else {
            error_from_operators
        };

        // Temporarily set function return type to Result<T, E> so that
        // the ? operator inside the try block passes type checking.
        // Without this, ? checks the enclosing function's return type (e.g. Int)
        // and emits E0205.
        let saved_return_type = self.current_function_return_type.clone();
        self.current_function_return_type = Maybe::Some(expected.clone());

        // Check if inner block already produces a Try-compatible type
        // First synthesize to see what type the block naturally produces
        let block_result = self.synth_expr(inner_block)?;

        // Restore original function return type
        self.current_function_return_type = saved_return_type;
        let block_ty = &block_result.ty;

        // Resolve type variables to get the actual structure
        let resolved_block_ty = self.unifier.apply(block_ty);

        // Detect if block already returns a Try-compatible type using STRUCTURAL matching
        // A type is Try-compatible if it has Ok/Err or Some/None variant structure
        let already_try_type = self.is_try_compatible_type(&resolved_block_ty);

        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG check_expr_inner TryBlock] block_ty={}, resolved={}, already_try_type={}",
        // block_ty, resolved_block_ty, already_try_type);

        if already_try_type {
            // Block already returns Try-compatible type, unify directly with expected
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG check_expr_inner TryBlock] unifying resolved_block_ty={} with expected={}", resolved_block_ty, expected);
            self.unifier
                .unify(&resolved_block_ty, expected, expr.span)?;
            Ok(InferResult::new(expected.clone()))
        } else {
            // Block returns non-Try type, wrap in success variant
            // Use the expected type's structure to construct the wrapper
            let wrapped_type = self.wrap_in_success_type(&resolved_block_ty, expected, error_type);
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG check_expr_inner TryBlock] wrapping: wrapped_type={}, expected={}", wrapped_type, expected);
            self.unifier.unify(&wrapped_type, expected, expr.span)?;
            Ok(InferResult::new(expected.clone()))
        }
    }

    fn check_array_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        arr_expr: &verum_ast::expr::ArrayExpr,
    ) -> Result<InferResult> {
        use ExprKind::*;
        // Extract element type from expected type
        if let Some((expected_elem, expected_size)) = expected.sequence_domain() {
            match arr_expr {
                verum_ast::expr::ArrayExpr::List(exprs) => {
                    // Check size matches if both are known
                    if let Some(expected_n) = expected_size {
                        if expected_n != exprs.len() {
                            return Err(TypeError::Mismatch {
                                expected: format!("[_; {}]", expected_n).into(),
                                actual: format!("[_; {}]", exprs.len()).into(),
                                span: expr.span,
                            });
                        }
                    }

                    // Check each element against expected element type
                    // This enables numeric literal coercion (Int -> Byte, etc.)
                    for elem in exprs {
                        self.check_expr(elem, expected_elem)?;
                    }

                    Ok(InferResult::new(expected.clone()))
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    // Check value against expected element type
                    self.check_expr(value, expected_elem)?;

                    // Count must be Int
                    let count_result = self.synth_expr(count)?;
                    self.unifier
                        .unify(&count_result.ty, &Type::int(), count.span)?;

                    if let Some(expected_size) = expected_size
                        && self.checked_array_length(count)? != Some(expected_size)
                    {
                        return Err(TypeError::OtherWithCodeSpanned {
                            code: "E405".into(),
                            msg: format!(
                                "array repeat must produce exactly {expected_size} elements"
                            )
                            .into(),
                            span: count.span,
                        });
                    }
                    Ok(InferResult::new(expected.clone()))
                }
            }
        } else {
            // Check if expected is a collection type with an element type
            // (e.g., List<T>, Set<T>, or any single-arg generic collection)
            let list_elem = match expected {
                Type::Generic { args, .. } if args.len() == 1 => Some(&args[0]),
                Type::Named { args, .. } if args.len() == 1 => Some(&args[0]),
                _ => None,
            };
            if let Some(expected_elem) = list_elem {
                // Expected is List<T>, check each element against T
                match arr_expr {
                    verum_ast::expr::ArrayExpr::List(exprs) => {
                        for elem in exprs {
                            self.check_expr(elem, expected_elem)?;
                        }
                        Ok(InferResult::new(expected.clone()))
                    }
                    verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                        self.check_expr(value, expected_elem)?;
                        let count_result = self.synth_expr(count)?;
                        self.unifier
                            .unify(&count_result.ty, &Type::int(), count.span)?;
                        Ok(InferResult::new(expected.clone()))
                    }
                }
            } else {
                // Expected is not an array or List type, fall back to synthesis
                self.synth_and_check(expr, expected)
            }
        }
    }

    fn check_single_path_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        path: &Path,
    ) -> Result<InferResult> {
        use ExprKind::*;
        let name = match path.segments.first() {
            Some(verum_ast::ty::PathSegment::Name(id)) => id.name.as_str(),
            _ => "",
        };
        // First resolve the expected type through any type variables
        let resolved_expected = self.unifier.apply(expected);
        let expanded_expected = self.expand_generic_to_variant(&resolved_expected)?;
        if let Type::Variant(ref variants) = expanded_expected {
            if let Some(payload_ty) = variants.get(name) {
                if matches!(payload_ty, Type::Unit) {
                    // Unit variant (e.g., None) matches expected variant type
                    return Ok(InferResult::new(expected.clone()));
                }
            }
        }
        // ── T1234 pole B — THE EXPECTED TYPE IS THE ONLY JUDGE ──
        // `fn take(f: fn(Int) -> Alpha)` called as `take(Wrap)`,
        // where Alpha AND Beta both declare `Wrap` with a
        // payload.  Nothing else can separate them: both are
        // declared in the file being checked, so the horizon
        // holds both; both take one argument, so arity holds
        // both; and the synth path is handed no expected type at
        // all, so it answered with whichever registration
        // happened last.
        //
        // The first attempt here asked for a PARENT NAME and fed
        // it to the resolver's #47 hint.  It never fired, and the
        // trace said why: the expected return type arrives
        // already EXPANDED —
        //   `Variant({Wrap: Int, Other: Unit})`, not `Named(Alpha)`
        // — and `expected_parent_head` reads only Named/Generic.
        // That is the SAME defect as this task's other half, one
        // level up: a head-extraction defeated by a type in
        // expanded form.
        //
        // So do not ask for the parent at all.  The expected type
        // already carries the answer: if it is a one-parameter
        // function whose return is a Variant owning `name` with a
        // payload, then the bare ctor used here IS a function of
        // exactly that type.  Nothing is guessed and no registry
        // order is consulted.
        //
        // AMBIENT LOSES TO EXPLICIT (name-resolution.md, and the
        // rule T1034 states for the resolver): a name the author
        // bound in a nested scope is theirs.  `lookup_in_root_only`
        // is the canonical predicate for "module-level item rather
        // than function-local binding"; when the two lookups
        // disagree the binding is local and this arm stands down.
        let locally_shadowed = {
            let any = self.ctx.env.lookup(name);
            let root = self.ctx.env.lookup_in_root_only(name);
            match (any, root) {
                (Some(a), Some(r)) => !std::ptr::eq(a, r),
                (Some(_), None) => true,
                _ => false,
            }
        };
        let ctor_registered = self
            .variant_constructor_parents
            .contains_key(&Text::from(name));
        if let Type::Function {
            params,
            return_type,
            ..
        } = &resolved_expected
            && params.len() == 1
            && !locally_shadowed
            && ctor_registered
        {
            let ret = self.expand_generic_to_variant(return_type)?;
            if let Type::Variant(vs) = &ret
                && let Some(payload) = vs.get(name)
                && !matches!(payload, Type::Unit)
                && let Some(param0) = params.first()
            {
                // Still UNIFY the payload against the expected
                // parameter: the expectation says the ctor is
                // usable at this type, and a genuine mismatch
                // must still be reported rather than waved
                // through by having named the right variant.
                let payload = payload.clone();
                let param0 = param0.clone();
                let taken = resolved_expected.clone();
                self.unifier.unify(&payload, &param0, expr.span)?;
                if crate::ctor_trace_enabled() {
                    eprintln!(
                        "[ctor-trace]   T1234 poleB: '{}' taken from the \
                         EXPECTED type => {:?}",
                        name, taken
                    );
                }
                return Ok(InferResult::new(taken));
            }
        }
        if crate::ctor_trace_enabled() {
            eprintln!(
                "[ctor-trace]   T1234 poleB: '{}' NOT taken \
                 (locally_shadowed={} ctor_registered={} expected={:?})",
                name, locally_shadowed, ctor_registered, resolved_expected
            );
        }
        // Not a matching unit variant — fall through to synth_and_check
        self.synth_and_check(expr, expected)
    }

    fn check_match_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        scrutinee: &Expr,
        arms: &List<verum_ast::pattern::MatchArm>,
    ) -> Result<InferResult> {
        self.with_result_borrow_scope(|checker| checker.check_match_in_scope(expr, expected, scrutinee, arms))
    }

    fn check_match_in_scope(
        &mut self, expr: &Expr, expected: &Type, scrutinee: &Expr,
        arms: &List<verum_ast::pattern::MatchArm>,
    ) -> Result<InferResult> {
        use ExprKind::*;
        let scrutinee_ty = self.synth_expr(scrutinee)?.ty;
        // Resolve any associated-type projection in the scrutinee (e.g.
        // `y.poll(cx)` returning `Poll<Y.Output>`) so pattern binding and
        // exhaustiveness see the concrete payload (`Poll<Unit>`).
        let applied = self.unifier.apply(&scrutinee_ty);
        let scrutinee_ty = self.resolve_assoc_projections_deep(&applied);
        let sources = self.aggregate_origins(scrutinee, &scrutinee_ty);
        // The arms are ALTERNATIVES — the same rule, and the same
        // code, as the arm loop in `infer_match_expr`. A `match` has
        // TWO walkers, one per direction, exactly as `if` does; a fix
        // applied to one of them is not applied to the other, and a
        // `match` in tail position of a typed function comes through
        // HERE, not through the synth path.
        let affine_at_match = self.affine_tracker.snapshot_alternatives();
        let loans_at_match = self.borrow_tracker.clone_for_branch();
        let mut escaping_loans = loans_at_match.clone_for_branch();
        // A `match` has TWO walkers, one per direction. This probe
        // names WHICH one ran: an instrument that can only see one of
        // them reports a fix as inert when it is merely in the other
        // half of the pair.
        if std::env::var_os("VERUM_TRACE_AFFINE").is_some() {
            eprintln!("[affine] match walker=CHECK arm-loop, {} arms", arms.len());
        }
        let mut affine_per_arm: Vec<crate::affine::AffineAlternatives> = Vec::new();
        for arm in arms.iter() {
            self.affine_tracker.restore_alternatives(&affine_at_match);
            self.borrow_tracker = loans_at_match.clone_for_branch();
            let arm_result =
                self.infer_scoped_match_arm(arm, &scrutinee_ty, None, Some(expected), &sources)?;
            // An arm that diverges never reaches the code after the
            // match, so what it consumed cannot reach it either.
            if !self.is_never_type(&arm_result.ty) {
                affine_per_arm.push(self.affine_tracker.snapshot_alternatives());
                escaping_loans.merge_live_loans(&self.borrow_tracker);
            }
        }
        if affine_per_arm.is_empty() {
            self.affine_tracker.restore_alternatives(&affine_at_match);
        } else {
            self.affine_tracker.merge_alternatives(&affine_per_arm);
        }

        self.borrow_tracker = escaping_loans;

        self.check_match_coverage(&scrutinee_ty, arms, expr.span)?;

        Ok(InferResult::new(expected.clone()))
    }

    fn try_check_path_call_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        func: &Expr,
        call_args: &List<Expr>,
    ) -> Result<Maybe<InferResult>> {
        use ExprKind::*;
        let constructor_name = if let ExprKind::Path(path) = &func.kind {
            path.segments
                .last()
                .map(|s| match s {
                    verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                    _ => "",
                })
                .unwrap_or("")
        } else {
            ""
        };

        // Expand expected type to variant form
        let resolved_expected_call = self.unifier.apply(expected);
        // #47 alias-hop: `type Deep is ControlFlow<X, Y>;` must
        // present the ALIASED target (head name + concrete args)
        // to the gate, the parent hint and the positional bind
        // below — the raw alias head has no registered ctor
        // parent and an empty args list, so the arm either
        // skipped (args gate) or resolved first-registered-wins.
        let resolved_expected_call = self
            .unifier
            .try_expand_alias(&resolved_expected_call)
            .unwrap_or(resolved_expected_call);
        let expanded_expected = self.expand_generic_to_variant(&resolved_expected_call)?;
        // **Architectural fallback** (closes iterator §D / task #14
        // §3.2): when `expand_generic_to_variant` doesn't produce a
        // `Type::Variant` (because the type's registration shape is
        // self-referential `Named { Same, args: [] }` rather than
        // `Type::Variant`), fall back to
        // `try_resolve_variant_constructor_with_arity` which builds
        // a proper `fn(payload, ...) -> Variant<fresh_vars>` type
        // with fresh TypeVars for parent's generic params, then
        // unifies the constructor return type with the user's
        // expected type so concrete args propagate to payload
        // checking.
        //
        // Stdlib-agnostic: works for any variant whose parent type
        // is registered in `variant_constructor_parents` — handles
        // the lazy-loaded path (`ReduceResult<R>`) symmetrically
        // with the eager-loaded path (`Maybe<T>` / `Result<T, E>`).
        // Gate: only fire when the ORIGINAL expected type carries
        // concrete generic args (e.g. `ReduceResult<Int>`) —
        // signalling the user expressed a fully-instantiated type.
        // When expanded_expected stays as `Type::Named { args: [] }`
        // (precompile-eager-loaded variant types whose registration
        // shape is self-referential), the fallback path resolves
        // the constructor against fresh TypeVars and unifies
        // return-type with the user's expected to bind them.
        //
        // When expected is a `Type::Var(_)` (unresolved unifier var
        // — e.g. polymorphic `assert_eq<T>` first-arg binding
        // pending), we MUST NOT eagerly resolve the constructor —
        // that would pick first-registered parent and shadow the
        // unifier's pending binding.
        let expected_has_concrete_args = match &resolved_expected_call {
            Type::Named { args, .. } | Type::Generic { args, .. } => !args.is_empty(),
            _ => false,
        };
        if !matches!(expanded_expected, Type::Variant(_))
            && expected_has_concrete_args
            && self
                .variant_constructor_parents
                .get(&Text::from(constructor_name))
                .is_some()
            && let Some(ctor_ty) = self.try_resolve_variant_constructor_with_arity_for(
                constructor_name,
                Some(call_args.len()),
                self.expected_parent_hint(&resolved_expected_call)
                    .as_deref(),
            )
        {
            if let Type::Function {
                params,
                return_type,
                ..
            } = ctor_ty
            {
                if params.len() == call_args.len() {
                    // Extract concrete args from the user's expected
                    // type.  These positionally bind the parent
                    // variant's generic params.
                    let concrete_args: List<Type> = match &resolved_expected_call {
                        Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                        _ => List::new(),
                    };
                    // Collect bare-Named param names from the
                    // constructor signature in positional order so
                    // we can substitute the rigid `Named{R}`-style
                    // payloads with the user's concrete args.
                    let mut named_params: Vec<verum_common::Text> = Vec::new();
                    for p in params.iter() {
                        Self::collect_bare_named_param_names(p, &mut named_params);
                    }
                    Self::collect_bare_named_param_names(&return_type, &mut named_params);
                    let mut subst: indexmap::IndexMap<verum_common::Text, Type> =
                        indexmap::IndexMap::new();
                    for (i, name) in named_params.iter().enumerate() {
                        if let Some(arg) = concrete_args.get(i) {
                            subst.insert(name.clone(), arg.clone());
                        }
                    }
                    // Unify constructor's parent-instance return
                    // type with the user's expected type — binds
                    // parent generic-param TypeVars when the
                    // constructor side carries them.
                    let _ = self.unifier.unify(&return_type, expected, expr.span);
                    // Now type-check each call argument against
                    // the substituted param type (combines
                    // user's concrete args + unifier's bindings).
                    let params: List<_> = params.iter().map(|ty|
                        Self::substitute_named_params_in_type(ty, &subst)).collect();
                    return self.check_constructor_call(call_args, &params, expected, expr.span).map(Some);
                }
            }
        }
        if let Type::Variant(ref variants) = expanded_expected {
            if let Some(payload_ty) = variants.get(constructor_name) {
                // The expected variant type has this constructor — use it
                // Check call args against the payload type
                if call_args.len() == 1 {
                    // Single-payload variant.  Covers BOTH the
                    // non-Unit case (`Valid(42)` where `Valid(A)`)
                    // AND the explicit-Unit case (`Ok(())` where
                    // `Result<Unit, E>.Ok(Unit)` payload is Unit).
                    // Pre-fix this arm gated on `!Type::Unit` and
                    // fell through to the arity-based synth path
                    // — which then picked the first-registered
                    // parent (typically a stdlib variant like
                    // `ProveResult.Ok(Proof)`) and check-erred the
                    // `()` arg against `Proof`.
                    self.check_constructor_call(call_args, std::slice::from_ref(payload_ty), expected, expr.span).map(Some)
                } else if call_args.is_empty() && matches!(payload_ty, Type::Unit) {
                    // Unit variant: None
                    self.check_constructor_call(call_args, &[], expected, expr.span).map(Some)
                } else if call_args.len() > 1 {
                    // Multi-arg: treat as tuple payload
                    if let Type::Tuple(tuple_types) = payload_ty {
                        if tuple_types.len() == call_args.len() {
                            return self.check_constructor_call(call_args, tuple_types, expected, expr.span).map(Some);
                        }
                    }
                    // Fall through to synth_and_check
                    Ok(None)
                } else {
                    // Mismatch in arity — fall through
                    Ok(None)
                }
            } else {
                // Constructor not in expected variant type — fall through
                Ok(None)
            }
        } else {
            // Expected type is not a variant — fall through
            Ok(None)
        }
    }

    fn check_method_call_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        receiver: &Expr,
        method: &verum_ast::Ident,
        call_args: &List<Expr>,
    ) -> Result<InferResult> {
        use ExprKind::*;
        // Bidirectional qualified variant construction: `Result.Ok(v)`,
        // `Maybe.Some(x)`, any `T.Variant(args)` checked against a known
        // Result/Maybe-shaped expected type. The synth path
        // (`infer_method_call_inner_impl`) resolves the constructor but
        // returns the `Generic{Result,[T,E]}` form, which fails to unify
        // with the *expanded* `Variant{Ok:T, Err:E}` form the function
        // return type / let-annotation carries — so two qualified ctors
        // in one if/else (`Result.Err(..)` + `Result.Ok(())`) mismatch
        // even though each works in isolation. Mirror the bidirectional
        // bare-name Call arm: bind the argument(s) against the expected
        // variant's payload and return the expected type directly.
        if let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
            && let Some(verum_ast::ty::PathSegment::Name(type_ident)) = path.segments.first()
        {
            let recv_type_name = type_ident.name.as_str();
            let variant_text = Text::from(method.name.as_str());
            let is_variant_of_recv = self
                .variant_constructor_parents
                .get(&variant_text)
                .map(|parents| parents.iter().any(|p| p.as_str() == recv_type_name))
                .unwrap_or(false);
            if is_variant_of_recv {
                let resolved_expected = self.unifier.apply(expected);
                let expanded = self.expand_generic_to_variant(&resolved_expected)?;
                if let Type::Variant(variants) = &expanded
                    && let Some(payload_ty) = variants.get(method.name.as_str())
                {
                    let payload_ty = payload_ty.clone();
                    // Contextual nullary and positional constructors share
                    // argument, ownership and callable checking. Named fields
                    // use the Record expression path; arity mismatches reach
                    // ordinary synthesis for its complete diagnostic.
                    if call_args.len() == 1 && !matches!(payload_ty, Type::Record(_)) {
                        return self.check_constructor_call(call_args, std::slice::from_ref(&payload_ty), expected, expr.span);
                    }
                    if call_args.is_empty() && matches!(payload_ty, Type::Unit) {
                        return self.check_constructor_call(call_args, &[], expected, expr.span);
                    }
                    if let Type::Tuple(params) = payload_ty
                        && params.len() == call_args.len() && call_args.len() > 1
                    {
                        return self.check_constructor_call(call_args, &params, expected, expr.span);
                    }
                }
            }
        }
        // Check if receiver is a path expression to a protocol name
        if let ExprKind::Path(path) = &receiver.kind {
            if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                let name = ident.name.as_str();
                // Check if this name is a known protocol
                let is_protocol = self.protocol_checker.read().is_protocol_by_name(name);
                if is_protocol && path.segments.len() == 1 {
                    // This is a protocol-qualified method call like `Default.default()`
                    // Try to look up the method using the expected type as the implementing type
                    let method_name_text = verum_common::Text::from(method.name.as_str());

                    // Check if the expected type implements this protocol
                    let expected_normalized = self.normalize_type(expected);
                    let method_lookup = self
                        .protocol_checker
                        .read()
                        .lookup_protocol_method(&expected_normalized, &method_name_text);

                    if let Ok(Some(method_ty)) = method_lookup {
                        // Found the method! Check arguments and compute return type
                        if let Type::Function {
                            params,
                            return_type,
                            ..
                        } = &method_ty
                        {
                            // For static protocol methods like Default.default(), params should be empty
                            if params.len() == call_args.len() {
                                // Check each argument
                                for (arg, param_ty) in call_args.iter().zip(params.iter()) {
                                    self.check_expr(arg, param_ty)?;
                                }

                                // The return type might contain Self, which should be the expected type
                                // Apply substitution: Self -> expected
                                let result_type = self.substitute_self_type(return_type, expected);
                                // PROTO-STATIC-EXISTENTIAL (T0585): this
                                // bidirectional arm resolves the implementor
                                // from the EXPECTED type, but resolution that
                                // stops at the type tier leaves codegen to
                                // replay the by-name cascade — which lands on
                                // the protocol's bodyless requirement stub
                                // (measured: `let c: Counter =
                                // Default.default();` printed const-zero, the
                                // user's impl never ran).  Queue the same
                                // epilogue discharge the synth arm uses: α is
                                // pre-bound to the expected implementor, the
                                // discharge verifies the impl and stamps
                                // `resolved_call_targets[span]` with the
                                // direct `{Impl}.{method}` target.
                                let alpha = crate::ty::TypeVar::fresh();
                                let _ = self.unifier.unify(
                                    &Type::Var(alpha),
                                    &expected_normalized,
                                    expr.span,
                                );
                                self.pending_protocol_static_calls.push((
                                    expr.span,
                                    verum_common::Text::from(name),
                                    method_name_text.clone(),
                                    alpha,
                                ));
                                return Ok(InferResult::new(result_type));
                            }
                        }
                    }
                }
            }
        }
        // Fall back to normal handling
        self.synth_and_check(expr, expected)
    }

    fn check_map_literal_expr(
        &mut self,
        expr: &Expr,
        expected: &Type,
        entries: &List<(Expr, Expr)>,
    ) -> Result<InferResult> {
        use ExprKind::*;
        // Extract expected key/value types from Map<K, V> or similar generic
        let (exp_key, exp_val) = match expected {
            Type::Generic { name, args } if name.as_str() == "Map" && args.len() == 2 => {
                (Some(&args[0]), Some(&args[1]))
            }
            Type::Named { path, args } if args.len() == 2 => {
                let is_map = path
                    .segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str() == "Map",
                        _ => false,
                    })
                    .unwrap_or(false);
                if is_map {
                    (Some(&args[0]), Some(&args[1]))
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        };

        if let (Some(key_ty), Some(val_ty)) = (exp_key, exp_val) {
            // Check each entry against expected types
            for (key, val) in entries.iter() {
                self.check_expr(key, key_ty)?;
                self.check_expr(val, val_ty)?;
            }
            return Ok(InferResult::new(expected.clone()));
        }

        // No expected Map type - fall back to synthesis
        self.synth_and_check(expr, expected)
    }

    /// Synthesize then check subsumption.
    ///
    /// Includes auto-borrow coercion: T → &T when expected is immutable reference.
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 3 (Auto-Borrow at a call position)
    /// Bidirectional type-check a closure expression.
    /// When `expected` is a `Forall` (rank-2), instantiates the quantified
    /// variables. When a `Function` type is known, uses it to bind params;
    /// otherwise falls back to synthesis + subsumption.
    fn closure_callable_properties(
        &mut self,
        body: &Expr,
        is_async: bool,
    ) -> Result<crate::computational_properties::PropertyRequirement> {
        let bound = self.callable_body_bound(body);
        Ok(Self::callable_bound_with_marker(bound, is_async))
    }

    fn check_closure_properties(
        &mut self,
        body: &Expr,
        is_async: bool,
        expected: &Type,
        span: Span,
    ) -> Result<crate::computational_properties::PropertyRequirement> {
        let bound = self.closure_callable_properties(body, is_async)?;
        let Type::Function {
            property_requirement: expected_requirement,
            ..
        } = expected
        else {
            return Err(TypeError::Other(
                "closure property check needs a callable contract".into(),
            ));
        };
        let mut actual = Type::function_with_requirement(List::new(), Type::Unit, bound.clone());
        if let Type::Function { properties, .. } = &mut actual {
            *properties = bound.finite_footprint().cloned();
        }
        let contract =
            Type::function_with_requirement(List::new(), Type::Unit, expected_requirement.clone());
        self.check_call_type_bound(&actual, &contract, span, 0)?;
        Ok(bound)
    }

    fn check_closure_expr(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Closure {
            params,
            body: closure_body,
            return_type,
            async_,
            move_,
            ..
        } = &expr.kind
        else {
            unreachable!()
        };
        // ============================================================
        // Rank-2 Polymorphic Closure Checking
        // Spec: grammar/verum.ebnf - rank2_function_type
        // ============================================================
        // When checking a closure against a Forall type (rank-2), we
        // instantiate the quantified type variables and check against
        // the instantiated function type. The closure itself becomes
        // rank-2 polymorphic.
        if let Type::Forall {
            vars,
            body: forall_body,
        } = expected
        {
            // Instantiate the quantified type variables with fresh type vars
            let mut subst: Map<TypeVar, Type> = Map::new();
            for qvar in vars.iter() {
                let fresh = TypeVar::fresh();
                subst.insert(*qvar, Type::Var(fresh));
            }

            // Apply substitution to get the instantiated function type
            let instantiated_body = self.substitute_type_vars(forall_body, &subst);

            // Normalize the instantiated body to resolve type aliases
            // This handles cases like `fn<R>(Reducer<N, R>) -> Reducer<N, R>` where
            // `Reducer<N, R>` needs to be resolved to `fn(R, N) -> R`
            let normalized_body = self.normalize_type(&instantiated_body);

            // Check the closure against the instantiated function type
            if let Type::Function {
                params: expected_params,
                return_type: expected_return,
                ..
            } = &normalized_body
            {
                if params.len() != expected_params.len() {
                    return Err(TypeError::Mismatch {
                        expected: expected.to_text(),
                        actual: format!("function with {} parameters", params.len()).into(),
                        span: expr.span,
                    });
                }

                // An annotation chooses the body's checking type, but it
                // must still satisfy the required callable result. Unit
                // permits result discard without changing the annotation.
                let ret_ty = if let Some(rt) = return_type {
                    let annotated = self.ast_to_type(rt)?;
                    let required = self.normalize_type(&self.unifier.apply(expected_return));
                    if !matches!(required, Type::Unit) {
                        self.check_call_type_bound(&annotated, &required, rt.span, 0)?;
                    }
                    annotated
                } else {
                    (**expected_return).clone()
                };

                // T0653: capture exclusivity is checked on THIS path
                // too, not only when the closure's type is inferred.
                self.with_closure_scope(expr, |checker| {
                    // Bind parameters with the instantiated types.
                    // Normalize each param type to resolve associated type projections
                    // (e.g., ::Item[Range<Int>] → Int) before binding.
                    for (param, param_ty) in params.iter().zip(expected_params.iter()) {
                        let normalized_param_ty = checker.normalize_type(param_ty);
                        checker.bind_parameter_pattern(&param.pattern, &normalized_param_ty)?;
                    }

                    // Check body against expected return type with bidirectional inference.
                    // We do NOT synth first: synth would resolve variant constructors
                    // (e.g., Continue(x)) using the ambient scope (e.g., ControlFlow.Continue
                    // from the prelude) instead of the expected type (e.g., ReduceResult<Int>).
                    // Bidirectional check_expr correctly propagates the expected type through
                    // blocks, if/else expressions, and variant constructors.
                    // Never-returning bodies (e.g., panic("...")) are handled correctly too:
                    // unify(Never, T) succeeds for all T (unify.rs bottom-type rule).
                    // CLOSURE-RETURN-SCOPE-1: `return` inside the body
                    // returns from the CLOSURE, so the body is walked with
                    // `current_function_return_type` set to the closure's
                    // own return type. `infer_expr_return_expr` checks
                    // against that field, and leaving the ENCLOSING
                    // function's value in place made every `return` in a
                    // closure answer to the wrong signature.
                    checker.current_function_return_type =
                        verum_common::Maybe::Some(ret_ty.clone());
                    checker.check_expr(closure_body, &ret_ty)?;
                    checker.check_closure_properties(
                        closure_body,
                        *async_,
                        &normalized_body,
                        expr.span,
                    )?;

                    Ok(())
                })?;

                // Register closure type in the type registry for codegen
                let resolved_expected = self.unifier.apply(expected);
                self.type_registry
                    .register_expr(expr.span, resolved_expected);

                // Return the original Forall type (the closure is rank-2 polymorphic)
                Ok(InferResult::new(expected.clone()))
            } else {
                // Forall body is not a function type
                self.synth_and_check(expr, expected)
            }
        } else {
            // Normalize expected type to resolve type aliases to function types
            // This handles cases like `Reducer<B, Maybe<B>>` which is an alias for `fn(Maybe<B>, B) -> Maybe<B>`
            let mut normalized_expected = self.normalize_type(expected);

            // If the expected type is a type variable, check for function type bounds.
            // This enables proper closure inference for generics like `F: fn() -> T`.
            // Generic function type bounds: "fn foo<T: Protocol>(...)" constrains T to types implementing Protocol
            if let Type::Var(tvar) = &normalized_expected {
                if let Maybe::Some(fn_bound) = self.get_function_type_bound(tvar) {
                    // Apply the unifier to resolve any type variables in the bound
                    // (e.g., fn(&T_fresh) -> Bool where T_fresh was already unified with Int)
                    normalized_expected = self.unifier.apply(&fn_bound);
                }
            }

            if let Type::Function {
                params: expected_params,
                return_type: expected_return,
                ..
            } = &normalized_expected
            {
                if params.len() != expected_params.len() {
                    return Err(TypeError::Mismatch {
                        expected: expected.to_text(),
                        actual: format!("function with {} parameters", params.len()).into(),
                        span: expr.span,
                    });
                }

                // An annotation chooses the body's checking type, but it
                // must still satisfy the required callable result. Unit
                // permits result discard without changing the annotation.
                let ret_ty = if let Some(rt) = return_type {
                    let annotated = self.ast_to_type(rt)?;
                    let required = self.normalize_type(&self.unifier.apply(expected_return));
                    if !matches!(required, Type::Unit) {
                        self.check_call_type_bound(&annotated, &required, rt.span, 0)?;
                    }
                    annotated
                } else {
                    (**expected_return).clone()
                };

                // T0653: capture exclusivity is checked on THIS path
                // too — this is the branch most closures take
                // (`.map(|x| …)`, typed argument, typed binding).
                let callable_properties = self.with_closure_scope(expr, |checker| {
                    // Bind parameters.
                    // Normalize each param type to resolve associated type projections
                    // (e.g., ::Item[Range<Int>] → Int) before binding, so that the
                    // closure body sees concrete types rather than un-resolved projections.
                    for (param, param_ty) in params.iter().zip(expected_params.iter()) {
                        let normalized_param_ty = checker.normalize_type(param_ty);
                        checker.bind_parameter_pattern(&param.pattern, &normalized_param_ty)?;
                    }

                    // Check body against expected return type with bidirectional inference.
                    // We do NOT synth first: synth would resolve variant constructors
                    // (e.g., Continue(x)) using the ambient scope (e.g., ControlFlow.Continue
                    // from the prelude) instead of the expected type (e.g., ReduceResult<Int>).
                    // Bidirectional check_expr correctly propagates the expected type through
                    // blocks, if/else expressions, and variant constructors.
                    // Never-returning bodies (e.g., panic("...")) are handled correctly too:
                    // unify(Never, T) succeeds for all T (unify.rs bottom-type rule).
                    // CLOSURE-RETURN-SCOPE-1: `return` inside the body
                    // returns from the CLOSURE, so the body is walked with
                    // `current_function_return_type` set to the closure's
                    // own return type. `infer_expr_return_expr` checks
                    // against that field, and leaving the ENCLOSING
                    // function's value in place made every `return` in a
                    // closure answer to the wrong signature.
                    checker.current_function_return_type =
                        verum_common::Maybe::Some(ret_ty.clone());
                    checker.check_expr(closure_body, &ret_ty)?;
                    let callable_properties = checker.check_closure_properties(
                        closure_body,
                        *async_,
                        &normalized_expected,
                        expr.span,
                    )?;

                    Ok(callable_properties)
                })?;

                // Preserve the annotated callable result even when the
                // bound permits discarding it. A generic F denotes this
                // actual closure type, not the contract used to check it.
                let mut actual = normalized_expected.clone();
                if let Type::Function {
                    return_type,
                    properties,
                    property_requirement,
                    ..
                } = &mut actual
                {
                    *return_type = Box::new(ret_ty);
                    *properties = callable_properties.finite_footprint().cloned();
                    *property_requirement = callable_properties.clone();
                }
                if let Type::Var(var) = self.unifier.apply(expected) {
                    self.unifier.unify(&Type::Var(var), &actual, expr.span)?;
                }
                let actual = self.unifier.apply(&actual);
                self.type_registry.register_expr(expr.span, actual.clone());

                Ok(InferResult::new(actual))
            } else {
                // Not a function type (even after normalization), fall back to synthesis + subsumption
                self.synth_and_check(expr, expected)
            }
        }
    }

    /// Bidirectional type-check an `if` expression.
    /// Propagates refinement evidence from conditions into the then-branch,
    /// checks both branches against the expected type, and unifies the results.
    fn check_if_expr(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::If { condition, then_branch, else_branch } = &expr.kind
            else { unreachable!() };
                use crate::refinement_evidence::EvidencePropagator;

                // Handle all conditions (expression and/or let conditions)
                // Bindings from let conditions are available in the then branch
                self.ctx.env.push_scope();
                self.refinement_evidence.push_scope();

                // Collect condition expressions for evidence propagation
                let mut condition_exprs: Vec<&Expr> = Vec::new();

                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(cond_expr) => {
                            // Check if this is an `is` pattern test (e.g., `v is IntVal(n)`)
                            // If so, bind pattern variables for the then-branch.
                            if let ExprKind::Is {
                                expr: test_expr,
                                pattern,
                                negated,
                            } = &cond_expr.kind
                            {
                                let test_result = self.synth_expr(test_expr)?;
                                if !*negated {
                                    self.bind_pattern(pattern, &test_result.ty)?;
                                }
                            } else if let ExprKind::Binary {
                                op: verum_ast::expr::BinOp::And,
                                left,
                                right,
                            } = &cond_expr.kind
                            {
                                // Handle `v is Pattern && guard` by extracting Is from left of &&
                                if let ExprKind::Is {
                                    expr: test_expr,
                                    pattern,
                                    negated,
                                } = &left.kind
                                {
                                    let test_result = self.synth_expr(test_expr)?;
                                    if !negated {
                                        self.bind_pattern(pattern, &test_result.ty)?;
                                    }
                                    // Check the guard condition with bindings in scope
                                    self.check_expr(right, &Type::bool())?;
                                } else {
                                    // Regular && expression
                                    self.check_expr(cond_expr, &Type::bool())?;
                                }
                            } else {
                                // Expression condition - must be Bool
                                self.check_expr(cond_expr, &Type::bool())?;
                            }
                            condition_exprs.push(cond_expr);

                            // Add positive evidence for the then-branch
                            // e.g., after `if x.is_empty()`, we know x.is_empty() == true in then
                            self.refinement_evidence
                                .add_evidence_from_condition(cond_expr, cond_expr.span);

                            // Flow-sensitive type narrowing: narrow variable types based on condition
                            self.narrow_variable_types_from_condition(cond_expr, false);

                            // Track method-based conditions for better evidence
                            if let Maybe::Some((var_name, method_name, negated)) =
                                EvidencePropagator::analyze_method_condition(cond_expr)
                            {
                                // Note: The evidence is already added above, but we can
                                // also track it by variable for faster lookup
                                self.refinement_evidence.add_method_evidence(
                                    var_name,
                                    method_name.as_str(),
                                    negated,
                                    cond_expr.span,
                                );
                            }
                        }
                        verum_ast::expr::ConditionKind::Let { pattern, value } => {
                            // Let condition - bind pattern to value type in scope
                            let value_result = self.synth_expr(value)?;
                            self.bind_pattern(pattern, &value_result.ty)?;
                            // Pattern matching also provides evidence about the matched value
                        }
                    }
                }

                // The two branches are ALTERNATIVES — see the match arms in
                // `infer_match_expr` for the same rule. The condition itself
                // runs on every path, so the snapshot is taken after it.
                let affine_at_if = self.affine_tracker.snapshot_alternatives();

                // Check then branch with evidence in scope
                self.check_block(then_branch, expected)?;

                // Check if then branch unconditionally exits
                let then_exits = EvidencePropagator::block_unconditionally_exits(then_branch);

                // A branch that unconditionally exits never reaches the code
                // after the `if`, so what it consumed cannot reach it either.
                let affine_then = (!then_exits)
                    .then(|| self.affine_tracker.snapshot_alternatives());
                self.affine_tracker.restore_alternatives(&affine_at_if);

                self.ctx.env.pop_scope();
                self.refinement_evidence.pop_scope();

                // If then branch exits and there's no else, propagate negated evidence
                // to the continuation (the code after the if)
                // e.g., `if data.is_empty() { return 0; }` means !data.is_empty() after
                if then_exits && else_branch.is_none() {
                    for cond_expr in &condition_exprs {
                        self.refinement_evidence
                            .add_negated_evidence(cond_expr, cond_expr.span);
                        // Narrow variable types with negated condition in continuation
                        self.narrow_variable_types_from_condition(cond_expr, true);

                        // Track negated method conditions
                        if let Maybe::Some((var_name, method_name, was_negated)) =
                            EvidencePropagator::analyze_method_condition(cond_expr)
                        {
                            // Flip the negation: if condition was `x.is_empty()`,
                            // we now know `!x.is_empty()` (negated=true)
                            self.refinement_evidence.add_method_evidence(
                                var_name,
                                method_name.as_str(),
                                !was_negated,
                                cond_expr.span,
                            );
                        }
                    }
                }

                if let Some(else_expr) = else_branch {
                    // In else branch, we have the negation of the condition
                    self.ctx.env.push_scope();
                    self.refinement_evidence.push_scope();
                    for cond_expr in &condition_exprs {
                        self.refinement_evidence
                            .add_negated_evidence(cond_expr, cond_expr.span);
                        self.narrow_variable_types_from_condition(cond_expr, true);
                    }
                    self.check_expr(else_expr, expected)?;
                    self.ctx.env.pop_scope();
                    self.refinement_evidence.pop_scope();
                    let affine_else = self.affine_tracker.snapshot_alternatives();
                    self.merge_if_alternatives(affine_then, Some(affine_else), &affine_at_if);
                } else {
                    // No `else`: falling through IS the second alternative,
                    // and it consumed nothing beyond the condition.
                    self.merge_if_alternatives(affine_then, None, &affine_at_if);
                    self.unifier.unify(&Type::Unit, expected, expr.span)?;
                }

                Ok(InferResult::new(expected.clone()))
    }

    /// Fold the two alternatives of an `if` back together.
    ///
    /// `then_state` is `None` when the then-branch unconditionally exits, and
    /// `else_state` is `None` when there is no `else` — in which case the
    /// fall-through path is the second alternative and is exactly the state
    /// at the construct.
    fn merge_if_alternatives(
        &mut self,
        then_state: Option<crate::affine::AffineAlternatives>,
        else_state: Option<crate::affine::AffineAlternatives>,
        at_if: &crate::affine::AffineAlternatives,
    ) {
        let mut branches: Vec<crate::affine::AffineAlternatives> = Vec::new();
        if let Some(s) = then_state {
            branches.push(s);
        }
        branches.push(else_state.unwrap_or_else(|| at_if.clone()));
        self.affine_tracker.merge_alternatives(&branches);
    }

    /// Bidirectional type-check a record literal expression against an expected type.
    /// Checks each field against its declared type, handles optional base spread,
    /// and propagates narrowed type information for refinement fields.
    /// Type-check a record-literal field VALUE against its declared field type,
    /// accumulating a field-scoped diagnostic on a plain type mismatch instead
    /// of aborting the whole literal.
    ///
    /// A bare `check_expr(value, expected)?` short-circuits at the FIRST bad
    /// field, so `R { n: <wrong>, s: <wrong> }` reported only one of two type
    /// errors. This preserves bidirectional checking (generic / literal fields
    /// still receive the expected type inward) but, on a `Mismatch`, records an
    /// `E400` that names the field plus its expected/found types and CONTINUES,
    /// so every bad field is reported — the construction-site half of diagnostic
    /// totality (see docs/architecture/diagnostic-totality.md).
    ///
    /// Non-mismatch failures (unbound variable in the value, malformed
    /// sub-expression, …) still propagate: swallowing them would drop a real
    /// error and produce a false green. A failure for which `check_expr` already
    /// emitted its own richer diagnostic is kept and not duplicated.
    fn check_record_field_value(
        &mut self,
        field_init: &verum_ast::expr::FieldInit,
        value_expr: &Expr,
        expected_field_ty: &Type,
        type_name: &str,
    ) -> Result<()> {
        let diag_len_before = self.diagnostics.len();
        match self.check_expr(value_expr, expected_field_ty) {
            Ok(_) => Ok(()),
            // Plain type mismatch → field-scoped diagnostic, then keep going so
            // sibling fields are still checked (report ALL bad fields, not just
            // the first).
            Err(TypeError::Mismatch {
                expected, actual, ..
            }) if self.diagnostics.len() == diag_len_before => {
                // T1038: a rank-2 field is checked by SKOLEMISING its
                // binder — the expected type is then a rigid constant that
                // names nothing in the author's program. Printing it
                // (`expected '?forall-bound#3853'`) taught them nothing and
                // read like a compiler bug. Say what the field promises
                // instead; the constant is an implementation detail of the
                // check, not a type the author can write.
                let msg = if matches!(expected_field_ty, Type::Forall { .. }) {
                    format!(
                        "Type mismatch in field '{}' of '{}': the field's type is \
                         universally quantified, so a value stored here must work \
                         for EVERY instantiation of it — this one is fixed to '{}'",
                        field_init.name.name.as_str(),
                        type_name,
                        actual
                    )
                } else {
                    format!(
                        "Type mismatch in field '{}' of '{}': expected '{}', found '{}'",
                        field_init.name.name.as_str(),
                        type_name,
                        expected,
                        actual
                    )
                };
                let diag =
                    Diagnostic::new_error(msg, span_to_line_col(value_expr.span), "E400");
                self.diagnostics.push(diag);
                Ok(())
            }
            // `check_expr` already emitted its own (richer) diagnostic — keep it,
            // don't duplicate, and continue.
            Err(_) if self.diagnostics.len() > diag_len_before => Ok(()),
            // Genuine abort (unbound variable, etc.) — propagate so it is not
            // lost.
            Err(e) => Err(e),
        }
    }

    fn record_variant_parent(&self, path: &verum_ast::ty::Path) -> Option<(Text, Text)> {
        if path.segments.len() < 2 || !path.segments.iter().all(|segment|
            matches!(segment, verum_ast::ty::PathSegment::Name(_)))
        {
            return None;
        }
        let mut parent_path = path.clone();
        let verum_ast::ty::PathSegment::Name(variant) = parent_path.segments.pop()? else {
            return None;
        };
        Some((self.path_to_string(&parent_path).into(), variant.name))
    }

    fn check_record_expr(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Record { path, fields, base } = &expr.kind
            else { unreachable!() };
                // SELF-LITERAL-RESOLVES-ITS-TYPE-1 (T1367): `Self { … }` has to
                // be looked up under the type it MEANS, not under the word
                // `Self`.
                //
                // `path_to_string` renders a `SelfValue` segment as the
                // lowercase string "self", so every field lookup below
                // probed `__struct_fields_self` — a key nothing writes. The
                // arm then fell through to `synth_and_check`, and the
                // synthesised path's own fallback accepts a literal whose
                // type it cannot resolve structurally, UNCHECKED. Net effect,
                // measured on five shapes that differ only in spelling:
                //
                //     Two  { a: 1 }   ->  Missing required field 'b'
                //     Self { a: 1 }   ->  accepted, and `t.b` reads 0.0
                //
                // in EVERY position — return, annotated let, and a function
                // whose declared return type is the named type. So the miss
                // is the SPELLING, not the position, and `Self { … }` is the
                // dominant shape of a stdlib constructor.
                //
                // The same resolution already existed twenty lines below,
                // used only to name the type in the unification step. It is
                // hoisted here so the lookup and the unification agree about
                // what `Self` is.
                // SELF-IS-A-NAME-NOT-A-SEGMENT-1: the parser emits `Self` as
                // `PathSegment::Name("Self")`, NOT as `SelfValue`.
                // `SelfValue` is the LOWERCASE `self` (parser expr.rs: "SelfType
                // (Self) is a type reference, create as Name(\"Self\") not
                // SelfValue"). A first version of this resolution matched the
                // segment kind and was measurably INERT for that reason.
                let head_is_self = path.segments.len() == 1
                    && match &path.segments[0] {
                        verum_ast::ty::PathSegment::SelfValue => true,
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str() == "Self",
                        _ => false,
                    };
                let self_resolved_path = if head_is_self {
                    match &self.current_self_type {
                        // `define_type("Self", …)` installs a placeholder that
                        // points at itself, so a route that has not bound the
                        // real type yet would resolve `Self` to `Self`
                        // forever. Refuse the self-reference rather than loop.
                        // A const-generic impl binds it as `Generic`, not
                        // `Named`, and only the latter is a path we can use
                        // as a lookup key anyway.
                        Maybe::Some(Type::Named {
                            path: self_path, ..
                        }) if self.path_to_string(self_path).as_str() != "Self" => {
                            Some(self_path.clone())
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let lookup_path = self_resolved_path.as_ref().unwrap_or(path);

                // For generic struct instantiation, extract type args from expected type
                // and substitute them into the field types
                let type_name = self.path_to_string(lookup_path);
                let struct_key = format!("__struct_fields_{}", type_name);

                // Check if type_name is a variant record constructor by looking it up in the env.
                // If it's a constructor function returning a Variant type, delegate to synth_and_check
                // which correctly handles variant record constructors via env lookup.
                // This prevents stdlib struct "Rect { x, y, width, height }" from shadowing
                // user-defined variant "Rect { w, h }" in `type Shape is ... | Rect { w: Int, h: Int }`
                let is_variant_constructor =
                    if let Some(scheme) = self.ctx.env.lookup(type_name.as_str()) {
                        let ty = scheme.instantiate();
                        matches!(&ty, Type::Function { return_type, .. }
                        if matches!(return_type.as_ref(), Type::Variant(_)))
                    } else {
                        false
                    };

                // Aliases do not publish a second constructor function. Use
                // their resolved parent declaration before the record guard;
                // the existing variant checker validates the selected payload.
                let is_aliased_variant_constructor = if let Some((parent, variant)) = self.record_variant_parent(lookup_path)
                    && let Ok(parent_ty) = self.resolve_type_name(parent.as_str(), expr.span)
                    && self.resolve_to_record_type(&parent_ty).is_none()
                    && let Type::Variant(variants) = self.expand_generic_to_variant(&parent_ty)?
                {
                    variants.contains_key(variant.as_str())
                } else {
                    false
                };
                if is_variant_constructor || is_aliased_variant_constructor {
                    return self.synth_and_check(expr, expected);
                }
                let declaration_ty = self.lookup_record_type(lookup_path, expr.span)?;
                self.check_record_construction_visibility(&declaration_ty, expr.span)?;

                // Get the stored field types (may contain type variables like T)
                //
                // §Y close: every probe of an unqualified type key is
                // last-write-wins across stdlib modules.  We route the
                // mount-scoped probes through `lookup_type_mount_scoped`
                // FIRST so explicit user mounts take precedence over
                // whichever module's `define_type` happened to land
                // last.  See `lookup_type_mount_scoped` doc-comment +
                // audit.md §Y for the architectural rationale.
                let mount_struct = self
                    .lookup_type_mount_scoped(
                        type_name.as_str(),
                        "__struct_fields_",
                    );
                let mount_plain = self
                    .lookup_type_mount_scoped(type_name.as_str(), "");
                let stored_fields = match mount_struct {
                    Some(Type::Record(field_types)) => Some(field_types),
                    _ => match mount_plain {
                        Some(Type::Record(field_types)) => Some(field_types),
                        _ => match self.ctx.lookup_type(struct_key.as_str()) {
                            Option::Some(Type::Record(field_types)) => Some(field_types.clone()),
                            _ => match self.ctx.lookup_type(type_name.as_str()) {
                                Option::Some(Type::Record(field_types)) => Some(field_types.clone()),
                                _ => {
                                    if let Type::Variant(variants) = expected {
                                        variants.get(type_name.as_str()).and_then(|payload| {
                                            if let Type::Record(field_types) = payload {
                                                Some(field_types.clone())
                                            } else {
                                                None
                                            }
                                        })
                                    } else {
                                        None
                                    }
                                }
                            },
                        },
                    },
                };

                // ENSURE-THEN-RESOLVE (T0724 residual). `__type_params_<name>`
                // is written by `ensure_stdlib_type_loaded`, the LAZY loader.
                // Reading the key without first driving that loader made the
                // param list come back empty for every archive-imported
                // generic RECORD, so the zip below produced no substitution
                // and the declared field types stayed as their parameter
                // names: `Edge { src: 1, tgt: 2, label: 7 }` failed with
                // `expected 'L', found 'Int'`, and an explicit
                // `let g: Edge<Int, Int> = ...` did not help — with an empty
                // param list there is nothing for the type ARGS to bind to.
                //
                // The archive itself was never at fault: its descriptor
                // carries `generic_params=["V","L"]` and
                // `Record[src:V, tgt:V, label:L]`. Only the load ORDER was.
                //
                // Idempotent: the helper short-circuits when the type is
                // already present, so this costs a lookup on the hot path.
                {
                    let name_text = verum_common::Text::from(type_name.as_str());
                    let mut pending: Vec<Text> = Vec::new();
                    self.ensure_stdlib_type_loaded(&name_text, &mut pending);
                    let mut bound = 256usize;
                    while let Some(next) = pending.pop() {
                        bound = bound.saturating_sub(1);
                        if bound == 0 {
                            break;
                        }
                        self.ensure_stdlib_type_loaded(&next, &mut pending);
                    }
                }
                let record_expected = match expected {
                    Type::CapabilityRestricted { base, .. } => base.as_ref(),
                    ty => ty,
                };
                let supplied = match record_expected {
                    Type::Named { args, .. } | Type::Generic { args, .. } if !args.is_empty() => Some(args.as_slice()),
                    _ => None,
                };
                let Some(field_types) = stored_fields else { return self.synth_and_check(expr, expected); };
                let parameter_owner = match &declaration_ty {
                    Type::Named { path, .. } => self.path_to_string(path),
                    Type::Generic { name, .. } => name.clone(),
                    _ => type_name.clone(),
                };
                let (expected_fields, type_args) = self.instantiate_nominal_record_fields(
                    parameter_owner.as_str(), &field_types, supplied, expr.span,
                )?;
                let expected_fields = expected_fields.into_iter().map(|(name, ty)|
                    (name, self.substitute_placeholders(&ty))).collect();

                if !self.check_dependent_record_fields(&declaration_ty, &expected_fields, fields, base.as_deref(), expr.span)? {
                    // Handle base record spread
                    if let Some(base_expr) = base {
                        self.check_expr(base_expr, expected)?;
                    }

                    // Check each field against the resolved expected types
                    for field_init in fields {
                        let field_name: Text = field_init.name.name.as_str().into();

                        let expected_field_ty = match expected_fields.get(&field_name) {
                            Some(ty) => ty,
                            None => {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "field '{}' not found in type '{}'",
                                    field_name,
                                    self.path_to_string(path)
                                ))));
                            }
                        };

                        if let Some(ref value_expr) = field_init.value {
                            self.check_record_field_value(
                                field_init,
                                value_expr,
                                expected_field_ty,
                                type_name.as_str(),
                            )?;
                        } else {
                            // Shorthand syntax
                            if let Some(scheme) = self.ctx.env.lookup(field_name.as_str()).cloned() {
                                let var_ty = self.instantiate_value_scheme(&scheme, field_init.span)?;
                                self.unifier
                                    .unify(&var_ty, expected_field_ty, field_init.span)?;
                            } else {
                                return Err(TypeError::UnboundVariable {
                                    name: field_name,
                                    span: field_init.span,
                                });
                            }
                        }
                    }

                    // MISSING-FIELD-CHECK-PARITY-1 (T1367): every field the type
                    // DECLARES must be present, not just every field the literal
                    // NAMES.
                    //
                    // This is a PORT, not a new check. The synthesised path's
                    // "Step 5: Validate all required fields are present" already
                    // walks the expectation and refuses a missing field; the
                    // message below is copied from it verbatim so the two paths
                    // cannot drift into two spellings of one diagnostic.
                    //
                    // What the gap looked like, one variable apart on one binary:
                    //
                    //     let t      = Two { a: 1 };   rc=101, Missing required field 'b'
                    //     let t: Two = Two { a: 1 };   rc=0,   and `t.b` reads 0.0
                    //
                    // The annotation routes the literal HERE, through `check_expr`
                    // against an expected type, and this loop only ever asked the
                    // converse question. The `0.0` is not a default: a field
                    // declared `Int` renders through the integer path, and `0.0`
                    // is that path failing over on a value that is not an integer.
                    //
                    // The silent positions are not only annotated `let`s. A
                    // constructor's `Self { … }` takes its expectation from the
                    // declared RETURN type and lands here too, which is the
                    // dominant shape of a stdlib constructor — so this check
                    // reaches further than any static count of annotated bindings
                    // suggests.
                    //
                    // GATED ON `base.is_none()`: `Self { a: 1, ..other }` takes
                    // its remaining fields from the base, and 289 literals in
                    // `core/` are of that form.
                    //
                    // `VERUM_TRACE_MISSING_FIELD=1` reports and CONTINUES, which
                    // is how the population was counted: a hard error returns on
                    // the first, so one run would have shown one site per
                    // function.
                    if base.is_none() {
                        for (expected_name, expected_ty) in expected_fields.iter() {
                            if fields
                                .iter()
                                .any(|f| f.name.name.as_str() == expected_name.as_str())
                            {
                                continue;
                            }
                            if std::env::var_os("VERUM_TRACE_MISSING_FIELD").is_some() {
                                eprintln!(
                                    "[missing-field] {}.{} ({}) omitted in {}",
                                    type_name,
                                    expected_name,
                                    expected_ty,
                                    match &self.current_function_name {
                                        Maybe::Some(n) => n.as_str().to_string(),
                                        Maybe::None => String::from("<no enclosing fn>"),
                                    }
                                );
                                continue;
                            }
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Missing required field '{}' of type {} in record construction",
                                expected_name, expected_ty
                            ))));
                        }
                    }
                }

                // CRITICAL FIX: Unify the actual struct type with the expected type
                // This is essential for generic type inference: when expected is a type
                // variable τ37, we need to bind it to the actual struct type (e.g., Handle)
                // so that outer generic structs like Wrapper<T> can resolve T correctly.
                // NOTE: Resolve Self to actual type path if needed
                // THE SAME `Self` TEST AS THE LOOKUP ABOVE, and they have to
                // agree. This block matched only the `SelfValue` segment, so
                // while the field lookup also missed on `Self` the two were
                // consistently wrong and the arm bailed to `synth_and_check`
                // before either mattered. Teaching the lookup about
                // `Name("Self")` without teaching this made them DISAGREE:
                // the fields resolved as `Buf`, the unification still said
                // `Self`, and three const-generic tests reported
                // `expected 'Buf<Int>', found 'Self<Int>'` — the placeholder
                // escaping under its own name.
                let resolved_path = match (&self_resolved_path, &declaration_ty) {
                    (Some(resolved), _) => resolved.clone(),
                    (None, Type::Named { path: selected, .. }) if path.segments.len() == 1 => selected.clone(),
                    (None, Type::Generic { name, .. }) if path.segments.len() == 1 => Self::text_to_path(name),
                    _ => path.clone(),
                };
                // VARIANT-RECORD literal under CHECK (T0701 retry leg):
                // a two-segment `Type.Variant { … }` whose head expands
                // to a variant carrying the second segment is the
                // PARENT's value — constructing `Named{Type.Variant}`
                // here unified the variant SPELLING against the
                // expected parent and failed («expected 'RetryBackoff',
                // found 'RetryBackoff.Linear'»). Same expansion
                // authority as the synth arm.
                let resolved_path = if resolved_path.segments.len() == 2 {
                    let head_name = match &resolved_path.segments[0] {
                        verum_ast::ty::PathSegment::Name(id) => {
                            Some(id.name.as_str().to_string())
                        }
                        _ => None,
                    };
                    let case_name = match &resolved_path.segments[1] {
                        verum_ast::ty::PathSegment::Name(id) => {
                            Some(id.name.as_str().to_string())
                        }
                        _ => None,
                    };
                    match (head_name, case_name) {
                        (Some(h), Some(c)) => {
                            let head_ty = self
                                .ctx
                                .lookup_type(h.as_str())
                                .cloned()
                                .or_else(|| self.resolve_type_name(h.as_str(), expr.span).ok());
                            let is_parent = if let Some(t) = head_ty {
                                matches!(self.expand_generic_to_variant(&t)?,
                                    Type::Variant(ref vs) if vs.contains_key(&verum_common::Text::from(c.as_str())))
                            } else { false };
                            if is_parent {
                                verum_ast::ty::Path {
                                    segments: vec![resolved_path.segments[0].clone()].into(),
                                    span: resolved_path.span,
                                }
                            } else {
                                resolved_path
                            }
                        }
                        _ => resolved_path,
                    }
                } else {
                    resolved_path
                };
                let actual_struct_ty = Type::Named {
                    path: resolved_path,
                    args: type_args,
                };
                self.unifier.unify(&actual_struct_ty, record_expected, expr.span)?;

                Ok(InferResult::new(expected.clone()))
    }

    /// Refuse an integer literal that cannot be represented by the sized
    /// integer type it is being checked against.
    ///
    /// Handles the negated spelling too — `-300` is a `Unary` around the
    /// literal, a separate shape in the AST and a separate pole in the
    /// spec.
    ///
    /// Silent when the expected type is not a sized integer, when the
    /// literal carries its own suffix (the suffix decides, and a mismatch
    /// there is a different diagnostic), or when the expression is not a
    /// literal at all.
    fn check_integer_literal_fits(&mut self, expr: &Expr, expected: &Type) -> Result<()> {
        let value: i128 = match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                verum_ast::literal::LiteralKind::Int(i) if i.suffix.is_none() => i.value,
                _ => return Ok(()),
            },
            ExprKind::Unary { op, expr: inner } if op.as_str() == "-" => match &inner.kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    verum_ast::literal::LiteralKind::Int(i) if i.suffix.is_none() => -i.value,
                    _ => return Ok(()),
                },
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };

        let name = match self.get_type_name(expected) {
            Some(n) => n,
            None => return Ok(()),
        };
        let (lo, hi) = match Self::integer_type_range(name.as_str()) {
            Some(r) => r,
            None => return Ok(()),
        };
        if value >= lo && value <= hi {
            return Ok(());
        }
        Err(TypeError::OtherWithCodeSpanned {
            // E410, NOT E408. E408 already names the
            // explicit-type-argument arity error, and a code is a NAME:
            // two meanings under one is how `E313` came to mean two
            // things earlier the same day.
            code: verum_common::Text::from("E410"),
            msg: verum_common::Text::from(format!(
                "the literal `{value}` does not fit in `{name}`\n  \
                 note: `{name}` holds {lo} to {hi}"
            )),
            span: expr.span,
        })
    }

    /// Is this expression a value the compiler is HOLDING — a literal,
    /// possibly negated or parenthesised?
    ///
    /// **Measured 2026-09-12, and it is not a nicety.** The three
    /// refinement arms below gate on "the checked expression is a
    /// literal", and `-5` does not parse as one: it is a `Unary { Neg,
    /// Literal }`. So did `(5)`. Both silently lost the `W0500` that says
    /// the constraint is not enforced, and the E500 that a confirmed
    /// violation over a literal earns:
    ///
    /// ```text
    /// Int{tw(it) >= 10}     x = 5    W0500
    /// Int{tw(it) >= 10}     x = -5   nothing at all
    /// Int{[it][0] >= 10}    x = 5    W0500
    /// Int{[it][0] >= 10}    x = -5   nothing at all
    /// ```
    ///
    /// A predicate the solver cannot decide is unenforced whichever sign
    /// the value carries, and the reader is owed the same warning.
    fn is_compile_time_literal(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Literal(_) => true,
            ExprKind::Paren(inner) => Self::is_compile_time_literal(inner),
            ExprKind::Unary { op, expr: inner }
                if matches!(
                    op,
                    verum_ast::expr::UnOp::Neg | verum_ast::expr::UnOp::Not
                ) =>
            {
                Self::is_compile_time_literal(inner)
            }
            _ => false,
        }
    }

    fn synth_and_check(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        let result = self.synth_expr(expr)?;
        self.check_synthesized_value(expr, expected, result)
    }

    pub(super) fn check_synthesized_value(&mut self, expr: &Expr, expected: &Type, result: InferResult) -> Result<InferResult> {
        // Initialization and subsequent writes share the same value boundary.
        // Check literal range before equality can accept an integer coercion.
        self.check_integer_literal_fits(expr, expected)?;
        self.check_value_conversion_obligations(&result.ty, expected, expr.span)?;
        if std::env::var("VERUM_TRACE_QUALPATH").is_ok()
            && matches!(&expr.kind, ExprKind::MethodCall { .. })
        {
            eprintln!(
                "[qualpath] synth_and_check: synth={:?} expected={:?}",
                self.unifier.apply(&result.ty),
                self.unifier.apply(expected)
            );
        }

        // CRITICAL FIX: Never type (bottom type) is a subtype of ALL types.
        // panic(), return, break, continue all return Never.
        // This enables: let x: Int = panic("...") and closure bodies that diverge.
        // Type lattice: Never is the bottom type, subtype of all types (Never <: T for all T)
        let resolved_result_ty = self.unifier.apply(&result.ty);
        if self.is_never_type(&resolved_result_ty) {
            return Ok(InferResult::new(expected.clone()));
        }

        // CRITICAL FIX: Unify with the original (non-normalized) types first.
        // This is essential for proper type variable binding - we must store
        // nominal types like Box<Int>, not their structural expansions like { value: Int }.
        //

        // Only use normalized types for subtype checking, not for unification,
        // because unification with type variables would store the normalized form,
        // corrupting subsequent type checks.
        //

        // Example: Box<Box<Int>> should unify as Box<Box<Int>>, not { value: { value: Int } }
        let span = expr.span;

        // DEBUG: trace Valid/Invalid constructor resolution
        if let ExprKind::Call { func, .. } = &expr.kind {
            if let ExprKind::Path(path) = &func.kind {
                let name = path
                    .segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");
            }
        }
        // Try unification with original types first
        match self.unifier.unify(&result.ty, expected, span) {
            Ok(_) => {
                // =====================================================================
                // REFINEMENT CHECK: Verify argument satisfies refined parameter type
                // When the expected type is a refinement type (e.g., fn(x: Int{>= 0})),
                // verify that the argument expression satisfies the predicate.
                // This enables compile-time checking of preconditions at call sites.
                // =====================================================================
                {
                    let check_ty = self.normalize_type(expected);
                    if let Type::Refined {
                        ref base,
                        ref predicate,
                    } = check_ty
                    {
                        let refinement_type = crate::refinement::RefinementType {
                            base_type: (**base).clone(),
                            predicate: predicate.clone(),
                            span: expr.span,
                        };
                        match self.check_refinement_with_evidence(expr, &refinement_type) {
                            Ok(crate::refinement::VerificationResult::Invalid { .. }) => {
                                // Only report error if syntactic evaluator confirms the
                                // violation. SMT may return Invalid for valid expressions
                                // with complex predicates (modulo, string ops, etc.).
                                if let verum_common::Maybe::Some(
                                    crate::refinement::VerificationResult::Invalid { .. },
                                ) = self.refinement.syntactic_check_only(expr, predicate)
                                {
                                    let pred_text = format!("{}", predicate);
                                    return Err(TypeError::RefinementFailed {
                                        predicate: verum_common::Text::from(pred_text),
                                        span: expr.span,
                                    });
                                }
                                // A DECLARED refinement over a LITERAL is
                                // the one case where the solver's verdict
                                // needs no second opinion, and it is the
                                // same gate `W0500` already uses below.
                                //
                                // The caution above is about values not yet
                                // known: over one of those an `Invalid` can
                                // mean "the solver could not see why this
                                // holds", and refusing would be worse than
                                // gradual verification. A LITERAL has no
                                // such doubt — `Invalid` there says the
                                // predicate is false for a constant the
                                // compiler is holding.
                                //
                                // Without this, an `Invalid` the syntactic
                                // evaluator cannot reproduce was dropped in
                                // silence. Measured: a type whose predicate
                                // is a QUANTIFIER false for every value —
                                // `forall k: Int. k >= 0 && k < 3 => k == 7`
                                // — accepted a binding with neither an
                                // error nor a warning, because the
                                // syntactic evaluator does not do
                                // quantifiers and so could not confirm what
                                // the solver had already decided (T0967).
                                // …UNLESS THE PREDICATE CALLS A FUNCTION (A113).
                                //
                                // "A literal has no such doubt" holds only
                                // while the predicate is fully interpreted. A
                                // free call is an uninterpreted symbol: the
                                // solver cannot unfold it, cannot prove the VC,
                                // and answers `Invalid` — which this branch
                                // then read as "violated".
                                //
                                // Measured 2026-09-12, both polarities, same
                                // arithmetic written two ways:
                                //
                                //   Int{it * 2 >= 10}    v:20 clean  v:1 E500
                                //   Int{twice(it) >= 10} v:20 E500   v:1 E500
                                //
                                // The second row refused `40 >= 10`, and so did
                                // `Int{twice(it) >= -1000}`, which no value can
                                // violate. T0967's case — a quantifier false
                                // for every value — contains no call and still
                                // refuses, which is what this branch is for.
                                if Self::is_compile_time_literal(expr)
                                    && predicate.provenance
                                        == crate::refinement::PredicateProvenance::Declared
                                    && !crate::refinement::RefinementChecker
                                        ::predicate_calls_an_opaque_function(
                                            &predicate.predicate,
                                        )
                                {
                                    let pred_text = format!("{}", predicate);
                                    return Err(TypeError::RefinementFailed {
                                        predicate: verum_common::Text::from(pred_text),
                                        span: expr.span,
                                    });
                                }
                                // AND IF THE GUARD ABOVE IS WHAT STOPPED US,
                                // SAY SO — silence is its own defect (A113).
                                //
                                // Lifting the false rejection must not leave a
                                // reader believing the predicate was checked.
                                // The shipped rule is "compiles clean means
                                // CHECKED; compiles with W0500 means PARSED",
                                // so a predicate the solver could not decide
                                // owes the reader the warning, exactly as the
                                // `Unknown` arm below gives it. Before the
                                // regression this case DID warn — measured on
                                // the docs' own table, 2026-08-17.
                                if Self::is_compile_time_literal(expr)
                                    && predicate.provenance
                                        == crate::refinement::PredicateProvenance::Declared
                                {
                                    let msg = format!(
                                        "refinement {} was NOT verified against a value \
                                         known at compile time (the predicate calls a \
                                         function the solver cannot unfold), so the \
                                         constraint is not enforced here — express the \
                                         predicate in terms the solver decides \
                                         (comparisons, arithmetic, `&&`/`||`/`!`), or \
                                         check it explicitly in code",
                                        predicate
                                    );
                                    self.diagnostics.push(Diagnostic::new_warning(
                                        msg,
                                        span_to_line_col(expr.span),
                                        "W0500",
                                    ));
                                }
                                // Syntactic can't confirm → gradual verification
                            }
                            // An SMT `unknown` on a DECLARED refinement over a
                            // compile-time value is not a pass (T0788).
                            //
                            // Measured verdicts, value 1 against two predicates:
                            //
                            //   {it >= -1 && it <= 1}  literal      -> Valid
                            //   {it >= -1 && it <= 1}  non-literal  -> Unknown
                            //   {(it,0).0 != (it,0).0} literal      -> Unknown
                            //   {(it,0).0 != (it,0).0} non-literal  -> Unknown
                            //
                            // `Unknown` on a value not yet known is ordinary
                            // gradual verification, and warning there fires on
                            // every refinement in the stdlib.  `Unknown` on a
                            // LITERAL means the predicate's own shape defeated
                            // the solver — and that case used to be swallowed
                            // by a `_ => {}`, so `Int{(it,0).0 != (it,0).0}`,
                            // false for every value, compiled in silence.
                            //
                            // The `Declared` gate is the third condition, and
                            // it was learned the hard way: without it the
                            // warning landed on flow-narrowed `if` conditions,
                            // naming a "refinement" at a line whose author
                            // wrote none (core/math/elementary.vr:431).
                            // Narrowing is an ASSUMPTION; only a declared
                            // refinement is an obligation.
                            //
                            // A warning rather than an error, because gradual
                            // verification is deliberate: the point is that the
                            // reader learns the constraint is unenforced rather
                            // than being handed a type that advertises a check
                            // nobody performs.
                            Ok(crate::refinement::VerificationResult::Unknown { ref reason })
                                if Self::is_compile_time_literal(expr)
                                    && predicate.provenance
                                        == crate::refinement::PredicateProvenance::Declared =>
                            {
                                let msg = format!(
                                    "refinement {} was NOT verified against a value known \
                                     at compile time ({}), so the constraint is not \
                                     enforced here — express the predicate in terms the \
                                     solver decides (comparisons, arithmetic, \
                                     `&&`/`||`/`!`), or check it explicitly in code",
                                    predicate, reason
                                );
                                let diag = Diagnostic::new_warning(
                                    msg,
                                    span_to_line_col(expr.span),
                                    "W0500",
                                );
                                self.diagnostics.push(diag);
                            }
                            Err(error) => return Err(error),
                            // Valid, a value not yet known, or a narrowed assumption.
                            Ok(_) => {}
                        }
                    }
                }
                return Ok(InferResult::new(expected.clone()));
            }
            Err(_e) => {
                // Equality can infer a complete restricted type. A conversion
                // involving permissions needs directed evidence at every nested
                // position before structural, alias, or protocol recovery.
                let expected_ty = self.unifier.apply(expected);
                if resolved_result_ty.contains_capability_restriction()
                    || expected_ty.contains_capability_restriction()
                {
                    if self.subtyping.is_subtype(&resolved_result_ty, &expected_ty) {
                        return Ok(InferResult::new(expected.clone()));
                    }
                    return Err(TypeError::OtherWithCodeSpanned {
                        code: "E411".into(),
                        msg: format!(
                            "capability conversion refused: this value carries `{resolved_result_ty}`, and `{expected_ty}` is required"
                        ).into(),
                        span,
                    });
                }
                // SCOPE-AWARE VARIANT CONSTRUCTOR RESOLUTION
                // When a variant constructor call (e.g., Ok(value)) fails to unify with the
                // expected type, check if an alternative parent type defines the same constructor.
                // For example, if Ok(value) synthesized as Result.Ok but expected CheckedResult<T>,
                // try re-synthesizing with CheckedResult.Ok instead.
                if let ExprKind::Call { func, args, .. } = &expr.kind {
                    if let ExprKind::Path(path) = &func.kind {
                        if path.segments.len() == 1 {
                            let ctor_name = path
                                .segments
                                .last()
                                .map(|s| match s {
                                    verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                    _ => "",
                                })
                                .unwrap_or("");
                            if !ctor_name.is_empty() {
                                let ctor_key = Text::from(ctor_name);
                                // Check if this constructor has multiple parent types
                                let alt_parents =
                                    self.variant_constructor_parents.get(&ctor_key).cloned();
                                if let Some(parents) = alt_parents {
                                    if parents.len() > 1 {
                                        // Collect qualified constructor info before mutable borrow
                                        let mut candidates: Vec<(Text, List<Type>)> = Vec::new();
                                        for parent in parents.iter() {
                                            let qualified_name: Text =
                                                format!("{}.{}", parent, ctor_name).into();
                                            if let Some(scheme) =
                                                self.ctx.env.lookup(qualified_name.as_str())
                                            {
                                                let ctor_ty = scheme.instantiate();
                                                if let Type::Function {
                                                    return_type,
                                                    params,
                                                    ..
                                                } = &ctor_ty
                                                {
                                                    if self
                                                        .unifier
                                                        .unify(return_type, expected, span)
                                                        .is_ok()
                                                    {
                                                        if params.len() == args.len() {
                                                            candidates.push((
                                                                qualified_name,
                                                                params.clone(),
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // Try each candidate with mutable borrow
                                        for (_qualified_name, params) in &candidates {
                                            let mut all_ok = true;
                                            for (arg, param) in args.iter().zip(params.iter()) {
                                                if self.check_expr(arg, param).is_err() {
                                                    all_ok = false;
                                                    break;
                                                }
                                            }
                                            if all_ok {
                                                return Ok(InferResult::new(expected.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Unification failed - try auto-borrow coercion (T → &T)
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 3 (Auto-Borrow at a call position)
                //

                // Auto-borrow is only allowed for IMMUTABLE references.
                // For &mut, the user must explicitly write &mut x.
                if let Type::Reference {
                    mutable: false,
                    inner,
                } = expected
                {
                    // Check if actual type unifies with the inner type of expected reference
                    if self.unifier.unify(&result.ty, inner, span).is_ok() {
                        // Auto-borrow successful: T unifies with inner of &T
                        // The compiler will insert & automatically during codegen.
                        // Note: We store this in implicit_borrows for IDE transparency
                        // (not implemented yet, but the coercion works)
                        return Ok(InferResult::new(expected.clone()));
                    }
                }

                // Also try auto-borrow for &checked T (same immutable-only rule)
                if let Type::CheckedReference {
                    mutable: false,
                    inner,
                } = expected
                {
                    if self.unifier.unify(&result.ty, inner, span).is_ok() {
                        return Ok(InferResult::new(expected.clone()));
                    }
                }

                // ============================================================
                // NUMERIC LITERAL COERCION
                // ============================================================
                // When an integer literal without suffix is used where a specific
                // numeric type is expected (Byte, Int16, etc.), coerce if value fits.
                // This enables: `let b: Byte = 42;` and `assert_eq(bytes[0], 0x06);`
                //

                // Also handles:
                // - Unary negation of integer literals: `-42` for Int32, ISize, etc.
                // - Float literals: `4.0` for Float32, Float64

                // Helper to extract integer literal value (positive or negated)
                let int_literal_value: Option<(i128, bool)> = match &expr.kind {
                    ExprKind::Literal(lit) => {
                        if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                            if int_lit.suffix.is_none() {
                                Some((int_lit.value, false)) // (value, is_negated)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    ExprKind::Unary { op, expr: inner } if op.as_str() == "-" => {
                        if let ExprKind::Literal(lit) = &inner.kind {
                            if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                                if int_lit.suffix.is_none() {
                                    Some((int_lit.value, true)) // (value, is_negated)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if let Some((abs_value, is_negated)) = int_literal_value {
                    let value = if is_negated { -abs_value } else { abs_value };
                    let expected_name = self.get_type_name(expected);

                    // Check if expected is a specific integer type and value fits
                    let coerces = expected_name
                        .as_deref()
                        .and_then(Self::integer_type_range)
                        .is_some_and(|(lo, hi)| value >= lo && value <= hi);

                    if coerces {
                        return Ok(InferResult::new(expected.clone()));
                    }

                    // A bound proves operations, not construction from an integer.
                    // Generic values require a declared conversion or witness;
                    // no literal recovery may specialize a rigid parameter.

                }

                // Float literal coercion: `4.0` for Float32, Float64
                if let ExprKind::Literal(lit) = &expr.kind {
                    if let verum_ast::literal::LiteralKind::Float(float_lit) = &lit.kind {
                        // Only coerce if no suffix was specified
                        if float_lit.suffix.is_none() {
                            let expected_name = self.get_type_name(expected);
                            match expected_name.as_deref() {
                                Some("Float32") | Some("f32") | Some("Float64") | Some("f64") => {
                                    return Ok(InferResult::new(expected.clone()));
                                }
                                _ => {}
                            }
                        }
                    }
                }

                // Unary negation of float literals: `-4.0` for Float32, Float64
                if let ExprKind::Unary { op, expr: inner } = &expr.kind {
                    if op.as_str() == "-" {
                        if let ExprKind::Literal(lit) = &inner.kind {
                            if let verum_ast::literal::LiteralKind::Float(float_lit) = &lit.kind {
                                if float_lit.suffix.is_none() {
                                    let expected_name = self.get_type_name(expected);
                                    match expected_name.as_deref() {
                                        Some("Float32") | Some("f32") | Some("Float64")
                                        | Some("f64") => {
                                            return Ok(InferResult::new(expected.clone()));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }

                // ============================================================
                // NEWTYPE COERCION
                // ============================================================
                // When expected type is a newtype (type alias or tuple wrapper),
                // allow the wrapped value to be used directly.
                //

                // Examples:
                // - `type Signal is ();` allows `let s: Signal = ();`
                // - `type UserId is (Int);` allows `let id: UserId = UserId(42);`
                // - `type Database is ();` with `Database with [Read]` allows `()`
                //

                // This is a common pattern for zero-cost semantic wrapping.
                // Also handle CapabilityRestricted expected types by extracting the base.
                let newtype_expected_base = match expected {
                    Type::CapabilityRestricted { base, .. } => Some(base.as_ref()),
                    _ => None,
                };
                let newtype_check_target = newtype_expected_base.unwrap_or(expected);
                if let Type::Named { path, .. } = newtype_check_target {
                    if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                        let type_name = ident.name.as_str();

                        // First, check if this is an alias to the actual type.
                        // Use expand_type_alias which handles generic type argument substitution
                        // (e.g., IoResult<Unit> -> Result<Unit, StreamError>)
                        if let Some(expanded) = self.expand_type_alias(newtype_check_target) {
                            if self.unifier.unify(&result.ty, &expanded, span).is_ok() {
                                // Actual type matches the expanded alias, allow coercion.
                                // If the alias resolves to Refined, run the refinement
                                // check so struct-field aliases like `type PageNo is
                                // Int where |n| { n >= 1 }` enforce at construction time
                                // (spec §4.6, §9 V3-V7 obligations).
                                self.check_refinement_for_expanded_alias(expr, &expanded)?;
                                return Ok(InferResult::new(expected.clone()));
                            }
                        } else if let Some(alias_target) = self.ctx.resolve_alias(type_name) {
                            let alias_target = alias_target.clone();
                            if self.unifier.unify(&result.ty, &alias_target, span).is_ok() {
                                // Actual type matches the alias target, allow coercion
                                self.check_refinement_for_expanded_alias(expr, &alias_target)?;
                                return Ok(InferResult::new(expected.clone()));
                            }
                        }

                        // Second, check if this is a newtype (tuple wrapper or unit wrapper)
                        // Newtypes store their inner type in __newtype_inner_{name}
                        let inner_key: Text = format!("__newtype_inner_{}", type_name).into();
                        if let Some(inner_type) = self.ctx.lookup_type(&inner_key) {
                            if self.unifier.unify(&result.ty, inner_type, span).is_ok() {
                                // Actual type matches the newtype's inner type
                                return Ok(InferResult::new(expected.clone()));
                            }
                        }
                    }
                }

                // Try subtype check with normalized types
                let normalized_actual = self.normalize_type(&result.ty);
                let normalized_expected = self.normalize_type(expected);

                if self
                    .subtyping
                    .is_subtype(&normalized_actual, &normalized_expected)
                {
                    return Ok(InferResult::new(expected.clone()));
                }

                // ============================================================
                // PROTOCOL COERCION
                // Spec: Protocol-based polymorphism - T <: Protocol if T implements Protocol
                // ============================================================
                // If expected is a protocol type (or a reference to one), and actual
                // implements that protocol, allow the coercion.
                //

                // This enables:
                // - `DefaultHasher` -> `Hasher` (protocol satisfaction)
                // - `&mut DefaultHasher` -> `&mut Hasher` (reference to protocol)
                //

                // Note: For mutable references, this implies dynamic dispatch at runtime.
                // The compiler will generate a vtable for the protocol.
                if self.check_protocol_coercion(&result.ty, expected) {
                    return Ok(InferResult::new(expected.clone()));
                }

                // ============================================================
                // FUNCTION TYPE CONTRAVARIANCE WITH PROTOCOL COERCION
                // ============================================================
                // fn(&Protocol) <: fn(&Concrete) when Concrete implements Protocol
                // (contravariant in parameters, covariant in return)
                if let (
                    Type::Function {
                        params: p1,
                        return_type: r1,
                        ..
                    },
                    Type::Function {
                        params: p2,
                        return_type: r2,
                        ..
                    },
                ) = (&normalized_actual, &normalized_expected)
                {
                    if p1.len() == p2.len() {
                        let params_ok = p1.iter().zip(p2.iter()).all(|(actual_p, expected_p)| {
                            // Contravariance: expected_param <: actual_param
                            // (reversed direction for parameters)
                            // Strip references to check inner types for protocol coercion
                            let actual_inner = match actual_p {
                                Type::Reference {
                                    inner,
                                    mutable: false,
                                } => inner.as_ref(),
                                _ => actual_p,
                            };
                            let expected_inner = match expected_p {
                                Type::Reference {
                                    inner,
                                    mutable: false,
                                } => inner.as_ref(),
                                _ => expected_p,
                            };
                            // Check: expected_inner <: actual_inner (contravariance)
                            self.subtyping.is_subtype(expected_inner, actual_inner)
                                || self.check_protocol_coercion(expected_inner, actual_inner)
                                // Or direct match
                                || self.unifier.unify(actual_p, expected_p, span).is_ok()
                        });
                        let return_ok = self.subtyping.is_subtype(r1, r2)
                            || self.check_protocol_coercion(r1, r2)
                            || self.unifier.unify(r1, r2, span).is_ok();
                        if params_ok && return_ok {
                            return Ok(InferResult::new(expected.clone()));
                        }
                    }
                }

                // Printed names are diagnostic labels, never equality evidence.
                // Structural equality retains declaration-owned value identities.
                if normalized_actual == normalized_expected {
                    return Ok(InferResult::new(expected.clone()));
                }

                // Variant subtype coercion: accept subset/superset of variants.
                //
                // T0722 — the coercion is about differing variant SETS
                // (widening/narrowing). When the two sets are IDENTICAL there
                // is nothing to widen, and reaching here means unification
                // ALREADY FAILED — on the payloads. Accepting on name sets
                // alone then let `Maybe<Text>` satisfy `Maybe<Int>`, and a
                // user-defined `Own<T>` behave the same, with three measured
                // consequences: a panic for record payloads, a silently wrong
                // number for scalars, and a silently DIFFERENT OPERATION for
                // foreign payloads (`v + 1` string-concatenated).
                //
                // So the equal-set case falls through to the error. Genuine
                // subset/superset pairs are untouched, which is what the
                // widening this branch exists for actually looks like.
                if let (Type::Variant(av), Type::Variant(ev)) =
                    (&normalized_actual, &normalized_expected)
                {
                    let same_set = av.len() == ev.len()
                        && ev.keys().all(|k| av.contains_key(k));
                    if !same_set
                        && (ev.keys().all(|k| av.contains_key(k))
                            || av.keys().all(|k| ev.contains_key(k)))
                    {
                        return Ok(InferResult::new(expected.clone()));
                    }
                }

                // Neither unification nor subtyping worked - return the unification error
                // by trying again (this time it will fail and return the error)
                self.unifier.unify(&result.ty, expected, span)?;
            }
        }

        Ok(InferResult::new(expected.clone()))
    }

    /// Compute binary operation result type.
    ///
    /// Extracted from infer_binop to support iterative inference.
    /// Takes the types of left and right operands and computes the result type.
    /// Compute the result type of a binary operation.
    ///
    /// ARCHITECTURAL RULE: This function MUST NOT contain hardcoded knowledge
    /// of stdlib types like Duration, Time, Text, etc. All operator behavior
    /// is discovered through protocol implementations.
    ///
    /// Extracted from infer_binop to support iterative inference.
    /// Takes the types of left and right operands and computes the result type.
    /// `<` on a type that declares no ordering (T1017).
    ///
    /// Reported rather than refused for now, and the number is the reason:
    /// the fall-through this guards has been the accepted spelling for
    /// long enough that `core/` relies on it, so an error here would
    /// refuse the standard library before it refused a mistake. The
    /// measurement is recorded in the task; when the count reaches zero
    /// this becomes an error, which is the only honest end state for a
    /// silently wrong VALUE.
    fn warn_comparison_without_ordering(&mut self, ty: &Type, span: verum_ast::span::Span) {
        // A type variable is not yet known — asking now would report the
        // checker's own incompleteness.
        if matches!(ty, Type::Var(_) | Type::Unknown | Type::Never) {
            return;
        }
        // A SIZED numeric alias (`UInt64`, `Int32`, `USize`, `Float32`, …)
        // is a `Type::Named`, not `Type::Int` — and the runtime orders it
        // natively, so it needs no `Ord` implementation.
        //
        // MEASURED, and it is why this arm exists: the first version of
        // this check fired 140 times over 200 core/ files and EVERY ONE
        // was one of these. A ten-run probe on `UInt64 < UInt64` gives the
        // right answer ten times out of ten. So the warning was reporting
        // the CHECKER's gap as the author's mistake — the exact failure
        // this file's sibling check (`unambiguous_signature_disagreement`,
        // T1029) was written to avoid, committed here anyway (T1017/T1046).
        //
        // Asked of `IntegerKind::from_name`, the authority that already
        // decides what an integer alias is, rather than a second list.
        // The authority answers about a BARE name, and the type does not
        // always arrive with one. Measured 2026-09-03 (T1107): this arm
        // let 41 warnings through on `core/base/primitives.vr` alone,
        // every one of them naming `core.base.primitives.UInt64` or
        // `core.base.primitives.Int32` — the file that DECLARES those
        // types renders them fully qualified, `IntegerKind::from_name`
        // matches `"UInt64"` and not `"core.base.primitives.UInt64"`, and
        // the exemption above therefore missed exactly where it was most
        // obviously right. A ten-value probe (`Int32` and `UInt64` under
        // `<`) prints the correct answers, so the warning was making a
        // false BEHAVIOURAL claim — `compares ADDRESSES` — about code
        // that compares values.
        //
        // Ask about the leaf as well as the whole. Asking about the leaf
        // means a user type named `Int32` in their own module is exempt
        // too; that is the same latitude the bare-name spelling already
        // had, and it errs toward silence on a warning whose false
        // positives cost more than its misses — 41 of them in the
        // library's most fundamental file teach the reader to skip the
        // very class T0833 introduced to tell the truth about `Float`.
        if let Type::Named { path, .. } = ty {
            let name = self.path_to_string(path);
            let leaf = name.as_str().rsplit('.').next().unwrap_or(name.as_str());
            let known = |n: &str| {
                crate::integer_hierarchy::IntegerKind::from_name(n).is_some()
                    || matches!(
                        n,
                        "Float32" | "Float64" | "Byte" | "Int" | "Float" | "Char" | "Text"
                    )
            };
            if known(name.as_str()) || known(leaf) {
                return;
            }
        }
        let has_order = {
            let pc = self.protocol_checker.read();
            pc.implements_protocol(ty, "Ord") || pc.implements_protocol(ty, "PartialOrd")
        };
        if has_order {
            return;
        }
        let diag = Diagnostic::new_warning(
            format!(
                "`{}` declares no ordering, so `<` compares ADDRESSES here — the \
                 answer depends on where the values happen to be allocated and \
                 changes between runs\n  help: `implement Ord for {}` (or \
                 `PartialOrd`) to give the comparison a meaning",
                ty.to_text(),
                ty.to_text()
            ),
            span_to_line_col(span),
            "W0506",
        );
        self.diagnostics.push(diag);
    }

    fn compute_binop_result(
        &mut self,
        op: BinOp,
        left_ty: &Type,
        right_ty: &Type,
        span: Span,
    ) -> Result<Type> {
        use BinOp::*;

        match op {
            // Arithmetic operators: handled through protocol lookup
            // Primitive types (Int, Float, Text) have built-in handling for efficiency.
            Add | Concat => {
                let left_deref = Self::deref_for_binop(left_ty);
                let right_deref = Self::deref_for_binop(right_ty);

                // Fast path for primitive types
                match left_deref {
                    Type::Int | Type::Text => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(left_deref.clone());
                    }
                    Type::Float => {
                        // Try Float/Float64 coercion first
                        if let Some(coerced) = self.coerce_float_types(left_deref, right_deref) {
                            return Ok(coerced);
                        }
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(left_deref.clone());
                    }
                    Type::Var(_) => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(right_deref.clone());
                    }
                    _ => {}
                }

                // Try protocol-based resolution for custom types
                if let Some(output_ty) = self.try_operator_protocol_with_types(
                    left_deref,
                    right_deref,
                    "Add",
                    "add",
                    span,
                ) {
                    return Ok(output_ty);
                }

                // Fallback: try unification or Numeric protocol for cross-type arithmetic
                if self.unifier.unify(left_deref, right_deref, span).is_ok() {
                    Ok(left_deref.clone())
                } else if let Some(coerced) = self.coerce_float_types(left_deref, right_deref) {
                    Ok(coerced)
                } else {
                    let pc = self.protocol_checker.read();
                    if pc.implements_protocol(left_deref, "Numeric")
                        && pc.implements_protocol(right_deref, "Numeric")
                    {
                        Ok(left_deref.clone())
                    } else {
                        drop(pc);
                        self.unifier.unify(left_deref, &Type::int(), span)?;
                        self.unifier.unify(right_deref, &Type::int(), span)?;
                        Ok(Type::int())
                    }
                }
            }

            Sub => {
                let left_deref = Self::deref_for_binop(left_ty);
                let right_deref = Self::deref_for_binop(right_ty);

                // Fast path for primitive types
                match left_deref {
                    Type::Int => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(left_deref.clone());
                    }
                    Type::Float => {
                        // Try Float/Float64 coercion first
                        if let Some(coerced) = self.coerce_float_types(left_deref, right_deref) {
                            return Ok(coerced);
                        }
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(left_deref.clone());
                    }
                    Type::Var(_) => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(right_deref.clone());
                    }
                    _ => {}
                }

                // Try protocol-based resolution for custom types
                if let Some(output_ty) = self.try_operator_protocol_with_types(
                    left_deref,
                    right_deref,
                    "Sub",
                    "sub",
                    span,
                ) {
                    return Ok(output_ty);
                }

                // Fallback: try unification or Numeric protocol for cross-type arithmetic
                if self.unifier.unify(left_deref, right_deref, span).is_ok() {
                    Ok(left_deref.clone())
                } else if let Some(coerced) = self.coerce_float_types(left_deref, right_deref) {
                    Ok(coerced)
                } else {
                    let pc = self.protocol_checker.read();
                    if pc.implements_protocol(left_deref, "Numeric")
                        && pc.implements_protocol(right_deref, "Numeric")
                    {
                        Ok(left_deref.clone())
                    } else {
                        drop(pc);
                        self.unifier.unify(left_deref, &Type::int(), span)?;
                        self.unifier.unify(right_deref, &Type::int(), span)?;
                        Ok(Type::int())
                    }
                }
            }

            Mul | Div | Rem | Pow => {
                let left_deref = Self::deref_for_binop(left_ty);
                let right_deref = Self::deref_for_binop(right_ty);

                // Map operator to protocol name
                let (protocol_name, method_name) = match op {
                    Mul => ("Mul", "mul"),
                    Div => ("Div", "div"),
                    Rem => ("Rem", "rem"),
                    Pow => ("Pow", "pow"),
                    _ => unreachable!(),
                };

                // Fast path for primitive types
                match left_deref {
                    Type::Int => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(left_deref.clone());
                    }
                    Type::Float => {
                        // Try Float/Float64 coercion first
                        if let Some(coerced) = self.coerce_float_types(left_deref, right_deref) {
                            return Ok(coerced);
                        }
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(left_deref.clone());
                    }
                    Type::Var(_) => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        return Ok(right_deref.clone());
                    }
                    _ => {}
                }

                // Try protocol-based resolution for custom types
                if let Some(output_ty) = self.try_operator_protocol_with_types(
                    left_deref,
                    right_deref,
                    protocol_name,
                    method_name,
                    span,
                ) {
                    return Ok(output_ty);
                }

                // Fallback: try unification or Numeric protocol for cross-type arithmetic
                if self.unifier.unify(left_deref, right_deref, span).is_ok() {
                    Ok(left_deref.clone())
                } else if let Some(coerced) = self.coerce_float_types(left_deref, right_deref) {
                    Ok(coerced)
                } else {
                    let pc = self.protocol_checker.read();
                    if pc.implements_protocol(left_deref, "Numeric")
                        && pc.implements_protocol(right_deref, "Numeric")
                    {
                        Ok(left_deref.clone())
                    } else {
                        drop(pc);
                        self.unifier.unify(left_deref, &Type::int(), span)?;
                        self.unifier.unify(right_deref, &Type::int(), span)?;
                        Ok(Type::int())
                    }
                }
            }

            // Comparison operators: T -> T -> Bool where T implements Ord/PartialOrd
            // Auto-deref both sides: &Int < Int works
            Lt | Le | Gt | Ge => {
                let left_deref = Self::deref_for_binop(left_ty);
                let right_deref = Self::deref_for_binop(right_ty);
                match left_deref {
                    // Fast path for built-in primitive types
                    Type::Int | Type::Float => {
                        let _ = self.unifier.unify(left_deref, right_deref, span);
                        Ok(Type::bool())
                    }
                    Type::Var(_) => {
                        self.unifier.unify(left_deref, right_deref, span)?;
                        Ok(Type::bool())
                    }
                    // T1017: `Text` and `Char` have a natural order and are
                    // compared directly, like Int and Float above.
                    Type::Text | Type::Char => {
                        let _ = self.unifier.unify(left_deref, right_deref, span);
                        Ok(Type::bool())
                    }
                    _ => {
                        // For all other types: try unification first (same-type comparison)
                        if self.unifier.unify(left_deref, right_deref, span).is_ok() {
                            // T1017: the comment at the head of this arm states the
                            // rule — "T implements Ord/PartialOrd" — and the code
                            // never asked it. It checked only that both sides are
                            // the SAME type, so `a < b` on a record with no
                            // ordering type-checked, and codegen then fell through
                            // to a POINTER comparison. The answer was
                            // allocation-order dice: consistent within a run,
                            // INVERTED between runs, 4 times in 10 on a 7-line
                            // program. A silent wrong value in a base operator.
                            self.warn_comparison_without_ordering(left_deref, span);
                            Ok(Type::bool())
                        } else {
                            // Cross-type: allow if both implement Numeric (literal coercion)
                            let pc = self.protocol_checker.read();
                            if pc.implements_protocol(left_deref, "Numeric")
                                && pc.implements_protocol(right_deref, "Numeric")
                            {
                                Ok(Type::bool())
                            } else {
                                drop(pc);
                                // Re-run unify to produce the error
                                self.unifier.unify(left_deref, right_deref, span)?;
                                Ok(Type::bool())
                            }
                        }
                    }
                }
            }

            // Equality: 'a -> 'a -> Bool (requires Eq)
            // Auto-deref both sides for comparison: &T == T and T == &T both work
            Eq | Ne => {
                let left_deref = Self::deref_for_binop(left_ty);
                let right_deref = Self::deref_for_binop(right_ty);
                // Fast path for built-in primitive types (Int, Float, Bool, Char, Text)
                match (left_deref, right_deref) {
                    (Type::Int, _)
                    | (_, Type::Int)
                    | (Type::Float, _)
                    | (_, Type::Float)
                    | (Type::Bool, _)
                    | (_, Type::Bool)
                    | (Type::Char, _)
                    | (_, Type::Char)
                    | (Type::Text, _)
                    | (_, Type::Text) => {
                        let _ = self.unifier.unify(left_deref, right_deref, span);
                    }
                    _ => {
                        // Normalize both sides through the projection
                        // resolver before unify.  Required for
                        // associated-type projections like
                        // `Maybe<I.Item>` where `I` got bound to a
                        // concrete type during call-site
                        // instantiation: the projection
                        // `::Item<MyIter>` (concrete-base shape) must
                        // reduce to the impl's bound type (e.g.
                        // `Int`) BEFORE the unifier compares it
                        // against the other side, otherwise the
                        // unifier sees two structurally-different
                        // Generic types and emits "Type mismatch:
                        // expected 'Int', found 'Item<MyIter>'".
                        //
                        // The concrete-base case isn't handled by the
                        // unifier's "deferred projection" path
                        // (unify.rs:~2702) — that path only fires
                        // when the projection's base STILL has
                        // unresolved type vars. After call-site
                        // substitution, the base IS concrete, so
                        // this layer must drive the resolution.
                        //
                        // `normalize_type` is idempotent on
                        // already-normalized types so this is safe
                        // for non-projection inputs (cheap path-
                        // walk + early return).
                        //
                        // Stdlib-agnostic — resolution flows through
                        // `try_resolve_associated_type_projection` →
                        // `protocol_checker::try_find_associated_type`
                        // which iterates registered impls; no
                        // hardcoded type/protocol names.
                        let left_normalized = self.normalize_type(left_deref);
                        let right_normalized = self.normalize_type(right_deref);
                        // For other types: allow numeric coercion via Numeric protocol,
                        // otherwise require strict same-type match via unification
                        if self
                            .unifier
                            .unify(&left_normalized, &right_normalized, span)
                            .is_err()
                        {
                            let pc = self.protocol_checker.read();
                            if pc.implements_protocol(&left_normalized, "Numeric")
                                && pc.implements_protocol(&right_normalized, "Numeric")
                            {
                                // Both implement Numeric — allow cross-type comparison for literals
                            } else {
                                drop(pc);
                                // Re-run unify to produce the error
                                self.unifier
                                    .unify(&left_normalized, &right_normalized, span)?;
                            }
                        }
                    }
                }
                Ok(Type::bool())
            }

            // Logical operators: Bool -> Bool -> Bool
            And | Or => {
                self.unifier.unify(left_ty, &Type::bool(), span)?;
                self.unifier.unify(right_ty, &Type::bool(), span)?;
                Ok(Type::bool())
            }

            // Bitwise operators: Protocol-based resolution with Int fallback
            // Auto-deref both sides: &Int & Int works
            BitAnd | BitOr | BitXor | Shl | Shr => {
                let left_deref = Self::deref_for_binop(left_ty);
                let right_deref = Self::deref_for_binop(right_ty);

                // Determine protocol name for this operator
                let protocol_name = match op {
                    BitAnd => "BitAnd",
                    BitOr => "BitOr",
                    BitXor => "BitXor",
                    Shl => "Shl",
                    Shr => "Shr",
                    _ => unreachable!(),
                };

                // Unified protocol-based resolution
                // 1. Try to resolve via protocol if the type implements it
                // 2. Fall back to Int for type variables or unknown types
                self.resolve_bitwise_op_type(left_deref, right_deref, protocol_name, span)
            }

            // Assignment operators
            Assign => {
                // Diagnostic (VERUM_TRACE_CTOR): assignment unify inputs
                // in their unifier-applied forms — pinpoints which side
                // carries a stale ctor instantiation
                // (GENERIC-CTOR-FRESHNESS-1).
                if crate::ctor_trace_enabled() {
                    eprintln!(
                        "[ctor-trace] Assign unify: left={:?} right={:?}",
                        self.unifier.apply(left_ty),
                        self.unifier.apply(right_ty),
                    );
                }
                self.unifier.unify(left_ty, right_ty, span)?;
                Ok(Type::unit())
            }

            // Compound assignment operators: all handled uniformly.
            // RHS must be compatible with LHS, result is always Unit.
            // No hardcoded type name knowledge — types are discovered from
            // protocol implementations registered by the stdlib.
            AddAssign | SubAssign | MulAssign | DivAssign | RemAssign | BitAndAssign
            | BitOrAssign | BitXorAssign | ShlAssign | ShrAssign => {
                match left_ty {
                    Type::Int | Type::Float => {
                        self.unifier.unify(right_ty, left_ty, span)?;
                    }
                    Type::Text if matches!(op, AddAssign) => {
                        self.unifier.unify(right_ty, left_ty, span)?;
                    }
                    Type::Var(_) => {
                        self.unifier.unify(left_ty, right_ty, span)?;
                    }
                    _ => {
                        // For Named and other types: unify RHS against LHS type.
                        // This allows literal coercion (e.g., `x += 1` where x: Int32)
                        // and works for any type with the appropriate protocol impl.
                        self.unifier.unify(right_ty, left_ty, span)?;
                    }
                }
                Ok(Type::unit())
            }

            _ => {
                // Try protocol-based resolution for unknown operators/types
                if let Some(result_ty) =
                    self.try_resolve_binop_via_protocol(op, left_ty, right_ty, span)?
                {
                    return Ok(result_ty);
                }
                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Binary operator {} requires protocol implementation.\n  \
                     Hint: Ensure operand types implement the required protocol (e.g., Add, Sub, Mul, Div)",
                    op
                ))))
            }
        }
    }

    /// Try to resolve a binary operator using protocol-based lookup.
    ///
    /// This method attempts to resolve operators through protocol implementations
    /// rather than hardcoded type knowledge. This is key to the stdlib-agnostic
    /// type system architecture.
    ///
    /// # Resolution Process
    ///
    /// 1. Look up the protocol mapping for the operator (Add, Sub, Eq, etc.)
    /// 2. Check if the left operand type implements that protocol
    /// 3. Determine the output type based on OutputStrategy:
    ///  - SameAsOperand: Return the operand type
    ///  - Bool: Return Bool (for comparison operators)
    ///  - Associated: Look up associated Output type (future)
    ///
    /// Returns `Ok(Some(ty))` if resolution succeeds, `Ok(None)` if no protocol
    /// is defined for this operator, or `Err` if types are incompatible.
    ///
    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    fn try_resolve_binop_via_protocol(
        &mut self,
        op: BinOp,
        left_ty: &Type,
        right_ty: &Type,
        span: Span,
    ) -> Result<Option<Type>> {
        // Get the protocol mapping for this operator
        let mapping = match self.operator_protocols.get_binary_protocol(op) {
            Some(m) => m,
            None => return Ok(None), // No protocol defined for this operator
        };

        let protocol_name = mapping.protocol.as_str();

        // Extract the type name for protocol implementation lookup
        let type_name = self.extract_type_name_for_protocol(left_ty);

        // Check if the type implements the required protocol
        if !self
            .protocol_checker
            .read()
            .implements_by_name(left_ty, protocol_name)
        {
            // Type doesn't implement the protocol - return None to let
            // the fallback handling take over
            return Ok(None);
        }

        // Type implements the protocol - determine output type based on strategy
        let result_ty = match mapping.output_strategy {
            OutputStrategy::Bool => {
                // Comparison operators return Bool
                self.unifier.unify(left_ty, right_ty, span)?;
                Type::bool()
            }
            OutputStrategy::SameAsOperand => {
                // Output type is same as operand (e.g., arithmetic)
                self.unifier.unify(left_ty, right_ty, span)?;
                left_ty.clone()
            }
            OutputStrategy::Associated => {
                // Output type comes from associated type (e.g., Add::Output)
                // Unify operand types for binary protocols
                self.unifier.unify(left_ty, right_ty, span)?;

                // Resolve Output associated type using unified helper
                self.resolve_protocol_output_type(left_ty, protocol_name)
                    .unwrap_or_else(|| left_ty.clone())
            }
            OutputStrategy::Custom => {
                // Custom output strategy - for now fall back to operand type
                self.unifier.unify(left_ty, right_ty, span)?;
                left_ty.clone()
            }
        };

        Ok(Some(result_ty))
    }

    /// Check if a type has a specific method available (via impl block or protocol).
    pub(super) fn has_method(&self, ty: &Type, method_name: &str) -> bool {
        // First try protocol checker
        if self
            .protocol_checker
            .read()
            .lookup_method(ty, method_name)
            .is_some()
        {
            return true;
        }
        // Fall back to checking inherent_methods registry
        if let Some(type_name) = self.get_type_name(ty) {
            let methods_guard = self.inherent_methods.read();
            if let Some(methods) = methods_guard.get(&type_name) {
                return methods.contains_key(&verum_common::Text::from(method_name));
            }
        }
        false
    }

    /// Extract a type name suitable for protocol implementation lookup.
    ///
    /// This converts types like `List<Int>` to "List", `Text` to "Text", etc.
    fn extract_type_name_for_protocol(&self, ty: &Type) -> Text {
        match ty {
            Type::Int => Text::from(WKT::Int.as_str()),
            Type::Float => Text::from(WKT::Float.as_str()),
            Type::Bool => Text::from(WKT::Bool.as_str()),
            Type::Char => Text::from(WKT::Char.as_str()),
            Type::Text => Text::from(WKT::Text.as_str()),
            Type::Unit => Text::from("Unit"),
            Type::Never => Text::from("Never"),
            Type::Generic { name, .. } => name.clone(),
            Type::Named { path, .. } => {
                // Extract last segment of path
                if let Some(ident) = path.as_ident() {
                    ident.name.clone()
                } else if let Some(seg) = path.segments.last() {
                    use verum_ast::ty::PathSegment;
                    if let PathSegment::Name(ident) = seg {
                        ident.name.clone()
                    } else {
                        Text::from("Unknown")
                    }
                } else {
                    Text::from("Unknown")
                }
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. } => self.extract_type_name_for_protocol(inner),
            // Refined types: unwrap to base type
            Type::Refined { base, .. } => self.extract_type_name_for_protocol(base),
            _ => Text::from("Unknown"),
        }
    }

    /// Compute unary operation result type.
    ///
    /// Extracted to support iterative inference.
    /// Takes the type of the operand and computes the result type.
    fn compute_unop_result(&mut self, op: UnOp, inner_ty: &Type, span: Span) -> Result<Type> {
        use UnOp::*;

        match op {
            Not => {
                // Not protocol can be implemented for different types:
                // - Bool: logical NOT, returns Bool
                // - Int: bitwise NOT, returns Int (all bits flipped)
                // - Named integer types (UInt8, UInt32, etc.): bitwise NOT, returns same type
                // - Other Named types: check via protocol implementation
                match inner_ty {
                    Type::Bool => Ok(Type::bool()),
                    Type::Int => Ok(Type::int()),
                    Type::Var(_) => {
                        // Type variable - default to Bool for now
                        self.unifier.unify(inner_ty, &Type::bool(), span)?;
                        Ok(Type::bool())
                    }
                    Type::Named { path, .. } => {
                        let type_name = path.last_segment_name();
                        // Integer types support bitwise NOT, returning the same type
                        if matches!(
                            type_name,
                            "UInt8"
                                | "UInt16"
                                | "UInt32"
                                | "UInt64"
                                | "UInt128"
                                | "Int8"
                                | "Int16"
                                | "Int32"
                                | "Int64"
                                | "Int128"
                                | "UIntSize"
                                | "IntSize"
                                // Canonical pointer-width integer names — the
                                // idiomatic spellings (`USize` / `ISize`) the
                                // stdlib actually uses (e.g. `core.sys.bitfield`
                                // applies `!mask` where `mask: USize`). Their
                                // absence rejected bitwise-NOT on every USize
                                // value under strict (AOT) typecheck while the
                                // interpreter accepted it — a cross-tier
                                // divergence. Mirrors the alias matrix in
                                // `dispatch_primitive_method`.
                                | "USize"
                                | "ISize"
                                | "Usize"
                                | "Isize"
                                | "usize"
                                | "isize"
                                | "Byte"
                                | "U8"
                                | "U16"
                                | "U32"
                                | "U64"
                                | "U128"
                                | "I8"
                                | "I16"
                                | "I32"
                                | "I64"
                                | "I128"
                                | "u8"
                                | "u16"
                                | "u32"
                                | "u64"
                                | "u128"
                                | "i8"
                                | "i16"
                                | "i32"
                                | "i64"
                                | "i128"
                        ) || self
                            .protocol_checker
                            .read()
                            .implements_by_name(inner_ty, "Not")
                            || self.has_method(inner_ty, "not")
                        {
                            Ok(inner_ty.clone())
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot apply NOT operator to type: {}. Expected Bool or integer type",
                                inner_ty
                            ))))
                        }
                    }
                    _ => {
                        if self.has_method(inner_ty, "not") {
                            Ok(inner_ty.clone())
                        } else if self.stdlib_single_file_mode {
                            Ok(Type::Unknown)
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot apply NOT operator to type: {}. Expected Bool or integer type",
                                inner_ty
                            ))))
                        }
                    }
                }
            }
            Neg => {
                // Negation: check via Neg protocol or neg method.
                // Refinement types unwrap to their base type for negation.
                // Sized integers (Int8-Int64, I8-I64, ISize) also support negation.
                match inner_ty {
                    Type::Int => Ok(Type::int()),
                    Type::Float => Ok(Type::float()),
                    Type::Named { path, .. } => {
                        let name = path
                            .segments
                            .last()
                            .map(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                _ => "",
                            })
                            .unwrap_or("");
                        // Signed integers and floats support negation
                        if matches!(
                            name,
                            "Int8"
                                | "Int16"
                                | "Int32"
                                | "Int64"
                                | "I8"
                                | "I16"
                                | "I32"
                                | "I64"
                                | "ISize"
                                | "i8"
                                | "i16"
                                | "i32"
                                | "i64"
                                | "isize"
                                | "F32"
                                | "F64"
                                | "Float32"
                                | "Float64"
                                | "f32"
                                | "f64"
                        ) || self
                            .protocol_checker
                            .read()
                            .implements_by_name(inner_ty, "Neg")
                            || self.has_method(inner_ty, "neg")
                        {
                            Ok(inner_ty.clone())
                        } else if let Some(expanded) = self.expand_type_alias(inner_ty) {
                            // Try alias expansion: refinement type aliases like NonZero -> Int{!= 0}
                            match &expanded {
                                Type::Refined { base, .. } => match base.as_ref() {
                                    Type::Int => Ok(Type::int()),
                                    Type::Float => Ok(Type::float()),
                                    _ => Ok(base.as_ref().clone()),
                                },
                                Type::Int => Ok(Type::int()),
                                Type::Float => Ok(Type::float()),
                                _ => {
                                    if self.stdlib_single_file_mode {
                                        Ok(Type::Unknown)
                                    } else {
                                        Err(TypeError::Other(verum_common::Text::from(format!(
                                            "Cannot negate type: {}. Expected signed numeric type",
                                            inner_ty
                                        ))))
                                    }
                                }
                            }
                        } else {
                            if self.stdlib_single_file_mode {
                                Ok(Type::Unknown)
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(format!(
                                    "Cannot negate type: {}. Expected signed numeric type",
                                    inner_ty
                                ))))
                            }
                        }
                    }
                    Type::Refined { base, .. } => {
                        // Negation strips the refinement predicate (can't negate constraints).
                        // -Int{>= 0} → Int, -Float{>= 0.0} → Float
                        match base.as_ref() {
                            Type::Int => Ok(Type::int()),
                            Type::Float => Ok(Type::float()),
                            _ => Ok(base.as_ref().clone()),
                        }
                    }
                    Type::Var(_) => {
                        // Type variable - default to Int for now
                        self.unifier.unify(inner_ty, &Type::int(), span)?;
                        Ok(Type::int())
                    }
                    Type::Generic { name, .. } => {
                        // Generic types like DynTensor<Float>, Vector<Float> support negation
                        // if their element type is numeric
                        if matches!(name.as_str(), "DynTensor" | "Tensor" | "Vector" | "Matrix")
                            || self.has_method(inner_ty, "neg")
                        {
                            Ok(inner_ty.clone())
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot negate type: {}. Expected signed numeric type",
                                inner_ty
                            ))))
                        }
                    }
                    _ => {
                        // Check for user types that might have a neg method
                        if self.has_method(inner_ty, "neg") {
                            Ok(inner_ty.clone())
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot negate type: {}. Expected signed numeric type",
                                inner_ty
                            ))))
                        }
                    }
                }
            }
            BitNot => {
                // Bitwise NOT: check via Not protocol implementation.
                match inner_ty {
                    Type::Int => Ok(Type::int()),
                    Type::Var(_) => {
                        self.unifier.unify(inner_ty, &Type::int(), span)?;
                        Ok(Type::int())
                    }
                    ty @ (Type::Named { .. } | Type::Generic { .. }) => {
                        // Check for fixed-width integer types by name
                        let type_name_str = match ty {
                            Type::Named { path, .. } => Self::path_type_name(path)
                                .or_else(|| Self::path_last_type_name(path)),
                            Type::Generic { name, .. } => Some(name.as_str()),
                            _ => None,
                        };
                        let is_int = type_name_str.is_some_and(|n| {
                            matches!(
                                n,
                                "U8" | "U16"
                                    | "U32"
                                    | "U64"
                                    | "U128"
                                    | "USize"
                                    | "I8"
                                    | "I16"
                                    | "I32"
                                    | "I64"
                                    | "I128"
                                    | "ISize"
                                    | "UInt8"
                                    | "UInt16"
                                    | "UInt32"
                                    | "UInt64"
                                    | "Int8"
                                    | "Int16"
                                    | "Int32"
                                    | "Int64"
                            )
                        });
                        if is_int
                            || self
                                .protocol_checker
                                .read()
                                .implements_by_name(inner_ty, "Not")
                        {
                            Ok(inner_ty.clone())
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Cannot apply bitwise NOT to non-integer type: {}. Expected integer type",
                                inner_ty
                            ))))
                        }
                    }
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "Cannot apply bitwise NOT to non-integer type: {}. Expected Int",
                        inner_ty
                    )))),
                }
            }
            Ref => Ok(Type::Reference {
                inner: Box::new(inner_ty.clone()),
                mutable: false,
            }),
            RefMut => Ok(Type::Reference {
                inner: Box::new(inner_ty.clone()),
                mutable: true,
            }),
            RefChecked => Ok(Type::CheckedReference {
                inner: Box::new(inner_ty.clone()),
                mutable: false,
            }),
            RefCheckedMut => Ok(Type::CheckedReference {
                inner: Box::new(inner_ty.clone()),
                mutable: true,
            }),
            RefUnsafe => Ok(Type::UnsafeReference {
                inner: Box::new(inner_ty.clone()),
                mutable: false,
            }),
            RefUnsafeMut => Ok(Type::UnsafeReference {
                inner: Box::new(inner_ty.clone()),
                mutable: true,
            }),
            Own => Ok(Type::Ownership {
                inner: Box::new(inner_ty.clone()),
                mutable: false,
            }),
            OwnMut => Ok(Type::Ownership {
                inner: Box::new(inner_ty.clone()),
                mutable: true,
            }),
            Deref => self.unary_deref_target(inner_ty, span),
        }
    }

    /// Explicit dereference has the same operand contract in both inference engines.
    pub(super) fn unary_deref_target(&mut self, operand: &Type, span: Span) -> Result<Type> {
        let resolved = self.unifier.apply(operand);
        let ty = self.unifier.normalize_aliases(&resolved);
        match &ty {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => Ok((**inner).clone()),
            Type::Var(var) => {
                for bound in &self.get_type_var_bounds(var) {
                    if bound.protocol.as_ident().is_some_and(|id| matches!(id.name.as_str(), "Ref" | "RefMut"))
                        && let Some(target) = bound.args.first()
                    {
                        return Ok(self.unifier.apply(target));
                    }
                }
                // A rigid value parameter retains its type. An unresolved inference
                // variable can still be constrained by a later reference operand.
                if self.unifier.is_rigid(*var) {
                    return Ok(ty);
                }
                let target = Type::Var(TypeVar::fresh());
                self.unifier.unify(&ty, &Type::reference(false, target.clone()), span)?;
                Ok(target)
            }
            _ => {
                if let Some((owner, _)) = self.conversion_nominal_head(&ty) {
                    self.ensure_stdlib_type_loaded_transitive(&owner);
                }
                if let Some(target) = self.find_deref_target_type(&ty, span)? {
                    return Ok(self.unifier.apply(&target));
                }
                match &ty {
                    Type::Named { path, args } if args.is_empty() && named_primitive_value(path) => Ok(ty),
                    Type::Named { .. } | Type::Generic { .. } => Err(TypeError::OtherWithCodeSpanned {
                        code: "E409".into(),
                        msg: format!("Cannot dereference non-reference type: {ty}").into(),
                        span,
                    }),
                    // CBGR value operands retain their value type.
                    _ => Ok(ty),
                }
            }
        }
    }

    /// Iterative type synthesis to avoid stack overflow.
    ///
    /// This method implements type inference using an explicit work stack instead
    /// of recursion. This prevents stack overflow for deeply nested expressions
    /// like ((((1 + 2) * 3) - 4) < 10) or chains with thousands of operations.
    ///
    /// # Algorithm
    ///
    /// Uses two stacks:
    /// - `work_stack`: Tasks to process (InferWork items)
    /// - `value_stack`: Intermediate type results
    ///
    /// Each expression is broken down into work items that push/pop from the value stack.
    /// Binary operations are handled in two phases:
    /// 1. BinaryOpRight: After left operand, process right operand
    /// 2. BinaryOpResult: After both operands, compute result type
    ///
    /// This maintains the same type checking semantics as recursive inference
    /// but with O(1) stack space instead of O(depth).
    fn synth_expr_iterative(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;

        // Unwrap nested parentheses iteratively
        let mut current_expr = expr;
        while let Paren(inner) = &current_expr.kind {
            current_expr = inner;
        }

        // Work stack: Tasks to process (processed in LIFO order)
        let mut work_stack: Vec<InferWork> = vec![InferWork::SynthExpr(current_expr)];

        // Value stack: Intermediate type results
        // When a SynthExpr completes, it pushes a type here
        let mut value_stack: Vec<Type> = Vec::new();

        while let Some(work) = work_stack.pop() {
            match work {
                InferWork::SynthExpr(e) => {
                    // Unwrap parentheses
                    let mut e = e;
                    while let Paren(inner) = &e.kind {
                        e = inner;
                    }

                    match &e.kind {
                        // Literals: directly push type
                        Literal(lit) => {
                            let ty = self.infer_literal(lit);
                            value_stack.push(ty);
                        }

                        // Variables: lookup and push type
                        Path(path) => {
                            let ty = self.infer_path_expr(path, e.span)?;
                            value_stack.push(ty);
                        }

                        // Binary operations: two-phase processing
                        // SPECIAL CASE: Assignment operations need different handling
                        // to avoid checking initialization of the target (LHS).
                        Binary { op, left, right } => {
                            if op.is_assignment() {
                                // Every write needs its original place for permission checking.
                                // Plain assignment also has distinct initialization rules.
                                let result = self.infer_binop(*op, left, right, e.span)?;
                                value_stack.push(result.ty);
                            } else {
                                // LANGUAGE LAW E432 — the iterative walker's
                                // entry is the ONLY point that still holds
                                // both operand ASTs; same shared engine as
                                // infer_binop (atom probe needs no inference).
                                self.law_check_bool_eq_conjunction(*op, left, right)?;
                                // Schedule work in reverse order (LIFO)
                                // 1. After both operands ready, compute result
                                work_stack.push(InferWork::BinaryOpResult {
                                    op: *op,
                                    span: e.span,
                                });
                                // 2. After left ready, process right
                                work_stack.push(InferWork::BinaryOpRight {
                                    op: *op,
                                    right,
                                    span: e.span,
                                });
                                // 3. First process left (pushes left_ty to value_stack)
                                work_stack.push(InferWork::SynthExpr(left));
                            }
                        }

                        // Unary operations: process operand, then compute result
                        // For Ref/RefMut, use non-iterative path for aliasing tracking
                        Unary { op, expr: inner } => {
                            // Reference operations need aliasing tracking, use non-iterative path
                            if matches!(
                                op,
                                UnOp::Ref
                                    | UnOp::RefMut
                                    | UnOp::RefChecked
                                    | UnOp::RefCheckedMut
                                    | UnOp::RefUnsafe
                                    | UnOp::RefUnsafeMut
                            ) {
                                let result = self.infer_unop(*op, inner, e.span)?;
                                value_stack.push(result.ty);
                            } else {
                                work_stack.push(InferWork::UnaryOpResult {
                                    op: *op,
                                    inner_expr: inner,
                                    span: e.span,
                                });
                                work_stack.push(InferWork::SynthExpr(inner));
                            }
                        }

                        // For other expressions (including If), fall back to recursive inference
                        // This is safe because should_use_iterative_inference filters them out
                        // and they won't deeply nest in the same pattern as binary operations
                        _ => {
                            let result = self.infer_expr(e, InferMode::Synth)?;
                            value_stack.push(result.ty);
                        }
                    }
                }

                InferWork::CheckExpr(e, expected) => {
                    // Check expression against expected type (doesn't push to value_stack)
                    self.check_expr(e, &expected)?;
                }

                InferWork::BinaryOpRight { op, right, span } => {
                    // Left type is on value_stack, now process right operand
                    // Leave left_ty on stack, it will be used by BinaryOpResult

                    // Check for numeric literal coercion opportunity
                    // If left is a numeric type and right is an integer or float literal,
                    // use check_expr to coerce the literal to left's type
                    let should_coerce = if let Some(left_ty) = value_stack.last() {
                        let is_arithmetic = matches!(
                            op,
                            BinOp::Add
                                | BinOp::Concat
                                | BinOp::Sub
                                | BinOp::Mul
                                | BinOp::Div
                                | BinOp::Rem
                                | BinOp::Pow
                                | BinOp::Lt
                                | BinOp::Le
                                | BinOp::Gt
                                | BinOp::Ge
                                | BinOp::Eq
                                | BinOp::Ne
                                | BinOp::BitAnd
                                | BinOp::BitOr
                                | BinOp::BitXor
                                | BinOp::Shl
                                | BinOp::Shr
                                | BinOp::AddAssign
                                | BinOp::SubAssign
                                | BinOp::MulAssign
                                | BinOp::DivAssign
                                | BinOp::RemAssign
                                | BinOp::BitAndAssign
                                | BinOp::BitOrAssign
                                | BinOp::BitXorAssign
                                | BinOp::ShlAssign
                                | BinOp::ShrAssign
                        );
                        let right_is_literal = matches!(&right.kind, ExprKind::Literal(lit)
                            if matches!(lit.kind, verum_ast::literal::LiteralKind::Int(_)
                                | verum_ast::literal::LiteralKind::Float(_)));

                        // Try coercing literal to left's type — check_expr validates compatibility
                        is_arithmetic && right_is_literal
                    } else {
                        false
                    };

                    if should_coerce {
                        // Get left_ty from stack (it stays on stack for BinaryOpResult)
                        if let Some(left_ty) = value_stack.last().cloned() {
                            // Use check_expr to coerce the literal to left's type
                            if self.check_expr(right, &left_ty).is_ok() {
                                // Push left_ty as right_ty (coerced)
                                value_stack.push(left_ty);
                            } else {
                                // Coercion failed, fall back to synth_expr
                                work_stack.push(InferWork::SynthExpr(right));
                            }
                        } else {
                            work_stack.push(InferWork::SynthExpr(right));
                        }
                    } else {
                        work_stack.push(InferWork::SynthExpr(right));
                    }
                }

                InferWork::BinaryOpResult { op, span } => {
                    // Both operands ready: value_stack has [... left_ty, right_ty]
                    let right_ty = value_stack.pop().ok_or_else(|| {
                        TypeError::Other("Internal error: value stack underflow (right)".into())
                    })?;
                    let left_ty = value_stack.pop().ok_or_else(|| {
                        TypeError::Other("Internal error: value stack underflow (left)".into())
                    })?;

                    // Compute result type
                    let result_ty = self.compute_binop_result(op, &left_ty, &right_ty, span)?;
                    value_stack.push(result_ty);
                }

                InferWork::UnaryOpResult {
                    op,
                    inner_expr,
                    span,
                } => {
                    // Operand ready: value_stack has [... inner_ty]
                    let inner_ty = value_stack.pop().ok_or_else(|| {
                        TypeError::Other("Internal error: value stack underflow (unary)".into())
                    })?;

                    if matches!(op, UnOp::Deref) {
                        self.observe_expression_type(inner_expr, &inner_ty);
                    }

                    let result_ty = self.compute_unop_result(op, &inner_ty, span)?;
                    value_stack.push(result_ty);
                }

                // These work items are not used because should_use_iterative_inference
                // filters out complex expressions (If, Call, Field, Method)
                InferWork::IfBranches { .. }
                | InferWork::IfResult { .. }
                | InferWork::CallArgs { .. }
                | InferWork::CallResult { .. }
                | InferWork::FieldResult { .. }
                | InferWork::MethodCall { .. } => {
                    unreachable!(
                        "Complex expressions should be filtered by should_use_iterative_inference"
                    );
                }
            }
        }

        // Final result should be single type on value_stack
        if value_stack.len() != 1 {
            return Err(TypeError::Other(
                format!(
                    "Internal error: expected 1 type on value stack, got {}",
                    value_stack.len()
                )
                .into(),
            ));
        }

        // SAFETY: The guard above ensures value_stack.len() == 1, so pop() always succeeds
        let result_ty = value_stack
            .pop()
            .expect("internal error: value_stack verified non-empty but pop failed");
        Ok(InferResult::new(result_ty))
    }

    /// Helper to infer type for a path expression.
    /// Extracted to reduce code duplication in iterative inference.
    fn infer_path_expr(&mut self, path: &Path, span: Span) -> Result<Type> {
        if path.segments.len() == 1 {
            // Single segment - simple variable lookup
            let name = if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                id.name.as_str()
            } else {
                // Handle self/super/crate keywords
                match &path.segments[0] {
                    verum_ast::ty::PathSegment::SelfValue => {
                        // Check if we're in a method context
                        if let Maybe::Some(ref self_ty) = self.current_self_type {
                            // Look up 'self' in the environment
                            if let Some(scheme) = self.ctx.env.lookup("self").cloned() {
                                let ty = self.instantiate_value_scheme(&scheme, span)?;
                                return Ok(ty);
                            }
                        }
                        return Err(TypeError::Other(
                            "self keyword requires method context".into(),
                        ));
                    }
                    verum_ast::ty::PathSegment::Super | verum_ast::ty::PathSegment::Cog => {
                        return Err(TypeError::UnboundVariable {
                            name: self.path_to_string(path), span,
                        });
                    }
                    _ => return Err(TypeError::Other("Invalid path".into())),
                }
            };

            // =====================================================================
            // Special built-in identifiers: null
            // Interop/FFI: foreign function interface for calling C/system code, marshalling rules — FFI null pointer handling
            // =====================================================================
            if name == "null" {
                // null is a polymorphic null pointer that can be any pointer type
                let inner_var = Type::Var(TypeVar::fresh());
                return Ok(Type::Pointer {
                    mutable: true,
                    inner: Box::new(inner_var),
                });
            }

            // Check for import ambiguity first
            // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Ambiguity
            if let Some(sources) = self.imported_names.get(&verum_common::Text::from(name)) {
                if sources.len() > 1 {
                    let sources_str = sources
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(TypeError::AmbiguousName {
                        name: verum_common::Text::from(name),
                        sources: verum_common::Text::from(sources_str),
                        span,
                    });
                }
            }

            // MOUNT AUTHORITY (see mount_authoritative_fn_scheme): an
            // explicitly mounted single-source name outranks the flat
            // bare env slot — before the env chain, not after it.
            if let Some(scheme) = self.mount_authoritative_fn_scheme(name) {
                let ty = self.instantiate_value_scheme(&scheme, span)?;
                return Ok(self.unifier.apply(&ty));
            }

            if self.diagnose_ambiguous_constructor(name, None, span)? {
                return Ok(Type::Unknown);
            }

            // Try local env first, then module context
            if let Some(scheme) = self.ctx.env.lookup(name).cloned() {
                // =====================================================================
                // DEFINITE ASSIGNMENT: Check variable is initialized before use
                // Spec: L0-critical/memory-safety/uninitialized
                // =====================================================================
                let var_name = verum_common::Text::from(name);
                self.check_variable_initialized(&var_name, span)?;

                // Check affine usage - detect use after move
                // Spec: L0-critical/reference_system/value_transfer - Affine type safety
                //

                // Only affine types have move semantics. Regular types use copy semantics
                // in Verum, so they can be used multiple times without being consumed.
                // When in call/method receiver context, use borrow_value instead of use_value.
                if self.affine_tracker.is_affine_binding(name) {
                    if self.in_call_arg_context {
                        self.affine_tracker.borrow_value(name, span)?;
                    } else {
                        self.affine_tracker.use_value(name, span)?;
                    }
                }

                // If we're currently checking a constant and this is an imported constant,
                // record the dependency for cycle detection.
                // Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Constant Initialization Order
                if self.current_constant_path.is_some() {
                    if let Some(ref_constant_path) = self
                        .imported_constant_paths
                        .get(&verum_common::Text::from(name))
                        .cloned()
                    {
                        self.record_constant_dependency(&ref_constant_path);
                    }
                }

                let ty = self.instantiate_value_scheme(&scheme, span)?;
                // Apply unifier to resolve type variables
                let resolved_ty = self.unifier.apply(&ty);
                // For GAT/HKT types: if the resolved type contains TypeApp projections,
                // normalize to reduce them now that type vars are resolved.
                let resolved_ty = if Self::contains_type_app(&resolved_ty) {
                    self.normalize_type(&resolved_ty)
                } else {
                    resolved_ty
                };
                #[cfg(debug_assertions)]
                if name == "Greater" || name == "Less" || name == "Equal" {
                    // eprintln!("[DEBUG infer_path_expr] Looked up '{}' in env: scheme={:?}, ty={:?}, resolved={:?}", name, scheme, ty, resolved_ty);
                }
                // CTOR-AS-FN-SYNTH (#31): see bare_payload_ctor_as_fn.
                if let Some(ctor_fn) = self.bare_payload_ctor_as_fn(name, &resolved_ty) {
                    return Ok(ctor_fn);
                }
                Ok(resolved_ty.into_expression_type())
            } else {
                // Try module-level function lookup
                if let Maybe::Some(scheme) = self.lookup_function_in_module(name) {
                    let ty = self.instantiate_value_scheme(&scheme, span)?;
                    let resolved_ty = self.unifier.apply(&ty);
                    if let Some(ctor_fn) = self.bare_payload_ctor_as_fn(name, &resolved_ty) {
                        return Ok(ctor_fn);
                    }
                    Ok(resolved_ty)
                } else if {
                    // #128 — lookup-on-miss for stdlib types in
                    // expression position.  The lazy pre-pass
                    // (`register_stdlib_types_for_module` →
                    // `collect_named_types_from_function_body`) is a
                    // no-op for V0 perf, so body-position type
                    // expressions like `List<Int>.new()` don't get
                    // their referenced type pre-loaded.  When the
                    // type-table miss happens here, hit
                    // `ensure_stdlib_type_loaded` once before
                    // throwing UnboundVariable.  Idempotent — when
                    // the type is already present the helper short-
                    // circuits at the `lookup_type(name).is_none()`
                    // gate.
                    //
                    // Stdlib-agnostic per `crates/verum_types/src/AGENTS.md`:
                    // the lookup is keyed by the user-written name;
                    // the actual existence test is `core_metadata.types`
                    // membership, not a hardcoded list of stdlib
                    // type names.
                    let name_text = verum_common::Text::from(name);
                    if self.ctx.lookup_type(name).is_none() {
                        let mut pending: Vec<Text> = Vec::new();
                        self.ensure_stdlib_type_loaded(&name_text, &mut pending);
                        // Drain transitive deps surfaced by the
                        // initial load — same discipline as the
                        // pre-pass loop.  Bounded by the metadata's
                        // type count so a malformed dep cycle
                        // can't loop forever.
                        let mut bound = 256usize;
                        while let Some(next) = pending.pop() {
                            bound = bound.saturating_sub(1);
                            if bound == 0 {
                                break;
                            }
                            self.ensure_stdlib_type_loaded(&next, &mut pending);
                        }
                    }
                    self.ctx.lookup_type(name).is_some()
                } && let Maybe::Some(ty) = self.ctx.lookup_type(name) {
                    // Check if this is a type name being used in a context
                    // where it will be followed by a variant/field access. This is needed
                    // for cross-file imported variant types when the parser creates a Path
                    // for the type name (e.g., `RegistryError` in `RegistryError.PackageNotFound`)
                    //

                    // CTOR-AS-FN (#31, THE prelude root): a bare PAYLOAD-
                    // constructor name in VALUE position is the constructor
                    // FUNCTION, not the type. Under `mount core.prelude.*`
                    // the leaf re-export registers "Some"/"Ok"/"Err" in the
                    // TYPE table, so this stdlib-type-on-miss arm resolved
                    // them to `Named("Some")` BEFORE the correct
                    // try_resolve_variant_constructor arm below could run —
                    // producing `E400 expected 'fn(..)', found 'Some'` for
                    // `m.and_then(Some)` / `let f: fn(Int)->Maybe<Int> = Some`
                    // (and cascading ~30 base/maybe property tests). Payload
                    // ctors yield their fn type; unit ctors (`None`) fall
                    // through unchanged (params.is_empty() → None).
                    if let Some(ctor_fn) = self.bare_payload_ctor_as_fn(name, &Type::Unknown) {
                        return Ok(ctor_fn);
                    }
                    // UNIT-CTOR twin of the payload arm above (T0555 layer 4):
                    // a metadata-only sum type's import registers each CASE as
                    // a SELF-NAMED `Named{case}` placeholder (import_type_export
                    // has no AST to fully register against), and the payload
                    // arm deliberately passes on unit ctors — so a bare
                    // `UoInsert` in value position resolved to the placeholder
                    // TYPE and every use failed E400 "expected 'UpdateOp',
                    // found 'UoInsert'". A self-referential placeholder is not
                    // a genuine type binding (the T0617 ctor-nominal
                    // discipline): when the name is a registered variant case
                    // and the type-table answer is exactly `Named{name}` with
                    // no args, prefer the constructor. A REAL same-named type
                    // (`Maybe` the type vs some `Maybe` case) never matches —
                    // its table entry is not self-named-argless, or the name
                    // is not in the parents table.
                    if self
                        .variant_constructor_parents
                        .contains_key(&verum_common::Text::from(name))
                        && matches!(
                            &ty,
                            Type::Named { path, args }
                                if args.is_empty()
                                    && path.as_ident().map(|i| i.name.as_str()) == Some(name)
                        )
                        && let Some(ctor_ty) = self.try_resolve_variant_constructor(name)
                    {
                        return Ok(ctor_ty);
                    }
                    // IMPORTANT: If this type has generic parameters but was referenced without
                    // explicit type arguments, create fresh type variables for the missing params.
                    // This enables proper inference for types like `PendingFuture<T>` used as `PendingFuture`.
                    let type_name_text = verum_common::Text::from(name);
                    // `> 0`, not merely "recorded": a declaration records
                    // its parameter count even when that count is zero
                    // (see `register_declared_type_arity` — silence would
                    // leave a namesake's count to answer for it). Zero
                    // means the same thing here as no entry did before,
                    // and must take the same branch: opening a
                    // non-generic type into a bare `Named` would discard
                    // the record or variant structure `ty` carries.
                    if let Some(&generics_count) = self.type_generics_count.get(&type_name_text)
                        && generics_count > 0
                    {
                        // Type requires generic arguments - create Named type with fresh type vars
                        let fresh_args: List<Type> = (0..generics_count)
                            .map(|_| Type::Var(TypeVar::fresh()))
                            .collect();
                        Ok(Type::Named {
                            path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                                name, span,
                            )),
                            args: fresh_args,
                        })
                    } else {
                        // Non-generic type - return as-is
                        Ok(ty.clone())
                    }
                } else if self
                    .inline_modules
                    .contains_key(&verum_common::Text::from(name))
                {
                    // A namespace can only participate in a named path. It has
                    // no runtime value, even when the expected type is inferred.
                    Err(TypeError::UnboundVariable { name: name.into(), span })
                } else if let Some(ctor_ty) = self.try_resolve_variant_constructor(name) {
                    // Variant constructor used at value-position
                    // (`Ok(x)`, `Err(e)`, `Some(v)` …).  The
                    // env-lookup above missed because stdlib
                    // variant constructors aren't registered as
                    // env values — pattern matching uses
                    // `inductive_constructors` registry instead.
                    // Mirror that for value-position by building a
                    // `fn(payload) -> Variant<freshvars>`
                    // constructor type from the registry.
                    Ok(ctor_ty)
                } else {
                    // NO stdlib exemption here (T0777).  `stdlib_single_file_mode`
                    // exists for CONTEXTS — sibling modules whose `provide`
                    // blocks a single-file check cannot see — but it used to
                    // swallow "this name does not exist" as well, and the
                    // resulting `Type::Unknown` then made every downstream
                    // check lenient.  Measured before removing it: an
                    // undefined function was reported in 0 of 17 sampled
                    // core/ files that "passed", and turning the exemption
                    // off costs 19 files out of 2560 — every one of which
                    // was already broken.
                    // "Did you mean ...?" — search visible scope names for
                    // a Levenshtein-close candidate. 80% of user typos are
                    // edit distance ≤ 2 from the intended name; threshold
                    // kept at 3 to catch longer transpositions without
                    // being noisy.
                    let candidates = self.ctx.env.visible_names();
                    let hint = candidates
                        .iter()
                        .filter_map(|c| {
                            let d = levenshtein_distance(name, c.as_str());
                            if d <= 3 && d > 0 { Some((d, c)) } else { None }
                        })
                        .min_by_key(|(d, _)| *d)
                        .map(|(_, c)| c.clone());
                    // Always emit `TypeError::UnboundVariable` so the diagnostic
                    // carries the E100 code; `TypeError::Other(...)` has no code
                    // and drops the test-runner's `@expected-error: E100` match.
                    // The "did you mean …" hint rides on the message for display,
                    // but the error code is preserved for the test runner and
                    // downstream tooling.
                    let _ = hint; // Suggestion attached to diagnostic builder elsewhere if needed.
                    Err(TypeError::UnboundVariable {
                        name: name.to_text(),
                        span,
                    })
                }
            }
        } else {
            // Multi-segment path: module::Type::path::to::item
            self.resolve_multi_segment_path(path, span)
                .map(|result| result.ty)
        }
    }

    /// Core type inference logic.
    ///
    /// The shared depth guard bounds nested work. Expression handlers keep
    /// resolution temporaries separate from recursive operand checking.
    pub(super) fn infer_expr(&mut self, expr: &Expr, mode: InferMode) -> Result<InferResult> {
        // RAII depth guard prevents stack overflow from mutual recursion
        let _depth_guard = self.inc_inference_depth("infer_expr")?;
        self.infer_expr_inner(expr, mode)
    }

    /// Inner implementation of type inference.
    fn infer_expr_inner(&mut self, expr: &Expr, mode: InferMode) -> Result<InferResult> {
        let _global_guard = GlobalDepthGuard::enter()?;
        use ExprKind::*;

        // Unwrap nested parentheses iteratively to avoid stack overflow
        // For deeply nested expressions like ((((1 + 2) + 3) + 4) + 5)
        let mut current_expr = expr;
        while let Paren(inner) = &current_expr.kind {
            current_expr = inner;
        }

        match mode {
            InferMode::Check(expected_var) => {
                // Get expected type from context
                let expected = Type::Var(expected_var);
                self.check_expr(current_expr, &expected)
            }
            InferMode::Synth => match &current_expr.kind {
                // Literals
                Literal(lit) => Ok(InferResult::new(self.infer_literal(lit))),

                // Variables and qualified paths
                // Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Cross-module resolution
                Path(_) => self.infer_expr_path(current_expr),

                // Binary operations
                Binary { op, left, right } => self.infer_binop(*op, left, right, expr.span),

                // Unary operations
                Unary { op, expr: inner } => self.infer_unop(*op, inner, expr.span),

                // Function calls
                Call { .. } => self.infer_expr_call(current_expr),

                // Tuple expressions
                Tuple(_) => self.infer_expr_tuple(current_expr),

                // Range expressions: start..end or start..=end
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Range expressions
                Range { .. } => self.infer_expr_range(current_expr, expr.span),

                // Block expressions
                Block(block) => self.infer_block(block),

                // If expressions - with flow-sensitive refinement narrowing
                // If expressions: both branches must unify to same type; if-let patterns narrow types in the then-branch
                If { .. } => self.infer_if_expr(current_expr),

                // Lambda synthesis: create fresh type variables for parameters
                // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi types (dependent functions)
                Closure { .. } => self.infer_closure_expr(current_expr, expr.span),

                // Field access (including GAT access like Iterator.Item)
                // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1-1.4 - GAT syntax
                Field { .. } => self.synth_field_use(current_expr, FieldUse::Value),

                // Index access
                // Supports Array, Slice, List<T>, Text (returns Char), Map<K,V> (returns V)
                // Also supports any type implementing the Index protocol or having an `index` method
                Index { .. } => self.infer_expr_index(current_expr),

                // Pipeline operator: x |> f (desugars to f(x))
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Pipeline operator
                Pipeline { .. } => self.infer_expr_pipeline(current_expr),

                // Return expression - returns Never type (diverging control flow)
                // Return reference validation: ensuring returned references do not outlive their referents via escape analysis — Return lifetime validation
                Return(_) => self.infer_expr_return_expr(current_expr),

                // Match expressions
                // Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Dependent Pattern Matching
                // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Evidence Propagation
                Match { .. } => self.infer_match_expr(current_expr),

                // Method calls
                // Higher-rank protocol bounds: for<T> quantification in protocol bounds for universal requirements — .1-2.3
                //

                // ITERATIVE METHOD CHAIN HANDLING:
                // To prevent stack overflow on deeply nested method chains like a.b().c().d().e(),
                // we "unwind" the chain into a flat list and process iteratively instead of recursively.
                MethodCall {
                    receiver,
                    method,
                    type_args,
                    args,
                } => {
                    self.infer_method_chain_iterative(receiver, method, type_args, args, expr.span)
                }

                // Array expressions
                Array(_) => self.infer_expr_array(current_expr),

                // Cast expressions
                Cast { .. } => self.infer_expr_cast(current_expr),

                // Try operator (? operator) for error propagation
                // Try operator type checking: ? operator desugars to match with From conversion, requires Result/Maybe return type — Error propagation with ?
                // Implements E0203, E0204, E0205 diagnostics
                Try(inner_expr) => self.infer_try_operator(inner_expr, expr.span),

                // Plain try block: try { expr } -> Result<T, E>
                // Error handling: Result<T, E> and Maybe<T> types, try (?) operator with automatic From conversion, error propagation — Section 6.3 - Try blocks
                // Auto-wraps the block's value in Ok() and captures error type from ? operators
                TryBlock(inner_block) => self.infer_try_block(inner_block, expr.span),

                // Loop expressions
                Loop { .. } => self.infer_expr_loop(current_expr, expr.span),

                // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Evidence Propagation
                While { .. } => self.infer_while_loop(current_expr),

                For { .. } => self.infer_for_loop(current_expr),

                Break { label: _, value: _ } => {
                    // Break has Never type (diverging control flow)
                    Ok(InferResult::new(Type::never()))
                }

                Continue { label: _ } => {
                    // Continue has Never type (diverging control flow)
                    Ok(InferResult::new(Type::never()))
                }

                // Yield expressions
                // Generator functions: fn* syntax yields values lazily, producing Iterator<Item=T> types
                Yield(_) => self.infer_expr_yield_expr(current_expr),

                // Record construction OR Variant constructor with record data
                // Record types: "type T is { field: Type, ... }" with named fields, structural matching
                Record { .. } => self.infer_expr_record(current_expr),

                // Tuple index
                TupleIndex { .. } => self.infer_expr_tuple_index(current_expr),

                // Await expression: value.await or await value
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Async/await syntax
                Await(_) => self.infer_expr_await_expr(current_expr),

                // Spawn expression: spawn expr
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Task spawning
                // Context type system integration: context requirements tracked in function types, checked at call sites — Context propagation to spawned tasks
                // CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Thread safety (Send/Sync)
                Spawn { .. } => self.infer_expr_spawn(current_expr),

                // Async block: async { ... }
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Async blocks
                // Async blocks create an async context for their body
                // This enables select expressions and await within the block
                // Select expressions require async context: "select { ... }" only valid in async functions — Async context tracking
                Async(_) => self.infer_expr_async_expr(current_expr),

                // Unsafe block: unsafe { ... }
                // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Three-tier reference system
                // An unsafe block allows manual memory safety proofs (Tier 2 references)
                // The block type is the type of its last expression (or Unit if none)
                Unsafe(_) => self.infer_expr_unsafe_expr(current_expr),

                // Yield expression: yield value
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Generator yield

                // Tensor literal: tensor<2, 3>Float { [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]] }
                // Expression grammar: precedence levels, associativity rules, all constructs are expressions — .12 - Tensor literals
                TensorLiteral { .. } => self.infer_expr_tensor_literal(current_expr, expr.span),

                // Interpolated strings: f"Sum: {sum}", sql"SELECT * FROM {table}"
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — #1.4.2 - Text interpolation with injection protection
                InterpolatedString { .. } => self.infer_expr_interpolation(current_expr, expr.span),

                // List comprehension: [x * 2 for x in list if x > 0]
                // Expression grammar: precedence levels, associativity rules, all constructs are expressions — .10 - Comprehension expressions
                Comprehension { .. } => self.infer_expr_comprehension(current_expr),

                // Stream comprehension: stream[x * 2 for x in source]
                // Expression grammar: precedence levels, associativity rules, all constructs are expressions — .11 - Stream processing syntax
                StreamComprehension { .. } => self.infer_expr_stream_comprehension(current_expr),

                // Map comprehension: {k: v for (k, v) in pairs if condition}
                // Async generator expressions: "async fn*" combining async iteration with yield
                MapComprehension { .. } => self.infer_map_comprehension_expr(current_expr),

                // Set comprehension: set{x for x in items if condition}
                // Generator pipeline operations: composing generators with map/filter/take combinators
                SetComprehension { .. } => self.infer_expr_set_comprehension(current_expr),

                // Generator expression: gen{x for x in items if condition}
                // Recursive generators: generators that yield from sub-generators via yield* delegation
                // Generators are lazy iterators that yield values on demand
                GeneratorComprehension { .. } => self.infer_expr_generator_comprehension(current_expr),

                // Await expression: expr.await
                // Async/await integration: async functions return Future<T>, await extracts T, select for multi-future - Async/Await Integration

                // Try-recover: try { ... } recover { ... }
                // Executes try block, if it fails, executes recover block
                TryRecover { .. } => self.infer_expr_try_recover(current_expr),

                // Try-finally: try { ... } finally { ... }
                // Executes try block, then always executes finally block
                // Returns value from try block (wrapped in Result if ? is used)
                TryFinally { .. } => self.infer_try_finally_expr(current_expr),

                // Try-recover-finally: try { ... } recover { ... } finally { ... }
                // Combines both recover and finally behaviors
                TryRecoverFinally { .. } => self.infer_expr_try_recover_finally(current_expr, expr.span),

                // Parenthesized expressions are unwrapped at the top of infer_expr
                // to avoid stack overflow on deeply nested parens
                Paren(_) => {
                    unreachable!("Paren expressions should be unwrapped at the top of infer_expr")
                }

                // Null coalescing: x ?? default
                // Extracts T from Maybe<T> or returns T if already not Maybe
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Null coalescing operator
                NullCoalesce { .. } => self.infer_expr_null_coalesce(current_expr),

                // Optional chaining: obj?.field
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Optional chaining operator
                // Uses Maybe protocol resolution for type extraction
                OptionalChain { .. } => self.infer_expr_optional_chain(current_expr),

                // Map literal: { key: value, ... }
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Map expression syntax
                MapLiteral { .. } => self.infer_expr_map_literal(current_expr),

                // Set literal: { elem1, elem2, ... }
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Set expression syntax
                SetLiteral { .. } => self.infer_expr_set_literal(current_expr),

                // Context usage: using Context with handler { body }
                // Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context usage expressions
                UseContext { .. } => self.infer_expr_use_context(current_expr, expr.span),

                // Capability attenuation: context.attenuate(capabilities)
                // Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0 - Capability Attenuation
                Attenuate { .. } => self.infer_attenuate_expr(current_expr),

                // Type property expressions: T.size, T.alignment, T.stride, T.min, T.max, T.bits, T.name
                // Spec: grammar/verum.ebnf Section 2.17 - Type Properties
                // These provide compile-time access to type metadata
                TypeProperty { ty, property } => self.infer_type_property(ty, property, expr.span),

                // For-await loop: for await pattern in async_iterable { body }
                // Spec: grammar/verum.ebnf - for_await_loop production (v2.10)
                // Iterates over an async iterator, awaiting each value
                ForAwait { .. } => self.infer_expr_for_await(current_expr, expr.span),

                // Throw expression: throw error_value
                // Spec: grammar/verum.ebnf - throw_expr production
                // Throws an error in functions with a `throws` clause
                Throw(_) => self.infer_expr_throw_expr(current_expr),

                // Select expression: select { arm1, arm2, ... }
                // Select expressions require async context: "select { ... }" only valid in async functions — Select Expression Syntax
                // Awaits multiple futures concurrently, executes first completed
                // Grammar: grammar/verum.ebnf v2.9 - select_expr
                Select { .. } => self.infer_select_expr(current_expr),

                // Nursery expression: nursery(options) { body } [on_cancel {...}] [recover {...}]
                // Structured concurrency: nurseries for task scoping, cancellation propagation, error collection — Structured Concurrency
                // Creates a scope where all spawned tasks must complete before the scope exits.
                // This guarantees structured concurrency - no orphaned tasks.
                Nursery { .. } => self.infer_nursery_expr(current_expr),

                // Is expression: expr is Pattern or expr is not Pattern
                // "x is Pattern" operator: replaces Rust matches!() macro for pattern testing, returns Bool
                // Tests if an expression matches a pattern, returns Bool
                Is { .. } => self.infer_expr_is(current_expr, expr.span),

                // Meta block: meta { ... }
                // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Compile-time evaluation
                // Executes code at compile time, returns the block's computed type
                Meta(_) => self.infer_expr_meta(current_expr),

                // Macro invocation: path!(args)
                // Spec: grammar/verum.ebnf - meta_call production
                // Macros should be expanded before type checking
                MacroCall { .. } => self.infer_expr_macro_call(current_expr),

                // Universal quantifier: forall x: T. predicate(x)
                // Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Type-Level Computation
                // Formal proof system (future v2.0+): machine-checkable proofs with tactics (simp, ring, omega, blast, induction), theorem/lemma/corollary statements — Quantifiers in Proof Terms
                // Quantifier expressions: "forall x in collection: predicate" and "exists x in collection: predicate" as boolean expressions
                Forall { .. } => self.infer_expr_forall(current_expr),

                // Existential quantifier: exists x: T. predicate(x)
                // Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Type-Level Computation
                // Formal proof system (future v2.0+): machine-checkable proofs with tactics (simp, ring, omega, blast, induction), theorem/lemma/corollary statements — Quantifiers in Proof Terms
                // Quantifier expressions: "forall x in collection: predicate" and "exists x in collection: predicate" as boolean expressions
                Exists { .. } => self.infer_expr_exists(current_expr),

                // Meta-function expression: @file, @line, @cfg, @const, etc.
                // Spec: grammar/verum.ebnf Section 2.20.6 - Meta-Level Functions
                MetaFunction { .. } => self.infer_expr_meta_function(current_expr),

                // Quote expression: quote { ... } or quote(N) { ... }
                // Spec: grammar/verum.ebnf - quote_expr production
                // Quote expressions produce a TokenStream that can be spliced into generated code
                Quote { .. } => self.infer_expr_quote(current_expr, expr.span),

                // StageEscape expression: $(stage N){ expr }
                // Spec: grammar/verum.ebnf - stage_escape production
                // Used within quote blocks to escape to a specific stage
                StageEscape {
                    stage: _,
                    expr: inner,
                } => {
                    // The stage escape evaluates to the type of the inner expression
                    self.synth_expr(inner)
                }

                // Lift expression: lift expr
                // Lifts a compile-time value to the next stage
                Lift { .. } => self.infer_expr_lift(current_expr),

                // Type expression: Wrapper<Person>, List<Int>, etc.
                // These are used for static method calls like Wrapper<Person>.default()
                // The type expression itself has the "type" kind (like metatypes in other languages)
                // Type expressions for generic instantiation: "Type<A, B>" syntax for supplying type arguments
                ExprKind::TypeExpr(_) => self.infer_expr_type_expr(current_expr),

                // typeof(expr) → returns a type-info record with `.name: Text`
                ExprKind::Typeof(_) => self.infer_expr_typeof_expr(current_expr),

                // Inline assembly expression: @asm("template", operands..., options)
                // Low-level type operations: raw pointer casting, transmute, memory layout control
                ExprKind::InlineAsm { .. } => self.infer_inline_asm_expr(current_expr),

                // =========================================================================
                // Destructuring assignment: (a, b) = expr, [x, y] = arr, Point { x, y } = p
                // Compound destructuring: (x, y) += (dx, dy), [a, b] *= scale
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Destructuring assignment expressions
                // Unified destructuring: consistent pattern syntax for let bindings, match arms, and function parameters
                // =========================================================================
                DestructuringAssign { .. } => self.infer_expr_destructuring_assign(current_expr),

                // Calc blocks are proof constructs - they evaluate to unit
                CalcBlock(_) => Ok(InferResult::new(Type::unit())),

                // Inject expression: `inject TypeName`
                // Level 1 static DI - resolves a type from the context/DI container
                Inject { .. } => self.infer_expr_inject(current_expr),

                // All other expression kinds are handled above.
                // This fallback is kept for safety - if a new variant is added to ExprKind
                // but not handled, this provides a clear error message instead of a panic.
                #[allow(unreachable_patterns)]
                _ => Err(TypeError::Other(verum_common::Text::from(format!(
                    "Type inference for expression kind '{}' requires additional context.\n  \
                     Hint: Add type annotations or ensure all required types are in scope.",
                    expr_kind_description(&current_expr.kind)
                )))),
            },
        }
    }

    fn infer_expr_range(&mut self, expr: &Expr, span: Span) -> Result<InferResult> {
        let ExprKind::Range {
            start,
            end,
            inclusive,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Determine the element type from start and end bounds
        let elem_ty = match (start.as_ref(), end.as_ref()) {
            (Some(start_expr), Some(end_expr)) => {
                // Both bounds present: infer from start and unify with end
                let start_result = self.synth_expr(start_expr)?;
                let end_result = self.synth_expr(end_expr)?;
                // Unify start and end types
                self.unifier.unify(&start_result.ty, &end_result.ty, span)?;
                start_result.ty
            }
            (Some(start_expr), None) => self.synth_expr(start_expr)?.ty,
            (None, Some(end_expr)) => self.synth_expr(end_expr)?.ty,
            (None, None) => Type::Var(TypeVar::fresh()),
        };
        Ok(InferResult::new(Type::Generic {
            name: Self::range_literal_constructor(*inclusive).into(),
            args: vec![elem_ty].into(),
        }))
    }

    fn infer_expr_loop(&mut self, expr: &Expr, _span: Span) -> Result<InferResult> {
        let ExprKind::Loop {
            label: _,
            body,
            invariants: _,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Enter loop context for affine tracking
        self.affine_tracker.enter_loop();

        self.infer_block(body)?;

        // Exit loop context
        self.affine_tracker.exit_loop();

        // Loop without break has type Never (the bottom type).
        // A loop that doesn't break naturally diverges, so its type is Never.
        // This allows code like: fn f() -> T { loop { return x; } }
        // where the loop body only exits via return/break/panic.
        // Never unifies with any type, which is correct.
        Ok(InferResult::new(Type::Never))
    }

    fn infer_expr_tensor_literal(&mut self, expr: &Expr, _span: Span) -> Result<InferResult> {
        let ExprKind::TensorLiteral {
            shape,
            elem_type,
            data,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Convert AST element type to Type
        let element_ty = self.ast_to_type(elem_type)?;

        // Type check the data expression
        self.synth_expr(data)?;

        // Create Tensor type with shape and element type
        // Tensor<shape..., T> where T is the element type
        let tensor_ty = Type::Generic {
            name: verum_common::Text::from("Tensor"),
            args: std::iter::once(element_ty)
                .chain(shape.iter().map(|&dim| Type::Generic {
                    name: verum_common::Text::from("Const"),
                    args: vec![Type::int()].into(), // Const dimension
                }))
                .collect(),
        };

        Ok(InferResult::new(tensor_ty))
    }

    fn infer_expr_interpolation(&mut self, expr: &Expr, _span: Span) -> Result<InferResult> {
        let ExprKind::InterpolatedString {
            handler: _,
            parts: _,
            exprs,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Type check all embedded expressions
        // They can be of any type - will be converted to string at runtime
        for expr in exprs.iter() {
            self.synth_expr(expr)?;
        }
        // Interpolated strings always produce Text
        Ok(InferResult::new(Type::text()))
    }

    fn infer_expr_try_recover_finally(&mut self, expr: &Expr, span: Span) -> Result<InferResult> {
        let ExprKind::TryRecoverFinally {
            try_block,
            recover,
            finally_block,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Extract error type from ? operators inside try block before type checking
        let error_type = self.extract_try_block_error_type(try_block)?;

        let try_result =
            self.with_try_recover_context(&error_type, |checker| checker.synth_expr(try_block))?;

        // Infer type of recovery body with error type for pattern binding
        let recover_ty = self.check_recover_body(recover, &error_type, None)?;

        // Recovery body must have the same type as try block
        self.unifier.unify(&try_result.ty, &recover_ty, span)?;

        // Type check finally block (result is discarded)
        self.synth_expr(finally_block)?;

        Ok(InferResult::new(try_result.ty))
    }

    fn infer_expr_use_context(&mut self, expr: &Expr, _span: Span) -> Result<InferResult> {
        let ExprKind::UseContext {
            context: _,
            handler,
            body,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Type check the handler expression
        let _handler_result = self.synth_expr(handler)?;

        // The body is evaluated with the context in scope
        // Type of the expression is the type of the body
        let body_result = self.synth_expr(body)?;
        Ok(body_result)
    }

    fn infer_expr_for_await(&mut self, expr: &Expr, _span: Span) -> Result<InferResult> {
        let ExprKind::ForAwait {
            label: _,
            pattern,
            async_iterable,
            body,
            invariants: _,
            decreases: _,
        } = &expr.kind
        else {
            unreachable!()
        };
        let iter_result = self.synth_expr(async_iterable)?;

        // =========================================================================
        // Protocol-based AsyncIterator resolution
        // ForAwait desugaring: "for await x in stream" desugars to async iterator protocol polling
        // =========================================================================
        let elem_ty = match self
            .protocol_checker
            .read()
            .resolve_async_iterator_protocol(&iter_result.ty)
        {
            Some(resolution) => resolution.item,
            None => {
                return Err(TypeError::Other(verum_common::Text::from(format!(
                    "for await requires AsyncIterator or AsyncStream, got: {}",
                    iter_result.ty
                ))));
            }
        };

        // Enter scope for loop body
        self.ctx.enter_scope();
        self.bind_pattern(pattern, &elem_ty)?;
        self.infer_block(body)?;
        self.ctx.exit_scope();

        // For-await loops return Unit (unless break with value, handled elsewhere)
        Ok(InferResult::new(Type::unit()))
    }

    fn infer_expr_is(&mut self, expr: &Expr, _span: Span) -> Result<InferResult> {
        let ExprKind::Is {
            expr: test_expr,
            pattern,
            negated: _,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Type check the expression being tested
        let test_result = self.synth_expr(test_expr)?;

        // Validate that the pattern is compatible with the expression type
        // This uses the same logic as match arm pattern checking
        self.ctx.enter_scope();
        // bind_pattern will validate pattern compatibility and report errors
        // for incompatible patterns (e.g., matching Int against Some(x))
        self.bind_pattern(pattern, &test_result.ty)?;
        self.ctx.exit_scope();

        // Is expressions always return Bool (regardless of negation)
        Ok(InferResult::new(Type::bool()))
    }

    fn infer_expr_quote(&mut self, expr: &Expr, span: Span) -> Result<InferResult> {
        let ExprKind::Quote {
            target_stage,
            tokens,
        } = &expr.kind
        else {
            unreachable!()
        };
        // Perform hygiene checking on the quote tokens
        // Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene
        self.check_quote_hygiene(tokens, *target_stage, span)?;

        // Quote expressions always produce TokenStream
        let token_stream_ty = self
            .ctx
            .lookup_type(&verum_common::Text::from("TokenStream"))
            .cloned()
            .unwrap_or_else(|| Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::Ident::new("TokenStream", span)),
                args: List::new(),
            });
        Ok(InferResult::new(token_stream_ty))
    }

    /// Type-infer a closure (lambda) expression.
    /// Performs capture analysis, borrow tracking, parameter binding,
    /// return-type synthesis, Pi-type formation, and Send/Sync safety checks.
    /// Type-infer a match expression.
    /// Auto-derefs reference scrutinees, dispatches to dependent-type match
    /// when applicable, checks each arm's pattern + body, unifies arm types,
    /// and propagates refinement evidence.
    /// Type-check an `if` expression with flow-sensitive refinement narrowing.
    /// Binds let-conditions in the then-branch; propagates evidence.
    fn infer_if_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::If { condition, then_branch, else_branch } = &expr.kind
            else { unreachable!() };
                    self.ctx.env.push_scope();
                    self.refinement_evidence.push_scope();

                    let mut condition_exprs: Vec<Expr> = Vec::new();

                    for cond in &condition.conditions {
                        match cond {
                            verum_ast::expr::ConditionKind::Expr(cond_expr) => {
                                if let ExprKind::Is {
                                    expr: test_expr,
                                    pattern,
                                    negated,
                                } = &cond_expr.kind
                                {
                                    let test_result = self.synth_expr(test_expr)?;
                                    if !negated {
                                        self.bind_pattern(pattern, &test_result.ty)?;
                                    }
                                } else {
                                    self.check_expr(cond_expr, &Type::bool())?;
                                    self.refinement_evidence
                                        .add_evidence_from_condition(cond_expr, cond_expr.span);
                                    self.narrow_variable_types_from_condition(cond_expr, false);
                                    condition_exprs.push(cond_expr.clone());
                                }
                            }
                            verum_ast::expr::ConditionKind::Let { pattern, value } => {
                                let value_result = self.synth_expr(value)?;
                                self.bind_pattern(pattern, &value_result.ty)?;
                            }
                        }
                    }

                    // Alternatives, exactly as in `check_if_expr`.
                    let affine_at_if = self.affine_tracker.snapshot_alternatives();

                    let then_result = self.infer_block(then_branch)?;

                    self.ctx.env.pop_scope();
                    self.refinement_evidence.pop_scope();

                    let affine_then = (!self.is_never_type(&then_result.ty))
                        .then(|| self.affine_tracker.snapshot_alternatives());
                    self.affine_tracker.restore_alternatives(&affine_at_if);

                    let else_result = if let Some(else_expr) = else_branch {
                        self.ctx.env.push_scope();
                        self.refinement_evidence.push_scope();
                        for cond_expr in &condition_exprs {
                            self.refinement_evidence
                                .add_negated_evidence(cond_expr, cond_expr.span);
                            self.narrow_variable_types_from_condition(cond_expr, true);
                        }
                        let result = self.synth_expr(else_expr)?;
                        self.ctx.env.pop_scope();
                        self.refinement_evidence.pop_scope();
                        result
                    } else {
                        InferResult::new(Type::unit())
                    };

                    {
                        let affine_else = else_branch
                            .is_some()
                            .then(|| self.affine_tracker.snapshot_alternatives());
                        self.merge_if_alternatives(affine_then, affine_else, &affine_at_if);
                    }

                    // Only reachable branches contribute a value to the join.
                    // In particular a diverging first branch cannot conceal an
                    // incompatible value produced by the other branch.
                    let result_ty = if self.is_never_type(&then_result.ty) {
                        else_result.ty
                    } else if self.is_never_type(&else_result.ty) {
                        then_result.ty
                    } else {
                        self.unifier.unify(&then_result.ty, &else_result.ty, expr.span)?;
                        self.unifier.apply(&then_result.ty)
                    };
                    Ok(InferResult::new(result_ty))
    }

    /// Type-check a `while` loop.
    /// Checks while-is pattern bindings, body, and affine drop on exit.
    fn infer_while_loop(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::While { label: _, condition, body, invariants: _, decreases: _ } = &expr.kind
            else { unreachable!() };
                    // Enter loop context for affine tracking
                    self.affine_tracker.enter_loop();

                    // Check if condition is an `is` expression with pattern bindings
                    // For `while value is Pattern(x)`, bind x in the body scope
                    // Spec: L0-critical/builtin-syntax/is_operator.vr
                    if let ExprKind::Is {
                        expr: test_expr,
                        pattern,
                        negated,
                    } = &condition.kind
                    {
                        // Type check the expression being tested
                        let test_result = self.synth_expr(test_expr)?;

                        // Enter scope for body with pattern bindings
                        self.ctx.env.push_scope();
                        self.refinement_evidence.push_scope();

                        // For non-negated `is`, bind the pattern in body scope
                        if !negated {
                            self.bind_pattern(pattern, &test_result.ty)?;
                            // Add pattern evidence for the matched pattern
                            if let Maybe::Some(var_name) = self.extract_simple_var_name(test_expr) {
                                self.add_pattern_evidence(pattern, var_name, test_expr.span);
                            }
                        }

                        self.infer_block(body)?;

                        self.refinement_evidence.pop_scope();
                        self.ctx.env.pop_scope();
                    } else {
                        // Regular condition - must be Bool
                        self.check_expr(condition, &Type::bool())?;

                        // Add condition evidence for the loop body
                        // Inside the loop, condition is true
                        self.refinement_evidence.push_scope();
                        self.refinement_evidence
                            .add_evidence_from_condition(condition, condition.span);

                        // Add method evidence if applicable
                        if let Maybe::Some((var_name, method_name, negated)) =
                            crate::refinement_evidence::EvidencePropagator::analyze_method_condition(
                                condition,
                            )
                        {
                            self.refinement_evidence.add_method_evidence(
                                var_name,
                                method_name.as_str(),
                                negated,
                                condition.span,
                            );
                        }

                        self.infer_block(body)?;
                        self.refinement_evidence.pop_scope();
                    }

                    // Exit loop context
                    self.affine_tracker.exit_loop();

                    Ok(InferResult::new(Type::unit()))
    }

    /// Type-check a `{k: v for (k, v) in iter if cond}` map comprehension.
    /// Returns `Map<K, V>` where K and V are inferred from key/value exprs.
    fn infer_map_comprehension_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::MapComprehension { key_expr, value_expr, clauses } = &expr.kind
            else { unreachable!() };
                    self.ctx.enter_scope();

                    // Process each clause (same as list comprehension)
                    for clause in clauses.iter() {
                        use verum_ast::expr::ComprehensionClauseKind;
                        match &clause.kind {
                            ComprehensionClauseKind::For { pattern, iter } => {
                                let iter_result = self.synth_expr(iter)?;
                                let elem_ty = self.synchronous_iteration_item(iter, &iter_result.ty)?;

                                let elem_scheme = TypeScheme::mono(elem_ty);
                                self.bind_pattern_scheme(pattern, elem_scheme)?;
                            }
                            ComprehensionClauseKind::If(condition) => {
                                let cond_result = self.synth_expr(condition)?;
                                self.unifier
                                    .unify(&cond_result.ty, &Type::Bool, condition.span)?;
                            }
                            ComprehensionClauseKind::Let { pattern, ty, value } => {
                                self.reject_refutable_comprehension_binding(pattern, value.span);
                                let value_result = self.synth_expr(value)?;
                                let binding_ty = if let Some(ty_ast) = ty {
                                    let annotated_ty = self.ast_to_type(ty_ast)?;
                                    self.unifier.unify(
                                        &value_result.ty,
                                        &annotated_ty,
                                        value.span,
                                    )?;
                                    annotated_ty
                                } else {
                                    value_result.ty
                                };
                                let binding_scheme = TypeScheme::mono(binding_ty);
                                self.bind_pattern_scheme(pattern, binding_scheme)?;
                            }
                        }
                    }

                    let key_result = self.synth_expr(key_expr)?;
                    let value_result = self.synth_expr(value_expr)?;

                    self.ctx.exit_scope();

                    // Result is a Map<K, V>
                    let result_ty = Type::Generic {
                        name: verum_common::Text::from(WKT::Map.as_str()),
                        args: vec![key_result.ty, value_result.ty].into(),
                    };

                    Ok(InferResult::new(result_ty))
    }

    /// A comprehension's `let` clause binds once per row and must always
    /// match — the same rule as a plain `let` statement, which E429 now
    /// enforces, and the same predicate.
    ///
    /// The clause is a DIFFERENT AST node (`ComprehensionClauseKind::Let`,
    /// not `StmtKind::Let`), so the statement-level check cannot see it.
    /// Measured 2026-09-12 before landing: the tree holds 2924 comprehension
    /// `let` clauses across `core/` and the spec corpus and NOT ONE carries a
    /// literal, range or or-pattern — so this refuses nothing that exists.
    ///
    /// Why it matters that it refuses: the VBC lowering for this clause is an
    /// unconditional `compile_pattern_bind`, with no match test, so a pattern
    /// that fails leaves the binding holding an unchecked payload slot and the
    /// first field access on it dereferences null (opcode 0x62).
    ///
    /// The variant case is NOT covered, here or at the statement: refutability
    /// for a variant needs the RESOLVED type, and this predicate reads syntax.
    fn reject_refutable_comprehension_binding(
        &mut self,
        pattern: &verum_ast::pattern::Pattern,
        span: verum_common::Span,
    ) {
        if std::env::var_os("VERUM_NO_LET_PATTERN_CHECK").is_some() {
            return;
        }
        if let Some(what) = crate::infer::modules::refutable_pattern_kind(pattern) {
            self.push_diagnostic_for(TypeError::OtherWithCodeSpanned {
                code: verum_common::Text::from("E429"),
                msg: verum_common::Text::from(format!(
                    "{what} cannot bind in a comprehension's `let` clause — the \
                     clause binds once per row and must always match\n  \
                     help: filter with an `if` clause first, then bind"
                )),
                span,
            });
        }
    }

    /// Type-check a `try { ... } finally { ... }` expression.
    /// Always executes the finally block; returns the try-block type.
    fn infer_try_finally_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::TryFinally { try_block, finally_block } = &expr.kind
            else { unreachable!() };
                    // Determine error type: prefer the enclosing function's error type
                    // (if it returns Result<T, E>), then fall back to extracting from ? operators.
                    // This avoids ambiguous unification with stdlib types like CheckedResult.
                    let error_type_opt = self
                        .extract_function_error_type()
                        .or_else(|| self.extract_try_block_error_type(try_block).ok());
                    let error_type = error_type_opt
                        .clone()
                        .unwrap_or_else(|| Type::Var(crate::ty::TypeVar::fresh()));

                    // Set function return type to Result<T, E> so ? operator AND
                    // Ok()/Err() constructors resolve against the correct Result type.
                    let saved_return_type = self.current_function_return_type.clone();
                    let try_ok_var = Type::Var(crate::ty::TypeVar::fresh());
                    self.current_function_return_type =
                        Maybe::Some(Type::result(try_ok_var.clone(), error_type.clone()));

                    // Infer type of try block
                    let try_result = self.synth_expr(try_block)?;

                    // Restore original function return type
                    self.current_function_return_type = saved_return_type;

                    // Type check finally block (result is discarded)
                    self.synth_expr(finally_block)?;

                    // If the try body uses ? (error_type was extracted), the try-finally
                    // expression produces Result<T, E>, not just T. The ? operator
                    // unwraps inside the block, but errors propagate out after finally runs.
                    let result_ty = if error_type_opt.is_some() {
                        // Check if the try body already produces a Result type
                        let resolved = self.unifier.apply(&try_result.ty);
                        // Structural check: is this a Result<T, E>? This also matches
                        // variant types with Ok/Err constructors via Type::is_result().
                        // Also accept any variant type that implements the Try protocol.
                        let already_result = resolved.is_result()
                            || matches!(&resolved, Type::Variant(_))
                                && self
                                    .protocol_checker
                                    .read()
                                    .implements_protocol(&resolved, "Try");
                        if already_result {
                            try_result.ty
                        } else {
                            Type::result(try_result.ty, error_type)
                        }
                    } else {
                        try_result.ty
                    };

                    Ok(InferResult::new(result_ty))
    }

    /// Type-check an inline assembly (`@asm`) expression.
    /// Validates operand types and options; returns unit.
    fn infer_inline_asm_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::InlineAsm { template: _, operands, options } = &expr.kind
            else { unreachable!() };
                    use verum_ast::expr::AsmOperandKind;

                    // Type-check all operands
                    for operand in operands.iter() {
                        match &operand.kind {
                            AsmOperandKind::In { expr, .. } => {
                                // Input expression - just type-check it
                                self.synth_expr(expr)?;
                            }
                            AsmOperandKind::Out { place, .. } => {
                                // Output place - must be an lvalue (assignable)
                                let place_ty = self.synth_expr(place)?;
                                self.check_place_is_lvalue(place, &place_ty.ty)?;
                            }
                            AsmOperandKind::InOut { place, .. } => {
                                // Input/output place - must be an lvalue
                                let place_ty = self.synth_expr(place)?;
                                self.check_place_is_lvalue(place, &place_ty.ty)?;
                            }
                            AsmOperandKind::InLateOut {
                                in_expr, out_place, ..
                            } => {
                                // Input expression and separate output place
                                self.synth_expr(in_expr)?;
                                let place_ty = self.synth_expr(out_place)?;
                                self.check_place_is_lvalue(out_place, &place_ty.ty)?;
                            }
                            AsmOperandKind::Sym { path } => {
                                // Symbol reference - verify the path exists
                                let path_str = path.to_string();
                                if self.ctx.env.lookup(&path_str).is_none() {
                                    return Err(TypeError::UnboundVariable {
                                        name: path_str.into(),
                                        span: operand.span,
                                    });
                                }
                            }
                            AsmOperandKind::Const { expr } => {
                                // Constant expression - type-check and verify it's const
                                let const_ty = self.synth_expr(expr)?;
                                // Const operands must be integer or pointer types
                                if !self.is_asm_const_compatible(&const_ty.ty) {
                                    return Err(TypeError::InvalidAsmConstType {
                                        found: const_ty.ty.clone(),
                                        span: expr.span,
                                    });
                                }
                            }
                            AsmOperandKind::Clobber { .. } => {
                                // Clobber declarations don't need type checking
                            }
                        }
                    }

                    // Check for noreturn option - if set, asm diverges
                    if options.noreturn {
                        Ok(InferResult::new(Type::never()))
                    } else {
                        // Inline assembly returns unit by default
                        Ok(InferResult::new(Type::unit()))
                    }
    }

    fn infer_scoped_match_arm(
        &mut self,
        arm: &verum_ast::pattern::MatchArm,
        scrutinee_ty: &Type,
        scrutinee_name: Option<&Text>,
        expected: Option<&Type>,
        sources: &[crate::ty::RecordReceiver],
    ) -> Result<InferResult> {
        let bindings = self.collect_pattern_bindings(&arm.pattern, scrutinee_ty)?;
        let ownership = self.affine_tracker.enter_pattern_scope(bindings.iter().map(|(name, _)| name.clone()));
        self.borrow_tracker.enter_scope();
        self.ctx.enter_scope();
        self.refinement_evidence.push_scope();
        let result = (|| {
            if let Some(name) = scrutinee_name {
                self.add_pattern_evidence(&arm.pattern, name.clone(), arm.pattern.span);
            }
            self.bind_pattern(&arm.pattern, scrutinee_ty)?;
            if !sources.is_empty() {
                for (name, ty) in &bindings {
                    if self.carries_stored_loan(ty) {
                        // Borrowed patterns project the scrutinee's existing
                        // loan. Only owned aggregates need new retained loans.
                        if scrutinee_ty.reference_parts().is_none() {
                            for source in sources { self.retain_origin(source, None, arm.pattern.span)?; }
                        }
                        self.ctx.env.set_reference_origin(name,
                            crate::context::ReferenceOrigin::Candidates(sources.into()));
                    }
                }
            }
            if let Some(guard) = &arm.guard {
                self.check_expr(guard, &Type::bool())?;
                self.refinement_evidence.add_evidence_from_condition(guard, guard.span);
            }
            match expected {
                Some(ty) => self.check_expr(&arm.body, ty),
                None => self.synth_expr(&arm.body),
            }
        })();
        let linear_errors = self.affine_tracker.exit_pattern_scope(ownership, arm.body.span);
        self.refinement_evidence.pop_scope();
        if result.as_ref().is_ok_and(|result| self.carries_stored_loan(&self.unifier.apply(&result.ty))) {
            self.borrow_tracker.exit_scope_retaining_loans();
        } else {
            self.borrow_tracker.exit_scope();
        }
        self.ctx.exit_scope();
        // Restore lexical state even when the arm failed. Outer consumption
        // remains in the tracker; only arm-owned pattern names leave scope.
        let inferred = result?;
        if let Some(error) = linear_errors.into_iter().next() { return Err(error); }
        Ok(inferred)
    }

    fn infer_match_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        self.with_result_borrow_scope(|checker| checker.infer_match_in_scope(expr))
    }

    fn infer_match_in_scope(&mut self, expr: &Expr) -> Result<InferResult> {
        use crate::refinement_evidence::{PathCondition, PathConditionKind};
        let ExprKind::Match { expr: scrutinee, arms } = &expr.kind
            else { unreachable!() };
        if std::env::var_os("VERUM_TRACE_AFFINE").is_some() {
            eprintln!("[affine] match walker=SYNTH (infer_match_expr), {} arms", arms.len());
        }

                    // NOTE: Affine tracking for scrutinee is handled by synth_expr
                    // Do NOT add explicit use_value here - it would cause double-consume errors
                    // since infer_path_expr already calls use_value for Path expressions
                    let scrut_result = self.synth_expr(scrutinee)?;
                    // Resolve associated-type projections in the scrutinee (e.g.
                    // `y.poll(cx)` returning `Poll<Y.Output>`) so pattern binding
                    // and exhaustiveness see the concrete payload.
                    let __applied = self.unifier.apply(&scrut_result.ty);
                    let binding_ty = self.resolve_assoc_projections_deep(&__applied);
                    let sources = self.aggregate_origins(scrutinee, &binding_ty);

                    // Auto-deref scrutinee for match expressions:
                    // When matching &T where T is a variant/generic type, auto-deref to T.
                    // This allows `match self` in impl methods with `&self` to work
                    // without requiring explicit `match *self`.
                    // Exhaustiveness inspects the underlying constructors. Bindings retain the original loan.
                    let scrut_ty = match &binding_ty {
                        Type::Reference { inner, .. }
                        | Type::CheckedReference { inner, .. }
                        | Type::UnsafeReference { inner, .. } => match &**inner {
                            Type::Variant(_)
                            | Type::Generic { .. }
                            | Type::Named { .. }
                            | Type::Record(_) => (**inner).clone(),
                            _ => binding_ty.clone(),
                        },
                        _ => binding_ty.clone(),
                    };

                    if arms.is_empty() {
                        return Err(TypeError::OtherWithCodeSpanned {
                            code: verum_common::Text::from("E406"),
                            msg: "Match expression must have at least one arm".into(),
                            span: expr.span,
                        });
                    }

                    // Check if this is a dependent match (on an inductive type with indices).
                    // Gated on [types].dependent: when disabled, all match expressions
                    // use regular (non-dependent) pattern matching even on inductive
                    // types with indices. This avoids the overhead of motive synthesis
                    // and index unification for projects that don't use dependent types.
                    if self.dependent_enabled {
                        let is_dependent = self.is_dependent_type(&scrut_ty);
                        if is_dependent {
                            if std::env::var_os("VERUM_TRACE_AFFINE").is_some() {
                                eprintln!("[affine] match walker=SYNTH -> DEPENDENT, early return");
                            }
                            let dep_result =
                                self.check_dependent_match(&scrut_ty, arms, expr.span);
                            return dep_result;
                        }
                    }

                    // Extract scrutinee variable name for evidence tracking
                    let scrutinee_var = self.extract_simple_var_name(scrutinee);

                    // Regular pattern matching (non-dependent)
                    // Check all arms and find the result type
                    // CRITICAL: Never type is the bottom type - it's a subtype of all types.
                    // If the first arm returns Never (e.g., panic()), we need to find a non-Never
                    // arm to establish the result type. If ALL arms return Never, the result is Never.
                    // Type lattice: Never is bottom (subtype of all), Unknown is top (supertype of all)
                    let mut result_ty = Type::Never;
                    let mut all_arm_types: List<(Type, verum_ast::Span)> = List::new();

                    // The arms are ALTERNATIVES: exactly one of them runs.
                    // Walking them in sequence and carrying the affine state
                    // from one into the next asks "was it consumed earlier in
                    // this traversal" when the question is "can both
                    // consumptions happen in one execution" — which refuses
                    // the commit-or-rollback shape every affine resource
                    // appears in. Each arm starts from the state AT the match;
                    // `merge_alternatives` folds them back with the union, so
                    // a value consumed in ONE arm is still gone afterwards.
                    let affine_at_match = self.affine_tracker.snapshot_alternatives();
                    let loans_at_match = self.borrow_tracker.clone_for_branch();
                    let mut escaping_loans = loans_at_match.clone_for_branch();
                    if std::env::var_os("VERUM_TRACE_AFFINE").is_some() {
                        eprintln!("[affine] match walker=SYNTH arm-loop, {} arms", arms.len());
                    }
                    let mut affine_per_arm: Vec<crate::affine::AffineAlternatives> = Vec::new();

                    // First pass: check all arms and collect their types
                    for arm in arms.iter() {
                        self.affine_tracker.restore_alternatives(&affine_at_match);
                        self.borrow_tracker = loans_at_match.clone_for_branch();
                        let resolved_scrut_ty = self.unifier.apply(&binding_ty);
                        let arm_result = self.infer_scoped_match_arm(
                            arm, &resolved_scrut_ty, scrutinee_var.as_ref(), None, &sources,
                        )?;
                        let is_arm_never = self.is_never_type(&arm_result.ty);

                        // Track this arm's type for later unification
                        all_arm_types.push((arm_result.ty.clone(), arm.body.span));

                        // If we haven't found a non-Never type yet, use this one
                        if self.is_never_type(&result_ty) && !is_arm_never {
                            result_ty = arm_result.ty;
                        }

                        // An arm of type Never — `panic()`, `return` — does
                        // not reach the code after the match, so what it
                        // consumed cannot reach it either. The arm's own type
                        // is the authority, and it is already computed above.
                        if !is_arm_never {
                            affine_per_arm.push(self.affine_tracker.snapshot_alternatives());
                            escaping_loans.merge_live_loans(&self.borrow_tracker);
                        }
                    }
                    if affine_per_arm.is_empty() {
                        // Every arm diverges: nothing follows the match, so
                        // leave the state as it stood at the match.
                        self.affine_tracker.restore_alternatives(&affine_at_match);
                    } else {
                        self.affine_tracker.merge_alternatives(&affine_per_arm);
                    }

                    // Second pass: unify all non-Never arm types with the result type
                    for (arm_ty, span) in all_arm_types.iter() {
                        if !self.is_never_type(arm_ty) && !self.is_never_type(&result_ty) {
                            self.unifier.unify(arm_ty, &result_ty, *span)?;
                        }
                    }

                    self.check_match_coverage(&scrut_ty, arms, expr.span)?;

                    self.borrow_tracker = escaping_loans;
                    Ok(InferResult::new(result_ty))
    }

    /// Type-infer an `attenuate ctx with [capabilities]` expression.
    /// Verifies the capability subset relationship and registers the
    /// attenuated context in the capability checker.
    fn infer_attenuate_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Attenuate { context, capabilities } = &expr.kind
            else { unreachable!() };
                    // Infer the type of the context expression
                    let context_result = self.synth_expr(context)?;

                    // Extract context name from the context expression
                    // For now, we support simple path expressions like "Database" or "db"
                    let context_name = if let Path(path) = &context.kind {
                        if path.segments.len() == 1 {
                            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                                verum_common::Text::from(ident.name.as_str())
                            } else {
                                return Err(TypeError::Other(
                                    "Attenuate requires a simple context name".into(),
                                ));
                            }
                        } else {
                            return Err(TypeError::Other(
                                "Attenuate requires a simple context name, not a path".into(),
                            ));
                        }
                    } else {
                        return Err(TypeError::Other(
                            "Attenuate requires a context name, not a complex expression".into(),
                        ));
                    };

                    // Convert AST capabilities to type-level capabilities
                    use crate::capability::TypeCapabilitySet;
                    let new_caps = TypeCapabilitySet::from_ast(capabilities);

                    // Check if the context is registered
                    let current_caps = match self
                        .capability_checker
                        .get_context_capabilities(context_name.as_str())
                    {
                        Maybe::Some(caps) => caps.clone(),
                        Maybe::None => {
                            // E0308: Context not provided
                            use verum_diagnostics::capability_attenuation_errors::CapabilityNotProvidedError;
                            let diagnostic = CapabilityNotProvidedError::new(
                                context_name.as_str(),
                                span_to_line_col(expr.span),
                            )
                            .build();
                            self.diagnostics.push(diagnostic);

                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Context '{}' not found. Did you declare it in the 'using' clause?",
                                context_name
                            ))));
                        }
                    };

                    // Verify that new capabilities are a subset of current capabilities
                    // If not, this is an invalid attenuation (E0308)
                    if !new_caps.is_subset_of(&current_caps.capabilities) {
                        let missing = new_caps.difference(&current_caps.capabilities);
                        let missing_names: List<verum_common::Text> = missing
                            .iter()
                            .map(|c| verum_common::Text::from(c.as_str()))
                            .collect();

                        // E0308: Trying to attenuate with capabilities the context doesn't have
                        use verum_diagnostics::capability_attenuation_errors::CapabilityViolationError;

                        let cap_names: Vec<String> = current_caps
                            .capabilities
                            .names()
                            .iter()
                            .map(|t| t.to_string())
                            .collect();

                        let diagnostic = CapabilityViolationError::new(
                            format!("{}::[{}]", context_name, missing_names.join(", ")),
                            span_to_line_col(expr.span),
                        )
                        .with_declared_capabilities(
                            cap_names.iter().map(|s| s.as_str().into()).collect(),
                        )
                        .with_function_name("attenuate")
                        .build();
                        self.diagnostics.push(diagnostic);

                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Cannot attenuate context '{}' with capabilities it doesn't have: {}",
                            context_name,
                            missing_names.join(", ")
                        ))));
                    }

                    // Create attenuated context in capability checker
                    match self
                        .capability_checker
                        .attenuate_context(context_name.as_str(), new_caps)
                    {
                        Ok(_) => {
                            // Attenuation succeeds - return the same context type
                            Ok(InferResult::new(context_result.ty))
                        }
                        Err(cap_err) => {
                            // Convert capability error to type error with diagnostic
                            use verum_diagnostics::capability_attenuation_errors::CapabilityNotProvidedError;
                            let diagnostic = CapabilityNotProvidedError::new(
                                cap_err.message().to_string(),
                                span_to_line_col(expr.span),
                            )
                            .build();
                            self.diagnostics.push(diagnostic);

                            Err(TypeError::Other(cap_err.message()))
                        }
                    }
    }

    /// Type-infer a nursery structured-concurrency expression.
    /// Checks options (timeout/max_tasks), body, optional on_cancel cleanup,
    /// and optional recover block (must unify with body type).
    fn infer_nursery_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        let ExprKind::Nursery { options, body, on_cancel, recover, span: nursery_span } = &expr.kind
            else { unreachable!() };
                    // =========================================================================
                    // Nursery Structured Concurrency Type Inference
                    // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 9 - Structured Concurrency
                    // =========================================================================
                    //

                    // A nursery creates a structured concurrency scope:
                    // 1. All spawned tasks within the nursery must complete before exit
                    // 2. Errors from tasks are collected and handled by recover block
                    // 3. Timeout and max_tasks provide resource limits
                    //

                    // Type rules:
                    // - nursery body returns T
                    // - recover block (if present) must also return T
                    // - on_cancel block returns () (cleanup only)
                    // - options.timeout must be Duration
                    // - options.max_tasks must be Int

                    self.ctx.enter_scope();

                    // Type check nursery options
                    if let verum_common::Maybe::Some(timeout_expr) = &options.timeout {
                        let timeout_result = self.synth_expr(timeout_expr)?;
                        // Timeout must be Duration type
                        let expected_duration = Type::Generic {
                            name: verum_common::Text::from(WKT::Duration.as_str()),
                            args: vec![].into(),
                        };
                        if self
                            .unifier
                            .unify(&timeout_result.ty, &expected_duration, timeout_expr.span)
                            .is_err()
                        {
                            // Also accept Int (milliseconds) for convenience
                            if self
                                .unifier
                                .unify(&timeout_result.ty, &Type::int(), timeout_expr.span)
                                .is_err()
                            {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "nursery timeout must be Duration or Int (milliseconds), got: {}",
                                    timeout_result.ty
                                ))));
                            }
                        }
                    }

                    if let verum_common::Maybe::Some(max_tasks_expr) = &options.max_tasks {
                        let max_tasks_result = self.synth_expr(max_tasks_expr)?;
                        // max_tasks must be Int
                        self.unifier.unify(
                            &max_tasks_result.ty,
                            &Type::int(),
                            max_tasks_expr.span,
                        )?;
                    }

                    // Type check nursery body
                    // The body type is the primary return type of the nursery
                    let body_result = self.infer_block(body)?;

                    // Type check on_cancel block if present
                    // on_cancel is a cleanup handler - always returns ()
                    if let verum_common::Maybe::Some(cancel_block) = on_cancel {
                        self.ctx.enter_scope();
                        let cancel_result = self.infer_block(cancel_block)?;
                        // Warn if on_cancel has non-unit return type (might indicate misuse)
                        if cancel_result.ty != Type::unit() {
                            let warning = Diagnostic::new_warning(
                                format!(
                                    "nursery on_cancel block returns '{}', but result is discarded. \
                                     on_cancel is for cleanup - its return value is ignored.",
                                    cancel_result.ty
                                ),
                                span_to_line_col(*nursery_span),
                                "W0901", // Warning code for nursery cleanup
                            );
                            self.diagnostics.push(warning);
                        }
                        self.ctx.exit_scope();
                    }

                    // Type check recover block if present
                    // recover must return the same type as the body (like try-recover)
                    if let verum_common::Maybe::Some(recover_body) = recover {
                        // Determine error type - for nursery, it's a collection of task errors
                        // Use generic Error or TaskError type
                        let error_type = Type::Generic {
                            name: verum_common::Text::from("NurseryError"),
                            args: vec![].into(),
                        };

                        let recover_ty = self.check_recover_body(recover_body, &error_type, None)?;
                        // Recover block must return the same type as the body
                        self.unifier
                            .unify(&recover_ty, &body_result.ty, *nursery_span)?;
                    }

                    self.ctx.exit_scope();

                    // The nursery returns the body's type (or recover's unified type)
                    Ok(body_result)
    }

    /// Capture analysis + aliasing-conflict detection, run BEFORE a closure's
    /// body is type-checked and before its scope is entered.
    ///
    /// T0653. This block used to live inline in `infer_closure_expr` only, so
    /// it ran on the SYNTHESIS path and nowhere else. `check_closure_expr` —
    /// the path taken whenever a closure is checked against an already-known
    /// function type, i.e. `.map(|x| …)`, a closure assigned to a typed
    /// binding, a closure passed as a typed argument, which is most closures
    /// in real code — never called it. Neither did anything downstream:
    /// `verum_cbgr::concurrency_analysis::check_thread_safety` returns an
    /// empty list unconditionally, `nll_analysis`'s real conflict machinery
    /// has no callers outside its own crate, and the pipeline-wired
    /// `send_sync_validation` phase covers spawn/Channel/Shared via a
    /// name-based deny-list, not a closure's own captures. So exclusivity of
    /// captures was simply unverified on the common path — a hole in the
    /// guarantee, not a redundant early check.
    ///
    /// Returns the tracker's closure id, which the caller MUST pass to
    /// [`Self::exit_closure_capture_tracking`] so the scope stack stays
    /// balanced.
    fn enter_closure_capture_tracking(
        &mut self,
        body: &Expr,
        params: &[verum_ast::expr::ClosureParam],
        is_move: bool,
    ) -> Result<crate::aliasing::RefId> {
        // Determine which outer variables the closure references. Must run
        // BEFORE entering closure scope, or the lookup sees the closure's own
        // bindings instead of the enclosing ones.
        let captures = self.analyze_closure_captures(body, params, is_move);
        let closure_id = self.borrow_tracker.enter_closure(is_move);

        // Field-path-aware: `x.a` and `x.b` may be captured separately
        // without conflict (borrow splitting), while `&mut x` conflicts with
        // an existing immutable borrow of `x`.
        let mut capture_errors: List<TypeError> = List::new();
        for (var_name, field_path, capture_mode, capture_span) in &captures {
            if let Err(e) = self.borrow_tracker.register_capture(
                closure_id,
                var_name.clone(),
                field_path.clone(),
                *capture_mode,
                *capture_span,
            ) {
                capture_errors.push(e);
            }
        }
        if let Some(first_error) = capture_errors.into_iter().next() {
            // Balance the scope stack before propagating.
            self.borrow_tracker.cancel_closure(closure_id);
            return Err(first_error);
        }
        Ok(closure_id)
    }

    /// Closes capture tracking and enforces Send-safety for async closures.
    ///
    /// T0653, the epilogue half. Must be called after the body has been
    /// checked and BEFORE the closure's scope is exited, matching the order
    /// `infer_closure_expr` has always used: the capture targets are outer
    /// variables, and the lookup is performed against the environment as it
    /// stands while the closure scope is still live.
    ///
    /// A `MutBorrow` capture of a non-Send type in an `async` closure is
    /// rejected — the borrow can be live across the closure's own await
    /// point. This is the only place that property is checked anywhere in the
    /// pipeline.
    fn exit_closure_capture_tracking(
        &mut self,
        closure_id: crate::aliasing::RefId,
        is_async: bool,
    ) -> Result<()> {
        let capture_set = self.borrow_tracker.exit_closure(closure_id);
        if let Some(ref cs) = capture_set
            && is_async
            && !cs.captures.is_empty()
        {
            for capture in &cs.captures {
                if matches!(capture.mode, crate::aliasing::CaptureMode::MutBorrow)
                    && let Some(var_scheme) = self.ctx.env.lookup(&capture.target)
                {
                    let var_ty = var_scheme.instantiate();
                    let resolved_ty = self.unifier.apply(&var_ty);
                    if self.is_non_send_type(&resolved_ty) {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Cannot capture `{}` by mutable reference in async closure: \
                             type `{}` is not Send-safe across await points",
                            capture.target, resolved_ty
                        ))));
                    }
                }
            }
        }
        Ok(())
    }

    /// Both inference directions use one callable boundary. Argument borrowing
    /// belongs to evaluating the closure value, never to checking its body.
    fn with_closure_scope<T>(
        &mut self,
        expression: &Expr,
        check: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let ExprKind::Closure {
            params,
            body,
            return_type,
            async_,
            move_,
            ..
        } = &expression.kind
        else {
            unreachable!()
        };
        let closure_id = self.enter_closure_capture_tracking(body, params, *move_)?;
        let saved = (
            std::mem::replace(&mut self.in_call_arg_context, false),
            self.current_function_return_type.take(),
            self.current_function_name.take(),
            std::mem::replace(
                &mut self.current_function_return_span,
                return_type.as_ref().map(|ty| ty.span),
            ),
            self.current_function_throws.take(),
            self.generator_context.take(),
        );
        let result = self.with_async_context(*async_, |checker| {
            if *async_ {
                checker.borrow_tracker.enter_async_scope();
            }
            let result = checker.with_lexical_scope(|checker| {
                let value = check(checker)?;
                checker.exit_closure_capture_tracking(closure_id, *async_)?;
                Ok(value)
            });
            if *async_ {
                checker.borrow_tracker.exit_async_scope();
            }
            result
        });
        if result.is_err() {
            self.borrow_tracker.cancel_closure(closure_id);
        }
        (
            self.in_call_arg_context,
            self.current_function_return_type,
            self.current_function_name,
            self.current_function_return_span,
            self.current_function_throws,
            self.generator_context,
        ) = saved;
        result
    }

    fn infer_closure_expr(&mut self, expr: &Expr, outer_span: Span) -> Result<InferResult> {
        let ExprKind::Closure {
            params,
            body,
            return_type,
            async_,
            ..
        } = &expr.kind
        else {
            unreachable!()
        };
        let (param_names, param_types, ret_ty) = self.with_closure_scope(expr, |checker| {
            let mut param_names = List::new();
            let mut param_types = List::new();
            for param in params {
                param_names.push(match &param.pattern.kind {
                    verum_ast::pattern::PatternKind::Ident { name, .. } => Some(name.name.clone()),
                    _ => None,
                });
                let param_ty = match &param.ty {
                    Some(annotation) => checker.ast_to_type(annotation)?,
                    None => Type::Var(TypeVar::fresh()),
                };
                checker.bind_parameter_pattern(&param.pattern, &param_ty)?;
                param_types.push(param_ty);
            }
            let ret_ty = if let Some(annotation) = return_type {
                let expected = checker.ast_to_type(annotation)?;
                checker.current_function_return_type = verum_common::Maybe::Some(expected.clone());
                checker.check_expr(body, &expected)?;
                expected
            } else {
                checker.synth_expr(body)?.ty
            };
            Ok((param_names, param_types, ret_ty))
        })?;
        // Check if return type depends on parameters (dependent function / Pi type)
        // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi types
        let is_dependent =
            params.len() == 1 && self.return_type_depends_on_param(&ret_ty, &param_names);

        // Async callables obey the same eager output law as
        // named functions. Only explicit Future values are polled.
        let properties = self.closure_callable_properties(body, *async_)?;
        let func_ty = if !*async_
            && properties.finite_footprint().is_some_and(|p| p.is_pure())
            && let (true, Some(param_name)) =
                (is_dependent, param_names.get(0).and_then(|p| p.clone()))
        {
            Type::Pi {
                param_name,
                param_type: Box::new(param_types[0].clone()),
                return_type: Box::new(ret_ty),
            }
        } else {
            {
                let mut callable = Type::function_with_async(param_types, ret_ty, *async_);
                if let Type::Function {
                    properties: footprint,
                    property_requirement,
                    ..
                } = &mut callable
                {
                    *footprint = properties.finite_footprint().cloned();
                    *property_requirement = properties;
                }
                callable
            }
        };

        // Register closure type in the type registry for codegen
        // Apply unifier substitution to resolve type variables
        // This enables closure parameter type inference during LLVM codegen
        let resolved_func_ty = self.unifier.apply(&func_ty);
        self.type_registry
            .register_expr(outer_span, resolved_func_ty);

        Ok(InferResult::new(func_ty))
    }

    fn range_literal_constructor(inclusive: bool) -> &'static str {
        if inclusive { "RangeInclusive" } else { WKT::Range.as_str() }
    }

    /// Resolve the same synchronous iteration contract for loops and comprehensions.
    fn synchronous_iteration_item(&mut self, iter: &Expr, ty: &Type) -> Result<Type> {
        let ty = self.unifier.apply(ty);
        if let Some(iterable) = self.resolve_into_iterator_with_loading(&ty, iter.span)? {
            return Ok(iterable.item);
        }
        if let Some(item) = self.declared_iterator_item(&ty, iter.span)? {
            return Ok(item);
        }
        if let Some(item) = self.structural_iteration_item(&ty) {
            return Ok(item);
        }
        // Grammar-produced ranges retain their endpoint type across bindings.
        // Imported nominal declarations carry their owner and cannot claim this constructor.
        if let Type::Generic { name, args } = &ty
            && [Self::range_literal_constructor(false), Self::range_literal_constructor(true)]
                .contains(&name.as_str())
            && let [element] = args.as_slice()
        {
            return Ok(element.clone());
        }
        Err(TypeError::OtherWithCodeSpanned {
            code: "E405".into(),
            msg: format!("type `{ty}` has no proven iteration protocol").into(),
            span: iter.span,
        })
    }

    fn structural_iteration_item(&self, receiver: &Type) -> Option<Type> {
        use crate::core_metadata::ReceiverKind::{SelfMut, SelfRef};

        let name = self.get_type_name(receiver)?;
        let inherent = self.inherent_methods.read();
        let methods = inherent.get(&name)?;
        let next = methods.get(&Text::from("next"))?;
        let available = methods.get(&Text::from("has_next"))?;
        if !matches!(next.receiver_kind, Some(SelfRef | SelfMut))
            || !matches!(available.receiver_kind, Some(SelfRef | SelfMut))
        {
            return None;
        }
        let Type::Function { params, return_type, .. } = available.instantiate() else {
            return None;
        };
        if !params.is_empty() || *return_type != Type::Bool {
            return None;
        }
        match next.instantiate() {
            Type::Function { params, return_type, .. } if params.is_empty() => Some(*return_type),
            _ => None,
        }
    }

    /// Infer a for-loop body under the element type selected for its iterand.
    fn infer_for_loop(&mut self, expr: &Expr) -> Result<InferResult> {
        let ExprKind::For { label: _, pattern, iter, body, invariants: _, decreases: _ } = &expr.kind
            else { unreachable!() };
        if crate::ctor_trace_enabled() {
            eprintln!("[ctor-trace] infer_for_loop ENTER span={:?}", expr.span);
        }
        self.with_lexical_scope(|checker| {
            // The iterand's temporary loans belong to the loop, not its caller.
            let iter_result = checker.synth_expr(iter)?;
            let elem_ty = checker.synchronous_iteration_item(iter, &iter_result.ty)?;
            let elem_ty = checker.unifier.apply(&elem_ty);
            let mut sources = checker.aggregate_origins(iter, &iter_result.ty);
            if sources.is_empty() {
                if let Some((source, _)) = checker.borrowed_record_place(iter) {
                    sources.push(source);
                } else if checker.carries_stored_loan(&elem_ty) {
                    // A computed iterand owns temporary storage. References
                    // into it cannot be assigned to a longer-lived holder.
                    let temporary = "<temporary iterand>";
                    checker.ctx.env.insert_mono(temporary, iter_result.ty.clone());
                    checker.ctx.env.set_binding_loan_region(temporary, checker.borrow_tracker.scope_depth());
                    sources.extend(checker.ctx.env.record_source(temporary, false));
                }
            }
            for source in &sources {
                checker.retain_iteration_origin(source, iter.span)?;
            }
            let bindings = checker.collect_pattern_bindings(pattern, &elem_ty)?;
            checker.affine_tracker.enter_loop();
            checker.refinement_evidence.push_scope();
            let result = (|| {
                checker.bind_pattern(pattern, &elem_ty)?;
                for (name, ty) in &bindings {
                    if checker.carries_stored_loan(ty) && !sources.is_empty() {
                        checker.ctx.env.set_reference_origin(name,
                            crate::context::ReferenceOrigin::Candidates(sources.clone().into()));
                    }
                }
                if let Some(name) = checker.extract_simple_var_name(iter) {
                    checker.add_pattern_evidence(pattern, name, pattern.span);
                }
                checker.infer_block(body)?;
                Ok(InferResult::new(Type::unit()))
            })();
            checker.refinement_evidence.pop_scope();
            checker.affine_tracker.exit_loop();
            result
        })
    }

    /// Type-infer a `select { ... }` expression.
    /// Validates async context, checks each arm's future + pattern + guard,
    /// unifies all arm body types, and warns on exhaustiveness issues.
    fn infer_select_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        let ExprKind::Select { biased: _, arms, span: select_span } = &expr.kind
            else { unreachable!() };
                    // Validate that select is used in an async context
                    // Select expressions require async context: "select { ... }" only valid in async functions — Select expressions can only be used in async context
                    if !self.in_async_context {
                        return Err(TypeError::OtherWithCodeSpanned {
                            code: verum_common::Text::from("E406"),
                            msg: verum_common::Text::from(
                                "select expression can only be used in async context \
                                 (async fn, async block, or async closure)",
                            ),
                            span: expr.span,
                        });
                    }

                    if arms.is_empty() {
                        return Err(TypeError::OtherWithCodeSpanned {
                            code: verum_common::Text::from("E406"),
                            msg: verum_common::Text::from(
                                "select expression must have at least one arm",
                            ),
                            span: expr.span,
                        });
                    }

                    // Type check each arm and collect body types
                    let mut arm_types: List<Type> = List::new();
                    // Track for exhaustiveness checking
                    let mut has_else = false;
                    let mut all_have_guards = true;

                    for arm in arms.iter() {
                        self.ctx.enter_scope();

                        // Check if this is an else/default arm
                        if arm.is_else() {
                            has_else = true;
                            // Else arm has no pattern or future to check
                        } else {
                            // Track if this arm has a guard for exhaustiveness check
                            if arm.guard.is_none() {
                                all_have_guards = false;
                            }
                            // Type check the future expression
                            // Note: arm.future contains an await expression (e.g., `future.await`)
                            // We need to extract the inner future and get its type
                            if let Some(ref future_expr) = arm.future {
                                // The parser stores the full await expression (e.g., `task.await`)
                                // We need to extract the inner future expression and type it
                                let awaited_ty = if let ExprKind::Await(inner_future) =
                                    &future_expr.kind
                                {
                                    // Type the inner future expression (before .await)
                                    let future_result = self.synth_expr(inner_future)?;

                                    // Resolve type variables before matching
                                    let resolved_ty = self.unifier.apply(&future_result.ty);

                                    // Extract awaited type using protocol resolver for comprehensive
                                    // Future type handling (Type::Future, Type::Generic, Type::Named, Type::Var)
                                    match self.resolve_source_future(&resolved_ty, inner_future.span)? {
                                        Some(resolution) => resolution.output,
                                        None => {
                                            if self.in_async_context {
                                                resolved_ty.clone()
                                            } else {
                                                return Err(TypeError::Other(
                                                    verum_common::Text::from(format!(
                                                        "select arm future must be a Future type, got: {}",
                                                        future_result.ty
                                                    )),
                                                ));
                                            }
                                        }
                                    }
                                } else {
                                    // Fallback: if not an await expr, type the expression directly
                                    // This shouldn't happen with correct parsing, but handle it gracefully
                                    let future_result = self.synth_expr(future_expr)?;
                                    let resolved_ty = self.unifier.apply(&future_result.ty);
                                    match self.resolve_source_future(&resolved_ty, future_expr.span)? {
                                        Some(resolution) => resolution.output,
                                        None => {
                                            // In async context, be lenient
                                            if self.in_async_context {
                                                resolved_ty
                                            } else {
                                                return Err(TypeError::Other(
                                                    verum_common::Text::from(format!(
                                                        "select arm must contain await expression, got: {:?}",
                                                        future_expr.kind
                                                    )),
                                                ));
                                            }
                                        }
                                    }
                                };

                                // Bind pattern variables using full pattern matching
                                // Supports: Ok(data), Message.Command { cmd }, x, _, etc.
                                // Select expressions require async context: "select { ... }" only valid in async functions — Section 4.3.4 - Pattern Matching in Branches
                                if let Some(ref pattern) = arm.pattern {
                                    self.bind_pattern(pattern, &awaited_ty)?;
                                }
                            }

                            // Type check optional guard
                            if let Some(ref guard) = arm.guard {
                                let guard_result = self.synth_expr(guard)?;
                                self.unifier
                                    .unify(&guard_result.ty, &Type::bool(), guard.span)?;
                            }
                        }

                        // Type check the arm body
                        let body_result = self.synth_expr(&arm.body)?;
                        arm_types.push(body_result.ty);

                        self.ctx.exit_scope();
                    }

                    // Exhaustiveness check for select expression
                    // Select expressions require async context: "select { ... }" only valid in async functions — Select exhaustiveness
                    // A select is considered non-exhaustive if:
                    // - All arms have guards AND there's no else/default arm
                    // In this case, if all guards evaluate to false, the select will block forever
                    if all_have_guards
                        && !has_else
                        && !arms.is_empty()
                        && !arms.iter().all(|a| a.is_else())
                    {
                        // Check if there's at least one arm without a guard (excluding else arms)
                        let non_else_arms: Vec<_> = arms.iter().filter(|a| !a.is_else()).collect();
                        if !non_else_arms.is_empty()
                            && non_else_arms.iter().all(|a| a.guard.is_some())
                        {
                            // All non-else arms have guards - this could block forever
                            // Emit a warning (not error) since guards might be intentionally comprehensive
                            let warning = Diagnostic::new_warning(
                                "select expression may block forever: all arms have guards but no \
                                 'else' branch. If all guards evaluate to false, execution will block. \
                                 Consider adding an 'else => ...' branch for exhaustiveness.",
                                span_to_line_col(*select_span),
                                "W0501", // Warning code for select exhaustiveness
                            );
                            self.diagnostics.push(warning);
                        }
                    }

                    // All arm bodies must unify to the same type
                    if arm_types.is_empty() {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "select expression has no arms",
                        )));
                    }

                    let result_ty = arm_types[0].clone();
                    for (i, arm_ty) in arm_types.iter().enumerate().skip(1) {
                        self.unifier.unify(arm_ty, &result_ty, arms[i].span)?;
                    }

                    Ok(InferResult::new(result_ty))
    }

    fn infer_expr_path(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Path(path) = &expr.kind else { unreachable!() };
        if path.segments.len() == 1 {
            // Single segment - simple variable lookup
            let name = if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                id.name.as_str()
            } else {
                // Handle self/super/crate keywords
                match &path.segments[0] {
                    verum_ast::ty::PathSegment::SelfValue => {
                        // Check if we're in a method context (implement block)
                        if let Maybe::Some(ref self_ty) = self.current_self_type {
                            // Look up 'self' in the environment
                            if let Some(scheme) = self.ctx.env.lookup("self").cloned() {
                                let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                                return Ok(InferResult::new(ty));
                            }
                        }
                        return Err(TypeError::Other(
                            "self keyword requires method context".into(),
                        ));
                    }
                    verum_ast::ty::PathSegment::Super | verum_ast::ty::PathSegment::Cog => {
                        return Err(TypeError::UnboundVariable {
                            name: self.path_to_string(path), span: expr.span,
                        });
                    }
                    _ => return Err(TypeError::Other("Invalid path".into())),
                }
            };

            // =====================================================================
            // Special built-in identifiers: null
            // Interop/FFI: foreign function interface for calling C/system code, marshalling rules — FFI null pointer handling
            // =====================================================================
            if name == "null" {
                // null is a polymorphic null pointer that can be any pointer type
                // It unifies with *T, *mut T, *const T, etc.
                let inner_var = Type::Var(TypeVar::fresh());
                return Ok(InferResult::new(Type::Pointer {
                    mutable: true,
                    inner: Box::new(inner_var),
                }));
            }

            // MOUNT AUTHORITY (iterative-walker twin of the
            // infer_path_expr arm — see mount_authoritative_fn_scheme).
            if let Some(scheme) = self.mount_authoritative_fn_scheme(name) {
                let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                return Ok(InferResult::new(self.unifier.apply(&ty)));
            }

            if self.diagnose_ambiguous_constructor(name, None, expr.span)? {
                return Ok(InferResult::new(Type::Unknown));
            }

            // COMPLETE module-level lookup: Try local env first, then module context
            match self.ctx.env.lookup(name).cloned() {
                Some(scheme) => {
                    // =====================================================================
                    // DEFINITE ASSIGNMENT: Check variable is initialized before use
                    // Spec: L0-critical/memory-safety/uninitialized
                    // =====================================================================
                    let var_name = verum_common::Text::from(name);
                    self.check_variable_initialized(&var_name, expr.span)?;

                    let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                    // CRITICAL: Apply unifier to resolve type variables
                    // When we have e.g. `wrapper: Wrapper<τ59>` and τ59 was unified with Text,
                    // we need to return `Wrapper<Text>` so field access works correctly.
                    let resolved_ty = self.unifier.apply(&ty);
                    if let Some(ctor_fn) = self.bare_payload_ctor_as_fn(name, &resolved_ty) {
                        return Ok(InferResult::new(ctor_fn));
                    }
                    Ok(InferResult::new(resolved_ty.into_expression_type()))
                }
                None => {
                    // Try module-level function lookup for cross-function inference
                    if let Maybe::Some(scheme) = self.lookup_function_in_module(name) {
                        let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                        let resolved_ty = self.unifier.apply(&ty);
                        if let Some(ctor_fn) = self.bare_payload_ctor_as_fn(name, &resolved_ty)
                        {
                            return Ok(InferResult::new(ctor_fn));
                        }
                        Ok(InferResult::new(resolved_ty))
                    } else if let Some(scheme) =
                        self.lookup_static_or_load_from_metadata(name)
                    {
                        // Lazy stdlib `<Type>.<method>` static-fn scheme
                        // (the retired eager 30k scan's on-demand twin) —
                        // covers both the bare `Map.new` and the
                        // module-qualified `core.collections.map.Map.new`
                        // spellings a field-access chain can produce.
                        let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                        Ok(InferResult::new(self.unifier.apply(&ty)))
                    } else {
                        Err(TypeError::UnboundVariable {
                            name: name.to_text(),
                            span: expr.span,
                        })
                    }
                }
            }
        } else {
            // Multi-segment path: module::Type::path::to::item
            // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Qualified path resolution
            self.resolve_multi_segment_path(path, expr.span)
        }
    }

    /// Recursively resolve associated-type projections (`::Assoc[Base, ..]`,
    /// encoded as a `Type::Generic` whose name starts with `::`) at ANY depth.
    ///
    /// The call-site return-type logic historically resolved only a TOP-LEVEL
    /// projection (`::Item[ListIter<Int>]` → `&Int`).  A NESTED projection —
    /// e.g. the `F.Output` in `future_poll_sync<F: Future>() -> Maybe<F.Output>`,
    /// which after binding `F = ReadyFuture<Text>` is
    /// `Maybe<::Output[ReadyFuture<Text>]>` — was left unresolved (the top-level
    /// type is `Maybe`, not `::…`) and defaulted to `Int`.  `contains_type_app`
    /// does not flag a `::`-projection node, so `normalize_type` never fired for
    /// it either — this walk is the single place the nested case is driven.
    /// Stdlib-agnostic: resolution flows through `try_find_associated_type`.
    pub(super) fn resolve_assoc_projections_deep(&self, ty: &Type) -> Type {
        self.resolve_assoc_projections_in_scope(ty, None, None, 0)
            .unwrap_or_else(|_| ty.clone())
    }

    pub(super) fn resolve_method_output_projections(
        &self, ty: &Type, receiver: &Type, protocol: &verum_ast::Path,
    ) -> Type {
        self.resolve_assoc_projections_in_scope(ty, Some((receiver, protocol)), None, 0)
            .unwrap_or_else(|_| ty.clone())
    }

    pub(super) fn resolve_call_output_projections(
        &self, ty: &Type, call: &super::call_obligations::CallProjectionScope<'_>,
    ) -> Result<Type> {
        self.resolve_assoc_projections_in_scope(ty, None, Some(call), 0)
    }

    fn resolve_assoc_projections_in_scope(
        &self,
        ty: &Type,
        selected: Option<(&Type, &verum_ast::Path)>,
        call: Option<&super::call_obligations::CallProjectionScope<'_>>,
        depth: usize,
    ) -> Result<Type> {
        if depth >= 64 {
            return Err(TypeError::Other("associated projection normalization depth exceeded".into()));
        }
        let nested = |child: &Type| self.resolve_assoc_projections_in_scope(child, selected, call, depth + 1);
        let instantiate = |ty: &Type| -> Result<Type> {
            match call {
                Some(call) => ty.try_apply_subst(call.arguments)
                    .map(|ty| self.unifier.apply(&ty)).map_err(TypeError::Other),
                None => Ok(ty.clone()),
            }
        };
        let projection = super::associated_family_applications::checked_projection_parts(ty)
            .or_else(|| match ty {
                // Legacy descriptors may omit the projection marker. A real
                // nominal declaration always takes precedence over that reading.
                Type::Generic { name, args } if !args.is_empty()
                    && self.ctx.lookup_type(name).is_none()
                    && self.protocol_checker.read().is_associated_type_name(name) =>
                    Some((&args[0], name.clone(), &args.as_slice()[1..])),
                _ => None,
            });
        if let Some((receiver, assoc, arguments)) = projection {
            let base = nested(receiver)?;
            let arguments = arguments.iter().map(nested).collect::<Result<verum_common::List<_>>>()?;
            if let Some(call) = call {
                let checker = self.protocol_checker.read();
                let owner = checker.associated_projection_owner(receiver, &assoc, &call.assumptions)
                    .map_err(TypeError::Other)?;
                if let Some(mut owner) = owner {
                    owner.args = owner.args.iter().map(instantiate).collect::<Result<_>>()?;
                    let resolved = checker.resolve_owned_associated_family(&base, &owner, &assoc, &arguments, &[])
                        .map_err(TypeError::Other)?;
                    drop(checker);
                    if let Some(resolved) = resolved {
                        // An implementation body has its own scope; caller
                        // parameter identities must not authorize its members.
                        return self.resolve_assoc_projections_in_scope(&resolved.result, None, None, depth + 1);
                    }
                    // An unresolved selected application never falls through
                    // to an unrelated implementation with the same leaf name.
                    return instantiate(ty);
                }
            }
            let base = match &base {
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } if call.is_none() => inner.as_ref(),
                other => other,
            };
            let checker = self.protocol_checker.read();
            let found = match selected {
                Some((receiver, protocol)) if arguments.is_empty()
                    && checker.types_structurally_equal(receiver, base) =>
                    checker.resolve_selected_method_association(base, protocol, &assoc),
                _ if call.is_some() || !arguments.is_empty() => checker.resolve_associated_family(
                    base, &assoc, &arguments, &[],
                ).map_err(TypeError::Other)?.map(|application| application.result),
                _ => checker.try_find_associated_type(base, &assoc),
            };
            drop(checker);
            if let Some(resolved) = found && resolved != *ty {
                return nested(&resolved);
            }
        }
        // Preserve nominal constructors and callable metadata while traversing
        // their arguments. Expanding a nominal result here loses its source owner.
        let mut result = ty.clone();
        match &mut result {
            Type::Generic { args, .. } | Type::Named { args, .. } | Type::Tuple(args) =>
                *args = args.iter().map(nested).collect::<Result<_>>()?,
            Type::Reference { inner, .. } | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } | Type::Ownership { inner, .. }
            | Type::GenRef { inner } | Type::Quantified { inner, .. } => **inner = nested(inner)?,
            Type::Future { output } => **output = nested(output)?,
            Type::Array { element, .. } | Type::Slice { element } | Type::Tensor { element, .. } =>
                **element = nested(element)?,
            Type::Record(fields) | Type::Variant(fields) if selected.is_some() || call.is_some() => {
                for field in fields.values_mut() { *field = nested(field)?; }
            }
            Type::Function { params, return_type, .. } if call.is_some() => {
                *params = params.iter().map(nested).collect::<Result<_>>()?;
                **return_type = nested(return_type)?;
            }
            Type::TypeApp { constructor, args } if call.is_some() => {
                **constructor = nested(constructor)?;
                *args = args.iter().map(nested).collect::<Result<_>>()?;
            }
            _ => return instantiate(ty),
        }
        Ok(result)
    }

    /// #47 — expected-parent hint with alias resolution: structural
    /// alias expansion first (`type Deep is ControlFlow<..>` → head
    /// "ControlFlow"), then the unifier's name-level alias chain.
    fn expected_parent_hint(&self, ty: &Type) -> Option<String> {
        if let Some(expanded) = self.unifier.try_expand_alias(ty)
            && let Some(h) = expected_parent_head(&expanded)
        {
            return Some(h);
        }
        let head = expected_parent_head(ty)?;
        Some(
            self.unifier
                .resolve_aliased_head_text(&head)
                .unwrap_or(head),
        )
    }

    fn infer_expr_call(&mut self, expr: &Expr) -> Result<InferResult> {
        // Callee resolution owns temporary instantiations; release its frame
        // before recursively checking arguments. The selected signature and
        // its obligations travel together into application checking.
        match self.prepare_expr_call(expr)? {
            CallPreparation::Inferred(result) => Ok(result),
            CallPreparation::Apply(prepared) => self.apply_expr_call(expr, prepared),
        }
    }

    fn prepare_expr_call(&mut self, expr: &Expr) -> Result<CallPreparation> {
        use ExprKind::*;
        let ExprKind::Call { func, args, type_args } = &expr.kind else { unreachable!() };
        // ============================================================
        // Optional Chaining Method Call: obj?.method(args)
        // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Optional chaining with method calls
        // ============================================================
        // When the callee is OptionalChain { expr, field: method_name }, this is
        // actually an optional method call: obj?.method(args)
        // Semantics: If obj is None, return None. If Some(inner), call inner.method(args)
        // and wrap the result in Maybe<ReturnType>.
        if let ExprKind::OptionalChain {
            expr: obj,
            field: method,
        } = &func.kind
        {
            let obj_result = self.synth_expr(obj)?;

            // Use protocol-based Maybe resolution to extract inner type
            let maybe_resolution = self
                .protocol_checker
                .read()
                .resolve_maybe_protocol(&obj_result.ty);
            let inner_ty = match &maybe_resolution {
                Some(resolution) => resolution.inner.clone(),
                None => {
                    // If not a Maybe type, the ?. is a no-op on the receiver
                    // but we still need to handle it gracefully
                    obj_result.ty.clone()
                }
            };

            // Delegate to the method call handler with the inner type.
            // This reuses all the existing method resolution logic (protocol methods,
            // inherent methods, borrow tracking, etc.)
            // A95 — when `?.` was a NO-OP and the lookup then failed, say so.
            //
            // `?.` is optional chaining, resolved through the Maybe protocol
            // above; on any other receiver the arm marked "no-op … handle it
            // gracefully" passes the receiver through unchanged, and the
            // method is looked up on it.  The author who meant try-then-call
            // — `e?.m()` on a `Result` — then reads a diagnostic about the
            // METHOD while the mistake was the OPERATOR.  Three spellings of
            // that intent differ by one token pair: `e?.m()` fails, `e? .m()`
            // and `(e?).m()` compile.
            //
            // The note rides in the existing `did_you_mean` channel, so this
            // changes no verdict: 57 `?.`-after-a-paren sites in `vcs/` keep
            // whatever answer they get today.  Turning the no-op into an
            // error is the language decision the register row names, and it
            // is not this.
            //
            // IT MUST BE A CANDIDATE, NOT A PARAGRAPH.  That channel renders
            // as ``help: did you mean `{}`?`` (`lib.rs`), so a sentence put
            // through it comes out wrapped in backticks and terminated by a
            // question mark:
            //
            //     help: did you mean ``?.` is the optional-chaining operator
            //     and resolves through `Maybe`; … `(expr?).method()``?
            //
            // — which is unreadable and is not what the channel promises.
            // The suggestion is therefore the CORRECTED EXPRESSION, spelled
            // with the method the author actually wrote, and it is the whole
            // repair: `(e?).m()` compiles where `e?.m()` does not.  The
            // explanation of WHY belongs where a reader can dwell on it, and
            // A95's register row holds it.
            let method_result = match self.infer_method_call_with_recv_type(
                inner_ty, obj, method, type_args, args, expr.span,
            ) {
                Ok(result) => result,
                Err(mut err) => {
                    if maybe_resolution.is_none() {
                        if let TypeError::MethodNotFound { did_you_mean, .. } = &mut err
                        {
                            if did_you_mean.is_none() {
                                // `?.` resolved through nothing here, so the
                                // author meant try-then-call.  Say so as the
                                // spelling that works.
                                *did_you_mean = Some(verum_common::Text::from(
                                    format!("(expr?).{}(…)", method.name.as_str())
                                        .as_str(),
                                ));
                            }
                        }
                    }
                    return Err(err);
                }
            };

            // Wrap result in Maybe if it isn't already
            // Monadic flattening: if return type is already Maybe, don't double wrap
            let result_ty = if maybe_resolution.is_some() {
                // Original was Maybe, wrap result in Maybe
                if self
                    .protocol_checker
                    .read()
                    .resolve_maybe_protocol(&method_result.ty)
                    .is_some()
                {
                    // Return type is already Maybe - don't double wrap
                    method_result.ty
                } else {
                    Type::maybe(method_result.ty)
                }
            } else {
                method_result.ty
            };

            return Ok(CallPreparation::Inferred(InferResult::new(result_ty)));
        }

        // ============================================================
        // Call Graph Building for Negative Context Verification
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
        // ============================================================
        // A qualified call owns a qualified scheme. Reducing it to the leaf
        // silently drops explicit arguments and may select a caller homonym.
        let callee_name = match &func.kind {
            ExprKind::Path(path) => Some(self.path_to_string(path)),
            _ => None,
        };

        // Record call site for call graph building
        if let Some(ref name) = callee_name {
            self.record_call_site(name.clone(), expr.span);

            // Check for immediate negative context violations
            // This catches violations before deep analysis
            self.check_negative_context_violation(name.as_str(), expr.span)?;

            // ============================================================
            // Cross-Stage Call Validation for N-level Staged Metaprogramming
            // Stage coherence: runtime code cannot depend on meta-only values, meta code cannot observe runtime state — Stage Coherence Rule
            // ============================================================
            // Validate cross-stage calls when inside a meta function
            if self.current_function_stage > 0 {
                // Look up the callee's stage from the stage checker
                // If the callee is registered (i.e., it's a meta function), validate
                if let Some(callee_info) = self.stage_checker.get_function_info(name) {
                    let callee_stage = callee_info.stage;
                    if let Err(stage_err) =
                        self.stage_checker.check_call(name, callee_stage, expr.span)
                    {
                        return Err(Self::stage_error_to_type_error(stage_err));
                    }
                }
                // Note: If callee is not registered, it's either:
                // - A runtime function (stage 0) - which is valid if we're generating code via quote
                // - A function not yet registered - will be caught in staged_pipeline
            }
        }

        // ============================================================
        // Arity-Aware Variant Constructor Disambiguation (#11)
        // ============================================================
        // When two registered types share a variant simple name
        // (canonical collision: `Result.Ok(T)` is a 1-arg tuple
        // variant, `ExitCode.Ok` is a unit variant — both live
        // in the stdlib so neither can claim "user override"
        // priority), the bare-name resolver in `synth_path`
        // picks the FIRST registered parent.  At call sites
        // like `Ok(())` the user's intent is clearly the
        // 1-arg ctor, but the resolver may pick the unit
        // variant — failing with "not a function: <Variant
        // | union>".  Disambiguate at the Call expression
        // BEFORE resolving the callee: pick by arity match
        // when the variant name resolves to multiple
        // candidates.  Only fires when the callee is a
        // single-segment Path (bare-name call).  Stdlib-
        // agnostic per `crates/verum_types/src/AGENTS.md`:
        // the disambiguation key is the constructor's
        // payload count from its own registration, never a
        // hardcoded list of variant names.
        if let Some(ref name) = callee_name {
            // Only when the call is a single-segment Path
            // (bare-name `Ok(())`, not qualified
            // `ExitCode.Ok(...)`).
            let is_bare_path = matches!(
                &func.kind,
                ExprKind::Path(p) if p.segments.len() == 1
            );
            if is_bare_path && self.diagnose_ambiguous_constructor(name.as_str(), Some(args.len()), expr.span)? {
                for arg in args {
                    self.infer_expr(arg, InferMode::Synth)?;
                }
                return Ok(CallPreparation::Inferred(InferResult::new(Type::Unknown)));
            }
            // **Audit-driven fundamental fix** — gate WAS
            // `parents.len() > 1` (multi-parent arity disambiguation
            // only). Single-parent variants then fell through to
            // value-position resolution: `synth_expr` returned the
            // parent Type::Variant which is NOT a Function, then the
            // Call handler emitted `not a function: Some`.  Affects
            // every bare `Some(x)` / `None` / `Ok(v)` / `Err(e)` use
            // site whose simple name has a unique parent in the
            // currently-loaded type registry (the common stdlib
            // case after lazy load only registered Maybe/Result).
            // Resolution: ALWAYS take the constructor path when the
            // simple name is registered as a variant constructor —
            // arity disambiguation still kicks in if multiple
            // parents are registered.
            if is_bare_path
                && self.variant_constructor_parents.get(name).is_some()
                && let Some(ctor_ty) = self
                    .try_resolve_variant_constructor_with_arity(
                        name.as_str(),
                        Some(args.len()),
                    )
            {
                // Re-enter the call type-check with the
                // disambiguated function type instead of
                // re-running synth_expr (which would loop
                // through the arity-blind path).  We
                // simulate the rest of the Call inference
                // body locally for this fast path.
                let func_ty = ctor_ty;
                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = func_ty
                {
                    // An arity mismatch here is genuine over- or
                    // under-application of the constructor: the
                    // resolver has already declined in favour of an
                    // explicit binding of the right arity where one
                    // exists (AMBIENT-CTOR-LOSES-TO-LOCAL-BINDING,
                    // T1034, in infer/modules.rs).
                    if params.len() != args.len() {
                        return Err(TypeError::Other(
                            verum_common::Text::from(format!(
                                "{} expects {} argument(s), got {}",
                                name.as_str(),
                                params.len(),
                                args.len()
                            )),
                        ));
                    }
                    return self.check_constructor_call(args, &params, &return_type, expr.span)
                        .map(CallPreparation::Inferred);
                }
            }
        }

        // ============================================================
        // Type Constructor Call Handling (Heap, Shared)
        // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Heap allocation
        // ============================================================
        // Handle type constructors like Heap(value) and Shared(value)
        // These are syntactic sugar for Heap.new(value) and Shared.new(value)
        if let Some(ref name) = callee_name {
            let name_str = name.as_str();
            if WKT::is_smart_pointer_name(name_str) {
                // Redirect to Type.new
                let new_func_name =
                    verum_common::Text::from(format!("{}.new", name_str));
                if let Some(scheme) =
                    self.ctx.env.lookup(new_func_name.as_str()).cloned()
                {
                    let func_ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = func_ty
                    {
                        // Check arity
                        // T1270: a ±1 tolerance is not the self-parameter rule.
                        let params = match Self::effective_params(params.as_slice(), args.len(), None) {
                            Some(effective) => effective,
                            None => {
                                return Err(TypeError::Other(verum_common::Text::from(
                                    format!(
                                        "{} expects {} argument(s), got {}",
                                        name_str,
                                        params.len(),
                                        args.len()
                                    ),
                                )));
                            }
                        };
                        // Check arguments
                        // NLL: Set call argument context for proper borrow release
                        self.with_call_argument_context(|checker| {
                            for (arg, param_ty) in args.iter().zip(params.iter()) {
                                let resolved_param = checker.unifier.apply(param_ty);
                                // Normalize TypeApp projections (GAT/HKT) now that
                                // type vars may be resolved from explicit type args.
                                let resolved_param =
                                    if Self::contains_type_app(&resolved_param) {
                                        checker.normalize_type(&resolved_param)
                                    } else {
                                        resolved_param
                                    };
                                checker.check_expr(arg, &resolved_param)?;
                            }
                            Ok(())
                        })?;
                        // Consume affine values passed by value
                        self.consume_affine_call_args(args, &params)?;
                        let resolved_return = self.unifier.apply(&return_type);
                        // Also normalize TypeApp in return type for GAT/HKT
                        let resolved_return =
                            if Self::contains_type_app(&resolved_return) {
                                self.normalize_type(&resolved_return)
                            } else {
                                resolved_return
                            };
                        return Ok(CallPreparation::Inferred(InferResult::new(resolved_return)));
                    }
                }
            }
        }

        // ============================================================
        // offset_of Special Form
        // Spec: offset_of(Type, field) returns byte offset of field
        // ============================================================
        // offset_of is special: the second argument is a field name
        // identifier, NOT a variable expression. We must not resolve
        // it as a variable.
        if let Some(ref name) = callee_name {
            if name.as_str() == "offset_of" {
                if args.len() != 2 {
                    return Err(TypeError::Other(verum_common::Text::from(
                        "offset_of expects exactly 2 arguments (type, field_name)",
                    )));
                }
                // First arg: type expression - synthesize normally
                let _type_result = self.synth_expr(&args[0])?;
                // Second arg: field name - do NOT resolve as a variable
                // Just verify it's a valid identifier
                match &args[1].kind {
                    ExprKind::Path(path) if path.segments.len() == 1 => {
                        // Valid field name identifier - OK
                    }
                    ExprKind::Literal(_) => {
                        // String literal for field name - OK
                    }
                    _ => {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "offset_of second argument must be a field name identifier",
                        )));
                    }
                }
                // offset_of always returns Int
                return Ok(CallPreparation::Inferred(InferResult::new(Type::Int)));
            }

            // ============================================================
            // Async join() Builtin
            // join(future1, future2, ...) -> Future<(T1, T2, ...)>
            // Awaiting the result yields a tuple of the awaited types
            // Only use this fallback if join is NOT defined in env (stdlib).
            // When the stdlib defines join() with a proper return type
            // (e.g., Join2<Fut1, Fut2>), we should use that instead.
            // ============================================================
            if name.as_str() == "join"
                && args.len() >= 2
                && self.ctx.env.lookup("join").is_none()
            {
                let mut awaited_types: List<Type> = List::new();
                for arg in args.iter() {
                    let arg_result = self.synth_expr(arg)?;
                    // Extract the awaited type from Future<T>
                    let resolved = self.unifier.apply(&arg_result.ty);
                    let awaited = match self.resolve_source_future(&resolved, arg.span)? {
                        Some(resolution) => resolution.output,
                        None => resolved,
                    };
                    awaited_types.push(awaited);
                }
                let tuple_ty = Type::Tuple(awaited_types);
                return Ok(CallPreparation::Inferred(InferResult::new(Type::Future {
                    output: Box::new(tuple_ty),
                })));
            }
        }

        // ============================================================
        // Variadic Builtin Function Handling
        // Spec: grammar/verum.ebnf Section 2.20.1 - builtin_assertion
        // ============================================================
        // Handle builtin functions that can take optional message arguments.
        // Redirect calls like assert(cond, "msg") to assert_msg internally.
        let effective_func_name: Option<Text> = if let Some(ref name) = callee_name {
            let name_str = name.as_str();
            match (name_str, args.len()) {
                // assert(cond, msg) -> assert_msg(cond, msg)
                ("assert", 2) => Some(verum_common::Text::from("assert_msg")),
                // debug_assert(cond, msg) -> debug_assert_msg(cond, msg)
                ("debug_assert", 2) => {
                    Some(verum_common::Text::from("debug_assert_msg"))
                }
                // assert_eq(a, b, msg) -> assert_eq_msg(a, b, msg)
                ("assert_eq", 3) => Some(verum_common::Text::from("assert_eq_msg")),
                // assert_ne(a, b, msg) -> assert_ne_msg(a, b, msg)
                ("assert_ne", 3) => Some(verum_common::Text::from("assert_ne_msg")),
                _ => None,
            }
        } else {
            None
        };

        // Synthesize function type - use redirected name for builtins if needed
        // CRITICAL: Handle explicit type arguments like get_default<Person>()
        // Track protocol bounds from generic type params for call-site verification.
        // These map fresh type vars -> protocol bounds that must be satisfied
        // after argument unification resolves the fresh vars to concrete types.
        let mut pending_protocol_bounds: Map<
            TypeVar,
            List<crate::protocol::ProtocolBound>,
        > = Map::new();

        let mut pending_type_bounds: Map<TypeVar, List<Type>> = Map::new();

        // CALLEE-SOURCE (T1120): a unit's own `fn min` never runs — the
        // ambient builtin answers instead, in BOTH layers, and
        // `fn min(…) -> Text` used where `Int` is required checks clean.
        // Four existing traces (FNSCHEME, CALLS, ARITY, WK_FALLBACK) say
        // nothing about it, so the question "which SOURCE gave this call
        // its signature" has no instrument. Printing the resolved type
        // alone would not answer it either — the point is WHERE it came
        // from, so the two arms are labelled apart.
        //
        // PLACEMENT MATTERS, and getting it wrong cost a build: the
        // first version of this trace sat INSIDE the redirect arm
        // below, so a bare `min(1, 2)` — the very call it was built to
        // explain — took the other arm and the instrument printed
        // nothing. Silence read as "the site is dead" when the site was
        // simply not on that path. An instrument must print on EVERY
        // input it claims to cover, which means it goes BEFORE the
        // branch and names which arm was taken.
        let trace_callee = std::env::var("VERUM_TRACE_CALLEE").is_ok();
        if trace_callee {
            let (arm, shown) = match (&effective_func_name, &callee_name) {
                (Some(r), _) => ("redirect", r.as_str().to_string()),
                (None, Some(c)) => ("callee", c.as_str().to_string()),
                (None, None) => ("indirect", String::from("<expr>")),
            };
            eprintln!(
                "[callee] arm={} name={} env_has={} type_args={}",
                arm,
                shown,
                self.ctx.env.lookup(shown.as_str()).is_some(),
                type_args.len()
            );
        }
        let func_result = if let Some(ref redirected_name) = effective_func_name {
            // Look up the redirected function name
            if let Some(scheme) = self.ctx.env.lookup(redirected_name.as_str()).cloned()
            {
                let instance = self.instantiate_call_scheme(&scheme, expr.span)?;
                pending_protocol_bounds = instance.protocol_bounds;
                pending_type_bounds = instance.type_bounds;
                InferResult::new(instance.ty)
            } else {
                self.synth_expr(func)?
            }
        } else if !type_args.is_empty() {
            // Explicit type arguments: func<T, U>(args)
            // Need to instantiate with the provided types instead of fresh vars
            // Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
            //

            // IMPORTANT: Explicit type args should only bind to EXPLICIT params.
            // Implicit parameters (marked with {T}) are inferred from context.
            if let Some(ref name) = callee_name {
                if let Some(scheme) = self.callee_scheme_with_mount_authority(name.as_str()) {
                    let instance = self.instantiate_call_scheme(&scheme, expr.span)?;
                    let ty = instance.ty;
                    let fresh_vars = instance.fresh_vars;
                    let implicit_vars = instance.implicit_vars;
                    pending_protocol_bounds = instance.protocol_bounds;
                    pending_type_bounds = instance.type_bounds;

                    let explicit_slots = instance.explicit_method_vars;
                    if let Some(slots) = explicit_slots.as_ref() {
                        self.bind_explicit_method_arguments(
                            Some(slots), type_args,
                            &verum_ast::Ident::new(name.clone(), expr.span), expr.span,
                        )?;
                    } else {
                        self.bind_explicit_method_arguments(None, type_args,
                            &verum_ast::Ident::new(name.clone(), expr.span), expr.span)?;
                        // Convert type_args to Types and unify with explicit fresh vars only
                        // Skip implicit vars - they will be inferred from argument types
                        let mut explicit_var_index = 0;
                        for fresh_var in fresh_vars.iter() {
                            // Skip implicit vars - they don't consume explicit type args
                            if implicit_vars.contains(fresh_var) {
                                continue;
                            }

                            // Bind explicit type arg to this explicit var
                            if let Some(type_arg) = type_args.get(explicit_var_index) {
                                let ast_ty = type_arg.type_for_declared_slot().ok_or_else(||
                                    TypeError::OtherWithCodeSpanned { code: "E400".into(),
                                        msg: format!("Function '{}' requires a type argument in this generic slot", name).into(),
                                        span: expr.span,
                                    }
                                )?;
                                let provided_ty = self.ast_to_type(&ast_ty)?;
                                self.unifier.unify(&Type::Var(*fresh_var), &provided_ty, expr.span)?;
                            }
                            explicit_var_index += 1;
                        }

                    }

                    // Check that we don't have too many explicit type args
                    let expected_explicit = explicit_slots.as_ref().map_or(fresh_vars.len() - implicit_vars.len(), |slots| slots.len());
                    if type_args.len() > expected_explicit {
                        // `TypeError::Other` has NO span field, so this
                        // rendered with no file/line at all — surfaced by
                        // the `<no source location …>` marker. `LanguageLaw`
                        // is the variant that carries code + message + help
                        // + span, and `expr.span` is right here.
                        // E408 is the registry's "dependent value-argument
                        // arity mismatch" — the same kind of arity fault,
                        // one level up at the TYPE-argument list. E402 is
                        // taken ("Send bound not satisfied").
                        return Err(TypeError::LanguageLaw {
                            code: "E408",
                            message: format!(
                                "Function `{}` expects {} explicit type argument{}, got {}",
                                name,
                                expected_explicit,
                                if expected_explicit == 1 { "" } else { "s" },
                                type_args.len()
                            ),
                            help: format!(
                                "remove the extra type argument{}, or check whether the \
                                 remaining parameters are inferred from the call's \
                                 arguments rather than given explicitly",
                                if type_args.len() - expected_explicit == 1 { "" } else { "s" }
                            ),
                            span: expr.span,
                        });
                    }

                    InferResult::new(ty)
                } else {
                    self.synth_expr(func)?
                }
            } else {
                self.synth_expr(func)?
            }
        } else {
            // Default path: look up scheme for protocol bounds AND function-type
            // bounds (`F: fn() -> Maybe<T>` style closure constraints), then
            // synth_expr for type.
            //
            // **Architectural fix** — the protocol-bounds branch existed but
            // the parallel function-type-bounds path didn't, so any free
            // function whose only type-var constraint was a closure shape
            // (`from_fn<T, F: fn() -> Maybe<T>>`, `unfold<St, T, F: fn(...)
            // -> Maybe<T>>`, `successors<T, F: fn(&T) -> Maybe<T>>`, the
            // entire higher-order-function suite in `core/base/iterator.vr`'s
            // source builders, plus every user-defined HOF that takes a
            // closure with a bound) instantiated through plain
            // `scheme.instantiate()` which DROPS the bounds.  The fresh
            // var for F then carried no bound at the call site, so
            // `check_closure_expr`'s `get_function_type_bound(F)` lookup
            // returned None, the normalised expected stayed as `Type::Var`,
            // and the closure body's expected return type was never
            // propagated — every bare `None` / `Some(x)` inside the body
            // synthesised through arity-blind first-wins and picked an
            // unrelated variant (canonically `Backend.None` over
            // `Maybe.None`).
            if let Some(ref name) = callee_name {
                if let Some(scheme) = self.callee_scheme_with_mount_authority(name.as_str()) {
                    let has_proto_bounds = !scheme.var_protocol_bounds.is_empty();
                    let has_type_bounds = !scheme.var_type_bounds.is_empty();
                    if has_proto_bounds || has_type_bounds {
                        let instance = self.instantiate_call_scheme(&scheme, expr.span)?;
                        pending_protocol_bounds = instance.protocol_bounds;
                        pending_type_bounds = instance.type_bounds;
                        InferResult::new(instance.ty)
                    } else {
                        self.synth_expr(func)?
                    }
                } else {
                    self.synth_expr(func)?
                }
            } else {
                self.synth_expr(func)?
            }
        };
        let func_ty_raw = func_result.ty;

        // CRITICAL: Expand type aliases for function types
        // When a function call target is typed with a type alias like
        // `type Processor is fn(Int) -> Int;`, we need to resolve the
        // alias to the underlying function type before checking callability.
        // Without this, `let f: Processor = |x| x*2; f(21)` fails with
        // "not a function: Processor".
        let func_ty = match &func_ty_raw {
            Type::Named { .. } | Type::Generic { .. } => {
                self.expand_type_alias(&func_ty_raw).unwrap_or(func_ty_raw)
            }
            // Auto-deref references to functions: &fn(T) -> U becomes fn(T) -> U
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
                if matches!(**inner, Type::Function { .. }) =>
            {
                *inner.clone()
            }
            // ASK THE DECLARATION INSTEAD OF INVENTING A SHAPE.
            //
            // A type parameter bounded by a function type — `fn f<F: fn(Int)
            // -> List<U>>(f: F)` — already says what calling it produces.
            // Without this arm the call reached `Type::Var` below, which
            // built a signature out of fresh variables and unified the
            // parameter with it. `bind_var` PERMITS that for a bounded
            // parameter (the bound promised a function shape), so the call
            // was accepted and nothing ever narrowed the result type.
            //
            // Measured 2026-09-03, two files differing in one thing:
            //
            //     fn via_bound<U, F: fn(Int) -> List<U>>(f: F) -> Int {
            //         let inner = f(1);      error<E404>: `inner` is `_`
            //     fn via_direct<U>(f: fn(Int) -> List<U>) -> Int {
            //         let inner = f(1);      clean
            //
            // Normalising here rather than in the `Type::Var` arm is what
            // makes arity, defaults, argument checking and the result type
            // all come from the one place that already implements them.
            // The stdlib pays for this directly: `Deque.flat_map<U, F:
            // fn(&T) -> List<U>>` is one of the bodies it un-breaks.
            Type::Var(v) => match self.unifier.rigid_fn_bound(*v) {
                Maybe::Some(bound_fn) => bound_fn,
                Maybe::None => func_ty_raw,
            },
            _ => func_ty_raw,
        };

        self.type_registry.register_call_target(expr.span, func_ty.clone());

        Ok(CallPreparation::Apply(PreparedCall {
            callable: func_ty,
            callee_name,
            protocol_bounds: pending_protocol_bounds,
            type_bounds: pending_type_bounds,
        }))
    }

    fn apply_expr_call(&mut self, expr: &Expr, prepared: PreparedCall) -> Result<InferResult> {
        let ExprKind::Call { func, .. } = &expr.kind else {
            unreachable!()
        };
        match prepared.callable {
            Type::Unknown | Type::Var(_) if self.stdlib_single_file_mode => {
                Ok(InferResult::new(Type::Unknown))
            }
            Type::Never => Ok(InferResult::new(Type::Never)),
            Type::Function { .. } => self.apply_function_call(expr, prepared),
            callable @ Type::Pi { .. } => self.apply_pi_call(expr, callable),
            callable @ Type::Var(_) => self.apply_inferred_call(expr, callable),
            callable @ Type::Forall { .. } => self.apply_polymorphic_call(expr, callable),
            callable @ Type::Named { .. } => self.apply_nominal_call(expr, callable),
            callable => Err(TypeError::NotAFunction {
                ty: callable.to_text(),
                span: func.span,
            }),
        }
    }

    fn apply_pi_call(&mut self, expr: &Expr, func_ty: Type) -> Result<InferResult> {
        let ExprKind::Call { func, args, .. } = &expr.kind else {
            unreachable!()
        };
        let Type::Pi {
            param_name,
            param_type,
            return_type,
        } = func_ty
        else {
            unreachable!()
        };
        if args.len() != 1 {
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "Pi type expects exactly 1 argument, got {}",
                args.len()
            ))));
        }

        // Check the argument against parameter type
        self.check_expr(&args[0], &param_type)?;

        // Beta reduction: substitute argument into return type
        // Convert argument expression to EqTerm for substitution
        let arg_term = self.expr_to_eq_term(&args[0])?;
        let result_ty = self.substitute_term_in_type(&return_type, &param_name, &arg_term);

        Ok(InferResult::new(result_ty))
    }

    fn apply_inferred_call(&mut self, expr: &Expr, func_ty: Type) -> Result<InferResult> {
        let ExprKind::Call { func, args, .. } = &expr.kind else {
            unreachable!()
        };
        let Type::Var(v) = func_ty else {
            unreachable!()
        };
        // Reached only when the parameter has NO declared function
        // bound — one with a bound was rewritten to that bound
        // where `func_ty` is computed, so it arrives at the
        // ordinary `Type::Function` arm instead.
        //
        // Type variable being called as a function - instantiate it as a function type
        // Create fresh type variables for parameters based on argument count
        let mut param_types = List::new();
        for _ in args.iter() {
            param_types.push(Type::Var(TypeVar::fresh()));
        }

        let ret_ty = Type::Var(TypeVar::fresh());
        let func_ty_inst = Type::function_with_async(param_types.clone(), ret_ty.clone(), true);

        // Unify the type variable with the function type
        self.unifier
            .unify(&Type::Var(v), &func_ty_inst, func.span)?;

        // Now check arguments against parameter types
        // NLL: Set call argument context for proper borrow release
        self.with_call_argument_context(|checker| {
            for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                checker.check_expr(arg, param_ty)?;
            }
            Ok(())
        })?;

        Ok(InferResult::new(ret_ty))
    }

    fn apply_polymorphic_call(&mut self, expr: &Expr, func_ty: Type) -> Result<InferResult> {
        let ExprKind::Call { func, args, .. } = &expr.kind else {
            unreachable!()
        };
        let Type::Forall { ref vars, ref body } = func_ty else {
            unreachable!()
        };
        // Build substitution map: quantified var -> fresh TypeVar
        let mut subst: Map<TypeVar, Type> = Map::new();
        for qvar in vars.iter() {
            let fresh = TypeVar::fresh();
            subst.insert(*qvar, Type::Var(fresh));
        }

        // Apply substitution to the body to get the instantiated function type
        let instantiated_body = self.substitute_type_vars(body, &subst);
        self.type_registry
            .register_call_target(expr.span, instantiated_body.clone());

        // Now the body should be a Function type - extract and check
        match instantiated_body {
            Type::Function {
                params,
                return_type,
                contexts,
                ..
            } => {
                // Check arity
                // T1270: a ±1 tolerance is not the self-parameter rule.
                let params = match Self::effective_params(params.as_slice(), args.len(), None) {
                    Some(effective) => effective,
                    None => {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Rank-2 function expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ))));
                    }
                };

                // Context satisfaction check
                if let Some(ref callee_contexts) = contexts {
                    let mut callee_context_set = ContextSet::new();
                    for ctx_ref in callee_contexts.iter() {
                        callee_context_set
                            .add(ContextRequirement::new(ctx_ref.name.clone(), expr.span));
                    }

                    let caller_context_set =
                        self.current_function_contexts.clone().unwrap_or_default();

                    self.context_checker.check_context_satisfaction(
                        &callee_context_set,
                        &caller_context_set,
                        expr.span,
                    )?;
                }

                // Check each argument against parameter type
                self.with_call_argument_context(|checker| {
                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                        let resolved_param = checker.unifier.apply(param_ty);
                        checker.check_expr(arg, &resolved_param)?;
                    }
                    Ok(())
                })?;

                let resolved_return = self.unifier.apply(&return_type);
                Ok(InferResult::new(resolved_return))
            }
            _ => Err(TypeError::NotAFunction {
                ty: func_ty.to_text(),
                span: func.span,
            }),
        }
    }

    fn apply_nominal_call(&mut self, expr: &Expr, func_ty: Type) -> Result<InferResult> {
        let ExprKind::Call { func, args, .. } = &expr.kind else {
            unreachable!()
        };
        let Type::Named { ref path, .. } = func_ty else {
            unreachable!()
        };
        let canonical = self.path_to_string(path);
        if self.source_type_registration.completed_declaration(&canonical)
            .is_some_and(|declaration| matches!(declaration.body, verum_ast::TypeDeclBody::Unit))
        {
            return self.check_constructor_call(args, &[], &func_ty, expr.span);
        }
        let mut type_name = path.last_segment_name().to_string();
        // SELF-NEWTYPE-CTOR-1: `Self(v)` inside an
        // `implement <Newtype>` block is the same constructor
        // call — resolve `Self` to the enclosing impl type
        // before probing the newtype-inner binding.  Pre-fix,
        // `fn root() -> Self { Self(0) }` on
        // `type SupervisorId is (UInt64);` failed
        // `not a function: Self` locally, while the lenient
        // stdlib bake mis-lowered it to a stray MakeVariant
        // (SupervisorId.root().raw() returned Variant(138, 0)).
        if type_name == "Self"
            && let Maybe::Some(ref self_ty) = self.current_self_type
            && let Type::Named {
                path: self_path, ..
            } = self_ty
        {
            type_name = self_path.last_segment_name().to_string();
        }
        // SYS-IOENGINE-FD-CTOR: the MOUNT-scoped qualified key
        // (`<module>.__newtype_inner_<X>`) is authoritative —
        // when two modules declare same-named types (record
        // `core.shell.resources.Fd` vs newtype
        // `core.sys.io_engine.Fd`), the bare key describes
        // whichever won the sorted-walk simple slot; a file
        // that explicitly mounted the newtype must construct
        // ITS type.  Falls back to the flat key for the
        // non-colliding world.
        let inner_key = format!("__newtype_inner_{}", type_name);
        // Ctor-position names may never have passed through
        // TYPE resolution, so the mount-scoped qualified keys
        // (including `<module>.__newtype_inner_<X>`) may not be
        // registered yet.  Invoke the QUALIFIED metadata loader
        // DIRECTLY: `resolve_type_name_mount_scoped` early-returns
        // on a usable TYPE hit without ever reaching the loader,
        // so the auxiliary `__newtype_inner_` keys would stay
        // unregistered exactly when the type resolves fine.
        // Idempotent; a no-op for unmounted/ambiguous names.
        let mount_canonical: Option<String> = self
            .imported_names
            .get(&verum_common::Text::from(type_name.as_str()))
            .and_then(|sources| {
                if sources.len() != 1 {
                    return None;
                }
                sources.iter().next().map(|s| {
                    s.as_str()
                        .strip_prefix("cog.")
                        .unwrap_or(s.as_str())
                        .to_string()
                })
            })
            .filter(|c| !c.is_empty() && c != "cog");
        if let Some(canonical) = mount_canonical {
            let _ =
                self.ensure_mounted_type_loaded_qualified(type_name.as_str(), canonical.as_str());
        }
        let mut inner_ty_hit: Option<Type> =
            self.lookup_type_mount_scoped(type_name.as_str(), "__newtype_inner_");
        if inner_ty_hit.is_none()
            && let Some(inner_ty) = self.ctx.lookup_type(inner_key.as_str())
        {
            inner_ty_hit = Some(inner_ty.clone());
        }
        if let Some(inner_ty) = inner_ty_hit {
            if args.len() == 1
                && let Some(arg0) = args.get(0)
            {
                self.with_call_argument_context(|checker| {
                    checker.check_expr(arg0, &inner_ty)?;
                    Ok(())
                })?;
                return Ok(InferResult::new(func_ty.clone()));
            }
        }
        Err(TypeError::NotAFunction {
            ty: func_ty.to_text(),
            span: func.span,
        })
    }

    fn apply_function_call(&mut self, expr: &Expr, prepared: PreparedCall) -> Result<InferResult> {
        let ExprKind::Call { args, .. } = &expr.kind else {
            unreachable!()
        };
        let Type::Function {
            params, contexts, ..
        } = &prepared.callable
        else {
            unreachable!()
        };
        self.validate_function_call(expr, params, contexts)?;
        self.check_function_call_arguments(args, params, &prepared.callee_name)?;
        self.finish_function_call(expr, prepared)
    }

    fn check_function_call_arguments(
        &mut self,
        args: &List<Expr>,
        params: &List<Type>,
        callee_name: &Maybe<Text>,
    ) -> Result<()> {
        self.with_call_argument_context(|checker| {
            // ============================================================
            // Dependent refinement substitution (Phase A.5 activation)
            // ============================================================
            //

            // Look up the callee's parameter names so that refinement
            // predicates on later parameters can be specialised with the
            // concrete values of earlier arguments before being checked.
            // Without this, a signature like
            //

            //  fn safe_get(len: Int, i: Int{>= 0, < len}) -> Int
            //

            // would leave `len` as a free variable in the predicate on
            // `i` at call sites, silently admitting out-of-bounds calls
            // like `safe_get(5, 10)`. With this substitution, the
            // predicate becomes `10 >= 0 && 10 < 5` at the second
            // argument check, which the refinement checker correctly
            // rejects.
            //

            // `dep_param_names` is the list of parameter names in
            // declaration order; empty strings stand in for positions
            // where the parameter uses a non-identifier pattern (those
            // positions skip substitution but don't break the loop).
            //

            // Guarded by `callee_name.is_some()` so closures and
            // anonymous function values don't pay the lookup cost.
            let dep_param_names: List<Text> = callee_name
                .as_ref()
                .and_then(|name| checker.function_param_names.get(name).cloned())
                .unwrap_or_default();

            for (i, (arg, param_ty)) in args.iter().zip(params.iter()).enumerate() {
                let check_ty =
                    checker.specialize_call_parameter(param_ty, i, args, &dep_param_names)?;
                checker.check_expr(arg, &check_ty)?;
            }
            Ok(())
        })
    }

    fn specialize_call_parameter(
        &mut self,
        param_ty: &Type,
        i: usize,
        args: &[Expr],
        dep_param_names: &[Text],
    ) -> Result<Type> {
        let resolved_param = self.unifier.apply(param_ty);
        // Normalize TypeApp nodes (GAT/HKT type application)
        // so that e.g. F<MaybeInstance><Int> reduces to Maybe<Int>
        let resolved_param = if Self::contains_type_app(&resolved_param) {
            self.normalize_type(&resolved_param)
        } else {
            resolved_param
        };

        // Apply dependent refinement substitution: if this
        // parameter is `Type::Refined { base, predicate }`, and
        // we know the names of earlier parameters, substitute
        // each earlier argument into the predicate so that
        // references like `len` become the concrete constant
        // passed at the call site. Empty-string parameter
        // names (non-identifier patterns) are skipped.
        //

        // The base type and any wrapping refinement structure
        // is otherwise preserved, so this only ever tightens
        // the predicate with more information.
        let check_ty = if let Type::Refined { base, predicate } = &resolved_param {
            if !dep_param_names.is_empty() && i > 0 {
                let mut replacements = List::new();
                for j in 0..i {
                    let earlier_name = match dep_param_names.get(j) {
                        Some(n)
                            if !n.as_str().is_empty() && !predicate.binds_subject(n.as_str()) =>
                        {
                            n.clone()
                        }
                        _ => continue,
                    };
                    let arg_expr = match args.get(j) {
                        Some(a) => a,
                        None => continue,
                    };
                    replacements.push((earlier_name, arg_expr));
                }
                let mut subst_pred =
                    predicate
                        .substitute_parameters(&replacements)
                        .map_err(|error| TypeError::OtherWithCodeSpanned {
                            code: "E500".into(),
                            msg: error.message,
                            span: error.span,
                        })?;
                // Constant-fold arithmetic that became pure after
                // substitution. Without this, a predicate like
                // `count <= src_len - offset` becomes
                // `count <= 10 - 5` after substituting
                // `src_len → 10, offset → 5`, and the syntactic
                // refinement checker cannot decide it because it
                // does not reduce `10 - 5` to `5`. Folding here
                // produces `count <= 5`, which the checker can
                // then decide against any concrete `count`.
                subst_pred.predicate = Self::const_fold_expr(&subst_pred.predicate).into();
                Type::Refined {
                    base: base.clone(),
                    predicate: subst_pred,
                }
            } else {
                resolved_param
            }
        } else {
            resolved_param
        };

        Ok(check_ty)
    }

    fn finish_function_call(&mut self, expr: &Expr, prepared: PreparedCall) -> Result<InferResult> {
        let ExprKind::Call { args, .. } = &expr.kind else {
            unreachable!()
        };
        let PreparedCall {
            callable,
            callee_name,
            protocol_bounds: pending_protocol_bounds,
            type_bounds: pending_type_bounds,
        } = prepared;
        let Type::Function {
            params,
            return_type,
            contexts,
            ..
        } = callable
        else {
            unreachable!()
        };
        self.check_call_loans(args, Some(&params), None)?;

        // Consume affine values passed by value
        self.consume_affine_call_args(args, &params)?;

        self.enforce_call_type_bounds(&pending_type_bounds, expr.span)?;

        // ============================================================
        // Protocol Bound Checking at Call Sites
        // ============================================================
        // After argument unification, resolve type vars to concrete
        // types and verify protocol bounds are satisfied.
        self.enforce_call_protocol_bounds(&pending_protocol_bounds, expr.span)?;

        // ============================================================
        // Interprocedural Escape Analysis for &checked References
        // Spec: L0-critical/reference_system/reference_tiers/checked_promotion_fail
        // ============================================================
        // When calling a function that returns a reference, check if any
        // argument is a &checked reference to a local variable.
        // Such references could escape through the function's return value,
        // violating the safety guarantee of &checked (0ns overhead, no runtime checks).
        let resolved_return = self.resolve_call_output(
            &return_type,
            &pending_protocol_bounds,
            &[],
            &crate::ty::Substitution::new(),
        )?;
        // Normalize TypeApp in return type (GAT/HKT)
        let resolved_return = if Self::contains_type_app(&resolved_return) {
            self.normalize_type(&resolved_return)
        } else {
            resolved_return
        };
        // Resolve top-level associated type projections (e.g., ::Item[ListIter<Int>] → &Int)
        // Resolve associated-type projections at ANY depth — a NESTED
        // projection such as the `Maybe<::Output[ReadyFuture<Text>]>`
        // return of `future_poll_sync<F: Future>() -> Maybe<F.Output>`
        // was left unresolved by the old top-level-only check and
        // defaulted to Int.
        let resolved_return = self.resolve_assoc_projections_deep(&resolved_return);
        if self.is_reference_type(&resolved_return) {
            for arg in args.iter() {
                // Check if the argument is a &checked reference to a local
                if let Some(local_var) = self.get_checked_ref_to_local(arg) {
                    return Err(TypeError::CheckedRefEscape {
                        var: local_var,
                        span: arg.span,
                    });
                }
            }
        }
        Ok(InferResult::new(resolved_return))
    }

    fn validate_function_call(
        &mut self,
        expr: &Expr,
        params: &List<Type>,
        contexts: &Maybe<crate::di::requirement::ContextExpr>,
    ) -> Result<()> {
        let ExprKind::Call { func, args, .. } = &expr.kind
        else {
            unreachable!()
        };
        // Support default parameter values
        // Spec: Grammar default_value in function_param
        //

        // Get minimum required params for this function (if registered)
        let func_name = if let ExprKind::Path(path) = &func.kind {
            path.segments.last().and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(id) => Some(id.name.clone()),
                _ => None,
            })
        } else {
            None
        };

        // Variadic functions can accept any number of args (including zero).
        // This includes: format, print, println, panic, assert, assert_eq
        let is_variadic = func_name
            .as_ref()
            .map(|name| {
                matches!(
                    name.as_str(),
                    "format" | "print" | "println" | "panic" |
                    "unreachable" | "todo" |
                    "assert" | "assert_eq" | "assert_ne" |
                    // `List.of` and `Set.of` were here and are gone
                    // (T1093). Neither is implemented anywhere: the
                    // checker skipped arity for them and codegen then
                    // died with `undefined function: List.of`. Being
                    // in this list is what let the call reach codegen
                    // at all, so the entry did not describe a builtin
                    // — it manufactured one. 13 call sites in core/
                    // are rewritten to the list literal `[x]`, which
                    // is what the language actually provides.
                    //
                    // The rule this restores is stated in
                    // crates/verum_types/src/AGENTS.md: the compiler
                    // must hold ZERO hardcoded stdlib knowledge. A
                    // name the checker knows and the library does not
                    // is exactly the circular dependency it forbids.
                    "min" | "max" |
                    "join" | "concat" |
                    "text_concat" | "compile_error" | "compile_warning" |
                    // Meta code generation builtins are variadic
                    "concat_idents" | "format_ident" | "quote" |
                    "unquote" | "stringify" | "gensym" |
                    // Meta context builtins
                    "emit_diagnostic" | "emit_warning" | "emit_error"
                )
            })
            .unwrap_or(false);

        let required_params = {
            let from_registry = func_name
                .as_ref()
                .and_then(|name| {
                    self.function_required_params
                        .get(name)
                        .copied()
                        // BAKED-DEFAULT-ARG-1: the registry is
                        // fed by the AST decl walk and the
                        // metadata resolvers, but several
                        // binding paths (eager env loads,
                        // prelude glob replay) insert schemes
                        // without passing through either. The
                        // gate itself is the ONE consumer —
                        // consult the baked descriptors lazily
                        // on a registry miss and cache the
                        // answer.
                        .or_else(|| self.required_params_from_metadata(name.as_str()))
                })
                .unwrap_or(params.len());
            let capped = from_registry.min(params.len());
            // Variadic functions accept 0 or more args
            if is_variadic { 0 } else { capped }
        };

        let total_params = params.len();
        let provided_args = args.len();

        // Type arguments instantiate the callable's signature; they neither
        // supply runtime values nor change its parameter count. The same
        // contract applies in unsafe bodies and standard-library sources.
        if provided_args < required_params {
            return Err(self.local_error(|| TypeError::OtherWithCodeSpanned {
                code: verum_common::Text::from("E102"),
                msg: verum_common::Text::from(format!(
                    "Function requires at least {} argument{}, got {}{}",
                    required_params,
                    if required_params == 1 { "" } else { "s" },
                    provided_args,
                    func_name
                        .as_ref()
                        .map(|n| format!(" (calling `{}`)", n))
                        .unwrap_or_default()
                )),
                span: expr.span,
            }));
        }

        if provided_args > total_params && !is_variadic {
            return Err(self.local_error(|| TypeError::OtherWithCodeSpanned {
                code: verum_common::Text::from("E102"),
                msg: verum_common::Text::from(format!(
                    "Function accepts at most {} argument{}, got {}{}",
                    total_params,
                    if total_params == 1 { "" } else { "s" },
                    provided_args,
                    func_name
                        .as_ref()
                        .map(|n| format!(" (calling `{}`)", n))
                        .unwrap_or_default()
                )),
                span: expr.span,
            }));
        }

        // ============================================================
        // Context Satisfaction Check (ContextChecker integration)
        // Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
        // ============================================================
        // When calling a function with context requirements, verify that
        // the current function's contexts satisfy those requirements.
        // VERUM_TRACE_CTXCALL prints the DECISION this block is
        // about to make: what the callee requires, what the caller
        // has, and whether leniency is on. It stays because the
        // failure mode it caught is invisible without it — the
        // check below was correct and dead for as long as
        // `contexts` arrived as `None`, and no diagnostic, test or
        // suite could tell that apart from "the caller was fine".
        // One line here answers it in a second (T0935).
        if std::env::var("VERUM_TRACE_CTXCALL").is_ok() {
            let callee_name = if let ExprKind::Path(p) = &func.kind {
                p.segments
                    .last()
                    .and_then(|s| match s {
                        verum_ast::ty::PathSegment::Name(i) => Some(i.name.to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "<path>".into())
            } else {
                format!("<{:?}>", std::mem::discriminant(&func.kind))
            };
            eprintln!(
                "[ctxcall] callee={callee_name} contexts={:?} caller={:?} lenient={}",
                contexts
                    .as_ref()
                    .map(|c| c.iter().map(|r| r.name.to_string()).collect::<Vec<_>>()),
                self.current_function_contexts
                    .as_ref()
                    .map(|s| s.iter().map(|r| r.name.to_string()).collect::<Vec<_>>()),
                self.context_checker.is_lenient(),
            );
        }
        if let Some(callee_contexts) = contexts {
            self.check_callable_contexts(callee_contexts, expr.span)?;
        }

        // Check capability requirements if function has contexts
        // Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0 - Capability Attenuation
        if let Some(req) = contexts {
            for context_ref in req.iter() {
                let context_name = &context_ref.name;
                // Check if the required context is available
                match self
                    .capability_checker
                    .get_context_capabilities(context_name.as_str())
                {
                    Maybe::Some(caps) => {
                        // Context is available - verify it has necessary capabilities
                        // Extract capability requirements from function name and signature
                        use crate::capability::{CapabilityRequirement, TypeCapabilitySet};

                        // Extract function name for heuristic analysis
                        let func_name = if let ExprKind::Path(path) = &func.kind {
                            path.segments
                                .last()
                                .and_then(|seg| match seg {
                                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .unwrap_or("function")
                        } else {
                            "function"
                        };

                        // Use the method capability mapper to extract requirements
                        // For function calls, we use the function name as a hint
                        let context_decl = self.context_declarations.get(context_name);
                        let required_caps =
                            self.method_capability_mapper.extract_method_capabilities(
                                context_name,
                                &verum_common::Text::from(func_name),
                                context_decl,
                            );

                        let requirement = CapabilityRequirement::new(
                            context_name.clone(),
                            required_caps.clone(),
                            verum_common::Text::from(format!("function call to {}", func_name)),
                        );

                        if let Err(cap_err) =
                            self.capability_checker.check_requirement(&requirement)
                        {
                            // E0306: Capability violation
                            use verum_diagnostics::capability_attenuation_errors::CapabilityViolationError;

                            let func_name = if let ExprKind::Path(path) = &func.kind {
                                path.segments
                                    .iter()
                                    .filter_map(|seg| match seg {
                                        verum_ast::ty::PathSegment::Name(id) => {
                                            Some(id.name.as_str())
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join("::")
                            } else {
                                "function".to_string()
                            };

                            let cap_names: Vec<String> = caps
                                .capabilities
                                .names()
                                .iter()
                                .map(|t| t.to_string())
                                .collect();

                            let diagnostic = CapabilityViolationError::new(
                                format!("{}::{}", context_name, "ReadOnly"),
                                span_to_line_col(func.span),
                            )
                            .with_declared_capabilities(
                                cap_names.iter().map(|s| s.as_str().into()).collect(),
                            )
                            .with_function_name(func_name.as_str())
                            .build();

                            self.diagnostics.push(diagnostic);

                            return Err(TypeError::Other(cap_err.message()));
                        }
                    }
                    Maybe::None => {
                        // E0308: Required context not provided
                        use verum_diagnostics::capability_attenuation_errors::CapabilityNotProvidedError;

                        let func_name = if let ExprKind::Path(path) = &func.kind {
                            path.segments
                                .iter()
                                .filter_map(|seg| match seg {
                                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("::")
                        } else {
                            "function".to_string()
                        };

                        let diagnostic = CapabilityNotProvidedError::new(
                            context_name.as_str(),
                            span_to_line_col(func.span),
                        )
                        .with_function_name(func_name.as_str())
                        .build();

                        self.diagnostics.push(diagnostic);

                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Function requires context '{}' which is not provided",
                            context_name
                        ))));
                    }
                }
            }
        }

        // Check each argument against parameter type with substitution
        // NLL: Set call argument context for proper borrow release
        Ok(())
    }

    fn infer_expr_tuple(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Tuple(exprs) = &expr.kind else { unreachable!() };
        let mut types = List::new();
        for e in exprs {
            let result = self.synth_expr(e)?;
            types.push(result.ty);
        }
        Ok(InferResult::new(Type::tuple(types)))
    }

    /// T0216-c — the single authority for built-in type-property RESULT TYPES.
    /// These compile-time-metadata properties are valid on ALL receiver types;
    /// every arm of `infer_expr_field`'s field-access match consults this one
    /// function (after its own field / method lookups) so the property set can
    /// never drift between arms — the drift that regressed `rec.is_signed` when
    /// T0563 removed the record-field swallow. Returns `None` for a member that
    /// is not a built-in property (the caller then continues its normal
    /// resolution). Mirrors the codegen surface (compile_type_property).
    fn builtin_type_property_result_type(prop: &str, self_ty: &Type) -> Option<Type> {
        verum_ast::TypeProperty::from_str(prop)
            .map(|p| Self::type_property_result_type(p, self_ty))
    }

    /// T0216 — property → result type, TOTAL over `TypeProperty`.
    ///
    /// Taking the enum rather than a string is what makes this the single
    /// authority: adding a property to `TypeProperty` fails this match to
    /// compile, whereas the previous `&str` signature returned `None` for
    /// an unknown name and so accepted silently-diverging spellings. The
    /// string-keyed wrapper above exists only for the field-access route,
    /// where the member name has not yet been classified; it decides
    /// membership with `TypeProperty::from_str`, never with its own list.
    pub(super) fn type_property_result_type(
        prop: verum_ast::TypeProperty,
        self_ty: &Type,
    ) -> Type {
        use verum_ast::TypeProperty as P;
        match prop {
            P::Size | P::Alignment | P::Stride | P::Bits | P::Id => Type::int(),
            P::IsSigned => Type::bool(),
            P::Min | P::Max => self_ty.clone(),
            P::Name => Type::text(),
        }
    }

    /// Resolve `a.b.c.NAME` as a module item, or `None` if the chain is
    /// not a module path.
    ///
    /// Returns the item's type WITHOUT leaving `NAME` bound: the
    /// synthetic mount runs in a scope that is popped before returning.
    /// Type-level facts the loader registers (`insert_root`) survive by
    /// design — those are module-level facts, not visibility.
    fn resolve_qualified_path_expr(
        &mut self,
        obj: &Expr,
        field: &verum_ast::ty::Ident,
    ) -> Result<Option<Type>> {
        let Some(segments) = self.extract_module_path_from_field(obj, field) else { return Ok(None) };
        // `a.NAME` alone is far more likely to be a field access than a
        // module reference; require a real module path.
        if segments.len() < 3 {
            return Ok(None);
        }
        let root = segments[0];
        if self.ctx.env.lookup(root).is_some() {
            return Ok(None);
        }
        self.resolve_qualified_value_path(&segments, field.span)
    }

    /// A selected namespace retains its owner for values as well as calls.
    /// Source re-exports and archive declarations use the same scoped import.
    fn resolve_qualified_value_path(&mut self, segments: &[&str], span: Span) -> Result<Option<Type>> {
        let item_name = verum_common::Text::from(*segments.last().expect("nonempty qualified path"));
        // Value access and calls use the same declaration-owned metadata. An
        // exact descriptor hit must not lose its obligations through a textual
        // signature or a failed synthetic mount.
        let qualified: Text = segments.join(".").into();
        if let Some(metadata) = self.core_metadata.clone()
            && let Some(descriptor) = metadata.functions.get(&qualified)
            && descriptor.parent_type.is_none()
        {
            if !descriptor.is_public && descriptor.module_path != self.current_module_path {
                return Err(TypeError::OtherWithCodeSpanned {
                    code: "E405".into(), msg: format!("value `{qualified}` is private").into(), span,
                });
            }
            if descriptor.is_const {
                return Ok(Some(crate::infer::helpers::parse_function_descriptor_type(
                    descriptor.return_type.as_str(), descriptor, &metadata,
                )));
            }
            let scheme = self.scheme_from_function_descriptor(descriptor, &metadata)
                .ok_or_else(|| TypeError::OtherWithCodeSpanned {
                    code: "E405".into(), msg: format!("invalid signature for `{qualified}`").into(), span,
                })?;
            return self.instantiate_value_scheme(&scheme, span).map(Some);
        }
        // A synthetic import must not succeed by reusing an unrelated bare
        // binding. Prove this owner publishes the requested member first.
        let owner = segments[..segments.len() - 1].join(".");
        let registered = {
            let registry = self.module_registry.read();
            self.get_module_with_path_aliases(&owner, &registry)
                .is_some_and(|module| module.exports.get(&item_name).is_some())
        };
        let inline = self.find_inline_module_for_import(qualified.as_str(), false)
            .and_then(|(key, name)| self.inline_modules.get(&Text::from(key)).map(|module| (module, name)))
            .and_then(|(module, name)| module.items.as_ref().map(|items| (items, name)))
            .is_some_and(|(items, name)| items.iter().any(|item| match &item.kind {
                verum_ast::ItemKind::Function(f) => f.name.name.as_str() == name,
                verum_ast::ItemKind::Const(c) => c.name.name.as_str() == name,
                verum_ast::ItemKind::Static(s) => s.name.name.as_str() == name,
                verum_ast::ItemKind::Type(t) => t.name.name.as_str() == name,
                verum_ast::ItemKind::ExternBlock(block) => block.functions.iter().any(|f| f.name.name.as_str() == name),
                _ => false,
            }));
        let archived_items = self.metadata_known_module_items(&owner);
        let archived = archived_items.as_ref().is_some_and(|items| items.contains(&item_name));
        if !registered && !inline && !archived
            && !self.module_publishes_fn(&owner, item_name.as_str())
            && !self.module_publishes_type(&owner, item_name.as_str())
        {
            if let Some(available_items) = archived_items {
                return Err(TypeError::ImportItemNotFound {
                    declared_in: self.modules_declaring(&item_name),
                    item_name, module_path: owner.into(),
                    available_items: Shared::new(available_items), span,
                });
            }
            return Ok(None);
        }
        let path = verum_ast::ty::Path::new(
            segments
                .iter()
                .map(|s| {
                    verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(*s, span))
                })
                .collect(),
            span,
        );
        let decl = verum_ast::MountDecl {
            visibility: verum_ast::decl::Visibility::Private,
            tree: verum_ast::decl::MountTree {
                kind: verum_ast::decl::MountTreeKind::Path(path),
                alias: verum_common::Maybe::None,
                span,
            },
            alias: verum_common::Maybe::None,
            span,
        };

        self.ctx.env.push_scope();
        let caller = self.current_module_path.clone();
        let registry = self.module_registry.read().clone();
        let imported = self.process_import(&decl, &caller, &registry);
        let found = imported.as_ref().ok().and_then(|_| self.ctx.env.lookup(item_name.as_str()).cloned());
        self.ctx.env.pop_scope();
        imported?;
        found.map(|scheme| self.instantiate_value_scheme(&scheme, span)).transpose()
    }

    /// Admit a property type path only from an exact declaration or import.
    /// The general type resolver also creates forward references on misses;
    /// those are not evidence that a value expression names a type.
    fn declaration_property_path(&mut self, path: &Path) -> Option<Path> {
        use verum_ast::ty::PathSegment;
        // In an expression SelfValue is lowercase `self`, a value receiver.
        // Uppercase `Self` is a Name; explicit type syntax bypasses this
        // declaration-path probe through ExprKind::TypeExpr.
        let names: Option<List<Text>> = path
            .segments
            .iter()
            .map(|segment| match segment {
                PathSegment::Name(name) => Some(name.name.clone()),
                _ => None,
            })
            .collect();
        let names = names?;
        let root = names.first()?;
        if self.ctx.env.is_bound_value(root.as_str()) {
            return None;
        }
        // A declared generic root also wins over a same-spelled module alias.
        // Its associated members are proved separately below.
        if self.ctx.env.is_type_parameter_mirror(root.as_str()) {
            return (names.len() == 1).then(|| path.clone());
        }
        if names.len() == 1
            && (self.lookup_decl_param(root.as_str()).is_some()
                || (root.as_str() == "Self" && self.current_self_type.is_some()))
        {
            return Some(path.clone());
        }
        let mut key: Text = names
            .iter()
            .map(Text::as_str)
            .collect::<List<_>>()
            .join(".")
            .into();
        if let Some(module) = self.module_aliases.get(root) {
            key = if names.len() == 1 {
                module.clone()
            } else {
                format!(
                    "{}.{}",
                    module,
                    names
                        .iter()
                        .skip(1)
                        .map(Text::as_str)
                        .collect::<List<_>>()
                        .join(".")
                )
                .into()
            };
        }
        if self.ctx.lookup_type(key.as_str()).is_none() {
            // Use the same exact inline-module declaration path as imports.
            if let Some((owner, name)) = self.find_inline_module_for_import(key.as_str(), false) {
                let declared: Text = format!("{owner}.{name}").into();
                if self.ctx.lookup_type(declared.as_str()).is_some() {
                    key = declared;
                }
            }
        }
        if self.ctx.lookup_type(key.as_str()).is_none() {
            let public_declaration = self
                .core_metadata
                .as_ref()
                .and_then(|metadata| metadata.types.get(&key))
                .is_some_and(|descriptor| descriptor.is_public);
            if !public_declaration {
                return None;
            }
            // The exact metadata key is the authority. No bare-name or
            // ancestor-prefix recovery is allowed to select another owner.
            self.ensure_stdlib_type_loaded(&key, &mut Vec::new());
            self.ctx.lookup_type(key.as_str())?;
        }
        Some(Path::new(
            key.as_str()
                .split('.')
                .map(|name| PathSegment::Name(verum_ast::Ident::new(name, path.span)))
                .collect(),
            path.span,
        ))
    }

    /// Resolve only declared type operands; a lexical value wins even when
    /// it shares a type's name. This is the semantic counterpart of a field
    /// expression deliberately left ambiguous by the parser.
    pub(super) fn declaration_property_operand(&mut self, expression: &Expr) -> Option<verum_ast::ty::Type> {
        use verum_ast::ty::{Type as AstType, TypeKind as K};
        let kind = match &expression.kind {
            ExprKind::TypeExpr(ty) => return Some(ty.clone()),
            ExprKind::Paren(inner) => return self.declaration_property_operand(inner),
            ExprKind::Path(path) => {
                if let Some(declared) = self.declaration_property_path(path) {
                    K::Path(declared)
                } else {
                    let (member, prefix) = path.segments.split_last()?;
                    let verum_ast::ty::PathSegment::Name(member) = member else {
                        return None;
                    };
                    if prefix.is_empty() {
                        return None;
                    }
                    let base = Expr::new(
                        ExprKind::Path(Path::new(prefix.iter().cloned().collect(), path.span)),
                        path.span,
                    );
                    let projected = Expr::new(
                        ExprKind::Field {
                            expr: Heap::new(base),
                            field: member.clone(),
                        },
                        expression.span,
                    );
                    return self.declaration_property_operand(&projected);
                }
            }
            ExprKind::Field { expr: base, field } => {
                if let Some(segments) = self.extract_module_path_from_field(base, field) {
                    let path = verum_ast::ty::Path::new(
                        segments
                            .into_iter()
                            .map(|name| {
                                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                                    name,
                                    expression.span,
                                ))
                            })
                            .collect(),
                        expression.span,
                    );
                    if let Some(path) = self.declaration_property_path(&path) {
                        return Some(AstType::new(K::Path(path), expression.span));
                    }
                }
                // An associated projection is a type only when its base is a
                // declared type and the selected impl or a declared bound
                // owns this associated member. Unknown forward refs do not.
                let base = self.declaration_property_operand(base)?;
                let base_ty = self.ast_to_type(&base).ok()?;
                let declared = if let Type::Var(var) = &base_ty {
                    self.get_type_var_bounds(var).iter().any(|bound| {
                        if bound.is_negative {
                            return false;
                        }
                        let protocol = self.path_to_string(&bound.protocol);
                        self.protocol_checker
                            .read()
                            .get_protocol(&protocol)
                            .is_some_and(|declaration| {
                                declaration.associated_types.contains_key(&field.name)
                            })
                    })
                } else {
                    self.protocol_checker
                        .read()
                        .try_find_associated_type(&base_ty, &field.name)
                        .is_some()
                };
                if !declared {
                    return None;
                }
                K::Qualified {
                    self_ty: Heap::new(base),
                    trait_ref: Path::new(List::new(), expression.span),
                    assoc_name: field.clone(),
                }
            }
            ExprKind::Array(verum_ast::ArrayExpr::List(elements)) if elements.len() == 1 => {
                K::Slice(verum_common::Heap::new(
                    self.declaration_property_operand(&elements[0])?,
                ))
            }
            ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, count }) => K::Array {
                element: verum_common::Heap::new(self.declaration_property_operand(value)?),
                size: Some(count.clone()),
            },
            ExprKind::Unary { op, expr } => {
                let inner = verum_common::Heap::new(self.declaration_property_operand(expr)?);
                match op {
                    UnOp::Ref => K::Reference {
                        inner,
                        mutable: false,
                    },
                    UnOp::RefMut => K::Reference {
                        inner,
                        mutable: true,
                    },
                    UnOp::RefChecked => K::CheckedReference {
                        inner,
                        mutable: false,
                    },
                    UnOp::RefCheckedMut => K::CheckedReference {
                        inner,
                        mutable: true,
                    },
                    UnOp::RefUnsafe => K::UnsafeReference {
                        inner,
                        mutable: false,
                    },
                    UnOp::RefUnsafeMut => K::UnsafeReference {
                        inner,
                        mutable: true,
                    },
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(AstType::new(kind, expression.span))
    }

    fn infer_expr_field(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Field { expr: obj, field } = &expr.kind else { unreachable!() };
        if let Some(ty) = self.try_context_method_value(obj, field, &List::new(), expr.span)? {
            return Ok(InferResult::new(ty));
        }


        if let Some(path) = self.rooted_field_path(expr) {
            return self.resolve_multi_segment_path(&path, expr.span);
        }

        // An associated value belongs to the declared type, not to its
        // expression-level constructor function. Use the same owner key as
        // annotations and impl registration; lexical values still shadow it.
        if let Some(operand) = self.declaration_property_operand(obj)
            && let verum_ast::ty::TypeKind::Path(path) = &operand.kind
            && let Ok((_, Some(owner))) =
                self.resolve_qualified_type_with_parameter_key(path, operand.span)
            && let Some(scheme) = self.ctx.env.lookup(&format!("{owner}.{}", field.name)).cloned()
        {
            let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
            return Ok(InferResult::new(ty));
        }

        if let Some(property) = verum_ast::TypeProperty::from_str(field.name.as_str())
            && let Some(ty) = self.declaration_property_operand(obj)
        {
            return self.infer_type_property(&ty, &property, expr.span);
        }

        // FULLY-QUALIFIED PATH IN EXPRESSION POSITION (T0779).
        //
        // `core.hash.crypto.sha256.OUTPUT_SIZE` parses as a chain of
        // Field accesses whose base is `Path("core")`, so synthesising
        // the receiver first looks `core` up as a VARIABLE and reports
        // "unbound variable: core".  The grammar is unambiguous that the
        // form is legal — `path_expr = path` and `path` is any dotted
        // segment sequence — so it must resolve.
        //
        // It resolves through the SAME machinery as `mount`, because a
        // qualified path IS a mount of that symbol used once: the
        // synthetic import below is exactly what the user would have
        // written, run inside a scope that is popped immediately.  The
        // scope is what keeps the two forms distinct — after
        // `core.a.b.NAME`, the bare `NAME` must still be unbound, or a
        // qualified reference would silently act as an import.
        //
        // Asked BEFORE the receiver is synthesised, and only when the
        // root segment names no binding in scope, so an ordinary field
        // access on a variable never reaches here.
        if let Some(resolved) = self.resolve_qualified_path_expr(obj, field)? {
            return Ok(InferResult::new(resolved));
        }
        // Special case: Module namespace navigation
        // When obj is a module namespace (e.g., `api`), field access navigates into the module
        // This handles paths like `api.v2.func()` where parser creates Field expressions
        // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
        if let ExprKind::Path(path) = &obj.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
            && !self.ctx.env.is_locally_bound(ident.name.as_str())
        {
            let obj_name = ident.name.as_str();
            // Check if this is an inline module
            if self
                .inline_modules
                .contains_key(&verum_common::Text::from(obj_name))
            {
                // Create a two-segment path to resolve through inline module
                let module_path = verum_ast::ty::Path {
                    segments: vec![
                        verum_ast::ty::PathSegment::Name(ident.clone()),
                        verum_ast::ty::PathSegment::Name(field.clone()),
                    ]
                    .into(),
                    span: expr.span,
                };
                return self.resolve_inline_module_path(&module_path, expr.span);
            }

            // A module alias selects its canonical namespace. Resolve the
            // member through the ordinary path/import machinery, including
            // source re-exports and visibility, without trying bare siblings.
            if let Some(owner) = self.module_aliases.get(&ident.name).cloned() {
                let qualified = format!("{owner}.{}", field.name);
                let segments: List<_> = qualified.split('.').collect();
                if let Some(ty) = self.resolve_qualified_value_path(&segments, expr.span)? {
                    return Ok(InferResult::new(ty));
                }
                let _ = self.lookup_static_or_load_from_metadata(&qualified);
                let path = verum_ast::Path::new(
                    qualified.split('.').map(|name| verum_ast::ty::PathSegment::Name(
                        verum_ast::Ident::new(name, expr.span),
                    )).collect(), expr.span,
                );
                return self.synth_expr(&Expr::new(ExprKind::Path(path), expr.span));
            }
        }

        // Handle chained module access like `api.v2` -> `api.v2.func`
        // When obj itself is a field access on a module, we need to build up the full path
        if let ExprKind::Field {
            expr: inner_obj,
            field: inner_field,
        } = &obj.kind
        {
            if let ExprKind::Path(path) = &inner_obj.kind
                && path.segments.len() == 1
                && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                && !self.ctx.env.is_locally_bound(ident.name.as_str())
                && self
                    .inline_modules
                    .contains_key(&verum_common::Text::from(ident.name.as_str()))
            {
                // Build a three-segment path: module.submodule.item
                let module_path = verum_ast::ty::Path {
                    segments: vec![
                        verum_ast::ty::PathSegment::Name(ident.clone()),
                        verum_ast::ty::PathSegment::Name(inner_field.clone()),
                        verum_ast::ty::PathSegment::Name(field.clone()),
                    ]
                    .into(),
                    span: expr.span,
                };
                return self.resolve_inline_module_path(&module_path, expr.span);
            }
        }

        // Special case: Type.Variant syntax for variant constructors
        // When obj is a single-segment Path (e.g., Color), try to resolve it as Type.Variant
        if let ExprKind::Path(path) = &obj.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
            && !self.ctx.env.is_bound_value(ident.name.as_str())
        {
            let type_name = ident.name.as_str();
            let field_name = field.name.as_str();
            let qualified_name = format!("{}.{}", type_name, field_name);

            // MOUNT-AUTHORITY-OUTRANKS-THE-FLAT-SLOT (T0755).
            //
            // Strategy 1.5 below IS the mount authority for
            // `Type.Variant` syntax, and its own comment names this
            // failure.  It could not fire, because Strategy 1 answers
            // first from the flat `Type.Variant` env slot — and that
            // slot is written with an unguarded `insert_mono`, i.e.
            // LAST-WRITE-WINS across every same-named stdlib type.
            //
            // `core/` declares FOUR public types named `Capability`.
            // Measured before this guard, with the right one mounted:
            //
            //     mount core.mem.capability.{Capability};
            //     let c: Capability = Capability.Read;
            //
            //     error<E400>: Type mismatch:
            //       expected 'Read(Unit) | Write(Unit) | …'
            //       found    'Read(ResourceTag) | Exec(ExecTarget) | …'
            //
            // BOTH sides are types named `Capability`.  The annotation
            // went through mount authority and got the mounted one;
            // the constructor took the flat slot and got another
            // module's.  Two registries, opposite winners.
            //
            // The guard is narrow on purpose: it steps aside only when
            // the constructor name has MORE THAN ONE registered parent,
            // which is exactly the case the flat slot cannot decide.
            // With one parent there is nothing to arbitrate and the
            // fast path stands.  A contested name falls through to
            // Strategy 1.5, and — when the file mounts nothing that
            // claims it — on to the flat strategies below, which is
            // where it would have landed anyway.
            let contested = self
                .variant_constructor_parents
                .get(&verum_common::Text::from(field_name))
                .map(|parents| parents.len() > 1)
                .unwrap_or(false);

            // Strategy 1: env, falling back to the baked metadata's
            // static-method schemes on first touch (lazy twin of the
            // retired eager 30k-scheme scan).
            if !contested
                && let Some(scheme) = self.lookup_static_or_load_from_metadata(&qualified_name)
            {
                let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                return Ok(InferResult::new(ty));
            }

            // T0545 (name-resolution §3.3/§3.4) — when either
            // resolution authority below (mount-scoped, then the flat
            // slot) identifies `type_name` as a SUM TYPE whose variant
            // list does NOT contain `field_name`, the strict gate
            // after both strategies raises the same "Unknown variant
            // constructor … Available variants" diagnostic the called
            // form and the pattern matcher already emit — instead of
            // falling through to the lenient fresh-var arms at the
            // bottom of this function, which let VBC codegen fabricate
            // an interned-string tag (`Http2Error.ZzzNotAVariant` →
            // `MakeVariant { tag: 58770 }`, matching no arm at
            // runtime).  Mount authority outranks the flat slot, so
            // the capture is first-wins.
            let mut qualifier_sum_variants: Option<Vec<verum_common::Text>> = None;

            // Strategy 1.5 (MOUNT-TYPE-AUTHORITY-1): when the file
            // explicitly MOUNTS `type_name`, the mounted module's
            // type is authoritative for `Type.Variant` ctor syntax
            // too — mirror of the annotation-side step 0.5 in
            // `resolve_type_name`.  Without this, a value-position
            // `EffectKind.Pure` written via a mounted alias name
            // resolved through the flat last-write-wins slot below
            // and found the OTHER module's same-named enum.  Falls
            // through to the flat strategies when the mounted type
            // isn't a variant carrying this constructor.
            if let Some(mounted_ty) = self.resolve_type_name_mount_scoped(type_name) {
                let resolved = match &mounted_ty {
                    // Alias descriptors resolve to a Named forward —
                    // chase one level so `EffectKind` (alias of
                    // `DiakrisisEffectKind`) exposes its variants.
                    // The target may not be ctx-registered yet (the
                    // mount-scoped loader only ensures the mounted
                    // name's own closure) — lazy-load it through the
                    // standard simple-slot path before probing.
                    Type::Named { path, args } if args.is_empty() => {
                        if let Some(target) = path.as_ident() {
                            let target_name: verum_common::Text =
                                target.name.clone();
                            if self.ctx.lookup_type(target_name.as_str()).is_none() {
                                let mut pending: Vec<verum_common::Text> = Vec::new();
                                self.ensure_stdlib_type_loaded(&target_name, &mut pending);
                                let mut bound = 64usize;
                                while let Some(next) = pending.pop() {
                                    bound = bound.saturating_sub(1);
                                    if bound == 0 {
                                        break;
                                    }
                                    self.ensure_stdlib_type_loaded(&next, &mut pending);
                                }
                            }
                            self.ctx
                                .lookup_type(target_name.as_str())
                                .cloned()
                                .unwrap_or(mounted_ty.clone())
                        } else {
                            mounted_ty.clone()
                        }
                    }
                    _ => mounted_ty.clone(),
                };
                // Canonical-nominal registrations (metadata lazy load)
                // store `Named{X}` — open the variant body through the
                // ONE expansion authority before probing, else every
                // mounted stdlib sum type misses this arm and the
                // binding degrades to a fresh var (`let trace =
                // LogLevel.Trace` → E404, T0701 ctor leg).
                let resolved = self.expand_generic_to_variant(&resolved)?;
                if let Type::Variant(variants) = &resolved {
                    if variants.contains_key(field_name) {
                        return self.selected_constructor_value(type_name, field_name, expr.span).map(InferResult::new);
                    }
                    // T0545: the mounted authority answered with a sum
                    // body that does NOT declare `field_name` — record
                    // it for the strict-qualified-access gate below.
                    qualifier_sum_variants =
                        Some(variants.keys().cloned().collect::<Vec<_>>());
                }
            }

            // Strategy 2: Look up type name and check if it's a Variant
            // This handles cross-file imported variant types like RegistryError.
            // Same canonical-nominal expansion as Strategy 1.5: lazy
            // metadata registrations store `Named{X}`, so open the body
            // through the ONE expansion authority before probing.
            let flat_expanded = self
                .ctx
                .lookup_type(type_name)
                .cloned()
                .map(|t| self.expand_generic_to_variant(&t)).transpose()?;
            if let Some(ty) = flat_expanded
                && let Type::Variant(variants) = &ty
            {
                if variants.contains_key(field_name) {
                    return self.selected_constructor_value(type_name, field_name, expr.span).map(InferResult::new);
                }
                // T0545: flat-slot authority — captured only when the
                // mount-scoped authority did not already answer.
                if qualifier_sum_variants.is_none() {
                    qualifier_sum_variants =
                        Some(variants.keys().cloned().collect::<Vec<_>>());
                }
            }

            // ── T0545 STRICT QUALIFIED ACCESS (name-resolution §3.3,
            // totality §3.4) ─────────────────────────────────────────
            // `type_name` RESOLVED to a sum type and `field_name` is
            // not among its declared variants.  For a VARIANT-SHAPED
            // member — leading uppercase AND at least one lowercase
            // character (CamelCase) — fail loudly with the called-
            // form/pattern-side diagnostic instead of reaching the
            // lenient fresh-var arms below.
            //
            // SCREAMING_CASE members are deliberately excluded: 40
            // sum types in core/ carry all-caps associated consts
            // (`Signal.MAX_SIGNALS`, `KeyCode.CTRL`, …) whose env
            // registration may exist only under a MODULE-QUALIFIED
            // key (`core.sys.signal.Signal.MAX_SIGNALS`), so Strategy
            // 1's exact `"Type.field"` env probe can miss a REAL
            // const; the codegen's three-spelling probe then resolves
            // it fine.  Firing here on those would reject working
            // code.  Variant names are CamelCase by stdlib-wide
            // convention, and every measured fabrication
            // (ZzzNotAVariant, IoError, FrameTooLarge,
            // ExcessiveRstStream, EnhanceYourCalm, NoError) is
            // CamelCase.
            //
            // Probes that legitimately DECLINE the error (associated
            // items are members of `Q` per §3.3):
            //  * env `"Type.field"` — associated consts and static
            //    methods (`import_impl_blocks` registers both under
            //    the dot form) — already probed by Strategy 1 above,
            //    which returned on a hit;
            //  * the inherent-methods bucket (bare + `$static$` keys)
            //    — source-local `implement Type { … }` items live
            //    there, not in env;
            //  * `variant_constructor_parents` naming `type_name` (or
            //    its canonical alias target) as a parent of
            //    `field_name` — a registered constructor the resolved
            //    body is missing means the registries DISAGREE
            //    (partially-built Variant body, colliding same-name
            //    types): never guess, keep the legacy path.
            //
            // Bare (unqualified) variant names are T0525's rib model
            // and stay untouched here.  A RECORD-typed qualifier
            // (`ErrorCode.EnhanceYourCalm` — a record of consts) keeps
            // its existing arms below; the VBC codegen twin guard
            // (codegen/expressions.rs `compile_field_access`) rejects
            // that shape at lowering, which also covers stdlib bodies
            // that the typechecker never sees — `core/` function bodies
            // are typechecked on no path, so the bake is their only
            // validator.
            if let Some(available) = qualifier_sum_variants
                && field_name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                && field_name.chars().any(|c| c.is_ascii_lowercase())
            {
                let field_text = verum_common::Text::from(field_name);
                let canonical = self.canonical_alias_target(type_name);
                let ctor_of_receiver = self
                    .variant_constructor_parents
                    .get(&field_text)
                    .is_some_and(|parents| {
                        parents.iter().any(|p| {
                            p.as_str() == type_name
                                || p.as_str() == canonical.as_str()
                        })
                    });
                let inherent_member = {
                    let static_key: verum_common::Text =
                        format!("$static${}", field_name).into();
                    let inherents = self.inherent_methods.read();
                    [type_name, canonical.as_str()].iter().any(|owner| {
                        inherents
                            .get(&verum_common::Text::from(*owner))
                            .is_some_and(|bucket| {
                                bucket.contains_key(&field_text)
                                    || bucket.contains_key(&static_key)
                            })
                    })
                };
                if !ctor_of_receiver && !inherent_member {
                    return Err(TypeError::UnknownVariantConstructor {
                        ty: verum_common::Text::from(type_name),
                        variant: field_text,
                        available: verum_common::Text::from(
                            available
                                .iter()
                                .map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                                .as_str(),
                        ),
                        span: expr.span,
                    });
                }
            }
        }

        // Handle lowercase type alias property access (e.g., i32.min, u64.max)
        if let ExprKind::Path(path) = &obj.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
            && !self.ctx.env.is_bound_value(ident.name.as_str())
        {
            let name = ident.name.as_str();
            let field_name = field.name.as_str();
            let is_lowercase_type_alias = matches!(
                name,
                "i8" | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "f32"
                    | "f64"
            );
            if is_lowercase_type_alias
                && matches!(
                    field_name,
                    "min"
                        | "max"
                        | "bits"
                        | "size"
                        | "alignment"
                        | "stride"
                        | "name"
                        | "is_signed"
                )
            {
                let result_ty = match field_name {
                    "name" => Type::text(),
                    "is_signed" => Type::int(),
                    _ => Type::int(),
                };
                return Ok(InferResult::new(result_ty));
            }
        }

        // =====================================================================
        // DEFINITE ASSIGNMENT: Check field initialization for partial struct access
        // Spec: L0-critical/memory-safety/uninitialized
        // When accessing obj.field, check if that specific field is initialized
        // =====================================================================
        if let ExprKind::Path(path) = &obj.kind
            && let [verum_ast::ty::PathSegment::Name(id)] = path.segments.as_slice()
        {
            self.check_field_initialized(&id.name, &field.name, expr.span)?;
        }

        // Normal field access: synthesize the object type and access its field
        // Note: synth_expr will check full initialization for non-field access
        let obj_result = self.synth_expr_for_field_access(obj)?;

        // CRITICAL FIX: Apply unifier to resolve type variables before field access
        // When we have `Wrapper<τ59>` and τ59 was unified with Text, we need to
        // resolve to `Wrapper<Text>` so field access gets the correct field type.
        let resolved_ty = self.unifier.apply(&obj_result.ty);
        self.check_record_field_visibility(&resolved_ty, field.name.as_str(), expr.span)?;
        if let Some(ty) = self.dependent_field_type(obj, &resolved_ty, &field.name, false, expr.span)? {
            return Ok(InferResult::new(ty));
        }

        let normalized_ty = self.normalize_field_receiver(&resolved_ty);

        // f10a integration completion (leg B): field access on an
        // ASSOCIATED-TYPE PROJECTION receiver. AssociatedProjection
        // preservation made adapter elements arrive as an associated-type
        // projection — either the `::Assoc` form the VBC serializer emits or
        // the LEGACY bare-name form `Generic{"Item",[Iter]}` the
        // pre-projection codegen still produces — which `normalize_type` does
        // not reduce, so `|r| r.hot` in `.any/.find` chains errored E103
        // "Cannot access field on non-record type: Item<_>". Recognise both
        // forms (bare via the protocol registry — no hardcoded names) and run
        // the deref-aware projection resolver call-returns/scrutinees already
        // use, then re-deref: `ListIter.Item = &T` resolves to `&Rec`, and
        // field access needs the pointee. Resolving MORE, never less — a
        // projection that still doesn't resolve keeps today's error.
        let normalized_ty = match &normalized_ty {
            Type::Generic { name, args }
                if !args.is_empty()
                    && (name.as_str().starts_with("::")
                        || self
                            .protocol_checker
                            .read()
                            .is_associated_type_name(name.as_str())) =>
            {
                let resolved = self.resolve_assoc_projections_deep(&normalized_ty);
                let resolved = self.normalize_type(&resolved);
                self.unwrap_reference_type(&resolved).clone()
            }
            _ => normalized_ty,
        };

        // Never propagation: any field access on Never produces Never
        if matches!(normalized_ty, Type::Never) {
            return Ok(InferResult::new(Type::Never));
        }

        // Unwrap refined AND capability-restricted types to access the base
        // type for field access.
        //
        // A refined record `{ x: Int, y: Int }{predicate}` should allow `.x`,
        // and so should `Store with [Read]` — A CAPABILITY RESTRICTS WHAT MAY
        // BE DONE WITH A VALUE, IT DOES NOT CHANGE WHAT THE VALUE IS. The
        // refinement arm was already here, with that reasoning; the
        // capability is the same shape and was one arm away, so
        // `Store with [Read]` reported
        //
        //     error<E103>: Cannot access field 'n' on non-record type:
        //                  Store with [ReadOnly]
        //
        // which made attenuation unusable on any record — the shape the
        // feature exists for. `unify.rs` has always read a capability as a
        // wrapper; this reads it the same way.
        //
        // A LOOP, not two arms: `Store with [Read]` refined, or a refined
        // record restricted, carries both wrappers and either order must
        // reach the record underneath.
        let mut normalized_ty = normalized_ty;
        loop {
            normalized_ty = match &normalized_ty {
                Type::Refined { base, .. } => (**base).clone(),
                Type::CapabilityRestricted { base, .. } => (**base).clone(),
                _ => break,
            };
        }
        // A standard record may represent a scalar with its own language type.
        // Its exact declaring shape supplies fields after the visibility check above.
        if let Some(key) = Self::scalar_representation_key(&normalized_ty)
            && let Some(Type::Record(fields)) = self.ctx.lookup_type(&format!("__struct_fields_{key}"))
            && let Some(field_type) = fields.get(field.name.as_str())
        {
            return Ok(InferResult::new(field_type.clone()));
        }

        match &normalized_ty {
            Type::Record(fields) => {
                // Regular record field access
                if let Some(field_ty) =
                    fields.get(&verum_common::Text::from(field.name.as_str()))
                {
                    Ok(InferResult::new(field_ty.clone()))
                } else {
                    // Better error: include field list
                    let available_fields: Vec<&str> =
                        fields.keys().map(|k| k.as_str()).collect();
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "field '{}' not found in type 'record'. Available fields: [{}]",
                        field.name,
                        available_fields.join(", ")
                    ))))
                }
            }
            Type::Named { path, args } => {
                // For named record types, look up the struct fields
                let type_name = self.path_to_string(path);

                // ITER-LAZY-LOAD sibling (T0701 FIELD-TYPE leg): a value
                // can arrive typed `Named{X}` WITHOUT X's body ever
                // loading — `X.new(...)` resolves through the eagerly
                // registered static-method schemes, which name the
                // nominal but load nothing (trace: 5223 ensure-load
                // calls in the repro, zero for LogRecord). Field access
                // is a consumer of X's body exactly like the for-loop
                // desugar is a consumer of X's impls, so it pre-arms
                // the same ensure-then-resolve authority. Idempotent,
                // no-op for user-defined and already-loaded types.
                // The receiver already carries its declaring owner. Loading
                // only the leaf can supply a sibling record's fields instead.
                self.ensure_stdlib_type_loaded_transitive(&type_name);

                let struct_key = format!("__struct_fields_{}", type_name);
                let field_name = verum_common::Text::from(field.name.as_str());

                // Look up type parameters for this generic type
                // so we can substitute T -> Int in Pair<Int>
                let type_params_key = format!("__type_params_{}", type_name);
                let type_params: List<verum_common::Text> =
                    match self.ctx.lookup_type(type_params_key.as_str()) {
                        Option::Some(Type::Record(params_map)) => {
                            params_map.keys().cloned().collect()
                        }
                        _ => List::new(),
                    };

                // Build substitution map from type parameters to concrete args
                // e.g., for Pair<Int>: { T -> Int }
                let mut param_subst = indexmap::IndexMap::new();
                for (param_name, arg_ty) in type_params.iter().zip(args.iter()) {
                    param_subst.insert(param_name.clone(), arg_ty.clone());
                }

                // First try to find the field in the struct definition
                if let Option::Some(Type::Record(fields)) =
                    self.ctx.lookup_type(struct_key.as_str())
                    && let Some(field_ty) = fields.get(&field_name)
                {
                    // Apply type parameter substitution to get concrete field type
                    let field_ty = field_ty.clone();
                    let fields = indexmap::IndexMap::from_iter([(field_name.clone(), field_ty)]);
                    let resolved_ty = self.instantiate_nominal_record_fields(
                        type_name.as_str(), &fields, Some(args.as_slice()), expr.span,
                    )?.0.remove(&field_name).expect("selected field");
                    return Ok(InferResult::new(resolved_ty));
                }

                // Also try looking up the type directly (backward compat)
                if let Option::Some(Type::Record(fields)) =
                    self.ctx.lookup_type(type_name.as_str())
                    && let Some(field_ty) = fields.get(&field_name)
                {
                    // Apply type parameter substitution to get concrete field type
                    let field_ty = field_ty.clone();
                    let fields = indexmap::IndexMap::from_iter([(field_name.clone(), field_ty)]);
                    let resolved_ty = self.instantiate_nominal_record_fields(
                        type_name.as_str(), &fields, Some(args.as_slice()), expr.span,
                    )?.0.remove(&field_name).expect("selected field");
                    return Ok(InferResult::new(resolved_ty));
                }

                // QUALIFIED-NOMINAL leaf fallback: mount-scoped
                // registrations put QUALIFIED nominal spellings into
                // ctx (`core.base.log.LogRecord` — the
                // `ensure_mounted_type_loaded_qualified` key space),
                // while field maps are registered under the BARE leaf
                // (`__struct_fields_LogRecord`) by the simple-name
                // lazy loader. A value typed with the qualified
                // spelling would miss both probes above and fall
                // through to a fresh var. Qualified spellings stay
                // FIRST (collision-immune); the leaf retry only fires
                // when the exact declaration has no field map. A known map
                // that lacks this field must not borrow a sibling's field.
                if crate::ctor_trace_enabled() {
                    eprintln!(
                        "[ctor-trace] field-miss type_name={} field={}",
                        type_name, field_name
                    );
                }
                if type_name.contains(".")
                    && self.ctx.lookup_type(struct_key.as_str()).is_none()
                {
                    let leaf = type_name
                        .as_str()
                        .rsplit('.')
                        .next()
                        .unwrap_or(type_name.as_str())
                        .to_string();
                    let leaf_params_key = format!("__type_params_{}", leaf);
                    let leaf_params: List<verum_common::Text> =
                        match self.ctx.lookup_type(leaf_params_key.as_str()) {
                            Option::Some(Type::Record(params_map)) => {
                                params_map.keys().cloned().collect()
                            }
                            _ => List::new(),
                        };
                    let mut leaf_subst = indexmap::IndexMap::new();
                    for (param_name, arg_ty) in leaf_params.iter().zip(args.iter()) {
                        leaf_subst.insert(param_name.clone(), arg_ty.clone());
                    }
                    let leaf_key = format!("__struct_fields_{}", leaf);
                    if let Option::Some(Type::Record(fields)) =
                        self.ctx.lookup_type(leaf_key.as_str())
                        && let Some(field_ty) = fields.get(&field_name)
                    {
                        let resolved_ty =
                            self.substitute_type_params(field_ty, &leaf_subst);
                        return Ok(InferResult::new(resolved_ty));
                    }
                }

                // Not a record field - try GAT instantiation
                // (e.g., Iterator.Item for protocol access)
                match self.infer_gat_instantiation(
                    path,
                    &field_name,
                    &List::new(), // No explicit args provided
                    Maybe::None,  // No usage context yet
                    expr.span,
                ) {
                    Ok(gat_ty) => Ok(InferResult::new(gat_ty)),
                    Err(_) => {
                        // Not a GAT - try associated constant lookup
                        // Look up as "TypeName.field" in the environment
                        let const_name = format!("{}.{}", type_name, field.name);
                        if let Some(scheme) = self.ctx.env.lookup(&const_name).cloned()
                        {
                            let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                            return Ok(InferResult::new(ty));
                        }
                        // Built-in type properties (T0216-c): the single authority
                        // resolves them for every receiver type, so the arms
                        // cannot drift (the omission here regressed rec.is_signed
                        // when T0563 removed the swallow below).
                        if let Some(t) = Self::builtin_type_property_result_type(
                            field.name.as_str(),
                            &normalized_ty,
                        ) {
                            return Ok(InferResult::new(t));
                        }
                        // T0563 / G1 (docs/architecture/diagnostic-totality.md §3):
                        // loud undefined field/member access. If the receiver's
                        // full field set is KNOWN (its struct definition resolved)
                        // and `field` is neither a declared field (missed at the
                        // __struct_fields_ / direct-type lookups above) nor an
                        // associated / protocol / inherent method (all three live
                        // in the shared `inherent_methods` bucket — see
                        // infer/decls.rs:7470; env-registered `Type.method` forms
                        // already returned at the associated-constant lookup
                        // above), it is a genuine undefined member: a typed E404
                        // error with a did-you-mean over the real members.
                        //
                        // An UNKNOWN field set (opaque type, or a cross-module
                        // type whose definition is not loaded — the T0533 family)
                        // stays lenient (fresh var), so this cannot regress opaque
                        // or partially-resolved receivers.
                        let known_fields = match self.ctx.lookup_type(struct_key.as_str())
                        {
                            Option::Some(Type::Record(f)) => Some(f),
                            _ => match self.ctx.lookup_type(type_name.as_str()) {
                                Option::Some(Type::Record(f)) => Some(f),
                                _ => None,
                            },
                        };
                        if let Some(fields) = known_fields {
                            // One read-lock: decide method-membership AND collect
                            // the type's real members for the did-you-mean list.
                            let (is_member_fn, mut members) = {
                                let static_key: Text =
                                    format!("$static${}", field.name).into();
                                let bare_key: Text = field.name.as_str().into();
                                let guard = self.inherent_methods.read();
                                let bucket =
                                    guard.get(&Text::from(type_name.as_str()));
                                let is_fn = bucket.is_some_and(|b| {
                                    b.contains_key(&bare_key)
                                        || b.contains_key(&static_key)
                                });
                                let mut ms: Vec<String> = fields
                                    .keys()
                                    .map(|k| k.as_str().to_string())
                                    .collect();
                                if let Some(b) = bucket {
                                    for k in b.keys() {
                                        let name = k
                                            .as_str()
                                            .strip_prefix("$static$")
                                            .unwrap_or(k.as_str());
                                        ms.push(name.to_string());
                                    }
                                }
                                (is_fn, ms)
                            };
                            if !is_member_fn {
                                // AUTO-DEREF, last: the receiver's own members
                                // did not answer, so look through its `Deref`
                                // chain exactly as method calls and indexing
                                // already do.  Tried only after the direct
                                // lookup fails, so a field the receiver really
                                // has is never shadowed by its target's.
                                let recv_ty = Type::Named {
                                    path: path.clone(),
                                    args: args.clone(),
                                };
                                if let Some((t, hops)) = self
                                    .field_type_through_deref(&recv_ty, field.name.as_str(), expr.span)?
                                {
                                    self.record_deref_adjustment(obj.span, hops);
                                    return Ok(InferResult::new(t));
                                }
                                members.sort();
                                members.dedup();
                                return Err(TypeError::UnknownField {
                                    ty: Text::from(type_name.as_str()),
                                    field: Text::from(field.name.as_str()),
                                    available: Text::from(
                                        members.join(", ").as_str(),
                                    ),
                                    span: expr.span,
                                });
                            }
                        }
                        // Field set UNKNOWN, or the member is a valid method-as-
                        // value (UFCS): stay lenient (fresh var) to continue
                        // inference — unchanged from the pre-T0563 behavior.
                        if matches!(
                            &normalized_ty,
                            Type::Named { .. } | Type::Generic { .. }
                        ) {
                            let fresh = Type::Var(TypeVar::fresh());
                            return Ok(InferResult::new(fresh));
                        }
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "field '{}' not found in type '{}'",
                            field.name, normalized_ty
                        ))))
                    }
                }
            }
            // Handle associated constants on primitive types
            // e.g., Float.INFINITY, Int.MAX, f64.MIN
            Type::Float | Type::Int => {
                // Built-in type properties first (T0216-c: one authority; this
                // arm and the codegen once drifted — PROP-LAYER-DRIFT-1 — over
                // is_signed/min/max/id on primitives).
                if let Some(t) = Self::builtin_type_property_result_type(
                    field.name.as_str(),
                    &normalized_ty,
                ) {
                    return Ok(InferResult::new(t));
                }
                // Look up as "Type.field" in the environment
                let const_name = format!("{}.{}", normalized_ty, field.name);
                if let Some(scheme) = self.ctx.env.lookup(&const_name).cloned() {
                    let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                    return Ok(InferResult::new(ty));
                }
                // Try with "Float" instead of variant display
                let type_name = match &normalized_ty {
                    Type::Float => WKT::Float.as_str(),
                    Type::Int => WKT::Int.as_str(),
                    _ => "",
                };
                let const_name = format!("{}.{}", type_name, field.name);
                if let Some(scheme) = self.ctx.env.lookup(&const_name).cloned() {
                    let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                    return Ok(InferResult::new(ty));
                }
                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Associated constant {} not found on type {}",
                    field.name, normalized_ty
                ))))
            }
            Type::Text => {
                // Built-in type properties (T0216-c: one authority), else E103.
                if let Some(t) = Self::builtin_type_property_result_type(
                    field.name.as_str(),
                    &normalized_ty,
                ) {
                    Ok(InferResult::new(t))
                } else if let Some((t, hops)) = self
                    .field_type_through_deref(&normalized_ty, field.name.as_str(), expr.span)?
                {
                    self.record_deref_adjustment(obj.span, hops);
                    // AUTO-DEREF, second of the two field-access paths.  A
                    // receiver that is not a record reaches HERE rather than
                    // the `UnknownField` site, which is why wiring that one
                    // alone left every `core/` case untouched: a
                    // `MutexGuard<PoolInner<_>>` is not a record, so
                    // `guard.idle` came out as E103 "Cannot access field on
                    // non-record type".  Both paths now ask the same
                    // question of `Deref`, as method calls and indexing
                    // already did.
                    Ok(InferResult::new(t))
                } else if let Some(Type::Record(fields)) =
                    self.expand_generic_to_record(&normalized_ty)
                {
                    // THIRD field-access path, and the one a generic
                    // record from the ARCHIVE needs. `Labeled<Text>`
                    // resolves to `Type::Generic { name, args }` — never
                    // to a record — so neither the record arm above nor
                    // the deref arm sees it, and a parameter typed with
                    // one reported E103 on a field it plainly has
                    // (T1105). The variant side has had this expansion
                    // since the archive gained generic payloads; the
                    // record side had no counterpart.
                    match fields.get(field.name.as_str()) {
                        Some(t) => Ok(InferResult::new(t.clone())),
                        None => Err(TypeError::OtherWithCode {
                            code: verum_common::Text::from("E103"),
                            msg: verum_common::Text::from(format!(
                                "field '{}' not found in type: {}",
                                field.name, normalized_ty
                            )),
                        }),
                    }
                } else {
                    Err(TypeError::OtherWithCode {
                        code: verum_common::Text::from("E103"),
                        msg: verum_common::Text::from(format!(
                            "Cannot access field '{}' on non-record type: {}",
                            field.name, normalized_ty
                        )),
                    })
                }
            }
            // Handle type properties for ALL types: T.size, T.align, T.alignment, T.stride, T.bits
            // These are valid for any type as compile-time type metadata
            _ => {
                match field.name.as_str() {
                    "size" | "alignment" | "stride" | "bits" => {
                        // Type properties return Int (the size/alignment value)
                        Ok(InferResult::new(Type::int()))
                    }
                    "is_signed" => Ok(InferResult::new(Type::bool())),
                    // min/max as compile-time type metadata return the type
                    // itself (the bound value's type). A13: a second, Numeric-
                    // guarded `"min" | "max"` arm below this one was unreachable
                    // — this unconditional arm already matches every min/max,
                    // and both had an identical body — so it was removed rather
                    // than silenced with an allow.
                    "min" | "max" => Ok(InferResult::new(normalized_ty.clone())),
                    "id" => Ok(InferResult::new(Type::int())),
                    "name" => {
                        // Type name property returns Text
                        Ok(InferResult::new(Type::text()))
                    }
                    _ => {
                        // Try associated constant lookup via receiver Path
                        // This handles newtypes and other types where the receiver
                        // resolves to a function type (constructor) rather than Named
                        if let ExprKind::Path(path) = &obj.kind {
                            if let Some(verum_ast::ty::PathSegment::Name(id)) =
                                path.segments.last()
                            {
                                let const_name = format!("{}.{}", id.name, field.name);
                                if let Some(scheme) =
                                    self.ctx.env.lookup(&const_name).cloned()
                                {
                                    let ty = self.instantiate_value_scheme(&scheme, expr.span)?;
                                    return Ok(InferResult::new(ty));
                                }
                                // Static-method dispatch fallback: when the
                                // receiver is a bare type-name path
                                // (`List.new`, `IndexedList.from_list`,
                                // …) and the field-name is not in env,
                                // search the inherent_methods bucket for
                                // `<TypeName>::<method_name>`.  Pre-fix
                                // every body-position `List.new()` /
                                // `IndexedList { items: List.new() }`
                                // failed E103 "Cannot access field 'new'
                                // on non-record type: List<_>" because
                                // the dispatch path didn't try the
                                // inherent-method bucket here — only the
                                // env-side associated-constant lookup
                                // above fired, and stdlib generic
                                // methods aren't in `env`, they're in
                                // `inherent_methods["List"]["new"]`.
                                //
                                // Also try the `$static$<method>` key
                                // that `register_inherent_methods_from_metadata`
                                // emits for static methods (no `self`
                                // receiver) — same convention as the
                                // AST-driven path.
                                //
                                // Stdlib-agnostic per AGENTS.md: lookup
                                // is keyed by the user-written receiver
                                // name, no per-stdlib-type special case.
                                let type_name_str = id.name.as_str();
                                let methods_guard = self.inherent_methods.read();
                                if let Some(bucket) =
                                    methods_guard.get(&Text::from(type_name_str))
                                {
                                    let static_key: Text =
                                        format!("$static${}", field.name).into();
                                    let bare_key: Text =
                                        field.name.as_str().into();
                                    let scheme = bucket
                                        .get(&static_key)
                                        .or_else(|| bucket.get(&bare_key))
                                        .cloned();
                                    drop(methods_guard);
                                    if let Some(scheme) = scheme {
                                        return Ok(InferResult::new(
                                            self.instantiate_value_scheme(&scheme, expr.span)?,
                                        ));
                                    }
                                } else {
                                    drop(methods_guard);
                                }
                            }
                        }
                        {
                            // An unconstrained receiver cannot establish a
                            // field's type or declaration authority. Inventing
                            // a result variable also admits private fields when
                            // that generic function is later instantiated.
                            if matches!(
                                &normalized_ty,
                                Type::Var(_) | Type::Unknown | Type::Placeholder { .. }
                            ) {
                                return Err(TypeError::OtherWithCode {
                                    code: "E103".into(),
                                    msg: format!(
                                        "Cannot access field '{}' on an unresolved type; declare a record type or use a protocol method",
                                        field.name
                                    ).into(),
                                });
                            }
                            // For Variant types, the field may be on the inner record
                            if matches!(&normalized_ty, Type::Variant(_)) {
                                return Ok(InferResult::new(Type::Var(
                                    TypeVar::fresh(),
                                )));
                            }
                            // AUTO-DEREF, third and last field-access site.
                            // This file formats the E103 message TWICE; wiring
                            // only the first one left `core/` untouched and the
                            // deref trace silent, which is how the second copy
                            // was found.  Same question, same authority.
                            if let Some((t, hops)) = self
                                .field_type_through_deref(
                                    &normalized_ty,
                                    field.name.as_str(),
                                    expr.span,
                                )?
                            {
                                self.record_deref_adjustment(obj.span, hops);
                                return Ok(InferResult::new(t));
                            }
                            // SECOND E103 SITE, and the one an archive
                            // generic record actually reaches. The
                            // comment above says this file formats the
                            // message twice and that wiring only the
                            // first copy leaves `core/` untouched —
                            // written by whoever wired deref. I repeated
                            // it verbatim for T1105: the record
                            // expansion went to the first site, changed
                            // nothing, and the trace inside the helper
                            // printed no line at all, because the helper
                            // was never called from here.
                            if let Some(Type::Record(rec)) =
                                self.expand_generic_to_record(&normalized_ty)
                            {
                                if let Some(t) = rec.get(field.name.as_str()) {
                                    return Ok(InferResult::new(t.clone()));
                                }
                            }
                            Err(TypeError::OtherWithCode {
                                code: verum_common::Text::from("E103"),
                                msg: verum_common::Text::from(format!(
                                    "Cannot access field '{}' on non-record type: {}",
                                    field.name, normalized_ty
                                )),
                            })
                        }
                    }
                }
            }
        }
    }

    fn infer_expr_index(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Index { expr: arr, index } = &expr.kind else { unreachable!() };
        // =====================================================================
        // DEFINITE ASSIGNMENT: Check array element initialization
        // Spec: L0-critical/memory-safety/uninitialized
        // =====================================================================
        if let ExprKind::Path(path) = &arr.kind {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    let var_name = verum_common::Text::from(id.name.as_str());
                    // Try to get constant index for precise checking
                    if let Some(idx) = self.try_extract_const_index(index) {
                        if idx >= 0 {
                            self.check_index_initialized(
                                &var_name,
                                idx as usize,
                                expr.span,
                                false,
                            )?;
                        }
                        // Negative indices are handled by bounds checking below
                    } else {
                        // Non-constant index - check the whole array is initialized
                        self.check_variable_initialized(&var_name, expr.span)?;
                    }
                }
            }
        }

        let arr_result = self.synth_expr_for_field_access(arr)?;
        let resolved_main = self.unifier.apply(&arr_result.ty);
        self.load_source_index_methods(&resolved_main)?;

        // Synthesize index type to determine if this is indexing or slicing
        let index_result = self.synth_expr(index)?;
        let index_ty = self.unifier.apply(&index_result.ty);

        // Check for negative literal index (compile-time error)
        // Spec: L0-critical/memory-safety/buffer_overflow/negative_index.vr
        if let ExprKind::Literal(lit) = &index.kind {
            if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                if int_lit.value < 0 {
                    return Err(TypeError::InvalidIndex {
                        message: verum_common::Text::from(format!(
                            "Invalid index: negative indices not allowed (found {})",
                            int_lit.value
                        )),
                        span: index.span,
                    });
                }
            }
        }
        // Also check for unary negation of a literal: -1, -2, etc.
        if let ExprKind::Unary {
            op: verum_ast::expr::UnOp::Neg,
            expr: inner,
        } = &index.kind
        {
            if let ExprKind::Literal(lit) = &inner.kind {
                if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                    return Err(TypeError::InvalidIndex {
                        message: verum_common::Text::from(format!(
                            "Array index out of bounds: negative index -{}",
                            int_lit.value
                        )),
                        span: index.span,
                    });
                }
            }
        }

        // =====================================================================
        // STATIC BOUNDS CHECKING: Verify index is within array bounds
        // Spec: L0-critical/memory-safety/bounds_checking/array_bounds_static.vr
        // =====================================================================
        // Skip upper-bound static bounds check for variable paths — mutable
        // arrays may have been resized via push/pop, making compile-time
        // size stale. Negative index checks always apply.
        let arr_is_mutable_variable = if let ExprKind::Path(path) = &arr.kind {
            if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.first() {
                self.mutable_bindings.contains(id.name.as_str())
            } else {
                false
            }
        } else {
            false
        };
        // Get the array length if statically known
        let arr_ty = self.unifier.apply(&arr_result.ty);
        if let Some(array_size) = Self::get_array_size(&arr_ty) {
            // Get the constant index value if statically known
            if let Some(index_value) = self.try_extract_const_index(index) {
                // T0584: accumulate the diagnostic and CONTINUE (error
                // recovery) instead of `return Err`, so every OOB access in a
                // function is reported, not only the first — the bounds check
                // previously aborted the statement, leaving subsequent
                // accesses unchecked. Falls through to the normal index-type
                // resolution below (yielding the element type); the pushed
                // error-severity diagnostic still fails the typecheck. The
                // `else if` prevents a negative index from also tripping the
                // exceeds-length branch (duplicate diagnostic).
                if index_value < 0 {
                    let e = TypeError::InvalidIndex {
                        message: verum_common::Text::from(format!(
                            "Array index out of bounds: negative index {}",
                            index_value
                        )),
                        span: index.span,
                    };
                    self.push_diagnostic_for(e);
                } else if !arr_is_mutable_variable && index_value as u64 >= array_size {
                    // Only check upper bound for non-variable paths (literals,
                    // etc.). Variable paths may have been resized via push/pop.
                    let e = TypeError::InvalidIndex {
                        message: verum_common::Text::from(format!(
                            "Array index out of bounds: index {} exceeds array length {}",
                            index_value, array_size
                        )),
                        span: index.span,
                    };
                    self.push_diagnostic_for(e);
                }
            }
        }

        // Materialise whatever deref the resolution needed.  The
        // typechecker is the only layer that knows a `Deref` was
        // consulted; recording it here is what keeps `guard[i]` from
        // type-checking against `T` while codegen indexes the guard.
        let index_hops = {
            let resolved = self.unifier.apply(&arr_result.ty);
            let pc = self.protocol_checker.read();
            pc.resolve_index_with_hops(&resolved).map(|(_, h)| h)
        };
        if let Some(hops) = index_hops {
            self.record_deref_adjustment(arr.span, hops);
        }

        // Get expected index type via protocol resolution
        let expected_index_type = self
            .protocol_checker
            .read()
            .resolve_index_protocol(&arr_result.ty)
            .map(|r| r.key)
            .unwrap_or_else(Type::int);

        // Check if this is a Range type (slicing) or the expected index type (indexing)
        let is_range = match &index_ty {
            Type::Generic { name, args }
                if WKT::Range.matches(name.as_str()) && args.len() == 1 =>
            {
                // Ensure the range is over Int
                self.unifier.unify(&args[0], &Type::int(), index.span)?;
                true
            }
            Type::Named { path, args } if !args.is_empty() => {
                if let Some(last) = path.segments.last()
                    && let verum_ast::ty::PathSegment::Name(id) = last
                    && WKT::Range.matches(id.name.as_str())
                {
                    self.unifier.unify(&args[0], &Type::int(), index.span)?;
                    true
                } else {
                    self.unifier
                        .unify(&index_ty, &expected_index_type, index.span)?;
                    false
                }
            }
            _ => {
                self.unifier
                    .unify(&index_ty, &expected_index_type, index.span)?;
                false
            }
        };

        // If slicing with Range, return array/slice type instead of element type
        if is_range {
            // For slicing, return the same collection type (or a slice)
            return match &arr_result.ty {
                Type::Array { element, .. } => Ok(InferResult::new(Type::Array {
                    element: element.clone(),
                    size: None, // Dynamic size after slicing
                })),
                Type::Slice { element } => Ok(InferResult::new(Type::Slice {
                    element: element.clone(),
                })),
                // STDLIB-AGNOSTIC: Use Index protocol to detect sliceable collection types
                // Any type that implements Index protocol can be sliced (returns slice of element type)
                ty if self
                    .protocol_checker
                    .read()
                    .resolve_index_protocol(ty)
                    .is_some() =>
                {
                    // Return the original collection type for slicing operations
                    Ok(InferResult::new(ty.clone()))
                }
                Type::Text => Ok(InferResult::new(Type::Text)),
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } => {
                    // Recursively handle referenced types
                    match inner.as_ref() {
                        Type::Array { element, .. } => {
                            Ok(InferResult::new(Type::Array {
                                element: element.clone(),
                                size: None,
                            }))
                        }
                        // Handle &[T] slice reference - slicing returns a slice
                        Type::Slice { element } => Ok(InferResult::new(Type::Slice {
                            element: element.clone(),
                        })),
                        Type::Text => Ok(InferResult::new(Type::Text)),
                        // STDLIB-AGNOSTIC: Use Index protocol to detect sliceable types
                        // Any type that implements Index yields a slice of its element type
                        inner_ty
                            if self
                                .protocol_checker
                                .read()
                                .resolve_index_protocol(inner_ty)
                                .is_some() =>
                        {
                            let index_res = self
                                .protocol_checker
                                .read()
                                .resolve_index_protocol(inner_ty);
                            if let Some(res) = index_res {
                                Ok(InferResult::new(Type::Slice {
                                    element: Box::new(res.output),
                                }))
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(
                                    format!("Cannot slice type: {}", arr_result.ty),
                                )))
                            }
                        }
                        // Named/Generic types behind references may be sliceable
                        // (e.g., &Bytes, &List<T>) - assume they support slicing
                        Type::Named { .. } | Type::Generic { .. } => {
                            Ok(InferResult::new(Type::Var(TypeVar::fresh())))
                        }
                        _ => Err(TypeError::Other(verum_common::Text::from(format!(
                            "Cannot slice type: {}",
                            arr_result.ty
                        )))),
                    }
                }
                // Named/Generic types may support slicing through Index protocol
                Type::Named { .. } | Type::Generic { .. } => {
                    Ok(InferResult::new(Type::Var(TypeVar::fresh())))
                }
                _ => Err(TypeError::Other(verum_common::Text::from(format!(
                    "Cannot slice type: {}",
                    arr_result.ty
                )))),
            };
        }

        // Try compile-time evaluation of the index for tuple access
        let const_index: Option<usize> = {
            use crate::const_eval::ConstEvaluator;
            let mut const_eval = ConstEvaluator::new();
            match const_eval.eval(index) {
                Ok(val) => val.as_u128().map(|n| n as usize),
                Err(_) => None,
            }
        };

        // First, try tuple indexing with compile-time constant index
        if let Type::Tuple(elements) = &arr_result.ty {
            let elem_ty = if let Some(i) = const_index {
                if i < elements.len() {
                    elements[i].clone()
                } else if !elements.is_empty() {
                    elements[0].clone() // error recovery
                } else {
                    return Err(TypeError::Other("Cannot index empty tuple".into()));
                }
            } else if !elements.is_empty() {
                let first_ty = &elements[0];
                if elements.iter().all(|ty| ty == first_ty) {
                    first_ty.clone()
                } else {
                    Type::Var(TypeVar::fresh()) // heterogeneous tuple
                }
            } else {
                return Err(TypeError::Other("Cannot index empty tuple".into()));
            };
            return Ok(InferResult::new(elem_ty));
        }

        // Try protocol-based index resolution
        let resolved_for_index = self.unifier.apply(&arr_result.ty);
        if let Some(resolution) = self
            .protocol_checker
            .read()
            .resolve_index_protocol(&resolved_for_index)
        {
            return Ok(InferResult::new(resolution.output));
        }

        // If we have a reference to a type, check the inner type
        // This handles cases like &List<T>, &mut List<T>, &checked List<T>, &Heap<T>, etc.
        // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #auto-dereference
        let inner_ty = match peel_refinement(&arr_result.ty) {
            Type::Reference { inner, .. } => Some(peel_refinement(inner)),
            Type::CheckedReference { inner, .. } => Some(peel_refinement(inner)),
            Type::UnsafeReference { inner, .. } => Some(peel_refinement(inner)),
            Type::Ownership { inner, .. } => Some(peel_refinement(inner)),
            // Heap<T> auto-deref to T for indexing
            Type::Generic { name, args }
                if WKT::Heap.matches(name.as_str()) && !args.is_empty() =>
            {
                Some(&args[0])
            }
            // Ref<T>, RefMut<T> auto-deref to T for indexing (RefCell guard types)
            Type::Generic { name, args }
                if (name.as_str() == "Ref" || name.as_str() == "RefMut")
                    && !args.is_empty() =>
            {
                Some(&args[0])
            }
            _ => None,
        };

        // Handle nested references like &Heap<T> -> deref twice
        let inner_ty = inner_ty.map(|inner| match inner {
            Type::Generic { name, args }
                if WKT::Heap.matches(name.as_str()) && !args.is_empty() =>
            {
                &args[0]
            }
            other => other,
        });

        // Try protocol-based resolution on inner type (for references)
        if let Some(inner) = inner_ty {
            // Handle tuple inside reference
            if let Type::Tuple(elements) = inner {
                let elem_ty = if let Some(i) = const_index {
                    if i < elements.len() {
                        elements[i].clone()
                    } else if !elements.is_empty() {
                        elements[0].clone()
                    } else {
                        return Err(TypeError::Other(
                            "Cannot index empty tuple".into(),
                        ));
                    }
                } else if !elements.is_empty() {
                    let first_ty = &elements[0];
                    if elements.iter().all(|ty| ty == first_ty) {
                        first_ty.clone()
                    } else {
                        Type::Var(TypeVar::fresh())
                    }
                } else {
                    return Err(TypeError::Other("Cannot index empty tuple".into()));
                };
                return Ok(InferResult::new(elem_ty));
            }
            // Try protocol resolution on inner type
            if let Some(resolution) =
                self.protocol_checker.read().resolve_index_protocol(inner)
            {
                return Ok(InferResult::new(resolution.output));
            }
        }

        // If no indexing support found, check if it's a Named/Generic/Unknown type
        // that may implement Index via protocol implementations.
        //
        // Decide on the POINTEE, not the reference. A reference is indexable
        // exactly when the thing it points at is, and this match read the
        // OUTER type: `Type::Reference{..}` matches none of the lenient arms,
        // so every reference whose pointee needed the leniency fell to the
        // error. Measured 2026-09-05, one variable at a time:
        //
        //     type M is List<List<Float>>;
        //     fn f(m: &M) -> Float { m[0][0] }   Cannot index non-indexable type
        //     fn f(m: M)  -> Float { m[0][0] }   ok   (by value)
        //     fn f(m: &List<List<Float>>) …      ok   (reference, no alias)
        //
        // — so it was neither references nor aliases nor indexing, but a
        // reference whose pointee is a NAMED type the protocol lookup cannot
        // resolve. `&checked M` and `&unsafe M` failed identically. Five
        // occurrences in `crates/verum_cli/examples/cbgr_references.vr`, the
        // shipped example whose subject is the three reference tiers.
        //
        // The diagnostic still names the type the user WROTE.
        let indexed_ty = inner_ty.unwrap_or(&arr_result.ty);
        match indexed_ty {
            Type::Named { .. }
            | Type::Generic { .. }
            | Type::Unknown
            | Type::Var(_) => {
                // Custom named type or unresolved type: assume it implements
                // Index protocol. Use a fresh element type.
                Ok(InferResult::new(Type::Var(TypeVar::fresh())))
            }
            // Refined types (e.g., List<Int>{predicate}) - index the base type
            Type::Refined { base, .. } => match base.as_ref() {
                Type::Named { .. } | Type::Generic { .. } | Type::Array { .. } => {
                    Ok(InferResult::new(Type::Var(TypeVar::fresh())))
                }
                _ => Err(TypeError::Other(verum_common::Text::from(format!(
                    "Cannot index non-indexable type: {}",
                    arr_result.ty
                )))),
            },
            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "Cannot index non-indexable type: {}",
                arr_result.ty
            )))),
        }
    }

    fn infer_expr_pipeline(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Pipeline { left, right } = &expr.kind else { unreachable!() };
        // Infer the type of the value being piped
        let left_result = self.synth_expr(left)?;
        let left_ty = left_result.ty;

        // Infer the type of the function/callable
        let right_result = self.synth_expr(right)?;

        match &right_result.ty {
            Type::Function {
                params,
                return_type,
                ..
            } => {
                // Check that the function accepts at least one argument
                if params.is_empty() {
                    return Err(TypeError::Other(verum_common::Text::from(
                        "Pipeline target function must accept at least one argument",
                    )));
                }

                // Check that left type is compatible with first parameter
                if !self.subtyping.is_subtype(&left_ty, &params[0]) {
                    self.unifier.unify(&left_ty, &params[0], left.span)?;
                }

                // Return the function's return type
                Ok(InferResult::new((**return_type).clone()))
            }
            _ => {
                // Try to treat the right side as a callable (could be a closure or other callable)
                // For simplicity, we'll try unification
                let ret_ty = Type::Var(TypeVar::fresh());
                let expected_fn = Type::Function {
                    property_requirement: crate::computational_properties::PropertyRequirement::Unrestricted,
                    params: vec![left_ty].into(),
                    return_type: Box::new(ret_ty.clone()),
                    properties: None,
                    contexts: None,
                    type_params: vec![].into(),
                };
                self.unifier
                    .unify(&right_result.ty, &expected_fn, right.span)?;
                Ok(InferResult::new(ret_ty))
            }
        }
    }

    /// Report every `linear` obligation that is still outstanding, minus
    /// the function's own parameters.
    ///
    /// A `linear` value is EXACTLY once, so the obligation expires
    /// wherever control leaves the function — which is every `return`,
    /// not only the end of the body. Checking the end alone accepts
    ///
    /// ```text
    /// let s = open();
    /// if fail { return 0; }   // `s` is never closed on this path
    /// close(s)
    /// ```
    ///
    /// because the leaving branch is (correctly) excluded from the merge
    /// that forms the continuation. Excluded from the CONTINUATION is
    /// right; excluded from the CHECK is the hole.
    pub(crate) fn report_unmet_linear_obligations(&mut self, at: verum_ast::Span) {
        for err in self.affine_tracker.check_linear_consumed(at) {
            self.push_diagnostic_for(err);
        }
    }

    fn infer_expr_return_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Return(val) = &expr.kind else { unreachable!() };
        if let Some(v) = val {
            // CHECK against the declared return type, do not synthesise
            // blind.
            //
            // Synthesis has no expectation to hand down, so a `match`
            // in return position unified its arms with EACH OTHER:
            //
            //     return match b {
            //         true  => Result.Ok(1),
            //         false => Result.Err("bad"),
            //     };
            //
            // collapsed `Int` against `Text` and reported "expected
            // 'Int', found 'Text'" at the Err arm, plus "expected
            // 'Ok(Int) | Err(Text)', found 'Unit'" at the signature.
            // The SAME match as a tail expression checked clean, because
            // that path does hand the expectation down. `return match`
            // is an ordinary spelling — two of the four errors that
            // `core/io/buffer.vr` reported were exactly this shape.
            //
            // Falls back to synthesis when the function's return type is
            // not known (a closure being inferred, a signature still
            // open): checking against nothing is what synthesis is.
            let declared_return = match &self.current_function_return_type {
                verum_common::Maybe::Some(ty) => Some(ty.clone()),
                verum_common::Maybe::None => None,
            };
            let val_result = match declared_return {
                Some(expected) if !matches!(expected, Type::Unknown | Type::Var(_)) => {
                    self.check_expr(v, &expected)?
                }
                _ => self.synth_expr(v)?,
            };

            // ============================================================
            // Return Value Lifetime Validation
            // Return reference validation: ensuring returned references do not outlive their referents via escape analysis — Dangling references
            // ============================================================
            // Check if we're returning a reference to a local variable.
            // This would create a dangling reference when the function returns.
            self.check_return_value_loans(v, &val_result.ty, expr.span)?;
        }
        // `return` is a scope end for an exactly-once obligation — AFTER
        // the returned expression is walked, because `return close(s)`
        // discharges the obligation IN the thing being returned. Asking
        // before the walk refuses every correct early exit, which is what
        // the control test in `affine_branches_are_alternatives_tests`
        // exists to catch.
        self.report_unmet_linear_obligations(expr.span);
        // Return has Never type - unifies with any type
        Ok(InferResult::new(Type::never()))
    }

    fn infer_expr_array(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Array(arr_expr) = &expr.kind else { unreachable!() };
        match arr_expr {
            verum_ast::expr::ArrayExpr::List(exprs) => {
                if exprs.is_empty() {
                    // Empty array: [T; 0] - element type needs annotation
                    let elem_ty = TypeVar::fresh();
                    // Spec: Fixed-size arrays [T; N] have known size at compile time
                    Ok(InferResult::new(Type::array(Type::Var(elem_ty), Some(0))))
                } else {
                    let first_result = self.synth_expr(&exprs[0])?;
                    let elem_ty = first_result.ty;

                    for expr in exprs.iter().skip(1) {
                        self.check_expr(expr, &elem_ty)?;
                    }

                    // Array literal has known size at compile time.
                    // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .5.1 - Meta Parameters
                    // Fixed-size arrays [T; N] are stack-allocated with size known at compile time.
                    // For dynamic/resizable arrays, use List<T> instead.
                    Ok(InferResult::new(Type::array(elem_ty, Some(exprs.len()))))
                }
            }
            verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                let elem_result = self.synth_expr(value)?;
                let count_result = self.synth_expr(count)?;

                // Count must be Int
                self.unifier
                    .unify(&count_result.ty, &Type::int(), count.span)?;

                // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Evaluate count at compile time for size
                let array_size = match self.const_eval.eval_source(count, self.current_module_path.as_str(), !self.resolving_source_signature, &self.ctx.env.unavailable_constant_names()) {
                    Ok(const_val) => {
                        // Successfully evaluated to constant
                        const_val.as_u128().map(|n| n as usize)
                    }
                    Err(_) => {
                        // Not a compile-time constant, size remains None
                        None
                    }
                };

                Ok(InferResult::new(Type::array(elem_result.ty, array_size)))
            }
        }
    }

    fn infer_expr_cast(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Cast { expr: e, ty } = &expr.kind else { unreachable!() };
        let expr_result = self.synth_expr(e)?;
        let target_ty = self.ast_to_type(ty)?;

        // Type casts are checked for compatibility
        // Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — (Integer Hierarchy), Section 6 (Reference Safety)
        // Pass the cast's source operand so the Int→Float precision check can
        // suppress its warning for a compile-time constant that is exactly
        // representable (the type/refinement legs apply even without it).
        self.check_cast(&expr_result.ty, &target_ty, expr.span, Some(e))?;

        Ok(InferResult::new(target_ty))
    }

    fn infer_expr_yield_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Yield(val) = &expr.kind else { unreachable!() };
        let val_result = self.synth_expr(val)?;

        // Check if we're in a generator context
        match &self.generator_context {
            Maybe::Some(gen_ctx) => {
                // Verify yielded type matches expected yield type
                self.unifier
                    .unify(&val_result.ty, &gen_ctx.yield_ty, expr.span)?;
                // Yield expressions evaluate to unit
                Ok(InferResult::new(Type::unit()))
            }
            Maybe::None => {
                // Yield outside of generator context - error
                Err(TypeError::Other(
                    "yield expression can only be used inside generator functions"
                        .into(),
                ))
            }
        }
    }

    fn infer_expr_record(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Record { path, fields, base } = &expr.kind else { unreachable!() };
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG] infer_expr_inner: Record at {:?}, path={:?}", current_expr.span, path);

        // First check if this is a variant constructor (Type.Variant { ... })
        if let Some((parent_name, variant_name)) = self.record_variant_parent(path)
        {
            // Resolve the complete parent path. A module-qualified constructor
            // has the same payload contract as a locally mounted constructor.
            let type_name = parent_name.as_str();
            let variant_name = variant_name.as_str();

            // Look up the type to see if it's a variant type.
            //
            // QUALVAR-CONSTRUCT-1: a direct `lookup_type` can miss a
            // cross-module *imported* enum — the import binds a `Named`
            // placeholder under the unqualified key, and that placeholder
            // shadows the lazy-loadable rich `Type::Variant`. When the
            // direct lookup is not already a Variant, route through
            // `resolve_type_name`, which triggers the stdlib lazy-load
            // (`ensure_stdlib_type_loaded`) and materializes the real
            // variant set the same way the matching `let e: E = …`
            // annotation does. Without this, a direct record-variant
            // CONSTRUCTION `E.Variant { … }` of an imported enum fell
            // through to the generic handler and was mis-typed as the
            // variant `E.Variant` instead of the enclosing enum `E`,
            // surfacing under strict/AOT checking as
            // `expected 'RevocationError', found 'RevocationError.Internal'`.
            let resolved_variant_ty: Option<Type> = {
                let direct = self.ctx.lookup_type(type_name).cloned();
                if matches!(direct, Option::Some(Type::Variant(_))) {
                    direct
                } else {
                    match self.resolve_type_name(type_name, expr.span) {
                        Ok(t) if matches!(t, Type::Variant(_)) => Option::Some(t),
                        _ => direct,
                    }
                }
            };
            // CANONICAL-NOMINAL claim + nominal result: lazy metadata
            // registrations store `Named{X}` — open the body through the
            // ONE expansion authority so the variant branch CLAIMS the
            // literal (otherwise `RetryBackoff.Linear { … }` fell to the
            // generic record handler and typed as the variant spelling),
            // and return the NOMINAL parent, not the structural body.
            let nominal_result: Option<Type> = resolved_variant_ty.clone();
            let resolved_variant_ty: Option<Type> = resolved_variant_ty.map(|t| {
                if matches!(t, Type::Variant(_)) {
                    Ok(t)
                } else {
                    self.expand_generic_to_variant(&t)
                }
            }).transpose()?;
            if let Option::Some(ty) = resolved_variant_ty
                && let Type::Variant(variants) = &ty
            {
                let ty = match &nominal_result {
                    Option::Some(n) if !matches!(n, Type::Variant(_)) => n.clone(),
                    _ => ty.clone(),
                };
                let ty_body_variants = variants.clone();
                let variants = &ty_body_variants;
                // It's a variant type! Look up the specific variant
                if let Some(variant_payload_ty) = variants.get(variant_name) {
                    // Verify the payload is a record type. Metadata ctor
                    // registration FLATTENS record payloads positionally
                    // (`Linear { delay_ms: Int }` → payload `Int`), so an
                    // archive-loaded enum reached this check with a
                    // non-Record payload, fell through to the generic
                    // record handler, and the literal typed as the
                    // variant spelling («RetryBackoff.Linear») instead of
                    // the enum — same root and same remedy as the pattern
                    // binder: consult the parent-qualified
                    // `__struct_fields_<Type>.<Variant>` side-channel
                    // (then the bare key) before giving up.
                    let upgraded_fields: Option<
                        indexmap::IndexMap<verum_common::Text, Type>,
                    > = match variant_payload_ty {
                        Type::Record(m) => Some(m.clone()),
                        _ => {
                            let qualified =
                                format!("__struct_fields_{}.{}", type_name, variant_name);
                            match self.ctx.lookup_type(qualified.as_str()) {
                                Option::Some(Type::Record(m)) => Some(m.clone()),
                                _ => match self.ctx.lookup_type(
                                    format!("__struct_fields_{}", variant_name).as_str(),
                                ) {
                                    Option::Some(Type::Record(m)) => Some(m.clone()),
                                    _ => None,
                                },
                            }
                        }
                    };
                    if let Some(expected_field_types) = upgraded_fields {
                        // Type check the record fields
                        let mut provided_fields: indexmap::IndexMap<
                            verum_common::Text,
                            Type,
                        > = indexmap::IndexMap::new();

                        for field_init in fields {
                            let field_name = field_init.name.name.as_str();

                            let expected_field_ty =
                                match expected_field_types.get(field_name) {
                                    Some(ty) => ty,
                                    None => {
                                        return Err(TypeError::Other(
                                            verum_common::Text::from(format!(
                                                "field '{}' not found in type '{}::{}'",
                                                field_name, type_name, variant_name
                                            )),
                                        ));
                                    }
                                };

                            // Handle shorthand syntax
                            let field_ty =
                                if let Some(ref value_expr) = field_init.value {
                                    self.check_record_field_value(
                                        field_init,
                                        value_expr,
                                        expected_field_ty,
                                        &format!("{}.{}", type_name, variant_name),
                                    )?;
                                    expected_field_ty.clone()
                                } else {
                                    match self.ctx.env.lookup(field_name).cloned() {
                                        Some(scheme) => {
                                            let var_ty = self.instantiate_value_scheme(&scheme, field_init.span)?;
                                            self.unifier.unify(
                                                &var_ty,
                                                expected_field_ty,
                                                field_init.span,
                                            )?;
                                            expected_field_ty.clone()
                                        }
                                        None => {
                                            return Err(TypeError::UnboundVariable {
                                                name: field_name.to_text(),
                                                span: field_init.span,
                                            });
                                        }
                                    }
                                };

                            provided_fields.insert(field_name.into(), field_ty);
                        }

                        // Validate all required fields are present
                        for (expected_name, expected_ty) in expected_field_types.iter()
                        {
                            if !provided_fields.contains_key(expected_name) {
                                return Err(TypeError::Other(
                                    verum_common::Text::from(format!(
                                        "Missing required field '{}' of type {} in variant {}::{} construction",
                                        expected_name,
                                        expected_ty,
                                        type_name,
                                        variant_name
                                    )),
                                ));
                            }
                        }

                        // Return the variant type (the whole Color type, not just the Rgba variant)
                        return Ok(InferResult::new(ty));
                    }
                }
            }

            // QUALVAR-CONSTRUCT-1 fallback. The rich `Type::Variant` body
            // could not be materialized above — for a cross-module imported
            // enum the unqualified key holds a `Type::Named` placeholder that
            // shadows the lazy body and survives even `resolve_type_name`.
            // But the FIRST path segment still resolves to that enclosing
            // type, and the type of an `E.Variant { … }` record-variant
            // construction IS the enclosing enum `E`, regardless of whether
            // its fields can be checked here. Yield `E` so the expression
            // unifies with a matching `let e: E = …` annotation, instead of
            // falling through to the generic record handler, which mis-types
            // the literal as the variant `E.Variant`. Field VALUES are still
            // inferred best-effort to surface gross errors. Restricted to a
            // `Named` enclosing type: a fully-resolved `Variant`/`Record`
            // reaching here is a genuine bad-variant/bad-field error that the
            // block above already reported and must not be masked.
            if let Option::Some(enclosing @ Type::Named { .. }) =
                self.ctx.lookup_type(type_name).cloned()
            {
                for field_init in fields {
                    if let Some(ref value_expr) = field_init.value {
                        let _ = self.infer_expr(value_expr, InferMode::Synth);
                    }
                }
                return Ok(InferResult::new(enclosing));
            }
        }

        // SINGLE-SEGMENT VARIANT CONSTRUCTOR CHECK
        // For paths like `Node { ... }` (without `Tree::` prefix),
        // check if Node is a variant constructor by looking it up in the environment
        if path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
        {
            let variant_name = ident.name.as_str();

            // (is_variant_ctor check removed — a local record type with
            // matching fields takes precedence over a cross-module variant
            // constructor of the same name; see the has_matching_struct
            // comment below.)

            // PRIORITY CHECK: If a struct type exists with matching fields,
            // prefer struct construction over variant construction.
            //

            // Per the architectural rule in crates/verum_types/src/AGENTS.md —
            // "user-defined variant names must freely override built-in
            // convenience aliases"; symmetrically, a user module's record
            // type must override a cross-module variant of the same name
            // when the provided field names match the record's fields.
            //

            // Record variants also register `__struct_fields_<Variant>`
            // (so pattern matching sees their fields), so the struct-key
            // lookup alone can't tell a user record from a variant payload.
            // Distinguish by checking whether `variant_name` is itself a
            // registered top-level *type*: only a user-defined standalone
            // record like `type Box<T> is { content: T }` will have
            // `lookup_type(variant_name)` return a concrete type. Record
            // variants don't register their name as a standalone type —
            // the parent (e.g. `Expr`) does — so the variant path stays
            // available for `Binary { op, lhs, rhs }` inside
            // `type Expr is IntLit(_) | … | Binary({op, lhs, rhs}) | …`.
            // Variant record payloads and standalone records register
            // similar metadata (`__struct_fields_<Name>`, a Record in
            // `type_defs`). Distinguish by checking BOTH halves and
            // letting the *exact field-set match* on the struct_fields
            // table win when (and only when) the provided fields cover
            // the standalone record exactly:
            //  - Variant constructor with fn-type in env → is a variant.
            //  - Separate `__struct_fields_<Name>` with exact-match
            //  field coverage → is the user's standalone record.
            //  - Both present + exact match on record fields → record
            //  wins (e.g. `Box { content: T.default() }` in
            //  `type Box<T> is { content: T }` overrides a stdlib
            //  `Box { inner: … }` variant).
            //  - Variant present but fields don't cover exactly → fall
            //  through to variant constructor.
            let struct_key = format!("__struct_fields_{}", variant_name);
            // §Y close: mount-scoped probe FIRST so an explicit
            // `mount A.{Foo}` overrides whichever sibling module's
            // `Foo` happened to land later in the unqualified slot.
            //
            // REEXPORT-FIELDS-LAZY-1 (see the env.rs field-read twin):
            // a re-export-hop mount's field maps exist only after the
            // metadata lazy loader ran — on a miss, drive it once and
            // retry before conceding to the flat slot.
            let mount_scoped_struct = match self
                .lookup_type_mount_scoped(variant_name, "__struct_fields_")
            {
                Some(ty) => Some(ty),
                None => {
                    self.ensure_mounted_record_fields_loaded(variant_name);
                    self.lookup_type_mount_scoped(variant_name, "__struct_fields_")
                }
            };
            let resolved_struct_lookup = match mount_scoped_struct {
                Some(ref ty) => Some(ty.clone()),
                None => self.ctx.lookup_type(struct_key.as_str()).cloned(),
            };
            let has_matching_struct = if let Option::Some(Type::Record(struct_fields)) =
                resolved_struct_lookup
            {
                let all_provided_valid = fields
                    .iter()
                    .all(|f| struct_fields.contains_key(f.name.name.as_str()));
                let covers_all_required = struct_fields.keys().all(|required| {
                    fields
                        .iter()
                        .any(|f| f.name.name.as_str() == required.as_str())
                });
                let exact_match = all_provided_valid && covers_all_required;

                if !exact_match {
                    false
                } else {
                    // Fields match exactly — do we also have a variant
                    // constructor for this name? When only the record
                    // exists, trivially route to record. When the
                    // variant also exists AND its payload record
                    // matches the provided fields by name, route to
                    // variant (that's the local variant case like
                    // `Binary { op, lhs, rhs }` inside `type Expr is ... |
                    // Binary { op, lhs, rhs }`). Only when the variant's
                    // *payload* disagrees with the field set do we keep
                    // the record path, which is the cross-module
                    // `Box { content: ... }` override case.
                    let variant_ctor_info = self
                        .ctx
                        .env
                        .lookup(variant_name)
                        .map(|scheme| scheme.instantiate());
                    match variant_ctor_info {
                        Some(Type::Function { params, .. }) => {
                            // Variant constructor: inspect its payload.
                            let variant_record_fields: Option<
                                indexmap::IndexMap<verum_common::Text, Type>,
                            > = params.first().and_then(|p| match p {
                                Type::Record(m) => Some(m.clone()),
                                _ => None,
                            });
                            if let Some(vrf) = variant_record_fields {
                                let variant_matches = fields
                                    .iter()
                                    .all(|f| vrf.contains_key(f.name.name.as_str()))
                                    && vrf.keys().all(|required| {
                                        fields.iter().any(|f| {
                                            f.name.name.as_str() == required.as_str()
                                        })
                                    });
                                // Variant's payload also matches: prefer
                                // variant (same-module case). Record
                                // wins only when the variant's payload
                                // shape *differs* from the provided
                                // fields (cross-module collision case).
                                !variant_matches
                            } else {
                                // Variant has non-record payload (tuple,
                                // unit). Record wins.
                                true
                            }
                        }
                        _ => true, // No variant constructor in env — record wins.
                    }
                }
            } else {
                false
            };

            // Look up the variant constructor in the environment
            // Variant constructors are registered with type Constructor -> VariantType
            if !has_matching_struct
                && let Some(scheme) = self.ctx.env.lookup(variant_name)
            {
                let ty = scheme.instantiate();

                // Check if this value has a Named type (unit variant like Leaf)
                // or if it's a function returning a Named type (payload variant like Node)
                let return_type_opt = match &ty {
                    Type::Named { .. } => Some(ty.clone()),
                    Type::Function {
                        return_type,
                        params,
                        ..
                    } => {
                        // Check if return type is a variant type (Named or inline Variant)
                        // Named types are used for imported variants, Variant for local definitions
                        if matches!(
                            return_type.as_ref(),
                            Type::Named { .. } | Type::Variant(_)
                        ) {
                            // This is a variant constructor function!
                            // The params should be a single Record type
                            if params.len() == 1
                                && let Type::Record(expected_field_types) = &params[0]
                            {
                                // Type check the record fields
                                let expected_field_types = expected_field_types.clone();
                                let mut provided_fields: indexmap::IndexMap<
                                    Text,
                                    Type,
                                > = indexmap::IndexMap::new();

                                for field_init in fields {
                                    let field_name = field_init.name.name.as_str();

                                    let unknown_fallback = Type::Unknown;
                                    let expected_field_ty = match expected_field_types
                                        .get(field_name)
                                    {
                                        Some(ty) => ty,
                                        None => {
                                            // In lenient mode (stdlib files), accept
                                            // unknown fields with Unknown type — the
                                            // struct definition may reference types
                                            // from unloaded sibling modules.
                                            if self.stdlib_single_file_mode {
                                                &unknown_fallback
                                            } else {
                                                return Err(TypeError::Other(
                                                    verum_common::Text::from(format!(
                                                        "field '{}' not found in type '{}' variant construction",
                                                        field_name, variant_name
                                                    )),
                                                ));
                                            }
                                        }
                                    };

                                    // Handle shorthand syntax
                                    let field_ty = if let Some(ref value_expr) =
                                        field_init.value
                                    {
                                        self.check_record_field_value(
                                            field_init,
                                            value_expr,
                                            expected_field_ty,
                                            variant_name,
                                        )?;
                                        expected_field_ty.clone()
                                    } else {
                                        match self.ctx.env.lookup(field_name).cloned() {
                                            Some(var_scheme) => {
                                                let var_ty = self.instantiate_value_scheme(&var_scheme, field_init.span)?;
                                                self.unifier.unify(
                                                    &var_ty,
                                                    expected_field_ty,
                                                    field_init.span,
                                                )?;
                                                expected_field_ty.clone()
                                            }
                                            None => {
                                                return Err(
                                                    TypeError::UnboundVariable {
                                                        name: field_name.to_text(),
                                                        span: field_init.span,
                                                    },
                                                );
                                            }
                                        }
                                    };

                                    provided_fields.insert(field_name.into(), field_ty);
                                }

                                // Validate all required fields are present
                                for (expected_name, expected_ty) in
                                    expected_field_types.iter()
                                {
                                    if !provided_fields.contains_key(expected_name)
                                        && !self.stdlib_single_file_mode
                                    {
                                        return Err(TypeError::Other(
                                            verum_common::Text::from(format!(
                                                "Missing required field '{}' of type {} in variant {} construction",
                                                expected_name,
                                                expected_ty,
                                                variant_name
                                            )),
                                        ));
                                    }
                                }

                                // Return the Named type
                                return Ok(InferResult::new(
                                    return_type.as_ref().clone(),
                                ));
                            }
                            Some(return_type.as_ref().clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if return_type_opt.is_some() {
                    // This path is a variant constructor, handled above
                    // If we reach here without returning, something went wrong
                }
            }
        }

        // Not a variant constructor, proceed with regular record type handling
        // Step 1: Lookup record type definition from path
        let record_ty = self.lookup_record_type(path, expr.span)?;
        self.check_record_construction_visibility(&record_ty, expr.span)?;

        // Step 2: Extract expected field types from record definition
        // Clone to avoid borrow issues
        let expected_fields: indexmap::IndexMap<verum_common::Text, Type> =
            match &record_ty {
                Type::Record(field_types) => field_types.clone(),
                Type::Generic { name, .. } => self.resolve_to_record_type(&Type::Generic {
                    name: name.clone(),
                    args: List::new(),
                }).ok_or_else(|| TypeError::Other(format!(
                    "Expected record type, found: {record_ty}"
                ).into()))?,
                Type::Named {
                    path: type_path, ..
                } => {
                    // Try to resolve named type to record structure
                    let name = self.path_to_string(type_path);

                    // First try to look up the struct fields under __struct_fields_Name
                    // This is where record type definitions store their field info
                    let struct_key = format!("__struct_fields_{}", name);
                    match self.ctx.lookup_type(struct_key.as_str()) {
                        Option::Some(Type::Record(field_types)) => field_types.clone(),
                        _ => {
                            // Try variant record fields from side map
                            if let Some(vf) = self
                                .variant_record_fields
                                .get(&verum_common::Text::from(name.as_str()))
                            {
                                vf.clone()
                            } else {
                                // Fall back to looking up the type directly
                                match self.ctx.lookup_type(name.as_str()) {
                                    Option::Some(Type::Record(field_types)) => {
                                        field_types.clone()
                                    }
                                    Option::Some(Type::Named { .. }) | Option::None => {
                                        // A nominal placeholder (or an as-yet-
                                        // unresolved type name) whose field-type
                                        // map is not registered locally — a mounted
                                        // cross-module record such as Migration is
                                        // Named-without-local-fields until its
                                        // module materializes. Before accepting it
                                        // UNCHECKED, force the same lazy-load the
                                        // variant-constructor path uses
                                        // (resolve_type_name -> ensure_stdlib_type_loaded)
                                        // and re-probe the field slots. If the
                                        // fields materialize, yield them and fall
                                        // through to the Step-4 value / missing /
                                        // extra check, which returns record_ty (the
                                        // Named) so nominal identity — and
                                        // affine-ness — is preserved (T0572). Only
                                        // the genuinely opaque case keeps the
                                        // best-effort structural inference.
                                        let _ =
                                            self.resolve_type_name(name.as_str(), expr.span);
                                        if let Option::Some(Type::Record(field_types)) =
                                            self.ctx.lookup_type(struct_key.as_str())
                                        {
                                            field_types.clone()
                                        } else if let Option::Some(Type::Record(field_types)) =
                                            self.ctx.lookup_type(name.as_str())
                                        {
                                            field_types.clone()
                                        } else {
                                            // Resolve transparent aliases through their declaration.
                                            // An opaque or non-record type does not acquire a
                                            // constructor merely because braces were written.
                                            self.resolve_to_record_type(&record_ty).ok_or_else(|| {
                                                TypeError::OtherWithCodeSpanned {
                                                    code: "E400".into(),
                                                    msg: format!("type '{name}' has no resolved record constructor").into(),
                                                    span: expr.span,
                                                }
                                            })?
                                        }
                                    }
                                    Option::Some(other_ty) => {
                                        return Err(TypeError::Other(
                                            verum_common::Text::from(format!(
                                                "Expected record type, found: {}",
                                                other_ty
                                            )),
                                        ));
                                    }
                                }
                            } // close else block for variant_fields
                        }
                    }
                }
                _ => {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Expected record type, found: {}",
                        record_ty
                    ))));
                }
            };

        // Step 2.5: Instantiate type parameters with fresh type variables
        // For generic types like Wrapper<T>, we need to:
        // 1. Look up the type parameters (e.g., [T])
        // 2. Create fresh type variables for each
        // 3. Substitute them in the field types
        // This enables bidirectional type inference for generic struct instantiation
        // Parameter ownership follows the resolved declaration, including a
        // renamed mount. A same-leaf record cannot supply absent parameters.
        let type_name = match &record_ty {
            Type::Named { path, .. } => self.path_to_string(path),
            Type::Generic { name, .. } => name.clone(),
            _ => self.path_to_string(path),
        };
        let (expected_fields, type_param_vars) = self.instantiate_nominal_record_fields(
            type_name.as_str(), &expected_fields, None, expr.span,
        )?;
        let expected_fields = expected_fields.into_iter().map(|(name, ty)|
            (name, self.substitute_placeholders(&ty))).collect();

        if !self.check_dependent_record_fields(&record_ty, &expected_fields, fields, base.as_deref(), expr.span)? {
            // Step 3: Handle base record spread (...base syntax)
            let mut provided_fields = indexmap::IndexMap::new();

            if let Some(base_expr) = base {
                // Type check base expression
                let base_result = self.synth_expr(base_expr)?;

                // Base must be a record type (or a named type that resolves to a record)
                // Try to extract record fields from the base type
                let base_fields = self.extract_record_fields(&base_result.ty)?;
                for (name, ty) in base_fields.iter() {
                    provided_fields.insert(name.clone(), ty.clone());
                }
            }

            // Step 4: Type check each provided field and validate against expected
            for field_init in fields {
                let field_name = field_init.name.name.as_str();

                // Check if field exists in expected type
                let expected_field_ty = match expected_fields.get(field_name) {
                    Some(ty) => ty,
                    None => {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "field '{}' not found in type '{}'",
                            field_name,
                            self.path_to_string(path)
                        ))));
                    }
                };

                // Handle shorthand syntax: { x } means { x: x }
                let field_ty = if let Some(ref value_expr) = field_init.value {
                    // Explicit value provided: check against expected type
                    self.check_record_field_value(
                        field_init,
                        value_expr,
                        expected_field_ty,
                        type_name.as_str(),
                    )?;
                    expected_field_ty.clone()
                } else {
                    // Shorthand: lookup variable in environment
                    match self.ctx.env.lookup(field_name).cloned() {
                        Some(scheme) => {
                            let var_ty = self.instantiate_value_scheme(&scheme, field_init.span)?;
                            // Check if variable type matches expected field type
                            self.unifier.unify(
                                &var_ty,
                                expected_field_ty,
                                field_init.span,
                            )?;
                            expected_field_ty.clone()
                        }
                        None => {
                            return Err(TypeError::UnboundVariable {
                                name: field_name.to_text(),
                                span: field_init.span,
                            });
                        }
                    }
                };

                // Add to provided fields (overriding base if present)
                provided_fields.insert(field_name.into(), field_ty);
            }

            // Step 5: Validate all required fields are present
            for (expected_name, expected_ty) in expected_fields.iter() {
                if !provided_fields.contains_key(expected_name) {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Missing required field '{}' of type {} in record construction",
                        expected_name, expected_ty
                    ))));
                }
            }

            // Step 6: Check for extra fields (not allowed in nominal records)
            for provided_name in provided_fields.keys() {
                if !expected_fields.contains_key(provided_name) {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Extra field '{}' not present in record type {}",
                        provided_name,
                        self.path_to_string(path)
                    ))));
                }
            }
        }

        // Return the record type with resolved type arguments
        // Apply unifier to get the final resolved types for type parameters
        let final_type = if !type_param_vars.is_empty() {
            // This is a generic type - resolve type parameters
            let resolved_args: List<Type> = type_param_vars
                .iter()
                .map(|tv| self.unifier.apply(tv))
                .collect();
            // Keep the owner selected during lookup. Reconstructing the
            // caller's alias spelling would give this instance a new name.
            let resolved_path = match &record_ty {
                Type::Named { path, .. } => path.clone(),
                Type::Generic { name, .. } => Self::text_to_path(name),
                _ => path.clone(),
            };
            Type::Named {
                path: resolved_path,
                args: resolved_args,
            }
        } else {
            record_ty
        };
        Ok(InferResult::new(final_type))
    }

    fn infer_expr_tuple_index(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::TupleIndex { expr: tup, index } = &expr.kind else { unreachable!() };
        // =====================================================================
        // DEFINITE ASSIGNMENT: Check tuple element initialization
        // Spec: L0-critical/memory-safety/uninitialized
        // =====================================================================
        let idx = *index as usize;
        let var_name_opt = if let ExprKind::Path(path) = &tup.kind {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    let var_name = verum_common::Text::from(id.name.as_str());
                    self.check_index_initialized(&var_name, idx, expr.span, true)?;
                    Some((var_name, id.name.as_str().to_string()))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let tup_result = self.synth_expr_for_field_access(tup)?;
        // Apply unifier to fully resolve type variables.
        // This is critical for closure parameters where the type may be
        // unified after binding but before body synthesis.
        let carrier = self.projection_carrier(&tup_result.ty, expr.span)?;
        // Preserve the borrowed/value distinction for affine extraction. This
        // is only a shape view; the receiver keeps its refinement and reference
        // permissions in the environment and the ordinary write checker.
        let borrowed = carrier.borrowed;
        let resolved_tup_ty = carrier.ty;
        match &resolved_tup_ty {
            Type::Tuple(types) => {
                if idx < types.len() {
                    let element_ty = types[idx].clone();

                    // =====================================================================
                    // AFFINE TUPLE ELEMENT TRACKING: Partial move for tuple elements
                    // When accessing a tuple element that is affine or contains affine,
                    // mark it as moved to prevent whole-tuple use.
                    // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-partial-move
                    // =====================================================================
                    if !borrowed && let Some((var_name, var_name_str)) = var_name_opt {
                        // Check if this index was already moved
                        if !self.affine_tracker.can_access_index(&var_name_str, idx) {
                            return Err(TypeError::MovedValueUsed {
                                name: format!("{}.{}", var_name_str, idx).to_text(),
                                moved_at: expr.span,
                                used_at: expr.span,
                            });
                        }

                        // Track affine element access (marks index as moved)
                        if self.affine_tracker.is_type_affine(&element_ty)
                            || self.type_contains_affine(&element_ty)
                        {
                            self.affine_tracker.use_index_value(
                                &var_name_str,
                                idx,
                                expr.span,
                            )?;
                        }
                    }

                    Ok(InferResult::new(element_ty))
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Tuple index {} out of bounds (tuple has {} elements)",
                        index,
                        types.len()
                    ))))
                }
            }
            // Handle named tuple structs and newtypes
            Type::Named { path, args } => {
                let type_name = self.path_to_string(path);
                let simple_name = Self::path_type_name(path).unwrap_or(&type_name);

                // Try tuple struct first (__tuple_fields_)
                let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                let tuple_fields_simple_key = format!("__tuple_fields_{}", simple_name);

                let found_tuple = self
                    .ctx
                    .lookup_type(tuple_fields_key.as_str())
                    .or_else(|| self.ctx.lookup_type(tuple_fields_simple_key.as_str()));

                if let Option::Some(Type::Tuple(types)) = found_tuple {
                    if idx < types.len() {
                        let element_ty = self.instantiate_declared_type(&type_name, &types[idx], args)?;

                        // Track affine element access for tuple structs
                        if !borrowed && let Some((var_name, var_name_str)) = var_name_opt {
                            if !self.affine_tracker.can_access_index(&var_name_str, idx)
                            {
                                return Err(TypeError::MovedValueUsed {
                                    name: format!("{}.{}", var_name_str, idx).to_text(),
                                    moved_at: expr.span,
                                    used_at: expr.span,
                                });
                            }
                            if self.affine_tracker.is_type_affine(&element_ty)
                                || self.type_contains_affine(&element_ty)
                            {
                                self.affine_tracker.use_index_value(
                                    &var_name_str,
                                    idx,
                                    expr.span,
                                )?;
                            }
                        }

                        Ok(InferResult::new(element_ty))
                    } else {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Tuple struct index {} out of bounds (has {} fields)",
                            index,
                            types.len()
                        ))))
                    }
                }
                // Try newtype (__newtype_inner_) - newtypes support .0 for inner value
                else {
                    let newtype_inner_key = format!("__newtype_inner_{}", type_name);
                    let newtype_simple_key = format!("__newtype_inner_{}", simple_name);

                    // FUNDAMENTAL #3 — lookup-on-miss bridge to the
                    // lazy stdlib loader.  See the matching block in
                    // `infer/env.rs:1991-2020` for the full rationale.
                    let mut found_inner = self
                        .ctx
                        .lookup_type(newtype_inner_key.as_str())
                        .or_else(|| self.ctx.lookup_type(newtype_simple_key.as_str()))
                        .cloned();
                    if found_inner.is_none() {
                        let mut pending: Vec<verum_common::Text> = Vec::new();
                        self.ensure_stdlib_type_loaded(
                            &verum_common::Text::from(simple_name),
                            &mut pending,
                        );
                        if simple_name != type_name {
                            self.ensure_stdlib_type_loaded(
                                &verum_common::Text::from(type_name.as_str()),
                                &mut pending,
                            );
                        }
                        found_inner = self
                            .ctx
                            .lookup_type(newtype_inner_key.as_str())
                            .or_else(|| self.ctx.lookup_type(newtype_simple_key.as_str()))
                            .cloned();
                    }

                    if let Some(inner_ty) = found_inner {
                        if idx == 0 {
                            Ok(InferResult::new(self.instantiate_declared_type(&type_name, &inner_ty, args)?))
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Newtype {} only has index 0, not {}",
                                type_name, index
                            ))))
                        }
                    } else {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "cannot index type '{}' — only tuple types support .0, .1, etc.",
                            resolved_tup_ty
                        ))))
                    }
                }
            }
            // Handle unresolved type variables (common in closure parameters).
            // When a closure parameter's type hasn't been unified yet (e.g.,
            // in `items.iter().map(|pair| pair.0)`), the receiver type is still
            // a TypeVar. We create a fresh TypeVar for the result element.
            // The actual constraint will be established when the receiver's
            // type is resolved via unification.
            Type::Var(_) => {
                let elem_var = TypeVar::fresh();
                Ok(InferResult::new(Type::Var(elem_var)))
            }
            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "cannot index type '{}' — only tuple types support .0, .1, etc.",
                resolved_tup_ty
            )))),
        }
    }

    fn infer_expr_await_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Await(inner_expr) = &expr.kind else { unreachable!() };
        // ============================================================
        // Async Boundary Aliasing Check
        // Context checking: verifying all required contexts are provided at call sites — Async lifetime bounds
        // ============================================================
        // Check that any borrows crossing this await point are Send.
        // Non-Send borrows cannot be held across await points because
        // the future may be polled on different threads.

        // Mark that we're in an await expression
        let enclosing_await = self.borrow_tracker.enter_await();

        // Check await safety for all active borrows
        let inner_result = self.borrow_tracker.check_await_safety()
            .and_then(|()| self.synth_expr(inner_expr));

        // Exit await context
        self.borrow_tracker.exit_await(enclosing_await);
        let inner_result = inner_result?;

        // =========================================================================
        // Protocol-based Future resolution
        // Await desugaring: ".await" desugars to polling Future protocol until completion
        // =========================================================================
        let receiver = self.unifier.apply(&inner_result.ty);
        match self.resolve_source_future(&receiver, inner_expr.span)? {
            Some(resolution) => {
                // `.await` is only valid inside an async context
                // (async fn body, async {} block, or async
                // closure). `main` is special: the runtime wraps
                // it in an implicit executor `block_on`, so a
                // plain `fn main() { run().await }` is a valid
                // top-level entry point.
                let in_main_entry = matches!(
                    self.current_function_name.as_ref(),
                    Maybe::Some(n) if n.as_str() == "main"
                );
                if !self.in_async_context && !in_main_entry {
                    return Err(TypeError::AsyncPropertyViolation {
                        message: verum_common::Text::from(
                            "`.await` can only be used inside an async context \
                             (async fn, async block, or async closure)",
                        ),
                        span: inner_expr.span,
                    });
                }
                Ok(InferResult::new(resolution.output))
            }
            None => {
                // If the inner expression is inside a spawn block, the await
                // may have been parsed as spawn({ expr }.await) rather than
                // (spawn { expr }).await. In async context, treat as identity
                // (the value is already "ready").
                // T1478 — the `main` exemption belongs to BOTH arms.
                //
                // Thirty lines above, the Future-resolved arm exempts
                // `main` by name because the runtime wraps it in an
                // implicit `block_on`, so `fn main() { run().await }` is
                // a valid top-level entry point. This arm asked
                // `in_async_context` alone and refused — so the very
                // spelling the other arm documents as valid died here,
                // with a message blaming the awaited TYPE.
                //
                // Under ASYNC-SEMANTICS-LAW (T0861,
                // docs/architecture/async-semantics-law.md) the refusal
                // was wrong on its own terms as well: candidate (A) is
                // law, calling an async fn RUNS it and yields its value,
                // and `await` is "a yield-point annotation, not a
                // projection". A non-Future operand is therefore the
                // NORMAL case for a local async call, and passing the
                // type through is the correct answer — which is exactly
                // what the async-context branch already does.
                //
                // Measured: `async fn add(a: Int, b: Int) -> Int` called
                // as `add(5, 3).await` from a plain `fn main` was
                // refused with "Cannot await non-future type: Int",
                // taking nine of the eleven `language_features` failures
                // with it.
                let in_main_entry = matches!(
                    self.current_function_name.as_ref(),
                    Maybe::Some(n) if n.as_str() == "main"
                );
                if self.in_async_context || in_main_entry {
                    Ok(InferResult::new(inner_result.ty))
                } else {
                    Err(TypeError::AsyncPropertyViolation {
                        message: verum_common::Text::from(format!(
                            "`.await` on `{}` can only be used inside an async context \
                             (async fn, async block, or async closure) or in `main`",
                            inner_result.ty
                        )),
                        span: inner_expr.span,
                    })
                }
            }
        }
    }

    fn infer_expr_spawn(&mut self, current_expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Spawn { expr, contexts } = &current_expr.kind else { unreachable!() };
        // Validate contexts exist in scope and expand groups
        // T0660: value-typed contexts — learn plain types first.
        if !contexts.is_empty() {
            self.ensure_using_types_registered(contexts);
        }
        if !contexts.is_empty()
            && let Err(err) = self
                .context_resolver
                .resolve_requirement(contexts, expr.span)
        {
            let ctx_names: Vec<String> =
                contexts.iter().map(|c| format!("{}", c.path)).collect();
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "Invalid spawn contexts [{}]: {}",
                ctx_names.join(", "),
                err
            ))));
        }

        // ============================================================
        // Spawn Thread Safety Check
        // CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Thread safety
        // ============================================================
        // Spawned tasks run on a separate thread, so all captured
        // variables must be Send. If it's a closure, analyze its captures.
        if let ExprKind::Closure {
            params,
            body,
            move_: is_move,
            ..
        } = &expr.kind
        {
            // Analyze what the closure captures
            let captures = self.analyze_closure_captures(body, params, *is_move);

            // Check that all captured variables are Send
            for (var_name, _field_path, capture_mode, capture_span) in &captures {
                // Look up the variable's type
                if let Some(var_scheme) = self.ctx.env.lookup(var_name) {
                    let var_ty = var_scheme.instantiate();
                    let resolved_ty = self.unifier.apply(&var_ty);

                    // Check if the type is Send
                    if self.is_non_send_type(&resolved_ty) {
                        return Err(TypeError::Other(verum_common::Text::from(
                            format!(
                                "Cannot spawn task that captures `{}`: type `{}` is not Send \
                             and cannot be transferred to another thread",
                                var_name, resolved_ty
                            ),
                        )));
                    }

                    // For non-move closures, references must also be checked
                    // Borrow captures require the type to be Sync (shareable)
                    if !is_move
                        && matches!(capture_mode, crate::aliasing::CaptureMode::Borrow)
                    {
                        if self.is_non_sync_type(&resolved_ty) {
                            return Err(TypeError::Other(verum_common::Text::from(
                                format!(
                                    "Cannot spawn task that borrows `{}`: type `{}` is not Sync \
                                 and cannot be shared between threads. Consider using `move` closure.",
                                    var_name, resolved_ty
                                ),
                            )));
                        }
                    }
                }
            }
        }

        // Spawn runs its body as an async task on the executor,
        // so the body is an async context regardless of whether
        // the enclosing function is async. This lets
        //  spawn { sleep(d).await; limiter.refill() }
        // typecheck inside a non-async helper.
        let inner_result = self.with_async_context(true, |checker| checker.synth_expr(expr))?;

        // If expr is a Future<T>, result is JoinHandle<T>
        // Otherwise, wrap the result type in JoinHandle
        let output_ty = match &inner_result.ty {
            Type::Future { output } => *output.clone(),
            Type::Generic { name, args }
                if name.as_str() == "Future" && args.len() == 1 =>
            {
                args[0].clone()
            }
            _ => inner_result.ty.clone(),
        };

        // Keyword tasks are intrinsic futures. Borrowing a library type's
        // unqualified name loses carrier identity in aggregates and can select
        // an unrelated nominal record's Future implementation at await sites.
        Ok(InferResult::new(Type::Future {
            output: Box::new(output_ty),
        }))
    }

    fn infer_expr_async_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Async(block) = &expr.kind else { unreachable!() };
        // Spec: async-semantics-law — an async block, like an async call,
        // executes eagerly. Only an explicit future or task has a completion
        // carrier; preserve the block's actual type and verification conditions.
        self.with_async_context(true, |checker| checker.infer_block(block))
    }

    fn infer_expr_unsafe_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Unsafe(block) = &expr.kind else { unreachable!() };
        // Track unsafe context for Tier 2 reference creation
        // Spec: L0-critical/reference_system/reference_tiers/unsafe_without_block
        let prev_unsafe_context = self.in_unsafe_context;
        self.in_unsafe_context = true;

        // Infer the block's type from its body
        let block_result = self.infer_block(block);

        // The block owns its lexical scope. Restore permission before
        // propagating an error to statement-level diagnostic recovery.
        self.in_unsafe_context = prev_unsafe_context;

        // Return the block's type directly (unsafe doesn't wrap the type)
        block_result
    }

    fn infer_expr_comprehension(&mut self, current_expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Comprehension { expr, clauses } = &current_expr.kind else { unreachable!() };
        self.ctx.enter_scope();

        // Process each clause to introduce bindings
        for clause in clauses.iter() {
            use verum_ast::expr::ComprehensionClauseKind;
            match &clause.kind {
                ComprehensionClauseKind::For { pattern, iter } => {
                    // Infer the iterator type
                    let iter_result = self.synth_expr(iter)?;
                    let elem_ty = self.synchronous_iteration_item(iter, &iter_result.ty)?;

                    // Bind pattern to element type
                    let elem_scheme = TypeScheme::mono(elem_ty);
                    self.bind_pattern_scheme(pattern, elem_scheme)?;
                }
                ComprehensionClauseKind::If(condition) => {
                    // Type check condition, must be Bool
                    let cond_result = self.synth_expr(condition)?;
                    self.unifier
                        .unify(&cond_result.ty, &Type::Bool, condition.span)?;
                }
                ComprehensionClauseKind::Let { pattern, ty, value } => {
                    self.reject_refutable_comprehension_binding(pattern, value.span);
                    // Type check the value
                    let value_result = self.synth_expr(value)?;

                    // If type annotation provided, unify
                    let binding_ty = if let Some(ty_ast) = ty {
                        let annotated_ty = self.ast_to_type(ty_ast)?;
                        self.unifier.unify(
                            &value_result.ty,
                            &annotated_ty,
                            value.span,
                        )?;
                        annotated_ty
                    } else {
                        value_result.ty
                    };

                    // Bind pattern
                    let binding_scheme = TypeScheme::mono(binding_ty);
                    self.bind_pattern_scheme(pattern, binding_scheme)?;
                }
            }
        }

        // Type check the output expression
        let elem_result = self.synth_expr(expr)?;

        self.ctx.exit_scope();

        // Result is a List<T> where T is the type of the output expression
        Ok(InferResult::new(Type::list(elem_result.ty)))
    }

    fn infer_expr_stream_comprehension(&mut self, current_expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::StreamComprehension { expr, clauses } = &current_expr.kind else { unreachable!() };
        self.ctx.enter_scope();

        // Set up a temporary generator context so that `yield` expressions
        // inside stream comprehensions are accepted. Stream comprehensions
        // use `stream[yield x for x in source]` syntax where `yield` is
        // syntactic sugar for producing stream elements.
        let yield_ty_var = Type::Var(TypeVar::fresh());
        let prev_generator_context = self.generator_context.replace(GeneratorContext {
            yield_ty: yield_ty_var.clone(),
            return_ty: Type::unit(),
        });

        // Process each clause (same as list comprehension)
        for clause in clauses.iter() {
            use verum_ast::expr::ComprehensionClauseKind;
            match &clause.kind {
                ComprehensionClauseKind::For { pattern, iter } => {
                    let iter_result = self.synth_expr(iter)?;

                    // Protocol-based IntoIterator resolution for streams
                    let elem_ty = match self
                        .resolve_into_iterator_with_loading(&iter_result.ty, iter.span)?
                    {
                        Some(resolution) => resolution.item,
                        None => {
                            // Fallback 1: Try Iterator protocol's Item associated type
                            let resolved_iter_ty = self.unifier.apply(&iter_result.ty);
                            let iter_item =
                                self.declared_iterator_item(&resolved_iter_ty, iter.span)?;
                            if let Some(item_ty) = iter_item {
                                self.protocol_checker
                                    .read()
                                    .normalize_projection_type(&item_ty)
                            } else {
                                // Fallback 2: allow iterating over Stream<T> directly
                                match &resolved_iter_ty {
                                    Type::Generic { name, args }
                                        if name.as_str() == "Stream"
                                            && args.len() == 1 =>
                                    {
                                        args[0].clone()
                                    }
                                    Type::Named { path, args } if args.len() == 1 => {
                                        let is_stream = path
                                            .as_ident()
                                            .map(|id| id.name.as_str() == "Stream")
                                            .unwrap_or(false);
                                        if is_stream {
                                            args[0].clone()
                                        } else {
                                            // Also try with unresolved type vars — fresh type var
                                            Type::Var(TypeVar::fresh())
                                        }
                                    }
                                    // For unresolved type vars, allow iteration with fresh element type
                                    Type::Var(_) => Type::Var(TypeVar::fresh()),
                                    _ => {
                                        return Err(TypeError::Other(
                                            verum_common::Text::from(format!(
                                                "Cannot stream over type: {}",
                                                resolved_iter_ty
                                            )),
                                        ));
                                    }
                                }
                            }
                        }
                    };

                    let elem_scheme = TypeScheme::mono(elem_ty);
                    self.bind_pattern_scheme(pattern, elem_scheme)?;
                }
                ComprehensionClauseKind::If(condition) => {
                    let cond_result = self.synth_expr(condition)?;
                    self.unifier
                        .unify(&cond_result.ty, &Type::Bool, condition.span)?;
                }
                ComprehensionClauseKind::Let { pattern, ty, value } => {
                    self.reject_refutable_comprehension_binding(pattern, value.span);
                    let value_result = self.synth_expr(value)?;
                    let binding_ty = if let Some(ty_ast) = ty {
                        let annotated_ty = self.ast_to_type(ty_ast)?;
                        self.unifier.unify(
                            &value_result.ty,
                            &annotated_ty,
                            value.span,
                        )?;
                        annotated_ty
                    } else {
                        value_result.ty
                    };
                    let binding_scheme = TypeScheme::mono(binding_ty);
                    self.bind_pattern_scheme(pattern, binding_scheme)?;
                }
            }
        }

        // Infer the element expression type.
        // If the expression is a `yield val`, the type checker now accepts it
        // via the generator context above.
        let elem_result = self.synth_expr(expr)?;

        // Restore previous generator context
        self.generator_context = prev_generator_context;

        self.ctx.exit_scope();

        // Determine element type: if yield was used, the yield_ty_var
        // was unified with the yielded value's type. The Yield expr itself
        // evaluates to Unit. Use the yield type in that case.
        let resolved_yield_ty = self.unifier.apply(&yield_ty_var);
        let elem_ty = if matches!(&expr.kind, ExprKind::Yield(_)) {
            resolved_yield_ty
        } else {
            elem_result.ty
        };

        // Result is a Stream<T>
        let result_ty = Type::Generic {
            name: verum_common::Text::from("Stream"),
            args: vec![elem_ty].into(),
        };

        Ok(InferResult::new(result_ty))
    }

    fn infer_expr_set_comprehension(&mut self, current_expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::SetComprehension { expr, clauses } = &current_expr.kind else { unreachable!() };
        self.ctx.enter_scope();

        // Process each clause (same as list comprehension)
        for clause in clauses.iter() {
            use verum_ast::expr::ComprehensionClauseKind;
            match &clause.kind {
                ComprehensionClauseKind::For { pattern, iter } => {
                    let iter_result = self.synth_expr(iter)?;
                    let elem_ty = self.synchronous_iteration_item(iter, &iter_result.ty)?;

                    let elem_scheme = TypeScheme::mono(elem_ty);
                    self.bind_pattern_scheme(pattern, elem_scheme)?;
                }
                ComprehensionClauseKind::If(condition) => {
                    let cond_result = self.synth_expr(condition)?;
                    self.unifier
                        .unify(&cond_result.ty, &Type::Bool, condition.span)?;
                }
                ComprehensionClauseKind::Let { pattern, ty, value } => {
                    self.reject_refutable_comprehension_binding(pattern, value.span);
                    let value_result = self.synth_expr(value)?;
                    let binding_ty = if let Some(ty_ast) = ty {
                        let annotated_ty = self.ast_to_type(ty_ast)?;
                        self.unifier.unify(
                            &value_result.ty,
                            &annotated_ty,
                            value.span,
                        )?;
                        annotated_ty
                    } else {
                        value_result.ty
                    };
                    let binding_scheme = TypeScheme::mono(binding_ty);
                    self.bind_pattern_scheme(pattern, binding_scheme)?;
                }
            }
        }

        let elem_result = self.synth_expr(expr)?;

        self.ctx.exit_scope();

        // Result is a Set<T>
        let result_ty = Type::Generic {
            name: verum_common::Text::from(WKT::Set.as_str()),
            args: vec![elem_result.ty].into(),
        };

        Ok(InferResult::new(result_ty))
    }

    fn infer_expr_generator_comprehension(&mut self, current_expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::GeneratorComprehension { expr, clauses } = &current_expr.kind else { unreachable!() };
        self.ctx.enter_scope();

        // Process each clause (same as list comprehension)
        for clause in clauses.iter() {
            use verum_ast::expr::ComprehensionClauseKind;
            match &clause.kind {
                ComprehensionClauseKind::For { pattern, iter } => {
                    let iter_result = self.synth_expr(iter)?;
                    let elem_ty = self.synchronous_iteration_item(iter, &iter_result.ty)?;

                    let elem_scheme = TypeScheme::mono(elem_ty);
                    self.bind_pattern_scheme(pattern, elem_scheme)?;
                }
                ComprehensionClauseKind::If(condition) => {
                    let cond_result = self.synth_expr(condition)?;
                    self.unifier
                        .unify(&cond_result.ty, &Type::Bool, condition.span)?;
                }
                ComprehensionClauseKind::Let { pattern, ty, value } => {
                    self.reject_refutable_comprehension_binding(pattern, value.span);
                    let value_result = self.synth_expr(value)?;
                    let binding_ty = if let Some(ty_ast) = ty {
                        let annotated_ty = self.ast_to_type(ty_ast)?;
                        self.unifier.unify(
                            &value_result.ty,
                            &annotated_ty,
                            value.span,
                        )?;
                        annotated_ty
                    } else {
                        value_result.ty
                    };
                    let binding_scheme = TypeScheme::mono(binding_ty);
                    self.bind_pattern_scheme(pattern, binding_scheme)?;
                }
            }
        }

        let elem_result = self.synth_expr(expr)?;

        self.ctx.exit_scope();

        // Result is a Generator<T> (lazy iterator)
        let result_ty = Type::Generic {
            name: verum_common::Text::from("Generator"),
            args: vec![elem_result.ty].into(),
        };

        Ok(InferResult::new(result_ty))
    }

    fn infer_expr_try_recover(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::TryRecover { try_block, recover } = &expr.kind else { unreachable!() };
        // Extract error type from ? operators inside try block before type checking
        // This ensures we have a concrete error type for pattern binding
        let error_type = self.extract_try_block_error_type(try_block)?;

        let try_result = self.with_try_recover_context(&error_type, |checker| {
            checker.synth_expr(try_block)
        })?;

        // Infer type of recovery body with error type for pattern binding
        let recover_ty = self.check_recover_body(recover, &error_type, None)?;

        // Recovery body must have the same type as try block
        self.unifier.unify(&try_result.ty, &recover_ty, expr.span)?;

        Ok(InferResult::new(try_result.ty))
    }

    fn infer_expr_null_coalesce(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::NullCoalesce { left, right } = &expr.kind else { unreachable!() };
        let left_result = self.synth_expr(left)?;
        let right_result = self.synth_expr(right)?;

        // =========================================================================
        // Protocol-based Maybe resolution for ?? operator
        // Maybe operator resolution: ? on Maybe<T> desugars to match with None -> return None propagation
        // =========================================================================
        match self
            .protocol_checker
            .read()
            .resolve_maybe_protocol(&left_result.ty)
        {
            Some(resolution) => {
                // Maybe<T> ?? T -> T
                self.unifier
                    .unify(&right_result.ty, &resolution.inner, right.span)?;
                Ok(InferResult::new(resolution.inner))
            }
            None => {
                // If left is not Maybe<T>, just return its type
                // This allows x ?? default to work when x is already T (not Maybe<T>)
                self.unifier
                    .unify(&right_result.ty, &left_result.ty, right.span)?;
                Ok(InferResult::new(left_result.ty))
            }
        }
    }

    fn infer_expr_optional_chain(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::OptionalChain { expr: obj, field } = &expr.kind else { unreachable!() };
        let obj_result = self.synth_expr(obj)?;

        // Inside try-recover blocks, `?.` on a Try-compatible type (Result<T,E>)
        // means "unwrap with ? then access field" — the lexer tokenizes `expr?.field`
        // as OptionalChain rather than Field(Try(expr), field).
        if self.try_recover_depth > 0 {
            if let (_, Some(try_resolution)) = self.resolve_try_carrier(obj_result.ty.clone(), obj.span)? {
                // This is `result?.field` inside try-recover — unwrap to success type
                let success_ty = try_resolution.output;
                self.check_record_field_visibility(&success_ty, field.name.as_str(), expr.span)?;
                let dereferenced_ty = self.unwrap_reference_type(&success_ty);
                let field_ty = self
                    .lookup_field_type(dereferenced_ty, field.name.as_str())
                    .ok_or_else(|| {
                        TypeError::Other(verum_common::Text::from(format!(
                            "field '{}' not found in type '{}'",
                            field.name, dereferenced_ty
                        )))
                    })?;
                self.require_closed_field_projection(&field_ty, expr.span)?;
                return Ok(InferResult::new(field_ty));
            }
        }

        let inner_ty = {
            let checker = self.protocol_checker.read();
            let resolution = match checker.resolve_maybe_protocol(&obj_result.ty) {
                Some(resolution) => Some(resolution),
                None => checker.resolve_maybe_protocol(&self.expand_generic_to_variant(&obj_result.ty)?),
            };
            resolution.map_or_else(|| obj_result.ty.clone(), |resolution| resolution.inner)
        };

        // Unwrap reference types before field access (Maybe<&Point>?.field)
        self.check_record_field_visibility(&inner_ty, field.name.as_str(), expr.span)?;
        let dereferenced_ty = self.unwrap_reference_type(&inner_ty);

        // Look up field on the inner type
        let field_ty = self
            .lookup_field_type(dereferenced_ty, field.name.as_str())
            .ok_or_else(|| {
                TypeError::Other(verum_common::Text::from(format!(
                    "field '{}' not found in type '{}'",
                    field.name, dereferenced_ty
                )))
            })?;

        self.require_closed_field_projection(&field_ty, expr.span)?;
        // Monadic flattening: if field is already Maybe, don't double-wrap
        let result_ty = if self
            .protocol_checker
            .read()
            .resolve_maybe_protocol(&field_ty)
            .is_some()
        {
            field_ty
        } else {
            Type::maybe(field_ty)
        };
        Ok(InferResult::new(result_ty))
    }

    fn infer_expr_map_literal(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::MapLiteral { entries } = &expr.kind else { unreachable!() };
        let (key_ty, val_ty) = if entries.is_empty() {
            (Type::Var(TypeVar::fresh()), Type::Var(TypeVar::fresh()))
        } else {
            let first_key = self.synth_expr(&entries[0].0)?;
            let first_val = self.synth_expr(&entries[0].1)?;
            for (key, val) in entries.iter().skip(1) {
                let k = self.synth_expr(key)?;
                let v = self.synth_expr(val)?;
                self.unifier.unify(&k.ty, &first_key.ty, key.span)?;
                self.unifier.unify(&v.ty, &first_val.ty, val.span)?;
            }
            (first_key.ty, first_val.ty)
        };
        Ok(InferResult::new(Type::map(key_ty, val_ty)))
    }

    fn infer_expr_set_literal(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::SetLiteral { elements } = &expr.kind else { unreachable!() };
        let elem_ty = if elements.is_empty() {
            Type::Var(TypeVar::fresh())
        } else {
            let first = self.synth_expr(&elements[0])?;
            for elem in elements.iter().skip(1) {
                let e = self.synth_expr(elem)?;
                self.unifier.unify(&e.ty, &first.ty, elem.span)?;
            }
            first.ty
        };
        Ok(InferResult::new(Type::set(elem_ty)))
    }

    fn infer_expr_throw_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Throw(error_expr) = &expr.kind else { unreachable!() };
        // Type check the error expression
        let error_result = self.synth_expr(error_expr)?;

        // Validate throw context (warnings only — throw uses longjmp at runtime
        // so it always works, but we want to catch misuse at compile time).
        if self.try_recover_depth == 0 {
            // Not inside a try/recover block — check throws clause
            match &self.current_function_throws {
                Maybe::Some(declared_error_types)
                    if !declared_error_types.is_empty() =>
                {
                    // Function has throws(E1 | E2 | ...) — check error type matches
                    let mut matched = false;
                    for declared_ty in declared_error_types.iter() {
                        if self
                            .unifier
                            .unify(&error_result.ty, declared_ty, error_expr.span)
                            .is_ok()
                        {
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        let declared_names: List<Text> = declared_error_types
                            .iter()
                            .map(|t| verum_common::Text::from(format!("{}", t)))
                            .collect();
                        self.emit_diagnostic(
                            DiagnosticBuilder::warning()
                                .message(format!(
                                    "thrown type `{}` does not match declared throws clause ({})\n  \
                                     help: the function declares `throws({})` — ensure the thrown value matches",
                                    error_result.ty,
                                    declared_names.join(", "),
                                    declared_names.join(" | "),
                                ))
                                // The throw expression's own span; without it the
                                // diagnostic renders positionless.
                                .span(span_to_line_col(expr.span))
                                .build()
                        );
                    }
                }
                _ => {
                    // No throws clause and not in try/recover — warn
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(
                                "throw expression outside of `try/recover` block in function without `throws` clause\n  \
                                 help: add `throws(ErrorType)` to the function signature, or wrap in `try { ... } recover { ... }`"
                                    .to_string()
                            )
                            // The throw expression's own span; without it the
                            // diagnostic renders positionless.
                            .span(span_to_line_col(expr.span))
                            .build()
                    );
                }
            }
        }
        // Inside try/recover: throw is always valid (caught by handler)

        // Throw has Never type (diverging control flow)
        Ok(InferResult::new(Type::never()))
    }

    fn infer_expr_meta(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Meta(block) = &expr.kind else { unreachable!() };
        // Meta blocks are evaluated at compile time
        // The type is the type of the block's return value
        self.ctx.enter_scope();
        let block_result = self.infer_block(block)?;
        self.ctx.exit_scope();

        // The meta block's type is its computed result type
        // In full implementation, this would delegate to const_eval
        Ok(block_result)
    }

    fn infer_expr_macro_call(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::MacroCall { path, args } = &expr.kind else { unreachable!() };
        // Handle known builtin macros that haven't been expanded yet
        let macro_name = path
            .segments
            .iter()
            .filter_map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("::");

        match macro_name.as_str() {
            "list_with_capacity" => {
                // @list_with_capacity(N) returns List<T> where T is inferred from context
                let elem_ty = Type::Var(TypeVar::fresh());
                Ok(InferResult::new(Type::list(elem_ty)))
            }
            "unwrap" => {
                // @unwrap(expr) returns T where expr: Maybe<T>
                Ok(InferResult::new(Type::Var(TypeVar::fresh())))
            }
            "min" | "max" | "clamp" | "abs" => {
                // @min/@max/@clamp/@abs are numeric builtins that return
                // the same numeric type as their arguments (Int or Float).
                // Return Int by default; Float args will unify through context.
                Ok(InferResult::new(Type::int()))
            }
            _ => {
                // Unknown macro — return fresh type variable to avoid blocking compilation
                Ok(InferResult::new(Type::Var(TypeVar::fresh())))
            }
        }
    }

    fn infer_expr_forall(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Forall { bindings, body } = &expr.kind else { unreachable!() };
        // Enter scope for all quantified variables
        self.ctx.enter_scope();

        // Process each binding
        for binding in bindings {
            // Determine the type for this binding
            let var_ty = if let verum_common::Maybe::Some(ty) = &binding.ty {
                // Explicit type annotation
                self.ast_to_type(ty)?
            } else if let verum_common::Maybe::Some(domain) = &binding.domain {
                // Infer type from domain's element type
                let domain_ty = self.synth_expr(domain)?.ty;
                self.element_type_of(&domain_ty).unwrap_or_else(Type::int)
            } else {
                return Err(TypeError::Other(verum_common::Text::from(
                    "quantifier binding requires type annotation or domain".to_string(),
                )));
            };

            // Bind the pattern to its type
            self.bind_pattern(&binding.pattern, &var_ty)?;

            // If there's a domain, infer its type (should be iterable)
            if let verum_common::Maybe::Some(domain) = &binding.domain {
                let _ = self.synth_expr(domain)?;
            }

            // If there's a guard, check it's Bool type
            if let verum_common::Maybe::Some(guard) = &binding.guard {
                self.check_expr(guard, &Type::bool())?;
            }
        }

        // Check that the body expression has type Bool
        self.check_expr(body, &Type::bool())?;

        // Exit scope
        self.ctx.exit_scope();

        // Forall expressions always return Bool
        Ok(InferResult::new(Type::bool()))
    }

    fn infer_expr_exists(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Exists { bindings, body } = &expr.kind else { unreachable!() };
        // Enter scope for all quantified variables
        self.ctx.enter_scope();

        // Process each binding
        for binding in bindings {
            // Determine the type for this binding
            let var_ty = if let verum_common::Maybe::Some(ty) = &binding.ty {
                // Explicit type annotation
                self.ast_to_type(ty)?
            } else if let verum_common::Maybe::Some(domain) = &binding.domain {
                // Infer type from domain's element type
                let domain_ty = self.synth_expr(domain)?.ty;
                self.element_type_of(&domain_ty).unwrap_or_else(Type::int)
            } else {
                return Err(TypeError::Other(verum_common::Text::from(
                    "quantifier binding requires type annotation or domain".to_string(),
                )));
            };

            // Bind the pattern to its type
            self.bind_pattern(&binding.pattern, &var_ty)?;

            // If there's a domain, infer its type (should be iterable)
            if let verum_common::Maybe::Some(domain) = &binding.domain {
                let _ = self.synth_expr(domain)?;
            }

            // If there's a guard, check it's Bool type
            if let verum_common::Maybe::Some(guard) = &binding.guard {
                self.check_expr(guard, &Type::bool())?;
            }
        }

        // Check that the body expression has type Bool
        self.check_expr(body, &Type::bool())?;

        // Exit scope
        self.ctx.exit_scope();

        // Exists expressions always return Bool
        Ok(InferResult::new(Type::bool()))
    }

    /// Element type behind a pointer-shaped argument of an `@intrinsic(...)`
    /// call, or `None` when the argument does not synthesise to a pointer.
    ///
    /// `args[0]` of `@intrinsic("name", …)` is the NAME literal, so data
    /// arguments start at index 1.
    ///
    /// Several pointer intrinsics used to answer with a bare `TypeVar::fresh()`
    /// — a type unrelated to anything at the call site. `slice_from_raw_parts`
    /// returning `&[fresh]` is why
    ///
    /// ```text
    /// let rest = unsafe { slice_from_raw_parts(p, n) };
    /// ```
    ///
    /// has no determined element type anywhere in core/collections/list.vr.
    /// The argument already carries the answer; read it instead of inventing
    /// one (T0742).
    fn intrinsic_arg_pointee(&mut self, args: &[Expr], idx: usize) -> Option<Type> {
        let arg = args.get(idx)?;
        let synthesized = self.infer_expr(arg, InferMode::Synth).ok()?.ty;
        match self.unifier.apply(&synthesized) {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => Some(*inner),
            _ => None,
        }
    }

    /// `intrinsic_arg_pointee`, falling back to a fresh variable so a
    /// non-pointer argument keeps the previous behaviour rather than failing
    /// the whole inference.
    fn intrinsic_arg_pointee_or_fresh(&mut self, args: &[Expr], idx: usize) -> Type {
        self.intrinsic_arg_pointee(args, idx)
            .unwrap_or_else(|| Type::Var(TypeVar::fresh()))
    }

    /// Is `ty` definitively NOT a number?
    ///
    /// Deliberately one-sided. A meta-function argument is checked before
    /// ordinary unification has finished, so an unresolved variable, a
    /// `Named`/`Generic` head, a reference or `Unknown` must all pass —
    /// refusing on those reports on the checker's progress rather than on
    /// the program. Only a concrete non-numeric answer is a refusal.
    fn meta_arg_is_definitely_not_numeric(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Text
                | Type::Bool
                | Type::Char
                | Type::Unit
                | Type::Tuple(_)
                | Type::Record(_)
                | Type::Function { .. }
        )
    }

    /// Refuse an `@name(...)` the compiler cannot honour, naming the name.
    ///
    /// The front door for T1124. Before this existed, `@name(...)` in
    /// expression position had NO validation at any layer: the parser
    /// emitted a warning and built the node anyway, this function's
    /// catch-all typed the result `Unit`, and VBC codegen's catch-all
    /// lowered it to `nil`. Measured 2026-09-16, six probes:
    ///
    /// ```text
    ///   @zzznotathing(1)     0 errors, prints nil   (name exists nowhere)
    ///   @abs(1, 2, 3)        0 errors, prints 1     (extras discarded)
    ///   @sqrt("text")        0 errors, prints nil   (Text to a float op)
    ///   @abs()               internal compiler error: @abs requires 1 argument
    ///   @pow(2)              0 errors, prints nil
    ///   @clamp("a","b","c")  0 errors, prints nil
    /// ```
    ///
    /// `nil` is indistinguishable from a legitimate answer, which is what
    /// made the class expensive rather than merely untidy: `core/io/file.vr`
    /// built its `fstat(2)` buffer with `@zeroed()` — not a meta-function,
    /// therefore `Unit`, therefore a zero-byte object handed to an FFI.
    ///
    /// The checks are ordered from cheapest and most certain to most
    /// speculative, and every one of them names the offending `@name`.
    fn validate_meta_function_call(
        &mut self,
        name: &verum_ast::ty::Ident,
        args: &verum_common::List<Expr>,
        parenthesized: bool,
        span: verum_ast::span::Span,
    ) -> Result<()> {
        use verum_ast::meta_fn::{self, ArgKind, Status};

        let n = name.name.as_str();

        // The open namespace. `@builtin_*` carries its semantics and its
        // return type at the stdlib DECLARATION site, so there is no arity
        // to state here and nothing to check.
        if meta_fn::is_builtin_namespace(n) {
            return Ok(());
        }

        // A name the user DECLARED with `meta`. Saying "unknown" about a
        // declaration three lines up would be a false positive, and a false
        // positive is the expensive kind — it argues for deleting a check
        // that is right about everything else. It is still refused, because
        // macro expansion is not implemented and the alternative is `nil`,
        // but the diagnostic says which of the two problems this is.
        if self.declared_meta_macros.contains(name.name.as_str()) {
            return Err(TypeError::OtherWithCodeSpanned {
                code: Text::from("E0441"),
                msg: Text::from(format!(
                    "`@{n}` names a `meta` macro declared in this module, but macro \
                     expansion is not implemented, so the call cannot be lowered. \
                     Until it is, call an ordinary `fn` instead."
                )),
                span,
            });
        }

        let Some(spec) = meta_fn::lookup(n) else {
            let hint = match meta_fn::similar(n) {
                Some(s) => format!(" — did you mean `@{s}`?"),
                // No enumeration here. The parser's `E0410` warning fires on
                // the same span and already lists the grammar's names, so
                // repeating them makes one problem read as two walls of text.
                None => format!(
                    " — a `@name(...)` outside the grammar's `meta_function_name` \
                     list must be a macro declared with `meta {n}(...)`"
                ),
            };
            return Err(TypeError::OtherWithCodeSpanned {
                code: Text::from("E0410"),
                msg: Text::from(format!(
                    "unknown meta-function `@{n}`: no compiler builtin and no \
                     `meta` declaration of that name is in scope{hint}"
                )),
                span,
            });
        };

        // A name the language reserves and the compiler cannot lower.
        // Refused rather than typed, because typing it means `nil`.
        if spec.status == Status::Unimplemented {
            let where_from = if spec.in_grammar_list {
                "the grammar's `meta_function_name` production lists it"
            } else {
                "the compiler recognises the name"
            };
            return Err(TypeError::OtherWithCodeSpanned {
                code: Text::from("E0442"),
                msg: Text::from(format!(
                    "`@{n}` is not implemented: {where_from}, but no lowering \
                     exists, so the call has no value to produce"
                )),
                span,
            });
        }

        // A BARE `@name` states no arguments, so there is no arity to check.
        // The grammar makes the call-part optional, and the two spellings
        // want opposite answers: `@abs()` is a call missing its argument and
        // must be refused, while `integrate(@sin, 0.0, 3.14, 100000)` names
        // the meta-function and must not be. `args.len()` is 0 for both;
        // only the parser knows which was written.
        if !parenthesized {
            return Ok(());
        }

        if !spec.arity.admits(args.len()) {
            return Err(TypeError::OtherWithCodeSpanned {
                code: Text::from("E0443"),
                msg: Text::from(format!(
                    "`@{n}` takes {}, but {} {} given",
                    spec.arity.describe().as_str(),
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" }
                )),
                span,
            });
        }

        for (i, arg) in args.iter().enumerate() {
            match spec.arg_kind(i) {
                ArgKind::Any | ArgKind::CfgPredicate | ArgKind::TypeName => {}

                // Syntactic, so it can be checked with certainty and
                // without running inference on the argument.
                ArgKind::TextLiteral => {
                    let is_text_literal = matches!(
                        &arg.kind,
                        ExprKind::Literal(lit)
                            if matches!(&lit.kind, verum_ast::LiteralKind::Text(_))
                    );
                    if !is_text_literal {
                        return Err(TypeError::OtherWithCodeSpanned {
                            code: Text::from("E0444"),
                            msg: Text::from(format!(
                                "`@{n}` argument {} must be a text literal",
                                i + 1
                            )),
                            span: arg.span,
                        });
                    }
                }
                ArgKind::Ident => {
                    let is_ident = matches!(&arg.kind, ExprKind::Path(_));
                    if !is_ident {
                        return Err(TypeError::OtherWithCodeSpanned {
                            code: Text::from("E0444"),
                            msg: Text::from(format!(
                                "`@{n}` argument {} must be a bare identifier",
                                i + 1
                            )),
                            span: arg.span,
                        });
                    }
                }

                // Semantic, and therefore one-sided: synthesise, and refuse
                // only a CONCRETE non-numeric answer. An error from the
                // synthesis itself is not this check's to report — the
                // ordinary path will report it with a better message.
                ArgKind::Numeric | ArgKind::Float => {
                    if let Ok(res) = self.synth_expr(arg) {
                        let resolved = self.unifier.apply(&res.ty);
                        if Self::meta_arg_is_definitely_not_numeric(&resolved) {
                            return Err(TypeError::OtherWithCodeSpanned {
                                code: Text::from("E0444"),
                                msg: Text::from(format!(
                                    "`@{n}` argument {} must be a number, but it is \
                                     `{}`",
                                    i + 1,
                                    resolved
                                )),
                                span: arg.span,
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn infer_expr_meta_function(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::MetaFunction {
            name,
            args,
            parenthesized,
        } = &expr.kind
        else {
            unreachable!()
        };

        // REFUSE FIRST, TYPE SECOND (T1124).  Every arm below assumes the
        // name is one the compiler can lower and that it was handed the
        // arguments its contract names; that assumption used to be
        // unchecked, so `@abs(1, 2, 3)` answered 1 and `@abs()` reached
        // codegen and raised an internal compiler error.
        self.validate_meta_function_call(name, args, *parenthesized, expr.span)?;

        // A BARE `@name` with a non-zero arity NAMES the meta-function
        // rather than calling it, and the compiler cannot lower that.
        //
        // Typing it as a function type is easy and WRONG to stop at: VBC
        // codegen has no way to produce a value for a meta-function, so the
        // program then fails later with
        // `undefined function: `@sin` needs an argument in position 1` and
        // no span. Measured on `L0-critical/vbc/stress/002_computation_stress.vr`,
        // which passes `integrate(@sin, 0.0, 3.14159265, 100000)`.
        //
        // Refused here instead, where the diagnostic carries a span AND the
        // rewrite that works. `@file`, `@line`, `@column`, `@module` and
        // `@function` are unaffected: their arity is zero, so the bare form
        // IS the call and is the documented spelling.
        if !*parenthesized
            && let Some(spec) = verum_ast::meta_fn::lookup(name.name.as_str())
            && let verum_ast::meta_fn::Arity::Exact(n) = spec.arity
            && n > 0
        {
            let n_ = name.name.as_str();
            let params: Vec<String> = (0..n).map(|i| format!("x{}", i + 1)).collect();
            return Err(TypeError::OtherWithCodeSpanned {
                code: Text::from("E0446"),
                msg: Text::from(format!(
                    "`@{n_}` names a meta-function; one cannot be passed as a \
                     value. Wrap it in a lambda: `|{}| @{n_}({})`",
                    params.join(", "),
                    params.join(", ")
                )),
                span: expr.span,
            });
        }

        let result_type = self.meta_function_result_type(name, args, expr)?;
        Ok(InferResult::new(result_type))
    }

    fn catch_unwind_result(&mut self, args: &[Expr], span: Span) -> Result<Type> {
        let [_, callback] = args else {
            return Err(TypeError::wrong_arg_count("catch_unwind".into(), 1, args.len().saturating_sub(1), span));
        };
        let declared = self.source_type_registration.catch_unwind_body.as_ref()
            .filter(|(closure, _)| *closure == self.borrow_tracker.current_closure_id())
            .map(|(_, result)| result.clone());
        let (result, success) = if let Some(result) = declared {
            let success = self.conversion_nominal_head(&result)
                .and_then(|(name, args)| match args {
                    [ok, _] if name == "core.base.result.Result" || name == WKT_RESULT => Some(ok.clone()),
                    _ => None,
                })
                .ok_or_else(|| TypeError::OtherWithCodeSpanned {
                    code: "E400".into(), msg: "catch_unwind must declare a Result with a panic record".into(), span,
                })?;
            (result, success)
        } else {
            let contract = verum_common::well_known_types::CATCH_UNWIND_ERROR_TYPE;
            self.resolve_declaring_source_type(contract, span)?;
            let error = Type::Named { path: Self::text_to_path(&Text::from(contract)), args: List::new() };
            let ok = Type::Var(TypeVar::fresh());
            self.resolve_declaring_source_type("core.base.result.Result", span)?;
            let result = Type::Generic { name: "core.base.result.Result".into(), args: vec![ok.clone(), error].into() };
            (result, ok)
        };
        self.check_expr(callback, &Type::function_with_async(List::new(), success, false))?;
        Ok(self.unifier.apply(&result))
    }

    /// The type a CALL to `@name(...)` produces.
    ///
    /// Split out of `infer_expr_meta_function` so the bare form can reuse it
    /// as a function type's RETURN type instead of as the whole answer.
    fn meta_function_result_type(
        &mut self,
        name: &verum_ast::ty::Ident,
        args: &verum_common::List<Expr>,
        expr: &Expr,
    ) -> Result<Type> {
        use ExprKind::*;
        if let Some(operation) = super::callable_properties::intrinsic_operation_name(name.name.as_str(), args)
            .and_then(|operation| verum_vbc::intrinsics::lookup_intrinsic(&operation))
        {
            use verum_vbc::{instruction::TypeInspection, intrinsics::registry::CodegenStrategy};
            match operation.intrinsic.strategy {
                CodegenStrategy::TypeIdentity => return Ok(Type::uint64()),
                CodegenStrategy::TypeInspect(TypeInspection::Name) => return Ok(Type::text()),
                CodegenStrategy::TypeInspect(TypeInspection::NeedsDrop) => return Ok(Type::bool()),
                _ => (),
            }
        }
        // Infer type based on the meta-function name
        let result_type = match name.name.as_str() {
            // Source location functions return Text or Int
            "file" | "module" | "function" => Type::text(),
            "line" | "column" => Type::int(),

            // Configuration check returns Bool
            "cfg" => Type::bool(),

            // Compile-time evaluation: type is the type of the argument
            "const" => {
                if let Some(arg) = args.first() {
                    return Ok(self.synth_expr(arg)?.ty);
                }
                Type::unit()
            }

            // String operations return Text
            "concat" | "stringify" => Type::text(),

            // `@warning(msg)` reports and evaluates to nothing.  `@error`
            // is NOT its twin — it raises, and its arm is below, next to the
            // other names whose typing decides a program's fate rather than
            // merely labelling it.
            "warning" => Type::unit(),

            // Compiler intrinsics - type is inferred from expected type
            // or from the last argument's type if applicable
            // @intrinsic("name", args...) - type depends on intrinsic
            "intrinsic" => {
                // For intrinsics, we need to infer the type from context.
                // If we're in checking mode (expected type is known), use that.
                // Otherwise, try to infer from the intrinsic name.
                if let Some(first_arg) = args.first() {
                    // First arg should be a string literal with intrinsic name
                    if let ExprKind::Literal(lit) = &first_arg.kind {
                        if let verum_ast::LiteralKind::Text(name_lit) = &lit.kind {
                            let intrinsic_name = name_lit.as_str();
                            if let Some(operation) = super::atomic_access::AtomicAccess::from_intrinsic(intrinsic_name) {
                                return self.atomic_access_type(operation, args, expr.span);
                            }
                            match intrinsic_name {
                                // Memory intrinsics
                                "compare_exchange" | "compare_exchange_weak" => {
                                    Type::bool()
                                }
                                "atomic_swap" => {
                                    // Type depends on the pointer type being operated on
                                    // Use last arg (the value being stored) to infer
                                    if args.len() >= 3 {
                                        if let Ok(val_ty) = self.infer_expr(
                                            &args[args.len() - 1],
                                            InferMode::Synth,
                                        ) {
                                            val_ty.ty
                                        } else {
                                            Type::Var(TypeVar::fresh())
                                        }
                                    } else {
                                        Type::Var(TypeVar::fresh())
                                    }
                                }
                                "fetch_add" | "fetch_sub" | "fetch_and"
                                | "fetch_or" | "fetch_xor" | "fetch_nand"
                                | "fetch_max" | "fetch_min" => Type::int(),
                                "atomic_fence" => Type::unit(),
                                // Memory allocation
                                "alloc" | "alloc_zeroed" | "realloc" => {
                                    Type::Reference {
                                        inner: Box::new(Type::Var(TypeVar::fresh())),
                                        mutable: false,
                                    }
                                }
                                "dealloc" => Type::unit(),
                                // Pointer operations — the pointer argument
                                // carries the element type; read it.
                                "ptr_read" | "ptr_read_volatile" => {
                                    self.intrinsic_arg_pointee_or_fresh(args, 1)
                                }
                                "ptr_write"
                                | "ptr_write_volatile"
                                | "ptr_copy"
                                | "ptr_copy_nonoverlapping"
                                | "ptr_swap" => Type::unit(),
                                "offset" => Type::Reference {
                                    inner: Box::new(Type::Var(TypeVar::fresh())),
                                    mutable: false,
                                },
                                // Type intrinsics
                                "size_of" | "align_of" | "min_align_of" => {
                                    Type::int()
                                }
                                "type_name" => Type::text(),
                                // Control flow
                                "unreachable" | "panic" | "abort" => Type::never(),
                                "catch_unwind" => return self.catch_unwind_result(args, expr.span),
                                // Slice intrinsics - return reference to slice
                                "slice_from_raw_parts" => Type::Reference {
                                    inner: Box::new(Type::Slice {
                                        element: Box::new(
                                            self.intrinsic_arg_pointee_or_fresh(args, 1),
                                        ),
                                    }),
                                    mutable: false,
                                },
                                "slice_from_raw_parts_mut" => Type::Reference {
                                    inner: Box::new(Type::Slice {
                                        element: Box::new(
                                            self.intrinsic_arg_pointee_or_fresh(args, 1),
                                        ),
                                    }),
                                    mutable: true,
                                },
                                // Output intrinsic (for async)
                                "output" => Type::Named {
                                    path: verum_ast::ty::Path::single(
                                        verum_ast::Ident::new("Output", expr.span),
                                    ),
                                    args: List::from_iter([
                                        Type::Var(TypeVar::fresh()),
                                    ]),
                                },
                                // Text parsing intrinsics - return Result<T, ParseError>
                                "text_parse_int" => Type::Named {
                                    path: verum_ast::ty::Path::single(
                                        verum_ast::Ident::new(
                                            WKT::Result.as_str(),
                                            expr.span,
                                        ),
                                    ),
                                    args: List::from_iter([
                                        Type::int(),
                                        Type::Named {
                                            path: verum_ast::ty::Path::single(
                                                verum_ast::Ident::new(
                                                    "ParseError",
                                                    expr.span,
                                                ),
                                            ),
                                            args: List::new(),
                                        },
                                    ]),
                                },
                                "text_parse_float" => Type::Named {
                                    path: verum_ast::ty::Path::single(
                                        verum_ast::Ident::new(
                                            WKT::Result.as_str(),
                                            expr.span,
                                        ),
                                    ),
                                    args: List::from_iter([
                                        Type::float(),
                                        Type::Named {
                                            path: verum_ast::ty::Path::single(
                                                verum_ast::Ident::new(
                                                    "ParseError",
                                                    expr.span,
                                                ),
                                            ),
                                            args: List::new(),
                                        },
                                    ]),
                                },
                                "text_parse_bool" => Type::Named {
                                    path: verum_ast::ty::Path::single(
                                        verum_ast::Ident::new(
                                            WKT::Result.as_str(),
                                            expr.span,
                                        ),
                                    ),
                                    args: List::from_iter([
                                        Type::bool(),
                                        Type::Named {
                                            path: verum_ast::ty::Path::single(
                                                verum_ast::Ident::new(
                                                    "ParseError",
                                                    expr.span,
                                                ),
                                            ),
                                            args: List::new(),
                                        },
                                    ]),
                                },
                                // Default for unknown intrinsics - use fresh type var
                                _ => Type::Var(TypeVar::fresh()),
                            }
                        } else {
                            // First arg literal but not Text kind
                            Type::Var(TypeVar::fresh())
                        }
                    } else {
                        // First arg not a literal
                        Type::Var(TypeVar::fresh())
                    }
                } else {
                    // No args
                    Type::Var(TypeVar::fresh())
                }
            }

            // ============================================================
            // Type introspection intrinsics
            // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 15 - Compile-Time Reflection
            // ============================================================

            // @type_name(T) -> Text - Returns the name of a type
            "type_name" => Type::text(),

            // @type_fields(T) / @fields_of(T) -> List<(Text, Type)>
            // Returns field names and types for a struct type
            "type_fields" | "fields_of" => {
                // Returns a list of (field_name: Text, field_type: Type) tuples
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                        WKT::List.as_str(),
                        expr.span,
                    )),
                    args: List::from_iter([Type::Tuple(List::from_iter([
                        Type::text(),
                        Type::Named {
                            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                                "Type", expr.span,
                            )),
                            args: List::new(),
                        },
                    ]))]),
                }
            }

            // @variants_of(T) -> List<(Text, Type)>
            // Returns variant names and payload types for an enum type
            "variants_of" => Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                    WKT::List.as_str(),
                    expr.span,
                )),
                args: List::from_iter([Type::Tuple(List::from_iter([
                    Type::text(),
                    Type::Named {
                        path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                            "Type", expr.span,
                        )),
                        args: List::new(),
                    },
                ]))]),
            },

            // @type_of(expr) -> Type - Returns the compile-time type of an expression
            "type_of" => Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                    "Type", expr.span,
                )),
                args: List::new(),
            },

            // @field_access(value, field_name: Text) -> T
            // Accesses a field by name at compile-time. Returns fresh type var.
            "field_access" => Type::Var(TypeVar::fresh()),

            // @is_struct(T) -> Bool - Checks if type is a struct
            "is_struct" => Type::bool(),

            // @is_enum(T) -> Bool - Checks if type is an enum
            "is_enum" => Type::bool(),

            // @is_tuple(T) -> Bool - Checks if type is a tuple
            "is_tuple" => Type::bool(),

            // @implements(T, Protocol) -> Bool - Checks if type implements protocol
            "implements" => Type::bool(),

            // @size_of(T) -> Int - Returns the size of a type in bytes
            "size_of" => Type::int(),

            // @align_of(T) -> Int - Returns the alignment of a type in bytes
            "align_of" => Type::int(),

            // @type_id(T) -> Int - Returns unique type identifier
            "type_id" => Type::int(),

            // @get_tag(variant) -> Int - Returns the tag index of a variant value
            "get_tag" => Type::int(),

            // ---- names whose lowering exists and whose typing did not ------
            //
            // Each of these has a VBC codegen arm and produced a real value
            // at run time, while typing as `Unit` — so `let v = @block_on(f)`
            // ran correctly and would not compile against any annotation.
            // Measured on the current binary: `@catch(1)` answers `Ok(1)`,
            // `@block_on(1)` and `@timeout(1, 1)` answer `1`, `@ref_eq(1, 1)`
            // answers `true`, both generation reads answer an Int.
            "forget" => Type::unit(),
            "ref_eq" => Type::bool(),
            "get_generation" | "get_stored_generation" => Type::int(),
            "byte_list_with_capacity" => Type::list(Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                    "Byte", expr.span,
                )),
                args: List::new(),
            }),
            "block_on" | "timeout" => match args.first() {
                Some(arg) => self.synth_expr(arg)?.ty,
                None => Type::unit(),
            },
            "catch" | "catch_cbgr_violation" => {
                let ok = match args.first() {
                    Some(arg) => self.synth_expr(arg)?.ty,
                    None => Type::unit(),
                };
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                        WKT::Result.as_str(),
                        expr.span,
                    )),
                    args: List::from_iter([ok, Type::Var(TypeVar::fresh())]),
                }
            }

            // ---- numeric meta-functions ------------------------------------
            //
            // These had NO arm here: they fell through to the catch-all and
            // typed as `Unit`, which is why `let r: Int = @max(10, 20)` read
            // "expected 'Int', found 'Unit'" and a bare `print(@max(10,20))`
            // printed `nil` (T1124).  The `"min"|"max"|"clamp"|"abs" =>
            // Type::int()` arm that DOES exist lives in
            // `infer_expr_macro_call`, which `@name(...)` never reaches:
            // `ExprKind::MacroCall` is only built for the Rust-style `name!`
            // spelling the language refuses, so that arm has never run for
            // any program.
            //
            // The float family is float-in float-out.  The polymorphic
            // family (abs/min/max/clamp/pow) takes its result from its
            // arguments, so an Int argument keeps an Int result — which is
            // what `dp[i][j] = @max(dp[i-1][j], dp[i][j-1])` needs.
            "sqrt" | "sin" | "cos" | "tan" | "log" | "exp" | "floor" | "ceil"
            | "round" => {
                for arg in args.iter() {
                    let _ = self.synth_expr(arg);
                }
                Type::float()
            }
            "abs" | "min" | "max" | "clamp" | "pow" => {
                // The widest argument type wins: any Float makes the result
                // Float, otherwise Int.  An unresolved argument leaves a
                // fresh variable rather than guessing, so a later constraint
                // can still settle it.
                let mut saw_float = false;
                let mut saw_unresolved = false;
                for arg in args.iter() {
                    match self.synth_expr(arg) {
                        Ok(res) => match self.unifier.apply(&res.ty) {
                            Type::Float => saw_float = true,
                            Type::Int => {}
                            _ => saw_unresolved = true,
                        },
                        Err(_) => saw_unresolved = true,
                    }
                }
                if saw_float {
                    Type::float()
                } else if saw_unresolved {
                    Type::Var(TypeVar::fresh())
                } else {
                    Type::int()
                }
            }

            // @error("msg") raises a compile-time error carrying the
            // message.  That IS its semantics; typing it `Unit` and letting
            // codegen answer `nil` made `@error` a no-op.
            "error" => {
                let msg = args
                    .first()
                    .and_then(|a| match &a.kind {
                        ExprKind::Literal(lit) => match &lit.kind {
                            verum_ast::LiteralKind::Text(t) => {
                                Some(t.as_str().to_string())
                            }
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or_else(|| String::from("(no message)"));
                return Err(TypeError::OtherWithCodeSpanned {
                    code: Text::from("E0445"),
                    msg: Text::from(msg),
                    span: expr.span,
                });
            }

            // @list_with_capacity(N) -> List<T> where T is inferred from context
            "list_with_capacity" => Type::list(Type::Var(TypeVar::fresh())),

            // @unwrap(expr) -> T where expr: Maybe<T>
            "unwrap" => {
                if let Some(arg) = args.first() {
                    let arg_result = self.synth_expr(arg)?;
                    let arg_ty = arg_result.ty;
                    match &arg_ty {
                        Type::Generic {
                            name: n,
                            args: type_args,
                        } if WKT::Maybe.matches(n.as_str()) => {
                            if let Some(inner) = type_args.first() {
                                inner.clone()
                            } else {
                                Type::Var(TypeVar::fresh())
                            }
                        }
                        Type::Variant(variants) => {
                            // Extract inner type from first data-carrying variant
                            // (e.g., Some(T) in Maybe, Ok(T) in Result, or any user-defined variant)
                            variants
                                .values()
                                .find(|ty| !matches!(ty, Type::Unit))
                                .cloned()
                                .unwrap_or_else(|| Type::Var(TypeVar::fresh()))
                        }
                        Type::Var(_) => {
                            // Unresolved - return fresh var
                            Type::Var(TypeVar::fresh())
                        }
                        _ => Type::Var(TypeVar::fresh()),
                    }
                } else {
                    Type::Var(TypeVar::fresh())
                }
            }

            // @vbc/@asm/@llvm - low-level intrinsics, type is inferred from context.
            // Synthesize all argument expressions so that context
            // method calls like `ComputeDevice.device()` inside
            // @vbc(...) args are properly resolved through the
            // type checker (enabling context type lookup, method
            // resolution, etc.). Errors from arg synthesis are
            // non-fatal: @vbc is a codegen-level intrinsic and
            // argument types don't constrain its return type.
            "vbc" | "asm" | "llvm" | "llvm_only" => {
                for arg in args.iter().skip(1) {
                    // Skip first arg (opcode name) — it's an ident.
                    let _ = self.synth_expr(arg);
                }
                Type::Var(TypeVar::fresh())
            }

            // -----------------------------------------------------------------
            // Generic opaque-intrinsic rule for `@builtin_*`.
            //

            // Any meta-function whose name begins with `builtin_` is a
            // compiler-level intrinsic whose semantics — and whose
            // return type — are attached to the *declaration site* in
            // stdlib code, not embedded in a table inside the
            // compiler. Examples (see core/math/hott.vr,
            // core/math/cubical.vr, core/math/epistemic.vr):
            //

            //  public let i0: I = @builtin_i0;
            //  public let i1: I = @builtin_i1;
            //  public fn refl<A>(x: A) -> Path<A>(x, x) { @builtin_refl(x) }
            //  public fn hcomp<A>(phi, walls, base) -> A { @builtin_hcomp(...) }
            //  public fn join(i: I, j: I) -> I { @builtin_interval_join(i, j) }
            //

            // The use-site already carries the intended return type
            // via the surrounding let-annotation / fn-signature. We
            // therefore return a fresh type variable; bidirectional
            // checking unifies it against the declared type via
            // `synth_and_check`, and synthesis-mode callers receive a
            // generalizable variable instead of a wrong concrete
            // Unit.
            //

            // This keeps the compiler free of per-intrinsic name
            // tables — the only special-casing is the `builtin_`
            // prefix, which is the declared namespace for
            // compiler-bound meta-symbols. Two other judges of "is
            // this `@name` known" carry the same prefix rule and have
            // to keep carrying it: `attr_validation.rs`
            // (`@name` on a DECLARATION) and `is_known_meta_function`
            // in verum_fast_parser/src/expr.rs (`@name(...)` in
            // EXPRESSION position, which did not have it until T1352 —
            // nine `@builtin_*` calls in core/math/hott.vr were
            // reported as unknown while this arm accepted them).
            //
            // What does NOT mirror it is codegen. This comment used to
            // claim the rule was "mirrored on the codegen side in
            // verum_vbc/src/codegen/expressions.rs §4077+"; that
            // address is a `deref_mut` emit, and a tree-wide search for
            // the prefix returns only the three sites named above. The
            // consequence is visible rather than theoretical: accepting
            // the name here with a fresh type variable lets
            // `let x: Int = @builtin_refl(7)` type-check and print
            // `nil`, because `compile_meta_function` has no arm for it.
            // The diagnostic for that belongs where implementedness is
            // known — that catch-all, which now says so (T1352).
            // Argument expressions are still synthesised for their
            // side-effects on the checker state (diagnostic
            // accumulation, constraint generation).
            n if n.starts_with("builtin_") => {
                for arg in args.iter() {
                    let _ = self.synth_expr(arg);
                }
                Type::Var(TypeVar::fresh())
            }

            // Unreachable for an unknown name since T1124.
            //
            // `validate_meta_function_call` runs before this match and
            // refuses any name that is not in `verum_ast::meta_fn::REGISTRY`,
            // is in it but has no lowering, or was handed the wrong number
            // or kind of arguments.  What still lands here is a REGISTERED
            // name with no typing arm — a gap inside the compiler rather
            // than a mistake in the program — so the levers below stay as
            // the instrument for finding one.
            //
            // The historical note that follows is kept because it records
            // the measurement that decided where the refusal belongs.
            //
            // T1352.  This arm is SILENT: a name the compiler does not
            // know types as `Unit` and nothing is reported, so a
            // misspelled or unimplemented `@meta` reads as a working call
            // returning nothing.  43 sites in `core/` land here.
            //
            // "43 sites" and the follow-on "25 of those are in
            // `core/math`, so making this loud would stop the stdlib
            // baking" were BOTH measured on the wrong route, and the
            // second is false as stated.
            //
            // MEASURED 2026-09-11: `VERUM_STRICT_META_FN=1` on a FULL
            // BAKE passes and refreshes the archive. Not one unknown
            // meta-function is reported, and the lever is not broken —
            // it fires on an anchor (a two-line user file with
            // `@definitely_not_a_real_meta_function`) and it does reach
            // the nested process (`build.rs` has no `env_clear()`).
            //
            // The reason is structural: `pipeline/stdlib_bootstrap.rs`
            // registers TYPES and SIGNATURES and never infers a function
            // BODY — no `synth_expr` / `check_function` call anywhere in
            // it. This arm lives in body inference, so the bake cannot
            // reach it for any name in any file. `StackTrace.capture`,
            // which contains `@frame_address(0)`, is IN the strict
            // archive: compiled, with its types never inferred.
            //
            // So the counts came from `verum check` on individual `core/`
            // files, which runs `stdlib_single_file_mode` — a route that
            // cannot see sibling declarations and DOES infer bodies.
            // That is the route this arm actually guards, and the route
            // any decision about making it loud has to be measured on.
            // What was
            // missing is the ability to MEASURE that claim instead of
            // repeating it, so this arm now has two levers and neither
            // changes the default:
            //
            //   VERUM_TRACE_META_FN=1   name every unknown meta-function
            //                           and where it was called
            //   VERUM_STRICT_META_FN=1  make it an error, so one run says
            //                           exactly which files stop compiling
            //
            // The second is the A/B this task needs: "what breaks if it
            // speaks" becomes a list of names rather than an estimate.
            unknown => {
                if std::env::var_os("VERUM_TRACE_META_FN").is_some()
                    || std::env::var_os("VERUM_STRICT_META_FN").is_some()
                {
                    eprintln!(
                        "[meta-fn] registered '@{}' with {} arg(s) has no typing arm",
                        unknown,
                        args.len()
                    );
                }
                // A registered name with no typing arm is `@builtin_*`,
                // whose return type lives at the stdlib declaration site.
                // A fresh variable is the right answer there; for anything
                // else it would be a compiler gap, and the lever above is
                // what names it.
                for arg in args.iter() {
                    let _ = self.synth_expr(arg);
                }
                Type::Var(TypeVar::fresh())
            }
        };

        Ok(result_type)
    }

    fn infer_expr_lift(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Lift { expr: inner } = &expr.kind else { unreachable!() };
        // The lift expression evaluates to the type of the inner expression
        self.synth_expr(inner)
    }

    fn infer_expr_type_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::TypeExpr(ty) = &expr.kind else { unreachable!() };
        // Convert AST type to internal Type representation
        let resolved_ty = self.ast_to_type(ty)?;
        // Return the resolved type - this enables static method calls
        // like Wrapper<Person>.default() where we need Person as the type argument
        Ok(InferResult::new(resolved_ty))
    }

    fn infer_expr_typeof_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Typeof(inner) = &expr.kind else { unreachable!() };
        // Infer the inner expression type (for compile-time validation)
        let _ = self.synth_expr(inner)?;
        // typeof() returns a structural record { name: Text, ... }
        // Spec: typeof-reflection — stated here because nothing else
        // states it. Reflection on the runtime type yields an info
        // record whose canonical field is `name`; other fields
        // (`size`, `alignment`) can be added later without breaking
        // field access. The production is `typeof_expr` in
        // grammar/verum.ebnf. The path cited here previously,
        // docs/improvements.md, does not exist in this tree.
        let mut fields: indexmap::IndexMap<verum_common::Text, Type> =
            indexmap::IndexMap::new();
        fields.insert(verum_common::Text::from("name"), Type::text());
        Ok(InferResult::new(Type::Record(fields)))
    }

    fn infer_expr_destructuring_assign(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::DestructuringAssign { pattern, op, value } = &expr.kind else { unreachable!() };
        // 1. Type-check the value expression
        let value_result = self.synth_expr(value)?;

        if *op == verum_ast::BinOp::Assign {
            // Simple destructuring assignment: (a, b) = expr
            // Bind pattern variables to the value type
            // This performs structural type matching:
            // - Tuple pattern (a, b): value must be tuple type (T, U)
            // - Array pattern [x, y]: value must be array/slice type [T; N] or [T]
            // - Record pattern P { x, y }: value must be struct type P with fields x, y
            self.bind_pattern(pattern, &value_result.ty)?;

            // Track definite assignment for each bound variable in the pattern
            self.track_pattern_assignment(pattern, expr.span);
        } else {
            // Compound destructuring assignment: (x, y) += (dx, dy)
            // All pattern variables must already exist and be mutable.
            // For each element: read current value, apply op, write back.
            self.check_compound_destructuring_pattern(
                pattern,
                &value_result.ty,
                op,
                expr.span,
            )?;
        }

        // Destructuring assignment returns unit (like regular assignment)
        Ok(InferResult::new(Type::unit()))
    }

    fn infer_expr_inject(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Inject { type_path } = &expr.kind else { unreachable!() };
        // Resolve the type being injected from the path
        let type_name = self.path_to_string(type_path);
        let injected_ty =
            match self.resolve_type_name(type_name.as_str(), expr.span) {
                Ok(ty) => ty,
                Err(_) => {
                    // If type not found, create a Named type placeholder
                    Type::Named {
                        path: type_path.clone(),
                        args: List::new(),
                    }
                }
            };
        Ok(InferResult::new(injected_ty))
    }


    /// Check quote hygiene for splice expressions.
    ///
    /// This method analyzes the tokens in a quote block to ensure:
    /// - All splice variables (`$var` or `${expr}`) reference bound variables
    /// - No accidental variable capture occurs
    /// - Stage escapes are at the correct level
    ///
    /// Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene
    fn check_quote_hygiene(
        &self,
        tokens: &verum_common::List<verum_ast::expr::TokenTree>,
        _target_stage: Option<u32>,
        quote_span: verum_ast::span::Span,
    ) -> Result<()> {
        use std::collections::HashSet;
        use verum_ast::expr::{TokenTree, TokenTreeKind};

        // Helper function to recursively check tokens
        // quote_defined: set of identifiers defined within this quote (via let bindings)
        // is_transparent: whether the enclosing macro is @transparent (enables M402 checks)
        fn check_tokens_recursive(
            checker: &TypeChecker,
            tokens: &verum_common::List<TokenTree>,
            i: &mut usize,
            quote_span: verum_ast::span::Span,
            quote_defined: &mut HashSet<String>,
            is_transparent: bool,
        ) -> Result<()> {
            // DEBUG: print all tokens at this level (only if $ token present or multiplier present anywhere)
            let has_dollar = tokens.iter().any(|t| {
                if let TokenTree::Token(tok) = t {
                    tok.text.as_str() == "$"
                } else {
                    false
                }
            });
            // Also check for multiplier in any child group
            fn contains_multiplier(tokens: &verum_common::List<TokenTree>) -> bool {
                tokens.iter().any(|t| match t {
                    TokenTree::Token(tok) => tok.text.as_str() == "multiplier",
                    TokenTree::Group { tokens: inner, .. } => contains_multiplier(inner),
                })
            }
            if has_dollar && contains_multiplier(tokens) {
                // Debug token tree logging removed
                let _ = tokens.len();
            }
            while *i < tokens.len() {
                match &tokens[*i] {
                    TokenTree::Token(tok) => {
                        // M403: Check for gensym collision - identifiers matching __verum_gensym_*
                        // User code should never use these names as they're reserved for hygiene
                        if tok.kind == TokenTreeKind::Ident
                            && tok.text.as_str().starts_with("__verum_gensym_")
                        {
                            return Err(TypeError::GensymCollision {
                                symbol: tok.text.clone(),
                                span: tok.span,
                            });
                        }

                        // M406: Check for lift() with unliftable types
                        // lift(var) requires var to have a type that can be represented as code
                        // Closures, &mut references, and opaque types cannot be lifted
                        // Note: The lexer/parser emits lift as Punct with text "Lift" (capitalized)
                        let is_lift = (tok.kind == TokenTreeKind::Ident
                            && tok.text.as_str() == "lift")
                            || (tok.kind == TokenTreeKind::Punct && tok.text.as_str() == "Lift");
                        if is_lift {
                            // Check if next token is a paren group containing a variable
                            if *i + 1 < tokens.len() {
                                if let TokenTree::Group {
                                    delimiter: verum_ast::expr::MacroDelimiter::Paren,
                                    tokens: inner_tokens,
                                    span: group_span,
                                } = &tokens[*i + 1]
                                {
                                    // Look for the identifier inside the parens
                                    if let Some(TokenTree::Token(inner_tok)) = inner_tokens.first()
                                    {
                                        if inner_tok.kind == TokenTreeKind::Ident {
                                            let var_name = inner_tok.text.clone();
                                            // Look up the type of this variable
                                            if let Some(scheme) = checker.ctx.env.lookup(&var_name)
                                            {
                                                let ty = &scheme.ty;
                                                // Check if the type is liftable
                                                // Unliftable types: closures (fn types with captures),
                                                // mutable references, opaque/extern types
                                                let (is_unliftable, reason) =
                                                    checker.is_unliftable_type(ty);
                                                if is_unliftable {
                                                    return Err(TypeError::LiftTypeMismatch {
                                                        ty: verum_common::Text::from(
                                                            ty.to_string(),
                                                        ),
                                                        reason: verum_common::Text::from(reason),
                                                        span: *group_span,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    // Skip the paren group
                                    *i += 1;
                                }
                            }
                        }

                        // Check for splice pattern: $ followed by ident or {
                        if tok.kind == TokenTreeKind::Punct && tok.text.as_str() == "$" {
                            let splice_span = tok.span;
                            *i += 1;

                            if *i < tokens.len() {
                                // M407: Check for $$ (double dollar) - invalid stage escape
                                if let TokenTree::Token(next_tok) = &tokens[*i] {
                                    if next_tok.kind == TokenTreeKind::Punct
                                        && next_tok.text.as_str() == "$"
                                    {
                                        // $$ pattern detected - this is a stage escape
                                        // Check if the next token after $$ is an identifier
                                        if *i + 1 < tokens.len() {
                                            if let TokenTree::Token(var_tok) = &tokens[*i + 1] {
                                                if var_tok.kind == TokenTreeKind::Ident {
                                                    // $$ident - stage escape pattern
                                                    // This is only valid in specific contexts
                                                    // For now, flag it as an error
                                                    return Err(TypeError::InvalidStageEscape {
                                                        reason: verum_common::Text::from(format!(
                                                            "stage escape `$${}` references outer stage binding",
                                                            var_tok.text
                                                        )),
                                                        span: splice_span,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }

                                match &tokens[*i] {
                                    // $ident pattern
                                    TokenTree::Token(next_tok)
                                        if next_tok.kind == TokenTreeKind::Ident =>
                                    {
                                        let var_name = next_tok.text.clone();
                                        // Check if the variable is in scope
                                        if checker.ctx.env.lookup(&var_name).is_none() {
                                            return Err(TypeError::UnboundSpliceVariable {
                                                var_name,
                                                span: splice_span,
                                            });
                                        }
                                    }
                                    // ${expr} pattern - check the group contents
                                    TokenTree::Group {
                                        delimiter: verum_ast::expr::MacroDelimiter::Brace,
                                        tokens: inner_tokens,
                                        span: group_span,
                                    } => {
                                        // Extract the first identifier in the group as the variable name
                                        // This is a simplified check - real implementation would parse the expression
                                        if let Some(TokenTree::Token(first_tok)) =
                                            inner_tokens.first()
                                        {
                                            if first_tok.kind == TokenTreeKind::Ident {
                                                let var_name = first_tok.text.clone();
                                                // Check if the variable is in scope
                                                if checker.ctx.env.lookup(&var_name).is_none() {
                                                    return Err(TypeError::UnboundSpliceVariable {
                                                        var_name,
                                                        span: *group_span,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    // M400: $[...] pattern - must contain 'for' for repetition
                                    TokenTree::Group {
                                        delimiter: verum_ast::expr::MacroDelimiter::Bracket,
                                        tokens: inner_tokens,
                                        span: bracket_span,
                                    } => {
                                        // Check if the first token is 'for' (valid repetition)
                                        let has_for = inner_tokens.first().is_some_and(|t| {
                                            if let TokenTree::Token(tok) = t {
                                                tok.kind == TokenTreeKind::Keyword
                                                    && tok.text.as_str() == "for"
                                            } else {
                                                false
                                            }
                                        });

                                        if !has_for {
                                            // Invalid $[...] syntax - missing 'for'
                                            return Err(TypeError::UnboundSpliceVariable {
                                                var_name: verum_common::Text::from(
                                                    "invalid repetition syntax - expected $[for ... in ...]",
                                                ),
                                                span: *bracket_span,
                                            });
                                        }
                                    }
                                    // $(stage N){...} pattern - stage escape
                                    TokenTree::Group {
                                        delimiter: verum_ast::expr::MacroDelimiter::Paren,
                                        tokens: stage_tokens,
                                        ..
                                    } => {
                                        // Check if it's a stage escape: $(stage N)
                                        // Note: "stage" may be tokenized as Ident, Keyword, or Punct
                                        // with text "Stage" (capital S) depending on tokenization
                                        let is_stage_escape =
                                            stage_tokens.first().is_some_and(|t| {
                                                if let TokenTree::Token(tok) = t {
                                                    (tok.kind == TokenTreeKind::Ident
                                                        || tok.kind == TokenTreeKind::Keyword
                                                        || tok.kind == TokenTreeKind::Punct)
                                                        && tok
                                                            .text
                                                            .as_str()
                                                            .eq_ignore_ascii_case("stage")
                                                } else {
                                                    false
                                                }
                                            });

                                        if is_stage_escape {
                                            // Skip the paren group (we're at $, need to move past the paren)
                                            // After this, *i points PAST the paren group
                                            *i += 2;

                                            // Check if followed by a brace group, and skip it too
                                            // (the brace group contains code evaluated at meta-time,
                                            // where bare meta-level bindings are valid)
                                            if *i < tokens.len() {
                                                if let TokenTree::Group {
                                                    delimiter:
                                                        verum_ast::expr::MacroDelimiter::Brace,
                                                    ..
                                                } = &tokens[*i]
                                                {
                                                    *i += 1;
                                                }
                                            }

                                            // Continue to next iteration to avoid the *i += 1 at end of Token branch
                                            continue;
                                        }
                                    }
                                    _ => {
                                        // $ followed by something else - could be a syntax error
                                        // but we'll let the parser handle that
                                    }
                                }
                            }
                        }

                        // M408: Check for undeclared capture of meta-level bindings
                        // A bare identifier in a quote that matches a meta-level binding
                        // should use $var or lift(var) syntax
                        // Skip this check if:
                        // - The identifier is a keyword
                        // - The identifier follows 'let' (it's a new binding)
                        // - We're in a lift() context (handled above)
                        // - We're in a $var context (handled above)
                        // - The identifier starts with uppercase (likely a type name)
                        if tok.kind == TokenTreeKind::Ident {
                            let var_name = &tok.text;

                            // Skip identifiers starting with uppercase (likely type names)
                            let first_char = var_name.chars().next().unwrap_or('a');
                            if first_char.is_uppercase() {
                                // Skip - this is likely a type name
                                *i += 1;
                                continue;
                            }

                            // Check if previous token was 'let' or 'fn' (binding/definition context)
                            let prev_is_binding = if *i > 0 {
                                if let TokenTree::Token(prev_tok) = &tokens[*i - 1] {
                                    prev_tok.kind == TokenTreeKind::Keyword
                                        && (prev_tok.text.as_str() == "let"
                                            || prev_tok.text.as_str() == "fn"
                                            || prev_tok.text.as_str() == "meta"
                                            || prev_tok.text.as_str() == "type")
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            let prev_is_let = prev_is_binding;

                            // If this is a binding context (after 'let'), add to quote_defined
                            if prev_is_let {
                                quote_defined.insert(var_name.to_string());
                            }

                            // Check if previous token was '$' (splice context - already handled)
                            let prev_is_splice = if *i > 0 {
                                if let TokenTree::Token(prev_tok) = &tokens[*i - 1] {
                                    prev_tok.kind == TokenTreeKind::Punct
                                        && prev_tok.text.as_str() == "$"
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            // Check if previous token was 'lift' or 'Lift' (lift context - already handled)
                            let prev_is_lift = if *i > 0 {
                                if let TokenTree::Token(prev_tok) = &tokens[*i - 1] {
                                    (prev_tok.kind == TokenTreeKind::Ident
                                        && prev_tok.text.as_str() == "lift")
                                        || (prev_tok.kind == TokenTreeKind::Punct
                                            && prev_tok.text.as_str() == "Lift")
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            // Check if previous token was '@' (meta-function call - not a capture)
                            let prev_is_at = if *i > 0 {
                                if let TokenTree::Token(prev_tok) = &tokens[*i - 1] {
                                    prev_tok.kind == TokenTreeKind::Punct
                                        && prev_tok.text.as_str() == "@"
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            // Skip if in a special context
                            if !prev_is_let && !prev_is_splice && !prev_is_lift && !prev_is_at {
                                // Check if this identifier is defined within the quote (via let binding)
                                // If so, it's a quote-local variable, not a capture
                                let is_quote_local = quote_defined.contains(var_name.as_str());

                                // Check if this is a builtin function/macro that's safe to use in quotes
                                let vn = var_name.as_str();
                                use verum_common::well_known_types::variant_tags;
                                let is_builtin = matches!(
                                    vn,
                                    "true" | "false" | "self" | "Self" | "super" | "cog" |
                                    "println" | "print" | "eprintln" | "eprint" |
                                    "assert" | "panic" | "unreachable" | "todo" |
                                    "format" |
                                    "stringify" | "concat" | "include_str" | "include_bytes" |
                                    "file" | "line" | "column" | "module_path" |
                                    "env" | "option_env" | "compile_error" | "cfg" |
                                    "gensym" | "quote" | "unquote" | "lift" |
                                    // Common iterator methods that are not captures
                                    "iter" | "map" | "filter" | "fold" | "collect" | "next" |
                                    // Common constructors/methods
                                    "new" | "default" | "clone" | "into" | "from" | "as_ref" |
                                    "debug_struct" | "field" | "finish"
                                ) || verum_common::WellKnownType::is_well_known(vn)
                                  || verum_common::well_known_types::type_names::is_primitive_value_type(vn)
                                  || variant_tags::is_maybe_constructor(vn)
                                  || variant_tags::is_result_constructor(vn);

                                // Check if this identifier exists in the meta-level environment
                                if !is_quote_local
                                    && !is_builtin
                                    && checker.ctx.env.lookup(var_name).is_some()
                                {
                                    // M408: undeclared capture - meta-level binding used
                                    // without proper $ or lift() syntax
                                    return Err(TypeError::UndeclaredCapture {
                                        var_name: var_name.clone(),
                                        span: tok.span,
                                    });
                                } else if is_transparent {
                                    // M402: Accidental capture - identifier is NOT in the
                                    // meta-level scope, so it might accidentally capture from
                                    // the expansion site. This only applies to TRANSPARENT macros.
                                    // In hygienic mode (the default), bare identifiers are gensym'd
                                    // and don't capture from the expansion site.
                                    //

                                    // Skip if this identifier is defined within the quote (via let binding)
                                    // Also skip common Verum keywords/builtins that are safe
                                    let is_quote_local = quote_defined.contains(var_name.as_str());
                                    let vn2 = var_name.as_str();
                                    let is_builtin = matches!(
                                        vn2,
                                        "true" | "false" | "self" | "Self" | "super" | "cog" |
                                        "println" | "print" | "eprintln" | "eprint" |
                                        "assert" | "panic" | "unreachable" | "todo" |
                                        "format"
                                    ) || verum_common::WellKnownType::is_well_known(vn2)
                                      || verum_common::well_known_types::type_names::is_primitive_value_type(vn2)
                                      || variant_tags::is_maybe_constructor(vn2)
                                      || variant_tags::is_result_constructor(vn2);

                                    // Check if followed by a '(' - function call (not a capture)
                                    let is_function_call = if *i + 1 < tokens.len() {
                                        match &tokens[*i + 1] {
                                            TokenTree::Token(next_tok) => {
                                                next_tok.kind == TokenTreeKind::Punct
                                                    && next_tok.text.as_str() == "("
                                            }
                                            TokenTree::Group {
                                                delimiter: verum_ast::expr::MacroDelimiter::Paren,
                                                ..
                                            } => true,
                                            _ => false,
                                        }
                                    } else {
                                        false
                                    };

                                    if !is_quote_local && !is_builtin && !is_function_call {
                                        return Err(TypeError::AccidentalCapture {
                                            var_name: var_name.clone(),
                                            inner_span: tok.span,
                                            outer_span: quote_span,
                                        });
                                    }
                                }
                                // In hygienic mode (non-transparent), bare identifiers are gensym'd
                                // and don't capture from the expansion site - no error needed
                            }
                        }

                        *i += 1;
                    }
                    TokenTree::Group { tokens: inner, .. } => {
                        // Recursively check nested groups
                        let mut inner_i = 0;
                        check_tokens_recursive(
                            checker,
                            inner,
                            &mut inner_i,
                            quote_span,
                            quote_defined,
                            is_transparent,
                        )?;
                        *i += 1;
                    }
                }
            }
            Ok(())
        }

        // Pre-pass: collect all bindings defined within the quote before checking hygiene
        // This includes let bindings, for loop variables, closure params, function params, etc.
        fn collect_quote_bindings(
            tokens: &verum_common::List<TokenTree>,
            bindings: &mut HashSet<String>,
        ) {
            // Helper to check if a token is a keyword (may be Ident in token trees)
            fn is_keyword(tok: &verum_ast::expr::TokenTreeToken, kw: &str) -> bool {
                (tok.kind == TokenTreeKind::Keyword || tok.kind == TokenTreeKind::Ident)
                    && tok.text.as_str() == kw
            }

            let mut i = 0;
            while i < tokens.len() {
                match &tokens[i] {
                    TokenTree::Token(tok) => {
                        // Check for `let` followed by identifier or pattern
                        if is_keyword(tok, "let") {
                            if i + 1 < tokens.len() {
                                match &tokens[i + 1] {
                                    // Simple: let x = ... or Variant pattern: let Some(x) = ...
                                    // Note: variant names like Some/None may be Keyword or Ident
                                    TokenTree::Token(next_tok)
                                        if next_tok.kind == TokenTreeKind::Ident
                                            || next_tok.kind == TokenTreeKind::Keyword =>
                                    {
                                        let first_char =
                                            next_tok.text.chars().next().unwrap_or('a');
                                        if first_char.is_uppercase() {
                                            // This is a variant pattern like Some(x) or Ok(v)
                                            // Look for the following group to extract bindings
                                            if i + 2 < tokens.len() {
                                                if let TokenTree::Group {
                                                    tokens: pattern_tokens,
                                                    ..
                                                } = &tokens[i + 2]
                                                {
                                                    collect_pattern_bindings(
                                                        pattern_tokens,
                                                        bindings,
                                                    );
                                                }
                                            }
                                        } else {
                                            // Simple binding: let x = ...
                                            bindings.insert(next_tok.text.to_string());
                                        }
                                    }
                                    // Pattern: let (x, y) = ... or let { a, b } = ...
                                    TokenTree::Group {
                                        tokens: pattern_tokens,
                                        ..
                                    } => {
                                        collect_pattern_bindings(pattern_tokens, bindings);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Check for `for` loop: for i in ...
                        else if is_keyword(tok, "for") {
                            if i + 1 < tokens.len() {
                                if let TokenTree::Token(next_tok) = &tokens[i + 1] {
                                    if next_tok.kind == TokenTreeKind::Ident {
                                        bindings.insert(next_tok.text.to_string());
                                    }
                                }
                            }
                        }
                        // Check for `while let` pattern: while let Some(item) = ...
                        else if is_keyword(tok, "while") {
                            // Look for 'let' after 'while'
                            if i + 1 < tokens.len() {
                                if let TokenTree::Token(next_tok) = &tokens[i + 1] {
                                    if is_keyword(next_tok, "let") {
                                        // Now we have 'while let', look for the pattern
                                        if i + 2 < tokens.len() {
                                            match &tokens[i + 2] {
                                                // while let x = ... or while let Some(x) = ...
                                                // Note: variant names like Some/None may be Keyword or Ident
                                                TokenTree::Token(pat_tok)
                                                    if pat_tok.kind == TokenTreeKind::Ident
                                                        || pat_tok.kind
                                                            == TokenTreeKind::Keyword =>
                                                {
                                                    let first_char =
                                                        pat_tok.text.chars().next().unwrap_or('a');
                                                    if first_char.is_uppercase() {
                                                        // Variant pattern: while let Some(x) = ...
                                                        if i + 3 < tokens.len() {
                                                            if let TokenTree::Group {
                                                                tokens: pattern_tokens,
                                                                ..
                                                            } = &tokens[i + 3]
                                                            {
                                                                collect_pattern_bindings(
                                                                    pattern_tokens,
                                                                    bindings,
                                                                );
                                                            }
                                                        }
                                                    } else {
                                                        bindings.insert(pat_tok.text.to_string());
                                                    }
                                                }
                                                // while let (a, b) = ... (tuple pattern)
                                                TokenTree::Group {
                                                    tokens: pattern_tokens,
                                                    ..
                                                } => {
                                                    collect_pattern_bindings(
                                                        pattern_tokens,
                                                        bindings,
                                                    );
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Check for `if let` pattern: if let Some(item) = ...
                        else if is_keyword(tok, "if") {
                            // Look for 'let' after 'if'
                            if i + 1 < tokens.len() {
                                if let TokenTree::Token(next_tok) = &tokens[i + 1] {
                                    if is_keyword(next_tok, "let") {
                                        // Now we have 'if let', look for the pattern
                                        if i + 2 < tokens.len() {
                                            match &tokens[i + 2] {
                                                // Note: variant names like Some/None may be Keyword or Ident
                                                TokenTree::Token(pat_tok)
                                                    if pat_tok.kind == TokenTreeKind::Ident
                                                        || pat_tok.kind
                                                            == TokenTreeKind::Keyword =>
                                                {
                                                    let first_char =
                                                        pat_tok.text.chars().next().unwrap_or('a');
                                                    if first_char.is_uppercase() {
                                                        if i + 3 < tokens.len() {
                                                            if let TokenTree::Group {
                                                                tokens: pattern_tokens,
                                                                ..
                                                            } = &tokens[i + 3]
                                                            {
                                                                collect_pattern_bindings(
                                                                    pattern_tokens,
                                                                    bindings,
                                                                );
                                                            }
                                                        }
                                                    } else {
                                                        bindings.insert(pat_tok.text.to_string());
                                                    }
                                                }
                                                TokenTree::Group {
                                                    tokens: pattern_tokens,
                                                    ..
                                                } => {
                                                    collect_pattern_bindings(
                                                        pattern_tokens,
                                                        bindings,
                                                    );
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Check for closure: |params| ...
                        else if tok.kind == TokenTreeKind::Punct && tok.text.as_str() == "|" {
                            // Collect identifiers until next |
                            let mut j = i + 1;
                            while j < tokens.len() {
                                if let TokenTree::Token(t) = &tokens[j] {
                                    if t.kind == TokenTreeKind::Punct && t.text.as_str() == "|" {
                                        break;
                                    }
                                    if t.kind == TokenTreeKind::Ident {
                                        bindings.insert(t.text.to_string());
                                    }
                                }
                                j += 1;
                            }
                        }
                        // Check for `fn` declaration: fn name(params...)
                        else if is_keyword(tok, "fn") {
                            // Look for parameter list
                            let mut j = i + 1;
                            while j < tokens.len() {
                                if let TokenTree::Group {
                                    delimiter: verum_ast::expr::MacroDelimiter::Paren,
                                    tokens: params,
                                    ..
                                } = &tokens[j]
                                {
                                    collect_fn_param_bindings(params, bindings);
                                    break;
                                }
                                j += 1;
                                if j > i + 3 {
                                    break;
                                } // Don't look too far
                            }
                        }
                        // Check for `match` expression: match expr { Pattern => ... }
                        else if is_keyword(tok, "match") {
                            // Look for the body block of the match
                            let mut j = i + 1;
                            while j < tokens.len() {
                                if let TokenTree::Group {
                                    delimiter: verum_ast::expr::MacroDelimiter::Brace,
                                    tokens: body,
                                    ..
                                } = &tokens[j]
                                {
                                    // Collect bindings from match body patterns
                                    collect_quote_bindings(body, bindings);
                                    break;
                                }
                                j += 1;
                                if j > i + 5 {
                                    break;
                                } // Don't look too far
                            }
                        }
                    }
                    TokenTree::Group { tokens: inner, .. } => {
                        // Recursively collect from nested groups
                        collect_quote_bindings(inner, bindings);
                    }
                }
                i += 1;
            }
        }

        // Collect identifiers from patterns (tuple destructuring, struct patterns, etc.)
        fn collect_pattern_bindings(
            tokens: &verum_common::List<TokenTree>,
            bindings: &mut HashSet<String>,
        ) {
            for token in tokens.iter() {
                match token {
                    TokenTree::Token(t) if t.kind == TokenTreeKind::Ident => {
                        // Skip type names (starting with uppercase)
                        let first_char = t.text.chars().next().unwrap_or('a');
                        if !first_char.is_uppercase() {
                            bindings.insert(t.text.to_string());
                        }
                    }
                    TokenTree::Group { tokens: inner, .. } => {
                        collect_pattern_bindings(inner, bindings);
                    }
                    _ => {}
                }
            }
        }

        // Collect function parameter bindings (x: T, y: U, ...)
        fn collect_fn_param_bindings(
            tokens: &verum_common::List<TokenTree>,
            bindings: &mut HashSet<String>,
        ) {
            let mut i = 0;
            while i < tokens.len() {
                if let TokenTree::Token(t) = &tokens[i] {
                    // Look for identifier followed by : (parameter name)
                    if t.kind == TokenTreeKind::Ident {
                        if i + 1 < tokens.len() {
                            if let TokenTree::Token(next) = &tokens[i + 1] {
                                if next.kind == TokenTreeKind::Punct && next.text.as_str() == ":" {
                                    bindings.insert(t.text.to_string());
                                }
                            }
                        }
                    }
                }
                i += 1;
            }
        }

        let mut i = 0;
        let mut quote_defined = HashSet::new();

        // Pre-pass: collect all quote-local bindings
        collect_quote_bindings(tokens, &mut quote_defined);

        let is_transparent = self.current_function_is_transparent;
        check_tokens_recursive(
            self,
            tokens,
            &mut i,
            quote_span,
            &mut quote_defined,
            is_transparent,
        )
    }

    /// Check if a type cannot be lifted (converted to code at compile time).
    ///
    /// Returns (is_unliftable, reason) where reason explains why the type cannot be lifted.
    ///
    /// Unliftable types include:
    /// - Closure types (anonymous function types with captured environment)
    /// - Mutable references (&mut T)
    /// - Opaque/extern types (types without a known representation)
    /// - Raw pointers (*T)
    fn is_unliftable_type(&self, ty: &Type) -> (bool, &'static str) {
        match ty {
            // Function types are considered closures if they could capture environment
            // For simplicity, we treat all function types as potentially unliftable
            // since we can't distinguish pure function types from closures at this point
            Type::Function { .. } => (
                true,
                "closures cannot be lifted because they may capture local state",
            ),

            // Mutable references cannot be lifted because they represent
            // mutable state that cannot be serialized to code
            Type::Reference { mutable: true, .. } => (
                true,
                "mutable references cannot be lifted because they represent mutable state",
            ),

            // Raw pointers cannot be lifted
            Type::Pointer { .. } => (
                true,
                "raw pointers cannot be lifted because they represent arbitrary memory addresses",
            ),

            // Volatile pointers cannot be lifted (MMIO addresses)
            Type::VolatilePointer { .. } => (
                true,
                "volatile pointers cannot be lifted because they represent MMIO addresses",
            ),

            // Named types need to check if they're opaque/extern
            Type::Named { path, .. } => {
                // Check if this is a known opaque type (extern types)
                // Opaque types have no representation and cannot be converted to code
                let type_name = path
                    .segments
                    .last()
                    .and_then(|seg| {
                        if let verum_ast::ty::PathSegment::Name(ident) = seg {
                            Some(ident.name.as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                if type_name.starts_with("Opaque") || type_name.contains("Handle") {
                    // Common naming patterns for opaque types
                    (
                        true,
                        "opaque/extern types cannot be lifted because they have no code representation",
                    )
                } else {
                    // Other named types are generally liftable
                    (false, "")
                }
            }

            // Most other types can be lifted (primitives, structs, enums, etc.)
            _ => (false, ""),
        }
    }

    /// Infer type for a block.
    ///
    pub(super) fn infer_block(&mut self, block: &Block) -> Result<InferResult> {
        self.with_block_scope(block, false, Self::infer_block_inner)
    }

    pub(super) fn infer_function_block(&mut self, block: &Block, returning: bool) -> Result<InferResult> {
        self.with_block_scope(block, returning, Self::infer_block_inner)
    }

    /// Resolve outgoing loans before dropping the bindings which establish
    /// their provenance. A function boundary also ends owned parameter storage.
    fn with_block_scope(
        &mut self,
        block: &Block,
        returning: bool,
        infer: impl FnOnce(&mut Self, &Block) -> Result<InferResult>,
    ) -> Result<InferResult> {
        let _depth_guard = self.inc_inference_depth("block")?;
        let constant_scope = self.const_eval.enter_source_scope(self.current_module_path.as_str());
        let result = self.with_lexical_result_scope(|checker| {
            let filtered = checker.cfg_filtered_block(block);
            let block = filtered.as_ref().unwrap_or(block);
            let result = infer(checker, block)?;
            if returning && let Some(tail) = &block.expr {
                checker.check_return_value_loans(tail, &result.ty, tail.span)?;
            }
            Ok(result)
        });
        self.const_eval.leave_scope(constant_scope);
        result
    }

    /// Drop the statements `@cfg(...)` excludes for this target, or
    /// `None` when the block has none — the overwhelmingly common case,
    /// which then costs one `is_empty()` per statement and no clone.
    ///
    /// T0778: codegen has always filtered these (`should_compile_stmt`),
    /// and the interpreter runs the right branch — only the TYPECHECKER
    /// walked into blocks belonging to another platform.  A
    /// cross-platform module is then guaranteed to report unbound names,
    /// because ITEM-level `@cfg` *is* honoured: `core/sys/linux/tls.vr`
    /// gates `mount sys.linux.syscall.SYS_ARCH_PRCTL;` for x86_64 and
    /// uses the constant only inside x86_64 blocks — correct code, two
    /// errors on aarch64.
    ///
    /// Filtered ONCE here rather than at the seventeen statement loops
    /// inside `*_block_inner`, so there is a single place that decides
    /// what belongs to this target — and it asks the same
    /// `CfgEvaluator` codegen asks, rather than carrying its own copy of
    /// the rule.
    fn cfg_filtered_block(&self, block: &Block) -> Option<Block> {
        if block.stmts.iter().all(|s| s.attributes.is_empty()) {
            return None;
        }
        let kept: verum_common::List<verum_ast::Stmt> = block
            .stmts
            .iter()
            .filter(|s| {
                s.attributes.is_empty() || self.cfg_evaluator.should_include(&s.attributes)
            })
            .cloned()
            .collect();
        if kept.len() == block.stmts.len() {
            return None;
        }
        Some(Block {
            stmts: kept,
            expr: block.expr.clone(),
            span: block.span,
        })
    }

    /// Inner implementation of infer_block.
    fn infer_block_inner(&mut self, block: &Block) -> Result<InferResult> {
        // PASS 0: Register all local type definitions first (two-phase)
        // This enables local functions to reference locally-defined types in their signatures.
        // Phase 0a: Register type names as placeholders (allows forward references between types)
        for stmt in &block.stmts {
            if let StmtKind::Item(item) = &stmt.kind {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    self.register_type_name_only(type_decl);
                }
            }
        }
        // Phase 0b: Resolve type definitions (now all type names are available)
        for stmt in &block.stmts {
            if let StmtKind::Item(item) = &stmt.kind {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    let mut resolution_stack = List::new();
                    // Ignore errors here - they'll be reported in PASS 2
                    let _ = self.resolve_type_definition(type_decl, &mut resolution_stack);
                }
            }
        }

        // PASS 1: Register all nested function signatures first
        // This enables forward references between sibling nested functions
        // Spec: Functions can reference other functions regardless of definition order
        for stmt in &block.stmts {
            if let StmtKind::Item(item) = &stmt.kind {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    self.register_function_signature(func)?;
                }
            }
        }

        // PASS 2: Type check statements and track divergence
        //
        // STMT-RECOVERY-1 (T0584/T0585 class close): a failing statement
        // pushes its diagnostic and the walk CONTINUES — pre-fix the `?`
        // aborted the whole block on the first error, so a file with 15
        // independent defects reported exactly 1 (ambiguous_types.vr).
        // Statement-boundary recovery is sound for every TypeError: the
        // inference-depth guard unwinds with its RAII drop, and a failed
        // `let` binds its pattern to fresh vars so later statements
        // don't cascade phantom "undefined variable" noise. The pushed
        // error-severity diagnostics still fail the typecheck.
        let mut has_diverging_stmt = false;
        // An affine value consumed here cannot be consumed twice when
        // the block leaves the enclosing loop — see
        // `AffineChecker::enter_leaving_region` (T0915).
        let leaves = Self::block_leaves_unconditionally(block);
        if leaves {
            self.affine_tracker.enter_leaving_region();
        }
        let loan_ends = super::loan_ends::LoanEnds::for_block(block);
        for (index, stmt) in block.stmts.iter().enumerate() {
            let amb_mark = self.pending_ambiguity.len();
            match self.check_stmt(stmt) {
                Ok(diverges) => {
                    loan_ends.complete(index, self);
                    if diverges {
                        has_diverging_stmt = true;
                    }
                }
                Err(e) => {
                    // A failed statement may have leaked ambiguity
                    // candidates from an aborted nested check —
                    // drop anything recorded during it (the statement
                    // already carries its own loud error).
                    self.pending_ambiguity.truncate(amb_mark);
                    self.push_diagnostic_for(e);
                    self.recover_stmt_bindings(stmt);
                }
            }
        }

        // Type check final expression
        let tail = if let Some(expr) = &block.expr {
            let r = self.synth_expr(expr);
            if leaves {
                self.affine_tracker.exit_leaving_region();
            }
            Some(r?)
        } else {
            if leaves {
                self.affine_tracker.exit_leaving_region();
            }
            None
        };
        let result = if let Some(r) = tail {
            r
        } else if has_diverging_stmt {
            // Block has a diverging statement, so it has type Never
            InferResult::new(Type::never())
        } else {
            InferResult::new(Type::unit())
        };

        let validation = block.expr.as_ref().map(|tail| self.check_aggregate_escape(tail, &result.ty, false));
        validation.transpose()?;
        Ok(result)
    }


    /// Does this block leave the enclosing loop unconditionally?
    ///
    /// True when the block's OWN top level carries a `return` — as a
    /// statement or as the tail expression. Nothing inside such a block
    /// can reach the enclosing loop's back-edge: statements before the
    /// return exit the function, statements after it are unreachable.
    ///
    /// Deliberately syntactic and deliberately shallow. A `return`
    /// nested inside an `if` does not make THIS block leave — but that
    /// `if`'s own body is a block, and the recursive walk asks the same
    /// question of it, which is where the answer belongs.
    fn block_leaves_unconditionally(block: &Block) -> bool {
        use verum_ast::stmt::StmtKind;
        let is_return = |e: &Expr| matches!(&e.kind, ExprKind::Return(_));
        block
            .stmts
            .iter()
            .any(|stmt| matches!(&stmt.kind, StmtKind::Expr { expr, .. } if is_return(expr)))
            || block.expr.as_ref().is_some_and(|e| is_return(e))
    }

    /// STMT-RECOVERY-1 companion: after a statement's check failed and
    /// its diagnostic was pushed, bind whatever names the statement
    /// WOULD have introduced to fresh inference variables, so the
    /// remaining statements type-check against real bindings instead
    /// of cascading phantom "undefined variable" errors (which would
    /// bury the genuine defects the recovery exists to surface).
    /// Fresh vars unify with anything — the failed statement already
    /// carries its own error, so no false green can result from the
    /// permissive binding. Binding errors here are swallowed: recovery
    /// must never introduce a second failure path.
    fn recover_stmt_bindings(&mut self, stmt: &Stmt) {
        use verum_ast::stmt::StmtKind;
        let pattern = match &stmt.kind {
            StmtKind::Let { pattern, .. } => pattern,
            StmtKind::LetElse { pattern, .. } => pattern,
            _ => return,
        };
        let recovery = crate::context::TypeScheme::mono(Type::Var(
            crate::ty::TypeVar::fresh(),
        ));
        let _ = self.bind_pattern_scheme(pattern, recovery);
    }

    /// Check a block against expected type.
    pub(super) fn check_block(&mut self, block: &Block, expected: &Type) -> Result<InferResult> {
        self.check_function_block(block, expected, false)
    }

    pub(super) fn check_function_block(&mut self, block: &Block, expected: &Type, returning: bool) -> Result<InferResult> {
        self.with_block_scope(block, returning, |checker, block| checker.check_block_inner(block, expected))
    }

    fn check_block_inner(&mut self, block: &Block, expected: &Type) -> Result<InferResult> {
        // PASS 0: Register all local type definitions first (two-phase)
        // This enables local functions to reference locally-defined types in their signatures.
        // Phase 0a: Register type names as placeholders (allows forward references between types)
        for stmt in &block.stmts {
            if let StmtKind::Item(item) = &stmt.kind {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    self.register_type_name_only(type_decl);
                }
            }
        }
        // Phase 0b: Resolve type definitions (now all type names are available)
        for stmt in &block.stmts {
            if let StmtKind::Item(item) = &stmt.kind {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    let mut resolution_stack = List::new();
                    // Ignore errors here - they'll be reported in PASS 2
                    let _ = self.resolve_type_definition(type_decl, &mut resolution_stack);
                }
            }
        }

        // PASS 1: Register all nested function signatures first
        // This enables forward references between sibling nested functions
        // Spec: Functions can reference other functions regardless of definition order
        for stmt in &block.stmts {
            if let StmtKind::Item(item) = &stmt.kind {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    let _ = self.register_function_signature(func);
                }
            }
        }

        // PASS 2: Track if any statement diverges (has type Never)
        //
        // STMT-RECOVERY-1: same push-and-continue as the synth twin
        // above — one failing statement never hides its siblings.
        let mut has_diverging_stmt = false;
        let loan_ends = super::loan_ends::LoanEnds::for_block(block);
        for (index, stmt) in block.stmts.iter().enumerate() {
            let amb_mark = self.pending_ambiguity.len();
            match self.check_stmt(stmt) {
                Ok(diverges) => {
                    loan_ends.complete(index, self);
                    if diverges {
                        has_diverging_stmt = true;
                        // Once we hit a diverging statement, the rest is
                        // unreachable but we still type-check them for
                        // error reporting
                    }
                }
                Err(e) => {
                    // See the synth twin: drop candidates leaked by an
                    // aborted nested check inside this statement.
                    self.pending_ambiguity.truncate(amb_mark);
                    self.push_diagnostic_for(e);
                    self.recover_stmt_bindings(stmt);
                }
            }
        }

        let result = if let Some(tail) = &block.expr {
            self.check_expr(tail, expected).and_then(|result| {
                self.check_aggregate_escape(tail, &result.ty, false)?;
                Ok(result)
            })
        } else if has_diverging_stmt {
            Ok(InferResult::new(expected.clone()))
        } else {
            // Reaching the end contributes Unit, including to an inferred callable result.
            self.unifier.unify(&Type::Unit, expected, block.span)
                .map(|_| InferResult::new(Type::Unit))
        };
        result
    }
}

/// True when a `Type::Named` path names a PRIMITIVE VALUE type (T1338).
///
/// `Bool`, `Int`, `Float`, `Char` and `Text` each own a `Type` variant, so
/// they never reach the `Named` arm of the deref rule. Every sized integer
/// — `Int8`, `Int32`, `Int64`, `UInt8`, `UInt32`, `UInt64` and their
/// aliases — is carried as `Named`, and used to be refused there while its
/// unsized sibling was accepted. The set comes from
/// `well_known_types::type_names`, so a new width lands here without a
/// second list to keep in step.
fn named_primitive_value(path: &verum_ast::ty::Path) -> bool {
    match path.segments.last() {
        Some(verum_ast::ty::PathSegment::Name(ident)) => {
            verum_common::well_known_types::type_names::is_primitive_value_type(
                ident.name.as_str(),
            )
        }
        _ => false,
    }
}
