//! Type resolution and normalization methods for the type-checker.
//!
//! Contains ~71 `TypeChecker` methods covering:
//! - AST→Type conversion (`ast_to_type`, `ast_to_type_inner`, `ast_to_type_lenient`)
//! - Type normalization (`normalize_type`, `normalize_named_type`, `normalize_type_app`)
//! - Cast checking, universe resolution, and associated-type projection

#[allow(unused_imports)]
use crate::const_eval::ConstEvaluator;
#[allow(unused_imports)]
use crate::context::{TypeContext, TypeScheme};
#[allow(unused_imports)]
use crate::context_check::{ContextChecker, ContextRequirement, ContextSet};
#[allow(unused_imports)]
use crate::integer_hierarchy::IntegerHierarchy;
#[allow(unused_imports)]
use crate::operator_protocols::{OperatorProtocols, OutputStrategy};
#[allow(unused_imports)]
use crate::protocol::ProtocolChecker;
#[allow(unused_imports)]
use crate::refinement::RefinementChecker;
#[allow(unused_imports)]
use crate::subtype::Subtyping;
#[allow(unused_imports)]
use crate::ty::{Type, TypeVar};
#[allow(unused_imports)]
use crate::unify::Unifier;
#[allow(unused_imports)]
use crate::{Result, TypeCheckMetrics, TypeError};
#[allow(unused_imports)]
use super::{
    DeferredConstraint, FunctionContract, GeneratorContext,
    GlobalDepthGuard, InferMode, InferResult, TypeChecker,
    WKT_HEAP, WKT_RESULT, WKT_SHARED,
    DEREF_COERCION_DEPTH, GLOBAL_CALL_DEPTH, NORMALIZE_DEPTH,
    AST_TO_TYPE_DEPTH, TYPE_RESOLUTION_STACK, NORMALIZE_TYPE_STACK,
    is_stdlib_toplevel_path, span_to_line_col,
    ConversionPath, ConversionStep, NormalizeTypeCycleGuard, ThreadLocalDepthGuard,
    resolve_builtin_meta_type, type_kind_description,
};
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use verum_ast::{BinOp, Block, Expr, ExprKind, LiteralKind, Stmt, StmtKind, TokenTree, UnOp, Item};
#[allow(unused_imports)]
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParamKind, ImplDecl, RecordField, TypeDecl, TypeDeclBody,
    ContextDecl, ProtocolDecl, Visibility, MountDecl, MountTree, MountTreeKind, ResourceModifier,
};
#[allow(unused_imports)]
use verum_ast::pattern::Pattern;
#[allow(unused_imports)]
use verum_ast::span::{Span, Spanned};
#[allow(unused_imports)]
use verum_ast::ty::{Ident, Path};
#[allow(unused_imports)]
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
#[allow(unused_imports)]
use verum_common::well_known_types::WellKnownType as WKT;
#[allow(unused_imports)]
use verum_common::well_known_types::type_names as wkt_names;
#[allow(unused_imports)]
use verum_common::{Heap, List, Map, Maybe, Set, Shared, Text, ToText};
#[allow(unused_imports)]
use verum_modules::{ModulePath, ModuleRegistry, NameResolver, resolver::NameKind};

impl TypeChecker {
    pub(crate) fn ast_to_type_lenient(&mut self, ast_ty: &verum_ast::ty::Type) -> Type {
        use verum_ast::ty::TypeKind;

        match &ast_ty.kind {
            // Handle Generic types specially to preserve structure
            TypeKind::Generic { base, args } => {
                // Try to resolve the base type leniently
                let base_ty = self.ast_to_type_lenient(base);

                // Resolve type arguments leniently
                let mut type_args: List<Type> = List::new();

                for arg in args {
                    use verum_ast::ty::GenericArg;
                    match arg {
                        GenericArg::Type(ty) => {
                            let arg_ty = self.ast_to_type_lenient(ty);
                            type_args.push(arg_ty);
                        }
                        GenericArg::Const(expr) => {
                            // Preserve the compile-time value of a const-generic argument
                            // so downstream unification can detect dimension mismatches.
                            // Falling back to only the expression type (Int, USize, ...)
                            // would collapse e.g. `Matrix<Float, 7, 7>` and `Matrix<Float, 5, 5>`
                            // to the same representation and mask the error.
                            match self.eval_const_arg(expr) {
                                Some(ty) => type_args.push(ty),
                                None => match self.synth_expr(expr) {
                                    Ok(result) => type_args.push(result.ty),
                                    Err(_) => type_args.push(Type::Var(TypeVar::fresh())),
                                },
                            }
                        }
                        GenericArg::Lifetime(lifetime) => {
                            let lifetime_ty = Type::lifetime(lifetime.name.clone());
                            type_args.push(lifetime_ty);
                        }
                        GenericArg::Binding(binding) => {
                            let arg_ty = self.ast_to_type_lenient(&binding.ty);
                            type_args.push(arg_ty);
                        }
                    }
                }

                // Reconstruct the generic type with resolved arguments

                match base_ty {
                    Type::Named { path, .. } => Type::Named {
                        path,
                        args: type_args.clone(),
                    },
                    Type::Generic { name, .. } => Type::Generic {
                        name,
                        args: type_args.clone(),
                    },
                    // For other base types, try to use Generic if we have a name
                    _ => {
                        // Extract name from the base AST if possible
                        if let TypeKind::Path(path) = &base.kind {
                            if let Some(verum_ast::ty::PathSegment::Name(ident)) =
                                path.segments.last()
                            {
                                Type::Generic {
                                    name: ident.name.clone(),
                                    args: type_args.clone(),
                                }
                            } else {
                                base_ty
                            }
                        } else {
                            base_ty
                        }
                    }
                }
            }

            // Handle Path types - try to resolve, but preserve the name if resolution fails
            TypeKind::Path(path) => {
                match self.ast_to_type(ast_ty) {
                    Ok(ty) => ty,
                    Err(_) => {
                        // Create a Named type that preserves the path structure
                        // This allows types like SearchResponse to be resolved later
                        // when the actual method is called
                        Type::Named {
                            path: path.clone(),
                            args: List::new(),
                        }
                    }
                }
            }

            // CRITICAL FIX: Handle Reference types by recursively resolving the inner type leniently.
            // This preserves the reference structure even when the inner type (like Hasher)
            // cannot be resolved in the current scope.
            // Without this, `&mut Hasher` would become a fresh type variable, causing
            // generalize() to capture it and break method signature matching.
            TypeKind::Reference { mutable, inner } => {
                let inner_ty = self.ast_to_type_lenient(inner);
                Type::reference(*mutable, inner_ty)
            }

            TypeKind::CheckedReference { mutable, inner } => {
                let inner_ty = self.ast_to_type_lenient(inner);
                Type::checked_reference(*mutable, inner_ty)
            }

            TypeKind::UnsafeReference { mutable, inner } => {
                let inner_ty = self.ast_to_type_lenient(inner);
                Type::unsafe_reference(*mutable, inner_ty)
            }

            // CRITICAL FIX: Handle Slice types by recursively resolving the element type leniently.
            // This preserves the slice structure even when the element type cannot be resolved.
            TypeKind::Slice(elem) => {
                let elem_ty = self.ast_to_type_lenient(elem);
                Type::slice(elem_ty)
            }

            // CRITICAL FIX: Handle Array types by recursively resolving the element type leniently.
            // For lenient conversion, we don't try to evaluate the size expression - leave it as None.
            // This preserves the array structure without requiring const evaluation in scope.
            TypeKind::Array { element, size: _ } => {
                let elem_ty = self.ast_to_type_lenient(element);
                Type::array(elem_ty, None)
            }

            // CRITICAL FIX: Handle Pointer types by recursively resolving the inner type leniently.
            TypeKind::Pointer { mutable, inner } => {
                let inner_ty = self.ast_to_type_lenient(inner);
                Type::Pointer {
                    mutable: *mutable,
                    inner: Box::new(inner_ty),
                }
            }

            // Handle VolatilePointer types (for MMIO) by recursively resolving the inner type leniently.
            TypeKind::VolatilePointer { mutable, inner } => {
                let inner_ty = self.ast_to_type_lenient(inner);
                Type::VolatilePointer {
                    mutable: *mutable,
                    inner: Box::new(inner_ty),
                }
            }

            // CRITICAL FIX: Handle Function types by recursively resolving parameter and return types.
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                let param_types: List<Type> =
                    params.iter().map(|p| self.ast_to_type_lenient(p)).collect();
                let ret_type = self.ast_to_type_lenient(return_type);
                Type::function(param_types, ret_type)
            }

            // For other type kinds, use the standard resolution with fallback
            _ => {
                match self.ast_to_type(ast_ty) {
                    Ok(ty) => ty,
                    Err(_) => {
                        // Fall back to a fresh type variable for truly unresolvable types
                        Type::Var(TypeVar::fresh())
                    }
                }
            }
        }
    }

    /// Convert AST type to Type for protocol implementation registration.
    ///

    /// This variant creates Named types WITHOUT expanding type aliases.
    /// This is critical for protocol implementation lookup because `get_implementations`
    /// matches by type key. If `Result<T, E>` is expanded to `Ok(T) | Err(E)`,
    /// the lookup for "Result" won't find the implementation.
    ///

    /// Used specifically for the `for_type` field of ProtocolImpl.
    pub(super) fn ast_to_type_for_protocol_impl(&mut self, ast_ty: &verum_ast::ty::Type) -> Result<Type> {
        use verum_ast::ty::TypeKind;

        match &ast_ty.kind {
            // For Path types, check if it's a registered type parameter first
            // This ensures that blanket impls like `implement<I: Iterator> IntoIterator for I {}`
            // have for_type as Type::Var (matching the where clause), not Type::Named.
            // This is necessary for substitution key consistency in unify_types/apply_substitution.
            TypeKind::Path(path) => {
                // Check if this is a simple identifier that's a registered type parameter
                if let Some(ident) = path.as_ident() {
                    if let Some(registered_ty) = self.ctx.lookup_type(ident.as_str()).cloned() {
                        // Return the registered Type::Var for type parameters
                        if matches!(registered_ty, Type::Var(_)) {
                            return Ok(registered_ty);
                        }
                    }
                }
                // Not a type parameter - create Named type
                Ok(Type::Named {
                    path: path.clone(),
                    args: List::new(),
                })
            }

            // For Generic types, create a Named/Generic type without expansion
            TypeKind::Generic { base, args } => {
                // Recursively resolve the base type
                let base_ty = self.ast_to_type_for_protocol_impl(base)?;

                // Resolve type arguments (these can use regular ast_to_type since they may need expansion)
                let mut type_args: List<Type> = List::new();
                for arg in args {
                    use verum_ast::ty::GenericArg;
                    match arg {
                        GenericArg::Type(ty) => {
                            // Use regular ast_to_type for arguments since they may be type variables
                            let arg_ty = self
                                .ast_to_type(ty)
                                .unwrap_or_else(|_| self.ast_to_type_lenient(ty));
                            type_args.push(arg_ty);
                        }
                        GenericArg::Const(expr) => {
                            // See eval_const_arg: we preserve the compile-time value so
                            // dimension mismatches are caught during unification.
                            match self.eval_const_arg(expr) {
                                Some(ty) => type_args.push(ty),
                                None => match self.synth_expr(expr) {
                                    Ok(result) => type_args.push(result.ty),
                                    Err(_) => type_args.push(Type::Var(TypeVar::fresh())),
                                },
                            }
                        }
                        GenericArg::Lifetime(lifetime) => {
                            type_args.push(Type::lifetime(lifetime.name.clone()));
                        }
                        GenericArg::Binding(binding) => {
                            let arg_ty = self
                                .ast_to_type(&binding.ty)
                                .unwrap_or_else(|_| self.ast_to_type_lenient(&binding.ty));
                            type_args.push(arg_ty);
                        }
                    }
                }

                // Construct the result type from base
                match base_ty {
                    Type::Named { path, .. } => Ok(Type::Named {
                        path,
                        args: type_args,
                    }),
                    Type::Generic { name, .. } => Ok(Type::Generic {
                        name,
                        args: type_args,
                    }),
                    _ => {
                        // Extract name from the base AST if possible
                        if let TypeKind::Path(path) = &base.kind {
                            if let Some(verum_ast::ty::PathSegment::Name(ident)) =
                                path.segments.last()
                            {
                                Ok(Type::Generic {
                                    name: ident.name.clone(),
                                    args: type_args,
                                })
                            } else {
                                // Fall back to standard resolution
                                self.ast_to_type(ast_ty)
                            }
                        } else {
                            self.ast_to_type(ast_ty)
                        }
                    }
                }
            }

            // For primitive types and others, use standard resolution
            _ => self.ast_to_type(ast_ty),
        }
    }

    /// Wrap a function's body-level return type with the externally-visible
    /// signature transformations that callers need to see:
    ///

    ///  1. `throws(E) -> T` → `Result<T, E>` (or the first error type
    ///  in a multi-throws union; multi-type unions are simplified per
    ///  the existing semantics at `register_function_signature`).
    ///  2. `async` → `Future<...>` wraps the throws-wrapped
    ///  result.
    ///

    /// Every site that builds a `Type::Function` from a `FunctionDecl`
    /// must apply both transformations in this order. Before this helper
    /// existed, the throws wrap was applied at some sites and omitted at
    /// others, producing call-site type mismatches (see 70be7846,
    /// 05bba3d3). Callers that already have an explicit `Result<T, E>`
    /// body are guarded by a `resolve_try_protocol` probe so we don't
    /// produce `Result<Result<T, E>, E>`.
    pub(crate) fn wrap_return_type_for_sig(
        &mut self,
        return_type: Type,
        throws_clause: &verum_common::Maybe<verum_ast::decl::ThrowsClause>,
        is_async: bool,
    ) -> Type {
        // Backward-compat shim — forwards to the full impl with
        // `is_generator = false`. Existing callers that don't yet
        // distinguish generator-vs-regular functions stay correct
        // for non-generator decls. The new
        // `wrap_return_type_for_sig_full` carries the generator flag
        // so async generators (`async fn*`) wrap as
        // `Future<Generator<Yield, Unit>>` instead of being
        // silently demoted to `Future<Yield>` (commit-level note
        // for SHELL-5a: this was the load-bearing bug that broke
        // `for await line in mounted_stream_lines(&t)` everywhere).
        self.wrap_return_type_for_sig_full(return_type, throws_clause, is_async, false)
    }

    /// Full version of `wrap_return_type_for_sig` that ALSO honours
    /// the generator flag. Order of wrapping (outermost last):
    ///

    ///  1. throws-clause → `Result<T, E>` (or pass-through if T
    ///  already implements Try)
    ///  2. generator → `Generator<T, Unit>`
    ///  3. async → `Future<T>`
    ///

    /// So an `async fn* foo() -> Y throws E` decl yields
    /// `Future<Generator<Result<Y, E>, Unit>>` — matches the
    /// declaration-time wrap at infer.rs:35117 (the function-decl
    /// path) so cross-module mounts get the same shape.
    pub(crate) fn wrap_return_type_for_sig_full(
        &mut self,
        return_type: Type,
        throws_clause: &verum_common::Maybe<verum_ast::decl::ThrowsClause>,
        is_async: bool,
        is_generator: bool,
    ) -> Type {
        let with_throws = if let verum_common::Maybe::Some(tc) = throws_clause {
            if !tc.error_types.is_empty() {
                // Double-wrap guard: if the body already returns a
                // Try-implementing type (Result<_, _> / Maybe<_> etc.),
                // leave it alone.
                if self
                    .protocol_checker
                    .read()
                    .resolve_try_protocol(&return_type)
                    .is_some()
                {
                    return_type
                } else {
                    // Build the error type. For `throws(E)` single error,
                    // use E directly. For `throws(A | B | …)` multi-error
                    // union, build a Variant whose entries are the error
                    // types themselves (flattening if an entry is already
                    // a Variant — matches the `infer_try_operator`
                    // semantics at line ~22710 so the closure inside a
                    // `.map_err(|e| …)` sees the union rather than just
                    // the first error type).
                    let resolved: Vec<Type> = tc
                        .error_types
                        .iter()
                        .filter_map(|t| self.ast_to_type(t).ok())
                        .collect();
                    let error_ty = if resolved.is_empty() {
                        Type::Var(TypeVar::fresh())
                    } else if resolved.len() == 1 {
                        resolved.into_iter().next().unwrap()
                    } else {
                        let mut variants = indexmap::IndexMap::new();
                        for t in resolved.into_iter() {
                            match t {
                                Type::Variant(inner) => {
                                    for (name, ty) in inner.iter() {
                                        variants.insert(name.clone(), ty.clone());
                                    }
                                }
                                other => {
                                    variants.insert(other.to_text(), other);
                                }
                            }
                        }
                        Type::Variant(variants)
                    };
                    Type::result(return_type, error_ty)
                }
            } else {
                return_type
            }
        } else {
            return_type
        };
        // Generator wrap: `fn*` returning T is `Generator<T, Unit>`.
        // Applied BEFORE the async wrap so `async fn*` becomes
        // `Future<Generator<T, Unit>>` (matches the function-decl
        // path's wrap order at infer.rs:35117 — see SHELL-5a
        // commit-level note for context).
        let with_generator = if is_generator {
            Type::generator(with_throws, Type::unit())
        } else {
            with_throws
        };
        if is_async {
            Type::Future {
                output: Box::new(with_generator),
            }
        } else {
            with_generator
        }
    }

    /// Convert AST type to internal type representation.
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deep recursion.
    /// Tracks recursion depth to detect infinite recursion early.
    pub fn ast_to_type(&mut self, ast_ty: &verum_ast::ty::Type) -> Result<Type> {
        // SPECIAL CASE: stdlib core/math/hott.vr uses `type I is @builtin_interval;`
        // — a top-level alias whose *body* is the meta-type marker. Intercept it
        // before the depth guard so recursive ast_to_type on the outer alias
        // doesn't try to resolve `@builtin_*` as a user-defined type via the
        // generic path below.
        if let verum_ast::ty::TypeKind::Path(path) = &ast_ty.kind {
            if path.segments.len() == 1 {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    if let Some(builtin) = resolve_builtin_meta_type(ident.name.as_str()) {
                        return Ok(builtin);
                    }
                }
            }
        }
        // RAII depth guard — decrements on drop even if we panic or return early.
        //

        // Sized for rayon worker threads (default 512 KiB stack on macOS).
        // Each `ast_to_type` / `ast_to_type_inner` pair recurses through
        // ~2 stack frames per logical depth; debug builds inflate the
        // frame size to ~6-8 KiB. Setting the limit at 64 caps the worst
        // case at ~64 × 2 × 8 KiB ≈ 1 MiB (debug) — still over the
        // 512 KiB worker budget, but well under the 8 MiB main-thread
        // budget. Release builds halve frame size, so 64 is safely
        // within the worker bound. Real-world AST nesting rarely
        // exceeds depth 30; the bound is a defence, not a working budget.
        //

        // Tuning history: was 128 before T0.5; that hit SIGBUS on
        // rayon workers during `verum test`'s type-check phase
        // because debug frames overflowed 512 KiB at depth ~50.
        const MAX_AST_TO_TYPE_DEPTH: usize = 64;
        let _guard = match ThreadLocalDepthGuard::new(&AST_TO_TYPE_DEPTH, MAX_AST_TO_TYPE_DEPTH) {
            Some(g) => g,
            None => {
                return Err(TypeError::RecursionLimit(
                    format!(
                        "ast_to_type recursion depth exceeded (max {})",
                        MAX_AST_TO_TYPE_DEPTH
                    )
                    .into(),
                ));
            }
        };

        self.ast_to_type_inner(ast_ty)
    }

    /// Allocate a fresh universe variable ID for a named level parameter.
    /// Uses a deterministic mapping from name to ID via hashing.
    pub(super) fn fresh_universe_var_id(&self, name: &verum_common::Text) -> u32 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        (hasher.finish() & 0x7FFF_FFFF) as u32
    }

    /// Convert an AST universe level expression to the internal UniverseLevel.
    fn ast_universe_level_to_internal(
        &self,
        expr: &verum_ast::UniverseLevelExpr,
    ) -> crate::ty::UniverseLevel {
        use crate::ty::UniverseLevel;
        match expr {
            verum_ast::UniverseLevelExpr::Concrete(n) => UniverseLevel::Concrete(*n),
            verum_ast::UniverseLevelExpr::Variable(ident) => {
                let var_id = self.fresh_universe_var_id(&ident.name);
                UniverseLevel::Variable(var_id)
            }
            verum_ast::UniverseLevelExpr::Max(a, b) => {
                let la = self.ast_universe_level_to_internal(a);
                let lb = self.ast_universe_level_to_internal(b);
                la.max(lb)
            }
            verum_ast::UniverseLevelExpr::Succ(inner) => {
                let l = self.ast_universe_level_to_internal(inner);
                l.succ()
            }
        }
    }

    fn ast_to_type_inner(&mut self, ast_ty: &verum_ast::ty::Type) -> Result<Type> {
        use verum_ast::ty::TypeKind;

        match &ast_ty.kind {
            TypeKind::Unit => Ok(Type::unit()),
            TypeKind::Bool => Ok(Type::bool()),
            TypeKind::Int => Ok(Type::int()),
            TypeKind::Float => Ok(Type::float()),
            TypeKind::Char => Ok(Type::Char),
            TypeKind::Text => Ok(Type::text()),
            TypeKind::Never => Ok(Type::never()),

            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                let param_types: Result<List<_>> =
                    params.iter().map(|p| self.ast_to_type(p)).collect();
                let param_types = param_types?;
                let ret_type = self.ast_to_type(return_type)?;

                // Check if this is a dependent function (Pi type)
                // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi Types (Dependent Functions)
                // A function type is dependent if its return type references parameter types
                if self.has_dependent_types() && !param_types.is_empty() {
                    use crate::dependent_helpers::{collect_type_vars, convert_internal_to_ast};
                    use crate::dependent_integration::DependentTypeConstraint;

                    // Collect type variables from parameters
                    let param_vars: std::collections::HashSet<_> = param_types
                        .iter()
                        .flat_map(|p| collect_type_vars(p).into_iter())
                        .collect();

                    // Check if return type references any parameter variables
                    let ret_vars = collect_type_vars(&ret_type);
                    let is_dependent = ret_vars.iter().any(|v| param_vars.contains(v));

                    if is_dependent {
                        // This is a Pi type - verify the dependent function is well-formed
                        // The return type must be valid for all possible parameter values
                        let constraint = DependentTypeConstraint::PiType {
                            param_name: verum_common::Text::from("_param"), // Use placeholder name for combined params
                            param_type: if param_types.len() == 1 {
                                convert_internal_to_ast(&param_types[0])
                            } else {
                                // Multiple params - create tuple type
                                let tuple_ty = Type::tuple(param_types.clone());
                                convert_internal_to_ast(&tuple_ty)
                            },
                            return_type: convert_internal_to_ast(&ret_type),
                            span: ast_ty.span,
                        };

                        self.verify_dependent_type_constraint(&constraint, ast_ty.span)?;
                    }
                }

                Ok(Type::function(param_types, ret_type))
            }

            TypeKind::Rank2Function { .. } => self.ast_to_rank2_function_type(ast_ty),

            TypeKind::Tuple(types) => {
                let types: Result<List<_>> = types.iter().map(|t| self.ast_to_type(t)).collect();
                Ok(Type::tuple(types?))
            }

            TypeKind::Array { element, size } => {
                let elem_ty = self.ast_to_type(element)?;
                // Extract array size via const evaluation if size expression is present
                let array_size = if let verum_common::Maybe::Some(size_expr) = size {
                    match self.const_eval.eval(size_expr) {
                        Ok(const_val) => const_val.as_u128().map(|n| n as usize),
                        Err(_) => None, // Not a compile-time constant
                    }
                } else {
                    None
                };
                Ok(Type::array(elem_ty, array_size))
            }

            TypeKind::Slice(element) => {
                let elem_ty = self.ast_to_type(element)?;
                Ok(Type::slice(elem_ty))
            }

            TypeKind::Reference { mutable, inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::reference(*mutable, inner_ty))
            }

            TypeKind::CheckedReference { mutable, inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::checked_reference(*mutable, inner_ty))
            }

            TypeKind::UnsafeReference { mutable, inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::unsafe_reference(*mutable, inner_ty))
            }

            // Path type: Path<A>(a, b) → Type::Eq { ty: A, lhs, rhs }
            TypeKind::PathType { carrier, lhs, rhs } => {
                let carrier_ty = self.ast_to_type(carrier)?;
                let lhs_eq = crate::expr_to_eqterm::expr_to_eq_term(lhs);
                let rhs_eq = crate::expr_to_eqterm::expr_to_eq_term(rhs);
                Ok(Type::Eq {
                    ty: Box::new(carrier_ty),
                    lhs: Box::new(lhs_eq),
                    rhs: Box::new(rhs_eq),
                })
            }

            // General dependent type application `T<A>(v..)`. We do not
            // yet check index expressions against a dependent type's
            // signature here — the carrier carries full generic info,
            // and the value indices are retained for downstream
            // verification passes (see refinement / dependent solver).
            TypeKind::DependentApp { carrier, .. } => {
                // For now, ignore the value indices and resolve as the
                // carrier type. A follow-up lands an index-checking
                // pass that re-unifies these against the type
                // constructor declaration. This matches how `Path<A>(a,
                // b)` worked before the sugared `PathType` split, so it
                // is the smallest change that keeps the stdlib parsing
                // without silently dropping index info (we still retain
                // the AST node for later passes).
                self.ast_to_type(carrier)
            }

            TypeKind::Path(path) => {
                // `@builtin_*` meta-type markers are language-level primitives
                // referenced from stdlib (e.g. `type I is @builtin_interval;`,
                // `type Path<A>(a, b) is @builtin_path;` in core/math/hott.vr).
                // They are not looked up in the user type environment — they
                // name compiler intrinsics directly and resolve here.
                if path.segments.len() == 1 {
                    if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                        let name = ident.name.as_str();
                        if let Some(builtin) = resolve_builtin_meta_type(name) {
                            return Ok(builtin);
                        }
                    }
                }
                // Named type (user-defined type or type alias)
                // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Cross-module type resolution
                // Use the new resolver for qualified paths
                self.resolve_qualified_type(path, ast_ty.span)
            }

            TypeKind::Generic { .. } => self.ast_to_generic_type(ast_ty),

            TypeKind::Refined { base, predicate } => {
                use crate::refinement::{
                    RefinementBinding, RefinementPredicate as TyRefinementPredicate,
                };
                use Option;

                let base_ty = self.ast_to_type(base)?;
                // Post collapse, `TypeKind::Refined` carries all three
                // surface forms. The sigma form reaches us with a
                // `predicate.binding = Some(name)`; preserve that distinction
                // by emitting a `RefinementBinding::Sigma` when the binding
                // came from the sigma surface syntax. We can no longer tell
                // sigma apart from lambda `T where |x| expr` at this layer
                // (both share the same AST shape), so both are treated as
                // Sigma — semantically equivalent in the type system.
                let binding = match &predicate.binding {
                    Some(ident) => RefinementBinding::Sigma(ident.name.clone()),
                    None => RefinementBinding::Inline,
                };
                let pred = TyRefinementPredicate {
                    predicate: predicate.expr.clone(),
                    binding: binding.clone(),
                    span: predicate.span,
                };

                // Verify dependent type constraint using SMT if enabled for
                // sigma-form refinements (previously the `TypeKind::Sigma`
                // arm's behaviour). Sigma types (dependent pairs):
                // `(x: A, B(x))` where second component type depends on first
                // value — refinement types desugar to Sigma.
                if let Some(name) = &predicate.binding {
                    if self.has_dependent_types() {
                        use crate::dependent_helpers::convert_internal_to_ast;
                        use crate::dependent_integration::DependentTypeConstraint;

                        let constraint = DependentTypeConstraint::SigmaType {
                            fst_name: name.name.clone(),
                            fst_type: convert_internal_to_ast(&base_ty),
                            snd_type: convert_internal_to_ast(&Type::Bool),
                            span: ast_ty.span,
                        };

                        match self.verify_dependent_type(&constraint) {
                            Ok(result) => {
                                if let crate::refinement::VerificationResult::Invalid {
                                    counterexample,
                                } = result
                                {
                                    // Log verification failure but don't fail type checking
                                    // This allows gradual adoption of dependent types
                                    // The constraint will be checked at runtime if not proven
                                    if let Maybe::Some(ref ce) = counterexample {
                                        // Record that this constraint needs runtime checking
                                        // (could be tracked in a separate data structure for diagnostics)
                                        let _ = ce; // Suppress unused warning for now
                                    }
                                }
                            }
                            Err(_) => {
                                // SMT verification failed - continue with type checking
                                // The constraint will be checked at runtime
                            }
                        }
                    }
                }

                Ok(Type::refined(base_ty, pred))
            }

            TypeKind::Inferred => {
                // Type inference placeholder
                Ok(Type::Var(TypeVar::fresh()))
            }

            TypeKind::Pointer { mutable, inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::Pointer {
                    mutable: *mutable,
                    inner: Box::new(inner_ty),
                })
            }

            TypeKind::VolatilePointer { mutable, inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::VolatilePointer {
                    mutable: *mutable,
                    inner: Box::new(inner_ty),
                })
            }

            TypeKind::Ownership { mutable, inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::Ownership {
                    mutable: *mutable,
                    inner: Box::new(inner_ty),
                })
            }

            TypeKind::GenRef { inner } => {
                let inner_ty = self.ast_to_type(inner)?;
                Ok(Type::genref(inner_ty))
            }

            TypeKind::Tensor {
                element,
                shape,
                layout: _,
            } => {
                use crate::const_eval::ConstEvaluator;
                use verum_common::ConstValue;

                let element_ty = self.ast_to_type(element)?;

                // Evaluate shape expressions to const values.
                // SIMD and tensor system: unified Tensor<T, Shape> type with compile-time shape validation, SIMD acceleration (SSE/AVX/NEON), auto-differentiation — Tensor types with compile-time shape
                //

                // Tensor dimensions can be:
                // 1. Literal integers (e.g., 3, 4, 5) - evaluated directly
                // 2. Meta parameters (e.g., N, M) - represented symbolically
                // 3. Expressions (e.g., N * 2) - evaluated if possible
                let mut shape_values = List::new();
                let mut const_eval = ConstEvaluator::new();

                for dim_expr in shape {
                    match const_eval.eval(dim_expr) {
                        Ok(val) => shape_values.push(val),
                        Err(_) => {
                            // Cannot evaluate at compile time.
                            // Check if this is a meta parameter reference that will be
                            // resolved during monomorphization.
                            if let verum_ast::expr::ExprKind::Path(path) = &dim_expr.kind {
                                if let Some(ident) = path.as_ident() {
                                    // Simple identifier - likely a meta parameter.
                                    // Use u128::MAX as sentinel for symbolic dimension.
                                    tracing::debug!(
                                        "Tensor dimension '{}' deferred to monomorphization",
                                        ident.name
                                    );
                                    shape_values.push(ConstValue::UInt(u128::MAX));
                                    continue;
                                }
                            }
                            // Complex expression - also defer to monomorphization.
                            // This handles cases like N * 2 where N is a meta parameter.
                            shape_values.push(ConstValue::UInt(u128::MAX));
                        }
                    }
                }

                Ok(Type::tensor(element_ty, shape_values, ast_ty.span))
            }

            TypeKind::TypeConstructor { base, arity } => {
                use crate::advanced_protocols::Kind;

                // Extract the name from the base type
                let name = if let TypeKind::Path(path) = &base.kind {
                    if let Some(ident) = path.as_ident() {
                        ident.name.clone()
                    } else {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Type constructor must have simple name, got: {}",
                            path
                        ))));
                    }
                } else {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Type constructor base must be a path, got: {}",
                        type_kind_description(&base.kind)
                    ))));
                };

                // Create appropriate kind based on arity
                let kind = match *arity {
                    0 => Kind::type_kind(),
                    1 => Kind::unary_constructor(),
                    2 => Kind::binary_constructor(),
                    _ => {
                        // For higher arities, build the kind recursively
                        let mut k = Kind::Type;
                        for _ in 0..*arity {
                            k = Kind::Arrow(Box::new(Kind::Type), Box::new(k));
                        }
                        k
                    }
                };

                Ok(Type::type_constructor(name, *arity, kind))
            }

            TypeKind::Qualified { .. } => self.ast_to_qualified_type(ast_ty),

            TypeKind::Bounded { base, bounds: _ } => {
                // Bounded type: T where T: Protocol
                // For type conversion purposes, we just use the base type
                // The bounds are checked separately during constraint solving
                self.ast_to_type(base)
            }

            TypeKind::DynProtocol { bounds, bindings } => {
                // Dynamic protocol object: dyn Display + Debug
                // Use the proper Type::DynProtocol representation
                let mut bound_names = List::new();
                for bound in bounds {
                    if let verum_ast::ty::TypeBoundKind::Protocol(path) = &bound.kind
                        && let Some(ident) = path.as_ident()
                    {
                        bound_names.push(ident.name.clone());
                    }
                }

                // Convert associated type bindings to a map
                let mut bindings_map = Map::new();
                if let Some(bindings_vec) = bindings {
                    for binding in bindings_vec {
                        let binding_ty = self.ast_to_type(&binding.ty)?;
                        bindings_map.insert(binding.name.name.clone(), binding_ty);
                    }
                }

                // Object safety check: verify each protocol bound is object-safe.
                // Object-unsafe protocols (generic methods, Self-returning, etc.)
                // cannot be used with `dyn`.
                for bound_name in &bound_names {
                    let pc = self.protocol_checker.read();
                    if let Err(errors) = pc.check_object_safety(bound_name) {
                        let error_msgs: Vec<String> =
                            errors.iter().map(|e| format!("{}", e)).collect();
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Protocol '{}' is not object-safe and cannot be used with `dyn`: {}",
                            bound_name,
                            error_msgs.join(", ")
                        ))));
                    }
                }

                Ok(Type::DynProtocol {
                    bounds: bound_names,
                    bindings: bindings_map,
                })
            }

            TypeKind::Existential { name, bounds } => {
                // Existential type: some T: Bound
                // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — Existential Types
                //

                // Creates an existentially quantified type: ∃T. T where T: Bound
                // Used for opaque return types and type erasure.
                //

                // The existential hides the concrete implementation type while
                // exposing only the protocol interface.
                let type_var = TypeVar::fresh();

                // Convert AST bounds to protocol bounds and register them
                let protocol_bounds: List<crate::protocol::ProtocolBound> = bounds
                    .iter()
                    .filter_map(|bound| {
                        match &bound.kind {
                            verum_ast::ty::TypeBoundKind::Protocol(path) => {
                                Some(crate::protocol::ProtocolBound {
                                    protocol: path.clone(),
                                    args: List::new(),
                                    is_negative: false,
                                })
                            }
                            verum_ast::ty::TypeBoundKind::NegativeProtocol(path) => {
                                Some(crate::protocol::ProtocolBound {
                                    protocol: path.clone(),
                                    args: List::new(),
                                    is_negative: true,
                                })
                            }
                            _ => None, // Handle other bound kinds as needed
                        }
                    })
                    .collect();

                // Register bounds on the type variable for method resolution
                self.register_type_var_bounds(type_var, protocol_bounds);

                // Create the existential type: ∃T. T
                // The body is just the type variable, bounds are tracked separately
                Ok(Type::Exists {
                    var: type_var,
                    body: Box::new(Type::Var(type_var)),
                })
            }

            TypeKind::AssociatedType { base, assoc } => {
                // Associated type path: T.Item or Self.Item
                // Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
                //

                // Represents a projection from a type to its associated type.
                // E.g., Iterator.Item, Container.Element, W.Inner.Item (chained)
                //

                // Resolution happens during protocol checking when the concrete
                // implementing type is known and we can look up the associated type
                // in the implementation.
                let base_ty = self.ast_to_type(base)?;

                // Represent associated types as Generic with projection naming convention.
                // Format: "::AssocName" with base_ty stored in args[0] for later resolution.
                // This allows chained projections like W.Inner.Item to be resolved iteratively.
                //

                // Example: Iterator<T>.Item becomes Generic { name: "::Item", args: [Iterator<T>] }
                // Chained: W.Inner.Item becomes Generic { name: "::Item", args: [W.Inner] }
                //  where W.Inner is also Generic { name: "::Inner", args: [W] }
                let assoc_name = assoc.name.as_str();
                let projection_name = verum_common::Text::from(format!("::{}", assoc_name));

                Ok(Type::Generic {
                    name: projection_name,
                    args: List::from(vec![base_ty]),
                })
            }

            TypeKind::CapabilityRestricted { base, capabilities } => {
                // Capability-restricted type: T with [Capabilities]
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
                //

                // Check the base type and track capabilities for capability-based
                // method filtering. The type system ensures that:
                // - T with [A, B, C] <: T with [A, B] (more caps = subtype of fewer caps)
                // - Method calls requiring capability C are only valid if C is in the set
                let base_ty = self.ast_to_type(base)?;

                // Convert AST CapabilitySet to structured TypeCapabilitySet
                // This enables proper set operations (subset, superset, intersection)
                // for capability attenuation subtyping
                let type_caps = crate::capability::TypeCapabilitySet::from_ast(capabilities);

                Ok(Type::CapabilityRestricted {
                    base: Box::new(base_ty),
                    capabilities: type_caps,
                })
            }

            // Unknown type - a safe top type (like `any` in TypeScript but safe)
            // It represents an unknown concrete type and doesn't unify with other types
            TypeKind::Unknown => Ok(Type::Unknown),

            // Record types: { field1: Type1, field2: Type2, ... }
            TypeKind::Record { fields, .. } => {
                use indexmap::IndexMap;
                let mut field_types: IndexMap<Text, Type> = IndexMap::new();
                for f in fields {
                    let ty = self.ast_to_type(&f.ty)?;
                    field_types.insert(f.name.name.clone(), ty);
                }
                Ok(Type::Record(field_types))
            }

            // Universe types: Type, Type(0), Type(1), Type(u)
            TypeKind::Universe { level } => {
                use crate::ty::UniverseLevel;
                // Honour `[types].universe_polymorphism = false`:
                // reject POLYMORPHIC universe forms (level variable
                // `Type(u)`, or expressions containing one — `Max`,
                // `Succ`). Concrete forms (`Type` and `Type(N)`)
                // are always allowed: the universe level is fixed
                // at declaration time, so no polymorphism is
                // introduced.
                //

                // Pre-fix the manifest field was tracing-only at
                // session.rs:472; the elaborator unconditionally
                // synthesised `UniverseLevel::Variable` for
                // `Type(u)` regardless of the flag — silently
                // permitting the language feature even when the
                // user explicitly disabled it.
                let level_is_polymorphic = matches!(
                    level,
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Variable(_))
                        | verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Max(_, _))
                        | verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Succ(_))
                );
                if level_is_polymorphic && !self.universe_poly_enabled {
                    return Err(TypeError::Other(verum_common::Text::from(
                        "universe-polymorphic types (e.g. `Type(u)`, `Type(max(a,b))`, \
                         `Type(succ u)`) require `[types].universe_polymorphism = true` \
                         in Verum.toml — concrete `Type` / `Type(N)` are always \
                         allowed",
                    )));
                }
                let uni_level = match level {
                    verum_common::Maybe::None => UniverseLevel::TYPE,
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Concrete(n)) => {
                        UniverseLevel::Concrete(*n)
                    }
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Variable(ident)) => {
                        // Look up the level variable in scope
                        // For now, assign a fresh variable ID based on the name
                        let var_id = self.fresh_universe_var_id(&ident.name);
                        UniverseLevel::Variable(var_id)
                    }
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Max(a, b)) => {
                        let la = self.ast_universe_level_to_internal(a);
                        let lb = self.ast_universe_level_to_internal(b);
                        la.max(lb)
                    }
                    verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Succ(inner)) => {
                        let l = self.ast_universe_level_to_internal(inner);
                        l.succ()
                    }
                };
                Ok(Type::Universe { level: uni_level })
            }

            // Meta type: meta T - compile-time type-level value
            TypeKind::Meta { inner } => {
                // For now, treat meta T as T at the type level
                // The meta qualifier is tracked for dependent type checking
                self.ast_to_type(inner)
            }

            // Type lambda: |x| T - used in sigma types and dependent type positions
            TypeKind::TypeLambda { params: _, body } => {
                // For now, evaluate the body type
                // Full dependent type support would track the lambda structure
                self.ast_to_type(body)
            }
        }
    }

    /// Infer the type for the `?` operator (try operator).
    ///

    /// Try operator type checking: ? operator desugars to match with From conversion, requires Result/Maybe return type — Error propagation with ?
    ///

    /// The `?` operator has the following semantics:
    /// - `expr?: T` where `expr: Result<T, E1>` and function returns `Result<U, E2>`
    /// - Requires `From<E1> for E2` to be implemented
    /// - Extracts the success value or early-returns the error (converted to E2)
    ///

    /// # Type Checking Rules
    ///

    /// 1. The inner expression must have type `Result<T, E>` or `Maybe<T>`
    /// 2. The enclosing function must return `Result<U, E2>` or `Maybe<U>`
    /// 3. For Result types: `E` must be convertible to `E2` via `From<E> for E2`
    /// 4. For Maybe types: no conversion needed
    ///

    /// # Error Diagnostics
    ///

    /// - **E0203**: Result type mismatch - error types not compatible
    /// - **E0204**: Multiple conversion paths - ambiguous From implementations
    /// - **E0205**: Cannot use `?` in non-Result context
    ///

    /// # Returns
    ///

    /// Returns `Ok(InferResult<T>)` where `T` is the success type from the Result/Maybe.
    /// Convert a rank-2 polymorphic function type `fn<R>(...) -> ...`
    /// to an internal `Type::Forall { vars, body: Function }`.
    /// Pushes/pops a scope for the universally-quantified type parameters;
    /// handles Type/HKT/Const/Meta/KindAnnotated param kinds.
    fn ast_to_rank2_function_type(&mut self, ast_ty: &verum_ast::ty::Type) -> Result<Type> {
        use verum_ast::ty::TypeKind;
        let TypeKind::Rank2Function { type_params, params, return_type, .. } = &ast_ty.kind
            else { unreachable!() };
                // Rank-2 polymorphic function types: fn<R>(Reducer<B, R>) -> Reducer<A, R>
                // Spec: grammar/verum.ebnf - rank2_function_type
                //

                // Represented internally as Forall { vars, body: Function }
                // This is the standard type-theoretic representation of rank-2 types.
                //

                // The universally quantified type parameters are scoped locally
                // to this function type, meaning callers must work with all possible
                // instantiations, not choose a specific one.

                // Push a scope for the rank-2 type parameters
                self.ctx.env.push_scope();

                // Convert type parameters to TypeVars and register them in scope
                let mut type_vars: List<TypeVar> = List::new();
                let mut param_names: List<Text> = List::new();

                for param in type_params {
                    use verum_ast::ty::GenericParamKind;
                    match &param.kind {
                        GenericParamKind::Type { name, bounds, .. } => {
                            // Create fresh type variable for this parameter
                            let tvar = TypeVar::fresh();
                            let type_var = Type::Var(tvar);
                            let name_text: Text = name.name.clone();

                            // Register in environment and type context so it can be resolved
                            self.ctx
                                .env
                                .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                            self.ctx.define_type(name_text.clone(), type_var);

                            // Register bounds if present
                            if !bounds.is_empty() {
                                if let Ok(protocol_bounds) =
                                    self.convert_type_bounds_to_protocol_bounds(bounds)
                                {
                                    self.register_type_var_bounds(tvar, protocol_bounds);
                                }
                            }

                            type_vars.push(tvar);
                            param_names.push(name_text);
                        }
                        GenericParamKind::HigherKinded { name, bounds, .. } => {
                            // Higher-kinded type parameter
                            let tvar = TypeVar::fresh();
                            let type_var = Type::Var(tvar);
                            let name_text: Text = name.name.clone();

                            self.ctx
                                .env
                                .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                            self.ctx.define_type(name_text.clone(), type_var);

                            // Side table: only register when a protocol bound
                            // is present — bare HKT slots can't be dispatched
                            // through anyway (see the paired site above for
                            // full rationale).
                            if !bounds.is_empty() {
                                self.hkt_type_var_by_name.insert(name_text.clone(), tvar);
                            }

                            // Register bounds if present
                            if !bounds.is_empty() {
                                if let Ok(protocol_bounds) =
                                    self.convert_type_bounds_to_protocol_bounds(bounds)
                                {
                                    self.register_type_var_bounds(tvar, protocol_bounds);
                                }
                            }

                            type_vars.push(tvar);
                            param_names.push(name_text);
                        }
                        GenericParamKind::Const { name, .. } => {
                            // Const generics - treat as type variable for now
                            let tvar = TypeVar::fresh();
                            let type_var = Type::Var(tvar);
                            let name_text: Text = name.name.clone();

                            self.ctx
                                .env
                                .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                            self.ctx.define_type(name_text.clone(), type_var);

                            // Audit-A4: register the const-generic in the
                            // const-generic environment alongside Meta-params.
                            // See the parallel comment on the Meta arm for
                            // the full rationale.
                            self.ctx.meta_param_environment.insert(
                                name_text.clone(),
                                crate::context::MetaParamBinding::Symbolic,
                            );

                            type_vars.push(tvar);
                            param_names.push(name_text);
                        }
                        GenericParamKind::Meta { name, .. } => {
                            // Meta parameters need type variables too for use in type arguments
                            // E.g., fn foo<N: meta USize>() -> Array<N>
                            let tvar = TypeVar::fresh();
                            let type_var = Type::Var(tvar);
                            let name_text: Text = name.name.clone();

                            self.ctx
                                .env
                                .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                            self.ctx.define_type(name_text.clone(), type_var);

                            // Audit-A4: register the meta-param in the
                            // const-generic environment. Until a concrete
                            // instantiation is observed (e.g. `foo::<5>()`),
                            // the binding stays Symbolic — refinement
                            // predicates referencing this name will be
                            // passed verbatim to SMT, where the solver can
                            // reason about the bounds. When instantiation
                            // does land in a future commit, the binding
                            // promotes to `Bound(MetaValue::Int(5))` and
                            // `substitute_in_refinement_predicate` inlines
                            // the value at every reference site.
                            self.ctx.meta_param_environment.insert(
                                name_text.clone(),
                                crate::context::MetaParamBinding::Symbolic,
                            );

                            type_vars.push(tvar);
                            param_names.push(name_text);
                        }
                        GenericParamKind::KindAnnotated {
                            name,
                            kind: kind_ann,
                            bounds,
                        } => {
                            // Kind-annotated HKT parameter in rank-2 function type context
                            let tvar = TypeVar::fresh();
                            let type_var = Type::Var(tvar);
                            let name_text: Text = name.name.clone();

                            self.ctx
                                .env
                                .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                            self.ctx.define_type(name_text.clone(), type_var);

                            // Register kind with kind_inferer
                            let infer_kind = Self::ast_kind_to_infer_kind(kind_ann);
                            self.kind_inferer
                                .register_type_constructor(name_text.clone(), infer_kind);

                            if !bounds.is_empty() {
                                if let Ok(protocol_bounds) =
                                    self.convert_type_bounds_to_protocol_bounds(bounds)
                                {
                                    self.register_type_var_bounds(tvar, protocol_bounds);
                                }
                            }

                            type_vars.push(tvar);
                            param_names.push(name_text);
                        }
                        GenericParamKind::Lifetime { .. }
                        | GenericParamKind::Context { .. }
                        | GenericParamKind::Level { .. } => {
                            // These don't introduce type variables in the same way
                        }
                    }
                }

                // Convert parameter and return types (now type params are in scope)
                let param_types: Result<List<_>> =
                    params.iter().map(|p| self.ast_to_type(p)).collect();
                let param_types = param_types?;
                let ret_type = self.ast_to_type(return_type)?;

                // Pop the scope - type parameters are no longer visible
                self.ctx.env.pop_scope();

                // Clean up type definitions (they were added to the flat type map)
                for name in &param_names {
                    self.ctx.remove_type(name);
                }

                // Build the inner function type
                let fn_type = Type::function(param_types, ret_type);

                // Wrap in Forall if there are quantified type parameters
                if type_vars.is_empty() {
                    // Degenerate case: no type parameters, just a regular function
                    Ok(fn_type)
                } else {
                    Ok(Type::Forall {
                        vars: type_vars,
                        body: Box::new(fn_type),
                    })
                }
    }

    /// Convert a `TypeKind::Generic` AST node (e.g. `List<T>`, `Matrix<Float,2,3>`)
    /// to an internal `Type`.
    /// Handles: type-args (Type/Const/Lifetime/Binding), arity validation,
    /// variant substitution, HKT TypeApp, dependent Eq/Fin well-formedness.
    fn ast_to_generic_type(&mut self, ast_ty: &verum_ast::ty::Type) -> Result<Type> {
        use verum_ast::ty::TypeKind;
        let TypeKind::Generic { base, args } = &ast_ty.kind
            else { unreachable!() };
                // Generic type with arguments: List<T>, Tensor<Float, [2, 3]>
                // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .2 - Generic type arguments
                let base_ty = self.ast_to_type(base)?;

                // DEBUG: trace HKT TypeApp construction

                // Separate type arguments and lifetime arguments
                // Lifetimes are tracked separately for borrow checking
                let mut type_args: List<Type> = List::new();
                let mut lifetime_args: List<verum_common::Text> = List::new();

                for arg in args {
                    use verum_ast::ty::GenericArg;
                    match arg {
                        GenericArg::Type(ty) => {
                            let arg_ty = self.ast_to_type(ty)?;
                            type_args.push(arg_ty);
                        }
                        GenericArg::Const(expr) => {
                            // Const generic arguments are compile-time values.
                            // Represent them as `Type::Meta` with the concrete `value` set
                            // (see `eval_const_arg`) so dimension mismatches like
                            // `Matrix<Float, 7, 7>` vs `Matrix<Float, 5, 5>` are caught
                            // during unification instead of being collapsed to the same
                            // `Int`-typed argument list.
                            match self.eval_const_arg(expr) {
                                Some(ty) => type_args.push(ty),
                                None => {
                                    let expr_ty = self.synth_expr(expr)?;
                                    type_args.push(expr_ty.ty);
                                }
                            }
                        }
                        GenericArg::Lifetime(lifetime) => {
                            // Lifetime arguments: 'a, 'b, 'static
                            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .2 - Lifetime parameters
                            //

                            // Verum has implicit lifetimes in most cases (like elision in Rust),
                            // but explicit lifetimes can be specified for complex cases:
                            // - &'a T for reference with named lifetime
                            // - Struct<'a, T> for types parameterized by lifetime
                            //

                            // For now, we track lifetime names but don't enforce them
                            // during type checking (CBGR handles memory safety)
                            lifetime_args.push(lifetime.name.clone());

                            // Create a phantom lifetime type for tracking
                            // This allows the type system to preserve lifetime info
                            // even though Verum uses CBGR instead of borrow checking
                            let lifetime_ty = Type::lifetime(lifetime.name.clone());
                            type_args.push(lifetime_ty);
                        }
                        GenericArg::Binding(binding) => {
                            // Type bindings like Iterator<Item = T>
                            let arg_ty = self.ast_to_type(&binding.ty)?;
                            type_args.push(arg_ty);
                        }
                    }
                }

                // Validate generic arity when we have declaration info.
                // Prevents silent acceptance of `List<Int, Int>` when List<T>
                // expects exactly one type argument. We only error when the
                // expected count is known (stored TypeVar order), otherwise
                // we stay permissive to avoid regressions.
                // Validate generic arity when we can determine the expected
                // parameter count. Prevents silent acceptance of things like
                // `List<Int, Int>` where List<T> only expects one type arg.
                // Expected count comes from `__type_params_{name}` (records)
                // or `__type_var_order_{name}` (variant types).
                if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                    let name_opt: Option<verum_common::Text> =
                        path.segments.last().and_then(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                            _ => None,
                        });
                    if let Some(type_name) = name_opt {
                        let params_key = format!("__type_params_{}", type_name);
                        let order_key = format!("__type_var_order_{}", type_name);
                        let expected: Option<usize> = match self.ctx.lookup_type(&params_key) {
                            Some(Type::Record(params_map)) => Some(params_map.len()),
                            _ => match self.ctx.lookup_type(&order_key) {
                                // Count *all* parameter slots, not just `Type::Var`.
                                // `type Matrix<T, Rows: meta Int, Cols: meta Int>`
                                // stores T as a Var and `Rows`/`Cols` as meta-Int
                                // placeholders that aren't `Type::Var`. Filtering
                                // down to vars made the arity checker reject
                                // legitimate three-arg uses like
                                // `Matrix<Float, 2, 3>` with
                                // "expects 1 type argument(s), but 3 were provided".
                                Some(Type::Tuple(type_vars)) => Some(type_vars.len()),
                                _ => None,
                            },
                        };
                        if let Some(expected_count) = expected {
                            let provided = type_args
                                .iter()
                                .filter(|t| !matches!(t, Type::Lifetime { .. }))
                                .count();
                            if expected_count > 0 && provided > expected_count {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "type `{}` expects {} type argument(s), but {} were provided",
                                    type_name, expected_count, provided
                                ))));
                            }
                        }
                    }
                }

                // Return the base type with arguments
                // Lifetimes are embedded in type_args as Type::Lifetime
                let result_type = match &base_ty {
                    Type::Named { path, .. } => Type::Named {
                        path: path.clone(),
                        args: type_args.clone(),
                    },
                    Type::Generic {
                        name,
                        args: _existing_args,
                    } => {
                        // Check if this is an associated type projection (e.g., Self.F)
                        // Associated types are represented as Generic { name: "::F", args: [Self] }
                        if name.starts_with("::") {
                            // For HKT associated types like Self.F<T>, we need to create a TypeApp
                            // The constructor is the projection itself, args are the type arguments
                            Type::TypeApp {
                                constructor: Box::new(base_ty.clone()),
                                args: type_args.clone(),
                            }
                        } else {
                            // Regular generic type - just update the args
                            Type::Generic {
                                name: name.clone(),
                                args: type_args.clone(),
                            }
                        }
                    }
                    // CRITICAL FIX: Handle HKT type parameter application
                    // When base is a TypeVar (HKT param like F from `fn map<F<_>: Functor, A>(fa: F<A>)`),
                    // we create a TypeApp to represent F<A> with the TypeVar as constructor.
                    // This allows unification to bind the TypeVar to a concrete constructor.
                    // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
                    Type::Var(_) => Type::TypeApp {
                        constructor: Box::new(base_ty.clone()),
                        args: type_args.clone(),
                    },
                    // Handle TypeConstructor for backwards compatibility
                    Type::TypeConstructor { .. } => Type::TypeApp {
                        constructor: Box::new(base_ty.clone()),
                        args: type_args.clone(),
                    },
                    // CRITICAL FIX: For Variant base types (like Maybe, Result), we need to
                    // substitute the type arguments into the variant payloads.
                    // This ensures Maybe<Maybe<Int>> has correctly substituted inner types.
                    // For Result<T, E>, Ok(T) should substitute T and Err(E) should substitute E.
                    Type::Variant(variants) => {
                        // Build type parameter substitution from original type definition
                        // This requires looking up the original type's parameter names
                        if !type_args.is_empty() {
                            // CRITICAL FIX: Get TypeVars in declaration order for proper substitution.
                            // Type arguments come in declaration order (e.g., for Validated<E, A>, first
                            // arg is E, second is A). We need to map TypeVars to args in the same order.
                            //

                            // Strategy:
                            // 1. Try to look up stored TypeVar order from __type_var_order_{type_name}
                            // 2. Fall back to collecting TypeVars recursively from variant payloads
                            //

                            // The stored order is authoritative when available because it was created
                            // at type registration time with the correct declaration order.
                            let type_var_order: List<TypeVar> = {
                                // Extract the type name directly from the AST base node.
                                // This avoids variant_type_names collisions when different types
                                // share the same variant constructor names (e.g., Validated vs Validation
                                // both having Valid/Invalid).
                                let maybe_type_name: Option<verum_common::Text> =
                                    if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                                        path.segments.last().and_then(|seg| match seg {
                                            verum_ast::ty::PathSegment::Name(id) => {
                                                Some(id.name.clone())
                                            }
                                            _ => None,
                                        })
                                    } else {
                                        // Fallback: use variant_type_names registry
                                        let variant_type = Type::Variant(variants.clone());
                                        let sig_opt = Self::variant_type_signature(&variant_type);
                                        if let Some(ref sig) = sig_opt {
                                            self.variant_type_names.get(sig).cloned()
                                        } else {
                                            None
                                        }
                                    };
                                // Try stored TypeVar order if type name is available
                                let stored_order: Option<List<TypeVar>> =
                                    maybe_type_name.as_ref().and_then(|type_name| {
                                        let type_var_order_key =
                                            format!("__type_var_order_{}", type_name);
                                        match self.ctx.lookup_type(&type_var_order_key) {
                                            verum_common::Maybe::Some(Type::Tuple(type_vars)) => {
                                                let vars: List<TypeVar> = type_vars
                                                    .iter()
                                                    .filter_map(|t| {
                                                        if let Type::Var(tv) = t {
                                                            Some(*tv)
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .collect();
                                                if !vars.is_empty() { Some(vars) } else { None }
                                            }
                                            _ => None,
                                        }
                                    });

                                if let Some(order) = stored_order {
                                    order
                                } else {
                                    // Fallback: Collect type variables recursively from variant payloads.
                                    // This preserves order of first appearance but may not match declaration
                                    // order if variants are declared differently than type params.
                                    fn collect_type_vars_ordered(
                                        ty: &Type,
                                        vars: &mut List<TypeVar>,
                                    ) {
                                        match ty {
                                            Type::Var(tv) => {
                                                if !vars.iter().any(|v| v == tv) {
                                                    vars.push(*tv);
                                                }
                                            }
                                            Type::Function {
                                                params,
                                                return_type,
                                                ..
                                            } => {
                                                for p in params {
                                                    collect_type_vars_ordered(p, vars);
                                                }
                                                collect_type_vars_ordered(return_type, vars);
                                            }
                                            Type::Tuple(types) => {
                                                for t in types {
                                                    collect_type_vars_ordered(t, vars);
                                                }
                                            }
                                            Type::Array { element, .. }
                                            | Type::Slice { element } => {
                                                collect_type_vars_ordered(element, vars);
                                            }
                                            Type::Record(fields) => {
                                                for t in fields.values() {
                                                    collect_type_vars_ordered(t, vars);
                                                }
                                            }
                                            Type::Variant(inner_variants) => {
                                                for t in inner_variants.values() {
                                                    collect_type_vars_ordered(t, vars);
                                                }
                                            }
                                            Type::Reference { inner, .. }
                                            | Type::CheckedReference { inner, .. }
                                            | Type::UnsafeReference { inner, .. }
                                            | Type::Ownership { inner, .. }
                                            | Type::Pointer { inner, .. }
                                            | Type::VolatilePointer { inner, .. } => {
                                                collect_type_vars_ordered(inner, vars);
                                            }
                                            Type::Named { args, .. }
                                            | Type::Generic { args, .. } => {
                                                for arg in args {
                                                    collect_type_vars_ordered(arg, vars);
                                                }
                                            }
                                            Type::Future { output } => {
                                                collect_type_vars_ordered(output, vars);
                                            }
                                            Type::TypeApp { constructor, args } => {
                                                collect_type_vars_ordered(constructor, vars);
                                                for arg in args {
                                                    collect_type_vars_ordered(arg, vars);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }

                                    let mut vars: List<TypeVar> = List::new();
                                    for (_, payload_ty) in variants.iter() {
                                        collect_type_vars_ordered(payload_ty, &mut vars);
                                    }
                                    vars
                                }
                            };

                            // Build substitution map from type variables to type arguments
                            // Note: Substitution is IndexMap<TypeVar, Type> per ty.rs
                            let mut subst_map: indexmap::IndexMap<TypeVar, Type> =
                                indexmap::IndexMap::new();
                            for (idx, tv) in type_var_order.iter().enumerate() {
                                if let Some(arg_ty) = type_args.get(idx) {
                                    subst_map.insert(*tv, arg_ty.clone());
                                }
                            }

                            // CRITICAL FIX: Apply substitution RECURSIVELY to each variant payload
                            // using Type::apply_subst which handles all nested types correctly.
                            // Previous implementation only substituted top-level Type::Var payloads.
                            let mut new_variants = indexmap::IndexMap::new();
                            for (variant_name, payload_ty) in variants {
                                let new_payload = payload_ty.apply_subst(&subst_map);
                                new_variants.insert(variant_name.clone(), new_payload);
                            }
                            Type::Variant(new_variants)
                        } else {
                            base_ty.clone()
                        }
                    }
                    // CRITICAL FIX: When base resolves to Placeholder (forward reference during
                    // two-pass compilation), preserve type arguments by creating a Generic type.
                    // This ensures Rev<Self> correctly preserves the Self argument rather than
                    // losing it and becoming just Placeholder { name: "Rev" }.
                    Type::Placeholder { name, .. } => Type::Generic {
                        name: name.clone(),
                        args: type_args.clone(),
                    },
                    _ => base_ty.clone(),
                };

                // Verify dependent type constraints for Eq and Fin types
                // Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — (Equality Types) and Section 3.3 (Fin)
                if self.has_dependent_types() {
                    if let Type::Named { ref path, .. } = result_type {
                        if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last()
                        {
                            let type_name = ident.name.as_str();

                            match type_name {
                                "Eq" if args.len() >= 3 => {
                                    // Eq<A, lhs, rhs> - Propositional equality type
                                    // Verify that both sides are well-formed and have type A
                                    use crate::dependent_helpers::convert_internal_to_ast;
                                    use crate::dependent_integration::DependentTypeConstraint;

                                    let value_type = convert_internal_to_ast(&type_args[0]);

                                    // Extract lhs and rhs expressions from type args
                                    let lhs = self.type_arg_to_expr(args.get(1), ast_ty.span);
                                    let rhs = self.type_arg_to_expr(args.get(2), ast_ty.span);

                                    let constraint = DependentTypeConstraint::Equality {
                                        value_type,
                                        lhs,
                                        rhs,
                                        span: ast_ty.span,
                                    };

                                    self.verify_dependent_type_constraint(
                                        &constraint,
                                        ast_ty.span,
                                    )?;
                                }
                                "Fin" if !args.is_empty() => {
                                    // Fin<n> - Finite natural numbers less than n
                                    // Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .3
                                    //

                                    // For well-formedness checking of Fin<n>:
                                    // 1. Verify bound n is non-negative
                                    // 2. Create a symbolic value constrained to [0, n)
                                    //

                                    // The symbolic value represents any valid inhabitant of Fin<n>
                                    // and allows SMT verification that n is a valid bound.
                                    use crate::dependent_integration::DependentTypeConstraint;

                                    let bound = self.type_arg_to_expr(args.first(), ast_ty.span);

                                    // Create symbolic bounded value expression for well-formedness check.
                                    // This represents an arbitrary value in range [0, n) and is used
                                    // to verify that n is a valid bound (n > 0 for non-empty Fin type).
                                    // When concrete values are assigned to Fin<n> variables, separate
                                    // verification ensures value < n.
                                    let symbolic_value =
                                        self.create_symbolic_fin_value(ast_ty.span);

                                    let constraint = DependentTypeConstraint::FinType {
                                        value: symbolic_value,
                                        bound,
                                        span: ast_ty.span,
                                    };

                                    // Verify bound is valid (well-formedness: n must be positive)
                                    self.verify_dependent_type_constraint(
                                        &constraint,
                                        ast_ty.span,
                                    )?;
                                }
                                _ => {}
                            }
                        }
                    }
                }

                Ok(result_type)
    }

    /// Resolve a qualified associated-type projection:
    /// `<T as Protocol>::Item`, `T.Item` (Verum sugar), or module-qualified path
    /// (`super.X`, `cog.X.Y`). Defers to a Generic projection when self-type
    /// contains unresolved vars.
    fn ast_to_qualified_type(&mut self, ast_ty: &verum_ast::ty::Type) -> Result<Type> {
        use verum_ast::ty::TypeKind;
        let TypeKind::Qualified { self_ty, trait_ref, assoc_name } = &ast_ty.kind
            else { unreachable!() };
                // CRITICAL: Before treating as associated type, check if this is actually a
                // module-qualified path (super.X, crate.X.Y.Z). The parser decomposes these
                // into nested Qualified types, but they should be resolved as module paths.
                //

                // Flatten nested Qualified types to extract the full path:
                //  Qualified { self_ty: Qualified { self_ty: Path(crate), assoc: database }, assoc: connection }
                //  -> [crate, database, connection]
                {
                    fn flatten_qualified_to_segments(
                        ty: &verum_ast::ty::Type,
                    ) -> Option<Vec<verum_ast::ty::PathSegment>> {
                        use verum_ast::ty::{PathSegment, TypeKind};
                        match &ty.kind {
                            TypeKind::Path(path) if path.segments.len() == 1 => {
                                let seg = &path.segments[0];
                                match seg {
                                    PathSegment::Super | PathSegment::Cog => {
                                        Some(vec![seg.clone()])
                                    }
                                    PathSegment::Name(ident) => Some(vec![seg.clone()]),
                                    _ => None,
                                }
                            }
                            TypeKind::Qualified {
                                self_ty: inner,
                                assoc_name: inner_assoc,
                                ..
                            } => {
                                if let Some(mut segs) = flatten_qualified_to_segments(inner) {
                                    segs.push(PathSegment::Name(inner_assoc.clone()));
                                    Some(segs)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    }

                    if let Some(mut segments) = flatten_qualified_to_segments(ast_ty) {
                        // Check if the root segment is super or crate (module path prefix)
                        let is_module_path = matches!(
                            segments.first(),
                            Some(verum_ast::ty::PathSegment::Super)
                                | Some(verum_ast::ty::PathSegment::Cog)
                        );

                        // Also check if the root segment is an inline module name
                        let is_inline_module =
                            if let Some(verum_ast::ty::PathSegment::Name(ident)) = segments.first()
                            {
                                self.inline_modules
                                    .contains_key(&verum_common::Text::from(ident.name.as_str()))
                            } else {
                                false
                            };

                        if is_module_path || is_inline_module {
                            // This is a module-qualified path, not an associated type
                            // Reconstruct the full path and resolve via module path resolution
                            // The full segments include the assoc_name as the final segment
                            segments.push(verum_ast::ty::PathSegment::Name(assoc_name.clone()));
                            let full_path = verum_ast::ty::Path::new(segments.into(), ast_ty.span);
                            return self.resolve_qualified_type(&full_path, ast_ty.span);
                        }
                    }
                }

                // Qualified type: <T as Protocol>::AssocType or T.Item (Verum syntax)
                // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated type resolution
                //

                // Resolution strategy:
                // 1. Convert AST self_ty to internal Type representation
                // 2. If trait_ref is empty (T.Item syntax), use try_find_associated_type
                // 3. Otherwise, resolve via explicit protocol implementation
                // 4. If self_type has unresolved type variables, create a deferred projection
                // 5. Handle generic associated types (GATs) by applying type substitutions
                let self_type = self.ast_to_type(self_ty)?;
                let assoc_name_text: Text = assoc_name.name.clone();

                // Check if trait_ref is empty - this means we're using Verum T.Item syntax
                // without specifying the protocol explicitly
                let is_empty_protocol = trait_ref.segments.is_empty();

                // Convert protocol path to text for error messages (may be empty)
                let protocol_name_text = if is_empty_protocol {
                    "".into()
                } else {
                    self.extract_protocol_name_from_path(trait_ref)
                };

                // Check if self_type contains unresolved type variables
                // If so, we need to defer resolution until unification resolves them
                // Also check if self_type is the symbolic "Self" type - this also needs deferral
                // because Self is abstract within protocol definitions
                let is_self_type = match &self_type {
                    Type::Named { path, args } if args.is_empty() => path
                        .as_ident()
                        .is_some_and(|ident| ident.as_str() == "Self"),
                    _ => false,
                };

                if self.has_unresolved_vars(&self_type) || is_self_type {
                    // Create a deferred projection type that will be resolved during normalization
                    // Use the new ::AssocName format with base type in args
                    let projection_name: Text = Text::from(format!("::{}", assoc_name_text));

                    return Ok(Type::Generic {
                        name: projection_name,
                        args: List::from(vec![self_type]),
                    });
                }

                // For empty protocol paths (T.Item syntax), use try_find_associated_type
                // which searches all implementations for the base type
                if is_empty_protocol {
                    if let Some(resolved_ty) = self
                        .protocol_checker
                        .read()
                        .try_find_associated_type(&self_type, &assoc_name_text)
                    {
                        return Ok(self.normalize_type(&resolved_ty));
                    }

                    // If we couldn't resolve immediately, create a deferred projection
                    // The normalization pass will try again when more information is available
                    let projection_name: Text = format!("::{}", assoc_name_text).into();
                    return Ok(Type::Generic {
                        name: projection_name,
                        args: List::from(vec![self_type]),
                    });
                }

                // Try to resolve the associated type through the explicit protocol implementation
                match self.protocol_checker.read().infer_associated_type(
                    &self_type,
                    trait_ref,
                    &assoc_name_text,
                ) {
                    Ok(resolved_ty) => {
                        // Successfully resolved the associated type
                        // Normalize the result to handle nested associated types
                        Ok(self.normalize_type(&resolved_ty))
                    }
                    Err(crate::protocol::ProtocolError::NotImplemented { .. }) => {
                        // The type does not implement the protocol
                        // Try to look up a default associated type from the protocol definition
                        if let Maybe::Some(proto_def) = self
                            .protocol_checker
                            .read()
                            .get_protocol(&protocol_name_text)
                        {
                            if let Some(assoc_def) =
                                proto_def.associated_types.get(&assoc_name_text)
                            {
                                if let Maybe::Some(ref default_ty) = assoc_def.default {
                                    return Ok(default_ty.clone());
                                }
                            }
                        }

                        // Create a placeholder projection for deferred resolution
                        let projection_name: Text = format!("::{}", assoc_name_text).into();
                        Ok(Type::Generic {
                            name: projection_name,
                            args: List::from(vec![self_type]),
                        })
                    }
                    Err(crate::protocol::ProtocolError::AssociatedTypeNotSpecified { .. }) => {
                        // The associated type is not specified in the implementation
                        // Check for a default in the protocol definition
                        if let Maybe::Some(proto_def) = self
                            .protocol_checker
                            .read()
                            .get_protocol(&protocol_name_text)
                        {
                            if let Some(assoc_def) =
                                proto_def.associated_types.get(&assoc_name_text)
                            {
                                if let Maybe::Some(ref default_ty) = assoc_def.default {
                                    return Ok(default_ty.clone());
                                }
                            }
                        }

                        // No default found - create a deferred projection
                        let projection_name: Text = format!("::{}", assoc_name_text).into();
                        Ok(Type::Generic {
                            name: projection_name,
                            args: List::from(vec![self_type]),
                        })
                    }
                    Err(crate::protocol::ProtocolError::ProtocolNotFound { name }) => {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Protocol '{}' not found",
                            name
                        ))))
                    }
                    Err(e) => {
                        // Other protocol errors
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "Error resolving associated type: {}",
                            e
                        ))))
                    }
                }
    }

    pub(super) fn infer_try_operator(&mut self, inner_expr: &Expr, try_span: Span) -> Result<InferResult> {
        // Step 1: Infer type of inner expression
        let inner_result = self.synth_expr(inner_expr)?;
        let inner_ty = self.unifier.apply(&inner_result.ty);

        // Never propagation: `never_value?` should return Never
        if matches!(inner_ty, Type::Never) {
            return Ok(InferResult::new(Type::Never));
        }

        // If the inner type is not Result/Maybe, the inner expression might be a call
        // to a throws function whose return type wasn't properly resolved to Result<T, E>.
        // Try to determine the error type from the caller's throws clause.
        //

        // Previously this fallback ALWAYS wrapped non-Try inner types in
        // `Result<inner, fresh>`, even when outside a throws context. That
        // silently produced a `Result<Never, _>` residual which then mismatched
        // with a `Maybe<_>` outer function, reporting a confusing
        // "cannot convert Result<Never, _> to Maybe<Never>" for code that
        // simply forgot that `?` needs a Try-implementing type. Now we only
        // wrap when an actual throws clause is in scope — otherwise the
        // standard Step 2 resolution emits `NotResultOrMaybe`, pointing the
        // user at the real issue (wrong method, missing Maybe wrapper, etc.).
        let inner_ty = if self
            .protocol_checker
            .read()
            .resolve_try_protocol(&inner_ty)
            .is_none()
        {
            // Only wrap when the caller's throws clause supplies an error
            // type — that's the scenario the fallback was designed for
            // (throws function whose return type wasn't yet elaborated).
            let error_ty_opt = if let Maybe::Some(ref throws_types) = self.current_function_throws {
                if !throws_types.is_empty() {
                    let ety = if throws_types.len() == 1 {
                        throws_types[0].clone()
                    } else {
                        let mut variants = indexmap::IndexMap::new();
                        for t in throws_types.iter() {
                            if let Type::Variant(v) = t {
                                for (k, v) in v.iter() {
                                    variants.insert(k.clone(), v.clone());
                                }
                            } else {
                                variants.insert(t.to_text(), t.clone());
                            }
                        }
                        Type::Variant(variants)
                    };
                    Some(ety)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(error_ty) = error_ty_opt {
                let wrapped = Type::result(inner_ty.clone(), error_ty);
                if self
                    .protocol_checker
                    .read()
                    .resolve_try_protocol(&wrapped)
                    .is_some()
                {
                    wrapped
                } else {
                    inner_ty
                }
            } else {
                inner_ty
            }
        } else {
            inner_ty
        };

        // Step 2: Resolve Try protocol for inner type
        // Protocol-based Try resolution: ? operator uses Carrier protocol to convert between error types
        // The protocol checker now handles all cases:
        // - Type variables: returns fresh type variables for Output/Residual
        // - Variant types: recognizes Ok/Err and Some/None patterns
        // - Named/Generic types: queries protocol implementations
        let inner_try = match self.protocol_checker.read().resolve_try_protocol(&inner_ty) {
            Some(resolution) => resolution,
            None => {
                // Type does not implement Try protocol
                return Err(TypeError::NotResultOrMaybe {
                    ty: inner_ty.to_text(),
                    span: inner_expr.span,
                });
            }
        };

        // Step 3: Get the function's return type
        // If the function has a throws clause, treat the return type as Result<T, E>
        let function_return_type = match &self.current_function_return_type {
            Maybe::Some(ret_ty) => {
                // If the function has a throws clause and the return type is not already Result/Maybe,
                // wrap it in Result<T, E> so the ? operator works inside throws functions
                if let Maybe::Some(ref throws_types) = self.current_function_throws {
                    let is_already_result = self
                        .protocol_checker
                        .read()
                        .resolve_try_protocol(ret_ty)
                        .is_some();
                    if !is_already_result && !throws_types.is_empty() {
                        // Construct the error type from all throws types
                        // For a single type: use it directly
                        // For multiple types: build a Variant (union) type
                        let error_ty = if throws_types.len() == 1 {
                            throws_types[0].clone()
                        } else {
                            // Build a variant (union) type from all throws types
                            let mut variants = indexmap::IndexMap::new();
                            for t in throws_types.iter() {
                                match t {
                                    // If throws type is already a variant, flatten its variants
                                    Type::Variant(inner_variants) => {
                                        for (name, ty) in inner_variants.iter() {
                                            variants.insert(name.clone(), ty.clone());
                                        }
                                    }
                                    // Named types become variant entries
                                    _ => {
                                        variants.insert(t.to_text(), t.clone());
                                    }
                                }
                            }
                            Type::Variant(variants)
                        };
                        Type::result(ret_ty.clone(), error_ty)
                    } else {
                        ret_ty.clone()
                    }
                } else {
                    ret_ty.clone()
                }
            }
            Maybe::None => {
                // E0205: Cannot use ? outside of a function
                return Err(TypeError::TryOperatorOutsideFunction { span: try_span });
            }
        };

        // Step 4: Resolve Try protocol for return type
        let outer_try = match self
            .protocol_checker
            .read()
            .resolve_try_protocol(&function_return_type)
        {
            Some(resolution) => resolution,
            None => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG try_op] function_return_type does NOT implement Try");
                // E0205: Function return type does not implement Try
                let fn_name_opt = self.current_function_name.as_ref().map(|t| t.to_string());
                let return_span_opt = self
                    .current_function_return_span
                    .as_ref()
                    .map(|s| span_to_line_col(*s));
                let inner_ty_text = inner_ty.to_text();
                let return_ty_text = function_return_type.to_text();
                let fn_name_text: Option<Text> = fn_name_opt.map(|s| s.into());
                let diag = verum_diagnostics::e0205_try_in_non_result_context(
                    span_to_line_col(try_span),
                    &inner_ty_text,
                    &return_ty_text,
                    fn_name_text.as_ref(),
                    return_span_opt,
                );
                return Err(TypeError::TryInNonResultContext {
                    expr_type: inner_ty.to_text(),
                    function_return_type: function_return_type.to_text(),
                    span: try_span,
                    diagnostic: diag,
                });
            }
        };

        // Step 5: Check residual type compatibility via FromResidual
        // The function's return type must implement FromResidual<inner.Residual>
        if !self
            .protocol_checker
            .read()
            .can_convert_residual(&function_return_type, &inner_try.residual)
        {
            // Check if error types can be unified (type variables, etc.)
            if !self.check_residual_type_compatibility(
                &inner_try.residual,
                &outer_try.residual,
                try_span,
            ) {
                // Extract error types from residuals for better error messages
                let inner_err = self.extract_error_from_residual(&inner_try.residual);
                let outer_err = self.extract_error_from_residual(&outer_try.residual);

                // Check if From conversion exists between error types
                if let (Maybe::Some(ie), Maybe::Some(oe)) = (&inner_err, &outer_err) {
                    if self.check_from_implementation(ie, oe) {
                        // From conversion exists, residuals are compatible
                        return Ok(InferResult::new(inner_try.output));
                    }
                }

                // Check if inner error is a subtype of the throws clause union
                // This handles: throws(A | B) accepting ? on functions that throw A or B
                {
                    let throws_types_clone = self.current_function_throws.clone();
                    if let Maybe::Some(throws_types) = throws_types_clone {
                        if let Maybe::Some(ref ie) = inner_err {
                            let inner_compatible = throws_types.iter().any(|throws_ty| {
                                // Check: inner error type matches or is subtype of a throws type
                                ie == throws_ty
                                    || self.unifier.unify(ie, throws_ty, try_span).is_ok()
                                    || self.check_from_implementation(ie, throws_ty)
                                    // Also check if inner error's variants are all contained
                                    // in the throws type's variants
                                    || match (ie, throws_ty) {
                                        (Type::Variant(inner_v), Type::Variant(outer_v)) => {
                                            inner_v.keys().all(|k| outer_v.contains_key(k))
                                        }
                                        _ => false,
                                    }
                            });
                            if inner_compatible {
                                return Ok(InferResult::new(inner_try.output));
                            }
                        }
                    }
                }

                // E0203: Residual type mismatch
                let inner_residual_text = inner_try.residual.to_text();
                let outer_residual_text = outer_try.residual.to_text();
                let diag = verum_diagnostics::e0203_result_type_mismatch(
                    span_to_line_col(try_span),
                    &inner_residual_text,
                    &outer_residual_text,
                    span_to_line_col(inner_expr.span),
                    self.current_function_return_span
                        .as_ref()
                        .map(|s| span_to_line_col(*s)),
                );
                return Err(TypeError::ResultTypeMismatch {
                    inner_error: inner_residual_text,
                    outer_error: outer_residual_text,
                    span: try_span,
                    diagnostic: diag,
                });
            }
        }

        // Step 6: Return the output type (success value)
        Ok(InferResult::new(inner_try.output))
    }

    /// Infer type for a plain try block: try { expr } -> Result<T, E>
    ///

    /// A plain try block creates a Result from its body:
    /// - T is the type of the block's final expression
    /// - E is inferred from ? operators within the block
    /// - If no ? operators, E defaults to Never (infallible)
    ///

    /// The final expression is auto-wrapped in Ok().
    ///

    /// Error handling: Result<T, E> and Maybe<T> types, try (?) operator with automatic From conversion, error propagation — Section 6.3 - Try blocks
    ///

    /// STDLIB-AGNOSTIC: Uses protocol-based detection via Try protocol.
    /// Works with any type implementing Try, not just Result/Maybe.
    pub(super) fn infer_try_block(&mut self, inner_block: &Expr, try_span: Span) -> Result<InferResult> {
        // Step 1: Find error type from ? operators inside the block
        let error_type = self
            .find_try_operator_error_type(inner_block)
            .unwrap_or_else(|| Type::Never); // Default to Never if no ? operators

        // Step 2: Temporarily set function return type to Result<T, E> so that
        // the ? operator inside the try block passes type checking.
        let saved_return_type = self.current_function_return_type.clone();
        let try_ok_var = Type::Var(crate::ty::TypeVar::fresh());
        self.current_function_return_type =
            Maybe::Some(Type::result(try_ok_var.clone(), error_type.clone()));

        // Step 3: Infer the block's type
        let block_result = self.synth_expr(inner_block)?;
        let success_type = block_result.ty;

        // Restore original function return type
        self.current_function_return_type = saved_return_type;

        // Step 3: Determine result type using PROTOCOL-BASED detection
        // If the block already returns a Try-compatible type, use it directly
        // Otherwise, wrap in Result<T, E>
        let result_type = if self.is_try_compatible_type(&success_type) {
            // Block already returns a Try-compatible type
            // Extract its structure and unify error types if needed
            let (_, block_error_type) = self.extract_try_output_types(&success_type);

            // Unify error types if both present and not Never
            if error_type != Type::Never && block_error_type != Type::Never {
                let _ = self.unifier.unify(&error_type, &block_error_type, try_span);
            }

            success_type.clone()
        } else {
            // Block returns non-Try type - wrap in Result<T, E>
            Type::result(success_type, error_type)
        };

        Ok(InferResult::new(result_type))
    }

    /// Check if two residual types are compatible for ? operator.
    fn check_residual_type_compatibility(
        &mut self,
        inner_residual: &Type,
        outer_residual: &Type,
        span: Span,
    ) -> bool {
        // Type variables are compatible (will be resolved later)
        if matches!(inner_residual, Type::Var(_)) || matches!(outer_residual, Type::Var(_)) {
            return true;
        }

        // Try to unify the residual types
        self.unifier
            .unify(inner_residual, outer_residual, span)
            .is_ok()
    }

    /// Extract error type from a residual type.
    /// STDLIB-AGNOSTIC: Uses protocol-based detection via Try protocol.
    /// Works with any type implementing Try, not just Result/Maybe.
    fn extract_error_from_residual(&self, residual: &Type) -> Maybe<Type> {
        // Use protocol-based resolution to get the error type from residual
        // The protocol_checker has a method for this that works structurally
        self.protocol_checker
            .read()
            .extract_error_from_residual(residual)
    }

    /// Check if `From<source> for target` is implemented.
    ///

    /// This queries the protocol checker to see if the From protocol is satisfied.
    /// Returns true if the conversion is possible, false otherwise.
    ///

    /// # Type Variable Handling
    ///

    /// When either source or target is a type variable, we return true to allow
    /// type inference to proceed. The actual error type compatibility will be
    /// verified once the types are fully resolved. This is necessary because
    /// context protocol method calls (like `Search.execute()`) may return types
    /// with unresolved type variables that will be unified later.
    fn check_from_implementation(&mut self, source: &Type, target: &Type) -> bool {
        // If types are identical, conversion is trivial
        if source == target {
            return true;
        }

        // Handle type variables specially:
        // If either type is a type variable, we need to be permissive during inference.
        // The actual conversion will be checked once types are fully resolved.
        match (source, target) {
            // Same type variable - trivially convertible
            (Type::Var(v1), Type::Var(v2)) if v1 == v2 => return true,

            // Different type variables - try to unify them
            // This handles the case where τ12 and τ8 should represent the same error type
            (Type::Var(_), Type::Var(_)) => {
                // Try to unify the two type variables
                // If they can be unified, they represent compatible error types
                if self.unifier.unify(source, target, Span::default()).is_ok() {
                    return true;
                }
                // If unification fails (e.g., occurs check), they're different types
                // but we still allow it during inference - the error will surface later
                return true;
            }

            // One is a type variable, the other is concrete
            // Allow this during inference - the type variable will be resolved later
            (Type::Var(_), _) | (_, Type::Var(_)) => return true,

            _ => {}
        }

        // Check if target has a From<source> implementation in the protocol checker
        //

        // Strategy:
        // 1. Get all protocol implementations for the target type
        // 2. Filter for "From" protocol implementations
        // 3. Check if any have protocol_args matching the source type
        // 4. Handle generic implementations with type variable matching

        // Get all implementations for the target type
        let protocol_checker_guard = self.protocol_checker.read();
        let target_impls = protocol_checker_guard.get_implementations(target);

        for impl_ in target_impls {
            // Check if this is a From protocol implementation
            let protocol_name = impl_
                .protocol
                .as_ident()
                .map(|i| i.as_str().to_string())
                .unwrap_or_default();

            if protocol_name == "From" {
                // From<T> has one type argument - the source type
                if let Some(from_source) = impl_.protocol_args.first() {
                    // Check if the From source matches our source type
                    if self.types_match_for_from(source, from_source) {
                        return true;
                    }
                }
            }
        }
        drop(protocol_checker_guard);

        // Also check if target type itself has From implementation through type parameters
        // This handles cases like generic error wrappers
        if self.check_generic_from_implementation(source, target) {
            return true;
        }

        // Check well-known automatic conversions
        // Standard library provides automatic conversions for common patterns
        if self.has_standard_from_conversion(source, target) {
            return true;
        }

        false
    }

    /// Check if two types match for From protocol lookup
    ///

    /// This handles:
    /// - Direct equality
    /// - Type variable matching
    /// - Generic type parameter substitution
    fn types_match_for_from(&self, expected: &Type, actual: &Type) -> bool {
        // Direct equality
        if expected == actual {
            return true;
        }

        // Handle type variables on either side
        match (expected, actual) {
            // If the From source is a type parameter, it matches anything
            // This handles From<T> implementations
            (_, Type::Var(_)) => true,

            // If our source is a type variable, allow it (will be checked later)
            (Type::Var(_), _) => true,

            // Named types - check structure by comparing paths
            (Type::Named { path: p1, args: a1 }, Type::Named { path: p2, args: a2 }) => {
                // Compare the paths (type names)
                let name1 = p1.as_ident().map(|i| i.as_str()).unwrap_or("");
                let name2 = p2.as_ident().map(|i| i.as_str()).unwrap_or("");

                if name1 == name2 && a1.len() == a2.len() {
                    // Check all type arguments match
                    a1.iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| self.types_match_for_from(t1, t2))
                } else {
                    false
                }
            }

            // Try unification for complex cases
            _ => {
                // For complex cases, we try structural comparison
                // Full unification would require cloning which Unifier doesn't support
                false
            }
        }
    }

    /// Check for generic From implementations through type parameters
    ///

    /// Handles patterns like:
    /// ```verum
    /// implement<E: Error> From<E> for AppError { ... }
    /// ```
    fn check_generic_from_implementation(&self, source: &Type, target: &Type) -> bool {
        // Get all generic From implementations
        let protocol_checker_guard = self.protocol_checker.read();
        let target_impls = protocol_checker_guard.get_implementations(target);

        for impl_ in target_impls {
            let protocol_name = impl_
                .protocol
                .as_ident()
                .map(|i| i.as_str().to_string())
                .unwrap_or_default();

            if protocol_name != "From" {
                continue;
            }

            // Check where clauses for bounded type parameters
            for where_clause in &impl_.where_clauses {
                // If the source type satisfies the where clause bounds,
                // the generic From implementation applies
                if self.satisfies_where_clause_impl(source, where_clause) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a type satisfies a where clause constraint
    fn satisfies_where_clause_impl(
        &self,
        ty: &Type,
        clause: &crate::protocol::WhereClause,
    ) -> bool {
        // Extract the bound from the where clause
        // WhereClause binds a type parameter to protocol bounds
        // e.g., "E: Error" means E must implement Error

        for bound in &clause.bounds {
            // ProtocolBound has a `protocol` field which is a Path
            let protocol_name = bound
                .protocol
                .as_ident()
                .map(|i: &verum_ast::Ident| i.as_str().to_string())
                .unwrap_or_default();

            // Check if the type implements the required protocol
            if !self
                .protocol_checker
                .read()
                .implements_protocol(ty, &protocol_name)
            {
                return false;
            }
        }

        true
    }

    /// Check for standard library automatic From conversions
    ///

    /// The standard library provides From implementations for common patterns:
    /// - From<&str> for Text
    /// - From<Int> for Float
    /// - From<T> for T (identity)
    /// - From<T> for Maybe<T>
    /// - From<E> for Result<T, E>
    fn has_standard_from_conversion(&self, source: &Type, target: &Type) -> bool {
        match (source, target) {
            // Identity conversion: From<T> for T
            (s, t) if s == t => true,

            // Int to Float
            (Type::Int, Type::Float) => true,

            // Char to Int (for ASCII/codepoint)
            (Type::Char, Type::Int) => true,

            // T to Wrapper<T> (single-arg generic types like Maybe<T>, List<T>, etc.)
            (inner, Type::Named { args, .. }) if args.len() == 1 => {
                self.types_match_for_from(inner, &args[0])
            }

            // &str to Text - check through Named path
            (Type::Named { path, .. }, Type::Text) => {
                let src_name = path.as_ident().map(|i| i.as_str()).unwrap_or("");
                src_name == "str"
            }

            // Subtype error conversions
            // If both are error types, check for standard error hierarchy
            _ => false,
        }
    }

    /// Instantiate a protocol method type for a specific receiver
    ///

    /// When looking up a method from a protocol, the method signature uses `Self`
    /// as a placeholder for the implementing type. This method substitutes the
    /// actual receiver type for `Self` in the method signature.
    ///

    /// # Parameters
    /// - `method_ty`: The method type from the protocol definition
    /// - `receiver_ty`: The actual receiver type to substitute for Self
    ///

    /// # Returns
    /// The method type with Self replaced by the receiver type
    pub(super) fn instantiate_method_for_receiver(&self, method_ty: &Type, receiver_ty: &Type) -> Type {
        self.substitute_self_type(method_ty, receiver_ty)
    }

    /// Compute how many of the method scheme's `ordered_fresh_vars` should be bound
    /// positionally against the receiver's type arguments.
    ///

    /// `impl_var_count` is the number of impl-level type params in the scheme (set
    /// at registration time — all method-scheme sites in
    /// `register_impl_block_inner` do this). Method-level params (the method's own
    /// `<U>`) live *after* the impl-level ones in `ordered_fresh_vars` and must
    /// NEVER be bound from receiver args. See #57: `Range<Int>.chain(b)` has
    /// impl_var_count=0 (impl takes no generic params) with a single method-level
    /// `U`; binding U := Int would pin U before the argument `b: Range<Int>` is
    /// checked.
    #[inline]
    pub(super) fn resolve_bind_limit(
        impl_var_count: usize,
        fresh_vars_len: usize,
        receiver_args_len: usize,
    ) -> usize {
        impl_var_count.min(fresh_vars_len).min(receiver_args_len)
    }

    /// Instantiate a method's type parameters and optionally unify with explicit type arguments.
    ///

    /// For generic methods like `fn collect<U>(&self) -> List<U>`, when called as
    /// `items.collect<Int>()`, this:
    /// 1. Creates fresh TypeVars for the method's type_params (e.g., U -> TypeVar(123))
    /// 2. Substitutes them into params and return_type
    /// 3. If explicit type_args are provided, unifies them with the fresh vars
    ///

    /// Refinement types: predicates on base types verified at compile-time (proof mode) or runtime, with SMT solver integration — Generic Methods
    pub(super) fn instantiate_method_type_params(
        &mut self,
        method_ty: Type,
        type_args: &List<verum_ast::ty::GenericArg>,
        span: Span,
    ) -> Result<Type> {
        // Only process if the method is a function type with type parameters
        if let Type::Function {
            params,
            return_type,
            contexts,
            properties,
            type_params,
        } = &method_ty
        {
            if type_params.is_empty() {
                // No type parameters to instantiate - return as-is
                // But if type_args were provided, that's an error
                if !type_args.is_empty() {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Method does not accept type arguments, but {} were provided",
                        type_args.len()
                    ))));
                }
                return Ok(method_ty);
            }

            // Create fresh TypeVars for each type parameter
            let mut param_subst: indexmap::IndexMap<verum_common::Text, Type> =
                indexmap::IndexMap::new();
            let mut fresh_vars: Vec<TypeVar> = Vec::new();
            // Name-based substitution handles `Type::Generic { name, args: [] }`
            // (bare-type-parameter AST form). The method's signature was
            // originally registered with `Type::Var(fresh)` aliased in the
            // context environment under the parameter's name. Collect those
            // original vars too so we can rewrite them to fresh vars per
            // call-site — otherwise successive invocations of the same
            // generic method would keep mapping to the same TypeVar and
            // effectively "pin" U to whatever Self.Item resolved to on the
            // first call (e.g., `fn chain<U: Iterator<Item = Self.Item>>`
            // pinned U := Int through the bound rather than inferring from
            // the argument).
            let mut original_var_subst: Map<TypeVar, Type> = Map::new();

            for type_param in type_params.iter() {
                let fresh_var = TypeVar::fresh();
                fresh_vars.push(fresh_var);
                param_subst.insert(type_param.name.clone(), Type::Var(fresh_var));
                // If the context already has a Var aliased under this name
                // (from method-signature registration), map it to the fresh
                // var. A TypeScheme carries no Vars directly, so we look up
                // the stored Type and extract any Var at the root.
                if let Some(scheme) = self.ctx.env.lookup(&type_param.name)
                    && let Type::Var(orig_var) = scheme.instantiate()
                {
                    original_var_subst.insert(orig_var, Type::Var(fresh_var));
                }
            }

            // Substitute type parameters in params and return_type
            let substituted_params: List<Type> = params
                .iter()
                .map(|p| {
                    let after_name = self.substitute_type_params(p, &param_subst);
                    self.substitute_type_vars(&after_name, &original_var_subst)
                })
                .collect();
            let substituted_return = {
                let after_name = self.substitute_type_params(return_type, &param_subst);
                self.substitute_type_vars(&after_name, &original_var_subst)
            };

            // If explicit type_args are provided, unify them with fresh vars
            if !type_args.is_empty() {
                if type_args.len() > fresh_vars.len() {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Method expects {} type argument{}, but {} were provided",
                        fresh_vars.len(),
                        if fresh_vars.len() == 1 { "" } else { "s" },
                        type_args.len()
                    ))));
                }

                for (i, type_arg) in type_args.iter().enumerate() {
                    if let verum_ast::ty::GenericArg::Type(ast_ty) = type_arg {
                        let provided_ty = self.ast_to_type(ast_ty)?;
                        // Unify the fresh var with the provided type
                        self.unifier
                            .unify(&Type::Var(fresh_vars[i]), &provided_ty, span)?;
                    }
                }
            }

            // Return the substituted function type (without type_params since they're now instantiated)
            Ok(Type::Function {
                params: substituted_params,
                return_type: Box::new(substituted_return),
                contexts: contexts.clone(),
                properties: properties.clone(),
                type_params: List::new(), // Type params are now resolved
            })
        } else {
            // Not a function type - just return as-is
            if !type_args.is_empty() {
                return Err(TypeError::Other(verum_common::Text::from(
                    "Type arguments provided to non-function method type",
                )));
            }
            Ok(method_ty)
        }
    }

    /// Freshen method-level type parameters in a method type returned by lookup_protocol_method.
    ///

    /// Protocol method types store the original TypeVars created during protocol registration.
    /// These TypeVars are shared across ALL call sites, which causes incorrect unification
    /// when the same method is called multiple times. This function creates fresh TypeVars
    /// for each method-level type parameter and substitutes them in the method type.
    ///

    /// For `fn collect<C: FromIterator<Self.Item>>() -> C`:
    /// - `type_param_names` = ["C"]
    /// - The original TypeVar for C in the method type gets replaced with a fresh TypeVar
    /// - If explicit `type_args` are provided (e.g., `collect<List<Int>>()`), the fresh vars
    ///  are unified with the provided types
    pub(super) fn freshen_method_type_params(
        &mut self,
        method_ty: Type,
        type_param_names: &[verum_common::Text],
        type_args: &List<verum_ast::ty::GenericArg>,
        span: Span,
    ) -> Result<Type> {
        if type_param_names.is_empty() {
            return Ok(method_ty);
        }

        // Find TypeVars in the method type that correspond to method-level type params.
        // These are TypeVars whose names (as Named types or TypeVars) match the param names.
        // We use find_unresolved_type_param_names + replace_named_with_var for Named params,
        // and direct TypeVar substitution for TypeVar params.
        let mut result_ty = method_ty;
        let mut fresh_vars: Vec<TypeVar> = Vec::new();

        for param_name in type_param_names {
            let fresh_var = TypeVar::fresh();
            fresh_vars.push(fresh_var);
            // Replace Named("C") with Var(fresh) — handles case where C was stored as Named
            result_ty = Self::replace_named_with_var(&result_ty, param_name.as_str(), fresh_var);
        }

        // Also handle the case where the type params are stored as TypeVars (from protocol registration).
        // Walk the method type to find TypeVars that aren't resolved yet, and if their count matches
        // the method type param count, replace them with fresh vars.
        // This handles the case where C is Type::Var(old_var) from protocol registration.
        if let Type::Function {
            params,
            return_type,
            contexts,
            properties,
            type_params,
        } = &result_ty
        {
            // Check if return type or params contain unresolved TypeVars that might be method-level params
            let mut old_to_fresh: Map<crate::TypeVar, crate::TypeVar> = Map::new();
            self.collect_stale_method_type_vars(
                &result_ty,
                type_param_names,
                &fresh_vars,
                &mut old_to_fresh,
            );

            if !old_to_fresh.is_empty() {
                let new_params: List<Type> = params
                    .iter()
                    .map(|p| self.substitute_type_vars_fresh(p, &old_to_fresh))
                    .collect();
                let new_return =
                    Box::new(self.substitute_type_vars_fresh(return_type, &old_to_fresh));
                result_ty = Type::Function {
                    params: new_params,
                    return_type: new_return,
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                };
            }
        }

        // If explicit type_args are provided, unify them with fresh vars
        if !type_args.is_empty() && type_args.len() <= fresh_vars.len() {
            for (i, type_arg) in type_args.iter().enumerate() {
                if let verum_ast::ty::GenericArg::Type(ast_ty) = type_arg {
                    let provided_ty = self.ast_to_type(ast_ty)?;
                    self.unifier
                        .unify(&Type::Var(fresh_vars[i]), &provided_ty, span)?;
                }
            }
        }

        Ok(result_ty)
    }

    /// Collect stale TypeVars in a method type that correspond to method-level type params.
    /// These are TypeVars that were created during protocol registration and need to be
    /// replaced with fresh ones at each call site.
    fn collect_stale_method_type_vars(
        &self,
        ty: &Type,
        type_param_names: &[verum_common::Text],
        fresh_vars: &[crate::TypeVar],
        old_to_fresh: &mut Map<crate::TypeVar, crate::TypeVar>,
    ) {
        // The protocol registration creates TypeVars for method-level params and they end up
        // in the method type. We need to identify them. The heuristic: find unresolved TypeVars
        // in the method type that aren't bound in the unifier. Match them positionally with
        // the type_param_names.
        let mut unresolved_vars: Vec<crate::TypeVar> = Vec::new();
        self.collect_unresolved_type_vars(ty, &mut unresolved_vars);
        unresolved_vars.sort_by_key(|v| v.id());
        unresolved_vars.dedup();

        // Match unresolved vars to type param names (positionally)
        // Only replace if the count matches or we have at least as many unresolved vars
        let count = type_param_names
            .len()
            .min(fresh_vars.len())
            .min(unresolved_vars.len());
        for i in 0..count {
            old_to_fresh.insert(unresolved_vars[i], fresh_vars[i]);
        }
    }

    /// Collect all unresolved TypeVars in a type (not yet bound in the unifier).
    fn collect_unresolved_type_vars(&self, ty: &Type, result: &mut Vec<crate::TypeVar>) {
        match ty {
            Type::Var(tv) => {
                // Check if this TypeVar is unresolved (not bound in unifier)
                let resolved = self.unifier.apply(ty);
                if matches!(resolved, Type::Var(resolved_tv) if resolved_tv == *tv) {
                    result.push(*tv);
                }
            }
            Type::Named { args, .. } | Type::Generic { args, .. } => {
                for a in args {
                    self.collect_unresolved_type_vars(a, result);
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    self.collect_unresolved_type_vars(p, result);
                }
                self.collect_unresolved_type_vars(return_type, result);
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                self.collect_unresolved_type_vars(inner, result);
            }
            Type::Tuple(elems) => {
                for e in elems {
                    self.collect_unresolved_type_vars(e, result);
                }
            }
            Type::Array { element, .. } | Type::Slice { element } => {
                self.collect_unresolved_type_vars(element, result);
            }
            _ => {}
        }
    }

    /// Substitute TypeVars in a type with fresh ones, using the old_to_fresh mapping.
    fn substitute_type_vars_fresh(
        &self,
        ty: &Type,
        old_to_fresh: &Map<crate::TypeVar, crate::TypeVar>,
    ) -> Type {
        match ty {
            Type::Var(tv) => {
                if let Some(fresh) = old_to_fresh.get(tv) {
                    Type::Var(*fresh)
                } else {
                    ty.clone()
                }
            }
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| self.substitute_type_vars_fresh(a, old_to_fresh))
                    .collect(),
            },
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.substitute_type_vars_fresh(a, old_to_fresh))
                    .collect(),
            },
            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.substitute_type_vars_fresh(p, old_to_fresh))
                    .collect(),
                return_type: Box::new(self.substitute_type_vars_fresh(return_type, old_to_fresh)),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.substitute_type_vars_fresh(inner, old_to_fresh)),
                mutable: *mutable,
            },
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|e| self.substitute_type_vars_fresh(e, old_to_fresh))
                    .collect(),
            ),
            _ => ty.clone(),
        }
    }

    /// Build a type parameter substitution map from impl's for_type and the concrete receiver type.
    ///

    /// For a generic impl like `implement<T> Iterator for MaybeIter<T>` called on `MaybeIter<&Int>`,
    /// this builds a map `{"T134": &Int}` where 134 is the TypeVar ID for T in the impl.
    ///

    /// This enables proper substitution of protocol method return types:
    /// - Iterator::next() returns `Maybe<T>` where T is TypeVar(134)
    /// - After substitution, it returns `Maybe<&Int>`
    pub(super) fn build_impl_type_subst(
        impl_for_type: &Type,
        receiver_type: &Type,
    ) -> indexmap::IndexMap<verum_common::Text, Type> {
        let mut subst = indexmap::IndexMap::new();

        // Extract type arguments from both types and match them positionally
        let impl_args = match impl_for_type {
            Type::Named { args, .. } => args.as_slice(),
            Type::Generic { args, .. } => args.as_slice(),
            _ => return subst,
        };

        let receiver_args = match receiver_type {
            Type::Named { args, .. } => args.as_slice(),
            Type::Generic { args, .. } => args.as_slice(),
            _ => return subst,
        };

        // Match type arguments positionally
        for (impl_arg, recv_arg) in impl_args.iter().zip(receiver_args.iter()) {
            match impl_arg {
                // TypeVar in impl maps to concrete type from receiver
                Type::Var(tv) => {
                    let var_name: verum_common::Text = format!("T{}", tv.id()).into();
                    subst.insert(var_name, recv_arg.clone());
                }
                // Named type that might be a type parameter
                Type::Named { path, args } if args.is_empty() => {
                    if let Some(ident) = path.as_ident() {
                        let name: verum_common::Text = ident.name.clone();
                        // Type parameters are typically single uppercase letters or CamelCase
                        if Self::looks_like_type_param_name(&name) {
                            subst.insert(name, recv_arg.clone());
                        }
                    }
                }
                // Recursively match nested generic types
                _ => {
                    // For nested generics, we could recurse, but for now the main case
                    // is the top-level type arguments which are the common case
                }
            }
        }

        subst
    }

    /// Check if a type contains a Named type with the given simple name (single-segment, no args).
    /// Used to detect method-level type parameters stored as Named types in protocol impls.
    fn type_contains_named(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    if ident.name.as_str() == name {
                        return true;
                    }
                }
                false
            }
            Type::Named { args, .. } | Type::Generic { args, .. } => {
                args.iter().any(|a| Self::type_contains_named(a, name))
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| Self::type_contains_named(p, name))
                    || Self::type_contains_named(return_type, name)
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => Self::type_contains_named(inner, name),
            Type::Array { element, .. } | Type::Slice { element } => {
                Self::type_contains_named(element, name)
            }
            Type::Tuple(elems) => elems.iter().any(|e| Self::type_contains_named(e, name)),
            _ => false,
        }
    }

    /// Replace all occurrences of a simple Named type with a TypeVar.
    /// Used to instantiate method-level type parameters to fresh type variables.
    pub(super) fn replace_named_with_var(ty: &Type, name: &str, var: TypeVar) -> Type {
        match ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    if ident.name.as_str() == name {
                        return Type::Var(var);
                    }
                }
                ty.clone()
            }
            Type::Named { path, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|a| Self::replace_named_with_var(a, name, var))
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: new_args,
                }
            }
            Type::Generic { name: gname, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|a| Self::replace_named_with_var(a, name, var))
                    .collect();
                Type::Generic {
                    name: gname.clone(),
                    args: new_args,
                }
            }
            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => {
                let new_params: List<Type> = params
                    .iter()
                    .map(|p| Self::replace_named_with_var(p, name, var))
                    .collect();
                let new_return = Box::new(Self::replace_named_with_var(return_type, name, var));
                Type::Function {
                    params: new_params,
                    return_type: new_return,
                    type_params: type_params.clone(),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                }
            }
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(Self::replace_named_with_var(inner, name, var)),
                mutable: *mutable,
            },
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|e| Self::replace_named_with_var(e, name, var))
                    .collect(),
            ),
            _ => ty.clone(),
        }
    }

    /// Find Named types in a type that look like unresolved method-level type parameters.
    /// These are single-segment Named types with no args, whose name looks like a type param
    /// (uppercase letter, short name) and wasn't already substituted by the impl param_subst.
    pub(super) fn find_unresolved_type_param_names(
        &self,
        ty: &Type,
        param_subst: &indexmap::IndexMap<verum_common::Text, Type>,
    ) -> Vec<String> {
        let mut result = Vec::new();
        self.collect_unresolved_type_param_names(ty, param_subst, &mut result);
        result.sort();
        result.dedup();
        result
    }

    fn collect_unresolved_type_param_names(
        &self,
        ty: &Type,
        param_subst: &indexmap::IndexMap<verum_common::Text, Type>,
        result: &mut Vec<String>,
    ) {
        match ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.name.as_str();
                    let name_text: verum_common::Text = name.into();
                    // Check: looks like a type param AND wasn't already substituted
                    if Self::looks_like_type_param_name(&name_text)
                        && !param_subst.contains_key(&name_text)
                    {
                        // Skip names that resolve to actual types in the environment.
                        // This replaces the old hardcoded is_known_type_name() list.
                        if self.ctx.lookup_type(name).is_none() {
                            result.push(name.to_string());
                        }
                    }
                }
            }
            Type::Named { args, .. } | Type::Generic { args, .. } => {
                for a in args {
                    self.collect_unresolved_type_param_names(a, param_subst, result);
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    self.collect_unresolved_type_param_names(p, param_subst, result);
                }
                self.collect_unresolved_type_param_names(return_type, param_subst, result);
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                self.collect_unresolved_type_param_names(inner, param_subst, result);
            }
            Type::Array { element, .. } | Type::Slice { element } => {
                self.collect_unresolved_type_param_names(element, param_subst, result);
            }
            Type::Tuple(elems) => {
                for e in elems {
                    self.collect_unresolved_type_param_names(e, param_subst, result);
                }
            }
            _ => {}
        }
    }

    /// Check if a name looks like a type parameter name.
    /// Only matches single uppercase letters (T, U, V, I, K, F, B, C, E, P, R, S, etc.)
    /// or two-letter type params (St). This avoids false positives with user-defined
    /// types like Point, Color, Entry, etc.
    /// TRANSITIONAL: Returns (min, max) range for integer type names.
    /// Long-term: derive from protocol-based bit-width metadata.
    pub(super) fn integer_type_range(name: &str) -> Option<(i128, i128)> {
        match name {
            "Byte" | "u8" | "UInt8" => Some((0, 255)),
            "Int8" | "i8" => Some((-128, 127)),
            "Int16" | "i16" => Some((-32768, 32767)),
            "UInt16" | "u16" => Some((0, 65535)),
            "Int32" | "i32" => Some((-2147483648, 2147483647)),
            "UInt32" | "u32" => Some((0, 4294967295)),
            "Int64" | "i64" | "IntSize" | "ISize" | "isize" => {
                Some((i64::MIN as i128, i64::MAX as i128))
            }
            "UInt64" | "u64" | "UIntSize" | "USize" | "usize" => Some((0, u64::MAX as i128)),
            "Int128" | "i128" => Some((i128::MIN, i128::MAX)),
            "UInt128" | "u128" => Some((0, i128::MAX)),
            _ => None,
        }
    }

    fn looks_like_type_param_name(name: &verum_common::Text) -> bool {
        let s = name.as_str();
        match s.len() {
            1 => s.chars().next().is_some_and(|c| c.is_uppercase()),
            2 => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(first), Some(second)) => first.is_uppercase() && second.is_lowercase(),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Substitute Self type with a concrete type
    ///

    /// Recursively traverses the type structure and replaces any occurrence
    /// of Self (represented as Type::Named with path "Self") with the given type.
    pub(super) fn substitute_self_type(&self, ty: &Type, self_ty: &Type) -> Type {
        match ty {
            // Check for Self type
            Type::Named { path, args } => {
                // Check if this is the Self type
                if let Some(ident) = path.as_ident() {
                    // #[cfg(debug_assertions)]
                    // if ident.name.as_str() == "Self" {
                    //  eprintln!("[DEBUG substitute_self_type] Found Self, substituting with {:?}", self_ty);
                    // }
                    if ident.name.as_str() == "Self" {
                        return self_ty.clone();
                    }
                }
                // else {
                //  // Debug: why did as_ident fail?
                //  #[cfg(debug_assertions)]
                //  {
                //  let path_str = path.segments.iter().map(|s| match s {
                //  verum_ast::PathSegment::Name(id) => id.name.to_string(),
                //  verum_ast::PathSegment::SelfValue => "self".to_string(),
                //  verum_ast::PathSegment::Super => "super".to_string(),
                //  verum_ast::PathSegment::Cog => "cog".to_string(),
                //  verum_ast::PathSegment::Relative => ".".to_string(),
                //  }).collect::<Vec<_>>().join("::");
                //  if path_str == "Self" {
                //  eprintln!("[DEBUG substitute_self_type] path is 'Self' but as_ident() returned None! path={:?}", path);
                //  }
                //  }
                // }

                // Recursively substitute in type arguments
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_self_type(arg, self_ty))
                    .collect();

                Type::Named {
                    path: path.clone(),
                    args: substituted_args,
                }
            }

            // Function types - substitute in params and return type
            Type::Function {
                params,
                return_type,
                contexts,
                properties,
                type_params,
            } => {
                let substituted_params: List<Type> = params
                    .iter()
                    .map(|p| self.substitute_self_type(p, self_ty))
                    .collect();
                let substituted_return = self.substitute_self_type(return_type, self_ty);

                Type::Function {
                    params: substituted_params,
                    return_type: Box::new(substituted_return),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                }
            }

            // Reference types
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.substitute_self_type(inner, self_ty)),
                mutable: *mutable,
            },
            Type::CheckedReference { inner, mutable } => Type::CheckedReference {
                inner: Box::new(self.substitute_self_type(inner, self_ty)),
                mutable: *mutable,
            },
            Type::UnsafeReference { inner, mutable } => Type::UnsafeReference {
                inner: Box::new(self.substitute_self_type(inner, self_ty)),
                mutable: *mutable,
            },
            Type::Ownership { inner, mutable } => Type::Ownership {
                inner: Box::new(self.substitute_self_type(inner, self_ty)),
                mutable: *mutable,
            },

            // Generic types
            Type::Generic { name, args } => {
                // Check if this is the Self type
                if name.as_str() == "Self" {
                    return self_ty.clone();
                }

                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_self_type(arg, self_ty))
                    .collect();

                // CRITICAL FIX: If this is an associated type projection (e.g., ::Item),
                // try to resolve it now that Self has been substituted in the base type.
                // This handles cases like Self.Item where Self = SliceIter<Int> and
                // Item should resolve to &Int via the Iterator implementation.
                if name.starts_with("::") && !substituted_args.is_empty() {
                    let assoc_name = &name[2..]; // Strip "::" prefix
                    let base_ty = &substituted_args[0];

                    // Only try to resolve if the base type is concrete (no unresolved type vars)
                    if !self.has_unresolved_vars(base_ty) {
                        if let Some(resolved) =
                            self.try_resolve_associated_type_projection(base_ty, assoc_name)
                        {
                            return resolved;
                        }
                    }
                }

                Type::Generic {
                    name: name.clone(),
                    args: substituted_args,
                }
            }

            // Tuple types
            Type::Tuple(elements) => {
                let substituted: List<Type> = elements
                    .iter()
                    .map(|e| self.substitute_self_type(e, self_ty))
                    .collect();
                Type::Tuple(substituted)
            }

            // Array and slice types
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_self_type(element, self_ty)),
                size: *size,
            },
            Type::Slice { element } => Type::Slice {
                element: Box::new(self.substitute_self_type(element, self_ty)),
            },

            // Future type
            Type::Future { output } => Type::Future {
                output: Box::new(self.substitute_self_type(output, self_ty)),
            },

            // Variant types - substitute in each variant's payload type
            Type::Variant(variants) => {
                let substituted_variants: indexmap::IndexMap<verum_common::Text, Type> = variants
                    .iter()
                    .map(|(name, payload_ty)| {
                        (name.clone(), self.substitute_self_type(payload_ty, self_ty))
                    })
                    .collect();
                Type::Variant(substituted_variants)
            }

            // Record types - substitute in each field's type
            Type::Record(fields) => {
                let substituted_fields: indexmap::IndexMap<verum_common::Text, Type> = fields
                    .iter()
                    .map(|(name, field_ty)| {
                        (name.clone(), self.substitute_self_type(field_ty, self_ty))
                    })
                    .collect();
                Type::Record(substituted_fields)
            }

            // TypeApp - substitute in constructor and args, then try to resolve
            Type::TypeApp { constructor, args } => {
                let substituted_ctor = self.substitute_self_type(constructor, self_ty);
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|a| self.substitute_self_type(a, self_ty))
                    .collect();

                // If constructor is a resolved projection, try to apply GAT args
                match &substituted_ctor {
                    Type::Generic {
                        name,
                        args: ctor_args,
                    } if name.starts_with("::") && !ctor_args.is_empty() => {
                        let assoc_name = &name[2..];
                        let base_ty = &ctor_args[0];
                        if !self.has_unresolved_vars(base_ty) {
                            if let Some(resolved) =
                                self.try_resolve_associated_type_projection(base_ty, assoc_name)
                            {
                                // resolved is the associated type body (e.g., List<T>)
                                // substituted_args are the GAT type args (e.g., [Int])
                                // Substitute the GAT params into the resolved type
                                if let Type::Generic { name: rn, args: ra } = &resolved {
                                    if !substituted_args.is_empty() {
                                        let mut subst = indexmap::IndexMap::new();
                                        for (i, arg) in ra.iter().enumerate() {
                                            if let Type::Var(tv) = arg {
                                                if let Some(replacement) = substituted_args.get(i) {
                                                    subst.insert(
                                                        verum_common::Text::from(format!(
                                                            "T{}",
                                                            tv.id()
                                                        )),
                                                        replacement.clone(),
                                                    );
                                                }
                                            }
                                        }
                                        if !subst.is_empty() {
                                            return self.substitute_type_params(&resolved, &subst);
                                        }
                                        return Type::Generic {
                                            name: rn.clone(),
                                            args: substituted_args,
                                        };
                                    }
                                }
                                return resolved;
                            }
                        }
                    }
                    _ => {}
                }

                Type::TypeApp {
                    constructor: Box::new(substituted_ctor),
                    args: substituted_args,
                }
            }

            // Primitive and other types - no substitution needed
            _ => ty.clone(),
        }
    }

    /// Resolve associated type projections like `::Item<SliceIter<Int>>` using an implementation's
    /// associated_types map.
    ///

    /// When a protocol method uses `Self.Item` (represented as `Generic { name: "::Item", args: [Self] }`),
    /// after Self substitution we get `::Item<SliceIter<Int>>`. This function resolves such projections
    /// to the concrete type defined in the implementation (e.g., `type Item = &T` -> `&Int`).
    pub(super) fn resolve_associated_type_projections_with_impl(
        &self,
        ty: &Type,
        associated_types: &verum_common::Map<verum_common::Text, Type>,
        param_subst: &indexmap::IndexMap<verum_common::Text, Type>,
    ) -> Type {
        match ty {
            Type::Generic { name, args } => {
                // Check if this is an associated type projection (::AssocName)
                if name.starts_with("::") {
                    let assoc_name: verum_common::Text = name[2..].into(); // Strip "::" prefix
                    // #[cfg(debug_assertions)]
                    // if assoc_name.as_str() == "Item" {
                    //  eprintln!("[DEBUG resolve_assoc_proj] Found ::Item, looking up in associated_types");
                    //  eprintln!(" associated_types keys: {:?}", associated_types.keys().collect::<Vec<_>>());
                    // }
                    // Look up the associated type in the implementation
                    if let Some(assoc_ty) = associated_types.get(&assoc_name) {
                        // #[cfg(debug_assertions)]
                        // if assoc_name.as_str() == "Item" {
                        //  eprintln!(" Found! assoc_ty={:?}", assoc_ty);
                        // }
                        // Apply param_subst to the associated type value (e.g., &T -> &Int)
                        return self.substitute_type_params(assoc_ty, param_subst);
                    }
                }
                // Recursively resolve in args
                let resolved_args: List<Type> = args
                    .iter()
                    .map(|arg| {
                        self.resolve_associated_type_projections_with_impl(
                            arg,
                            associated_types,
                            param_subst,
                        )
                    })
                    .collect();
                Type::Generic {
                    name: name.clone(),
                    args: resolved_args,
                }
            }

            // Recursively resolve in compound types
            Type::Function {
                params,
                return_type,
                contexts,
                properties,
                type_params,
            } => {
                let resolved_params: List<Type> = params
                    .iter()
                    .map(|p| {
                        self.resolve_associated_type_projections_with_impl(
                            p,
                            associated_types,
                            param_subst,
                        )
                    })
                    .collect();
                Type::Function {
                    params: resolved_params,
                    return_type: Box::new(self.resolve_associated_type_projections_with_impl(
                        return_type,
                        associated_types,
                        param_subst,
                    )),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                }
            }

            Type::Variant(variants) => {
                let resolved_variants: indexmap::IndexMap<verum_common::Text, Type> = variants
                    .iter()
                    .map(|(name, payload_ty)| {
                        (
                            name.clone(),
                            self.resolve_associated_type_projections_with_impl(
                                payload_ty,
                                associated_types,
                                param_subst,
                            ),
                        )
                    })
                    .collect();
                Type::Variant(resolved_variants)
            }

            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.resolve_associated_type_projections_with_impl(
                    inner,
                    associated_types,
                    param_subst,
                )),
            },

            Type::Tuple(elements) => {
                let resolved: List<Type> = elements
                    .iter()
                    .map(|e| {
                        self.resolve_associated_type_projections_with_impl(
                            e,
                            associated_types,
                            param_subst,
                        )
                    })
                    .collect();
                Type::Tuple(resolved)
            }

            Type::Named { path, args } => {
                let resolved_args: List<Type> = args
                    .iter()
                    .map(|arg| {
                        self.resolve_associated_type_projections_with_impl(
                            arg,
                            associated_types,
                            param_subst,
                        )
                    })
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: resolved_args,
                }
            }

            // TypeApp - resolve in constructor and args, then try GAT resolution
            Type::TypeApp { constructor, args } => {
                let resolved_ctor = self.resolve_associated_type_projections_with_impl(
                    constructor,
                    associated_types,
                    param_subst,
                );
                let resolved_args: List<Type> = args
                    .iter()
                    .map(|a| {
                        self.resolve_associated_type_projections_with_impl(
                            a,
                            associated_types,
                            param_subst,
                        )
                    })
                    .collect();

                // If constructor resolved from ::AssocName to concrete type, apply GAT args
                match &resolved_ctor {
                    Type::Generic { name, args: ga }
                        if !name.starts_with("::") && !resolved_args.is_empty() =>
                    {
                        // Constructor is now concrete (e.g., List<T>) — apply GAT params
                        Type::Generic {
                            name: name.clone(),
                            args: resolved_args,
                        }
                    }
                    Type::Named { path, .. } if !resolved_args.is_empty() => Type::Named {
                        path: path.clone(),
                        args: resolved_args,
                    },
                    _ => Type::TypeApp {
                        constructor: Box::new(resolved_ctor),
                        args: resolved_args,
                    },
                }
            }

            // Other types - no projections to resolve
            _ => ty.clone(),
        }
    }

    /// Check if a type contains unresolved associated type projections (::Item, etc.)
    pub(super) fn contains_unresolved_projection(&self, ty: &Type) -> bool {
        match ty {
            Type::Generic { name, args } => {
                // Check if this is an unresolved projection
                if name.starts_with("::") {
                    return true;
                }
                // Recursively check args
                args.iter()
                    .any(|arg| self.contains_unresolved_projection(arg))
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params
                    .iter()
                    .any(|p| self.contains_unresolved_projection(p))
                    || self.contains_unresolved_projection(return_type)
            }
            Type::Variant(variants) => variants
                .values()
                .any(|v| self.contains_unresolved_projection(v)),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. } => self.contains_unresolved_projection(inner),
            Type::Tuple(elements) => elements
                .iter()
                .any(|e| self.contains_unresolved_projection(e)),
            Type::Named { args, .. } => args
                .iter()
                .any(|arg| self.contains_unresolved_projection(arg)),
            _ => false,
        }
    }

    /// Substitute Self.F<T> patterns with F<T> in HKT method return types
    ///

    /// When calling a method on an HKT type like `F<A>.map(f)`, the protocol method
    /// signature has return type `Self.F<B>`. This needs to be substituted with
    /// the concrete type constructor application `F<B>`.
    ///

    /// # Parameters
    ///

    /// - `ty`: The type to substitute in (e.g., the return type of the protocol method)
    /// - `hkt_name`: The name of the HKT parameter (e.g., "F")
    /// - `constructor`: The actual type constructor to substitute
    ///

    /// Find a method in a protocol hierarchy, searching the protocol itself and all super_protocols.
    /// Returns the ProtocolMethod if found in this protocol or any of its ancestors.
    ///

    /// This enables protocol inheritance: when M<_>: Monad, calling `.map()` should find the
    /// method in Functor (which Monad extends through Applicative).
    pub(super) fn find_method_in_protocol_hierarchy(
        &self,
        protocol_name: &Text,
        method_name: &Text,
    ) -> Option<crate::protocol::ProtocolMethod> {
        // Get the protocol definition
        let protocol_checker_guard = self.protocol_checker.read();
        let protocol_opt = protocol_checker_guard.get_protocol(protocol_name);
        if let Maybe::Some(protocol) = protocol_opt {
            // First, check if this protocol directly has the method
            for (_, protocol_method) in &protocol.methods {
                if &protocol_method.name == method_name {
                    return Some(protocol_method.clone());
                }
            }

            // If not found, search in super_protocols recursively
            for super_bound in &protocol.super_protocols {
                if let Some(super_ident) = super_bound.protocol.as_ident() {
                    let super_name: Text = super_ident.name.clone();
                    if let Some(found) =
                        self.find_method_in_protocol_hierarchy(&super_name, method_name)
                    {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// # Examples
    ///

    /// ```ignore
    /// // For: fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>
    /// // With F = ListConstructor, this substitutes Self.F<B> -> List<B>
    /// ```
    pub(super) fn substitute_self_hkt_in_type(&self, ty: &Type, hkt_name: &Text, constructor: &Type) -> Type {
        match ty {
            // Handle Qualified types like Self.F<B>
            Type::Named { path, args } => {
                // Check for patterns like Self.F or just F
                let segments: Vec<&verum_ast::ty::PathSegment> = path.segments.iter().collect();

                // Pattern: Self.F<args> where F matches hkt_name
                // Self in type context is represented as Name("Self"), not SelfValue
                if segments.len() == 2 {
                    if let (
                        verum_ast::ty::PathSegment::Name(self_ident),
                        verum_ast::ty::PathSegment::Name(name_ident),
                    ) = (&segments[0], &segments[1])
                    {
                        if self_ident.name.as_str() == "Self"
                            && name_ident.name.as_str() == hkt_name.as_str()
                        {
                            // This is Self.F<args> - substitute with constructor<args>
                            let substituted_args: List<Type> = args
                                .iter()
                                .map(|arg| {
                                    self.substitute_self_hkt_in_type(arg, hkt_name, constructor)
                                })
                                .collect();

                            return Type::TypeApp {
                                constructor: Box::new(constructor.clone()),
                                args: substituted_args,
                            };
                        }
                    }
                }

                // Not a Self.F pattern - recursively substitute in args
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_self_hkt_in_type(arg, hkt_name, constructor))
                    .collect();

                Type::Named {
                    path: path.clone(),
                    args: substituted_args,
                }
            }

            // Handle TypeApp - recursively substitute
            Type::TypeApp {
                constructor: inner_ctor,
                args,
            } => {
                let substituted_ctor =
                    self.substitute_self_hkt_in_type(inner_ctor, hkt_name, constructor);
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_self_hkt_in_type(arg, hkt_name, constructor))
                    .collect();

                Type::TypeApp {
                    constructor: Box::new(substituted_ctor),
                    args: substituted_args,
                }
            }

            // Function types - substitute in params and return type
            Type::Function {
                params,
                return_type,
                contexts,
                properties,
                type_params,
            } => {
                let substituted_params: List<Type> = params
                    .iter()
                    .map(|p| self.substitute_self_hkt_in_type(p, hkt_name, constructor))
                    .collect();
                let substituted_return =
                    self.substitute_self_hkt_in_type(return_type, hkt_name, constructor);

                Type::Function {
                    params: substituted_params,
                    return_type: Box::new(substituted_return),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                }
            }

            // Reference types
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.substitute_self_hkt_in_type(inner, hkt_name, constructor)),
                mutable: *mutable,
            },

            // Generic types - check if this is the HKT name
            Type::Generic { name, args } => {
                // Check if this is Self or the HKT name directly
                // Protocol HKT names are stored with :: prefix (e.g., "::F")
                // When a method is inherited from a super_protocol, the HKT name in the method
                // (like "::F" from Functor) needs to be substituted with the caller's HKT (like "M")
                let name_str = name.as_str();
                let hkt_str = hkt_name.as_str();
                let prefixed_hkt = format!("::{}", hkt_str);
                // Match: "Self", the exact hkt_name, "::hkt_name", or any "::X" (protocol HKT convention)
                let is_protocol_hkt = name_str.starts_with("::");
                if name_str == "Self"
                    || name_str == hkt_str
                    || name_str == prefixed_hkt
                    || is_protocol_hkt
                {
                    // For associated type projections like ::F, the args contain the base type (Self)
                    // When this Generic is used as a TypeApp constructor (Self.F<B>), we should return
                    // just the constructor, as the outer TypeApp will handle the type args
                    // For standalone Self.F (no outer TypeApp), return the constructor
                    return constructor.clone();
                }

                // Otherwise recursively substitute in args
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_self_hkt_in_type(arg, hkt_name, constructor))
                    .collect();

                Type::Generic {
                    name: name.clone(),
                    args: substituted_args,
                }
            }

            // Tuple types
            Type::Tuple(elements) => {
                let substituted: List<Type> = elements
                    .iter()
                    .map(|e| self.substitute_self_hkt_in_type(e, hkt_name, constructor))
                    .collect();
                Type::Tuple(substituted)
            }

            // Array types
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_self_hkt_in_type(element, hkt_name, constructor)),
                size: *size,
            },

            // Other types - no substitution needed
            _ => ty.clone(),
        }
    }

    /// Find all conversion paths from source to target type via From implementations.
    ///

    /// E0204 Multiple conversion paths: when try (?) operator finds multiple From implementations for error conversion, requiring explicit disambiguation — E0204 Multiple conversion paths
    ///

    /// Uses BFS to discover all possible conversion paths through From implementations.
    /// Returns a list of paths, where each path is a sequence of conversion steps.
    ///

    /// # Parameters
    ///

    /// - `from_type`: Source error type
    /// - `to_type`: Target error type
    ///

    /// # Returns
    ///

    /// List of all conversion paths found. Empty if no paths exist.
    ///

    /// # Performance
    ///

    /// - Maximum depth: 5 levels (prevents infinite recursion)
    /// - Cycle detection: Tracks visited types to avoid loops
    /// - Caching: Protocol checker results are cached
    ///

    /// # Visibility
    ///

    /// This method is `pub` to enable external testing but is not part of the stable API.
    pub fn find_all_conversion_paths(
        &self,
        from_type: &Type,
        to_type: &Type,
    ) -> List<ConversionPath> {
        use std::collections::{HashSet, VecDeque};

        const MAX_DEPTH: usize = 5;
        let mut paths = List::new();
        let mut queue = VecDeque::new();

        // Initialize queue with starting path
        queue.push_back(ConversionPath {
            steps: List::new(),
            current_type: from_type.clone(),
            visited: {
                let mut set = HashSet::new();
                set.insert(self.type_to_key(from_type));
                set
            },
        });

        while let Some(current_path) = queue.pop_front() {
            // Check if we've exceeded max depth
            if current_path.steps.len() >= MAX_DEPTH {
                continue;
            }

            // Check if we've reached the target
            if self.types_equivalent(&current_path.current_type, to_type) {
                paths.push(ConversionPath {
                    steps: current_path.steps.clone(),
                    current_type: current_path.current_type.clone(),
                    visited: current_path.visited.clone(),
                });
                continue;
            }

            // Find all From<current_type> implementations
            let next_conversions = self.find_from_implementations(&current_path.current_type);

            for (next_type, impl_span) in next_conversions {
                let type_key = self.type_to_key(&next_type);

                // Skip if we've already visited this type (cycle detection)
                if current_path.visited.contains(&type_key) {
                    continue;
                }

                // Create new path with this conversion step
                let mut new_steps = current_path.steps.clone();
                new_steps.push(ConversionStep {
                    from_type: current_path.current_type.clone(),
                    to_type: next_type.clone(),
                    impl_span,
                });

                let mut new_visited = current_path.visited.clone();
                new_visited.insert(type_key);

                queue.push_back(ConversionPath {
                    steps: new_steps,
                    current_type: next_type,
                    visited: new_visited,
                });
            }
        }

        paths
    }

    /// Find all From<source> implementations in the protocol checker.
    ///

    /// Returns list of (target_type, impl_span) tuples where From<source> for target_type exists.
    ///

    /// This searches the protocol registry for all implementations of the From protocol
    /// where the source type parameter matches the given source type.
    fn find_from_implementations(&self, source: &Type) -> List<(Type, Span)> {
        let mut results = List::new();

        // Strategy 1: Get all implementations registered in the protocol checker
        // We iterate through all known types and their implementations
        // Note: This is O(n) where n is the number of registered implementations
        // For large codebases, consider building an index by protocol name

        // Get all implementations for all types that might have From<source>
        // Since we can't iterate all types, we check common patterns:

        // Check if source type has known conversions to other types
        // These are the "outgoing" From conversions from this source

        // Pattern: From<source> for target - source converts TO target
        // We need to find all `target` types that have From<source>

        // Strategy 2: Check well-known types that commonly implement From
        let candidate_targets = self.get_candidate_from_targets(source);

        for candidate in candidate_targets {
            // Get implementations for this candidate type
            let protocol_checker_guard = self.protocol_checker.read();
            let impls = protocol_checker_guard.get_implementations(&candidate);

            for impl_ in impls {
                let protocol_name = impl_
                    .protocol
                    .as_ident()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();

                if protocol_name == "From" {
                    // Check if this From<T> matches our source
                    if let Some(from_source) = impl_.protocol_args.first() {
                        if self.types_match_for_from(source, from_source) {
                            // This implementation converts from our source to candidate
                            let span = impl_.protocol.span();
                            results.push((candidate.clone(), span));
                        }
                    }
                }
            }
        }

        // Strategy 3: Add standard library conversions
        results.extend(self.get_standard_from_targets(source));

        results
    }

    /// Get candidate target types that might have From<source> implementations
    ///

    /// This returns types that commonly implement From for the given source type.
    fn get_candidate_from_targets(&self, source: &Type) -> List<Type> {
        use verum_ast::Path;

        let mut candidates = List::new();

        // Helper to create a named type
        let make_named = |name: &str, args: List<Type>| -> Type {
            Type::Named {
                path: Path::from_ident(verum_ast::Ident::new(name, Span::default())),
                args,
            }
        };

        match source {
            // Error types commonly convert to wrapper error types
            Type::Named { path, .. } => {
                let name = path.as_ident().map(|i| i.as_str()).unwrap_or("");
                if name.ends_with("Error") {
                    // Check for common error wrappers
                    candidates.push(make_named("AppError", List::new()));
                    candidates.push(make_named("BoxError", List::new()));
                } else {
                    // For any named type T, check Maybe<T>, List<T>
                    candidates.push(make_named(WKT::Maybe.as_str(), vec![source.clone()].into()));
                    candidates.push(make_named(WKT::List.as_str(), vec![source.clone()].into()));
                }
            }

            // Int converts to Float, Text, etc.
            Type::Int => {
                candidates.push(Type::Float);
                candidates.push(Type::Text);
            }

            // Char converts to Int, Text
            Type::Char => {
                candidates.push(Type::Int);
                candidates.push(Type::Text);
            }

            // Bool converts to Int, Text
            Type::Bool => {
                candidates.push(Type::Int);
                candidates.push(Type::Text);
            }

            // For any other type T, check Maybe<T>, List<T>
            ty => {
                candidates.push(make_named(WKT::Maybe.as_str(), vec![ty.clone()].into()));
                candidates.push(make_named(WKT::List.as_str(), vec![ty.clone()].into()));
            }
        }

        candidates
    }

    /// Get standard library From implementations for a source type
    ///

    /// Returns predefined conversions from the standard library.
    fn get_standard_from_targets(&self, source: &Type) -> List<(Type, Span)> {
        use verum_ast::Path;

        let mut results = List::new();
        let default_span = Span::default();

        // Helper to create a named type
        let make_named = |name: &str, args: List<Type>| -> Type {
            Type::Named {
                path: Path::from_ident(verum_ast::Ident::new(name, Span::default())),
                args,
            }
        };

        match source {
            // Int -> Float
            Type::Int => {
                results.push((Type::Float, default_span));
            }

            // Char -> Int
            Type::Char => {
                results.push((Type::Int, default_span));
            }

            // T -> Maybe<T>
            ty => {
                results.push((
                    make_named(WKT::Maybe.as_str(), vec![ty.clone()].into()),
                    default_span,
                ));
            }
        }

        results
    }

    /// Check if two types are equivalent for conversion path detection.
    ///

    /// This is used instead of direct equality to handle type aliases and normalization.
    ///

    /// # Visibility
    ///

    /// This method is `pub` to enable external testing but is not part of the stable API.
    pub fn types_equivalent(&self, ty1: &Type, ty2: &Type) -> bool {
        // Normalize both types and compare
        let norm1 = self.normalize_type(ty1);
        let norm2 = self.normalize_type(ty2);
        norm1 == norm2
    }

    /// Convert a type to a unique key for cycle detection.
    ///

    /// This creates a stable hash key that can be used to track visited types.
    ///

    /// # Visibility
    ///

    /// This method is `pub` to enable external testing but is not part of the stable API.
    pub fn type_to_key(&self, ty: &Type) -> Text {
        // Use the type's textual representation as a key
        // In production, this might use a more sophisticated hashing scheme
        ty.to_text()
    }

    /// Unwrap reference types to get the underlying type.
    /// Used for auto-deref in pattern matching (e.g., matching &[T] with array pattern).
    pub(crate) fn unwrap_references(&self, ty: &Type) -> Type {
        match ty {
            Type::Reference { inner, .. } => self.unwrap_references(inner),
            Type::CheckedReference { inner, .. } => self.unwrap_references(inner),
            Type::UnsafeReference { inner, .. } => self.unwrap_references(inner),
            _ => ty.clone(),
        }
    }

    /// Normalize a type by resolving type aliases and simplifying.
    pub(crate) fn normalize_type(&self, ty: &Type) -> Type {
        // RAII depth guard — decrements on drop even if we panic or return early
        const MAX_NORMALIZE_DEPTH: usize = 128;
        let _guard = match ThreadLocalDepthGuard::new(&NORMALIZE_DEPTH, MAX_NORMALIZE_DEPTH) {
            Some(g) => g,
            None => {
                // Exceeded depth limit — return type as-is to prevent stack overflow
                return ty.clone();
            }
        };

        self.normalize_type_impl(ty, 0)
    }

    // Note: normalize_type_impl is the actual implementation that tracks depth

    /// Internal implementation with depth tracking to prevent infinite recursion.
    fn normalize_type_impl(&self, ty: &Type, depth: usize) -> Type {
        // Prevent infinite recursion with a reasonable depth limit
        const MAX_DEPTH: usize = 50;
        if depth > MAX_DEPTH {
            return ty.clone();
        }

        use Type::*;

        match ty {
            // Resolve Named types to their underlying type definitions (type aliases)
            Named { path, args } => self.normalize_named_type(path, args, depth),

            // Recursively normalize nested types
            Tuple(tys) => {
                let d = depth + 1;
                let normalized_tys: List<Type> =
                    tys.iter().map(|t| self.normalize_type_impl(t, d)).collect();
                Tuple(normalized_tys)
            }

            Array { element, size } => Array {
                element: Box::new(self.normalize_type_impl(element, depth + 1)),
                size: *size,
            },

            Slice { element } => Slice {
                element: Box::new(self.normalize_type_impl(element, depth + 1)),
            },

            Record(fields) => {
                let d = depth + 1;
                let normalized_fields: indexmap::IndexMap<verum_common::Text, Type> = fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), self.normalize_type_impl(ty, d)))
                    .collect();
                Record(normalized_fields)
            }

            Variant(variants) => {
                let d = depth + 1;
                let normalized_variants: indexmap::IndexMap<verum_common::Text, Type> = variants
                    .iter()
                    .map(|(tag, ty)| (tag.clone(), self.normalize_type_impl(ty, d)))
                    .collect();
                Variant(normalized_variants)
            }

            Reference { mutable, inner } => Reference {
                mutable: *mutable,
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            CheckedReference { mutable, inner } => CheckedReference {
                mutable: *mutable,
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            UnsafeReference { mutable, inner } => UnsafeReference {
                mutable: *mutable,
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            Ownership { mutable, inner } => Ownership {
                mutable: *mutable,
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            Pointer { mutable, inner } => Pointer {
                mutable: *mutable,
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            VolatilePointer { mutable, inner } => VolatilePointer {
                mutable: *mutable,
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            Refined { base, predicate } => Refined {
                base: Box::new(self.normalize_type_impl(base, depth + 1)),
                predicate: predicate.clone(),
            },

            Generic { name, args } => {
                let d = depth + 1;
                let normalized_args: List<Type> = args
                    .iter()
                    .map(|t| self.normalize_type_impl(t, d))
                    .collect();

                // Check if this is an associated type projection
                // Format 1: <SelfType as Protocol>::AssocName (old Rust-like format)
                // Format 2: ::AssocName with base_ty in args[0] (new Verum format)

                // Format 1: <T as Protocol>::AssocName
                if name.starts_with("<") && name.contains(" as ") && name.contains(">::") {
                    // Try to resolve the projection now that args are normalized
                    if let Some(resolved) =
                        self.try_resolve_deferred_projection(name, &normalized_args)
                    {
                        return self.normalize_type_impl(&resolved, depth + 1);
                    }
                }

                // Format 2: ::AssocName with base_ty in args[0]
                // This format is used for T.Item, C.Iter.Item etc.
                if name.starts_with("::") && !normalized_args.is_empty() {
                    let assoc_name = &name[2..]; // Strip "::" prefix
                    let base_ty = &normalized_args[0];

                    // Try to resolve projection from base type's protocol implementations
                    if let Some(resolved) =
                        self.try_resolve_associated_type_projection(base_ty, assoc_name)
                    {
                        return self.normalize_type_impl(&resolved, depth + 1);
                    }
                }

                Generic {
                    name: name.clone(),
                    args: normalized_args,
                }
            }

            Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => {
                let d = depth + 1;
                let normalized_params: List<Type> = params
                    .iter()
                    .map(|t| self.normalize_type_impl(t, d))
                    .collect();
                Function {
                    params: normalized_params,
                    return_type: Box::new(self.normalize_type_impl(return_type, depth + 1)),
                    type_params: type_params.clone(),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                }
            }

            Meta {
                name,
                ty: inner_ty,
                refinement,
                value,
            } => Meta {
                name: name.clone(),
                ty: Box::new(self.normalize_type_impl(inner_ty, depth + 1)),
                refinement: refinement.clone(),
                value: value.clone(),
            },

            Exists { var, body } => Exists {
                var: *var,
                body: Box::new(self.normalize_type_impl(body, depth + 1)),
            },

            Forall { vars, body } => Forall {
                vars: vars.clone(),
                body: Box::new(self.normalize_type_impl(body, depth + 1)),
            },

            Future { output } => Future {
                output: Box::new(self.normalize_type_impl(output, depth + 1)),
            },

            Generator {
                yield_ty,
                return_ty,
            } => Generator {
                yield_ty: Box::new(self.normalize_type_impl(yield_ty, depth + 1)),
                return_ty: Box::new(self.normalize_type_impl(return_ty, depth + 1)),
            },

            Tensor {
                element,
                shape,
                strides,
                span,
            } => Tensor {
                element: Box::new(self.normalize_type_impl(element, depth + 1)),
                shape: shape.clone(),
                strides: strides.clone(),
                span: *span,
            },

            Lifetime { name } => Lifetime { name: name.clone() },

            GenRef { inner } => GenRef {
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
            },

            TypeConstructor { name, arity, kind } => TypeConstructor {
                name: name.clone(),
                arity: *arity,
                kind: kind.clone(),
            },

            TypeApp { constructor, args } => self.normalize_type_app(constructor, args, depth),

            // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            Pi {
                param_name,
                param_type,
                return_type,
            } => Pi {
                param_name: param_name.clone(),
                param_type: Box::new(self.normalize_type_impl(param_type, depth + 1)),
                return_type: Box::new(self.normalize_type_impl(return_type, depth + 1)),
            },

            Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Sigma {
                fst_name: fst_name.clone(),
                fst_type: Box::new(self.normalize_type_impl(fst_type, depth + 1)),
                snd_type: Box::new(self.normalize_type_impl(snd_type, depth + 1)),
            },

            Eq {
                ty: eq_ty,
                lhs,
                rhs,
            } => Eq {
                ty: Box::new(self.normalize_type_impl(eq_ty, depth + 1)),
                lhs: lhs.clone(),
                rhs: rhs.clone(),
            },

            // Path type: Path<A>(a, b) — normalize the space type; endpoints are value-level
            PathType { space, left, right } => PathType {
                space: Box::new(self.normalize_type_impl(space, depth + 1)),
                left: left.clone(),
                right: right.clone(),
            },

            // Partial element type: Partial<A>(φ) — normalize the element type; face is value-level
            Partial { element_type, face } => Partial {
                element_type: Box::new(self.normalize_type_impl(element_type, depth + 1)),
                face: face.clone(),
            },

            // Interval is a built-in primitive; nothing to normalize
            Interval => Interval,

            Universe { level } => Universe { level: *level },
            Prop => Prop,

            Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => {
                let d = depth + 1;
                Inductive {
                    name: name.clone(),
                    params: params
                        .iter()
                        .map(|(n, t)| (n.clone(), Box::new(self.normalize_type_impl(t, d))))
                        .collect(),
                    indices: indices
                        .iter()
                        .map(|(n, t)| (n.clone(), Box::new(self.normalize_type_impl(t, d))))
                        .collect(),
                    universe: *universe,
                    constructors: constructors.clone(),
                }
            }

            Coinductive {
                name,
                params,
                destructors,
            } => {
                let d = depth + 1;
                Coinductive {
                    name: name.clone(),
                    params: params
                        .iter()
                        .map(|(n, t)| (n.clone(), Box::new(self.normalize_type_impl(t, d))))
                        .collect(),
                    destructors: destructors.clone(),
                }
            }

            HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => {
                let d = depth + 1;
                HigherInductive {
                    name: name.clone(),
                    params: params
                        .iter()
                        .map(|(n, t)| (n.clone(), Box::new(self.normalize_type_impl(t, d))))
                        .collect(),
                    point_constructors: point_constructors.clone(),
                    path_constructors: path_constructors.clone(),
                }
            }

            Quantified { inner, quantity } => Quantified {
                inner: Box::new(self.normalize_type_impl(inner, depth + 1)),
                quantity: *quantity,
            },

            // Primitive types and tuples don't need normalization
            Unit | Bool | Int | Float | Char | Text | Never => ty.clone(),

            // Type variables: try to resolve through unifier before returning
            Var(tv) => {
                let resolved = self.unifier.apply(ty);
                if let Type::Var(resolved_tv) = &resolved {
                    if resolved_tv == tv {
                        // Not resolved — return as-is to avoid infinite loop
                        return ty.clone();
                    }
                }
                // Resolved to something new — normalize recursively
                self.normalize_type_impl(&resolved, depth + 1)
            }

            // Placeholder types - resolve to actual type if available
            // Placeholders are used during two-pass type resolution to allow forward references.
            // After pass 2 completes, all placeholders should be resolved to actual types.
            // This normalization step handles any remaining placeholders by looking them up.
            Placeholder { name, span: _ } => {
                if let Maybe::Some(resolved_ty) = self.ctx.lookup_type(name.as_str()) {
                    // Found the resolved type! Check if it's still a placeholder to avoid infinite loops
                    if let Placeholder { .. } = resolved_ty {
                        // Still a placeholder - return as-is (indicates unresolved forward reference)
                        ty.clone()
                    } else {
                        // Recursively normalize the resolved type
                        self.normalize_type_impl(resolved_ty, depth + 1)
                    }
                } else {
                    // Not found - return as-is (will be caught by verify_no_placeholders)
                    ty.clone()
                }
            }

            // ExtensibleRecord - normalize fields
            ExtensibleRecord { fields, row_var } => {
                let normalized_fields = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.normalize_type_impl(v, depth + 1)))
                    .collect();
                Type::ExtensibleRecord {
                    fields: normalized_fields,
                    row_var: *row_var,
                }
            }

            // CapabilityRestricted - normalize base type
            CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(self.normalize_type_impl(base, depth + 1)),
                capabilities: capabilities.clone(),
            },

            // Unknown type - no normalization needed (it's already fully reduced)
            Unknown => ty.clone(),

            // DynProtocol - normalize the associated type bindings
            DynProtocol { bounds, bindings } => Type::DynProtocol {
                bounds: bounds.clone(),
                bindings: bindings
                    .iter()
                    .map(|(k, v)| (k.clone(), self.normalize_type_impl(v, depth + 1)))
                    .collect(),
            },
        }
    }

    /// Type-level computation: Apply a dependent function type (Pi type) to a term argument.
    ///

    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 - Type-Level Computation
    ///

    /// This implements beta reduction for Pi types:
    /// - Given `(x: A) -> B(x)` and a term `t: A`, produce `B(t)`
    /// - The return type may reference the parameter name, so we substitute it
    ///

    /// # Examples
    /// ```ignore
    /// // Type: (n: Nat) -> Vec n Int
    /// // Argument: 5
    /// // Result: Vec 5 Int
    /// ```
    ///

    /// # Parameters
    /// - `pi_type`: The Pi type to apply
    /// - `arg_term`: The EqTerm representing the argument value
    ///

    /// # Returns
    /// The instantiated return type with the parameter substituted by the argument.
    /// Normalize a `Named { path, args }` type.
    /// Resolves type aliases, applies generic-param substitution, and
    /// guards against self-referential / mutually-recursive cycles.
    fn normalize_named_type(
        &self,
        path: &verum_ast::ty::Path,
        args: &List<Type>,
        depth: usize,
    ) -> Type {
        use Type::*;
                // Convert path to string for lookup (handles both simple and qualified paths)
                let type_name = self.path_to_string(path);

                // CYCLE GUARD: Detect indirect circular type normalization.
                // For types like A{b:B} -> B{c:C} -> C{a:A}, direct self-reference
                // checks fail because A doesn't mention itself directly. The thread-local
                // stack catches these multi-step cycles and prevents infinite expansion.
                let _normalize_guard =
                    match NormalizeTypeCycleGuard::try_enter(type_name.to_string()) {
                        Some(guard) => guard,
                        None => {
                            // Cycle detected: preserve nominal type, normalize args only
                            let normalized_args: List<Type> = args
                                .iter()
                                .map(|a| self.normalize_type_impl(a, depth + 1))
                                .collect();
                            return Named {
                                path: path.clone(),
                                args: normalized_args,
                            };
                        }
                    };

                // CRITICAL FIX: For generic types like Pair<Int>, we need to:
                // 1. Look up the struct fields (which have unsubstituted type params like T)
                // 2. Build a substitution map from type params to concrete args (T -> Int)
                // 3. Apply substitution to field types before returning
                //

                // This ensures that `pair.first` where `pair: Pair<Int>` returns `Int`, not `T`

                // Build type parameter substitution map for generic types
                // This maps type param names to concrete args: e.g., { T -> Int } for Pair<Int>
                let build_param_subst = || -> indexmap::IndexMap<verum_common::Text, Type> {
                    if args.is_empty() {
                        return indexmap::IndexMap::new();
                    }
                    let type_params_key = format!("__type_params_{}", type_name);
                    let type_params: List<verum_common::Text> =
                        match self.ctx.lookup_type(type_params_key.as_str()) {
                            Maybe::Some(Type::Record(params_map)) => {
                                params_map.keys().cloned().collect()
                            }
                            _ => List::new(),
                        };

                    let mut param_subst: indexmap::IndexMap<verum_common::Text, Type> =
                        indexmap::IndexMap::new();
                    for (param_name, arg_ty) in type_params.iter().zip(args.iter()) {
                        param_subst.insert(param_name.clone(), arg_ty.clone());
                    }
                    param_subst
                };

                // For type aliases, first try to resolve via the alias table
                // This ensures proper substitution for generic aliases like Reducer<Int, Int>
                if let Maybe::Some(underlying_alias_ty) = self.ctx.resolve_alias(type_name.as_str())
                {
                    // Guard against self-referential aliases: if the alias resolves to a Named type
                    // with the same base name (e.g., Transducer -> Named(Transducer, ...)),
                    // recursing would loop infinitely. Instead, normalize the args directly.
                    let is_self_referential = match &underlying_alias_ty {
                        Named {
                            path: alias_path, ..
                        } => {
                            let alias_name = self.path_to_string(alias_path);
                            alias_name == type_name
                        }
                        Variant(_) | Record(_) => {
                            Self::type_mentions_name(underlying_alias_ty, &type_name)
                        }
                        _ => false,
                    };
                    if is_self_referential {
                        // Self-referential alias: just normalize args in-place
                        let normalized_args: List<Type> = args
                            .iter()
                            .map(|a| self.normalize_type_impl(a, depth + 1))
                            .collect();
                        return Named {
                            path: path.clone(),
                            args: normalized_args,
                        };
                    }
                    let param_subst = build_param_subst();
                    let substituted_ty = if param_subst.is_empty() {
                        underlying_alias_ty.clone()
                    } else {
                        self.substitute_type_params(underlying_alias_ty, &param_subst)
                    };
                    return self.normalize_type_impl(&substituted_ty, depth + 1);
                }

                // Look up the type definition using qualified lookup
                // This handles both simple types (e.g., "Int") and qualified types (e.g., "dto.user_dto.CreateTokenRequestDto")
                if let Maybe::Some(underlying_ty) =
                    self.ctx.lookup_qualified_type(type_name.as_str())
                {
                    // Check if it's the same type (self-reference) to avoid infinite loop
                    let is_self_referential = match underlying_ty {
                        Named { path: p2, .. } => {
                            let underlying_name = self.path_to_string(p2);
                            underlying_name == type_name
                        }
                        // CRITICAL FIX: Variant types that contain recursive references
                        // to the defining type (e.g., type Expr is | Lit(Int) | Add(Heap<Expr>, Heap<Expr>))
                        // must NOT be expanded during normalization. Expanding them creates
                        // exponential blowup: each level expands N recursive variants, leading to
                        // O(N^depth) work. Instead, preserve the nominal Named type reference.
                        Variant(_) | Record(_) => {
                            Self::type_mentions_name(underlying_ty, &type_name)
                        }
                        _ => false,
                    };
                    if is_self_referential {
                        // Self-referential type - preserve nominal identity but normalize args.
                        let normalized_args: List<Type> = args
                            .iter()
                            .map(|a| self.normalize_type_impl(a, depth + 1))
                            .collect();
                        return Named {
                            path: path.clone(),
                            args: normalized_args,
                        };
                    }
                    // CRITICAL FIX: Apply type parameter substitution before normalizing
                    // This handles generic types like Maybe<Maybe<Int>> where we need to
                    // substitute T -> Maybe<Int> in the variant definition
                    let param_subst = build_param_subst();
                    let substituted_ty = if param_subst.is_empty() {
                        underlying_ty.clone()
                    } else {
                        self.substitute_type_params(underlying_ty, &param_subst)
                    };
                    // Recursively normalize the underlying type
                    // This handles cases like: type A = B; type B = (Int, Int);
                    return self.normalize_type_impl(&substituted_ty, depth + 1);
                }

                // NOMINAL TYPING: Do NOT resolve Named struct types to their underlying Record.
                // Named struct types are nominal - `User` and `Admin` are different types even
                // if they have identical fields. The __struct_fields_ lookup is only used
                // for field access validation, NOT for type equivalence during unification.
                // Preserve nominal type identity but normalize args to resolve any associated
                // type projections (e.g., ::Item[Range<Int>] -> Int) inside the type arguments.
                Named {
                    path: path.clone(),
                    args: args
                        .iter()
                        .map(|a| self.normalize_type_impl(a, depth + 1))
                        .collect(),
                }
    }

    /// Normalize a `TypeApp { constructor, args }` type.
    /// Handles GAT resolution, HKT constructor application, and
    /// associated-type projection re-normalization after unification.
    fn normalize_type_app(
        &self,
        constructor: &Box<Type>,
        args: &List<Type>,
        depth: usize,
    ) -> Type {
        use Type::*;
                let d = depth + 1;
                let normalized_ctor = self.normalize_type_impl(constructor, d);
                let normalized_args: List<Type> = args
                    .iter()
                    .map(|t| self.normalize_type_impl(t, d))
                    .collect();

                // GAT resolution: if the constructor resolved to a concrete type
                // (e.g., ::Item on CloneableWrapper resolved to List<T>),
                // substitute the GAT type parameters with the TypeApp args.
                // This turns TypeApp { ctor: List<$tv>, args: [Int] } into List<Int>.

                // Helper: collect free TypeVars from ctor_args positionally and build
                // a TypeVar→Type substitution using normalized_args.
                let try_subst_vars = |ctor_args: &[Type]| -> Option<crate::ty::Substitution> {
                    let mut subst = crate::ty::Substitution::default();
                    for (i, arg) in ctor_args.iter().enumerate() {
                        if let Type::Var(tv) = arg {
                            if let Some(replacement) = normalized_args.get(i) {
                                subst.insert(*tv, replacement.clone());
                            }
                        }
                    }
                    if subst.is_empty() { None } else { Some(subst) }
                };

                match &normalized_ctor {
                    Generic {
                        name,
                        args: ctor_args,
                    } if !name.starts_with("::") => {
                        if let Some(subst) = try_subst_vars(ctor_args) {
                            return self
                                .normalize_type_impl(&normalized_ctor.apply_subst(&subst), d);
                        }
                        // Fallback: if no vars to substitute, merge args
                        if normalized_args.is_empty() {
                            normalized_ctor
                        } else {
                            Generic {
                                name: name.clone(),
                                args: normalized_args,
                            }
                        }
                    }
                    Named {
                        path,
                        args: ctor_args,
                    } => {
                        if let Some(subst) = try_subst_vars(ctor_args) {
                            return self
                                .normalize_type_impl(&normalized_ctor.apply_subst(&subst), d);
                        }
                        if normalized_args.is_empty() {
                            normalized_ctor
                        } else {
                            Named {
                                path: path.clone(),
                                args: normalized_args,
                            }
                        }
                    }
                    // Constructor normalized to a Variant type (e.g., Maybe<T> → None(Unit) | Some(T)).
                    // Substitute free type vars positionally with the TypeApp args.
                    Variant(_) => {
                        if let Some(subst) = try_subst_vars(
                            // Collect all Var nodes from the variant type to build positional subst
                            &Self::collect_free_vars_ordered(&normalized_ctor)
                                .into_iter()
                                .map(Type::Var)
                                .collect::<Vec<_>>(),
                        ) {
                            return self
                                .normalize_type_impl(&normalized_ctor.apply_subst(&subst), d);
                        }
                        // No vars to substitute — return as-is
                        normalized_ctor
                    }
                    // Constructor is a :: projection that couldn't be resolved yet.
                    // Try applying the unifier to resolve type vars in the constructor's
                    // args, then re-normalize. This handles the case where C.Item<T> has
                    // C still as a type var at initial normalization time, but the unifier
                    // has since resolved it to a concrete type (e.g., CloneableWrapper).
                    Generic {
                        name,
                        args: proj_args,
                    } if name.starts_with("::") => {
                        let resolved_ctor = self.unifier.apply(&normalized_ctor);
                        if resolved_ctor != normalized_ctor {
                            // Type vars were resolved — re-normalize the whole TypeApp
                            let new_typeapp = TypeApp {
                                constructor: Box::new(resolved_ctor),
                                args: normalized_args,
                            };
                            return self.normalize_type_impl(&new_typeapp, d);
                        }
                        // Still unresolved — return as-is
                        TypeApp {
                            constructor: Box::new(normalized_ctor),
                            args: normalized_args,
                        }
                    }
                    // HKT: TypeConstructor applied to args -> Generic<args>
                    Type::TypeConstructor { name, arity, .. } => {
                        if *arity > 0 && normalized_args.len() == *arity {
                            if let Maybe::Some(underlying) = self.ctx.lookup_type(name.as_str()) {
                                match underlying {
                                    Variant(_) | Record(_) => {
                                        let free_vars = Self::collect_free_vars_ordered(underlying);
                                        let mut subst = crate::ty::Substitution::default();
                                        for (i, tv) in free_vars.iter().enumerate() {
                                            if let Some(arg) = normalized_args.get(i) {
                                                subst.insert(*tv, arg.clone());
                                            }
                                        }
                                        if !subst.is_empty() {
                                            return self.normalize_type_impl(
                                                &underlying.apply_subst(&subst),
                                                d,
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Generic {
                                name: name.clone(),
                                args: normalized_args,
                            }
                        } else if normalized_args.is_empty() {
                            normalized_ctor
                        } else {
                            Generic {
                                name: name.clone(),
                                args: normalized_args,
                            }
                        }
                    }
                    Type::Var(_) => TypeApp {
                        constructor: Box::new(normalized_ctor),
                        args: normalized_args,
                    },
                    _ => TypeApp {
                        constructor: Box::new(normalized_ctor),
                        args: normalized_args,
                    },
                }
    }

    fn apply_pi_type(&self, pi_type: &Type, arg_term: &crate::ty::EqTerm) -> Type {
        match pi_type {
            Type::Pi {
                param_name,
                return_type,
                ..
            } => {
                // Substitute the parameter with the argument term in the return type
                self.substitute_term_in_type(return_type, param_name, arg_term)
            }
            _ => {
                // Not a Pi type, return as-is (caller should verify this doesn't happen)
                pi_type.clone()
            }
        }
    }

    /// Substitute a term for a variable in a dependent type.
    ///

    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 - Type-Level Computation
    ///

    /// This is needed for dependent types where the type depends on a value.
    /// For example, in `(n: Nat) -> Vec n Int`, the return type `Vec n Int`
    /// references the parameter `n`.
    ///

    /// # Parameters
    /// - `ty`: The type containing references to the variable
    /// - `var_name`: Name of the variable to substitute
    /// - `term`: The term to substitute for the variable
    ///

    /// # Returns
    /// The type with all occurrences of `var_name` replaced by `term`.
    ///

    /// # Implementation Notes
    /// - Performs capture-avoiding substitution
    /// - Handles shadowing correctly in nested scopes
    pub(super) fn substitute_term_in_type(
        &self,
        ty: &Type,
        var_name: &Text,
        term: &crate::ty::EqTerm,
    ) -> Type {
        self.substitute_term_in_type_impl(ty, var_name, term, 0)
    }

    /// Internal implementation of term-in-type substitution with depth tracking.
    fn substitute_term_in_type_impl(
        &self,
        ty: &Type,
        var_name: &Text,
        term: &crate::ty::EqTerm,
        depth: usize,
    ) -> Type {
        const MAX_DEPTH: usize = 50;
        if depth > MAX_DEPTH {
            return ty.clone();
        }

        use Type::*;
        let next_depth = depth + 1;

        match ty {
            // For Named and Generic types, we need to check if they contain term-level indices
            // This is for types like `Vec n T` where `n` is a value parameter
            Named { path, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_term_in_type_impl(arg, var_name, term, next_depth))
                    .collect();
                Named {
                    path: path.clone(),
                    args: new_args,
                }
            }

            Generic { name, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_term_in_type_impl(arg, var_name, term, next_depth))
                    .collect();
                Generic {
                    name: name.clone(),
                    args: new_args,
                }
            }

            // Pi type: (x: A) -> B
            // Substitute in param_type, but be careful with return_type
            Pi {
                param_name,
                param_type,
                return_type,
            } => {
                let new_param_type =
                    self.substitute_term_in_type_impl(param_type, var_name, term, next_depth);

                // If the Pi's parameter shadows our variable, don't substitute in return_type
                let new_return_type = if param_name == var_name {
                    (**return_type).clone()
                } else {
                    self.substitute_term_in_type_impl(return_type, var_name, term, next_depth)
                };

                Pi {
                    param_name: param_name.clone(),
                    param_type: Box::new(new_param_type),
                    return_type: Box::new(new_return_type),
                }
            }

            // Sigma type: (x: A, B(x))
            Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => {
                let new_fst_type =
                    self.substitute_term_in_type_impl(fst_type, var_name, term, next_depth);

                // If the Sigma's first component shadows our variable, don't substitute in snd_type
                let new_snd_type = if fst_name == var_name {
                    (**snd_type).clone()
                } else {
                    self.substitute_term_in_type_impl(snd_type, var_name, term, next_depth)
                };

                Sigma {
                    fst_name: fst_name.clone(),
                    fst_type: Box::new(new_fst_type),
                    snd_type: Box::new(new_snd_type),
                }
            }

            // Equality type: Eq<T, lhs, rhs>
            // The type T may contain the variable, but lhs/rhs are terms
            Eq {
                ty: eq_ty,
                lhs,
                rhs,
            } => {
                let new_ty = self.substitute_term_in_type_impl(eq_ty, var_name, term, next_depth);

                // Substitute in the term-level expressions (lhs and rhs are EqTerms)
                let new_lhs = self.substitute_term_in_eq_term(lhs, var_name, term);
                let new_rhs = self.substitute_term_in_eq_term(rhs, var_name, term);

                Eq {
                    ty: Box::new(new_ty),
                    lhs: Box::new(new_lhs),
                    rhs: Box::new(new_rhs),
                }
            }

            // Path type: Path<A>(a, b)
            // The space A may contain the variable; endpoints are CubicalTerms (value-level)
            PathType { space, left, right } => PathType {
                space: Box::new(
                    self.substitute_term_in_type_impl(space, var_name, term, next_depth),
                ),
                left: left.clone(),
                right: right.clone(),
            },

            // Partial element type: Partial<A>(φ)
            // The element type A may contain the variable; face φ is a CubicalTerm (value-level)
            Partial { element_type, face } => Partial {
                element_type: Box::new(self.substitute_term_in_type_impl(
                    element_type,
                    var_name,
                    term,
                    next_depth,
                )),
                face: face.clone(),
            },

            // Interval is a built-in primitive with no inner types; nothing to substitute
            Interval => Interval,

            // Function types
            Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => {
                let new_params: List<Type> = params
                    .iter()
                    .map(|p| self.substitute_term_in_type_impl(p, var_name, term, next_depth))
                    .collect();
                let new_return =
                    self.substitute_term_in_type_impl(return_type, var_name, term, next_depth);

                Function {
                    params: new_params,
                    return_type: Box::new(new_return),
                    type_params: type_params.clone(),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                }
            }

            // Container types
            Tuple(tys) => {
                let new_tys: List<Type> = tys
                    .iter()
                    .map(|t| self.substitute_term_in_type_impl(t, var_name, term, next_depth))
                    .collect();
                Tuple(new_tys)
            }

            Array { element, size } => Array {
                element: Box::new(
                    self.substitute_term_in_type_impl(element, var_name, term, next_depth),
                ),
                size: *size,
            },

            Slice { element } => Slice {
                element: Box::new(
                    self.substitute_term_in_type_impl(element, var_name, term, next_depth),
                ),
            },

            Record(fields) => {
                let new_fields: indexmap::IndexMap<verum_common::Text, Type> = fields
                    .iter()
                    .map(|(name, ty)| {
                        (
                            name.clone(),
                            self.substitute_term_in_type_impl(ty, var_name, term, next_depth),
                        )
                    })
                    .collect();
                Record(new_fields)
            }

            Variant(variants) => {
                let new_variants: indexmap::IndexMap<verum_common::Text, Type> = variants
                    .iter()
                    .map(|(tag, ty)| {
                        (
                            tag.clone(),
                            self.substitute_term_in_type_impl(ty, var_name, term, next_depth),
                        )
                    })
                    .collect();
                Variant(new_variants)
            }

            // Reference types
            Reference { mutable, inner } => Reference {
                mutable: *mutable,
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            CheckedReference { mutable, inner } => CheckedReference {
                mutable: *mutable,
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            UnsafeReference { mutable, inner } => UnsafeReference {
                mutable: *mutable,
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            Ownership { mutable, inner } => Ownership {
                mutable: *mutable,
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            Pointer { mutable, inner } => Pointer {
                mutable: *mutable,
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            VolatilePointer { mutable, inner } => VolatilePointer {
                mutable: *mutable,
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            // Refined types
            Refined { base, predicate } => Refined {
                base: Box::new(self.substitute_term_in_type_impl(base, var_name, term, next_depth)),
                predicate: predicate.clone(),
            },

            // Quantified types
            Exists { var, body } => Exists {
                var: *var,
                body: Box::new(self.substitute_term_in_type_impl(body, var_name, term, next_depth)),
            },

            Forall { vars, body } => Forall {
                vars: vars.clone(),
                body: Box::new(self.substitute_term_in_type_impl(body, var_name, term, next_depth)),
            },

            // Meta parameters
            Meta {
                name,
                ty: meta_ty,
                refinement,
                value,
            } => Meta {
                name: name.clone(),
                ty: Box::new(
                    self.substitute_term_in_type_impl(meta_ty, var_name, term, next_depth),
                ),
                refinement: refinement.clone(),
                value: value.clone(),
            },

            // Async types
            Future { output } => Future {
                output: Box::new(
                    self.substitute_term_in_type_impl(output, var_name, term, next_depth),
                ),
            },

            Generator {
                yield_ty,
                return_ty,
            } => Generator {
                yield_ty: Box::new(
                    self.substitute_term_in_type_impl(yield_ty, var_name, term, next_depth),
                ),
                return_ty: Box::new(
                    self.substitute_term_in_type_impl(return_ty, var_name, term, next_depth),
                ),
            },

            // Tensor types
            Tensor {
                element,
                shape,
                strides,
                span,
            } => Tensor {
                element: Box::new(
                    self.substitute_term_in_type_impl(element, var_name, term, next_depth),
                ),
                shape: shape.clone(),
                strides: strides.clone(),
                span: *span,
            },

            // Higher-kinded types
            TypeConstructor { .. } => ty.clone(),

            TypeApp { constructor, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_term_in_type_impl(arg, var_name, term, next_depth))
                    .collect();
                TypeApp {
                    constructor: Box::new(self.substitute_term_in_type_impl(
                        constructor,
                        var_name,
                        term,
                        next_depth,
                    )),
                    args: new_args,
                }
            }

            // Inductive types
            Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => {
                let new_params: List<(verum_common::Text, Box<Type>)> = params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(
                                self.substitute_term_in_type_impl(t, var_name, term, next_depth),
                            ),
                        )
                    })
                    .collect();
                let new_indices: List<(verum_common::Text, Box<Type>)> = indices
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(
                                self.substitute_term_in_type_impl(t, var_name, term, next_depth),
                            ),
                        )
                    })
                    .collect();

                Inductive {
                    name: name.clone(),
                    params: new_params,
                    indices: new_indices,
                    universe: *universe,
                    constructors: constructors.clone(),
                }
            }

            Coinductive {
                name,
                params,
                destructors,
            } => {
                let new_params: List<(verum_common::Text, Box<Type>)> = params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(
                                self.substitute_term_in_type_impl(t, var_name, term, next_depth),
                            ),
                        )
                    })
                    .collect();

                Coinductive {
                    name: name.clone(),
                    params: new_params,
                    destructors: destructors.clone(),
                }
            }

            HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => {
                let new_params: List<(verum_common::Text, Box<Type>)> = params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(
                                self.substitute_term_in_type_impl(t, var_name, term, next_depth),
                            ),
                        )
                    })
                    .collect();

                HigherInductive {
                    name: name.clone(),
                    params: new_params,
                    point_constructors: point_constructors.clone(),
                    path_constructors: path_constructors.clone(),
                }
            }

            // Quantified linear types
            Quantified { inner, quantity } => Quantified {
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
                quantity: *quantity,
            },

            // Lifetime types
            Lifetime { .. } => ty.clone(),
            GenRef { inner } => GenRef {
                inner: Box::new(
                    self.substitute_term_in_type_impl(inner, var_name, term, next_depth),
                ),
            },

            // Base types and type variables - no substitution needed
            Unit | Bool | Int | Float | Char | Text | Var(_) | Never | Universe { .. } | Prop => {
                ty.clone()
            }

            // Placeholder types - no substitution needed (they're resolved separately)
            Placeholder { .. } => ty.clone(),

            // ExtensibleRecord - substitute in fields
            ExtensibleRecord { fields, row_var } => {
                let new_fields = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_term_in_type(v, var_name, term)))
                    .collect();
                Type::ExtensibleRecord {
                    fields: new_fields,
                    row_var: *row_var,
                }
            }

            // CapabilityRestricted - substitute in base type
            CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(self.substitute_term_in_type_impl(base, var_name, term, next_depth)),
                capabilities: capabilities.clone(),
            },

            // Unknown type - no inner types to substitute
            Unknown => ty.clone(),

            // DynProtocol - substitute in associated type bindings
            DynProtocol { bounds, bindings } => Type::DynProtocol {
                bounds: bounds.clone(),
                bindings: bindings
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            self.substitute_term_in_type_impl(v, var_name, term, next_depth),
                        )
                    })
                    .collect(),
            },
        }
    }

    /// Substitute a term for a variable in an EqTerm expression.
    ///

    /// This is needed for equality types where the lhs/rhs may reference
    /// dependent parameters.
    fn substitute_term_in_eq_term(
        &self,
        eq_term: &crate::ty::EqTerm,
        var_name: &Text,
        replacement: &crate::ty::EqTerm,
    ) -> crate::ty::EqTerm {
        use crate::ty::EqTerm;

        match eq_term {
            EqTerm::Var(name) => {
                if name == var_name {
                    replacement.clone()
                } else {
                    eq_term.clone()
                }
            }

            EqTerm::Const(_) => eq_term.clone(),

            EqTerm::App { func, args } => {
                let new_func = self.substitute_term_in_eq_term(func, var_name, replacement);
                let new_args: List<EqTerm> = args
                    .iter()
                    .map(|arg| self.substitute_term_in_eq_term(arg, var_name, replacement))
                    .collect();
                EqTerm::App {
                    func: Box::new(new_func),
                    args: new_args,
                }
            }

            EqTerm::Lambda { param, body } => {
                // If lambda parameter shadows our variable, don't substitute in body
                if param == var_name {
                    eq_term.clone()
                } else {
                    let new_body = self.substitute_term_in_eq_term(body, var_name, replacement);
                    EqTerm::Lambda {
                        param: param.clone(),
                        body: Box::new(new_body),
                    }
                }
            }

            EqTerm::Proj { pair, component } => {
                let new_pair = self.substitute_term_in_eq_term(pair, var_name, replacement);
                EqTerm::Proj {
                    pair: Box::new(new_pair),
                    component: *component,
                }
            }

            EqTerm::Refl(inner) => {
                let new_inner = self.substitute_term_in_eq_term(inner, var_name, replacement);
                EqTerm::Refl(Box::new(new_inner))
            }

            EqTerm::J {
                proof,
                motive,
                base,
            } => {
                let new_proof = self.substitute_term_in_eq_term(proof, var_name, replacement);
                let new_motive = self.substitute_term_in_eq_term(motive, var_name, replacement);
                let new_base = self.substitute_term_in_eq_term(base, var_name, replacement);
                EqTerm::J {
                    proof: Box::new(new_proof),
                    motive: Box::new(new_motive),
                    base: Box::new(new_base),
                }
            }
        }
    }

    /// Project a component from a Sigma type.
    ///

    /// Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma — Sigma Types (Dependent Pairs)
    ///

    /// Given a Sigma type `(x: A, B(x))`:
    /// - First projection returns type `A`
    /// - Second projection returns type `B(fst(pair))` (depends on first component)
    ///

    /// # Parameters
    /// - `sigma_type`: The Sigma type to project from
    /// - `component`: Which component to project (First or Second)
    /// - `pair_term`: The term representing the pair value
    ///

    /// # Returns
    /// The type of the projected component.
    fn project_sigma_type(
        &self,
        sigma_type: &Type,
        component: crate::ty::ProjComponent,
        pair_term: &crate::ty::EqTerm,
    ) -> Type {
        match sigma_type {
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => match component {
                crate::ty::ProjComponent::Fst => (**fst_type).clone(),
                crate::ty::ProjComponent::Snd => {
                    // The second type depends on the first value
                    // Substitute fst(pair) for the first component name
                    let fst_term = crate::ty::EqTerm::Proj {
                        pair: Box::new(pair_term.clone()),
                        component: crate::ty::ProjComponent::Fst,
                    };
                    self.substitute_term_in_type(snd_type, fst_name, &fst_term)
                }
            },
            _ => {
                // Not a Sigma type, return as-is (caller should verify this doesn't happen)
                sigma_type.clone()
            }
        }
    }

    /// Normalize a dependent type by reducing type-level computations.
    ///

    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 - Type-Level Computation
    ///

    /// This extends the basic normalize_type to also perform:
    /// - Beta reduction for Pi type applications
    /// - Sigma type projections
    /// - Equality type simplifications
    ///

    /// # Examples
    /// ```ignore
    /// // Pi application: ((n: Nat) -> Vec n Int) 5 => Vec 5 Int
    /// // Sigma projection: (x: Int, Vec x Bool).fst => Int
    /// ```
    ///

    /// # Parameters
    /// - `ty`: The dependent type to normalize
    ///

    /// # Returns
    /// The normalized type with type-level computations reduced.
    pub fn normalize_dependent_type(&self, ty: &Type) -> Type {
        self.normalize_dependent_type_impl(ty, 0)
    }

    /// Internal implementation of dependent type normalization with depth tracking.
    fn normalize_dependent_type_impl(&self, ty: &Type, depth: usize) -> Type {
        const MAX_DEPTH: usize = 50;
        if depth > MAX_DEPTH {
            return ty.clone();
        }

        // First, apply the standard normalization
        let normalized = self.normalize_type_impl(ty, depth);

        // Then, check for dependent type reductions
        use Type::*;
        match &normalized {
            // For Pi types, check if we can beta-reduce
            // This would require tracking applications in the type system
            // For now, we just normalize the components
            Pi {
                param_name,
                param_type,
                return_type,
            } => Pi {
                param_name: param_name.clone(),
                param_type: Box::new(self.normalize_dependent_type_impl(param_type, depth + 1)),
                return_type: Box::new(self.normalize_dependent_type_impl(return_type, depth + 1)),
            },

            // For Sigma types, normalize both components
            Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Sigma {
                fst_name: fst_name.clone(),
                fst_type: Box::new(self.normalize_dependent_type_impl(fst_type, depth + 1)),
                snd_type: Box::new(self.normalize_dependent_type_impl(snd_type, depth + 1)),
            },

            // For Eq types, try to simplify if both sides are equal
            Eq {
                ty: eq_ty,
                lhs,
                rhs,
            } => {
                // Normalize the type
                let norm_ty = self.normalize_dependent_type_impl(eq_ty, depth + 1);

                // Try to simplify: if lhs == rhs syntactically, this is reflexivity
                // For now, we just return the normalized form
                Eq {
                    ty: Box::new(norm_ty),
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                }
            }

            // For all other types, the standard normalization is sufficient
            _ => normalized,
        }
    }

    /// Evaluate a type-level expression to weak head normal form (WHNF).
    ///

    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 - Type-Level Computation
    ///

    /// WHNF means we reduce until the head of the expression is a constructor
    /// or a stuck term (variable/unknown). This is sufficient for type checking
    /// and faster than full normalization.
    ///

    /// # Parameters
    /// - `ty`: The type expression to evaluate
    ///

    /// # Returns
    /// The type in weak head normal form.
    pub fn eval_type_to_whnf(&self, ty: &Type) -> Type {
        self.eval_type_to_whnf_impl(ty, 0)
    }

    /// Internal implementation of WHNF evaluation with depth tracking.
    fn eval_type_to_whnf_impl(&self, ty: &Type, depth: usize) -> Type {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return ty.clone();
        }

        use Type::*;
        match ty {
            // Type variables are stuck (unification will resolve them)
            // We don't evaluate them here since the unifier handles variable resolution
            Var(_) => ty.clone(),

            // Named types might be aliases - resolve them
            Named { path, args } => {
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(id) = &path.segments[0]
                    && let Maybe::Some(underlying_ty) = self.ctx.lookup_type(id.name.as_str())
                {
                    // Apply any type arguments and evaluate
                    if args.is_empty() {
                        return self.eval_type_to_whnf_impl(underlying_ty, depth + 1);
                    }
                }
                ty.clone()
            }

            // For Pi/Sigma/Eq, we're already in WHNF (they're type constructors)
            Pi { .. } | Sigma { .. } | Eq { .. } => ty.clone(),

            // Type application might beta-reduce
            TypeApp { constructor, args } => {
                let eval_constructor = self.eval_type_to_whnf_impl(constructor, depth + 1);

                // If constructor is a type lambda, beta-reduce
                // For now, we just return it as-is since we don't have explicit type lambdas yet
                TypeApp {
                    constructor: Box::new(eval_constructor),
                    args: args.clone(),
                }
            }

            // All other types are already in WHNF
            _ => ty.clone(),
        }
    }

    /// Check if a type expression is stuck (cannot be reduced further).
    ///

    /// A type is stuck if:
    /// - It's a type variable with no binding
    /// - It's an application of a stuck type
    /// - It's waiting for more information to reduce
    ///

    /// This is useful for determining when type checking needs to wait
    /// for more constraints to be solved.
    pub fn is_type_stuck(&self, ty: &Type) -> bool {
        use Type::*;
        match ty {
            // Type variables are always stuck (waiting for unification)
            Var(_) => true,

            Named { path, .. } => {
                // Check if it's a resolvable alias
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(id) = &path.segments[0]
                {
                    return self.ctx.lookup_type(id.name.as_str()).is_none();
                }
                false
            }

            TypeApp { constructor, .. } => self.is_type_stuck(constructor),

            // Type constructors are not stuck
            _ => false,
        }
    }

    /// Check for ambiguous conversions and emit E0204 if multiple paths exist.
    ///

    /// E0204 Multiple conversion paths: when try (?) operator finds multiple From implementations for error conversion, requiring explicit disambiguation — E0204 Multiple conversion paths
    ///

    /// # Parameters
    ///

    /// - `from_type`: Source error type
    /// - `to_type`: Target error type
    /// - `span`: Location of the ? operator for diagnostics
    ///

    /// # Returns
    ///

    /// - `Ok(())` if there is exactly one conversion path or no ambiguity
    /// - `Err(TypeError::MultipleConversionPaths)` if multiple paths exist
    ///

    /// # Visibility
    ///

    /// This method is `pub` to enable external testing but is not part of the stable API.
    pub fn check_for_ambiguous_conversions(
        &self,
        from_type: &Type,
        to_type: &Type,
        span: Span,
    ) -> Result<()> {
        let paths = self.find_all_conversion_paths(from_type, to_type);

        if paths.len() > 1 {
            // Format paths for diagnostic
            let path_descriptions: List<verum_common::Text> = paths
                .iter()
                .map(|path| {
                    if path.steps.is_empty() {
                        // This shouldn't happen, but handle it gracefully
                        format!(
                            "{} -> {} (unknown path)",
                            from_type.to_text(),
                            to_type.to_text()
                        )
                        .into()
                    } else if path.steps.len() == 1 {
                        // Direct conversion
                        format!("{} -> {} (direct)", from_type.to_text(), to_type.to_text()).into()
                    } else {
                        // Indirect conversion through intermediate types
                        let mut path_str = from_type.to_text().to_string();
                        for step in path.steps.iter() {
                            path_str.push_str(" -> ");
                            path_str.push_str(step.to_type.to_text().as_ref());
                        }
                        format!("{} (indirect)", path_str).into()
                    }
                })
                .collect();

            let path_descriptions_list: List<verum_common::Text> =
                path_descriptions.iter().cloned().collect();
            let from_ty_text = from_type.to_text();
            let to_ty_text = to_type.to_text();
            let diag = verum_diagnostics::e0204_multiple_conversion_paths(
                span_to_line_col(span),
                &from_ty_text,
                &to_ty_text,
                &path_descriptions_list,
            );

            return Err(TypeError::MultipleConversionPaths {
                from_type: from_type.to_text(),
                to_type: to_type.to_text(),
                paths: path_descriptions,
                span,
                diagnostic: diag,
            });
        }

        Ok(())
    }

    /// Check if a cast from `from_ty` to `to_ty` is safe.
    ///

    /// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — (Integer Hierarchy), Section 6 (Reference Safety)
    ///

    /// # Cast Rules
    ///

    /// 1. **Subtype casts** (implicit, always safe):
    ///  - `T{p1} → T{p2}` if p1 => p2 (refinement strengthening)
    ///  - `&checked T → &T` (adds CBGR checks)
    ///  - `&unsafe T → &T` (adds CBGR checks)
    ///

    /// 2. **Numeric casts** (explicit, checked):
    ///  - Int → i8/i16/i32/i64/i128 (range check)
    ///  - i32 → i64 (widening, safe)
    ///  - i64 → i32 (narrowing, runtime check)
    ///  - Int → Float (precision loss warning)
    ///

    /// 3. **Protocol casts** (explicit):
    ///  - T → &dyn Protocol if T implements Protocol
    ///

    /// 4. **FORBIDDEN casts**:
    ///  - &T → &checked T (cannot invent proof)
    ///  - &T → &unsafe T (requires @unsafe context)
    ///  - Unrelated types (e.g., Int → Text)
    ///

    /// # Safety
    ///

    /// This function enforces Verum's zero-tolerance policy:
    /// - **NO undefined behavior** - all casts are checked
    /// - **NO false negatives** - invalid casts are rejected
    /// - **NO data races** - reference safety preserved
    pub fn check_cast(&mut self, from_ty: &Type, to_ty: &Type, span: Span) -> Result<()> {
        use Type::*;

        // Strip refinement types for cast checking — casting a refined type
        // should be equivalent to casting its base type (e.g., Int{>= 0} as Float ≡ Int as Float)
        let from_ty = match from_ty {
            Refined { base, .. } => base.as_ref(),
            other => other,
        };
        let to_ty = match to_ty {
            Refined { base, .. } => base.as_ref(),
            other => other,
        };
        // Also normalize Named types that are refinement aliases
        let from_ty = &self.normalize_type(from_ty);
        let from_ty = match from_ty {
            Refined { base, .. } => base.as_ref(),
            other => other,
        };
        let to_ty = &self.normalize_type(to_ty);
        let to_ty = match to_ty {
            Refined { base, .. } => base.as_ref(),
            other => other,
        };

        // Rule 0: Auto-dereference — if source is &T or &mut T and target is T, strip the reference
        // This allows implicit deref in cast/assignment contexts: &Int → Int, &mut Int → Int
        // Works recursively: &&T → &T → T
        let from_ty = {
            let mut t = from_ty;
            loop {
                match t {
                    Reference { inner, .. }
                    | CheckedReference { inner, .. }
                    | UnsafeReference { inner, .. }
                        if !matches!(
                            to_ty,
                            Reference { .. }
                                | CheckedReference { .. }
                                | UnsafeReference { .. }
                                | Pointer { .. }
                                | VolatilePointer { .. }
                        ) =>
                    {
                        t = inner.as_ref();
                    }
                    _ => break,
                }
            }
            t
        };
        // Rule 1: Subtype relationship (always safe)
        if self.subtyping.is_subtype(from_ty, to_ty) {
            return Ok(());
        }

        match (from_ty, to_ty) {
            // Char <-> UInt8/Byte coercion (both are 8-bit values)
            (Char, Named { path, .. }) => {
                let name = path
                    .segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");
                if matches!(name, "UInt8" | "Byte" | "u8") {
                    Ok(())
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    })
                }
            }
            (Named { path, .. }, Char) => {
                let name = path
                    .segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");
                if matches!(name, "UInt8" | "Byte" | "u8") {
                    Ok(())
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    })
                }
            }

            // Numeric casts
            (Int | Named { .. }, Int | Named { .. }) => {
                // Check if both are integer types
                self.check_numeric_cast(from_ty, to_ty, span)
            }

            // Char <-> Int casts (Char is a Unicode codepoint, which is an integer)
            // Char type: Unicode scalar value (0..=0x10FFFF excluding surrogates), distinct from u32 (0..=0x10FFFF)
            (Char, Int) => {
                // Char -> Int: always safe, returns Unicode codepoint
                Ok(())
            }
            (Int, Char) => {
                // Int -> Char: allowed but may fail at runtime if not a valid codepoint
                // Runtime will check 0..=0x10FFFF and not surrogate pairs
                Ok(())
            }

            // Bool <-> Int casts (common in many languages)
            // Spec: Bool -> Int: false = 0, true = 1
            // Spec: Int -> Bool: 0 = false, non-zero = true
            (Bool, Int) => {
                // Bool -> Int: always safe (false -> 0, true -> 1)
                Ok(())
            }
            (Int, Bool) => {
                // Int -> Bool: allowed (0 -> false, non-zero -> true)
                Ok(())
            }
            (Bool, Float) => {
                // Bool -> Float: always safe (false -> 0.0, true -> 1.0)
                Ok(())
            }
            (Float, Bool) => {
                // Float -> Bool: allowed (0.0 -> false, non-zero -> true)
                Ok(())
            }

            // Float conversions
            (Int | Named { .. }, Float) => {
                // Int → Float: allowed with precision loss warning
                // Spec: Numeric conversions - precision loss detection
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message(format!(
                            "implicit cast from `{}` to `{}` may lose precision\n  \
                             help: use `.to_f64()` explicitly if this is intentional\n  \
                             note: integers larger than 2^53 cannot be represented exactly in IEEE 754 doubles",
                            from_ty, to_ty
                        ))
                        .build()
                );
                Ok(())
            }

            (Float, Int | Named { .. }) => {
                // Float → Int: allowed but emits a warning about truncation
                // Explicit casts: "as" keyword for type conversions that may lose precision (e.g., Float as Int truncates)
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message(format!(
                            "cast from `{}` to `{}` truncates toward zero\n  \
                             help: use `.trunc()`, `.floor()`, `.ceil()`, or `.round()` for explicit rounding control",
                            from_ty, to_ty
                        ))
                        .build()
                );
                Ok(())
            }

            // Slice/Array reference to struct reference casts (memory reinterpretation)
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Low-level memory operations
            // Allows: &[Byte] as &T, &mut [Byte] as &mut T for treating raw bytes as struct
            // This must come BEFORE the general reference-to-reference handler
            (
                Reference {
                    inner: from_inner,
                    mutable: from_mut,
                },
                Reference {
                    inner: to_inner,
                    mutable: to_mut,
                },
            ) if matches!(from_inner.as_ref(), Type::Slice { .. } | Type::Array { .. })
                && !matches!(to_inner.as_ref(), Type::Slice { .. } | Type::Array { .. }) =>
            {
                // Mutability must match or be a downgrade (mut -> immut ok, immut -> mut not ok)
                if *to_mut && !*from_mut {
                    return Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "cannot cast immutable slice reference to mutable struct reference"
                            .to_text(),
                        span,
                    });
                }
                // Memory reinterpretation should be in unsafe context for full safety
                if !self.in_unsafe_context {
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "casting slice reference to struct reference reinterprets memory layout\n  \
                                 help: ensure the slice contains valid data for `{}`; consider @unsafe block",
                                to_inner
                            ))
                            .build()
                    );
                }
                Ok(())
            }

            // UnsafeReference slice to struct casts
            (
                UnsafeReference {
                    inner: from_inner,
                    mutable: from_mut,
                },
                UnsafeReference {
                    inner: to_inner,
                    mutable: to_mut,
                },
            ) if matches!(from_inner.as_ref(), Type::Slice { .. } | Type::Array { .. })
                && !matches!(to_inner.as_ref(), Type::Slice { .. } | Type::Array { .. }) =>
            {
                if *to_mut && !*from_mut {
                    return Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "cannot cast immutable slice reference to mutable struct reference"
                            .to_text(),
                        span,
                    });
                }
                Ok(())
            }

            // CheckedReference slice to struct casts
            (
                CheckedReference {
                    inner: from_inner,
                    mutable: from_mut,
                },
                CheckedReference {
                    inner: to_inner,
                    mutable: to_mut,
                },
            ) if matches!(from_inner.as_ref(), Type::Slice { .. } | Type::Array { .. })
                && !matches!(to_inner.as_ref(), Type::Slice { .. } | Type::Array { .. }) =>
            {
                if *to_mut && !*from_mut {
                    return Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "cannot cast immutable slice reference to mutable struct reference"
                            .to_text(),
                        span,
                    });
                }
                Ok(())
            }

            // Int to reference cast (for FFI/null pointer creation)
            // Allows: 0 as &PageHeader, address as &T
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — FFI interoperability
            (Int, Reference { .. })
            | (Int, CheckedReference { .. })
            | (Int, UnsafeReference { .. }) => {
                if !self.in_unsafe_context {
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message("casting integer to reference - ensure valid memory address and alignment")
                            .build()
                    );
                }
                Ok(())
            }

            // Reference casts - handled through subtyping
            // &unsafe T → &checked T → &T allowed
            // The opposite direction is forbidden
            (Reference { inner: inner1, .. }, Reference { inner: inner2, .. })
            | (CheckedReference { inner: inner1, .. }, CheckedReference { inner: inner2, .. }) => {
                // Allow when target is dyn Protocol (protocol object cast)
                // Allow when either inner is a type variable
                if matches!(inner2.as_ref(), Type::DynProtocol { .. })
                    || matches!(inner1.as_ref(), Type::Var(_))
                    || matches!(inner2.as_ref(), Type::Var(_))
                {
                    return Ok(());
                }
                // Check inner types are compatible - allow with warning if not
                if !self.subtyping.is_subtype(inner1, inner2) {
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "reinterpret cast between reference types: `{}` to `{}`",
                                from_ty, to_ty
                            ))
                            .build(),
                    );
                    return Ok(());
                }
                Ok(())
            }
            // Unsafe reference to unsafe reference: allow arbitrary type reinterpretation
            // This is the purpose of &unsafe T - to enable low-level memory operations
            // like type punning, pointer casting, etc.
            (
                UnsafeReference {
                    mutable: from_mut, ..
                },
                UnsafeReference {
                    mutable: to_mut, ..
                },
            ) => {
                // Only check mutability: can't cast &unsafe T to &unsafe mut T
                if *to_mut && !*from_mut {
                    return Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "cannot cast immutable unsafe reference to mutable".to_text(),
                        span,
                    });
                }
                Ok(())
            }

            // Upcast from unsafe/checked to managed (safe)
            (UnsafeReference { .. }, CheckedReference { .. })
            | (UnsafeReference { .. }, Reference { .. })
            | (CheckedReference { .. }, Reference { .. }) => {
                // These are always safe - adding checks
                Ok(())
            }

            // Downcast from managed to checked/unsafe
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Reference tier downcasts
            // &T → &unsafe T: allowed in @unsafe context (tier 0 to tier 2)
            // &T → &unsafe T: allowed with explicit `as` cast (tier 0 to tier 2)
            // &checked T → &unsafe T: allowed with explicit `as` cast (tier 1 to tier 2)
            // The `as` keyword in the cast expression IS the explicit opt-in for unsafe.
            // @unsafe blocks are for operations with potential UB (e.g., raw pointer deref),
            // not for explicit type casts that simply change the reference tier.
            (
                Reference {
                    inner: _from_inner, ..
                },
                UnsafeReference {
                    inner: _to_inner, ..
                },
            )
            | (
                CheckedReference {
                    inner: _from_inner, ..
                },
                UnsafeReference {
                    inner: _to_inner, ..
                },
            ) => {
                // Casting to &unsafe T allows arbitrary type reinterpretation.
                // This is the purpose of unsafe references - to enable low-level
                // memory operations like type punning, pointer casting, etc.
                // The "unsafe" keyword in &unsafe T IS the opt-in for potentially
                // type-unsafe operations.
                Ok(())
            }
            (Reference { .. }, CheckedReference { .. }) => {
                // Downcast to checked tier requires proof, not just unsafe context
                Err(TypeError::InvalidCast {
                    from: from_ty.to_text(),
                    to: to_ty.to_text(),
                    reason: "cannot downcast references to checked tier - requires proof of static lifetimes".to_text(),
                    span,
                })
            }

            // Reference to raw pointer casts - allowed in unsafe context
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Raw pointer interoperability
            // &T → *const T: always allowed (reference to immutable pointer)
            // &T → *mut T: only if reference is mutable (not checked here, checked at use site)
            // &checked T → *const T / *mut T: also allowed
            // &unsafe T → *const T / *mut T: also allowed
            (
                Reference {
                    inner: ref_inner, ..
                },
                Pointer {
                    inner: ptr_inner, ..
                },
            )
            | (
                CheckedReference {
                    inner: ref_inner, ..
                },
                Pointer {
                    inner: ptr_inner, ..
                },
            ) => {
                // Check that inner types are compatible
                // Allow when either inner type is a type variable (will be resolved later)
                if self.subtyping.is_subtype(ref_inner, ptr_inner)
                    || self.subtyping.is_subtype(ptr_inner, ref_inner)
                    || matches!(ref_inner.as_ref(), Type::Var(_))
                    || matches!(ptr_inner.as_ref(), Type::Var(_))
                {
                    Ok(())
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "reference and pointer inner types are not compatible".to_text(),
                        span,
                    })
                }
            }

            // Unsafe reference to raw pointer cast - relaxed inner type check
            // Both are unsafe/unmanaged, so reinterpret casts are allowed
            // &unsafe Byte → *mut Int, &unsafe T → *const U, etc.
            (
                UnsafeReference {
                    inner: ref_inner, ..
                },
                Pointer {
                    inner: ptr_inner, ..
                },
            ) => {
                if self.subtyping.is_subtype(ref_inner, ptr_inner)
                    || self.subtyping.is_subtype(ptr_inner, ref_inner)
                {
                    Ok(())
                } else {
                    // Incompatible inner types but both are unsafe - allow with warning
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "casting between unsafe reference and pointer with incompatible inner types: `{}` to `{}`",
                                from_ty, to_ty
                            ))
                            .build()
                    );
                    Ok(())
                }
            }

            // Raw pointer to raw pointer casts - allowed if inner types compatible
            (
                Pointer {
                    inner: from_inner, ..
                },
                Pointer {
                    inner: to_inner, ..
                },
            ) => {
                // Pointer casts are always allowed (like in C/Rust unsafe)
                // This is inherently unsafe but allowed at the type level
                if self.subtyping.is_subtype(from_inner, to_inner)
                    || self.subtyping.is_subtype(to_inner, from_inner)
                {
                    Ok(())
                } else {
                    // Even incompatible pointer casts are allowed (like void* casts)
                    // but emit a warning
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "casting between incompatible pointer types: `{}` to `{}`",
                                from_ty, to_ty
                            ))
                            .build(),
                    );
                    Ok(())
                }
            }

            // Raw pointer to unsafe reference cast - allowed (both are unsafe/unmanaged)
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Raw pointer interoperability
            // *const T → &unsafe T, *mut T → &unsafe T: allowed with explicit `as` cast
            (
                Pointer {
                    inner: ptr_inner, ..
                },
                UnsafeReference {
                    inner: ref_inner, ..
                },
            ) => {
                if self.subtyping.is_subtype(ptr_inner, ref_inner)
                    || self.subtyping.is_subtype(ref_inner, ptr_inner)
                {
                    Ok(())
                } else {
                    // Incompatible inner types but both are unsafe level - allow with warning
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "casting between pointer and unsafe reference with incompatible inner types: `{}` to `{}`",
                                from_ty, to_ty
                            ))
                            .build()
                    );
                    Ok(())
                }
            }

            // Integer to raw pointer cast (for FFI/low-level code)
            (Int, Pointer { .. }) => {
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message("casting integer to pointer is inherently unsafe")
                        .build(),
                );
                Ok(())
            }

            // Unsafe reference to integer cast (for address extraction)
            (UnsafeReference { .. }, Int) => Ok(()),

            // Unsafe reference to USize/ISize cast (for address extraction)
            // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Low-level memory operations
            (UnsafeReference { .. }, Named { path, .. }) => {
                let name = self.path_to_string(path);
                match name.as_str() {
                    "USize" | "ISize" | "usize" | "isize" | "UInt64" | "Int64" => Ok(()),
                    _ => Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    }),
                }
            }

            // Reference to USize/ISize cast (for address extraction)
            (Reference { .. }, Named { path, .. })
            | (CheckedReference { .. }, Named { path, .. }) => {
                let name = self.path_to_string(path);
                match name.as_str() {
                    "USize" | "ISize" | "usize" | "isize" | "UInt64" | "Int64" => Ok(()),
                    _ => Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    }),
                }
            }

            // Raw pointer to integer cast (for address manipulation)
            (Pointer { .. }, Int) => Ok(()),

            // Raw pointer to USize/ISize cast (for address manipulation)
            (Pointer { .. }, Named { path, .. }) => {
                let name = self.path_to_string(path);
                match name.as_str() {
                    "USize" | "ISize" | "usize" | "isize" | "UInt64" | "Int64" => Ok(()),
                    _ => Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    }),
                }
            }

            // Volatile pointer casts (same rules as raw pointers)
            (Int, VolatilePointer { .. }) => Ok(()),
            (VolatilePointer { .. }, Int) => Ok(()),
            (VolatilePointer { .. }, Named { path, .. }) => {
                let name = self.path_to_string(path);
                match name.as_str() {
                    "USize" | "ISize" | "usize" | "isize" | "UInt64" | "Int64" => Ok(()),
                    _ => Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    }),
                }
            }
            (Named { path, .. }, VolatilePointer { .. }) => {
                let name = self.path_to_string(path);
                match name.as_str() {
                    "USize" | "ISize" | "usize" | "isize" | "UInt64" | "Int64" => Ok(()),
                    _ => Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    }),
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Volatile pointer casts (MMIO support)
            // Low-level features: SIMD intrinsics, unsafe blocks, raw pointers, inline assembly — Section 3 - Volatile pointer types
            // ═══════════════════════════════════════════════════════════════

            // Reference to volatile pointer: &T → *volatile T, &mut T → *volatile mut T
            (
                Reference {
                    inner: ref_inner, ..
                },
                VolatilePointer {
                    inner: vol_inner, ..
                },
            )
            | (
                CheckedReference {
                    inner: ref_inner, ..
                },
                VolatilePointer {
                    inner: vol_inner, ..
                },
            )
            | (
                UnsafeReference {
                    inner: ref_inner, ..
                },
                VolatilePointer {
                    inner: vol_inner, ..
                },
            ) => {
                if self.subtyping.is_subtype(ref_inner, vol_inner)
                    || self.subtyping.is_subtype(vol_inner, ref_inner)
                {
                    Ok(())
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "reference and volatile pointer inner types are not compatible"
                            .to_text(),
                        span,
                    })
                }
            }

            // Volatile pointer to volatile pointer (reinterpretation)
            (VolatilePointer { .. }, VolatilePointer { .. }) => Ok(()),

            // Raw pointer ↔ volatile pointer (interconversion)
            (Pointer { .. }, VolatilePointer { .. }) | (VolatilePointer { .. }, Pointer { .. }) => {
                Ok(())
            }

            // Protocol object casts
            (from_ty, Named { .. }) if self.is_protocol_type(to_ty) => {
                // Check if from_ty implements the protocol
                // Spec: Protocol constraints - implementation checking
                match self.extract_protocol_name(to_ty) {
                    Maybe::Some(protocol_name) => {
                        // Check if from_ty implements this protocol
                        if !self
                            .protocol_checker
                            .read()
                            .implements_protocol(from_ty, protocol_name.as_str())
                        {
                            return Err(TypeError::ProtocolNotSatisfied {
                                ty: from_ty.to_text(),
                                protocol: protocol_name,
                                span,
                            });
                        }
                        Ok(())
                    }
                    Maybe::None => Ok(()),
                }
            }

            // Refinement erasure (T{p} → T)
            (Refined { base, .. }, to_ty) if self.subtyping.is_subtype(base, to_ty) => Ok(()),

            // Refinement strengthening (T → T{p})
            (from_ty, Refined { base, .. }) if self.subtyping.is_subtype(from_ty, base) => {
                // This requires proving the refinement predicate
                // For casts, this is not allowed - use runtime check instead
                Err(TypeError::InvalidCast {
                    from: from_ty.to_text(),
                    to: to_ty.to_text(),
                    reason: "cannot cast to refined type - use .try_from() for runtime check or prove statically".to_text(),
                    span,
                })
            }

            // Generic type casts - allow when base types match and source has unresolved type vars
            // This handles cases like: None as Maybe<Int>, Some(x) as Maybe<Int>
            // Spec: Type annotation casts for specifying type parameters
            (
                Generic {
                    name: from_name,
                    args: from_args,
                },
                Generic {
                    name: to_name,
                    args: to_args,
                },
            ) if from_name == to_name && from_args.len() == to_args.len() => {
                // Same generic type - check if args are compatible or can be unified
                // Allow if: from_args contains type variables, or from_args are subtypes of to_args
                let mut compatible = true;
                for (from_arg, to_arg) in from_args.iter().zip(to_args.iter()) {
                    if !self.types_compatible_for_cast(from_arg, to_arg) {
                        compatible = false;
                        break;
                    }
                }
                if compatible {
                    Ok(())
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "generic type arguments are not compatible".to_text(),
                        span,
                    })
                }
            }

            // Variant type casts - allow when variant names match and contained types are compatible
            // This handles cases like: None as Maybe<Int> where Maybe is a sum type None | Some(T)
            (Variant(from_variants), Variant(to_variants))
                if from_variants.len() == to_variants.len() =>
            {
                // Check that all variant names match and their types are compatible
                let mut compatible = true;
                for (name, from_inner) in from_variants.iter() {
                    if let Some(to_inner) = to_variants.get(name) {
                        if !self.types_compatible_for_cast(from_inner, to_inner) {
                            compatible = false;
                            break;
                        }
                    } else {
                        // Variant name doesn't exist in target
                        compatible = false;
                        break;
                    }
                }
                if compatible {
                    Ok(())
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "variant types are not compatible".to_text(),
                        span,
                    })
                }
            }

            // Variant to Generic casts - stdlib-agnostic handling
            // Allows: Err("error") as Result<Int, Text>, None as Maybe<Int>
            // For partial variants, we find the target type's full variant signature and check
            // that the source's variant names are a valid subset.
            (
                Variant(from_variants),
                Generic {
                    name: to_name,
                    args: to_args,
                },
            ) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG check_cast Variant->Generic] from_variants={:?}, to_name={}, to_args={:?}",
                // from_variants.keys().collect::<Vec<_>>(), to_name, to_args);
                // First, find a registered signature that maps to the target type name
                // and contains ALL of the source's variant names
                let source_variant_names: std::collections::HashSet<&str> =
                    from_variants.keys().map(|k| k.as_str()).collect();

                let mut found_matching_type = false;
                for (sig, registered_name) in self.variant_type_names.iter() {
                    if registered_name.as_str() == to_name.as_str() {
                        found_matching_type = true;
                        // Extract variant names from the signature "Variant(A|B|C)"
                        if let Some(inner) = sig
                            .as_str()
                            .strip_prefix("Variant(")
                            .and_then(|s| s.strip_suffix(")"))
                        {
                            let target_variant_names: std::collections::HashSet<&str> =
                                inner.split('|').collect();
                            // Check if source variants are a subset of target variants
                            if source_variant_names.is_subset(&target_variant_names) {
                                // Variant names match! Now check payload compatibility.
                                // For partial variant casts, we just verify that present payloads
                                // are compatible with at least one type arg.
                                let mut compatible = true;
                                for (_name, payload) in from_variants.iter() {
                                    if *payload == Type::Unit {
                                        continue;
                                    }
                                    let payload_compatible = to_args
                                        .iter()
                                        .any(|arg| self.types_compatible_for_cast(payload, arg));
                                    if !payload_compatible {
                                        compatible = false;
                                        break;
                                    }
                                }
                                if compatible {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                // If we found the type but variants didn't match, give specific error
                if found_matching_type {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "variant names or payloads are not compatible".to_text(),
                        span,
                    })
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    })
                }
            }

            // Generic to Variant casts - stdlib-agnostic handling (reverse direction)
            // Allows casting from Generic type to its expanded Variant form
            (
                Generic {
                    name: from_name,
                    args: from_args,
                },
                Variant(to_variants),
            ) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG check_cast Generic->Variant] from_name={}, from_args={:?}, to_variants={:?}",
                // from_name, from_args, to_variants.keys().collect::<Vec<_>>());
                // Generate signature from target variant structure
                let signature = Self::variant_type_signature(&Type::Variant(to_variants.clone()));
                if let Some(sig) = signature {
                    // Look up registered type name for this variant structure
                    if let Some(registered_name) = self.variant_type_names.get(&sig) {
                        if registered_name.as_str() == from_name.as_str() {
                            // Type names match! Check compatibility using the same approach.
                            let mut compatible = true;
                            for (_name, payload) in to_variants.iter() {
                                if *payload == Type::Unit {
                                    continue;
                                }
                                let payload_compatible = from_args
                                    .iter()
                                    .any(|arg| self.types_compatible_for_cast(arg, payload));
                                if !payload_compatible {
                                    compatible = false;
                                    break;
                                }
                            }
                            if compatible {
                                return Ok(());
                            }
                        }
                    }
                }
                Err(TypeError::InvalidCast {
                    from: from_ty.to_text(),
                    to: to_ty.to_text(),
                    reason: "types are not compatible for casting".to_text(),
                    span,
                })
            }

            // Named to Variant casts - stdlib-agnostic handling
            // Allows: Result<Int, Text> (Named) to Ok(Int) | Err(Text) (Variant)
            (
                Named {
                    path: from_path,
                    args: from_args,
                },
                Variant(to_variants),
            ) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG check_cast Named->Variant] from_path={:?}, from_args={:?}, to_variants={:?}",
                // from_path, from_args, to_variants.keys().collect::<Vec<_>>());
                // Extract type name from path
                let from_name = self.path_to_string(from_path);
                // Generate signature from target variant structure
                let signature = Self::variant_type_signature(&Type::Variant(to_variants.clone()));
                if let Some(sig) = signature {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG check_cast Named->Variant] from_name={}, sig={}", from_name, sig);
                    // Look up registered type name for this variant structure
                    if let Some(registered_name) = self.variant_type_names.get(&sig) {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG check_cast Named->Variant] registered_name={}", registered_name);
                        if registered_name.as_str() == from_name.as_str() {
                            // Type names match! Check compatibility.
                            let mut compatible = true;
                            for (_name, payload) in to_variants.iter() {
                                if *payload == Type::Unit {
                                    continue;
                                }
                                let payload_compatible = from_args
                                    .iter()
                                    .any(|arg| self.types_compatible_for_cast(arg, payload));
                                if !payload_compatible {
                                    compatible = false;
                                    break;
                                }
                            }
                            if compatible {
                                return Ok(());
                            }
                        }
                    }
                }
                Err(TypeError::InvalidCast {
                    from: from_ty.to_text(),
                    to: to_ty.to_text(),
                    reason: "types are not compatible for casting".to_text(),
                    span,
                })
            }

            // Variant to Named casts - stdlib-agnostic handling
            // Allows: Err("error") (Variant) as Result<Int, Text> (Named)
            (
                Variant(from_variants),
                Named {
                    path: to_path,
                    args: to_args,
                },
            ) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG check_cast Variant->Named] from_variants={:?}, to_path={:?}, to_args={:?}",
                // from_variants.keys().collect::<Vec<_>>(), to_path, to_args);
                let to_name = self.path_to_string(to_path);
                // Find a registered signature that maps to the target type name
                let source_variant_names: std::collections::HashSet<&str> =
                    from_variants.keys().map(|k| k.as_str()).collect();

                let mut found_matching_type = false;
                for (sig, registered_name) in self.variant_type_names.iter() {
                    if registered_name.as_str() == to_name.as_str() {
                        found_matching_type = true;
                        if let Some(inner) = sig
                            .as_str()
                            .strip_prefix("Variant(")
                            .and_then(|s| s.strip_suffix(")"))
                        {
                            let target_variant_names: std::collections::HashSet<&str> =
                                inner.split('|').collect();
                            if source_variant_names.is_subset(&target_variant_names) {
                                // Check payload compatibility
                                let mut compatible = true;
                                for (_name, payload) in from_variants.iter() {
                                    if *payload == Type::Unit {
                                        continue;
                                    }
                                    let payload_compatible = to_args
                                        .iter()
                                        .any(|arg| self.types_compatible_for_cast(payload, arg));
                                    if !payload_compatible {
                                        compatible = false;
                                        break;
                                    }
                                }
                                if compatible {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                if found_matching_type {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "variant names or payloads are not compatible".to_text(),
                        span,
                    })
                } else {
                    Err(TypeError::InvalidCast {
                        from: from_ty.to_text(),
                        to: to_ty.to_text(),
                        reason: "types are not compatible for casting".to_text(),
                        span,
                    })
                }
            }

            // Type variable casts - if either side is unresolved, allow the cast.
            // These will be checked again after type inference resolves the variables.
            (Var(_), _) | (_, Var(_)) => Ok(()),

            // Array to integer cast - used in FFI for packing fixed-size arrays into register values
            (Array { .. }, Int) | (Array { .. }, Named { .. }) => {
                if self.in_unsafe_context {
                    Ok(())
                } else {
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message("casting array to integer reinterprets memory layout")
                            .build(),
                    );
                    Ok(())
                }
            }

            // USize/Named integer to unsafe reference cast - low-level pointer construction
            (Named { .. }, UnsafeReference { .. }) => {
                if self.in_unsafe_context {
                    Ok(())
                } else {
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message("casting integer type to unsafe reference - ensure valid memory address")
                            .build()
                    );
                    Ok(())
                }
            }

            // Named integer to reference cast - low-level pointer construction
            (Named { .. }, Reference { .. } | CheckedReference { .. }) => {
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message("casting named type to reference - ensure valid memory address")
                        .build(),
                );
                Ok(())
            }

            // Function pointer <-> Int casts (for signal handlers and FFI trampolines)
            (Type::Function { .. }, Int) | (Int, Type::Function { .. }) => {
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message("casting between function pointer and integer - ensure valid function address")
                        .build()
                );
                Ok(())
            }

            // Unit to unsafe reference cast (for FFI null pointer representations)
            (Unit, UnsafeReference { .. }) => {
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message(
                            "casting Unit to unsafe reference - likely represents null pointer",
                        )
                        .build(),
                );
                Ok(())
            }

            // Variant/Result to Int cast (for FFI error code extraction)
            (Variant(_), Int) => {
                self.emit_diagnostic(
                    DiagnosticBuilder::warning()
                        .message("casting variant type to integer - ensure valid conversion")
                        .build(),
                );
                Ok(())
            }

            // In unsafe contexts, allow all casts (raw pointer manipulation)
            _ if self.in_unsafe_context => Ok(()),

            // All other casts are invalid
            _ => Err(TypeError::InvalidCast {
                from: from_ty.to_text(),
                to: to_ty.to_text(),
                reason: "types are not compatible for casting".to_text(),
                span,
            }),
        }
    }

    /// Check numeric cast safety using IntegerHierarchy.
    ///

    /// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — Integer Type Hierarchy
    /// Verifies that numeric casts respect the integer hierarchy constraints.
    fn check_numeric_cast(&self, from_ty: &Type, to_ty: &Type, _span: Span) -> Result<()> {
        // Extract integer kind from type if possible
        let from_kind = self.extract_integer_kind(from_ty);
        let to_kind = self.extract_integer_kind(to_ty);

        match (from_kind, to_kind) {
            (Maybe::Some(from), Maybe::Some(to)) => {
                // Both are specific integer types - check subtyping relationship
                if self.integer_hierarchy.is_subtype(from, to) {
                    // Upcast is always safe
                    return Ok(());
                }
                // Downcast is allowed but will require runtime checking
                // The language will emit a warning or require explicit annotation
                Ok(())
            }
            _ => {
                // At least one is a generic Int type - allow the cast
                // Runtime checking will handle overflow concerns
                Ok(())
            }
        }
    }

    /// Extract integer kind from a named type
    fn extract_integer_kind(&self, ty: &Type) -> Maybe<crate::integer_hierarchy::IntegerKind> {
        if let Type::Named { path, .. } = ty
            && let Some(segment) = path.segments.first()
            && let verum_ast::ty::PathSegment::Name(ident) = segment
            && let Some(kind) =
                crate::integer_hierarchy::IntegerKind::from_name(ident.name.as_str())
        {
            return Maybe::Some(kind);
        }
        Maybe::None
    }

    /// Check if a type is a protocol type (protocol object).
    fn is_protocol_type(&self, ty: &Type) -> bool {
        if let Type::Named { path, .. } = ty {
            // Check if path starts with "dyn " to indicate protocol object
            if let Some(segment) = path.segments.first()
                && let verum_ast::ty::PathSegment::Name(ident) = segment
            {
                return ident.name.as_str().starts_with("dyn ");
            }
        }
        false
    }

    /// Extract protocol name from a protocol object type (e.g., "dyn Eq" -> "Eq")
    pub(super) fn extract_protocol_name(&self, ty: &Type) -> Maybe<Text> {
        if let Type::Named { path, .. } = ty
            && let Some(segment) = path.segments.first()
            && let verum_ast::ty::PathSegment::Name(ident) = segment
        {
            let name = ident.name.as_str();
            if name.starts_with("dyn ") {
                return Maybe::Some(verum_common::Text::from(&name[4..]));
            }
            // Also handle regular protocol names
            return Maybe::Some(verum_common::Text::from(name));
        }
        Maybe::None
    }

    /// Check if an expression is an lvalue (assignable location).
    ///

    /// Inline assembly output operands must be lvalues - variables, field accesses,
    /// or index expressions that can be assigned to.
    ///

    /// Low-level type operations: raw pointer casting, transmute, memory layout control
    pub(super) fn check_place_is_lvalue(&self, expr: &Expr, _ty: &Type) -> Result<()> {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Path expressions (variables) are lvalues
            ExprKind::Path(_) => Ok(()),
            // Field access on an lvalue is an lvalue
            ExprKind::Field { expr: object, .. } => self.check_place_is_lvalue(object, _ty),
            // Index access on an lvalue is an lvalue
            ExprKind::Index { expr: object, .. } => self.check_place_is_lvalue(object, _ty),
            // Dereference of a mutable reference is an lvalue
            ExprKind::Unary { op, expr: operand } if op.as_str() == "*" => {
                self.check_place_is_lvalue(operand, _ty)
            }
            // Everything else is not an lvalue
            _ => Err(TypeError::AsmOutputNotLvalue { span: expr.span }),
        }
    }

    /// Check if a type is compatible with inline assembly const operands.
    ///

    /// Const operands must be integer or pointer types - they represent
    /// compile-time constants embedded directly in the assembly.
    ///

    /// Low-level type operations: raw pointer casting, transmute, memory layout control
    pub(super) fn is_asm_const_compatible(&self, ty: &Type) -> bool {
        match ty {
            // Integer types
            Type::Named { path, .. } => {
                if let Some(segment) = path.segments.first()
                    && let verum_ast::ty::PathSegment::Name(ident) = segment
                {
                    let name = ident.name.as_str();
                    // Check for integer types
                    matches!(
                        name,
                        "Int"
                            | "I8"
                            | "I16"
                            | "I32"
                            | "I64"
                            | "I128"
                            | "U8"
                            | "U16"
                            | "U32"
                            | "U64"
                            | "U128"
                            | "USize"
                            | "ISize"
                    )
                } else {
                    false
                }
            }
            // References/pointers are valid as addresses
            Type::Reference { .. } => true,
            // Type variables might resolve to valid types
            Type::Var(_) => true,
            // Other types are not valid
            _ => false,
        }
    }

    /// Check if two types are compatible for type annotation casts.
    ///

    /// This allows casts like `None as Maybe<Int>` where the source type
    /// has unresolved type variables that can match the target type's arguments.
    fn types_compatible_for_cast(&self, from_ty: &Type, to_ty: &Type) -> bool {
        // Type variables match anything
        if matches!(from_ty, Type::Var(_)) {
            return true;
        }

        // Same types are compatible
        if from_ty == to_ty {
            return true;
        }

        // Subtype relationship
        if self.subtyping.is_subtype(from_ty, to_ty) {
            return true;
        }

        // Generic types - check if base matches and args are compatible
        if let (
            Type::Generic {
                name: from_name,
                args: from_args,
            },
            Type::Generic {
                name: to_name,
                args: to_args,
            },
        ) = (from_ty, to_ty)
        {
            if from_name == to_name && from_args.len() == to_args.len() {
                return from_args
                    .iter()
                    .zip(to_args.iter())
                    .all(|(f, t)| self.types_compatible_for_cast(f, t));
            }
        }

        false
    }

    /// Try to resolve a deferred associated type projection.
    ///

    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated type resolution
    ///

    /// Deferred projections are created when an associated type cannot be resolved
    /// immediately because the self type contains unresolved type variables.
    /// This method attempts to resolve them once the type variables are known.
    ///

    /// # Arguments
    /// * `projection_name` - The projection name in format `<SelfType as Protocol>::AssocName`
    /// * `args` - The normalized type arguments (first arg is the self type)
    ///

    /// # Returns
    /// * `Some(Type)` - The resolved associated type
    /// * `None` - If resolution still cannot complete
    fn try_resolve_deferred_projection(
        &self,
        projection_name: &Text,
        args: &List<Type>,
    ) -> Option<Type> {
        // The projection format is: <SelfType as Protocol>::AssocName
        // We need to parse this to extract the protocol name and associated type name

        let name_str = projection_name.as_str();

        // Find the protocol and associated type names from the projection
        // Format: <... as Protocol>::AssocName
        let as_pos = name_str.find(" as ")?;
        let close_pos = name_str.find(">::")?;

        if as_pos >= close_pos {
            return None;
        }

        let protocol_name = &name_str[as_pos + 4..close_pos];
        let assoc_name = &name_str[close_pos + 3..];

        // Get the self type from the args (first argument)
        let self_type = args.first()?;

        // Check if the self type still has unresolved type variables
        if self.has_unresolved_vars(self_type) {
            // Still can't resolve - return None to keep the projection as-is
            return None;
        }

        // Create a protocol path for the lookup
        let protocol_path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            protocol_name.to_string(),
            verum_ast::span::Span::dummy(),
        ));

        let assoc_name_text: Text = assoc_name.into();

        // Try to resolve using the protocol checker
        match self.protocol_checker.read().infer_associated_type(
            self_type,
            &protocol_path,
            &assoc_name_text,
        ) {
            Ok(resolved_ty) => Some(resolved_ty),
            Err(_) => {
                // Resolution failed - this could mean:
                // 1. The type doesn't implement the protocol
                // 2. The associated type is not specified
                // Keep the projection as-is for better error messages later
                None
            }
        }
    }

    /// Try to resolve an associated type projection from base type.
    ///

    /// This handles the `::AssocName` format where base type is in args[0].
    /// Used for projections like T.Item, C.Iter.Item etc.
    ///

    /// The resolution process:
    /// 1. Check if base type is concrete (no unresolved type variables)
    /// 2. Find all protocols that base type implements
    /// 3. Look for one that defines the associated type
    /// 4. Return the resolved type from the implementation
    pub(super) fn try_resolve_associated_type_projection(
        &self,
        base_ty: &Type,
        assoc_name: &str,
    ) -> Option<Type> {
        // RAII depth guard: prevent stack overflow on deeply nested iterator adapter chains.
        thread_local! {
            static ASSOC_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        let depth = ASSOC_DEPTH.with(|d| {
            let v = d.get();
            d.set(v + 1);
            v
        });
        // RAII: ensure decrement even on panic/early-return
        struct AssocDepthGuard;
        impl Drop for AssocDepthGuard {
            fn drop(&mut self) {
                ASSOC_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            }
        }
        let _guard = AssocDepthGuard;
        self.try_resolve_associated_type_projection_inner(base_ty, assoc_name, depth)
    }

    fn try_resolve_associated_type_projection_inner(
        &self,
        base_ty: &Type,
        assoc_name: &str,
        depth: u32,
    ) -> Option<Type> {
        if depth > 100 {
            return None; // Too deep — graceful fallback prevents stack overflow
        }

        // Resolve type variables through the unifier before checking.
        // This is critical for GAT resolution: when a generic function's return type
        // contains C.Item<T> and C is still a type var during normalization, the unifier
        // may already have resolved C to a concrete type (e.g., CloneableWrapper).
        let resolved_base = self.unifier.apply(base_ty);

        // If base type still has unresolved variables, try to resolve via bounds
        if self.has_unresolved_vars(&resolved_base) {
            // CRITICAL FIX: For bounded type variables (e.g., T: Iterator),
            // look up the associated type from the protocol bounds.
            // This enables T.Item resolution when T: Iterator.
            if let Type::Var(tv) = &resolved_base {
                let bounds = self.get_type_var_bounds(tv);
                let assoc_name_text: Text = assoc_name.into();
                for bound in bounds.iter() {
                    if let Some(protocol_ident) = bound.protocol.as_ident() {
                        let protocol_name: Text = protocol_ident.name.clone();
                        let protocol_opt = self
                            .protocol_checker
                            .read()
                            .get_protocol(&protocol_name)
                            .cloned();
                        if let Maybe::Some(protocol) = protocol_opt {
                            if let Some(assoc_type_def) =
                                protocol.associated_types.get(&assoc_name_text)
                            {
                                // Found the associated type in the bounded protocol.
                                // If it has a default, return that. Otherwise, create
                                // a projection that preserves the type variable link.
                                if let Some(default_ty) = &assoc_type_def.default {
                                    return Some(default_ty.clone());
                                }
                                // No default - the associated type is abstract.
                                // Return None to keep the projection deferred.
                                // This is correct: T.Item where T: Iterator stays as ::Item[T]
                                // until T is resolved to a concrete type.
                                return None;
                            }
                        }
                    }
                }
            }
            return None;
        }
        let base_ty = &resolved_base;

        let assoc_name_text: Text = assoc_name.into();

        // Try to find the associated type from any implemented protocol
        // This iterates through all known implementations for the base type
        if let Some(resolved) = self
            .protocol_checker
            .read()
            .try_find_associated_type(base_ty, &assoc_name_text)
        {
            return Some(resolved);
        }

        // Fallback: resolve Item for generic wrapper types based on adapter semantics.
        if assoc_name == "Item" {
            if let Type::Generic { name, args } = base_ty {
                let adapter_name = name.as_str();

                // Category 1: Item-transparent (Item = I.Item)
                let is_item_transparent = matches!(
                    adapter_name,
                    "Rev"
                        | "Filter"
                        | "Take"
                        | "Skip"
                        | "Peekable"
                        | "Fuse"
                        | "Inspect"
                        | "StepBy"
                        | "Chain"
                        | "Cycle"
                        | "TakeWhile"
                        | "SkipWhile"
                        | "Flatten"
                );
                if is_item_transparent && !args.is_empty() {
                    if let Some(inner_item) =
                        self.try_resolve_associated_type_projection(&args[0], assoc_name)
                    {
                        return Some(inner_item);
                    }
                }

                // Category 2: Enumerate<I> → (Int, I.Item)
                if adapter_name == "Enumerate" && !args.is_empty() {
                    if let Some(inner_item) =
                        self.try_resolve_associated_type_projection(&args[0], assoc_name)
                    {
                        return Some(Type::Tuple(verum_common::List::from(vec![
                            Type::int(),
                            inner_item,
                        ])));
                    }
                }

                // Category 3: Zip<A, B> → (A.Item, B.Item)
                if adapter_name == "Zip" && args.len() >= 2 {
                    let a_item = self.try_resolve_associated_type_projection(&args[0], assoc_name);
                    let b_item = self.try_resolve_associated_type_projection(&args[1], assoc_name);
                    if let (Some(a), Some(b)) = (a_item, b_item) {
                        return Some(Type::Tuple(verum_common::List::from(vec![a, b])));
                    }
                }

                // Category 4: ZipLongest<A, B> → (Maybe<A.Item>, Maybe<B.Item>)
                if (adapter_name == "ZipLongest" || adapter_name == "ZipLongestIter")
                    && args.len() >= 2
                {
                    let a_item = self.try_resolve_associated_type_projection(&args[0], assoc_name);
                    let b_item = self.try_resolve_associated_type_projection(&args[1], assoc_name);
                    if let (Some(a), Some(b)) = (a_item, b_item) {
                        return Some(Type::Tuple(verum_common::List::from(vec![
                            Type::maybe(a),
                            Type::maybe(b),
                        ])));
                    }
                }
            }
        }

        // If the base type is itself a projection (chained case like W.Inner.Item),
        // we may not be able to resolve it until the inner projection is resolved.
        None
    }
}
