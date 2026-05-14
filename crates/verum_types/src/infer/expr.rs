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
use crate::ty::{Type, TypeVar};
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

impl TypeChecker {
    pub fn synth_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        self.metrics.synth_count += 1;

        // Use iterative inference for expressions that might be deeply nested
        // to avoid stack overflow. Falls back to recursive inference for
        // complex expressions that need full context.
        if Self::should_use_iterative_inference(expr) {
            self.synth_expr_iterative(expr)
        } else {
            self.infer_expr(expr, InferMode::Synth)
        }
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
        self.check_expr_inner(expr, expected)
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
            TryBlock(inner_block) => {
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
                    let wrapped_type =
                        self.wrap_in_success_type(&resolved_block_ty, expected, error_type);
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG check_expr_inner TryBlock] wrapping: wrapped_type={}, expected={}", wrapped_type, expected);
                    self.unifier.unify(&wrapped_type, expected, expr.span)?;
                    Ok(InferResult::new(expected.clone()))
                }
            }

            // Record literal with expected type - enables bidirectional inference for generics
            // e.g., `Box { value: 42 }` with expected type `Box<Int>` should infer T = Int
            Record { .. } => self.check_record_expr(expr, expected),

            // Array with expected type - enables bidirectional inference for element types
            // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .5.1 - Meta Parameters
            // When expected is [T; N], check each element against T instead of synthesizing
            // This enables: `let bytes: [Byte; 8] = [0x01, 0x02, ...]` with coercion
            Array(arr_expr) => {
                // Extract element type from expected type
                if let Type::Array {
                    element: expected_elem,
                    size: expected_size,
                } = expected
                {
                    match arr_expr {
                        verum_ast::expr::ArrayExpr::List(exprs) => {
                            // Check size matches if both are known
                            if let Some(expected_n) = expected_size {
                                if *expected_n != exprs.len() {
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

            // Bare variant path with expected variant type (e.g., `None` when return type is `Maybe<Int>`):
            // When the expected type is a variant and the expression is a bare path matching
            // one of its unit variants, resolve to the expected type directly.
            // This prevents collisions with other types that have the same variant name.
            Path(path) if path.segments.len() == 1 => {
                let name = match path.segments.first() {
                    Some(verum_ast::ty::PathSegment::Name(id)) => id.name.as_str(),
                    _ => "",
                };
                // First resolve the expected type through any type variables
                let resolved_expected = self.unifier.apply(expected);
                let expanded_expected = self.expand_generic_to_variant(&resolved_expected);
                if let Type::Variant(ref variants) = expanded_expected {
                    if let Some(payload_ty) = variants.get(name) {
                        if matches!(payload_ty, Type::Unit) {
                            // Unit variant (e.g., None) matches expected variant type
                            return Ok(InferResult::new(expected.clone()));
                        }
                    }
                }
                // Not a matching unit variant — fall through to synth_and_check
                self.synth_and_check(expr, expected)
            }

            // Try/recover with expected type: propagate expected type to both try block
            // and recover arm bodies. This enables bidirectional resolution for `None`/`Some`
            // in try { Some(r) } recover { ... => None } with expected Maybe<T>.
            TryRecover { try_block, recover } => {
                let error_type = self.extract_try_block_error_type(try_block)?;
                self.try_recover_depth += 1;
                let saved_return_type = self.current_function_return_type.clone();
                let try_ok_var = Type::Var(crate::ty::TypeVar::fresh());
                self.current_function_return_type =
                    verum_common::Maybe::Some(Type::result(try_ok_var.clone(), error_type.clone()));
                self.check_expr(try_block, expected)?;
                self.current_function_return_type = saved_return_type;
                self.try_recover_depth -= 1;

                // Check recover arms against expected type
                match recover {
                    verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            self.bind_pattern(&arm.pattern, &error_type)?;
                            if let Some(ref guard) = arm.guard {
                                self.check_expr(guard, &Type::bool())?;
                            }
                            self.check_expr(&arm.body, expected)?;
                        }
                    }
                    verum_ast::expr::RecoverBody::Closure { param, body, .. } => {
                        // Bind the error parameter pattern
                        self.bind_pattern(&param.pattern, &error_type)?;
                        self.check_expr(body, expected)?;
                    }
                }
                Ok(InferResult::new(expected.clone()))
            }

            // Match expression with expected type: propagate expected type to each arm body.
            // This enables bidirectional type checking for bare variant names like `None`
            // in match arms, preventing collisions with other types (e.g., Command.None vs Maybe.None).
            Match {
                expr: scrutinee,
                arms,
            } => {
                let scrutinee_ty = self.synth_expr(scrutinee)?.ty;
                for arm in arms.iter() {
                    self.bind_pattern(&arm.pattern, &scrutinee_ty)?;
                    if let Some(ref guard) = arm.guard {
                        self.check_expr(guard, &Type::bool())?;
                    }
                    self.check_expr(&arm.body, expected)?;
                }

                // Exhaustiveness check (check_expr path)
                // Only for variant/bool types without guards to avoid false positives
                let applied_scrut_chk = self.unifier.apply(&scrutinee_ty);
                // Resolve named types to their underlying definition
                let resolved_scrut = match &applied_scrut_chk {
                    Type::Named { path, .. } => {
                        let name = path
                            .segments
                            .last()
                            .and_then(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                                _ => None,
                            })
                            .unwrap_or("");
                        self.ctx
                            .lookup_type(name)
                            .cloned()
                            .unwrap_or(applied_scrut_chk.clone())
                    }
                    _ => applied_scrut_chk.clone(),
                };
                let should_check = matches!(&resolved_scrut, Type::Variant(_) | Type::Bool);
                let has_guards = arms.iter().any(|arm| arm.guard.is_some());
                // Check if patterns contain complex forms the exhaustiveness checker can't handle
                let has_complex_patterns = arms
                    .iter()
                    .any(|arm| Self::pattern_has_complex_forms(&arm.pattern));
                if should_check && !has_guards {
                    let match_patterns: Vec<verum_ast::pattern::Pattern> =
                        arms.iter().map(|arm| arm.pattern.clone()).collect();
                    if let Ok(result) = crate::exhaustiveness::check_exhaustiveness(
                        &match_patterns,
                        &resolved_scrut,
                        &self.ctx.env,
                    ) {
                        if !result.is_exhaustive {
                            let witness_str = result
                                .uncovered_witnesses
                                .iter()
                                .map(|w| format!("{}", w))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let msg =
                                format!("non-exhaustive patterns: `{}` not covered", witness_str);
                            if has_complex_patterns {
                                // Complex patterns may cause false positives - use warning
                                let diag = Diagnostic::new_warning(
                                    msg,
                                    span_to_line_col(expr.span),
                                    "W0601",
                                );
                                self.diagnostics.push(diag);
                            } else {
                                // Simple patterns - emit error
                                let diag = Diagnostic::new_error(
                                    msg,
                                    span_to_line_col(expr.span),
                                    "E0601",
                                );
                                self.diagnostics.push(diag);
                            }
                        }
                    }
                }

                Ok(InferResult::new(expected.clone()))
            }

            // Variant constructor call with expected variant type:
            // When checking `Valid(42)` against `Validation<Text, Int>`, resolve the
            // constructor from the expected type's variants rather than the global scope.
            // This enables user-defined types to use variant names that overlap with stdlib.
            Call {
                func,
                args: call_args,
                ..
            } if matches!(&func.kind, ExprKind::Path(_)) => {
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
                let expanded_expected = self.expand_generic_to_variant(expected);
                if let Type::Variant(ref variants) = expanded_expected {
                    if let Some(payload_ty) = variants.get(constructor_name) {
                        // The expected variant type has this constructor — use it
                        // Check call args against the payload type
                        if call_args.len() == 1 && !matches!(payload_ty, Type::Unit) {
                            // Single-payload variant: Valid(42) where Valid(A)
                            self.check_expr(&call_args[0], payload_ty)?;
                            Ok(InferResult::new(expected.clone()))
                        } else if call_args.is_empty() && matches!(payload_ty, Type::Unit) {
                            // Unit variant: None
                            Ok(InferResult::new(expected.clone()))
                        } else if call_args.len() > 1 {
                            // Multi-arg: treat as tuple payload
                            if let Type::Tuple(tuple_types) = payload_ty {
                                if tuple_types.len() == call_args.len() {
                                    for (arg, ty) in call_args.iter().zip(tuple_types.iter()) {
                                        self.check_expr(arg, ty)?;
                                    }
                                    return Ok(InferResult::new(expected.clone()));
                                }
                            }
                            // Fall through to synth_and_check
                            self.synth_and_check(expr, expected)
                        } else {
                            // Mismatch in arity — fall through
                            self.synth_and_check(expr, expected)
                        }
                    } else {
                        // Constructor not in expected variant type — fall through
                        self.synth_and_check(expr, expected)
                    }
                } else {
                    // Expected type is not a variant — fall through
                    self.synth_and_check(expr, expected)
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
            Unary {
                op: UnOp::Ref,
                expr: inner,
            } => {
                if let Type::Reference {
                    mutable: false,
                    inner: expected_inner,
                } = expected
                {
                    // Try checking the inner expression against the expected inner type first
                    match self.check_expr(inner, expected_inner) {
                        Ok(_) => {
                            // Borrow tracking for aliasing detection
                            // CRITICAL: Do NOT discard the Result — borrow conflicts must propagate
                            //

                            // Root fix for Issue #4 (NLL over-retain): when
                            // the `&x` appears in a call-argument position,
                            // use `borrow_immut_for_call` — a transient
                            // borrow that does not persist past the call
                            // boundary. The symmetric `borrow_mut_for_call`
                            // already exists for the RefMut arm below; the
                            // missing immut counterpart caused
                            // `call(&value); mutate(&mut value)` sequences
                            // to fail with a phantom "previous immutable
                            // borrow" because the call-arg borrow lingered
                            // in the tracker with no live holder.
                            match &inner.kind {
                                ExprKind::Path(path) => {
                                    if let Some(verum_ast::ty::PathSegment::Name(id)) =
                                        path.segments.first()
                                    {
                                        let var_name = id.name.as_str();
                                        if self.in_call_arg_context {
                                            self.borrow_tracker
                                                .borrow_immut_for_call(var_name, expr.span)?;
                                        } else {
                                            self.borrow_tracker
                                                .borrow_immut(var_name, expr.span)?;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            Ok(InferResult::new(expected.clone()))
                        }
                        Err(_) => {
                            // Fall back to synth_and_check (auto-deref path)
                            self.synth_and_check(expr, expected)
                        }
                    }
                } else {
                    self.synth_and_check(expr, expected)
                }
            }

            Unary {
                op: UnOp::RefMut,
                expr: inner,
            } => {
                if let Type::Reference {
                    mutable: true,
                    inner: expected_inner,
                } = expected
                {
                    match self.check_expr(inner, expected_inner) {
                        Ok(_) => {
                            // Borrow tracking for aliasing detection
                            // CRITICAL: Do NOT discard the Result — borrow conflicts must propagate
                            // Spec: L0-critical/reference_system/access_rules/ref_aliasing_function
                            match &inner.kind {
                                ExprKind::Path(path) => {
                                    if let Some(verum_ast::ty::PathSegment::Name(id)) =
                                        path.segments.first()
                                    {
                                        let var_name = id.name.as_str();
                                        if self.in_call_arg_context {
                                            // NLL: For call arguments, use temporary borrow
                                            self.borrow_tracker
                                                .borrow_mut_for_call(var_name, expr.span)?;
                                        } else {
                                            // Normal: For let bindings, use strict borrow checking
                                            self.borrow_tracker.borrow_mut(var_name, expr.span)?;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            Ok(InferResult::new(expected.clone()))
                        }
                        Err(_) => self.synth_and_check(expr, expected),
                    }
                } else {
                    self.synth_and_check(expr, expected)
                }
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
            MethodCall {
                receiver,
                method,
                args: call_args,
                ..
            } => {
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
                                        let result_type =
                                            self.substitute_self_type(return_type, expected);
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

            // Map literal: check entries against expected Map<K, V> type
            MapLiteral { entries } => {
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

            // Fall back to synthesis + subsumption check
            _ => self.synth_and_check(expr, expected),
        }
    }

    /// Synthesize then check subsumption.
    ///

    /// Includes auto-borrow coercion: T → &T when expected is immutable reference.
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 3 (Auto-Borrow в позиции вызова)
    /// Bidirectional type-check a closure expression.
    /// When `expected` is a `Forall` (rank-2), instantiates the quantified
    /// variables. When a `Function` type is known, uses it to bind params;
    /// otherwise falls back to synthesis + subsumption.
    fn check_closure_expr(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Closure { params, body: closure_body, return_type, async_, .. } = &expr.kind
            else { unreachable!() };
                // Flip `in_async_context` for async closures so their body
                // may use `.await`. Mirrors the synth-mode handler at
                // ~11725. The flag is restored by the `AsyncCtxGuard`
                // below on every return path (including early `?` errors
                // and success).
                struct AsyncCtxGuard<'a> {
                    checker: *mut TypeChecker,
                    prev: Option<bool>,
                    _lt: std::marker::PhantomData<&'a ()>,
                }
                impl Drop for AsyncCtxGuard<'_> {
                    fn drop(&mut self) {
                        if let Some(prev) = self.prev {
                            // SAFETY: the guard borrows `self` through the
                            // raw pointer for its lifetime only; the
                            // surrounding `&mut self` enforces uniqueness.
                            unsafe {
                                (*self.checker).in_async_context = prev;
                            }
                        }
                    }
                }
                let _async_ctx_guard = if *async_ {
                    let prev = std::mem::replace(&mut self.in_async_context, true);
                    AsyncCtxGuard {
                        checker: self as *mut TypeChecker,
                        prev: Some(prev),
                        _lt: std::marker::PhantomData,
                    }
                } else {
                    AsyncCtxGuard {
                        checker: self as *mut TypeChecker,
                        prev: None,
                        _lt: std::marker::PhantomData,
                    }
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

                        // Enter new scope for lambda
                        self.ctx.enter_scope();

                        // Bind parameters with the instantiated types.
                        // Normalize each param type to resolve associated type projections
                        // (e.g., ::Item[Range<Int>] → Int) before binding.
                        for (param, param_ty) in params.iter().zip(expected_params.iter()) {
                            let normalized_param_ty = self.normalize_type(param_ty);
                            self.bind_pattern(&param.pattern, &normalized_param_ty)?;
                        }

                        // Check body against return type
                        let ret_ty = if let Some(rt) = return_type {
                            self.ast_to_type(rt)?
                        } else {
                            (**expected_return).clone()
                        };

                        // Check body against expected return type with bidirectional inference.
                        // We do NOT synth first: synth would resolve variant constructors
                        // (e.g., Continue(x)) using the ambient scope (e.g., ControlFlow.Continue
                        // from the prelude) instead of the expected type (e.g., ReduceResult<Int>).
                        // Bidirectional check_expr correctly propagates the expected type through
                        // blocks, if/else expressions, and variant constructors.
                        // Never-returning bodies (e.g., panic("...")) are handled correctly too:
                        // unify(Never, T) succeeds for all T (unify.rs bottom-type rule).
                        self.check_expr(closure_body, &ret_ty)?;

                        self.ctx.exit_scope();

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

                        // Enter new scope for lambda
                        self.ctx.enter_scope();

                        // Bind parameters.
                        // Normalize each param type to resolve associated type projections
                        // (e.g., ::Item[Range<Int>] → Int) before binding, so that the
                        // closure body sees concrete types rather than un-resolved projections.
                        for (param, param_ty) in params.iter().zip(expected_params.iter()) {
                            let normalized_param_ty = self.normalize_type(param_ty);
                            self.bind_pattern(&param.pattern, &normalized_param_ty)?;
                        }

                        // Check body against return type
                        let ret_ty = if let Some(rt) = return_type {
                            self.ast_to_type(rt)?
                        } else {
                            (**expected_return).clone()
                        };

                        // Check body against expected return type with bidirectional inference.
                        // We do NOT synth first: synth would resolve variant constructors
                        // (e.g., Continue(x)) using the ambient scope (e.g., ControlFlow.Continue
                        // from the prelude) instead of the expected type (e.g., ReduceResult<Int>).
                        // Bidirectional check_expr correctly propagates the expected type through
                        // blocks, if/else expressions, and variant constructors.
                        // Never-returning bodies (e.g., panic("...")) are handled correctly too:
                        // unify(Never, T) succeeds for all T (unify.rs bottom-type rule).
                        self.check_expr(closure_body, &ret_ty)?;

                        self.ctx.exit_scope();

                        // Register closure type in the type registry for codegen
                        // Apply unifier substitution to resolve type variables
                        // This enables closure parameter type inference during LLVM codegen
                        let resolved_expected = self.unifier.apply(expected);
                        self.type_registry
                            .register_expr(expr.span, resolved_expected);

                        Ok(InferResult::new(expected.clone()))
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

                // Check then branch with evidence in scope
                self.check_block(then_branch, expected)?;

                // Check if then branch unconditionally exits
                let then_exits = EvidencePropagator::block_unconditionally_exits(then_branch);

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
                }

                Ok(InferResult::new(expected.clone()))
    }

    /// Bidirectional type-check a record literal expression against an expected type.
    /// Checks each field against its declared type, handles optional base spread,
    /// and propagates narrowed type information for refinement fields.
    fn check_record_expr(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Record { path, fields, base } = &expr.kind
            else { unreachable!() };
                // For generic struct instantiation, extract type args from expected type
                // and substitute them into the field types
                let type_name = self.path_to_string(path);
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

                if is_variant_constructor {
                    return self.synth_and_check(expr, expected);
                }

                // Get the stored field types (may contain type variables like T)
                let stored_fields = match self.ctx.lookup_type(struct_key.as_str()) {
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
                };

                // Get type parameters from the stored type definition
                let type_params_key = format!("__type_params_{}", type_name);
                let type_params: List<verum_common::Text> = match self
                    .ctx
                    .lookup_type(type_params_key.as_str())
                {
                    Option::Some(Type::Record(params_map)) => params_map.keys().cloned().collect(),
                    _ => List::new(), // No type parameters
                };

                // Extract concrete type args from expected type (e.g., Int from Box<Int>)
                // CRITICAL FIX: If expected is a type variable and this is a generic type,
                // create fresh type variables for the type parameters.
                let type_args: List<Type> = match expected {
                    Type::Named { args, .. } if !args.is_empty() => args.clone(),
                    Type::Named { args, .. } if args.is_empty() && !type_params.is_empty() => {
                        // Named type without args but we have type params - create fresh vars
                        type_params
                            .iter()
                            .map(|_| Type::Var(TypeVar::fresh()))
                            .collect()
                    }
                    _ if !type_params.is_empty() => {
                        // Expected is a type variable or other - create fresh type vars for generic types
                        type_params
                            .iter()
                            .map(|_| Type::Var(TypeVar::fresh()))
                            .collect()
                    }
                    _ => List::new(),
                };

                // Build a substitution from type parameters to type arguments
                let mut param_subst = indexmap::IndexMap::new();
                for (param_name, arg_ty) in type_params.iter().zip(type_args.iter()) {
                    param_subst.insert(param_name.clone(), arg_ty.clone());
                }

                // Apply the substitution to get resolved field types
                let expected_fields = if let Some(field_types) = stored_fields {
                    let mut resolved_fields = indexmap::IndexMap::new();
                    for (fname, fty) in field_types.iter() {
                        // Substitute type variables with concrete types
                        let resolved_ty = self.substitute_type_params(fty, &param_subst);
                        // CRITICAL FIX: Also resolve any placeholder types (forward references)
                        // When types are defined out of order, field types may contain placeholders
                        // like <placeholder:Metadata> that need to be resolved to the actual type.
                        let resolved_ty = self.substitute_placeholders(&resolved_ty);
                        resolved_fields.insert(fname.clone(), resolved_ty);
                    }
                    resolved_fields
                } else {
                    return self.synth_and_check(expr, expected);
                };

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
                        self.check_expr(value_expr, expected_field_ty)?;
                    } else {
                        // Shorthand syntax
                        if let Some(scheme) = self.ctx.env.lookup(field_name.as_str()) {
                            let var_ty = scheme.instantiate();
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

                // CRITICAL FIX: Unify the actual struct type with the expected type
                // This is essential for generic type inference: when expected is a type
                // variable τ37, we need to bind it to the actual struct type (e.g., Handle)
                // so that outer generic structs like Wrapper<T> can resolve T correctly.
                // NOTE: Resolve Self to actual type path if needed
                let resolved_path = if path.segments.len() == 1
                    && matches!(path.segments[0], verum_ast::ty::PathSegment::SelfValue)
                {
                    if let Maybe::Some(Type::Named {
                        path: self_path, ..
                    }) = &self.current_self_type
                    {
                        self_path.clone()
                    } else {
                        path.clone()
                    }
                } else {
                    path.clone()
                };
                let actual_struct_ty = Type::Named {
                    path: resolved_path,
                    args: type_args,
                };
                self.unifier.unify(&actual_struct_ty, expected, expr.span)?;

                Ok(InferResult::new(expected.clone()))
    }

    fn synth_and_check(&mut self, expr: &Expr, expected: &Type) -> Result<InferResult> {
        let result = self.synth_expr(expr)?;

        // CRITICAL FIX: Never type (bottom type) is a subtype of ALL types.
        // panic(), return, break, continue all return Never.
        // This enables: let x: Int = panic("...") and closure bodies that diverge.
        // Type lattice: Never is the bottom type, subtype of all types (Never <: T for all T)
        let resolved_result_ty = self.unifier.apply(&result.ty);
        // Check for Never type (handles both Type::Never and Named("Never"))
        if resolved_result_ty.is_never() {
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
                                // Syntactic can't confirm → gradual verification
                            }
                            // Valid, Unknown, or Err — gradual verification
                            _ => {}
                        }
                    }
                }
                return Ok(InferResult::new(expected.clone()));
            }
            Err(_e) => {
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
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 3 (Auto-Borrow в позиции вызова)
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
                // CAPABILITY ATTENUATION COERCION
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
                // ============================================================
                // Automatic capability attenuation: T with [More] → T with [Less]
                // When actual has more capabilities than expected, automatically attenuate.
                //

                // Example:
                // ```verum
                // fn analyze(db: Database with [Read]) -> Stats { ... }
                // fn process(db: Database with [Read, Write]) {
                //  analyze(db); // OK: automatic attenuation [Read, Write] → [Read]
                // }
                // ```
                //

                // Cases handled:
                // 1. CapabilityRestricted → CapabilityRestricted (subset of capabilities)
                // 2. CapabilityRestricted → Base (forgetful upcast)
                // 3. Base → CapabilityRestricted (ERROR: cannot add capabilities)
                if let (
                    Type::CapabilityRestricted {
                        base: base1,
                        capabilities: caps1,
                    },
                    Type::CapabilityRestricted {
                        base: base2,
                        capabilities: caps2,
                    },
                ) = (&result.ty, expected)
                {
                    // Check if base types unify
                    if self.unifier.unify(base1, base2, span).is_ok() {
                        // Check if caps1 ⊇ caps2 (actual has MORE capabilities)
                        // Uses TypeCapabilitySet::is_subset_of for proper set comparison
                        if caps2.is_subset_of(caps1) {
                            // Automatic attenuation: actual has more capabilities, coerce to expected
                            return Ok(InferResult::new(expected.clone()));
                        }
                    }
                }

                // Also handle: T with [Caps] → T (forgetful upcast)
                if let Type::CapabilityRestricted { base, .. } = &result.ty {
                    if self.unifier.unify(base, expected, span).is_ok() {
                        return Ok(InferResult::new(expected.clone()));
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

                // Normalized type structural equality: accept if alias-expanded
                // types match textually (e.g., Result<T,E> -> Ok(T)|Err(E)).
                if normalized_actual.to_text() == normalized_expected.to_text() {
                    return Ok(InferResult::new(expected.clone()));
                }

                // Variant subtype coercion: accept subset/superset of variants.
                if let (Type::Variant(av), Type::Variant(ev)) =
                    (&normalized_actual, &normalized_expected)
                {
                    if ev.keys().all(|k| av.contains_key(k))
                        || av.keys().all(|k| ev.contains_key(k))
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
                    _ => {
                        // For all other types: try unification first (same-type comparison)
                        if self.unifier.unify(left_deref, right_deref, span).is_ok() {
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
            Deref => match inner_ty {
                // Never propagation
                Type::Never => Ok(Type::Never),
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. }
                | Type::Ownership { inner, .. }
                | Type::Pointer { inner, .. }
                | Type::VolatilePointer { inner, .. } => Ok((**inner).clone()),
                // Smart pointer types are dereferenceable to T
                // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Smart pointer types
                Type::Generic { name, args } => {
                    // Protocol-based deref: check for Ref<T>/Deref implementation
                    if let Some(target_ty) = self.find_deref_target_type(inner_ty) {
                        // CRITICAL FIX: If the protocol lookup returns an unresolved type variable
                        // (from a blanket impl like `implement<T> Ref<T> for Heap<T>`), prefer
                        // the concrete type arg from args[0] when available.
                        if matches!(&target_ty, Type::Var(_)) && args.len() == 1 {
                            // Resolve through unifier for recursive types (e.g., Heap<Var>
                            // where Var was unified with Node from List<Heap<Node>>).
                            Ok(self.unifier.apply(&args[0]))
                        } else {
                            Ok(target_ty)
                        }
                    } else if args.len() == 1 {
                        // Fallback: single-arg generic types are likely smart pointer wrappers
                        Ok(self.unifier.apply(&args[0]))
                    } else {
                        Err(TypeError::Other(
                            format!("Cannot dereference non-reference type: {}", inner_ty).into(),
                        ))
                    }
                }
                Type::Named { path, args } => {
                    // Protocol-based deref: check for Ref<T>/Deref implementation
                    if let Some(target_ty) = self.find_deref_target_type(inner_ty) {
                        // CRITICAL FIX: If the protocol lookup returns an unresolved type variable
                        // (from a blanket impl like `implement<T> Ref<T> for Heap<T>`), prefer
                        // the concrete type arg from args[0] when available. The blanket impl's
                        // fresh type variable won't have been unified with the actual type arg.
                        if matches!(&target_ty, Type::Var(_)) && args.len() == 1 {
                            // Resolve the type arg through unification in case it's a Var
                            // that was unified with a concrete type (e.g., Heap<Var> where
                            // Var was unified with Node from List<Heap<Node>> indexing).
                            Ok(self.unifier.apply(&args[0]))
                        } else {
                            Ok(target_ty)
                        }
                    } else if args.len() == 1 {
                        // Fallback: single-arg generic types are likely smart pointer wrappers
                        Ok(self.unifier.apply(&args[0]))
                    } else {
                        Err(TypeError::Other(
                            format!("Cannot dereference non-reference type: {}", inner_ty).into(),
                        ))
                    }
                }
                // Auto-deref through protocol bounds
                // Spec: L0-critical/reference_system/reference_tiers/tier_conversion.vr
                // When `*r` is used on a type variable `R` with a `Ref<T>` bound,
                // the dereference returns `T` (the target type from the protocol).
                Type::Var(var) => {
                    // First, try to resolve the type variable through unification
                    let resolved = self.unifier.apply(&Type::Var(*var));
                    // If the type variable resolved to a reference type, use that
                    match &resolved {
                        Type::Reference { inner, .. } => {
                            return Ok(*inner.clone());
                        }
                        Type::CheckedReference { inner, .. } => {
                            return Ok(*inner.clone());
                        }
                        Type::UnsafeReference { inner, .. } => {
                            return Ok(*inner.clone());
                        }
                        _ => {}
                    }

                    let bounds = self.get_type_var_bounds(var);
                    for bound in &bounds {
                        if let Some(ident) = bound.protocol.as_ident() {
                            let proto_name = ident.name.as_str();
                            if proto_name == "Ref" || proto_name == "RefMut" {
                                if let Some(target_ty) = bound.args.first() {
                                    return Ok(target_ty.clone());
                                }
                            }
                        }
                    }
                    // When the type variable is unresolved and has no Ref bound,
                    // produce a fresh type variable for the deref result.
                    // This handles cases like `*value` where `value: T` and T
                    // will be resolved to a reference type later through unification
                    // (e.g., Iterator.next() returning Maybe<&T>).
                    let result_var = TypeVar::fresh();
                    // Constrain: the original var must be a reference to the result
                    let ref_ty = Type::Reference {
                        inner: Box::new(Type::Var(result_var)),
                        mutable: false,
                    };
                    let _ = self.unifier.unify(&resolved, &ref_ty, span);
                    Ok(Type::Var(result_var))
                }
                _ => {
                    if let Some(target_ty) = self.find_deref_target_type(inner_ty) {
                        Ok(target_ty)
                    } else if self.in_unsafe_context {
                        // In unsafe context, allow dereferencing any type
                        // (raw pointer arithmetic)
                        Ok(Type::Var(TypeVar::fresh()))
                    } else {
                        // Transparent deref: when `*x` is applied to a value type
                        // (not a reference/pointer), treat it as a no-op and return
                        // the type itself. This handles common patterns where iterator
                        // items may be resolved as values rather than references
                        // (e.g., `for v in map.values() { *v }` where v is already Int).
                        // Spec: CBGR three-tier reference model — deref on value types is identity.
                        Ok(inner_ty.clone())
                    }
                }
            },
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
                            if matches!(op, BinOp::Assign) {
                                // For assignments, use the non-iterative path which handles
                                // init tracking properly via check_expr_assignment_target
                                let result = self.infer_binop(*op, left, right, e.span)?;
                                value_stack.push(result.ty);
                            } else {
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

                    // ============================================================
                    // NLL: Release borrow at last use for Deref operations
                    // Spec: L0-critical/reference_system/access_rules/ref_scope_valid
                    // ============================================================
                    if matches!(op, UnOp::Deref) {
                        if let ExprKind::Path(path) = &inner_expr.kind {
                            if let Some(verum_ast::ty::PathSegment::Name(id)) =
                                path.segments.first()
                            {
                                let holder_name = id.name.as_str();
                                self.borrow_tracker.release_borrow_at_last_use(holder_name);
                            }
                        }
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
                            if let Some(scheme) = self.ctx.env.lookup("self") {
                                let ty = scheme.instantiate();
                                return Ok(ty);
                            }
                        }
                        return Err(TypeError::Other(
                            "self keyword requires method context".into(),
                        ));
                    }
                    verum_ast::ty::PathSegment::Super => {
                        // super as a standalone path is used as a base for field access
                        // chains like super.module.function(). Return Never to suppress
                        // downstream errors (Never is a bottom type compatible with any type).
                        return Ok(Type::Never);
                    }
                    verum_ast::ty::PathSegment::Cog => {
                        // cog as a standalone path is used as a base for qualified access
                        return Ok(Type::Never);
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

                let ty = scheme.instantiate();
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
                Ok(resolved_ty)
            } else {
                // Try module-level function lookup
                if let Maybe::Some(scheme) = self.lookup_function_in_module(name) {
                    let ty = scheme.instantiate();
                    let resolved_ty = self.unifier.apply(&ty);
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
                    // Stdlib-agnostic per `crates/verum_types/src/CLAUDE.md`:
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

                    // IMPORTANT: If this type has generic parameters but was referenced without
                    // explicit type arguments, create fresh type variables for the missing params.
                    // This enables proper inference for types like `PendingFuture<T>` used as `PendingFuture`.
                    let type_name_text = verum_common::Text::from(name);
                    if let Some(&generics_count) = self.type_generics_count.get(&type_name_text) {
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
                    // This is an inline module name being used as a namespace
                    // Return a special marker type that signals this is a module namespace
                    // The field access handling will navigate into the module
                    // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
                    Ok(Type::Named {
                        path: verum_ast::ty::Path {
                            segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::Ident {
                                name: format!("__module__{}", name).into(),
                                span,
                            })]
                            .into(),
                            span,
                        },
                        args: List::new(),
                    })
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
                } else if self.stdlib_single_file_mode {
                    Ok(Type::Unknown)
                } else {
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

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deep recursion.
    /// Tracks recursion depth to detect infinite recursion early.
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
                Range {
                    start,
                    end,
                    inclusive,
                } => {
                    // Determine the element type from start and end bounds
                    let elem_ty = match (start.as_ref(), end.as_ref()) {
                        (Some(start_expr), Some(end_expr)) => {
                            // Both bounds present: infer from start and unify with end
                            let start_result = self.synth_expr(start_expr)?;
                            let end_result = self.synth_expr(end_expr)?;
                            // Unify start and end types
                            self.unifier
                                .unify(&start_result.ty, &end_result.ty, expr.span)?;
                            start_result.ty
                        }
                        (Some(start_expr), None) => self.synth_expr(start_expr)?.ty,
                        (None, Some(end_expr)) => self.synth_expr(end_expr)?.ty,
                        (None, None) => Type::Var(TypeVar::fresh()),
                    };
                    let range_type_name = if *inclusive {
                        verum_common::Text::from("RangeInclusive")
                    } else {
                        verum_common::Text::from(WKT::Range.as_str())
                    };
                    Ok(InferResult::new(Type::Generic {
                        name: range_type_name,
                        args: vec![elem_ty].into(),
                    }))
                }

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
                Field { .. } => self.infer_expr_field(current_expr),

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
                Loop {
                    label: _,
                    body,
                    invariants: _,
                } => {
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
                TensorLiteral {
                    shape,
                    elem_type,
                    data,
                } => {
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

                // Interpolated strings: f"Sum: {sum}", sql"SELECT * FROM {table}"
                // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — #1.4.2 - Text interpolation with injection protection
                InterpolatedString {
                    handler: _,
                    parts: _,
                    exprs,
                } => {
                    // Type check all embedded expressions
                    // They can be of any type - will be converted to string at runtime
                    for expr in exprs.iter() {
                        self.synth_expr(expr)?;
                    }
                    // Interpolated strings always produce Text
                    Ok(InferResult::new(Type::text()))
                }

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
                TryRecoverFinally {
                    try_block,
                    recover,
                    finally_block,
                } => {
                    // Extract error type from ? operators inside try block before type checking
                    let error_type = self.extract_try_block_error_type(try_block)?;

                    // Track that we're inside a try/recover block so throw is allowed
                    self.try_recover_depth += 1;

                    // Temporarily set function return type to Result<T, E> so that
                    // the ? operator inside the try block passes type checking.
                    let saved_return_type = self.current_function_return_type.clone();
                    let try_ok_var = Type::Var(crate::ty::TypeVar::fresh());
                    self.current_function_return_type =
                        Maybe::Some(Type::result(try_ok_var.clone(), error_type.clone()));

                    // Infer type of try block
                    let try_result = self.synth_expr(try_block)?;

                    // Restore original function return type
                    self.current_function_return_type = saved_return_type;
                    self.try_recover_depth -= 1;

                    // Infer type of recovery body with error type for pattern binding
                    let recover_ty = self.infer_recover_body(recover, &error_type)?;

                    // Recovery body must have the same type as try block
                    self.unifier.unify(&try_result.ty, &recover_ty, expr.span)?;

                    // Type check finally block (result is discarded)
                    self.synth_expr(finally_block)?;

                    Ok(InferResult::new(try_result.ty))
                }

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
                UseContext {
                    context: _,
                    handler,
                    body,
                } => {
                    // Type check the handler expression
                    let _handler_result = self.synth_expr(handler)?;

                    // The body is evaluated with the context in scope
                    // Type of the expression is the type of the body
                    let body_result = self.synth_expr(body)?;
                    Ok(body_result)
                }

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
                ForAwait {
                    label: _,
                    pattern,
                    async_iterable,
                    body,
                    invariants: _,
                    decreases: _,
                } => {
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
                Is {
                    expr: test_expr,
                    pattern,
                    negated: _,
                } => {
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
                Quote {
                    target_stage,
                    tokens,
                } => {
                    // Perform hygiene checking on the quote tokens
                    // Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene
                    self.check_quote_hygiene(tokens, *target_stage, expr.span)?;

                    // Quote expressions always produce TokenStream
                    let token_stream_ty = self
                        .ctx
                        .lookup_type(&verum_common::Text::from("TokenStream"))
                        .cloned()
                        .unwrap_or_else(|| Type::Named {
                            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                                "TokenStream",
                                expr.span,
                            )),
                            args: List::new(),
                        });
                    Ok(InferResult::new(token_stream_ty))
                }

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

                    let then_result = self.infer_block(then_branch)?;

                    self.ctx.env.pop_scope();
                    self.refinement_evidence.pop_scope();

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

                    // Unify branch types
                    let span = expr.span;
                    self.unifier.unify(&then_result.ty, &else_result.ty, span)?;

                    Ok(InferResult::new(then_result.ty))
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

                                // Protocol-based IntoIterator resolution
                                let elem_ty = match self
                                    .protocol_checker
                                    .read()
                                    .resolve_into_iterator_protocol(&iter_result.ty)
                                {
                                    Some(resolution) => resolution.item,
                                    None => {
                                        // Fallback: Try Iterator protocol's Item associated type
                                        let resolved_iter_ty = self.unifier.apply(&iter_result.ty);
                                        let iter_item =
                                            self.protocol_checker.read().try_find_associated_type(
                                                &resolved_iter_ty,
                                                &verum_common::Text::from("Item"),
                                            );
                                        if let Some(item_ty) = iter_item {
                                            self.protocol_checker
                                                .read()
                                                .normalize_projection_type(&item_ty)
                                        } else {
                                            return Err(TypeError::Other(
                                                verum_common::Text::from(format!(
                                                    "Cannot iterate over type in map comprehension: {}",
                                                    resolved_iter_ty
                                                )),
                                            ));
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

    fn infer_match_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use crate::refinement_evidence::{PathCondition, PathConditionKind};
        let ExprKind::Match { expr: scrutinee, arms } = &expr.kind
            else { unreachable!() };

                    // NOTE: Affine tracking for scrutinee is handled by synth_expr
                    // Do NOT add explicit use_value here - it would cause double-consume errors
                    // since infer_path_expr already calls use_value for Path expressions
                    let scrut_result = self.synth_expr(scrutinee)?;
                    let scrut_ty = scrut_result.ty;

                    // Auto-deref scrutinee for match expressions:
                    // When matching &T where T is a variant/generic type, auto-deref to T.
                    // This allows `match self` in impl methods with `&self` to work
                    // without requiring explicit `match *self`.
                    // The variant payload bindings will NOT be ref-wrapped.
                    let scrut_ty = match &scrut_ty {
                        Type::Reference { inner, .. }
                        | Type::CheckedReference { inner, .. }
                        | Type::UnsafeReference { inner, .. } => match &**inner {
                            Type::Variant(_)
                            | Type::Generic { .. }
                            | Type::Named { .. }
                            | Type::Record(_) => (**inner).clone(),
                            _ => scrut_ty,
                        },
                        _ => scrut_ty,
                    };

                    if arms.is_empty() {
                        return Err(TypeError::Other(
                            "Match expression must have at least one arm".into(),
                        ));
                    }

                    // Check if this is a dependent match (on an inductive type with indices).
                    // Gated on [types].dependent: when disabled, all match expressions
                    // use regular (non-dependent) pattern matching even on inductive
                    // types with indices. This avoids the overhead of motive synthesis
                    // and index unification for projects that don't use dependent types.
                    if self.dependent_enabled {
                        let is_dependent = self.is_dependent_type(&scrut_ty);
                        if is_dependent {
                            return self.check_dependent_match(&scrut_ty, arms, expr.span);
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

                    // First pass: check all arms and collect their types
                    for arm in arms.iter() {
                        self.ctx.enter_scope();
                        self.refinement_evidence.push_scope();

                        // Add pattern evidence for this arm
                        if let Maybe::Some(ref var_name) = scrutinee_var {
                            self.add_pattern_evidence(
                                &arm.pattern,
                                var_name.clone(),
                                arm.pattern.span,
                            );
                        }

                        // CRITICAL: Bind pattern variables BEFORE checking guard
                        // Guards can reference pattern bindings (e.g., `Some(x) if x > 0`)
                        // CRITICAL FIX: Apply unifier to scrutinee type before pattern binding.
                        // When the scrutinee comes from a generic method call (e.g., `Validated.validate_all(items)`),
                        // the returned type may contain type variables that were unified with concrete types
                        // during argument checking. Without applying the unifier here, patterns like
                        // `Valid(values)` would bind `values` to a type variable instead of the concrete type.
                        let resolved_scrut_ty = self.unifier.apply(&scrut_ty);
                        self.bind_pattern(&arm.pattern, &resolved_scrut_ty)?;

                        // Handle guard expression evidence (now pattern bindings are in scope)
                        if let Maybe::Some(ref guard) = arm.guard {
                            self.check_expr(guard, &Type::bool())?;
                            self.refinement_evidence
                                .add_evidence_from_condition(guard, guard.span);
                        }
                        let arm_result = self.synth_expr(&arm.body)?;
                        let is_arm_never = self.is_never_type(&arm_result.ty);

                        // Track this arm's type for later unification
                        all_arm_types.push((arm_result.ty.clone(), arm.body.span));

                        // If we haven't found a non-Never type yet, use this one
                        if self.is_never_type(&result_ty) && !is_arm_never {
                            result_ty = arm_result.ty;
                        }

                        self.refinement_evidence.pop_scope();
                        self.ctx.exit_scope();
                    }

                    // Second pass: unify all non-Never arm types with the result type
                    for (arm_ty, span) in all_arm_types.iter() {
                        if !self.is_never_type(arm_ty) && !self.is_never_type(&result_ty) {
                            self.unifier.unify(arm_ty, &result_ty, *span)?;
                        }
                    }

                    // Exhaustiveness check: verify all possible values are covered
                    // Only check for types with finite constructors (Variant, Bool)
                    // to avoid false positives with Int/Float/Text and guarded patterns.
                    let applied_scrut = self.unifier.apply(&scrut_ty);
                    // Resolve named types to their underlying definition for exhaustiveness checking
                    let resolved_scrut = match &applied_scrut {
                        Type::Named { path, .. } => {
                            let name = path
                                .segments
                                .last()
                                .and_then(|s| match s {
                                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .unwrap_or("");
                            self.ctx
                                .lookup_type(name)
                                .cloned()
                                .unwrap_or(applied_scrut.clone())
                        }
                        _ => applied_scrut.clone(),
                    };
                    let should_check_exhaustiveness =
                        matches!(&resolved_scrut, Type::Variant(_) | Type::Bool);
                    // Don't check if any arm has a guard (guards make analysis imprecise)
                    let has_guards = arms.iter().any(|arm| arm.guard.is_some());
                    let has_complex_patterns = arms
                        .iter()
                        .any(|arm| Self::pattern_has_complex_forms(&arm.pattern));
                    if should_check_exhaustiveness && !has_guards {
                        let match_patterns: Vec<verum_ast::pattern::Pattern> =
                            arms.iter().map(|arm| arm.pattern.clone()).collect();
                        match crate::exhaustiveness::check_exhaustiveness(
                            &match_patterns,
                            &resolved_scrut,
                            &self.ctx.env,
                        ) {
                            Ok(result) => {
                                if !result.is_exhaustive {
                                    let witness_str = result
                                        .uncovered_witnesses
                                        .iter()
                                        .map(|w| format!("{}", w))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    let msg = format!(
                                        "non-exhaustive patterns: `{}` not covered",
                                        witness_str
                                    );
                                    if has_complex_patterns {
                                        let diag = Diagnostic::new_warning(
                                            msg,
                                            span_to_line_col(expr.span),
                                            "W0601",
                                        );
                                        self.diagnostics.push(diag);
                                    } else {
                                        let diag = Diagnostic::new_error(
                                            msg,
                                            span_to_line_col(expr.span),
                                            "E0601",
                                        );
                                        self.diagnostics.push(diag);
                                    }
                                }
                            }
                            Err(_) => {
                                // Exhaustiveness check failed - skip silently
                            }
                        }
                    }

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

                        let recover_ty = self.infer_recover_body(recover_body, &error_type)?;
                        // Recover block must return the same type as the body
                        self.unifier
                            .unify(&recover_ty, &body_result.ty, *nursery_span)?;
                    }

                    self.ctx.exit_scope();

                    // The nursery returns the body's type (or recover's unified type)
                    Ok(body_result)
    }

    fn infer_closure_expr(&mut self, expr: &Expr, outer_span: Span) -> Result<InferResult> {
        let ExprKind::Closure { params, body, return_type, async_, move_, .. } = &expr.kind
            else { unreachable!() };
                    // ============================================================
                    // Closure Capture Analysis
                    // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .5 - Closure captures
                    // ============================================================
                    // Track what variables this closure captures and check for
                    // aliasing conflicts with existing borrows.

                    // Analyze captured variables BEFORE entering closure scope
                    // This determines what outside variables the closure references
                    let captures = self.analyze_closure_captures(body, params, *move_);

                    // Enter closure tracking for capture analysis
                    let closure_id = self.borrow_tracker.enter_closure(*move_);

                    // Register each captured variable and check for aliasing conflicts
                    // Collect all capture errors to report them together
                    let mut capture_errors: List<TypeError> = List::new();

                    for (var_name, field_path, capture_mode, capture_span) in &captures {
                        // Check if capturing this variable would conflict with existing borrows
                        // E.g., can't capture `&mut x` if x is already immutably borrowed
                        // Field-level tracking enables borrow splitting: `x.a` and `x.b` can be
                        // captured separately without conflict.
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

                    // Report first capture error if any exist
                    // (We collect all to provide better diagnostics in the future)
                    if let Some(first_error) = capture_errors.into_iter().next() {
                        // Clean up before returning error
                        self.borrow_tracker.exit_closure(closure_id);
                        return Err(first_error);
                    }

                    // Enter new scope for lambda
                    self.ctx.enter_scope();

                    // Enter async scope if this is an async closure
                    // Also flip `in_async_context` so the body may use
                    // `.await` — c7b4d71's enforcement was keyed on the
                    // function-scope flag and missed async closures. For
                    // `async move |x| { future.await }` the body is as
                    // valid a .await site as an `async fn` body.
                    let prev_async_context = if *async_ {
                        self.borrow_tracker.enter_async_scope();
                        Some(std::mem::replace(&mut self.in_async_context, true))
                    } else {
                        None
                    };

                    // Collect parameter names and types for dependent type analysis
                    let mut param_names = List::new();
                    let mut param_types = List::new();

                    for param in params.iter() {
                        // Extract parameter name for potential dependent type use
                        let param_name = match &param.pattern.kind {
                            verum_ast::pattern::PatternKind::Ident { name, .. } => {
                                Some(name.name.clone())
                            }
                            _ => None,
                        };
                        param_names.push(param_name);

                        let param_ty = if let Some(ty_annotation) = &param.ty {
                            self.ast_to_type(ty_annotation)?
                        } else {
                            Type::Var(TypeVar::fresh())
                        };
                        self.bind_pattern(&param.pattern, &param_ty)?;
                        param_types.push(param_ty);
                    }

                    // Synthesize or check body
                    let ret_ty = if let Some(rt) = return_type {
                        let expected_ret = self.ast_to_type(rt)?;
                        self.check_expr(body, &expected_ret)?;
                        expected_ret
                    } else {
                        let body_result = self.synth_expr(body)?;
                        body_result.ty
                    };

                    // Exit async scope if this was an async closure
                    if let Some(prev) = prev_async_context {
                        self.borrow_tracker.exit_async_scope();
                        self.in_async_context = prev;
                    }

                    // Exit closure tracking and get capture set
                    let capture_set = self.borrow_tracker.exit_closure(closure_id);

                    // ============================================================
                    // Send/Sync Safety for Async Closures
                    // CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Thread safety
                    // ============================================================
                    // Async closures that capture references must ensure those
                    // references are Send if the closure crosses await points.
                    if let Some(ref cs) = capture_set {
                        if *async_ && !cs.captures.is_empty() {
                            // Check thread safety for captured variables
                            // MutBorrow captures are NOT Send-safe across await points
                            for capture in &cs.captures {
                                if matches!(capture.mode, crate::aliasing::CaptureMode::MutBorrow) {
                                    // Look up the variable's type to check if it's Send
                                    if let Some(var_scheme) = self.ctx.env.lookup(&capture.target) {
                                        let var_ty = var_scheme.instantiate();
                                        let resolved_ty = self.unifier.apply(&var_ty);
                                        // Check if the type is known to be !Send
                                        if self.is_non_send_type(&resolved_ty) {
                                            return Err(TypeError::Other(
                                                verum_common::Text::from(format!(
                                                    "Cannot capture `{}` by mutable reference in async closure: \
                                                 type `{}` is not Send-safe across await points",
                                                    capture.target, resolved_ty
                                                )),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Exit lambda scope
                    self.ctx.exit_scope();

                    // Check if return type depends on parameters (dependent function / Pi type)
                    // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi types
                    let is_dependent = params.len() == 1
                        && self.return_type_depends_on_param(&ret_ty, &param_names);

                    // If async closure, wrap return type in Future<T>
                    let func_ty = if *async_ {
                        let future_ret_ty = Type::Generic {
                            name: verum_common::Text::from("Future"),
                            args: vec![ret_ty].into(),
                        };
                        if let (true, Some(param_name)) =
                            (is_dependent, param_names.get(0).and_then(|p| p.clone()))
                        {
                            // Dependent async function: (x: A) -> Future<B(x)>
                            Type::Pi {
                                param_name,
                                param_type: Box::new(param_types[0].clone()),
                                return_type: Box::new(future_ret_ty),
                            }
                        } else {
                            Type::function(param_types, future_ret_ty)
                        }
                    } else if let (true, Some(param_name)) =
                        (is_dependent, param_names.get(0).and_then(|p| p.clone()))
                    {
                        // Dependent function: (x: A) -> B(x)
                        // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi type introduction
                        Type::Pi {
                            param_name,
                            param_type: Box::new(param_types[0].clone()),
                            return_type: Box::new(ret_ty),
                        }
                    } else {
                        // Regular function: A -> B
                        Type::function(param_types, ret_ty)
                    };

                    // Register closure type in the type registry for codegen
                    // Apply unifier substitution to resolve type variables
                    // This enables closure parameter type inference during LLVM codegen
                    let resolved_func_ty = self.unifier.apply(&func_ty);
                    self.type_registry
                        .register_expr(outer_span, resolved_func_ty);

                    Ok(InferResult::new(func_ty))
    }

    /// Type-infer a `for` loop expression.
    /// Synthesizes the iterator type, resolves the Iterator protocol to get Item,
    /// binds the pattern, checks the body, and returns unit.
    fn infer_for_loop(&mut self, expr: &Expr) -> Result<InferResult> {
        let ExprKind::For { label: _, pattern, iter, body, invariants: _, decreases: _ } = &expr.kind
            else { unreachable!() };
                    // NLL: Before synthesizing the iterator expression, release any
                    // expired borrows on the collection being iterated. This handles
                    // patterns like:
                    //  let slice = &mut array; slice[0] = 10;
                    //  for item in &mut array { ... } // slice's borrow has expired
                    if let ExprKind::Unary { op, expr: inner } = &iter.kind {
                        if matches!(op, UnOp::Ref | UnOp::RefMut) {
                            if let ExprKind::Path(path) = &inner.kind {
                                if let Some(verum_ast::ty::PathSegment::Name(id)) =
                                    path.segments.first()
                                {
                                    let target = verum_common::Text::from(id.name.as_str());
                                    self.borrow_tracker.nll_release_expired_borrows_for(&target);
                                }
                            }
                        }
                    }

                    let iter_result = self.synth_expr(iter)?;

                    // CRITICAL: Apply current substitution to resolve type variables
                    // This fixes: for (a, b) in pairs where pairs: List<(A, B)>
                    let resolved_iter_ty = self.unifier.apply(&iter_result.ty);

                    // =========================================================================
                    // Protocol-based IntoIterator resolution
                    // For-loop desugaring: "for x in iter" desugars to IntoIterator protocol calls (into_iter -> next loop)
                    // =========================================================================
                    let elem_ty = match self
                        .protocol_checker
                        .read()
                        .resolve_into_iterator_protocol(&resolved_iter_ty)
                    {
                        Some(resolution) => resolution.item,
                        None => {
                            // =========================================================================
                            // Fallback 1: Try Iterator protocol's Item associated type
                            // If the type implements Iterator directly (not IntoIterator),
                            // extract the Item type from the Iterator protocol impl.
                            // This handles types like RangeIterator, MapIterator, etc. that
                            // implement Iterator but not IntoIterator.
                            // =========================================================================
                            let iter_item = self.protocol_checker.read().try_find_associated_type(
                                &resolved_iter_ty,
                                &verum_common::Text::from("Item"),
                            );
                            if let Some(item_ty) = iter_item {
                                // Normalize projection types (e.g., ::Item[SomeType] -> ConcreteType)

                                self.protocol_checker
                                    .read()
                                    .normalize_projection_type(&item_ty)
                            } else {
                                // Duck-typing fallback: extract type name from any type representation
                                // and check if it has has_next/next methods in inherent_methods.
                                // This enables `for v in CustomIter { ... }` without requiring
                                // an explicit IntoIterator implementation.
                                let try_duck_type_iter = |ty: &Type| -> Option<Type> {
                                    let type_name: Option<&str> = match ty {
                                        Type::Named { path, .. } => Self::path_type_name(path)
                                            .or_else(|| Self::path_last_type_name(path)),
                                        Type::Generic { name, .. } => Some(name.as_str()),
                                        _ => None,
                                    };
                                    type_name.and_then(|tn| {
                                        let methods_guard = self.inherent_methods.read();
                                        let methods =
                                            methods_guard.get(&verum_common::Text::from(tn))?;
                                        let _has_next =
                                            methods.get(&verum_common::Text::from("has_next"))?;
                                        let next_scheme =
                                            methods.get(&verum_common::Text::from("next"))?;
                                        let next_ty = next_scheme.instantiate();
                                        if let Type::Function { return_type, .. } = &next_ty {
                                            Some(*return_type.clone())
                                        } else {
                                            None
                                        }
                                    })
                                };

                                // Special case for known iterator types and path-like types
                                if let Type::Named { path, .. } = &resolved_iter_ty {
                                    let tname = Self::path_type_name(path)
                                        .or_else(|| Self::path_last_type_name(path))
                                        .unwrap_or("");
                                    if tname == "PathIter" {
                                        Type::Text
                                    } else if tname.contains("Iter")
                                        || tname.contains("iter")
                                        || matches!(
                                            tname,
                                            "Components"
                                                | "ReadDir"
                                                | "Range"
                                                | "Entries"
                                                | "Chunks"
                                                | "Windows"
                                                | "Lines"
                                                | "Chars"
                                                | "Bytes"
                                                | "Keys"
                                                | "Values"
                                                | "Items"
                                                | "DirEntry"
                                        )
                                    {
                                        // Known iterator-like types: return a fresh type variable
                                        // for the element type
                                        Type::Var(TypeVar::fresh())
                                    } else if let Some(elem) = try_duck_type_iter(&resolved_iter_ty)
                                    {
                                        elem
                                    } else {
                                        // Try expanding Named type to its variant definition
                                        // to detect linked-list patterns like Cons/Nil
                                        let expanded =
                                            self.expand_generic_to_variant(&resolved_iter_ty);
                                        if let Type::Variant(ref variants) = expanded {
                                            if let Some(elem) =
                                                Self::infer_linked_list_element_type(variants)
                                            {
                                                elem
                                            } else {
                                                // Named variant type without linked-list pattern:
                                                // allow iteration with a fresh element type for types
                                                // that may implement IntoIterator via protocol impls
                                                Type::Var(TypeVar::fresh())
                                            }
                                        } else {
                                            // Custom named type: assume it implements IntoIterator
                                            // via protocol implementations. Use a fresh element type.
                                            // If it truly isn't iterable, a runtime error will catch it.
                                            Type::Var(TypeVar::fresh())
                                        }
                                    }
                                } else if let Some(elem) = try_duck_type_iter(&resolved_iter_ty) {
                                    // Handles Type::Generic and other types with duck-typed iteration
                                    elem
                                } else if let Type::Generic { name, args } = &resolved_iter_ty {
                                    // Well-known collection types: List<T>->T, Set<T>->T, Map<K,V>->(K,V)
                                    match name.as_str() {
                                        "List" | "Set" | "Deque" | "PriorityQueue" | "BTreeSet"
                                        | "HashSet" | "Queue" | "Stack" | "Stream" => args
                                            .first()
                                            .cloned()
                                            .unwrap_or(Type::Var(TypeVar::fresh())),
                                        "Map" | "BTreeMap" | "HashMap" | "OrderedMap" => {
                                            if args.len() >= 2 {
                                                Type::Tuple(List::from(vec![
                                                    args[0].clone(),
                                                    args[1].clone(),
                                                ]))
                                            } else {
                                                Type::Var(TypeVar::fresh())
                                            }
                                        }
                                        _ => Type::Var(TypeVar::fresh()),
                                    }
                                } else {
                                    // Try unwrapping reference types (&T, &mut T) for duck-typing
                                    let inner_ty = match &resolved_iter_ty {
                                        Type::Reference { inner, .. }
                                        | Type::CheckedReference { inner, .. }
                                        | Type::UnsafeReference { inner, .. } => {
                                            Some(inner.as_ref())
                                        }
                                        _ => None,
                                    };
                                    if let Some(inner) = inner_ty {
                                        if let Some(elem) = try_duck_type_iter(inner) {
                                            elem
                                        } else if let Type::Generic { name, args } = inner {
                                            // Collection types behind references: &List<T>->T, &Set<T>->T
                                            match name.as_str() {
                                                "List" | "Set" | "Deque" | "PriorityQueue"
                                                | "BTreeSet" | "HashSet" | "Queue" | "Stack"
                                                | "Stream" => args
                                                    .first()
                                                    .cloned()
                                                    .unwrap_or(Type::Var(TypeVar::fresh())),
                                                "Map" | "BTreeMap" | "HashMap" | "OrderedMap" => {
                                                    if args.len() >= 2 {
                                                        Type::Tuple(List::from(vec![
                                                            args[0].clone(),
                                                            args[1].clone(),
                                                        ]))
                                                    } else {
                                                        Type::Var(TypeVar::fresh())
                                                    }
                                                }
                                                _ => Type::Var(TypeVar::fresh()),
                                            }
                                        } else {
                                            // For other reference types, use fresh var
                                            Type::Var(TypeVar::fresh())
                                        }
                                    } else if let Type::Variant(variants) = &resolved_iter_ty {
                                        // Heuristic for user-defined linked list types:
                                        // If the variant type has a Cons-like variant with a tuple payload
                                        // and a Nil-like variant with Unit payload, treat it as iterable.
                                        // The element type is inferred from the first element of the Cons tuple.
                                        if let Some(elem) =
                                            Self::infer_linked_list_element_type(variants)
                                        {
                                            elem
                                        } else {
                                            // Fallback: for Variant types, infer element as the variant type itself
                                            // This allows `for x in variant_value { }` to type-check
                                            // (the runtime semantics depend on the user's iterator impl)
                                            return Err(TypeError::Other(
                                                verum_common::Text::from(format!(
                                                    "Cannot iterate over non-iterable type: {}. Type must implement IntoIterator or provide has_next()/next() methods.",
                                                    resolved_iter_ty
                                                )),
                                            ));
                                        }
                                    } else {
                                        // Last resort: try expanding to variant for Generic/other types
                                        let expanded =
                                            self.expand_generic_to_variant(&resolved_iter_ty);
                                        if let Type::Variant(ref variants) = expanded {
                                            if let Some(elem) =
                                                Self::infer_linked_list_element_type(variants)
                                            {
                                                elem
                                            } else {
                                                // For Named/Generic iterator-like types, allow iteration
                                                // with a fresh element type rather than hard-erroring
                                                Type::Var(TypeVar::fresh())
                                            }
                                        } else {
                                            // Fallback: assume custom iterator type, use fresh element type
                                            // This handles types like ZipIter, ∃_. _, etc.
                                            Type::Var(TypeVar::fresh())
                                        }
                                    }
                                }
                            } // end else: no Iterator Item found, duck-typing fallback
                        } // end None branch
                    };

                    // Apply the current substitution to resolve any type variables
                    // that were already unified during iterable expression inference
                    // (e.g., the closure return type in `items.map(|x| x.cmp(0))`
                    // unifies the Item TypeVar with Ordering before we reach here,
                    // so `ord` in the for body must be Ordering, not an opaque TypeVar).
                    let elem_ty = self.unifier.apply(&elem_ty);

                    // Enter loop context for affine tracking
                    self.affine_tracker.enter_loop();

                    // ============================================================
                    // Iterator Invalidation Tracking
                    // CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Iterator safety
                    // ============================================================
                    // Track iterators to prevent modifications to collections
                    // during iteration, which would invalidate the iterator.

                    // Extract collection name from iterable expression for tracking
                    // Handle various iterator expression patterns:
                    // - `for item in items` -> Path(items)
                    // - `for item in &items` -> Unary(Ref, Path(items))
                    // - `for item in &mut items` -> Unary(RefMut, Path(items))
                    // - `for item in items.iter()` -> MethodCall(items, "iter", [])
                    let collection_name: Option<Text> = match &iter.kind {
                        ExprKind::Path(path) => path.segments.first().and_then(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => {
                                Some(verum_common::Text::from(id.name.as_str()))
                            }
                            _ => None,
                        }),
                        // Handle &items and &mut items
                        ExprKind::Unary { op, expr: inner }
                            if matches!(op, UnOp::Ref | UnOp::RefMut) =>
                        {
                            if let ExprKind::Path(path) = &inner.kind {
                                path.segments.first().and_then(|seg| match seg {
                                    verum_ast::ty::PathSegment::Name(id) => {
                                        Some(verum_common::Text::from(id.name.as_str()))
                                    }
                                    _ => None,
                                })
                            } else {
                                None
                            }
                        }
                        // Handle items.iter() and items.iter_mut()
                        ExprKind::MethodCall {
                            receiver, method, ..
                        } if method.name.as_str() == "iter"
                            || method.name.as_str() == "iter_mut" =>
                        {
                            self.extract_receiver_name(receiver)
                        }
                        _ => None,
                    };

                    // Register iterator if we can track the collection
                    let iter_var_name = verum_common::Text::from("__iter");
                    if let Some(ref collection) = collection_name {
                        // Determine if this is a mutable iterator
                        let is_mutable_iter = match &iter.kind {
                            ExprKind::Unary { op, .. } if matches!(op, UnOp::RefMut) => true,
                            ExprKind::MethodCall { method, .. }
                                if method.name.as_str() == "iter_mut" =>
                            {
                                true
                            }
                            _ => false,
                        };
                        let _ = self.borrow_tracker.register_iterator(
                            iter_var_name.clone(),
                            collection.clone(),
                            is_mutable_iter,
                            iter.span,
                        );
                    }

                    self.ctx.enter_scope();
                    self.borrow_tracker.enter_scope();
                    self.refinement_evidence.push_scope();

                    self.bind_pattern(pattern, &elem_ty)?;

                    // Add pattern evidence for for-loop pattern
                    // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Evidence Propagation
                    // For `for Some(x) in items`, we know x is Some within the body
                    if let Maybe::Some(iter_name) = self.extract_simple_var_name(iter) {
                        self.add_pattern_evidence(pattern, iter_name, pattern.span);
                    }

                    self.infer_block(body)?;

                    self.refinement_evidence.pop_scope();
                    self.borrow_tracker.exit_scope();
                    self.ctx.exit_scope();

                    // Release iterator tracking
                    if collection_name.is_some() {
                        self.borrow_tracker.release_iterator(iter_var_name.as_str());
                    }

                    // Exit loop context
                    self.affine_tracker.exit_loop();

                    Ok(InferResult::new(Type::unit()))
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
                        return Err(TypeError::Other(verum_common::Text::from(
                            "select expression can only be used in async context \
                             (async fn, async block, or async closure)",
                        )));
                    }

                    if arms.is_empty() {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "select expression must have at least one arm",
                        )));
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
                                    match self
                                        .protocol_checker
                                        .read()
                                        .resolve_future_protocol(&resolved_ty)
                                    {
                                        Some(resolution) => resolution.output,
                                        None => {
                                            // Also handle JoinHandle<T> as an awaitable type
                                            match &resolved_ty {
                                                Type::Generic { name, args }
                                                    if name.as_str() == "JoinHandle"
                                                        && args.len() == 1 =>
                                                {
                                                    args[0].clone()
                                                }
                                                Type::Named { path, args } if args.len() == 1 => {
                                                    if let Some(ident) = path.as_ident() {
                                                        if ident.as_str() == "JoinHandle" {
                                                            args[0].clone()
                                                        } else {
                                                            // In async context, be lenient for unknown awaitable types
                                                            resolved_ty.clone()
                                                        }
                                                    } else {
                                                        resolved_ty.clone()
                                                    }
                                                }
                                                _ => {
                                                    // In async context, be lenient - type may implement Future
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
                                        }
                                    }
                                } else {
                                    // Fallback: if not an await expr, type the expression directly
                                    // This shouldn't happen with correct parsing, but handle it gracefully
                                    let future_result = self.synth_expr(future_expr)?;
                                    let resolved_ty = self.unifier.apply(&future_result.ty);
                                    match self
                                        .protocol_checker
                                        .read()
                                        .resolve_future_protocol(&resolved_ty)
                                    {
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
                            if let Some(scheme) = self.ctx.env.lookup("self") {
                                let ty = scheme.instantiate();
                                return Ok(InferResult::new(ty));
                            }
                        }
                        return Err(TypeError::Other(
                            "self keyword requires method context".into(),
                        ));
                    }
                    verum_ast::ty::PathSegment::Super => {
                        // super as a standalone path is used as a base for
                        // field access chains like super.module.function().
                        return Ok(InferResult::new(Type::Never));
                    }
                    verum_ast::ty::PathSegment::Cog => {
                        // cog as a standalone path is used as a base for qualified access
                        return Ok(InferResult::new(Type::Never));
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

            // COMPLETE module-level lookup: Try local env first, then module context
            match self.ctx.env.lookup(name) {
                Some(scheme) => {
                    // =====================================================================
                    // DEFINITE ASSIGNMENT: Check variable is initialized before use
                    // Spec: L0-critical/memory-safety/uninitialized
                    // =====================================================================
                    let var_name = verum_common::Text::from(name);
                    self.check_variable_initialized(&var_name, expr.span)?;

                    let ty = scheme.instantiate();
                    // CRITICAL: Apply unifier to resolve type variables
                    // When we have e.g. `wrapper: Wrapper<τ59>` and τ59 was unified with Text,
                    // we need to return `Wrapper<Text>` so field access works correctly.
                    let resolved_ty = self.unifier.apply(&ty);
                    Ok(InferResult::new(resolved_ty))
                }
                None => {
                    // Try module-level function lookup for cross-function inference
                    if let Maybe::Some(scheme) = self.lookup_function_in_module(name) {
                        let ty = scheme.instantiate();
                        let resolved_ty = self.unifier.apply(&ty);
                        Ok(InferResult::new(resolved_ty))
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

    fn infer_expr_call(&mut self, expr: &Expr) -> Result<InferResult> {
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
            let method_result = self.infer_method_call_with_recv_type(
                inner_ty, obj, method, type_args, args, expr.span,
            )?;

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

            return Ok(InferResult::new(result_ty));
        }

        // ============================================================
        // Call Graph Building for Negative Context Verification
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
        // ============================================================
        // Extract function name and record call site for transitive checking
        let callee_name: Option<Text> = match &func.kind {
            ExprKind::Path(path) => path.segments.last().and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(id) => Some(id.name.clone()),
                _ => None,
            }),
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
        // agnostic per `crates/verum_types/src/CLAUDE.md`:
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
                    let old_call_context = self.in_call_arg_context;
                    self.in_call_arg_context = true;
                    for (arg, param_ty) in
                        args.iter().zip(params.iter())
                    {
                        let resolved_param =
                            self.unifier.apply(param_ty);
                        let resolved_param = if Self::contains_type_app(
                            &resolved_param,
                        ) {
                            self.normalize_type(&resolved_param)
                        } else {
                            resolved_param
                        };
                        self.check_expr(arg, &resolved_param)?;
                    }
                    self.in_call_arg_context = old_call_context;
                    self.consume_affine_call_args(args, &params)?;
                    let resolved_return =
                        self.unifier.apply(&return_type);
                    let resolved_return = if Self::contains_type_app(
                        &resolved_return,
                    ) {
                        self.normalize_type(&resolved_return)
                    } else {
                        resolved_return
                    };
                    return Ok(InferResult::new(resolved_return));
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
                    let func_ty = scheme.instantiate();
                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = func_ty
                    {
                        // Check arity
                        if params.len().abs_diff(args.len()) > 1 {
                            return Err(TypeError::Other(verum_common::Text::from(
                                format!(
                                    "{} expects {} argument(s), got {}",
                                    name_str,
                                    params.len(),
                                    args.len()
                                ),
                            )));
                        }
                        // Check arguments
                        // NLL: Set call argument context for proper borrow release
                        let old_call_context = self.in_call_arg_context;
                        self.in_call_arg_context = true;
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            // Normalize TypeApp projections (GAT/HKT) now that
                            // type vars may be resolved from explicit type args.
                            let resolved_param =
                                if Self::contains_type_app(&resolved_param) {
                                    self.normalize_type(&resolved_param)
                                } else {
                                    resolved_param
                                };
                            self.check_expr(arg, &resolved_param)?;
                        }
                        self.in_call_arg_context = old_call_context;
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
                        return Ok(InferResult::new(resolved_return));
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
                return Ok(InferResult::new(Type::Int));
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
                    let awaited = match self
                        .protocol_checker
                        .read()
                        .resolve_future_protocol(&resolved)
                    {
                        Some(resolution) => resolution.output,
                        None => resolved,
                    };
                    awaited_types.push(awaited);
                }
                let tuple_ty = Type::Tuple(awaited_types);
                return Ok(InferResult::new(Type::Future {
                    output: Box::new(tuple_ty),
                }));
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

        let func_result = if let Some(ref redirected_name) = effective_func_name {
            // Look up the redirected function name
            if let Some(scheme) = self.ctx.env.lookup(redirected_name.as_str()).cloned()
            {
                let (ty, _fresh_vars, proto_bounds) =
                    scheme.instantiate_with_protocol_bounds();
                pending_protocol_bounds = proto_bounds;
                InferResult::new(ty)
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
                if let Some(scheme) = self.ctx.env.lookup(name.as_str()).cloned() {
                    let (ty, fresh_vars, implicit_vars) =
                        scheme.instantiate_with_implicit_info();
                    // Also get protocol bounds mapped to fresh vars
                    if !scheme.var_protocol_bounds.is_empty() {
                        let mut old_to_fresh_map: Map<TypeVar, TypeVar> = Map::new();
                        for (orig, fresh) in scheme.vars.iter().zip(fresh_vars.iter()) {
                            old_to_fresh_map.insert(*orig, *fresh);
                        }
                        for (old_var, bounds) in &scheme.var_protocol_bounds {
                            if let Some(fresh_var) = old_to_fresh_map.get(old_var) {
                                pending_protocol_bounds
                                    .insert(*fresh_var, bounds.clone());
                            }
                        }
                    }

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
                            if let verum_ast::ty::GenericArg::Type(ast_ty) = type_arg {
                                let provided_ty = self.ast_to_type(ast_ty)?;
                                // Unify the fresh var with the provided type
                                self.unifier.unify(
                                    &Type::Var(*fresh_var),
                                    &provided_ty,
                                    expr.span,
                                )?;
                            }
                        }
                        explicit_var_index += 1;
                    }

                    // Check that we don't have too many explicit type args
                    let expected_explicit = fresh_vars.len() - implicit_vars.len();
                    if type_args.len() > expected_explicit {
                        return Err(TypeError::Other(verum_common::Text::from(
                            format!(
                                "Function `{}` expects {} explicit type argument{}, got {}",
                                name,
                                expected_explicit,
                                if expected_explicit == 1 { "" } else { "s" },
                                type_args.len()
                            ),
                        )));
                    }

                    InferResult::new(ty)
                } else {
                    self.synth_expr(func)?
                }
            } else {
                self.synth_expr(func)?
            }
        } else {
            // Default path: look up scheme for protocol bounds, then synth_expr for type
            if let Some(ref name) = callee_name {
                if let Some(scheme) = self.ctx.env.lookup(name.as_str()).cloned() {
                    if !scheme.var_protocol_bounds.is_empty() {
                        let (ty, _fresh_vars, proto_bounds) =
                            scheme.instantiate_with_protocol_bounds();
                        pending_protocol_bounds = proto_bounds;
                        InferResult::new(ty)
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
            _ => func_ty_raw,
        };

        match func_ty {
            // Unknown propagation: in stdlib single-file mode,
            // unresolvable callees return Unknown — skip all
            // argument checking and return Unknown.
            Type::Unknown if self.stdlib_single_file_mode => {
                Ok(InferResult::new(Type::Unknown))
            }

            // Var propagation: unresolved type variables from
            // lenient-resolved generic functions skip checking.
            Type::Var(_) if self.stdlib_single_file_mode => {
                Ok(InferResult::new(Type::Unknown))
            }

            // Never propagation: calling Never returns Never
            Type::Never => Ok(InferResult::new(Type::Never)),

            // Pi type application: beta reduction for dependent functions
            // Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 - Type-level computation
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => {
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
                let result_ty =
                    self.substitute_term_in_type(&return_type, &param_name, &arg_term);

                Ok(InferResult::new(result_ty))
            }

            Type::Function {
                params,
                return_type,
                contexts,
                ..
            } => {
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
                            "List.of" | "Set.of" | "min" | "max" |
                            "join" | "concat" |
                            // Meta reflection builtins accept variable args
                            // (e.g. type_name(Int, Text) for tuple type)
                            "type_name" | "type_id" | "size_of" | "align_of" |
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
                            self.function_required_params.get(name).copied()
                        })
                        .unwrap_or(params.len());
                    let capped = from_registry.min(params.len());
                    // Variadic functions accept 0 or more args
                    if is_variadic { 0 } else { capped }
                };

                let total_params = params.len();
                let provided_args = args.len();

                let has_explicit_type_args = !type_args.is_empty();
                if provided_args < required_params
                    && !has_explicit_type_args
                    && !self.stdlib_single_file_mode
                {
                    return Err(TypeError::OtherWithCodeSpanned {
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
                    });
                }

                // In unsafe contexts, arity mismatches are often caused by
                // FFI function name collisions (e.g., extern `read` vs method `read`).
                // Allow extra args in unsafe contexts to avoid false positives.
                //

                // Also allow extra args when the function is generic and arguments
                // look like type names (PascalCase). This handles meta intrinsics
                // like `stride_of(U8)` where the stdlib declares `stride_of<T>()`
                // but the call passes the type as a regular arg.
                let all_args_are_type_like = args.iter().all(|arg| {
                    if let ExprKind::Path(path) = &arg.kind {
                        if let Some(ident) = path.as_ident() {
                            let name = ident.as_str();
                            name.chars()
                                .next()
                                .map(|c| c.is_ascii_uppercase())
                                .unwrap_or(false)
                        } else {
                            false
                        }
                    } else {
                        matches!(&arg.kind, ExprKind::TypeExpr(_))
                    }
                });
                let is_generic_type_call =
                    has_explicit_type_args || all_args_are_type_like;
                if provided_args > total_params
                    && !is_variadic
                    && !self.in_unsafe_context
                    && !is_generic_type_call
                    && !self.stdlib_single_file_mode
                {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Function accepts at most {} argument{}, got {}",
                        total_params,
                        if total_params == 1 { "" } else { "s" },
                        provided_args
                    ))));
                }

                // ============================================================
                // Context Satisfaction Check (ContextChecker integration)
                // Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
                // ============================================================
                // When calling a function with context requirements, verify that
                // the current function's contexts satisfy those requirements.
                if let Some(ref callee_contexts) = contexts {
                    // Build a ContextSet from the callee's requirements
                    let mut callee_context_set = ContextSet::new();
                    for ctx_ref in callee_contexts.iter() {
                        callee_context_set.add(ContextRequirement::new(
                            ctx_ref.name.clone(),
                            expr.span,
                        ));
                    }

                    // Get the current function's available contexts
                    let caller_context_set =
                        self.current_function_contexts.clone().unwrap_or_default();

                    // Use ContextChecker to validate satisfaction
                    self.context_checker.check_context_satisfaction(
                        &callee_context_set,
                        &caller_context_set,
                        expr.span,
                    )?;
                }

                // Check capability requirements if function has contexts
                // Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0 - Capability Attenuation
                if let Some(ref req) = contexts {
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
                                use crate::capability::{
                                    CapabilityRequirement, TypeCapabilitySet,
                                };

                                // Extract function name for heuristic analysis
                                let func_name = if let ExprKind::Path(path) = &func.kind
                                {
                                    path.segments
                                        .last()
                                        .and_then(|seg| match seg {
                                            verum_ast::ty::PathSegment::Name(id) => {
                                                Some(id.name.as_str())
                                            }
                                            _ => None,
                                        })
                                        .unwrap_or("function")
                                } else {
                                    "function"
                                };

                                // Use the method capability mapper to extract requirements
                                // For function calls, we use the function name as a hint
                                let context_decl =
                                    self.context_declarations.get(context_name);
                                let required_caps = self
                                    .method_capability_mapper
                                    .extract_method_capabilities(
                                        context_name,
                                        &verum_common::Text::from(func_name),
                                        context_decl,
                                    );

                                let requirement = CapabilityRequirement::new(
                                    context_name.clone(),
                                    required_caps.clone(),
                                    verum_common::Text::from(format!(
                                        "function call to {}",
                                        func_name
                                    )),
                                );

                                if let Err(cap_err) = self
                                    .capability_checker
                                    .check_requirement(&requirement)
                                {
                                    // E0306: Capability violation
                                    use verum_diagnostics::capability_attenuation_errors::CapabilityViolationError;

                                    let func_name = if let ExprKind::Path(path) =
                                        &func.kind
                                    {
                                        path.segments
                                            .iter()
                                            .filter_map(|seg| match seg {
                                                verum_ast::ty::PathSegment::Name(
                                                    id,
                                                ) => Some(id.name.as_str()),
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
                                        cap_names
                                            .iter()
                                            .map(|s| s.as_str().into())
                                            .collect(),
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

                                let func_name = if let ExprKind::Path(path) = &func.kind
                                {
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

                                let diagnostic = CapabilityNotProvidedError::new(
                                    context_name.as_str(),
                                    span_to_line_col(func.span),
                                )
                                .with_function_name(func_name.as_str())
                                .build();

                                self.diagnostics.push(diagnostic);

                                return Err(TypeError::Other(
                                    verum_common::Text::from(format!(
                                        "Function requires context '{}' which is not provided",
                                        context_name
                                    )),
                                ));
                            }
                        }
                    }
                }

                // ============================================================
                // Function Parameter Borrow Checking
                // Reference safety invariants: managed refs validated at dereference, checked refs proven safe at compile time, unsafe refs unchecked — Aliasing Rules
                // ============================================================
                // Check for potential aliasing conflicts between function arguments:
                // - Multiple mutable references to the same data
                // - Mutable reference + immutable reference to overlapping data
                self.check_function_arg_aliasing(args, &params, expr.span)?;

                // Check each argument against parameter type with substitution
                // NLL: Set call argument context for proper borrow release
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                #[cfg(debug_assertions)]
                let is_assert_eq = func_name
                    .as_ref()
                    .map(|n| n.as_str() == "assert_eq")
                    .unwrap_or(false);

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
                    .and_then(|name| self.function_param_names.get(name).cloned())
                    .unwrap_or_default();

                for (i, (arg, param_ty)) in args.iter().zip(params.iter()).enumerate() {
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
                    let check_ty =
                        if let Type::Refined { base, predicate } = &resolved_param {
                            if !dep_param_names.is_empty() && i > 0 {
                                let mut subst_pred = predicate.clone();
                                for j in 0..i {
                                    let earlier_name = match dep_param_names.get(j) {
                                        Some(n) if !n.as_str().is_empty() => n.clone(),
                                        _ => continue,
                                    };
                                    let arg_expr = match args.get(j) {
                                        Some(a) => a.clone(),
                                        None => continue,
                                    };
                                    subst_pred.predicate =
                                        self.refinement.substitute_in_expr(
                                            &subst_pred.predicate,
                                            &earlier_name,
                                            &arg_expr,
                                        );
                                }
                                // Constant-fold arithmetic that became pure after
                                // substitution. Without this, a predicate like
                                // `count <= src_len - offset` becomes
                                // `count <= 10 - 5` after substituting
                                // `src_len → 10, offset → 5`, and the syntactic
                                // refinement checker cannot decide it because it
                                // does not reduce `10 - 5` to `5`. Folding here
                                // produces `count <= 5`, which the checker can
                                // then decide against any concrete `count`.
                                subst_pred.predicate =
                                    Self::const_fold_expr(&subst_pred.predicate);
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

                    #[cfg(debug_assertions)]
                    if is_assert_eq {
                        // eprintln!("[DEBUG assert_eq] arg[{}]: param_ty={}, resolved_param={}, arg.span={:?}",
                        // i, param_ty, resolved_param, arg.span);
                    }
                    self.check_expr(arg, &check_ty)?;
                    #[cfg(debug_assertions)]
                    if is_assert_eq {
                        let resolved_after = self.unifier.apply(param_ty);
                        // eprintln!("[DEBUG assert_eq] after check arg[{}]: resolved_param_after={}", i, resolved_after);
                    }
                }
                self.in_call_arg_context = old_call_context;

                // Consume affine values passed by value
                self.consume_affine_call_args(args, &params)?;

                // ============================================================
                // Protocol Bound Checking at Call Sites
                // ============================================================
                // After argument unification, resolve type vars to concrete
                // types and verify protocol bounds are satisfied.
                for (type_var, bounds) in &pending_protocol_bounds {
                    let resolved_ty = self.unifier.apply(&Type::Var(*type_var));
                    // Only check bounds when the type var resolved to a concrete type
                    // (not still a type variable — those are checked at their own call sites)
                    if !matches!(resolved_ty, Type::Var(_)) {
                        if let Err(_e) = self
                            .protocol_checker
                            .read()
                            .check_bounds(&resolved_ty, bounds)
                        {
                            // Emit diagnostic but don't hard-error — some stdlib impls
                            // may not yet be loaded depending on compilation order.
                            // The runtime will catch actual violations.
                            tracing::debug!("Protocol bound check warning: {}", _e);
                        }
                    }
                }

                // ============================================================
                // Interprocedural Escape Analysis for &checked References
                // Spec: L0-critical/reference_system/reference_tiers/checked_promotion_fail
                // ============================================================
                // When calling a function that returns a reference, check if any
                // argument is a &checked reference to a local variable.
                // Such references could escape through the function's return value,
                // violating the safety guarantee of &checked (0ns overhead, no runtime checks).
                let resolved_return = self.unifier.apply(&return_type);
                // Normalize TypeApp in return type (GAT/HKT)
                let resolved_return = if Self::contains_type_app(&resolved_return) {
                    self.normalize_type(&resolved_return)
                } else {
                    resolved_return
                };
                // Resolve top-level associated type projections (e.g., ::Item[ListIter<Int>] → &Int)
                let resolved_return =
                    if let Type::Generic { name, args } = &resolved_return {
                        if name.as_str().starts_with("::") && !args.is_empty() {
                            let assoc_name = &name.as_str()[2..];
                            let assoc_text: Text = assoc_name.into();
                            self.protocol_checker
                                .read()
                                .try_find_associated_type(&args[0], &assoc_text)
                                .unwrap_or(resolved_return)
                        } else {
                            resolved_return
                        }
                    } else {
                        resolved_return
                    };
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
            Type::Var(v) => {
                // Type variable being called as a function - instantiate it as a function type
                // Create fresh type variables for parameters based on argument count
                let mut param_types = List::new();
                for _ in args.iter() {
                    param_types.push(Type::Var(TypeVar::fresh()));
                }

                let ret_ty = Type::Var(TypeVar::fresh());
                let func_ty_inst = Type::function(param_types.clone(), ret_ty.clone());

                // Unify the type variable with the function type
                self.unifier
                    .unify(&Type::Var(v), &func_ty_inst, func.span)?;

                // Now check arguments against parameter types
                // NLL: Set call argument context for proper borrow release
                let old_call_context = self.in_call_arg_context;
                self.in_call_arg_context = true;
                for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                    self.check_expr(arg, param_ty)?;
                }
                self.in_call_arg_context = old_call_context;

                Ok(InferResult::new(ret_ty))
            }

            // ============================================================
            // Rank-2 Polymorphic Function Call (Forall instantiation)
            // Spec: grammar/verum.ebnf - rank2_function_type
            // ============================================================
            // When calling a rank-2 polymorphic function (∀R. fn(...) -> ...),
            // instantiate the quantified type variables with fresh inference
            // variables and then type-check as a normal function call.
            Type::Forall { ref vars, ref body } => {
                // Build substitution map: quantified var -> fresh TypeVar
                let mut subst: Map<TypeVar, Type> = Map::new();
                for qvar in vars.iter() {
                    let fresh = TypeVar::fresh();
                    subst.insert(*qvar, Type::Var(fresh));
                }

                // Apply substitution to the body to get the instantiated function type
                let instantiated_body = self.substitute_type_vars(body, &subst);

                // Now the body should be a Function type - extract and check
                match instantiated_body {
                    Type::Function {
                        params,
                        return_type,
                        contexts,
                        ..
                    } => {
                        // Check arity
                        if params.len().abs_diff(args.len()) > 1 {
                            return Err(TypeError::Other(verum_common::Text::from(
                                format!(
                                    "Rank-2 function expects {} argument(s), got {}",
                                    params.len(),
                                    args.len()
                                ),
                            )));
                        }

                        // Context satisfaction check
                        if let Some(ref callee_contexts) = contexts {
                            let mut callee_context_set = ContextSet::new();
                            for ctx_ref in callee_contexts.iter() {
                                callee_context_set.add(ContextRequirement::new(
                                    ctx_ref.name.clone(),
                                    expr.span,
                                ));
                            }

                            let caller_context_set = self
                                .current_function_contexts
                                .clone()
                                .unwrap_or_default();

                            self.context_checker.check_context_satisfaction(
                                &callee_context_set,
                                &caller_context_set,
                                expr.span,
                            )?;
                        }

                        // Check each argument against parameter type
                        let old_call_context = self.in_call_arg_context;
                        self.in_call_arg_context = true;
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }
                        self.in_call_arg_context = old_call_context;

                        let resolved_return = self.unifier.apply(&return_type);
                        Ok(InferResult::new(resolved_return))
                    }
                    _ => Err(TypeError::NotAFunction {
                        ty: func_ty.to_text(),
                        span: func.span,
                    }),
                }
            }

            _ => Err(TypeError::NotAFunction {
                ty: func_ty.to_text(),
                span: func.span,
            }),
        }
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

    fn infer_expr_field(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Field { expr: obj, field } = &expr.kind else { unreachable!() };
        // Special case: Module namespace navigation
        // When obj is a module namespace (e.g., `api`), field access navigates into the module
        // This handles paths like `api.v2.func()` where parser creates Field expressions
        // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
        if let ExprKind::Path(path) = &obj.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
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

            // Mount-aliased module: `mount X.Y.Z;` (or `mount X.Y as Z;`)
            // makes `Z` a module-namespace receiver, so `Z.field`
            // resolves through qualified-name lookup
            // `<full-path>.field` rather than synth_expr'ing `Z` as
            // a value (which fails with `UnboundVariable: Z`).
            //
            // Mirrors the equivalent method-call dispatch path
            // (`modules.rs` ~line 17914) — the two paths together
            // cover both `Z.field` (here) and `Z.method(args)` (there)
            // forms of bare-mount module-qualified access.  Closes
            // the typechecker-side half of task #121.
            if let Some(module_path) = self
                .module_aliases
                .get(&verum_common::Text::from(obj_name))
                .cloned()
            {
                let field_name = field.name.as_str();
                let qualified: verum_common::Text =
                    format!("{}.{}", module_path, field_name).into();
                // The qualified function/const may already be in env
                // via the cross-module import pass that ran for the
                // archive-side load.
                if let Some(scheme) = self.ctx.env.lookup(&qualified) {
                    let ty = scheme.instantiate();
                    return Ok(InferResult::new(self.unifier.apply(&ty)));
                }
                if let Maybe::Some(scheme) = self.lookup_function_in_module(qualified.as_str()) {
                    let ty = scheme.instantiate();
                    return Ok(InferResult::new(self.unifier.apply(&ty)));
                }
                // **Type.method canonical-form fallback**: when `mount
                // core.X.Y.Z.T;` registers `T` as a module-alias
                // mapping to the path `core.X.Y.Z.T`, but `T` is
                // actually a TYPE (not a submodule), the qualified
                // form `core.X.Y.Z.T.method` misses both env and
                // module-fn lookups. The function is actually
                // registered under the bare `T.method` canonical form
                // by `register_coercion_markers_from_metadata` +
                // archive_ctx_loader's Type.method-form-registration
                // path (commit 74312ed4e). Probe that form here so
                // `Map.new()` / `Reservoir.new()` / etc. resolve when
                // the user has multi-mounted sibling collection types
                // that pushed this site into the module-alias arm.
                //
                // Pre-fix 10 tests failed with:
                //   `unbound variable: core.collections.map.Map.new`
                // when `mount core.collections.map.Map` appeared
                // alongside any sibling collection mount (List/Set).
                let type_method_form: verum_common::Text = {
                    let last_seg = module_path
                        .as_str()
                        .rsplit('.')
                        .next()
                        .unwrap_or(module_path.as_str());
                    format!("{}.{}", last_seg, field_name).into()
                };
                if type_method_form != qualified {
                    if let Some(scheme) = self.ctx.env.lookup(&type_method_form) {
                        let ty = scheme.instantiate();
                        return Ok(InferResult::new(self.unifier.apply(&ty)));
                    }
                    if let Maybe::Some(scheme) =
                        self.lookup_function_in_module(type_method_form.as_str())
                    {
                        let ty = scheme.instantiate();
                        return Ok(InferResult::new(self.unifier.apply(&ty)));
                    }
                }
                // Field-access fallback: lookup the simple field
                // name in env as a last-ditch effort.  Same shape
                // as the inline-module bail-out below.
                if let Some(scheme) = self.ctx.env.lookup(field_name) {
                    let ty = scheme.instantiate();
                    return Ok(InferResult::new(self.unifier.apply(&ty)));
                }
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
        {
            let type_name = ident.name.as_str();
            let field_name = field.name.as_str();
            let qualified_name = format!("{}.{}", type_name, field_name);

            // Strategy 1: Try to look up the qualified variant name in env
            if let Some(scheme) = self.ctx.env.lookup(&qualified_name) {
                let ty = scheme.instantiate();
                return Ok(InferResult::new(ty));
            }

            // Strategy 2: Look up type name and check if it's a Variant
            // This handles cross-file imported variant types like RegistryError
            if let Maybe::Some(ty) = self.ctx.lookup_type(type_name)
                && let Type::Variant(variants) = &ty
                && let Some(payload_ty) = variants.get(field_name)
            {
                // Found variant constructor - return function type
                if matches!(payload_ty, Type::Unit) {
                    // Nullary variant - return the variant type itself
                    return Ok(InferResult::new(ty.clone()));
                } else {
                    // Constructor function: fn(payload_ty) -> VariantType
                    let params = match payload_ty {
                        Type::Tuple(tuple_types) => tuple_types.clone(),
                        _ => {
                            let mut p = List::new();
                            p.push(payload_ty.clone());
                            p
                        }
                    };
                    let constructor_ty = Type::function(params, ty.clone());
                    return Ok(InferResult::new(constructor_ty));
                }
            }
        }

        // Handle lowercase type alias property access (e.g., i32.min, u64.max)
        if let ExprKind::Path(path) = &obj.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
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
        // Track the variable name for affine tracking (if applicable)
        let var_name_for_affine = if let ExprKind::Path(path) = &obj.kind {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    let var_name = verum_common::Text::from(id.name.as_str());
                    let field_name_text = verum_common::Text::from(field.name.as_str());
                    // Check if this specific field is initialized
                    self.check_field_initialized(
                        &var_name,
                        &field_name_text,
                        expr.span,
                    )?;

                    // PARTIAL MOVE: Check if this field has already been moved
                    // Spec: L0-critical/reference_system/value_transfer - Partial move tracking
                    // If a field was moved out of a container, subsequent access should fail
                    if !self
                        .affine_tracker
                        .can_access_field(id.name.as_str(), field.name.as_str())
                    {
                        return Err(TypeError::PartiallyMovedValue {
                            name: var_name.clone(),
                            moved_field: field_name_text,
                            moved_at: expr.span,
                            used_at: expr.span,
                        });
                    }

                    Some(id.name.as_str().to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let field_name_str = field.name.as_str().to_string();

        // Normal field access: synthesize the object type and access its field
        // Note: synth_expr will check full initialization for non-field access
        let obj_result = self.synth_expr_for_field_access(obj)?;

        // CRITICAL FIX: Apply unifier to resolve type variables before field access
        // When we have `Wrapper<τ59>` and τ59 was unified with Text, we need to
        // resolve to `Wrapper<Text>` so field access gets the correct field type.
        let resolved_ty = self.unifier.apply(&obj_result.ty);

        // CRITICAL FIX: Unwrap reference types before field access
        // When we have `&Point`, we need to dereference to get `Point` first
        // This handles &T, &checked T, &unsafe T, and nested references
        let dereferenced_ty = self.unwrap_reference_type(&resolved_ty);

        // CRITICAL FIX: Normalize type to resolve type aliases before field access
        // When we have a type alias like `type CreateTokenRequestDto is { name: Text, ... }`,
        // we need to resolve it to the underlying Record type before checking fields
        let normalized_ty = self.normalize_type(dereferenced_ty);

        // Never propagation: any field access on Never produces Never
        if matches!(normalized_ty, Type::Never) {
            return Ok(InferResult::new(Type::Never));
        }

        // Unwrap refined types to access base type for field access.
        // A refined record { x: Int, y: Int }{predicate} should allow .x and .y access.
        let normalized_ty = match &normalized_ty {
            Type::Refined { base, .. } => (**base).clone(),
            other => other.clone(),
        };

        match &normalized_ty {
            Type::Record(fields) => {
                // Regular record field access
                if let Some(field_ty) =
                    fields.get(&verum_common::Text::from(field.name.as_str()))
                {
                    // Track affine field access (partial move tracking)
                    if let Some(ref var_name) = var_name_for_affine {
                        self.track_affine_field_access(
                            var_name,
                            &field_name_str,
                            field_ty,
                            expr.span,
                        )?;
                    }
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
                    let resolved_ty =
                        self.substitute_type_params(field_ty, &param_subst);
                    // Track affine field access (partial move tracking)
                    if let Some(ref var_name) = var_name_for_affine {
                        self.track_affine_field_access(
                            var_name,
                            &field_name_str,
                            &resolved_ty,
                            expr.span,
                        )?;
                    }
                    return Ok(InferResult::new(resolved_ty));
                }

                // Also try looking up the type directly (backward compat)
                if let Option::Some(Type::Record(fields)) =
                    self.ctx.lookup_type(type_name.as_str())
                    && let Some(field_ty) = fields.get(&field_name)
                {
                    // Apply type parameter substitution to get concrete field type
                    let resolved_ty =
                        self.substitute_type_params(field_ty, &param_subst);
                    // Track affine field access (partial move tracking)
                    if let Some(ref var_name) = var_name_for_affine {
                        self.track_affine_field_access(
                            var_name,
                            &field_name_str,
                            &resolved_ty,
                            expr.span,
                        )?;
                    }
                    return Ok(InferResult::new(resolved_ty));
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
                            let ty = scheme.instantiate();
                            return Ok(InferResult::new(ty));
                        }
                        // Check for built-in type properties (size, alignment, stride, etc.)
                        // These are valid for ALL types as compile-time metadata
                        match field.name.as_str() {
                            "size" | "align" | "alignment" | "stride" | "bits" => {
                                return Ok(InferResult::new(Type::int()));
                            }
                            "name" => {
                                return Ok(InferResult::new(Type::text()));
                            }
                            _ => {}
                        }
                        // Field not found - for Named types, the field may exist
                        // in the actual type definition but not be visible due to
                        // module resolution. Return a fresh type variable to allow
                        // type inference to continue.
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
                // Check type properties first (size, align, alignment, stride, bits, name)
                match field.name.as_str() {
                    "size" | "align" | "alignment" | "stride" | "bits" => {
                        return Ok(InferResult::new(Type::int()));
                    }
                    "name" => {
                        return Ok(InferResult::new(Type::text()));
                    }
                    _ => {}
                }
                // Look up as "Type.field" in the environment
                let const_name = format!("{}.{}", normalized_ty, field.name);
                if let Some(scheme) = self.ctx.env.lookup(&const_name).cloned() {
                    let ty = scheme.instantiate();
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
                    let ty = scheme.instantiate();
                    return Ok(InferResult::new(ty));
                }
                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Associated constant {} not found on type {}",
                    field.name, normalized_ty
                ))))
            }
            // CRITICAL: Handle user-defined `Text` struct field access.
            // text.vr defines `public type Text is { ptr, len, cap }`. When the type
            // checker resolves `TypeKind::Text` to `Type::Text` (the primitive), direct
            // field accesses inside `implement Text` fail with E103. Check for
            // user-registered struct fields before emitting the error.
            Type::Text => {
                let struct_key = "__struct_fields_Text".to_string();
                if let Some(Type::Record(fields)) = self.ctx.lookup_type(&struct_key) {
                    let field_name_key = verum_common::Text::from(field.name.as_str());
                    if let Some(field_ty) = fields.get(&field_name_key).cloned() {
                        return Ok(InferResult::new(field_ty));
                    }
                    // Field not found in user-defined struct
                    let available: Vec<&str> =
                        fields.keys().map(|k| k.as_str()).collect();
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "field '{}' not found in Text struct. Available: [{}]",
                        field.name,
                        available.join(", ")
                    ))));
                }
                // Fall through to default handling
                match field.name.as_str() {
                    "size" | "align" | "alignment" | "stride" | "bits" => {
                        Ok(InferResult::new(Type::int()))
                    }
                    "name" => Ok(InferResult::new(Type::text())),
                    _ => Err(TypeError::OtherWithCode {
                        code: verum_common::Text::from("E103"),
                        msg: verum_common::Text::from(format!(
                            "Cannot access field '{}' on non-record type: {}",
                            field.name, normalized_ty
                        )),
                    }),
                }
            }
            // Handle type properties for ALL types: T.size, T.align, T.alignment, T.stride, T.bits
            // These are valid for any type as compile-time type metadata
            _ => {
                match field.name.as_str() {
                    "size" | "align" | "alignment" | "stride" | "bits" => {
                        // Type properties return Int (the size/alignment value)
                        Ok(InferResult::new(Type::int()))
                    }
                    "name" => {
                        // Type name property returns Text
                        Ok(InferResult::new(Type::text()))
                    }
                    "min" | "max"
                        if {
                            let pc = self.protocol_checker.read();
                            pc.implements_protocol(&normalized_ty, "Numeric")
                        } || matches!(normalized_ty, Type::Int | Type::Float) =>
                    {
                        // min/max return the type itself for Numeric types
                        Ok(InferResult::new(normalized_ty.clone()))
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
                                    let ty = scheme.instantiate();
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
                                // Stdlib-agnostic per CLAUDE.md: lookup
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
                                            scheme.instantiate(),
                                        ));
                                    }
                                } else {
                                    drop(methods_guard);
                                }
                            }
                        }
                        {
                            // For unresolved/unknown types and Variant types,
                            // allow field access with a fresh type variable
                            if matches!(
                                &normalized_ty,
                                Type::Var(_) | Type::Unknown | Type::Placeholder { .. }
                            ) {
                                return Ok(InferResult::new(Type::Var(
                                    TypeVar::fresh(),
                                )));
                            }
                            // For Variant types, the field may be on the inner record
                            if matches!(&normalized_ty, Type::Variant(_)) {
                                return Ok(InferResult::new(Type::Var(
                                    TypeVar::fresh(),
                                )));
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
                if index_value < 0 {
                    return Err(TypeError::InvalidIndex {
                        message: verum_common::Text::from(format!(
                            "Array index out of bounds: negative index {}",
                            index_value
                        )),
                        span: index.span,
                    });
                }
                // Only check upper bound for non-variable paths (literals, etc.)
                // Variable paths may have been resized via push/pop
                if !arr_is_mutable_variable && index_value as u64 >= array_size {
                    return Err(TypeError::InvalidIndex {
                        message: verum_common::Text::from(format!(
                            "Array index out of bounds: index {} exceeds array length {}",
                            index_value, array_size
                        )),
                        span: index.span,
                    });
                }
            }
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
        let inner_ty = match &arr_result.ty {
            Type::Reference { inner, .. } => Some(inner.as_ref()),
            Type::CheckedReference { inner, .. } => Some(inner.as_ref()),
            Type::UnsafeReference { inner, .. } => Some(inner.as_ref()),
            Type::Ownership { inner, .. } => Some(inner.as_ref()),
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
        // that may implement Index via protocol implementations
        match &arr_result.ty {
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

    fn infer_expr_return_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Return(val) = &expr.kind else { unreachable!() };
        if let Some(v) = val {
            // Type check the return value (for error messages)
            let val_result = self.synth_expr(v)?;

            // ============================================================
            // Return Value Lifetime Validation
            // Return reference validation: ensuring returned references do not outlive their referents via escape analysis — Dangling references
            // ============================================================
            // Check if we're returning a reference to a local variable.
            // This would create a dangling reference when the function returns.
            if self.is_reference_type(&val_result.ty) {
                self.check_return_lifetime(v, expr.span)?;
            }
        }
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
                let array_size = match self.const_eval.eval(count) {
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
        self.check_cast(&expr_result.ty, &target_ty, expr.span)?;

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
        if path.segments.len() == 2
            && let (
                verum_ast::ty::PathSegment::Name(type_ident),
                verum_ast::ty::PathSegment::Name(variant_ident),
            ) = (&path.segments[0], &path.segments[1])
        {
            let type_name = type_ident.name.as_str();
            let variant_name = variant_ident.name.as_str();

            // Look up the type to see if it's a variant type
            if let Option::Some(ty) = self.ctx.lookup_type(type_name).cloned()
                && let Type::Variant(variants) = &ty
            {
                // It's a variant type! Look up the specific variant
                if let Some(variant_payload_ty) = variants.get(variant_name) {
                    // Verify the payload is a record type
                    if let Type::Record(expected_field_types) = variant_payload_ty {
                        // Clone to avoid borrowing issues
                        let expected_field_types = expected_field_types.clone();
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
                                    self.check_expr(value_expr, expected_field_ty)?;
                                    expected_field_ty.clone()
                                } else {
                                    match self.ctx.env.lookup(field_name) {
                                        Some(scheme) => {
                                            let var_ty = scheme.instantiate();
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

            // Per the architectural rule in crates/verum_types/src/CLAUDE.md —
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
            let has_matching_struct = if let Option::Some(Type::Record(struct_fields)) =
                self.ctx.lookup_type(struct_key.as_str())
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
                                        self.check_expr(value_expr, expected_field_ty)?;
                                        expected_field_ty.clone()
                                    } else {
                                        match self.ctx.env.lookup(field_name) {
                                            Some(var_scheme) => {
                                                let var_ty = var_scheme.instantiate();
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

        // Step 2: Extract expected field types from record definition
        // Clone to avoid borrow issues
        let expected_fields: indexmap::IndexMap<verum_common::Text, Type> =
            match &record_ty {
                Type::Record(field_types) => field_types.clone(),
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
                                    Option::Some(Type::Named { .. }) => {
                                        // Type is nominal - infer fields structurally but return Named type
                                        // This handles types defined in other modules where __struct_fields_ isn't registered
                                        let base_maybe = base
                                            .as_ref()
                                            .map(|b| Heap::new((**b).clone()));
                                        let _inferred = self.infer_structural_record(
                                            fields,
                                            &base_maybe,
                                            expr.span,
                                        )?;
                                        // Return the Named type (nominal) instead of the structural record
                                        return Ok(InferResult::new(record_ty.clone()));
                                    }
                                    Option::Some(other_ty) => {
                                        return Err(TypeError::Other(
                                            verum_common::Text::from(format!(
                                                "Expected record type, found: {}",
                                                other_ty
                                            )),
                                        ));
                                    }
                                    Option::None => {
                                        // Not a pre-defined type but path has a type name
                                        // Infer fields structurally but return Named type (nominal)
                                        let base_maybe = base
                                            .as_ref()
                                            .map(|b| Heap::new((**b).clone()));
                                        let _inferred = self.infer_structural_record(
                                            fields,
                                            &base_maybe,
                                            expr.span,
                                        )?;
                                        // Return Named type to preserve nominal type identity
                                        return Ok(InferResult::new(record_ty.clone()));
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
        // NOTE: Use resolved type name (handles Self -> actual type)
        let type_name = if let Type::Named {
            path: resolved_path,
            ..
        } = &record_ty
        {
            self.path_to_string(resolved_path)
        } else {
            self.path_to_string(path)
        };

        // Try to look up type parameters - first try full path, then try simple name
        let type_params_key = format!("__type_params_{}", type_name);
        let type_params_lookup =
            self.ctx.lookup_type(type_params_key.as_str()).or_else(|| {
                // CRITICAL FIX: If full path lookup fails, try simple name (last segment)
                // Type parameters are registered under simple names, but paths may be qualified
                let simple_name = if let Some(last_dot) = type_name.as_str().rfind('.')
                {
                    &type_name.as_str()[last_dot + 1..]
                } else {
                    type_name.as_str()
                };
                let simple_key = format!("__type_params_{}", simple_name);
                self.ctx.lookup_type(simple_key.as_str())
            });

        let (expected_fields, type_param_vars): (
            indexmap::IndexMap<verum_common::Text, Type>,
            List<Type>,
        ) = if let Option::Some(Type::Record(params_map)) = type_params_lookup {
            // This type has type parameters - instantiate with fresh variables
            let type_params: List<verum_common::Text> =
                params_map.keys().cloned().collect();
            if !type_params.is_empty() {
                // Create fresh type variables for each parameter
                let mut param_subst = indexmap::IndexMap::new();
                let mut fresh_vars = List::new();
                for param_name in type_params.iter() {
                    let fresh_var = Type::Var(TypeVar::fresh());
                    fresh_vars.push(fresh_var.clone());
                    param_subst.insert(param_name.clone(), fresh_var);
                }
                // Apply substitution to all field types
                let mut resolved_fields = indexmap::IndexMap::new();
                for (fname, fty) in expected_fields.iter() {
                    let resolved_ty = self.substitute_type_params(fty, &param_subst);
                    // CRITICAL FIX: Also resolve any placeholder types (forward references)
                    let resolved_ty = self.substitute_placeholders(&resolved_ty);
                    resolved_fields.insert(fname.clone(), resolved_ty);
                }
                (resolved_fields, fresh_vars)
            } else {
                // Non-generic type: still need to resolve placeholders
                let mut resolved_fields = indexmap::IndexMap::new();
                for (fname, fty) in expected_fields.iter() {
                    let resolved_ty = self.substitute_placeholders(fty);
                    resolved_fields.insert(fname.clone(), resolved_ty);
                }
                (resolved_fields, List::new())
            }
        } else {
            // No type parameters: still need to resolve placeholders
            let mut resolved_fields = indexmap::IndexMap::new();
            for (fname, fty) in expected_fields.iter() {
                let resolved_ty = self.substitute_placeholders(fty);
                resolved_fields.insert(fname.clone(), resolved_ty);
            }
            (resolved_fields, List::new())
        };

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
                self.check_expr(value_expr, expected_field_ty)?;
                expected_field_ty.clone()
            } else {
                // Shorthand: lookup variable in environment
                match self.ctx.env.lookup(field_name) {
                    Some(scheme) => {
                        let var_ty = scheme.instantiate();
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

        // Return the record type with resolved type arguments
        // Apply unifier to get the final resolved types for type parameters
        let final_type = if !type_param_vars.is_empty() {
            // This is a generic type - resolve type parameters
            let resolved_args: List<Type> = type_param_vars
                .iter()
                .map(|tv| self.unifier.apply(tv))
                .collect();
            // Resolve Self to actual type path if needed
            let resolved_path = if path.segments.len() == 1
                && matches!(path.segments[0], verum_ast::ty::PathSegment::SelfValue)
            {
                // Get path from self type
                if let Maybe::Some(Type::Named {
                    path: self_path, ..
                }) = &self.current_self_type
                {
                    self_path.clone()
                } else {
                    path.clone()
                }
            } else {
                path.clone()
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
        let resolved_tup_ty = self.unifier.apply(&tup_result.ty);
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
                    if let Some((var_name, var_name_str)) = var_name_opt {
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
            Type::Named { path, .. } => {
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
                        let element_ty = types[idx].clone();

                        // Track affine element access for tuple structs
                        if let Some((var_name, var_name_str)) = var_name_opt {
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
                            Ok(InferResult::new(inner_ty))
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
            // Handle references to named tuple structs: &UserId where UserId is (Int)
            // Auto-dereference and index the underlying type
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                if let Type::Named { path, .. } = inner.as_ref() {
                    let type_name = self.path_to_string(path);
                    let simple_name = Self::path_type_name(path).unwrap_or(&type_name);

                    // Try tuple struct first (__tuple_fields_)
                    let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                    let tuple_fields_simple_key =
                        format!("__tuple_fields_{}", simple_name);

                    let found_tuple =
                        self.ctx.lookup_type(tuple_fields_key.as_str()).or_else(|| {
                            self.ctx.lookup_type(tuple_fields_simple_key.as_str())
                        });

                    if let Option::Some(Type::Tuple(types)) = found_tuple {
                        if idx < types.len() {
                            let element_ty = types[idx].clone();
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
                        let newtype_inner_key =
                            format!("__newtype_inner_{}", type_name);
                        let newtype_simple_key =
                            format!("__newtype_inner_{}", simple_name);

                        let found_inner =
                            self.ctx.lookup_type(newtype_inner_key.as_str()).or_else(
                                || self.ctx.lookup_type(newtype_simple_key.as_str()),
                            );

                        if let Option::Some(inner_ty) = found_inner {
                            if idx == 0 {
                                Ok(InferResult::new(inner_ty.clone()))
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(
                                    format!(
                                        "Newtype {} only has index 0, not {}",
                                        type_name, index
                                    ),
                                )))
                            }
                        } else {
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "cannot index type '{}' — only tuple types support .0, .1, etc.",
                                tup_result.ty
                            ))))
                        }
                    }
                } else if let Type::Tuple(types) = inner.as_ref() {
                    // Also handle reference to bare tuple
                    if idx < types.len() {
                        let element_ty = types[idx].clone();
                        Ok(InferResult::new(element_ty))
                    } else {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Tuple index {} out of bounds",
                            index
                        ))))
                    }
                } else if matches!(inner.as_ref(), Type::Var(_)) {
                    // Reference to unresolved type variable - create fresh var
                    let elem_var = TypeVar::fresh();
                    Ok(InferResult::new(Type::Var(elem_var)))
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "cannot index type '{}' — only tuple types support .0, .1, etc.",
                        resolved_tup_ty
                    ))))
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
        self.borrow_tracker.enter_await();

        // Check await safety for all active borrows
        self.borrow_tracker.check_await_safety()?;

        let inner_result = self.synth_expr(inner_expr)?;

        // Exit await context
        self.borrow_tracker.exit_await();

        // =========================================================================
        // Protocol-based Future resolution
        // Await desugaring: ".await" desugars to polling Future protocol until completion
        // =========================================================================
        // Also handle JoinHandle<T> as a special awaitable type
        if let Type::Generic { name, args } = &inner_result.ty {
            if name.as_str() == "JoinHandle" && args.len() == 1 {
                return Ok(InferResult::new(args[0].clone()));
            }
        }

        match self
            .protocol_checker
            .read()
            .resolve_future_protocol(&inner_result.ty)
        {
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
                // (the value is already "ready"). Also handles user types
                // that may implement Future but aren't registered yet.
                if self.in_async_context {
                    Ok(InferResult::new(inner_result.ty))
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Cannot await non-future type: {}. Type must implement Future.",
                        inner_result.ty
                    ))))
                }
            }
        }
    }

    fn infer_expr_spawn(&mut self, current_expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Spawn { expr, contexts } = &current_expr.kind else { unreachable!() };
        // Validate contexts exist in scope and expand groups
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
        let prev_async_context = std::mem::replace(&mut self.in_async_context, true);
        let inner_result = self.synth_expr(expr);
        self.in_async_context = prev_async_context;
        let inner_result = inner_result?;

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

        // JoinHandle represented as Generic type
        Ok(InferResult::new(Type::Generic {
            name: verum_common::Text::from("JoinHandle"),
            args: vec![output_ty].into(),
        }))
    }

    fn infer_expr_async_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Async(block) = &expr.kind else { unreachable!() };
        // Set async context for the block body
        let prev_async_context = std::mem::replace(&mut self.in_async_context, true);
        let block_result = self.infer_block(block)?;
        self.in_async_context = prev_async_context;

        Ok(InferResult::new(Type::Future {
            output: Box::new(block_result.ty),
        }))
    }

    fn infer_expr_unsafe_expr(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::Unsafe(block) = &expr.kind else { unreachable!() };
        // Enter a new scope for the unsafe block
        self.ctx.enter_scope();

        // Track unsafe context for Tier 2 reference creation
        // Spec: L0-critical/reference_system/reference_tiers/unsafe_without_block
        let prev_unsafe_context = self.in_unsafe_context;
        self.in_unsafe_context = true;

        // Infer the block's type from its body
        let block_result = self.infer_block(block)?;

        // Restore unsafe context and exit scope
        self.in_unsafe_context = prev_unsafe_context;
        self.ctx.exit_scope();

        // Return the block's type directly (unsafe doesn't wrap the type)
        Ok(block_result)
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

                    // Protocol-based IntoIterator resolution
                    let resolved_comprehension_iter =
                        self.unifier.apply(&iter_result.ty);
                    let elem_ty = match self
                        .protocol_checker
                        .read()
                        .resolve_into_iterator_protocol(&resolved_comprehension_iter)
                    {
                        Some(resolution) => resolution.item,
                        None => {
                            // Fallback: Try Iterator protocol's Item associated type
                            let iter_item =
                                self.protocol_checker.read().try_find_associated_type(
                                    &resolved_comprehension_iter,
                                    &verum_common::Text::from("Item"),
                                );
                            if let Some(item_ty) = iter_item {
                                self.protocol_checker
                                    .read()
                                    .normalize_projection_type(&item_ty)
                            } else {
                                return Err(TypeError::Other(
                                    verum_common::Text::from(format!(
                                        "Cannot iterate over type in comprehension: {}",
                                        resolved_comprehension_iter
                                    )),
                                ));
                            }
                        }
                    };

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
                        .protocol_checker
                        .read()
                        .resolve_into_iterator_protocol(&iter_result.ty)
                    {
                        Some(resolution) => resolution.item,
                        None => {
                            // Fallback 1: Try Iterator protocol's Item associated type
                            let resolved_iter_ty = self.unifier.apply(&iter_result.ty);
                            let iter_item =
                                self.protocol_checker.read().try_find_associated_type(
                                    &resolved_iter_ty,
                                    &verum_common::Text::from("Item"),
                                );
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

                    // Protocol-based IntoIterator resolution
                    let elem_ty = match self
                        .protocol_checker
                        .read()
                        .resolve_into_iterator_protocol(&iter_result.ty)
                    {
                        Some(resolution) => resolution.item,
                        None => {
                            // Fallback: Try Iterator protocol's Item associated type
                            let resolved_iter_ty = self.unifier.apply(&iter_result.ty);
                            let iter_item =
                                self.protocol_checker.read().try_find_associated_type(
                                    &resolved_iter_ty,
                                    &verum_common::Text::from("Item"),
                                );
                            if let Some(item_ty) = iter_item {
                                self.protocol_checker
                                    .read()
                                    .normalize_projection_type(&item_ty)
                            } else {
                                return Err(TypeError::Other(
                                    verum_common::Text::from(format!(
                                        "Cannot iterate over type in set comprehension: {}",
                                        resolved_iter_ty
                                    )),
                                ));
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

                    // Protocol-based IntoIterator resolution
                    let elem_ty = match self
                        .protocol_checker
                        .read()
                        .resolve_into_iterator_protocol(&iter_result.ty)
                    {
                        Some(resolution) => resolution.item,
                        None => {
                            // Fallback: Try Iterator protocol's Item associated type
                            let resolved_iter_ty = self.unifier.apply(&iter_result.ty);
                            let iter_item =
                                self.protocol_checker.read().try_find_associated_type(
                                    &resolved_iter_ty,
                                    &verum_common::Text::from("Item"),
                                );
                            if let Some(item_ty) = iter_item {
                                self.protocol_checker
                                    .read()
                                    .normalize_projection_type(&item_ty)
                            } else {
                                return Err(TypeError::Other(
                                    verum_common::Text::from(format!(
                                        "Cannot iterate over type in generator: {}",
                                        resolved_iter_ty
                                    )),
                                ));
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

        // Track that we're inside a try/recover block so throw is allowed
        self.try_recover_depth += 1;

        // Temporarily set function return type to Result<T, E> so that
        // the ? operator inside the try block passes type checking.
        // Without this, ? checks the enclosing function's return type
        // (which may be non-Result like Int) and rejects it.
        let saved_return_type = self.current_function_return_type.clone();
        let try_ok_var = Type::Var(crate::ty::TypeVar::fresh());
        self.current_function_return_type =
            Maybe::Some(Type::result(try_ok_var.clone(), error_type.clone()));

        // Infer type of try block
        let try_result = self.synth_expr(try_block)?;

        // Restore original function return type
        self.current_function_return_type = saved_return_type;
        self.try_recover_depth -= 1;

        // Infer type of recovery body with error type for pattern binding
        let recover_ty = self.infer_recover_body(recover, &error_type)?;

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
            if let Some(try_resolution) = self
                .protocol_checker
                .read()
                .resolve_try_protocol(&obj_result.ty)
            {
                // This is `result?.field` inside try-recover — unwrap to success type
                let success_ty = try_resolution.output;
                let dereferenced_ty = self.unwrap_reference_type(&success_ty);
                let field_ty = self
                    .lookup_field_type(dereferenced_ty, field.name.as_str())
                    .ok_or_else(|| {
                        TypeError::Other(verum_common::Text::from(format!(
                            "field '{}' not found in type '{}'",
                            field.name, dereferenced_ty
                        )))
                    })?;
                return Ok(InferResult::new(field_ty));
            }
        }

        // Use protocol-based Maybe resolution
        let inner_ty = match self
            .protocol_checker
            .read()
            .resolve_maybe_protocol(&obj_result.ty)
        {
            Some(resolution) => resolution.inner,
            None => obj_result.ty.clone(),
        };

        // Unwrap reference types before field access (Maybe<&Point>?.field)
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

    fn infer_expr_meta_function(&mut self, expr: &Expr) -> Result<InferResult> {
        use ExprKind::*;
        let ExprKind::MetaFunction { name, args } = &expr.kind else { unreachable!() };
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
                    return self.synth_expr(arg);
                }
                Type::unit()
            }

            // String operations return Text
            "concat" | "stringify" => Type::text(),

            // Diagnostics return Unit
            "warning" | "error" => Type::unit(),

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
                            match intrinsic_name {
                                // Memory intrinsics
                                "compare_exchange" | "compare_exchange_weak" => {
                                    Type::bool()
                                }
                                "atomic_load" | "atomic_store" | "atomic_swap" => {
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
                                // Pointer operations
                                "ptr_read" | "ptr_read_volatile" => {
                                    Type::Var(TypeVar::fresh())
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
                                "size_of" | "align_of" | "min_align_of" | "type_id" => {
                                    Type::int()
                                }
                                "type_name" => Type::text(),
                                // Control flow
                                "unreachable" | "panic" | "abort" => Type::never(),
                                // Catch unwind
                                "catch_unwind" => {
                                    // Returns Result<T, PanicInfo>
                                    Type::Named {
                                        path: verum_ast::ty::Path::single(
                                            verum_ast::Ident::new(
                                                WKT::Result.as_str(),
                                                expr.span,
                                            ),
                                        ),
                                        args: List::from_iter([
                                            Type::Var(TypeVar::fresh()),
                                            Type::Named {
                                                path: verum_ast::ty::Path::single(
                                                    verum_ast::Ident::new(
                                                        "PanicInfo",
                                                        expr.span,
                                                    ),
                                                ),
                                                args: List::new(),
                                            },
                                        ]),
                                    }
                                }
                                // Slice intrinsics - return reference to slice
                                "slice_from_raw_parts" => Type::Reference {
                                    inner: Box::new(Type::Slice {
                                        element: Box::new(Type::Var(TypeVar::fresh())),
                                    }),
                                    mutable: false,
                                },
                                "slice_from_raw_parts_mut" => Type::Reference {
                                    inner: Box::new(Type::Slice {
                                        element: Box::new(Type::Var(TypeVar::fresh())),
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
            // compiler-bound meta-symbols (mirrored on the codegen
            // side in verum_vbc/src/codegen/expressions.rs §4077+).
            // Argument expressions are still synthesised for their
            // side-effects on the checker state (diagnostic
            // accumulation, constraint generation).
            n if n.starts_with("builtin_") => {
                for arg in args.iter() {
                    let _ = self.synth_expr(arg);
                }
                Type::Var(TypeVar::fresh())
            }

            // Unknown meta-function - default to unit
            _ => Type::unit(),
        };

        Ok(InferResult::new(result_type))
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
        // Spec: docs/improvements.md Section 13.2 — reflection on
        // the runtime type yields an info record whose canonical
        // field is `name`. Other fields (`size`, `alignment`) can
        // be added later without breaking field access.
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

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deep recursion.
    pub(super) fn infer_block(&mut self, block: &Block) -> Result<InferResult> {
        let _depth_guard = self.inc_inference_depth("infer_block")?;
        self.infer_block_inner(block)
    }

    /// Inner implementation of infer_block.
    fn infer_block_inner(&mut self, block: &Block) -> Result<InferResult> {
        self.ctx.enter_scope();
        self.borrow_tracker.enter_scope();

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
        let mut has_diverging_stmt = false;
        for stmt in &block.stmts {
            let diverges = self.check_stmt(stmt)?;
            if diverges {
                has_diverging_stmt = true;
            }
        }

        // Type check final expression
        let result = if let Some(expr) = &block.expr {
            self.synth_expr(expr)?
        } else if has_diverging_stmt {
            // Block has a diverging statement, so it has type Never
            InferResult::new(Type::never())
        } else {
            InferResult::new(Type::unit())
        };

        self.borrow_tracker.exit_scope();
        self.ctx.exit_scope();
        Ok(result)
    }

    /// Check a block against expected type.
    pub(super) fn check_block(&mut self, block: &Block, expected: &Type) -> Result<InferResult> {
        let _depth_guard = self.inc_inference_depth("check_block")?;
        self.check_block_inner(block, expected)
    }

    fn check_block_inner(&mut self, block: &Block, expected: &Type) -> Result<InferResult> {
        self.ctx.enter_scope();
        self.borrow_tracker.enter_scope();

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
        let mut has_diverging_stmt = false;
        for stmt in block.stmts.iter() {
            let diverges = self.check_stmt(stmt)?;
            if diverges {
                has_diverging_stmt = true;
                // Once we hit a diverging statement, the rest is unreachable
                // but we still type-check them for error reporting
            }
        }

        let result = if let Some(expr) = &block.expr {
            self.check_expr(expr, expected)?
        } else {
            // If there's no trailing expression, check if we have a diverging statement
            if has_diverging_stmt {
                // A diverging statement (like return, break, continue) means the block
                // can have any type, so we return the expected type
                InferResult::new(expected.clone())
            } else if !matches!(expected, Type::Unit) {
                return Err(TypeError::Mismatch {
                    expected: expected.to_text(),
                    actual: "Unit".to_text(),
                    span: block.span,
                });
            } else {
                InferResult::new(Type::unit())
            }
        };

        self.borrow_tracker.exit_scope();
        self.ctx.exit_scope();
        Ok(result)
    }
}
