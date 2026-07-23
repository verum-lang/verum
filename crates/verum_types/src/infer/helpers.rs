//! Standalone helper functions and trait implementations for the type-checker.
//!
//! Contains: ConditionExt, mount_tree_exports_name, resolve_builtin_meta_type,
//! check_condition, levenshtein_distance, span_to_line_col, expr_kind_description,
//! type_kind_description, HKT kind-checking TypeChecker methods, QTT helpers,
//! type-size calculation, make_maybe_type, resolve_primitive_method,
//! meta_value_to_literal, collect_named_types_from_*, parse_descriptor_type_string.

use super::TypeChecker;
use crate::ty::Type;
use crate::{Result, TypeError};
use verum_ast::{Expr, ExprKind, Pattern};
use verum_ast::span::{Span, Spanned};
use verum_ast::decl::{MountTree, MountTreeKind};
use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::well_known_types::WellKnownType as WKT;
use std::collections::HashSet;

// Size constants (used by calculate_type_size).
//
// All scalar / pointer / CBGR-reference sizes flow through the
// canonical layout module — `verum_common::layout` is the single
// source of truth and shares the constants with @const evaluation,
// MIR lowering, and codegen.
const MAX_STACK_ALLOCATION_BYTES: u64 = 1024 * 1024;
use verum_common::layout::{
    BOOL_SIZE as SIZE_OF_BOOL, CHAR_SIZE as SIZE_OF_CHAR, FLOAT_SIZE as SIZE_OF_FLOAT,
    INT_SIZE as SIZE_OF_INT, POINTER_SIZE as SIZE_OF_POINTER, REF_TIER0_SIZE,
    REF_TIER2_SIZE, SLICE_FAT_PTR_SIZE, TEXT_SIZE,
};

/// Helper trait for conditions (used in if/while condition parsing)
pub(crate) trait ConditionExt {
    fn as_expr(&self) -> Result<&Expr>;
    fn is_let(&self) -> bool;
    fn as_let(&self) -> Option<(&Pattern, &Expr)>;
}

impl ConditionExt for verum_ast::expr::ConditionKind {
    fn as_expr(&self) -> Result<&Expr> {
        match self {
            verum_ast::expr::ConditionKind::Expr(e) => Ok(e),
            verum_ast::expr::ConditionKind::Let { .. } => {
                Err(TypeError::Other(verum_common::Text::from(
                    "Expected boolean expression, found let pattern. Use as_let() for let conditions.",
                )))
            }
        }
    }

    fn is_let(&self) -> bool {
        matches!(self, verum_ast::expr::ConditionKind::Let { .. })
    }

    fn as_let(&self) -> Option<(&Pattern, &Expr)> {
        match self {
            verum_ast::expr::ConditionKind::Let { pattern, value } => Some((pattern, value)),
            _ => None,
        }
    }
}

/// Helper to check a condition and optionally bind patterns to scope
///

/// Map `@builtin_*` meta-type markers that appear on the RHS of stdlib
/// type aliases (e.g. `type I is @builtin_interval;`) to the internal
/// `Type` primitive they stand for. Returns `None` for names that are
/// not cubical / HoTT language primitives — those fall through to the
/// regular qualified-path resolution so that user-defined `@` meta
/// types continue to work.
/// Returns true if the given mount tree re-exports an item named `name`.
/// Used by import resolution to recognise that a name is re-exported
/// through a `public mount .path.{item}` chain even when it isn't a
/// direct item in the module's items list.
pub(crate) fn mount_tree_exports_name(tree: &verum_ast::decl::MountTree, name: &str) -> bool {
    use verum_ast::decl::MountTreeKind;
    use verum_ast::ty::PathSegment;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            // Last segment is the imported item name (or rename target).
            if let Some(PathSegment::Name(ident)) = path.segments.last() {
                return ident.name.as_str() == name;
            }
            false
        }
        MountTreeKind::Glob(_) => {
            // Glob re-exports everything; conservatively treat as exporting
            // any name. Callers fall through to real resolution.
            true
        }
        MountTreeKind::Nested { trees, .. } => {
            trees.iter().any(|t| mount_tree_exports_name(t, name))
        }
        // #5 / P1.5 — file-relative mounts contribute exports
        // through the session loader's per-file module
        // registration, not through this AST-level export
        // probe. The alias (if any) is the only name
        // observable from the importing scope.
        MountTreeKind::File { .. } => tree
            .alias
            .as_ref()
            .map(|a| a.name.as_str() == name)
            .unwrap_or(false),
    }
}

pub(crate) fn resolve_builtin_meta_type(name: &str) -> Option<Type> {
    match name {
        "@builtin_interval" => Some(Type::Interval),
        // Path / Glue are type *constructors* that already have their own
        // AST sugaring (`TypeKind::PathType`, `TypeKind::DependentApp`) at
        // use sites. The bare marker on the alias RHS is an opaque primitive
        // stand-in — the declared carrier (`Path`, `Glue`) becomes a named
        // type pointing at this opaque marker, and real uses `Path<A>(a, b)`
        // take the sugared AST path and lower to `Type::Eq` / dependent app.
        "@builtin_path" => Some(Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                verum_common::Text::from("Path"),
                verum_ast::span::Span::dummy(),
            )),
            args: List::new(),
        }),
        "@builtin_glue" => Some(Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                verum_common::Text::from("Glue"),
                verum_ast::span::Span::dummy(),
            )),
            args: List::new(),
        }),
        _ => None,
    }
}

/// This handles both expression conditions and let conditions:
/// - `if x > 0` - Expression condition (must evaluate to Bool)
/// - `if let Some(v) = opt` - Let condition (pattern must match value)
///

/// Returns the type of bindings introduced (empty for expression conditions)
pub(crate) fn check_condition(
    checker: &mut TypeChecker,
    condition: &verum_ast::expr::ConditionKind,
) -> Result<List<(Text, Type)>> {
    match condition {
        verum_ast::expr::ConditionKind::Expr(expr) => {
            // Expression condition - must be Bool
            checker.check_expr(expr, &Type::bool())?;
            Ok(List::new())
        }
        verum_ast::expr::ConditionKind::Let { pattern, value } => {
            // Let condition - bind pattern to value
            // If-let expressions: "if let Pattern = expr { ... }" for refutable pattern matching with type narrowing
            //

            // The pattern is checked against the type of the value,
            // and any bound variables become available in the then-branch.
            let value_result = checker.synth_expr(value)?;
            let value_ty = value_result.ty;

            // Bind pattern to value type
            checker.bind_pattern(pattern, &value_ty)?;

            // Collect bindings introduced by the pattern
            let bindings = checker.collect_pattern_bindings(pattern, &value_ty)?;

            Ok(bindings)
        }
    }
}

/// Check all conditions in an if-condition chain
///

/// Handles both simple conditions and let-chains like:
/// `if let Some(x) = opt && x > 0`
pub(crate) fn check_all_conditions(
    checker: &mut TypeChecker,
    conditions: &verum_ast::expr::IfCondition,
) -> Result<List<(Text, Type)>> {
    let mut all_bindings = List::new();

    for cond in &conditions.conditions {
        let mut bindings = check_condition(checker, cond)?;
        all_bindings.append(&mut bindings);
    }

    Ok(all_bindings)
}

/// Compute Levenshtein distance between two strings for suggestions
///

/// Used to provide "did you mean?" suggestions in error messages.
pub(crate) fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.len();
    let len2 = s2.len();

    // Fast paths
    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }
    if s1 == s2 {
        return 0;
    }

    // Standard dynamic programming approach
    let mut prev_row: List<usize> = (0..=len2).collect();
    let mut curr_row: List<usize> = vec![0; len2 + 1].into();

    for (i, c1) in s1.chars().enumerate() {
        curr_row[0] = i + 1;

        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[len2]
}

// ==================== Helper Functions ====================

/// Helper function to convert Span to LineColSpan using the global source file registry
pub(crate) fn span_to_line_col(span: Span) -> verum_common::span::LineColSpan {
    // Use verum_common's global registry as that's where compiler registers files
    verum_common::global_span_to_line_col(span)
}

/// Helper function to get a human-readable description of an ExprKind
pub(crate) fn expr_kind_description(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::Literal(_) => "literal",
        ExprKind::Path(_) => "path",
        ExprKind::Binary { .. } => "binary operation",
        ExprKind::Unary { .. } => "unary operation",
        ExprKind::Call { .. } => "function call",
        ExprKind::MethodCall { .. } => "method call",
        ExprKind::Field { .. } => "field access",
        ExprKind::OptionalChain { .. } => "optional chain",
        ExprKind::TupleIndex { .. } => "tuple index",
        ExprKind::Index { .. } => "index operation",
        ExprKind::Pipeline { .. } => "pipeline",
        ExprKind::NullCoalesce { .. } => "null coalesce",
        ExprKind::Cast { .. } => "type cast",
        ExprKind::Try(_) => "try expression",
        ExprKind::TryBlock(_) => "try block",
        ExprKind::TryRecover { .. } => "try-recover expression",
        ExprKind::TryFinally { .. } => "try-finally expression",
        ExprKind::TryRecoverFinally { .. } => "try-recover-finally expression",
        ExprKind::Block(_) => "block",
        ExprKind::If { .. } => "if expression",
        ExprKind::Match { .. } => "match expression",
        ExprKind::While { .. } => "while loop",
        ExprKind::For { .. } => "for loop",
        ExprKind::ForAwait { .. } => "for await loop",
        ExprKind::Loop { .. } => "loop",
        ExprKind::Break { .. } => "break",
        ExprKind::Continue { .. } => "continue",
        ExprKind::Return(_) => "return",
        ExprKind::Yield(_) => "yield",
        ExprKind::Closure { .. } => "closure",
        ExprKind::Tuple(_) => "tuple",
        ExprKind::Array(_) => "array",
        ExprKind::Record { .. } => "record",
        ExprKind::InterpolatedString { .. } => "interpolated string",
        ExprKind::TensorLiteral { .. } => "tensor literal",
        ExprKind::MapLiteral { .. } => "map literal",
        ExprKind::SetLiteral { .. } => "set literal",
        ExprKind::Range { .. } => "range",
        ExprKind::Comprehension { .. } => "comprehension",
        ExprKind::StreamComprehension { .. } => "stream comprehension",
        ExprKind::MapComprehension { .. } => "map comprehension",
        ExprKind::SetComprehension { .. } => "set comprehension",
        ExprKind::GeneratorComprehension { .. } => "generator expression",
        ExprKind::StreamLiteral(_) => "stream literal",
        ExprKind::Forall { .. } => "forall expression",
        ExprKind::Exists { .. } => "exists expression",
        ExprKind::Attenuate { .. } => "attenuate expression",
        ExprKind::Async(_) => "async expression",
        ExprKind::Await(_) => "await expression",
        ExprKind::Spawn { .. } => "spawn expression",
        ExprKind::Inject { .. } => "inject expression",
        ExprKind::Unsafe(_) => "unsafe block",
        ExprKind::Meta(_) => "meta block",
        ExprKind::Quote { .. } => "quote expression",
        ExprKind::StageEscape { .. } => "stage escape expression",
        ExprKind::Lift { .. } => "lift expression",
        ExprKind::MacroCall { .. } => "macro call",
        ExprKind::UseContext { .. } => "context handler binding",
        ExprKind::Paren(_) => "parenthesized expression",
        ExprKind::TypeProperty { .. } => "type property expression",
        ExprKind::TypeExpr(_) => "type expression",
        ExprKind::Select { .. } => "select expression",
        ExprKind::Throw(_) => "throw expression",
        ExprKind::Typeof(_) => "typeof expression",
        ExprKind::Is { .. } => "is pattern test",
        ExprKind::TypeBound { .. } => "type bound expression",
        ExprKind::MetaFunction { .. } => "meta function",
        ExprKind::Nursery { .. } => "nursery expression",
        ExprKind::InlineAsm { .. } => "inline assembly",
        ExprKind::DestructuringAssign { .. } => "destructuring assignment",
        ExprKind::CalcBlock(_) => "calc block",
        ExprKind::NamedArg { .. } => "named argument",
        ExprKind::CopatternBody { .. } => "copattern body",
    }
}

/// Helper function to get a human-readable description of a TypeKind
pub(crate) fn type_kind_description(kind: &verum_ast::ty::TypeKind) -> String {
    use verum_ast::ty::TypeKind;
    match kind {
        TypeKind::Unit => "unit type ()".to_string(),
        TypeKind::Bool => WKT::Bool.as_str().to_string(),
        TypeKind::Int => WKT::Int.as_str().to_string(),
        TypeKind::Float => WKT::Float.as_str().to_string(),
        TypeKind::Char => WKT::Char.as_str().to_string(),
        TypeKind::Text => WKT::Text.as_str().to_string(),
        TypeKind::Never => "never type !".to_string(),
        TypeKind::Path(path) => format!("path '{}'", path),
        TypeKind::PathType { .. } => "path type Path<A>(a, b)".to_string(),
        TypeKind::DependentApp { .. } => "dependent type application T<..>(v..)".to_string(),
        TypeKind::Tuple(_) => "tuple type".to_string(),
        TypeKind::Array { .. } => "array type".to_string(),
        TypeKind::Slice(_) => "slice type".to_string(),
        TypeKind::Function { .. } => "function type".to_string(),
        TypeKind::Rank2Function { .. } => "rank-2 function type".to_string(),
        TypeKind::Reference { .. } => "reference type".to_string(),
        TypeKind::CheckedReference { .. } => "checked reference type".to_string(),
        TypeKind::UnsafeReference { .. } => "unsafe reference type".to_string(),
        TypeKind::Pointer { .. } => "pointer type".to_string(),
        TypeKind::VolatilePointer { .. } => "volatile pointer type".to_string(),
        TypeKind::Generic { .. } => "generic type".to_string(),
        TypeKind::Qualified { .. } => "qualified type".to_string(),
        TypeKind::Refined { .. } => "refinement type".to_string(),
        TypeKind::Inferred => "inferred type".to_string(),
        TypeKind::Bounded { .. } => "bounded type".to_string(),
        TypeKind::DynProtocol { .. } => "dyn protocol type".to_string(),
        TypeKind::Ownership { .. } => "ownership type".to_string(),
        TypeKind::GenRef { .. } => "GenRef type".to_string(),
        TypeKind::TypeConstructor { .. } => "type constructor".to_string(),
        TypeKind::Tensor { .. } => "tensor type".to_string(),
        TypeKind::Existential { .. } => "existential type".to_string(),
        TypeKind::AssociatedType { .. } => "associated type".to_string(),
        TypeKind::CapabilityRestricted { .. } => "capability-restricted type".to_string(),
        TypeKind::Unknown => "Unknown type".to_string(),
        TypeKind::Record { .. } => "record type".to_string(),
        TypeKind::Universe { .. } => "universe type".to_string(),
        TypeKind::Meta { .. } => "meta type".to_string(),
        TypeKind::TypeLambda { .. } => "type lambda".to_string(),
    }
}

// ==================== Type Registration for Pipeline ====================

// Type declaration registration methods (register_type_declaration*, register_variant*, …)
// → see infer/decls.rs in this module


// ==================== HKT Instantiation ====================

impl TypeChecker {
    /// Check kind compatibility when applying a type constructor to arguments.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
    ///

    /// When applying `F<Int>` where `F: * -> *`, this verifies:
    /// 1. F has the expected constructor kind (* -> *)
    /// 2. Int has kind * (the expected argument kind)
    /// 3. The resulting application F<Int> has kind *
    ///

    /// # Arguments
    ///

    /// * `constructor` - The type constructor being applied (e.g., F, List, Map)
    /// * `args` - The type arguments being applied
    /// * `span` - Source location for error reporting
    ///

    /// # Returns
    ///

    /// * `Ok(Kind)` - The resulting kind after application
    /// * `Err(TypeError)` - If kind mismatch or arity error
    ///

    /// # Examples
    ///

    /// ```ignore
    /// // F<Int> where F: * -> *
    /// let result_kind = checker.check_type_application_kind(
    ///  &Type::TypeConstructor { name: "F".into(), arity: 1, kind: Kind::unary_constructor() },
    ///  &[Type::Int],
    ///  Span::default()
    /// )?;
    /// assert_eq!(result_kind, Kind::Type);
    /// ```
    pub fn check_type_application_kind(
        &mut self,
        constructor: &Type,
        args: &[Type],
        span: Span,
    ) -> Result<crate::kind_inference::Kind> {
        if !self.higher_kinded_enabled {
            // HKT disabled — skip kind checking, assume kind Type for all.
            return Ok(crate::kind_inference::Kind::Type);
        }
        self.kind_inferer
            .check_type_application_kind(constructor, args, span)
    }

    /// Instantiate an HKT parameter with a concrete type constructor.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — HKT parameter instantiation
    ///

    /// When calling `fn foo<F<_>: Functor>(x: F<Int>)` with `foo::<List>(...)`,
    /// this verifies:
    /// 1. `List` has kind `* -> *` (matches F's expected kind)
    /// 2. `List` implements `Functor` (satisfies protocol bound)
    ///

    /// # Arguments
    ///

    /// * `hkt_param_name` - Name of the HKT parameter (e.g., "F")
    /// * `expected_kind` - The expected kind for the parameter (e.g., * -> *)
    /// * `concrete_constructor` - The concrete type constructor being substituted (e.g., List)
    /// * `protocol_bounds` - Protocol bounds that must be satisfied (e.g., Functor)
    /// * `span` - Source location for error reporting
    ///

    /// # Returns
    ///

    /// * `Ok(HKTInstantiationResult)` - Successful instantiation with result info
    /// * `Err(TypeError)` - If kind mismatch or protocol not implemented
    ///

    /// # Examples
    ///

    /// ```ignore
    /// // Instantiate F<_> with List where F<_>: Functor
    /// let result = checker.instantiate_hkt_param(
    ///  "F",
    ///  &Kind::unary_constructor(),
    ///  &Type::TypeConstructor { name: "List".into(), arity: 1, kind: Kind::unary_constructor() },
    ///  &[ProtocolBound::simple("Functor".into())],
    ///  Span::default(),
    /// )?;
    /// ```
    pub fn instantiate_hkt_param(
        &mut self,
        hkt_param_name: &str,
        expected_kind: &crate::kind_inference::Kind,
        concrete_constructor: &Type,
        protocol_bounds: &[crate::protocol::ProtocolBound],
        span: Span,
    ) -> Result<crate::kind_inference::HKTInstantiationResult> {
        // Extract protocol checker reference to avoid self borrow conflict
        // We use a reference to the protocol_checker directly in the closure
        let protocol_checker = &self.protocol_checker;

        // Create a closure that checks protocol implementation using the protocol_checker
        let check_protocol = |ty: &Type, bound: &crate::protocol::ProtocolBound| -> bool {
            // Extract the constructor name
            let constructor_name: Text = match ty {
                Type::TypeConstructor { name, .. } => name.clone(),
                Type::Named { path, .. } => path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                        _ => "unknown".into(),
                    })
                    .unwrap_or_else(|| "unknown".into()),
                Type::Generic { name, .. } => name.clone(),
                _ => return false,
            };

            // Extract protocol name from path
            let protocol_name: Text = bound
                .protocol
                .segments
                .last()
                .map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                    _ => verum_common::Text::from("unknown"),
                })
                .unwrap_or_else(|| verum_common::Text::from("unknown"));

            // Check if the protocol implementation is registered
            protocol_checker
                .read()
                .type_constructor_implements_protocol(&constructor_name, &protocol_name)
        };

        self.kind_inferer.instantiate_hkt_param(
            hkt_param_name,
            expected_kind,
            concrete_constructor,
            protocol_bounds,
            span,
            check_protocol,
        )
    }

    /// Check if a type constructor implements a protocol.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Protocol checking for type constructors
    ///

    /// For HKT bounds like `F<_>: Functor + Monad`, this checks if the type
    /// constructor (e.g., List, Maybe) implements the required protocol.
    ///

    /// # Arguments
    ///

    /// * `constructor` - The type constructor to check
    /// * `bound` - The protocol bound that must be satisfied
    ///

    /// # Returns
    ///

    /// * `true` if the constructor implements the protocol
    /// * `false` otherwise
    ///

    /// # Examples
    ///

    /// ```ignore
    /// let list_ctor = Type::TypeConstructor { name: "List".into(), arity: 1, kind: Kind::unary_constructor() };
    /// let functor_bound = ProtocolBound::simple("Functor".into());
    /// let implements = checker.check_type_constructor_implements_protocol(&list_ctor, &functor_bound);
    /// ```
    pub fn check_type_constructor_implements_protocol(
        &self,
        constructor: &Type,
        bound: &crate::protocol::ProtocolBound,
    ) -> bool {
        // Extract the constructor name
        let constructor_name = match constructor {
            Type::TypeConstructor { name, .. } => name.clone(),
            Type::Named { path, .. } => path
                .segments
                .last()
                .map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                    _ => "unknown".into(),
                })
                .unwrap_or_else(|| "unknown".into()),
            Type::Generic { name, .. } => name.clone(),
            _ => return false,
        };

        // Extract the protocol name from the Path
        let protocol_name: Text = bound
            .protocol
            .segments
            .last()
            .map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                _ => verum_common::Text::from("unknown"),
            })
            .unwrap_or_else(|| verum_common::Text::from("unknown"));

        // Check if the protocol implementation is registered
        // The protocol checker tracks implementations by (type_name, protocol_name)
        self.protocol_checker
            .read()
            .type_constructor_implements_protocol(&constructor_name, &protocol_name)
    }

    /// Verify HKT bounds for a function call with type constructor arguments.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — HKT verification during type checking
    ///

    /// When calling a function like `fn traverse<F<_>: Applicative, A, B>(...)`
    /// with concrete type constructor arguments, this method verifies all HKT
    /// constraints are satisfied.
    ///

    /// # Arguments
    ///

    /// * `hkt_params` - List of (param_name, expected_kind, protocol_bounds)
    /// * `concrete_args` - The concrete type constructors being substituted
    /// * `span` - Source location for error reporting
    ///

    /// # Returns
    ///

    /// * `Ok(List<HKTInstantiationResult>)` - All instantiations succeeded
    /// * `Err(TypeError)` - First failing constraint
    pub fn verify_hkt_bounds(
        &mut self,
        hkt_params: &[(
            Text,
            crate::kind_inference::Kind,
            List<crate::protocol::ProtocolBound>,
        )],
        concrete_args: &[Type],
        span: Span,
    ) -> Result<List<crate::kind_inference::HKTInstantiationResult>> {
        if hkt_params.len() != concrete_args.len() {
            return Err(TypeError::Other(
                format!(
                    "Expected {} HKT arguments but got {}",
                    hkt_params.len(),
                    concrete_args.len()
                )
                .into(),
            ));
        }

        let mut results = List::new();
        let mut protocol_errors = List::new();

        for (i, (param_name, expected_kind, bounds)) in hkt_params.iter().enumerate() {
            let concrete = &concrete_args[i];

            let result = self.instantiate_hkt_param(
                param_name.as_str(),
                expected_kind,
                concrete,
                bounds.as_slice(),
                span,
            )?;

            // Collect protocol bound failures for better error messages
            if !result.protocol_bounds_satisfied {
                for bound in bounds {
                    if !self.check_type_constructor_implements_protocol(concrete, bound) {
                        protocol_errors.push((
                            param_name.clone(),
                            bound.protocol.clone(),
                            concrete.clone(),
                        ));
                    }
                }
            }

            results.push(result);
        }

        // Report all protocol violations at once for better error messages
        if !protocol_errors.is_empty() {
            let error_msg = protocol_errors
                .iter()
                .map(|(param, protocol, ty)| {
                    format!(
                        "HKT parameter '{}' requires '{}' to implement '{}'",
                        param,
                        self.type_display(ty),
                        protocol
                    )
                })
                .collect::<List<String>>()
                .join("; ");

            return Err(TypeError::ProtocolNotSatisfied {
                ty: format!("{:?}", concrete_args).into(),
                protocol: error_msg,
                span,
            });
        }

        Ok(results)
    }

    /// Helper to display a type for error messages
    fn type_display(&self, ty: &Type) -> String {
        match ty {
            Type::TypeConstructor { name, arity, .. } => {
                if *arity > 0 {
                    format!("{}<{}>", name, "_,".repeat(*arity).trim_end_matches(','))
                } else {
                    name.to_string()
                }
            }
            Type::Named { path, args } => {
                let name = path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => "?".to_string(),
                    })
                    .unwrap_or_else(|| "?".to_string());
                if args.is_empty() {
                    name
                } else {
                    format!("{}<{}>", name, args.len())
                }
            }
            Type::Generic { name, args } => {
                if args.is_empty() {
                    name.to_string()
                } else {
                    format!("{}<{}>", name, args.len())
                }
            }
            Type::TypeApp { constructor, args } => {
                format!("{}<{} args>", self.type_display(constructor), args.len())
            }
            _ => format!("{:?}", ty),
        }
    }
}

// ==================== Kind Annotation Conversion ====================

impl TypeChecker {
    /// Convert an AST `KindAnnotation` (from `verum_ast`) to the type-checker's
    /// `kind_inference::Kind`, which is used internally for kind constraint solving.
    ///

    /// Both types represent the same algebra (`Type | K1 -> K2`) but live in
    /// different crates to avoid a circular dependency.
    pub(crate) fn ast_kind_to_infer_kind(
        ann: &verum_ast::ty::KindAnnotation,
    ) -> crate::kind_inference::Kind {
        match ann {
            verum_ast::ty::KindAnnotation::Type => crate::kind_inference::Kind::Type,
            verum_ast::ty::KindAnnotation::Arrow(lhs, rhs) => crate::kind_inference::Kind::Arrow(
                Box::new(Self::ast_kind_to_infer_kind(lhs)),
                Box::new(Self::ast_kind_to_infer_kind(rhs)),
            ),
        }
    }
}

// ==================== Kind Inference Integration ====================

impl crate::kind_inference::KindInference for TypeChecker {
    fn kind_inferer(&mut self) -> &mut crate::kind_inference::KindInferer {
        &mut self.kind_inferer
    }

    fn check_kind(&mut self, ty: &Type, expected_kind: &crate::kind_inference::Kind) -> Result<()> {
        self.kind_inferer.check_kind(ty, expected_kind)
    }

    fn infer_kind(&mut self, ty: &Type) -> Result<crate::kind_inference::Kind> {
        self.kind_inferer.infer_kind(ty)
    }

    fn check_protocol_kinds(&mut self, protocol: &crate::protocol::Protocol) -> Result<()> {
        self.kind_inferer.check_protocol_kinds(protocol)
    }
}

// ==================== QTT V2 helpers (#235, A.Z.5 §7.6) ====================

/// extract the declared QTT [`crate::ty::Quantity`]
/// from a parameter's attribute list.
///

/// Reads the first `@quantity(...)` attribute via
/// [`verum_ast::attr::QuantityAttr::from_attribute`] and maps the
/// AST-side enum (`Zero / One / Many`) to the verum_types-side
/// [`crate::ty::Quantity`] (`Zero / One / Omega / AtMost / Graded`).
/// Returns `Quantity::Omega` (unrestricted) when no `@quantity`
/// attribute is present — matches default.
pub(crate) fn extract_quantity_from_attrs(
    attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> crate::ty::Quantity {
    use verum_ast::attr::{Quantity as AstQty, QuantityAttr};
    use verum_common::Maybe;
    for a in attrs.iter() {
        if let Maybe::Some(parsed) = QuantityAttr::from_attribute(a) {
            return match parsed.quantity {
                AstQty::Zero => crate::ty::Quantity::Zero,
                AstQty::One => crate::ty::Quantity::One,
                AstQty::Many => crate::ty::Quantity::Omega,
            };
        }
    }
    crate::ty::Quantity::Omega
}

/// walk a single statement node, accumulating QTT
/// usage for tracked bindings into `usage`. Per QTT calculus,
/// statements compose sequentially — each contributes
/// `merge_sequential` to the running tally.
///

/// Recognised statement shapes:
///  * `Stmt::Expr { expr, .. }` — recurse into expr.
///  * `Stmt::Let { value, .. }` — recurse into the initialiser.
///  * `Stmt::LetElse { value, else_block, .. }` — initialiser is
///  sequential; else_block is taken as a branch (worst-case
///  accumulated via merge_sequential since the LetElse else
///  path runs only on pattern-mismatch — pessimistic).
///  * `Stmt::Defer(expr)` / `Errdefer(expr)` — recurse.
///  * Other Stmt variants (Item, etc.) — no value-usage, skip.
pub(crate) fn walk_stmt_for_qtt_usage(
    tracked: &std::collections::HashSet<verum_common::Text>,
    stmt: &verum_ast::stmt::Stmt,
    usage: &mut crate::qtt_usage::UsageMap,
) {
    use verum_ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Let { value, .. } => {
            if let verum_common::Maybe::Some(v) = value {
                let d = crate::qtt_walker::walk_expr(tracked, v);
                let merged = std::mem::take(usage).merge_sequential(d);
                *usage = merged;
            }
        }
        StmtKind::LetElse {
            value, else_block, ..
        } => {
            let value_usage = crate::qtt_walker::walk_expr(tracked, value);
            let merged = std::mem::take(usage).merge_sequential(value_usage);
            *usage = merged;
            // else_block is a Block; walk its statements recursively.
            for s in else_block.stmts.iter() {
                walk_stmt_for_qtt_usage(tracked, s, usage);
            }
            if let verum_common::Maybe::Some(tail) = &else_block.expr {
                let tail_usage = crate::qtt_walker::walk_expr(tracked, tail);
                let merged = std::mem::take(usage).merge_sequential(tail_usage);
                *usage = merged;
            }
        }
        StmtKind::Expr { expr, .. } => {
            let d = crate::qtt_walker::walk_expr(tracked, expr);
            let merged = std::mem::take(usage).merge_sequential(d);
            *usage = merged;
        }
        StmtKind::Defer(e) | StmtKind::Errdefer(e) => {
            let d = crate::qtt_walker::walk_expr(tracked, e);
            let merged = std::mem::take(usage).merge_sequential(d);
            *usage = merged;
        }
        // Other statement kinds (Item declarations, etc.) don't
        // produce variable references at this scope.
        _ => {}
    }
}

// ==================== Stack Safety Checks ====================
// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow

impl TypeChecker {
    /// Calculate the size of a type in bytes for stack allocation checking.
    ///

    /// Returns None if the size cannot be determined at compile time
    /// (e.g., for dynamically-sized types or circular types).
    pub fn calculate_type_size(&self, ty: &Type) -> Option<u64> {
        // Use depth-tracked version to prevent stack overflow on circular types
        self.calculate_type_size_impl(ty, &mut HashSet::new())
    }

    /// Internal implementation with cycle detection via visited set.
    /// Prevents stack overflow from circular struct types (A -> B -> C -> A).
    fn calculate_type_size_impl(&self, ty: &Type, visited: &mut HashSet<String>) -> Option<u64> {
        match ty {
            // Primitive types
            Type::Int => Some(SIZE_OF_INT),
            Type::Float => Some(SIZE_OF_FLOAT),
            Type::Bool => Some(SIZE_OF_BOOL),
            Type::Char => Some(SIZE_OF_CHAR),
            Type::Unit => Some(0),
            Type::Never => Some(0),
            Type::Text => Some(TEXT_SIZE), // ptr + len + cap (canonical layout)

            // CBGR Tier-0 / Tier-1 references are ThinRef-shaped:
            // 16 bytes = ptr + generation + epoch_caps. Required for
            // correct stack-allocation budgets and `@sizeof` answers.
            Type::Reference { .. }
            | Type::CheckedReference { .. } => Some(REF_TIER0_SIZE),

            // Tier-2 (`&unsafe`) and raw pointers strip the CBGR
            // metadata and lower to a bare 8-byte pointer.
            Type::UnsafeReference { .. }
            | Type::Pointer { .. }
            | Type::VolatilePointer { .. } => Some(REF_TIER2_SIZE),

            // Array with known size: element_size * count
            Type::Array {
                element,
                size: Some(count),
            } => {
                let elem_size = self.calculate_type_size_impl(element, visited)?;
                Some(elem_size * (*count as u64))
            }

            // Array without known size - dynamic, can't determine
            Type::Array { size: None, .. } => None,

            // Slice is fat pointer (ptr + len) — canonical SLICE_FAT_PTR_SIZE.
            Type::Slice { .. } => Some(SLICE_FAT_PTR_SIZE),

            // Tuple: sum of all element sizes (simplified, ignoring alignment)
            Type::Tuple(elements) => {
                let mut total = 0u64;
                for elem in elements.iter() {
                    total += self.calculate_type_size_impl(elem, visited)?;
                }
                Some(total)
            }

            // Named types - look up struct fields
            Type::Named { path, args } => {
                let type_name = self.path_to_string(path);

                // CYCLE GUARD: Detect circular struct types (A -> B -> C -> A).
                // If we're already computing the size of this type, we have an
                // infinite-size type. Return None (unknown size) to prevent stack overflow.
                if !visited.insert(type_name.to_string()) {
                    return None; // Circular type detected - size is infinite/unknown
                }

                let struct_key = format!("__struct_fields_{}", type_name);

                let result =
                    if let Maybe::Some(Type::Record(fields)) = self.ctx.lookup_type(&struct_key) {
                        let mut total = 0u64;
                        for field_ty in fields.values() {
                            // Substitute type parameters if present
                            let resolved_ty = if !args.is_empty() {
                                // For simplicity, use the field type as-is for size calculation
                                // A full implementation would substitute type params
                                field_ty.clone()
                            } else {
                                field_ty.clone()
                            };
                            match self.calculate_type_size_impl(&resolved_ty, visited) {
                                Some(size) => total += size,
                                None => {
                                    visited.remove(type_name.as_str());
                                    return None;
                                }
                            }
                        }
                        Some(total)
                    } else {
                        // Assume pointer-sized for unknown named types (conservative)
                        Some(SIZE_OF_POINTER)
                    };

                visited.remove(type_name.as_str());
                result
            }

            // Record types: sum of field sizes
            Type::Record(fields) => {
                let mut total = 0u64;
                for field_ty in fields.values() {
                    total += self.calculate_type_size_impl(field_ty, visited)?;
                }
                Some(total)
            }

            // Generic types - unknown size without resolving the full type definition
            // This is stdlib-agnostic: no hardcoded type names
            Type::Generic { .. } => None,

            // Function pointers
            Type::Function { .. } => Some(SIZE_OF_POINTER),

            // Type variables - can't determine size
            Type::Var(_) => None,

            // Variants - size of largest variant
            Type::Variant(variants) => {
                let mut max_size = 0u64;
                for variant_ty in variants.values() {
                    if let Some(size) = self.calculate_type_size_impl(variant_ty, visited) {
                        max_size = max_size.max(size);
                    }
                }
                // Add discriminant size
                Some(max_size + SIZE_OF_INT)
            }

            // Other types - conservatively assume unknown
            _ => None,
        }
    }

    /// Check if a stack allocation exceeds the safe limit.
    ///

    /// Returns an error if the type's size exceeds MAX_STACK_ALLOCATION_BYTES.
    /// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
    pub fn check_stack_allocation_size(&self, ty: &Type, span: Span) -> Result<()> {
        if let Some(size) = self.calculate_type_size(ty) {
            if size > MAX_STACK_ALLOCATION_BYTES {
                return Err(TypeError::StackAllocationExceedsLimit {
                    size,
                    limit: MAX_STACK_ALLOCATION_BYTES,
                    span,
                });
            }
        }
        Ok(())
    }
}

/// Create a Maybe<T> type for use in return types.
pub(crate) fn make_maybe_type(inner: Type) -> Type {
    use smallvec::smallvec;
    use verum_ast::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    let ident = Ident::new("Maybe", Span::dummy());
    Type::Named {
        path: Path {
            segments: smallvec![PathSegment::Name(ident)],
            span: Span::dummy(),
        },
        args: List::from(vec![inner]),
    }
}

/// Resolve built-in methods on primitive types (Int, Float, Bool, Char, Byte).
/// These are language built-in types with inherent methods, not stdlib types.
/// HARDCODED FALLBACK for primitive type method return types.
///

/// This function maps (primitive_type, method_name, arg_count) -> return_type for
/// Int, Float, Bool, Char, and Byte methods. It serves as a safety net when the
/// stdlib .vr implement blocks are not loaded into inherent_methods.
///

/// In normal compilation (stdlib loaded via pipeline Pass 5), all these methods
/// should be resolved from inherent_methods BEFORE reaching this fallback.
/// The checked/saturating/wrapping arithmetic methods intentionally return None
/// here to force resolution through stdlib (for correct unsigned type handling).
///

/// HARDCODE(#7): Once confirmed that inherent_methods always has these
/// signatures, this function can be removed entirely.
pub(crate) fn resolve_primitive_method(recv_ty: &Type, method: &str, arg_count: usize) -> Option<Type> {
    // Peel references to get the underlying type
    let base_ty = match recv_ty {
        Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::UnsafeReference { inner, .. } => inner.as_ref(),
        _ => recv_ty,
    };

    // Classify the primitive type
    let prim = match base_ty {
        Type::Int => "int",
        Type::Float => "float",
        Type::Bool => "bool",
        Type::Char => "char",
        Type::Named { path, .. } => {
            let id = path.as_ident()?;
            let tn = id.name.as_str();
            match tn {
                _ if verum_common::well_known_types::type_names::is_integer_type(tn)
                    && tn != "Byte" =>
                {
                    "int"
                }
                _ if verum_common::well_known_types::type_names::is_float_type(tn) => "float",
                "Char" => "char",
                "Byte" => "byte",
                "Bool" => "bool",
                _ => return None,
            }
        }
        _ => return None,
    };

    match prim {
        "int" => match (method, arg_count) {
            ("abs", 0) | ("signum", 0) => Some(Type::Int),
            ("is_positive", 0) | ("is_negative", 0) | ("is_zero", 0) => Some(Type::Bool),
            ("min", 1) | ("max", 1) | ("clamp", 2) | ("pow", 1) => Some(Type::Int),
            // CRITICAL: Do NOT handle checked/saturating/wrapping arithmetic here!
            // These must fall through to inherent method lookup so that UInt64.checked_add
            // resolves to the correct unsigned intrinsic (checked_add_u64) instead of
            // using signed Int arithmetic. The stdlib defines type-specific methods.
            ("checked_add", 1) | ("checked_sub", 1) | ("checked_mul", 1) | ("checked_div", 1) => {
                None
            }
            ("saturating_add", 1) | ("saturating_sub", 1) => None,
            ("wrapping_add", 1) | ("wrapping_sub", 1) => None,
            ("to_float", 0) | ("to_f64", 0) => Some(Type::Float),
            ("to_int", 0) => Some(Type::Int),
            ("count_ones", 0) | ("count_zeros", 0) => Some(Type::Int),
            ("leading_zeros", 0) | ("trailing_zeros", 0) => Some(Type::Int),
            ("reverse_bits", 0) | ("swap_bytes", 0) => Some(Type::Int),
            ("rotate_left", 1) | ("rotate_right", 1) => Some(Type::Int),
            ("in_range", 2) => Some(Type::Bool),
            // CBGR epoch_caps bit inspection methods (packed capability integer)
            ("can_read", 0) | ("can_write", 0) | ("can_extend", 0) | ("is_unique", 0) => {
                Some(Type::Bool)
            }
            ("epoch", 0) | ("raw", 0) => Some(Type::Int), // Extract epoch / identity for capabilities
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            ("to_hex_string", 0) | ("to_binary_string", 0) | ("to_octal_string", 0) => {
                Some(Type::Text)
            }
            ("max_value", 0) | ("min_value", 0) | ("MIN", 0) | ("MAX", 0) | ("BITS", 0) => {
                Some(Type::Int)
            }
            // NOTE: to_le_bytes/to_be_bytes/from_le_bytes/from_be_bytes must fall through
            // to proper method resolution so type-specific byte sizes are used.
            // Int uses 8 bytes, Int32 uses 4 bytes, Int16 uses 2 bytes, etc.
            _ => None,
        },
        "float" => match (method, arg_count) {
            ("abs", 0)
            | ("ceil", 0)
            | ("floor", 0)
            | ("round", 0)
            | ("trunc", 0)
            | ("fract", 0) => Some(Type::Float),
            ("sqrt", 0) | ("sin", 0) | ("cos", 0) | ("tan", 0) | ("ln", 0) | ("signum", 0) => {
                Some(Type::Float)
            }
            ("log2", 0) | ("log10", 0) | ("exp", 0) | ("exp2", 0) | ("cbrt", 0) => {
                Some(Type::Float)
            }
            ("asin", 0) | ("acos", 0) | ("atan", 0) => Some(Type::Float),
            ("atan2", 1) | ("log", 1) => Some(Type::Float),
            ("is_nan", 0) | ("is_infinite", 0) | ("is_finite", 0) => Some(Type::Bool),
            ("is_positive", 0) | ("is_negative", 0) | ("is_zero", 0) => Some(Type::Bool),
            ("to_int", 0) | ("to_i64", 0) => Some(Type::Int),
            ("to_degrees", 0) | ("to_radians", 0) => Some(Type::Float),
            ("min", 1) | ("max", 1) | ("clamp", 2) => Some(Type::Float),
            ("pow", 1) | ("powi", 1) | ("hypot", 1) => Some(Type::Float),
            ("pi", 0) | ("e", 0) | ("epsilon", 0) => Some(Type::Float),
            ("infinity", 0) | ("neg_infinity", 0) | ("nan", 0) => Some(Type::Float),
            ("max_value", 0) | ("min_value", 0) => Some(Type::Float),
            ("MIN", 0)
            | ("MAX", 0)
            | ("EPSILON", 0)
            | ("INFINITY", 0)
            | ("NEG_INFINITY", 0)
            | ("NAN", 0) => Some(Type::Float),
            ("BITS", 0) | ("MIN_POSITIVE", 0) => Some(Type::Int),
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        "bool" => match (method, arg_count) {
            ("and_then", 1) | ("or_else", 1) => Some(Type::Bool),
            // NOTE: select<T> is a generic method - must fall through to proper method resolution
            // so the type variable T is correctly inferred from arguments
            ("xor", 1) => Some(Type::Bool),
            ("to_int", 0) => Some(Type::Int),
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        "char" => match (method, arg_count) {
            ("is_alphabetic", 0) | ("is_alphanumeric", 0) | ("is_numeric", 0) => Some(Type::Bool),
            ("is_uppercase", 0) | ("is_lowercase", 0) | ("is_whitespace", 0) => Some(Type::Bool),
            ("is_ascii", 0) | ("is_ascii_alphabetic", 0) | ("is_ascii_alphanumeric", 0) => {
                Some(Type::Bool)
            }
            ("is_ascii_digit", 0) | ("is_ascii_whitespace", 0) => Some(Type::Bool),
            ("to_uppercase", 0) | ("to_lowercase", 0) => Some(Type::Char),
            ("to_ascii_uppercase", 0) | ("to_ascii_lowercase", 0) => Some(Type::Char),
            ("to_digit", 1) => Some(make_maybe_type(Type::Int)),
            ("from_digit", 1) | ("from_digit", 2) => Some(make_maybe_type(Type::Char)),
            ("len_utf8", 0) | ("len_utf16", 0) => Some(Type::Int),
            ("is_control", 0) | ("is_digit", 1) => Some(Type::Bool),
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        "byte" => match (method, arg_count) {
            ("to_int", 0) => Some(Type::Int),
            ("is_ascii", 0) | ("is_ascii_alphabetic", 0) | ("is_ascii_digit", 0) => {
                Some(Type::Bool)
            }
            ("min_value", 0) | ("max_value", 0) | ("MIN", 0) | ("MAX", 0) | ("BITS", 0) => {
                Some(Type::Int)
            }
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        _ => None,
    }
}

// Tests moved to tests/infer_tests.rs

// ---------------------------------------------------------------------------
// Mount-cycle-detection regression tests (SIGBUS fix, 2026-04-24).
// ---------------------------------------------------------------------------


// =============================================================================
// Audit-A4: meta-value → AST literal conversion (file-scope free function)
// =============================================================================

/// Convert a `MetaValue` (the const-generic environment's binding type) to
/// an AST `Literal` so a refinement predicate's `Path(N)` can be rewritten
/// to a literal at substitution time.
///

/// Returns `None` for `MetaValue` shapes that have no direct literal
/// representation (compound types, AST values). The caller leaves the path
/// unchanged in that case so SMT continues to see a symbolic reference.
pub(crate) fn meta_value_to_literal(value: &verum_ast::MetaValue) -> Option<verum_ast::literal::Literal> {
    use verum_ast::literal::{FloatLit, IntLit, Literal, LiteralKind, StringLit};
    use verum_ast::span::Span;
    let span = Span::dummy();
    match value {
        verum_ast::MetaValue::Bool(b) => Some(Literal::new(LiteralKind::Bool(*b), span)),
        verum_ast::MetaValue::Int(i) => Some(Literal::new(
            LiteralKind::Int(IntLit {
                value: *i,
                suffix: None,
            }),
            span,
        )),
        // UInt is folded into Int (i128 covers practical const-generic ranges).
        verum_ast::MetaValue::UInt(u) => Some(Literal::new(
            LiteralKind::Int(IntLit {
                value: (*u) as i128,
                suffix: None,
            }),
            span,
        )),
        verum_ast::MetaValue::Float(f) => Some(Literal::new(
            LiteralKind::Float(FloatLit {
                value: *f,
                suffix: None,
            }),
            span,
        )),
        verum_ast::MetaValue::Char(c) => Some(Literal::new(LiteralKind::Char(*c), span)),
        verum_ast::MetaValue::Text(t) => Some(Literal::new(
            LiteralKind::Text(StringLit::Regular(t.clone())),
            span,
        )),
        _ => None,
    }
}

// ============================================================================
// T2-extended-perf: lazy stdlib type registration helpers
// ============================================================================

/// Scan a top-level [`verum_ast::Item`] for every named type
/// reference (in field types, function signatures, type
/// declarations, etc.) and accumulate the bare names into `out`.
///
/// Used by [`TypeChecker::register_stdlib_types_for_module`] to
/// build the closure of stdlib types the user module references,
/// so only those are pulled out of `core_metadata` (skipping the
/// other 99% of stdlib types every cold start used to register
/// upfront).
pub(crate) fn collect_named_types_from_item(
    item: &verum_ast::Item,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::ItemKind;
    match &item.kind {
        ItemKind::Function(f) => {
            for p in f.params.iter() {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    collect_named_types_from_ty(ty, out);
                }
            }
            if let Some(rt) = f.return_type.as_ref() {
                collect_named_types_from_ty(rt, out);
            }
            if let verum_common::Maybe::Some(body) = &f.body {
                collect_named_types_from_function_body(body, out);
            }
        }
        ItemKind::Type(td) => {
            // Field / variant payload types pull in their referenced
            // names so the user-defined type's transitive closure
            // through stdlib ends up loaded.
            collect_named_types_from_type_decl_body(&td.body, out);
        }
        ItemKind::Const(c) => {
            collect_named_types_from_ty(&c.ty, out);
        }
        ItemKind::Static(s) => {
            collect_named_types_from_ty(&s.ty, out);
        }
        ItemKind::Mount(_) => {
            // mount declarations carry symbol names, not type names —
            // the symbols themselves get resolved through other
            // registration paths.  No type-name harvest here.
        }
        ItemKind::Impl(impl_decl) => {
            collect_named_types_from_impl_kind(&impl_decl.kind, out);
            for it in impl_decl.items.iter() {
                collect_named_types_from_impl_item(it, out);
            }
        }
        ItemKind::Protocol(_)
        | ItemKind::Module(_)
        | ItemKind::Theorem(_)
        | ItemKind::Lemma(_)
        | ItemKind::Corollary(_)
        | ItemKind::Axiom(_) => {
            // Less common in user scripts; the lazy loader will
            // catch them via direct lookup-on-miss when needed.
        }
        _ => {}
    }
}

pub(crate) fn collect_named_types_from_ty(
    ty: &verum_ast::ty::Type,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        TypeKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                out.insert(ident.name.clone());
            }
            // Multi-segment paths: also harvest the LAST segment as
            // a likely type name.  `core.io.fs.Path` brings in
            // `Path`.  The first-segment names tend to be modules,
            // not types, so we don't harvest them.
            if path.segments.len() > 1 {
                if let Some(verum_ast::ty::PathSegment::Name(last)) = path.segments.last() {
                    out.insert(last.name.clone());
                }
            }
        }
        TypeKind::Generic { base, args } => {
            collect_named_types_from_ty(base, out);
            for a in args {
                if let verum_ast::ty::GenericArg::Type(t) = a {
                    collect_named_types_from_ty(t, out);
                }
            }
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. }
        | TypeKind::Pointer { inner, .. } => {
            collect_named_types_from_ty(inner, out);
        }
        TypeKind::Slice(inner) => {
            collect_named_types_from_ty(inner, out);
        }
        TypeKind::Array { element, .. } => {
            collect_named_types_from_ty(element, out);
        }
        TypeKind::Tuple(elems) => {
            for e in elems {
                collect_named_types_from_ty(e, out);
            }
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for p in params {
                collect_named_types_from_ty(p, out);
            }
            collect_named_types_from_ty(return_type, out);
        }
        _ => {}
    }
}

pub(crate) fn collect_named_types_from_type_decl_body(
    body: &verum_ast::decl::TypeDeclBody,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::decl::TypeDeclBody;
    match body {
        TypeDeclBody::Alias(t) | TypeDeclBody::Newtype(t) => {
            collect_named_types_from_ty(t, out);
        }
        TypeDeclBody::Record(fields) => {
            for f in fields.iter() {
                collect_named_types_from_ty(&f.ty, out);
            }
        }
        TypeDeclBody::Variant(variants) => {
            for v in variants.iter() {
                if let verum_common::Maybe::Some(data) = &v.data {
                    use verum_ast::decl::VariantData;
                    match data {
                        VariantData::Tuple(tys) => {
                            for t in tys.iter() {
                                collect_named_types_from_ty(t, out);
                            }
                        }
                        VariantData::Record(fields) => {
                            for f in fields.iter() {
                                collect_named_types_from_ty(&f.ty, out);
                            }
                        }
                    }
                }
            }
        }
        TypeDeclBody::Tuple(tys) | TypeDeclBody::SigmaTuple(tys) => {
            for t in tys.iter() {
                collect_named_types_from_ty(t, out);
            }
        }
        TypeDeclBody::Protocol(_) | TypeDeclBody::Unit => {}
        // Less common forms — pull names conservatively where shape is known.
        _ => {}
    }
}

/// Walk a `verum_ast::decl::ImplItem` (a function / type / const
/// inside an impl block) and harvest its named type references.
pub(crate) fn collect_named_types_from_impl_item(
    item: &verum_ast::decl::ImplItem,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::decl::ImplItemKind;
    match &item.kind {
        ImplItemKind::Function(f) => {
            for p in f.params.iter() {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    collect_named_types_from_ty(ty, out);
                }
            }
            if let Some(rt) = f.return_type.as_ref() {
                collect_named_types_from_ty(rt, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_named_types_from_impl_kind(
    kind: &verum_ast::decl::ImplKind,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::decl::ImplKind;
    match kind {
        ImplKind::Inherent(ty) => {
            collect_named_types_from_ty(ty, out);
        }
        ImplKind::Protocol {
            protocol,
            for_type,
            ..
        } => {
            if let Some(ident) = protocol.as_ident() {
                out.insert(ident.name.clone());
            }
            collect_named_types_from_ty(for_type, out);
        }
    }
}

pub(crate) fn collect_named_types_from_function_body(
    _body: &verum_ast::FunctionBody,
    _out: &mut std::collections::HashSet<Text>,
) {
    // Function body harvest is intentionally NOT recursed into for
    // the V0 lazy pre-pass — function-body type references go
    // through the bare `ast_to_type` path which falls back to
    // `Type::Named` opaque on miss, and then through the path
    // resolution in expression typechecking.  Adding deep body
    // walking here would re-introduce most of the eager-load cost.
    //
    // Real-world scripts: function signatures + record/variant
    // fields cover ~95% of stdlib type references; body-only
    // references (e.g., a transient `let x: Maybe<Int> = ...`
    // inside a function) are picked up by the lookup-on-miss
    // fallback at typecheck time.
}

/// Per-variant signature registration extracted from
/// `load_stdlib_from_metadata` so the lazy loader can register one
/// type's variants without walking the entire stdlib.  Mirrors the
/// eager loader's behaviour for that single type.
/// Parse a `archive_metadata::type_ref_to_text` output string
/// back into a structured `Type`.
///
/// **Stdlib-agnostic**: no hardcoded type names.  Built-in
/// primitive type names (`Int`, `Float`, `Bool`, `Char`, `Text`,
/// `Unit`, …) are registered as `Type::*` variants in
/// `ctx.type_defs` by `register_primitives`; user-side resolution
/// via `Type::Named` lookup recovers them at unify time.  This
/// parser is a pure structural decoder for compound type strings:
///
/// * empty / `"()"` → `Type::Unit` (the single language-level
///   special case — `()` is a sigil, not a type name);
/// * `"&T"` / `"&mut T"` → `Type::Reference` over the parsed
///   inner type;
/// * `"Base<arg1, arg2, …>"` → `Type::Generic` with parsed args
///   (top-level commas only, nested generics handled via
///   depth counter);
/// * bare identifiers → `Type::Named` — the unifier's
///   `try_expand_alias` and `ctx.lookup_type` resolve these
///   against the user's type registry.
///
/// Without this parser, signatures stored as compound strings
/// degrade to opaque `Type::Named { path: "IoResult<Metadata>" }`
/// blobs that never unify with `Type::Generic { name: "IoResult",
/// args: [Type::Named { Metadata }] }` at call sites.
thread_local! {
    /// Per-function-scheme interning map for `__generic_N` placeholders.
    /// `Some` while inside [`with_generic_var_scope`]; `None` otherwise.
    static GENERIC_VAR_SCOPE: std::cell::RefCell<Option<indexmap::IndexMap<String, crate::ty::TypeVar>>> =
        const { std::cell::RefCell::new(None) };
}

/// Intern a `__generic_N` placeholder to a stable `TypeVar` for the duration of
/// the current [`with_generic_var_scope`].  Returns `None` when no scope is
/// active (caller then falls back to a fresh var, preserving legacy behaviour).
fn generic_var_scope_intern(name: &str) -> Option<crate::ty::TypeVar> {
    GENERIC_VAR_SCOPE.with(|s| {
        let mut guard = s.borrow_mut();
        guard.as_mut().map(|map| {
            *map.entry(name.to_string())
                .or_insert_with(crate::ty::TypeVar::fresh)
        })
    })
}

thread_local! {
    // SLICE-METHOD-TYPECHECK-E400 (#51): the descriptor's DECLARED
    // generic-param names (impl-level + method-level), active while
    // parsing that descriptor's signature strings. Carried verbatim
    // return names (RETNAME-CARRY, e.g. "&[T]") spell generics by
    // SOURCE NAME rather than the __generic_N placeholder — a bare
    // "T" must intern to the scope TypeVar, not parse as Named{T}.
    static DECLARED_GENERIC_NAMES: std::cell::RefCell<Option<std::collections::HashSet<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether `name` is a declared generic param of the descriptor whose
/// signature is currently being parsed (see DECLARED_GENERIC_NAMES).
fn is_declared_generic_name(name: &str) -> bool {
    DECLARED_GENERIC_NAMES.with(|s| {
        s.borrow()
            .as_ref()
            .is_some_and(|set| set.contains(name))
    })
}

/// Run `f` with `names` registered as the active descriptor's declared
/// generic-param names. Composes with `with_generic_var_scope_capture`
/// (the interning map itself); nesting saves/restores.
pub(crate) fn with_declared_generic_names<R>(
    names: std::collections::HashSet<String>,
    f: impl FnOnce() -> R,
) -> R {
    let prev = DECLARED_GENERIC_NAMES.with(|s| s.borrow_mut().replace(names));
    let out = f();
    DECLARED_GENERIC_NAMES.with(|s| *s.borrow_mut() = prev);
    out
}

/// Run `f` with a fresh generic-var interning scope so that repeated
/// `__generic_N` placeholders parsed by [`parse_descriptor_type_string`] within
/// it map to the SAME `TypeVar`.  Used by the metadata-driven function-scheme
/// builders to parse a function's params AND return under one scope, keeping a
/// generic param linked to a projection over it (`F` ↔ `F.Output`).  Nesting is
/// supported: the previous scope is saved and restored.
pub(crate) fn with_generic_var_scope<R>(f: impl FnOnce() -> R) -> R {
    with_generic_var_scope_capture(f).0
}

/// Like [`with_generic_var_scope`], but ALSO returns the scope's
/// placeholder → `TypeVar` interning map (insertion-ordered =
/// appearance order in the parsed signature).
///
/// Pillar-3 increment 1 (ARRAY-ITER-CONCRETIZE-1): the metadata
/// scheme-birth site needs to know WHICH TypeVar each `__generic_i`
/// placeholder produced so it can match the carried
/// `FunctionDescriptor::impl_generic_names` (impl-level params are
/// serialised as `__generic_i` with `i < impl_generic_names.len()`
/// by construction — see the field's doc) and order impl-level vars
/// first in the resulting `TypeScheme`.
pub(crate) fn with_generic_var_scope_capture<R>(
    f: impl FnOnce() -> R,
) -> (R, indexmap::IndexMap<String, crate::ty::TypeVar>) {
    let prev = GENERIC_VAR_SCOPE.with(|s| s.borrow_mut().replace(indexmap::IndexMap::new()));
    let result = f();
    let scope = GENERIC_VAR_SCOPE
        .with(|s| s.borrow_mut().take())
        .unwrap_or_default();
    GENERIC_VAR_SCOPE.with(|s| *s.borrow_mut() = prev);
    (result, scope)
}

/// ONE authority for turning a METADATA-reconstructed FREE-FUNCTION
/// signature (already parsed to `fn_ty` under
/// [`with_generic_var_scope_capture`], which yields `scope_vars`) into its
/// quantified [`crate::context::TypeScheme`].
///
/// Two coupled properties this guarantees — both regressions the previous
/// birth site (`dependent_helpers::collect_type_vars` into a hashed `Set`)
/// violated, which made mounted stdlib generics such as
/// `simd_extract<V, T>(vec: V, idx: UInt32) -> T` type-check
/// NON-DETERMINISTICALLY (same fixed archive, only ~1/3 of runs green):
///
///  1. **Deterministic quantification order.**  Vars are read from
///     `scope_vars`, whose iteration order is INSERTION order = first
///     appearance in the signature (params left-to-right, then return).
///     `collect_type_vars` returned a `verum_common::Set` (hashed) whose
///     per-process-random iteration order let one two-generic call
///     `f<A, B>(..)` bind its arguments as `[V, T]` on one run and
///     `[T, V]` (or onto a degraded var) on the next.
///
///  2. **Only declared generics are explicit.**  A placeholder spelled
///     `__generic_N`, or a bare name listed in `declared_generic_names`,
///     is a DECLARED type parameter — the only kind a caller may bind with
///     an explicit `f<..>(..)` type argument.  Any OTHER interned var
///     comes from `parse_descriptor_type_string` mapping a
///     `__opaque_type_N` spelling (a concrete VBC `TypeId` the archive
///     could not name, e.g. `UInt32`) to a fresh var; that var is an
///     EXISTENTIAL to be inferred from the call arguments, never a
///     user-facing generic.  It is recorded in `implicit_vars` so the
///     call-site explicit-type-argument binder (`infer/expr.rs`) SKIPS it.
///     Otherwise the spurious var steals a positional `<..>` slot, the
///     real generics shift, and an unrelated argument is checked against
///     the wrong binding — e.g. `simd_extract`'s `idx: UInt32` collapses
///     onto `V` and reports `expected 'Float', found 'Int'`.
///
/// Declared generics come first (appearance order); existentials follow.
/// Every var stays quantified (freshly instantiated per use); `implicit`
/// governs ONLY explicit-type-argument binding, so a call WITHOUT `<..>`
/// args is byte-for-byte unaffected.
pub(crate) fn build_metadata_function_scheme(
    fn_ty: Type,
    scope_vars: &indexmap::IndexMap<String, crate::ty::TypeVar>,
    declared_generic_names: &std::collections::HashSet<String>,
) -> crate::context::TypeScheme {
    use crate::context::TypeScheme;
    use crate::ty::TypeVar;
    let is_declared = |placeholder: &str| -> bool {
        placeholder.starts_with("__generic_") || declared_generic_names.contains(placeholder)
    };
    let mut explicit: List<TypeVar> = List::new();
    let mut implicit: List<TypeVar> = List::new();
    for (placeholder, tv) in scope_vars.iter() {
        if is_declared(placeholder.as_str()) {
            explicit.push(*tv);
        } else {
            implicit.push(*tv);
        }
    }
    if explicit.is_empty() && implicit.is_empty() {
        return TypeScheme::mono(fn_ty);
    }
    let mut implicit_set: Set<TypeVar> = Set::new();
    for tv in implicit.iter() {
        implicit_set.insert(*tv);
    }
    let mut var_list: List<TypeVar> = explicit;
    for tv in implicit.iter() {
        var_list.push(*tv);
    }
    TypeScheme::poly_with_implicit(var_list, fn_ty, implicit_set)
}

#[cfg(test)]
mod build_metadata_function_scheme_tests {
    use super::build_metadata_function_scheme;
    use crate::ty::{Type, TypeVar};
    use verum_common::List;

    // A `__opaque_type_N` existential in the middle of the signature must
    // NOT be quantified as an explicit generic, and the declared generics
    // must be ordered by appearance — deterministically — regardless of
    // TypeVar identity.  Models the baked `simd_extract` descriptor
    // `(vec: __generic_1, idx: __opaque_type_8) -> __generic_0`.
    #[test]
    fn opaque_param_is_implicit_and_order_is_deterministic() {
        // Run the reconstruction many times; the partition + ordering must
        // be identical every time (no dependence on hashed-Set iteration).
        let mut shapes = std::collections::HashSet::new();
        for _ in 0..64 {
            let v_vec = TypeVar::fresh();
            let v_idx = TypeVar::fresh();
            let v_ret = TypeVar::fresh();
            let mut scope: indexmap::IndexMap<String, TypeVar> = indexmap::IndexMap::new();
            scope.insert("__generic_1".to_string(), v_vec); // vec: V, appears first
            scope.insert("__opaque_type_8".to_string(), v_idx); // idx: UInt32 (opaque)
            scope.insert("__generic_0".to_string(), v_ret); // return: T
            let fn_ty = Type::function(
                List::from(vec![Type::Var(v_vec), Type::Var(v_idx)]),
                Type::Var(v_ret),
            );
            let declared: std::collections::HashSet<String> =
                ["V", "T"].iter().map(|s| s.to_string()).collect();
            let scheme = build_metadata_function_scheme(fn_ty, &scope, &declared);

            // Two explicit generics (V, T) in appearance order, existential last.
            let vars: Vec<TypeVar> = scheme.vars.iter().copied().collect();
            assert_eq!(vars, vec![v_vec, v_ret, v_idx]);
            assert_eq!(scheme.explicit_var_count(), 2);
            assert!(scheme.implicit_vars.contains(&v_idx));
            assert!(!scheme.implicit_vars.contains(&v_vec));
            assert!(!scheme.implicit_vars.contains(&v_ret));

            // Record a structural fingerprint (positions of explicit vars).
            let idx_of = |t: TypeVar| vars.iter().position(|x| *x == t).unwrap();
            shapes.insert((idx_of(v_vec), idx_of(v_ret), idx_of(v_idx)));
        }
        assert_eq!(shapes.len(), 1, "scheme shape must be deterministic");
    }

    // A bare declared source name ("T") interns by name (RETNAME-CARRY) and
    // must still be treated as an explicit generic, not an existential.
    #[test]
    fn declared_source_name_is_explicit() {
        let v_t = TypeVar::fresh();
        let v_op = TypeVar::fresh();
        let mut scope: indexmap::IndexMap<String, TypeVar> = indexmap::IndexMap::new();
        scope.insert("T".to_string(), v_t);
        scope.insert("__opaque_type_3".to_string(), v_op);
        let fn_ty = Type::function(
            List::from(vec![Type::Var(v_t), Type::Var(v_op)]),
            Type::Var(v_t),
        );
        let declared: std::collections::HashSet<String> =
            ["T"].iter().map(|s| s.to_string()).collect();
        let scheme = build_metadata_function_scheme(fn_ty, &scope, &declared);
        assert_eq!(scheme.explicit_var_count(), 1);
        assert!(!scheme.implicit_vars.contains(&v_t));
        assert!(scheme.implicit_vars.contains(&v_op));
    }

    // No interned vars at all -> monomorphic scheme (unchanged behaviour).
    #[test]
    fn no_vars_is_mono() {
        let scope: indexmap::IndexMap<String, TypeVar> = indexmap::IndexMap::new();
        let fn_ty = Type::function(List::from(vec![Type::Int]), Type::Bool);
        let declared: std::collections::HashSet<String> = std::collections::HashSet::new();
        let scheme = build_metadata_function_scheme(fn_ty, &scope, &declared);
        assert!(scheme.vars.is_empty());
        assert!(scheme.implicit_vars.is_empty());
    }
}

pub(crate) fn parse_descriptor_type_string(raw: &str) -> Type {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "()" {
        return Type::Unit;
    }
    // VBC opaque-type fallbacks → fresh type variable.  The
    // string `"__opaque_type_N"` (from `archive_metadata::type_ref_to_text`'s
    // fallback for unmapped concrete TypeIds) and `"__generic_N"`
    // (TypeRef::Generic(N) without a param-name map) both
    // represent "VBC didn't resolve this further" — the unifier
    // should treat them as fresh type variables so they unify
    // with any concrete type at use sites.  Without this, every
    // method signature carrying an unresolved cross-module
    // TypeId fails downstream unification (`expected
    // '__opaque_type_14', found 'Text'`).
    if trimmed.starts_with("__opaque_type_")
        || trimmed.starts_with("__generic_")
        || trimmed == "__opaque_typeref"
        // Source-scan injector spelling (precompile.rs render_type's
        // exotic-shape fallback). Same contract as the archive
        // spellings above: opaque = unknown = fresh var; a rigid
        // Named("__opaque_src") fails every use-site unification
        // (`sqrt(9.0)` → E400 "expected '__opaque_src', found
        // 'Float'" — UMBRELLA-REEXPORT-RESOLVE-1 one-hop probe).
        || trimmed == "__opaque_src"
    {
        // Within a function-scheme scope (`with_generic_var_scope`), the SAME
        // `__generic_N` placeholder must map to the SAME TypeVar across the
        // params and the return type so a generic param `F` stays linked to a
        // projection over it (`F.Output`).  Without this, `fn(&mut __generic_5)
        // -> Maybe<::Output<__generic_5>>` parses the two occurrences to
        // DISTINCT fresh vars, the return payload never resolves from the bound
        // `F`, and it defaults to Int (async future_poll_sync suite).  Outside a
        // scope, keep the historic fresh-per-occurrence behaviour.
        if let Some(tv) = generic_var_scope_intern(trimmed) {
            return Type::Var(tv);
        }
        return Type::Var(crate::ty::TypeVar::fresh());
    }
    // References: "&[checked|unsafe] [mut] T" (grammar tier spellings) and
    // the plain "&mut T" / "&T".  ARCHIVE-REF-TIER-DROP-1: the metadata
    // writer (`archive_metadata::type_ref_to_text`) renders the CBGR tier
    // faithfully; the tiered arms MUST come before the generic `&` arm or
    // "&unsafe T" strips to a garbage `Named { "unsafe T" }`.
    if let Some(rest) = trimmed.strip_prefix("&unsafe mut ") {
        return Type::UnsafeReference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: true,
        };
    }
    if let Some(rest) = trimmed.strip_prefix("&unsafe ") {
        return Type::UnsafeReference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: false,
        };
    }
    if let Some(rest) = trimmed.strip_prefix("&checked mut ") {
        return Type::CheckedReference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: true,
        };
    }
    if let Some(rest) = trimmed.strip_prefix("&checked ") {
        return Type::CheckedReference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: false,
        };
    }
    if let Some(rest) = trimmed.strip_prefix("&mut ") {
        return Type::Reference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: true,
        };
    }
    if let Some(rest) = trimmed.strip_prefix('&') {
        return Type::Reference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: false,
        };
    }
    // #51: tuple spelling "(A, B, …)" — carried source-verbatim return
    // names (`split_at() -> (&[T], &[T])`) reach this parser; without a
    // tuple arm the whole spelling degraded to a non-tuple and every
    // `let (l, r) = s.split_at(..)` failed "Expected tuple type for
    // tuple pattern". Unit "()" is handled at the top of the fn.
    if trimmed.starts_with('(') && trimmed.ends_with(')') && trimmed.len() > 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        let parts = split_top_level_commas(inner);
        // A single part with no top-level comma is a PARENTHESISED
        // type, not a 1-tuple — parse the inner type transparently.
        if parts.len() == 1 {
            return parse_descriptor_type_string(parts[0].trim());
        }
        let elems: List<Type> = parts
            .into_iter()
            .map(|p| parse_descriptor_type_string(p.trim()))
            .collect();
        return Type::Tuple(elems);
    }
    // SLICE-METHOD-TYPECHECK-E400 (#51): slice spelling "[T]" — the
    // carried source-verbatim return names ("&[T]" via RETNAME-CARRY)
    // reach this parser now that the metadata writer prefers them over
    // the lossy TypeRef render (VBC TypeRefs have NO slice form, so
    // `slice() -> &[T]` used to serialise as bare "List" and the
    // chained receiver misinfered as List<_>).
    if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        // "[T; N]" (array) — element is everything before the
        // top-level ';'. Metadata keeps no length; Slice is the
        // honest structural reading for method-signature purposes.
        let elem_text = match inner.rfind(';') {
            Some(semi) if !inner[..semi].contains('[') => inner[..semi].trim_end(),
            _ => inner,
        };
        return Type::Slice {
            element: Box::new(parse_descriptor_type_string(elem_text.trim())),
        };
    }
    // **Function-type spelling** (`fn(arg1, arg2, ...) -> ret`) — must
    // be parsed BEFORE the generic-instantiation `<>` check below, since
    // `fn(A) -> Maybe<T>` ends with `>` and would otherwise be mis-
    // captured as `Type::Named { path: "fn(A) -> Maybe", args: [T] }`.
    //
    // Serialised by `archive_metadata::type_ref_to_text_with_params`'s
    // `TypeRef::Function` arm (around line ~1059) using the form
    // `fn(<args>) -> <ret>`.  Required for stdlib HOF metadata-side
    // `F: fn(A) -> B` bound recovery (task #5/#13 §F).
    //
    // Parse shape: find the matching `)` at depth 0, split args at
    // top-level commas, parse args + return-type recursively.
    if let Some(rest) = trimmed.strip_prefix("fn(") {
        let bytes = rest.as_bytes();
        let mut depth = 1usize;
        let mut close_idx: Option<usize> = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' | b'<' | b'[' => depth += 1,
                b')' if depth == 1 => {
                    close_idx = Some(i);
                    break;
                }
                b')' | b'>' | b']' => depth -= 1,
                _ => {}
            }
        }
        if let Some(close) = close_idx {
            let args_text = &rest[..close];
            let after = rest[close + 1..].trim_start();
            let ret_text = after
                .strip_prefix("->")
                .map(|s| s.trim_start())
                .unwrap_or("");
            let params: List<Type> = if args_text.trim().is_empty() {
                List::new()
            } else {
                split_top_level_commas(args_text)
                    .into_iter()
                    .map(|s| parse_descriptor_type_string(s.trim()))
                    .collect()
            };
            let return_type = if ret_text.is_empty() {
                Type::Unit
            } else {
                parse_descriptor_type_string(ret_text)
            };
            return Type::Function {
                params,
                return_type: Box::new(return_type),
                contexts: None,
                type_params: List::new(),
                properties: None,
            };
        }
    }
    // Generic instantiation: "Base<arg1, arg2, ...>".
    //
    // Task #25 — canonical form is `Type::Named { path, args }`, NOT
    // `Type::Generic { name, args }`.  The source-driven path
    // (`ast_to_generic_type`) emits Named when the base resolves to
    // a named stdlib type (Maybe / Result / List / Map / …); the
    // method-dispatch carrier extraction and the inherent_methods
    // bucket lookup both probe Named-form first.  Emitting Generic
    // here means `metadata.functions["arg"]` returns `fn(Int) ->
    // Type::Generic{Maybe, [Text]}` while the call site is checked
    // against `Type::Named{Maybe, [Text]}`, so `Maybe.is_none()`
    // dispatch fails with "no method named is_none found for type
    // Text" (the unifier unwraps the wrong way and reports the
    // generic-arg type at the failure site).
    //
    // `Type::Generic` remains the right form for HKT-style
    // projections (`::Item`, `::F`) where the "name" isn't a path
    // segment.  Those never round-trip through
    // `parse_descriptor_type_string` because they're synthesized
    // by the typechecker, never serialised as text by
    // `archive_metadata::type_ref_to_text`.
    if let Some(open) = trimmed.find('<') {
        if trimmed.ends_with('>') {
            let base = &trimmed[..open];
            let inside = &trimmed[open + 1..trimmed.len() - 1];
            let args = split_top_level_commas(inside)
                .into_iter()
                .map(|s| parse_descriptor_type_string(s.trim()))
                .collect();
            // Associated-type projection spelling `::Assoc<Base>` (emitted by
            // `core_loader::type_ref_to_text` for `TypeRef::AssociatedProjection`).
            // Reconstruct the unifier's internal projection form
            // `Type::Generic { name: "::Assoc", args: [Base] }` so a bound base
            // (the `F` in `F.Output`) still resolves the associated type — the
            // fix for the async future_poll_sync payload defaulting to Int.
            if base.starts_with("::") {
                return Type::Generic {
                    name: Text::from(base),
                    args,
                };
            }
            return Type::Named {
                path: TypeChecker::text_to_path(&Text::from(base)),
                args,
            };
        }
    }
    // Language primitives — these have dedicated `TypeKind`
    // variants in the AST (`TypeKind::Bool`, `TypeKind::Int`, …)
    // and dedicated `Type::Bool` / `Type::Int` / … sentinels in
    // the type system.  They are NOT stdlib types — the grammar
    // parses them as built-ins distinct from `TypeKind::Path`.
    // Without this normalisation, archive_metadata's textual
    // payloads ("Bool" / "Int" / …) round-trip through the
    // structural parser as `Type::Named { path: "Bool" }` and
    // every downstream check that branches on `Type::Bool`
    // (logical NOT, integer arithmetic, etc.) misses them.
    match trimmed {
        "Bool" => return Type::Bool,
        "Int" => return Type::Int,
        "Float" => return Type::Float,
        "Char" => return Type::Char,
        "Text" => return Type::Text,
        "Never" => return Type::Never,
        "Unit" => return Type::Unit,
        _ => {}
    }
    // #51: a bare name that IS a declared generic param of the
    // descriptor being parsed (source-verbatim carried strings spell
    // "T", not "__generic_0") interns to the scope's TypeVar so the
    // param and return occurrences stay linked.
    if is_declared_generic_name(trimmed) {
        if let Some(tv) = generic_var_scope_intern(trimmed) {
            return Type::Var(tv);
        }
    }
    // Other named types → Type::Named.  Resolution flows
    // through the unifier's lookup against `ctx.type_defs`
    // populated by `register_primitives`.
    Type::Named {
        path: TypeChecker::text_to_path(&Text::from(trimmed)),
        args: List::new(),
    }
}

/// Split a string on commas at depth=0 (top-level), respecting
/// nesting in `<>`/`()`/`[]`.  Used by
/// `parse_descriptor_type_string` for generic-arg lists.
pub(crate) fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut depth: i32 = 0;
    let mut start = 0;
    let mut out = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

pub(crate) fn register_variant_signature_for_lazy(
    checker: &mut TypeChecker,
    name: &Text,
    type_desc: &crate::core_metadata::TypeDescriptor,
    cases: &List<crate::core_metadata::VariantCase>,
    pending: &mut Vec<Text>,
) {
    // Push payload type names → pending so the lazy loader
    // closure picks them up.
    for case in cases.iter() {
        if let Maybe::Some(payload) = &case.payload {
            match payload {
                crate::core_metadata::VariantPayload::Tuple(types) => {
                    for t in types.iter() {
                        if !t.is_empty() {
                            pending.push(t.clone());
                        }
                    }
                }
                crate::core_metadata::VariantPayload::Record(fields) => {
                    for f in fields.iter() {
                        if !f.ty.is_empty() {
                            pending.push(f.ty.clone());
                        }
                    }
                }
            }
        }
    }

    // #126 — generic-parameter substitution at variant construction.
    //
    // Map every parent generic-param NAME (e.g. `"T"`, `"A"`, `"E"`)
    // to a freshly-allocated `TypeVar`.  The variant payload types
    // are then built using these vars instead of literal
    // `Type::Named { path: "T" }` placeholders.  When the unit-variant
    // env entry is inserted as a `TypeScheme::poly` quantified over
    // the same vars, every lookup yields a freshly-instantiated
    // `Type::Variant` whose generic positions are fresh `Type::Var`s
    // — exactly what the unifier expects at `mapped == None` sites.
    //
    // Pre-fix this function inserted the variant_type as a `mono`
    // scheme whose payload positions held rigid `Type::Named "T"`,
    // so `Maybe<Int> == None` failed with `expected 'T', found 'Int'`
    // because the unifier compared `Int` (concrete) against `Named "T"`
    // (rigid name) with no rule to unify.
    use indexmap::IndexMap;
    use crate::ty::TypeVar;
    let param_to_var: IndexMap<Text, TypeVar> = type_desc
        .generic_params
        .iter()
        .map(|gp| (gp.name.clone(), TypeVar::fresh()))
        .collect();
    // Task #41 — payload type strings like "List<Byte>" / "Maybe<Text>"
    // / "Result<T, E>" MUST go through the structural parser so the
    // generic head and args land in `Type::Named { path, args }` shape
    // (not a single Ident with the whole generic name).  Generic
    // parameter names ("T", "E", …) still get replaced by the parent's
    // fresh TypeVars via `param_to_var`, but the parser path handles
    // nested generics where the name appears INSIDE another type.
    //
    // For a bare param name (`Box<T>` → "T"), we still bypass the parser
    // and substitute directly so the persistent-TypeVar discipline
    // (variant types share one TypeVar for the parent's generic) holds.
    let resolve_payload_name = |t: &Text| -> Type {
        if let Some(tv) = param_to_var.get(t) {
            return Type::Var(*tv);
        }
        let parsed = parse_descriptor_type_string(t.as_str());
        // Substitute any remaining bare param names that appear inside
        // a parsed generic head (e.g. "List<T>" → Named { List, [Named { T }] }
        // → Named { List, [Var(T_fresh)] }).
        if param_to_var.is_empty() {
            return parsed;
        }
        let subst: indexmap::IndexMap<Text, Type> = param_to_var
            .iter()
            .map(|(name, tv)| (name.clone(), Type::Var(*tv)))
            .collect();
        TypeChecker::substitute_named_params_in_type(&parsed, &subst)
    };

    let mut variant_map: IndexMap<Text, Type> = IndexMap::new();
    for case in cases.iter() {
        let payload_ty = match &case.payload {
            Maybe::None => Type::Unit,
            Maybe::Some(crate::core_metadata::VariantPayload::Tuple(types)) => {
                if types.len() == 1 {
                    resolve_payload_name(&types[0])
                } else {
                    Type::Tuple(types.iter().map(&resolve_payload_name).collect())
                }
            }
            Maybe::Some(crate::core_metadata::VariantPayload::Record(fields)) => {
                let field_map: IndexMap<Text, Type> = fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_payload_name(&f.ty)))
                    .collect();
                Type::Record(field_map)
            }
        };
        variant_map.insert(case.name.clone(), payload_ty);
    }

    let variant_type = Type::Variant(variant_map.clone());
    if let Some(sig) = TypeChecker::variant_type_signature(&variant_type) {
        checker.register_variant_type_name_first_wins(sig.clone(), name.clone());
        if let Some(relaxed) = TypeChecker::variant_type_signature_relaxed(&variant_type) {
            if relaxed != sig {
                checker.register_variant_type_name_first_wins(relaxed, name.clone());
            }
        }
    }

    // T0594: the lazy loader must ALSO feed the unifier's
    // original-variant-type + type-var-order registries — exactly as the
    // eager decl path does (decls.rs:1018-1023) — so Named/Generic<->Variant
    // unification takes the SOUND extract_type_var_mapping path rather than
    // `unify_generic_variant_fallback`. That fallback uses "payload != Unit"
    // as a proxy for "this arm carries a generic parameter", which mishandles
    // an all-Unit instantiation such as `Maybe<Unit>`: both `None` and
    // `Some(Unit)` are filtered out, so its arity reads 0 != 1 (args = [Unit])
    // and it raises a spurious E400 ("expected 'Maybe<Unit>', found
    // 'None(Unit) | Some(Unit)'"). `variant_type` already carries `Type::Var`
    // payloads and `param_to_var` is the declaration-order param→var map, so
    // the extractor recovers `T := Unit` from the template. No hardcoded type
    // names (satisfies src/CLAUDE.md).
    checker
        .unifier
        .register_original_variant_type(name.clone(), variant_type.clone());
    checker
        .unifier
        .register_type_var_order(name.clone(), param_to_var.values().copied().collect());

    // Variant constructor parent mappings.
    for (vname, _payload_ty) in &variant_map {
        let parents = checker
            .variant_constructor_parents
            .entry(vname.clone())
            .or_default();
        if !parents.iter().any(|p| p == name) {
            parents.push(name.clone());
        }
    }

    // Register unit-variant constructors as env values (so `None`,
    // `Less`, `Greater`, … resolve as expressions) and ALWAYS the
    // qualified `Type.Variant` form.
    //
    // #126 — when the parent has generic params, the env entry must
    // be a *polymorphic* `TypeScheme` quantified over the same fresh
    // TypeVars that we substituted into the payload positions. This
    // way every `lookup → instantiate` yields a fresh per-call-site
    // copy with independent unification slots.
    use crate::context::TypeScheme;
    let scheme_vars: List<TypeVar> = param_to_var.values().copied().collect();
    let make_scheme = || -> TypeScheme {
        if scheme_vars.is_empty() {
            TypeScheme::mono(variant_type.clone())
        } else {
            TypeScheme::poly(scheme_vars.clone(), variant_type.clone())
        }
    };

    for (vname, payload_ty) in &variant_map {
        if *payload_ty == Type::Unit {
            if checker.ctx.env.lookup(vname.as_str()).is_none() {
                checker.ctx.env.insert(vname.clone(), make_scheme());
            }
        }
        let qualified_name: Text = format!("{}.{}", name, vname).into();
        checker.ctx.env.insert(qualified_name, make_scheme());
    }

    // Payload-bearing variant constructors are NOT registered as
    // env functions here.  The eager `load_stdlib_from_metadata`
    // path (lines 1717-1731) doesn't register them either —
    // dispatch goes through `variant_constructor_parents` (set
    // above) and the typechecker's own variant-resolution path.
    // Pre-fix attempt to register them as `fn(T) -> Variant`
    // typed env entries broke method dispatch on generics
    // (`list.len()`, `maybe.unwrap_or(0)`, …) because the
    // constructor's typed shape interfered with the type-method
    // resolution path.
    //
    // Task #26 — overwrite the rigid `Type::Variant` that
    // `type_descriptor_to_type` (called earlier in
    // `ensure_stdlib_type_loaded`) stored with the polymorphic
    // form built here (payloads carry `Type::Var(fresh)` instead
    // of rigid `Type::Named { "T" }`).  Register the parent's
    // generic-param TypeVars under `__type_var_order_<name>` so
    // `ast_to_generic_type::Type::Variant` substitution at use
    // sites (line ~1573 of types.rs) can recover the declaration-
    // order var list and substitute `Result<Int, Int>`'s args
    // positionally into the variant payloads.
    //
    // Pre-fix `let r: Result<Int, Int> = Err(7)` failed typecheck
    // with `expected 'E', found 'Int'` because the stored
    // variant kept the rigid `Type::Named { "E" }` payload — the
    // substitution loop at types.rs:1573 found no `Type::Var`s
    // (the fallback `collect_type_vars_ordered` only finds vars,
    // not rigid named placeholders), so `subst_map` stayed empty
    // and the literal `7` was checked against rigid `"E"`.
    if !param_to_var.is_empty() {
        checker.ctx.define_type(name.clone(), variant_type.clone());
        let var_order_key: Text = format!("__type_var_order_{}", name).into();
        let tvars_in_order: List<Type> = type_desc
            .generic_params
            .iter()
            .filter_map(|gp| param_to_var.get(&gp.name).map(|tv| Type::Var(*tv)))
            .collect();
        checker
            .ctx
            .define_type(var_order_key, Type::Tuple(tvars_in_order));
    }
}
