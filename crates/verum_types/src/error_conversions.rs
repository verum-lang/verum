//! Conversions from verum_types::TypeError to verum_error::VerumError
//!
//! This module implements the `From` trait to convert type checking errors
//! to the unified VerumError type, enabling seamless error propagation across
//! crate boundaries.

use crate::TypeError;
use verum_error::unified::VerumError;

impl From<TypeError> for VerumError {
    fn from(err: TypeError) -> Self {
        match err {
            // Direct mappings to VerumError variants
            TypeError::Mismatch {
                expected, actual, ..
            } => VerumError::TypeMismatch {
                expected: expected.clone(),
                actual: actual.clone(),
            },
            TypeError::CannotInferLambda { .. } => VerumError::CannotInferLambda,
            TypeError::UnboundVariable { name, .. } => VerumError::UnboundVariable {
                name: name.clone(),
            },
            TypeError::NotAFunction { ty, .. } => VerumError::NotAFunction { ty: ty.clone() },
            TypeError::InfiniteType { var, ty, .. } => VerumError::InfiniteType {
                var: var.clone(),
                ty: ty.clone(),
            },
            TypeError::ProtocolNotSatisfied { ty, protocol, .. } => {
                VerumError::ProtocolNotSatisfied {
                    ty: ty.clone(),
                    protocol: protocol.clone(),
                }
            }
            TypeError::RefinementFailed { predicate, .. } => VerumError::RefinementFailed {
                predicate: predicate.clone(),
            },
            TypeError::RefinementPredicateInvalid { message, .. } => VerumError::RefinementFailed {
                predicate: message.clone(),
            },
            TypeError::AffineViolation { ty, .. } => {
                VerumError::AffineViolation { ty: ty.clone() }
            }
            TypeError::MissingContext { context, .. } => VerumError::MissingContext {
                context: context.clone(),
            },

            // Branch mismatch is a type mismatch
            TypeError::BranchMismatch {
                then_ty, else_ty, ..
            } => VerumError::TypeMismatch {
                expected: then_ty.clone(),
                actual: else_ty.clone(),
            },

            // Invalid index errors (negative indices, out of bounds)
            TypeError::InvalidIndex { message, .. } => VerumError::Other {
                message: message.clone(),
            },

            // Protocol-related errors
            TypeError::ProtocolNotImplemented {
                ty,
                protocol,
                method,
                ..
            } => VerumError::ProtocolNotSatisfied {
                ty: ty.clone(),
                protocol: format!("{} (for method {})", protocol, method).into(),
            },

            // Try operator errors map to type mismatches
            TypeError::ResultTypeMismatch {
                inner_error,
                outer_error,
                ..
            } => VerumError::TypeMismatch {
                expected: outer_error.clone(),
                actual: inner_error.clone(),
            },

            // All other type errors map to VerumError::Other
            TypeError::LinearViolation {
                ty, usage_count, ..
            } => VerumError::Other {
                message: format!("linear type violation: {} used {} times", ty, usage_count).into(),
            },
            TypeError::MovedValueUsed { name, .. } => VerumError::Other {
                message: format!("value {} used after move", name).into(),
            },
            TypeError::LinearNotConsumed { name, .. } => VerumError::Other {
                message: format!("linear value {} must be consumed exactly once", name).into(),
            },
            TypeError::UndefinedContext { name, .. } => VerumError::Other {
                message: format!("undefined context: {}", name).into(),
            },
            TypeError::DuplicateProvide { context, .. } => VerumError::Other {
                message: format!("E808: duplicate provide for context '{}' in same scope", context).into(),
            },
            TypeError::UndefinedContextMethod {
                context, method, ..
            } => VerumError::Other {
                message: format!("undefined context method: {}.{}", context, method).into(),
            },
            TypeError::InvalidSubContext {
                context,
                sub_context,
                ..
            } => VerumError::Other {
                message: format!("invalid sub-context: {}.{}", context, sub_context).into(),
            },
            TypeError::ContextMismatch {
                expected, actual, ..
            } => VerumError::Other {
                message: format!("context mismatch: expected {}, found {}", expected, actual).into(),
            },
            TypeError::ContextPropagationError {
                context, callee, ..
            } => VerumError::Other {
                message: format!(
                    "context propagation error: {} requires {} but caller doesn't provide it",
                    callee, context
                ).into(),
            },
            TypeError::ExcludedContextViolation { context, .. } => VerumError::Other {
                message: format!(
                    "excluded context violation: cannot use `{}` which is explicitly excluded",
                    context
                ).into(),
            },
            TypeError::TransitiveNegativeContextViolation {
                excluded_context,
                callee,
                ..
            } => VerumError::Other {
                message: format!(
                    "transitive negative context violation: calling `{}` uses excluded context `{}`",
                    callee, excluded_context
                ).into(),
            },
            TypeError::ContextNotAllowed { context, .. } => VerumError::Other {
                message: format!("context not allowed: {}", context).into(),
            },
            TypeError::ConstMismatch {
                expected, actual, ..
            } => VerumError::Other {
                message: format!(
                    "const generic mismatch: expected {}, found {}",
                    expected, actual
                ).into(),
            },
            TypeError::AmbiguousType { .. } => VerumError::Other {
                message: "ambiguous type: cannot infer without more context".into(),
            },
            TypeError::InvalidCast {
                from, to, reason, ..
            } => VerumError::Other {
                message: format!("invalid cast from {} to {}: {}", from, to, reason).into(),
            },
            TypeError::MethodNotFound { ty, method, .. } => VerumError::Other {
                message: format!("method {} not found for type {}", method, ty).into(),
            },
            TypeError::CapabilityViolation {
                method,
                type_name,
                required_capability,
                available_capabilities,
                ..
            } => {
                let available: Vec<&str> = available_capabilities.iter().map(|c| c.as_str()).collect();
                VerumError::Other {
                    message: format!(
                        "capability violation: method `{}` on `{}` requires `{}` but only [{}] are available",
                        method, type_name, required_capability, available.join(", ")
                    ).into(),
                }
            },
            TypeError::WrongArgCount {
                method,
                expected,
                actual,
                ..
            } => VerumError::Other {
                message: format!(
                    "wrong number of arguments for {}: expected {}, found {}",
                    method, expected, actual
                ).into(),
            },
            TypeError::AmbiguousMethod { method, .. } => VerumError::Other {
                message: format!("ambiguous method call: {}", method).into(),
            },
            TypeError::TypeNotFound { name, .. } => VerumError::Other {
                message: format!("type not found: {}", name).into(),
            },
            TypeError::NotAType {
                name, actual_kind, ..
            } => VerumError::Other {
                message: format!("{} is not a type (it is a {})", name, actual_kind).into(),
            },
            TypeError::TryOperatorOutsideFunction { .. } => VerumError::Other {
                message: "cannot use '?' operator outside of function context".into(),
            },
            TypeError::MultipleConversionPaths {
                from_type, to_type, ..
            } => VerumError::Other {
                message: format!(
                    "multiple conversion paths from {} to {}",
                    from_type, to_type
                ).into(),
            },
            TypeError::TryInNonResultContext {
                expr_type,
                function_return_type,
                ..
            } => VerumError::Other {
                message: format!(
                    "cannot use '?' operator on {} in function returning {}",
                    expr_type, function_return_type
                ).into(),
            },
            TypeError::NotResultOrMaybe { ty, .. } => VerumError::Other {
                message: format!("type {} is not Result or Maybe", ty).into(),
            },
            // Two-pass type resolution errors
            TypeError::TypeCycle { cycle_path, .. } => VerumError::Other {
                message: format!("cyclic type definition detected: {}", cycle_path).into(),
            },
            TypeError::UnresolvedPlaceholder { name, .. } => VerumError::Other {
                message: format!("unresolved type placeholder: {}", name).into(),
            },
            TypeError::IncompleteTypeReference { name, .. } => VerumError::Other {
                message: format!(
                    "type {} is not yet fully defined (forward reference in same declaration)",
                    name
                ).into(),
            },
            // Import Resolution Errors (E401-E403)
            TypeError::ImportItemNotFound { item_name, module_path, .. } => VerumError::Other {
                message: format!("E401: cannot find `{}` in module `{}`", item_name, module_path).into(),
            },
            TypeError::ImportModuleNotFound { module_path, .. } => VerumError::Other {
                message: format!("E402: module `{}` not found", module_path).into(),
            },
            TypeError::UndefinedFunction { func_name, .. } => VerumError::Other {
                message: format!("E403: undefined function `{}`", func_name).into(),
            },

            TypeError::Other(msg) => VerumError::Other {
                message: msg.clone(),
            },
            TypeError::OtherWithCode { msg, .. } => VerumError::Other {
                message: msg.clone(),
            },
            TypeError::NonContextProtocolInUsing { name, .. } => VerumError::Other {
                message: format!(
                    "protocol '{}' cannot be used as a context; use 'context protocol {}' to make it injectable",
                    name, name
                ).into(),
            },
            // Advanced context patterns - direct negative context violation: function uses a context it explicitly excludes with !Ctx (E3050)
            TypeError::DirectNegativeContextViolation {
                context,
                function_name,
                ..
            } => VerumError::Other {
                message: format!(
                    "E3050: direct negative context violation in function `{}`: cannot use `{}` which is explicitly excluded via `!{}`",
                    function_name, context, context
                ).into(),
            },
            // Advanced context patterns - context alias conflict: two context aliases resolve to overlapping/contradicting contexts (E3060)
            TypeError::ContextAliasConflict {
                alias,
                first_context,
                first_function,
                second_context,
                second_function,
                ..
            } => VerumError::Other {
                message: format!(
                    "E3060: context alias conflict: alias `{}` used for `{}` in `{}` but also for `{}` in `{}`",
                    alias, first_context, first_function, second_context, second_function
                ).into(),
            },

            // Advanced type syntax errors: malformed HKTs, invalid associated type projections, existential type misuse
            TypeError::ExistentialEscape { skolem_name, .. } => VerumError::Other {
                message: format!(
                    "existential type escapes its scope: skolem '{}' cannot be used outside its unpacking scope",
                    skolem_name
                ).into(),
            },

            TypeError::ExistentialBoundNotSatisfied {
                witness_type,
                protocol,
                ..
            } => VerumError::ProtocolNotSatisfied {
                ty: witness_type.clone(),
                protocol: protocol.clone(),
            },

            TypeError::KindMismatch {
                expected_kind,
                actual_kind,
                type_name,
                ..
            } => VerumError::Other {
                message: format!(
                    "kind mismatch for '{}': expected kind {}, found kind {}",
                    type_name, expected_kind, actual_kind
                ).into(),
            },

            TypeError::TypeConstructorArityMismatch {
                name,
                expected_arity,
                actual_arity,
                ..
            } => VerumError::Other {
                message: format!(
                    "type constructor '{}' has arity {}, but {} arguments were provided",
                    name, expected_arity, actual_arity
                ).into(),
            },

            TypeError::CannotResolveAssociatedType {
                base_type,
                assoc_name,
                reason,
                ..
            } => VerumError::Other {
                message: format!(
                    "cannot resolve associated type '{}.{}': {}",
                    base_type, assoc_name, reason
                ).into(),
            },

            TypeError::AmbiguousAssociatedType {
                base_type,
                assoc_name,
                candidates,
                ..
            } => VerumError::Other {
                message: format!(
                    "ambiguous associated type '{}.{}': multiple candidates: {}",
                    base_type,
                    assoc_name,
                    candidates.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ")
                ).into(),
            },

            TypeError::NegativeBoundViolated { ty, protocol, .. } => VerumError::Other {
                message: format!(
                    "negative bound violated: '{}' implements '{}', but '!{}' was required",
                    ty, protocol, protocol
                ).into(),
            },

            TypeError::SpecializationOverlap {
                ty, impl1, impl2, ..
            } => VerumError::Other {
                message: format!(
                    "specialization overlap for '{}': implementations '{}' and '{}' conflict",
                    ty, impl1, impl2
                ).into(),
            },

            TypeError::HKTBoundNotSatisfied {
                type_constructor,
                protocol,
                ..
            } => VerumError::ProtocolNotSatisfied {
                ty: type_constructor.clone(),
                protocol: protocol.clone(),
            },

            TypeError::TryOnNonResult { ty, .. } => VerumError::TypeMismatch {
                expected: "Result<T, E> or Maybe<T>".into(),
                actual: format!("{}", ty).into(),
            },

            TypeError::TryOperatorMismatch { expr_type, return_type, .. } => VerumError::TypeMismatch {
                expected: format!("{}", return_type).into(),
                actual: format!("{}", expr_type).into(),
            },

            // Definite Assignment Analysis Errors (E201)
            // Spec: L0-critical/memory-safety/uninitialized
            TypeError::UseOfUninitializedVariable { name, .. } => VerumError::Other {
                message: format!("E201: use of uninitialized variable '{}'", name).into(),
            },

            TypeError::PartiallyInitializedVariable { name, missing, .. } => VerumError::Other {
                message: format!(
                    "E201: use of partially initialized variable '{}' (missing: {})",
                    name, missing
                ).into(),
            },

            TypeError::UninitializedField { var, field, .. } => VerumError::Other {
                message: format!("E201: field '{}' of variable '{}' is not initialized", field, var).into(),
            },

            TypeError::UninitializedArrayElement { var, index, .. } => VerumError::Other {
                message: format!(
                    "E201: array element at index {} of '{}' is not initialized",
                    index, var
                ).into(),
            },

            TypeError::UninitializedTupleElement { var, index, .. } => VerumError::Other {
                message: format!(
                    "E201: tuple element {} of '{}' is not initialized",
                    index, var
                ).into(),
            },

            TypeError::IterationOverPartialArray { var, .. } => VerumError::Other {
                message: format!("E201: cannot iterate over partially initialized array '{}'", var).into(),
            },

            TypeError::AffineValueInLoop { name, .. } => VerumError::Other {
                message: format!(
                    "E302: affine value '{}' cannot be used in loop (would be moved multiple times)",
                    name
                ).into(),
            },

            TypeError::PartiallyMovedValue { name, moved_field, .. } => VerumError::Other {
                message: format!(
                    "E302: value '{}' used after partial move (field '{}' was moved)",
                    name, moved_field
                ).into(),
            },

            TypeError::VisibilityError {
                name,
                visibility,
                module_path,
                ..
            } => VerumError::Other {
                message: format!(
                    "E601: visibility error: '{}' is {} in module '{}'",
                    name, visibility, module_path
                ).into(),
            },

            TypeError::AmbiguousName { name, sources, .. } => VerumError::Other {
                message: format!(
                    "E602: ambiguous name: '{}' is imported from multiple modules: {}",
                    name, sources
                ).into(),
            },

            TypeError::CircularConstantDependency { cycle_path, .. } => VerumError::Other {
                message: format!(
                    "E600: circular constant dependency detected: {}",
                    cycle_path
                ).into(),
            },

            // Reference Aliasing Errors (E310-E314)
            // Reference safety invariants: managed refs validated at dereference, checked refs proven safe at compile time, unsafe refs unchecked
            TypeError::BorrowConflict { var, existing_is_mut, new_is_mut, .. } => VerumError::Other {
                message: format!(
                    "E310: cannot borrow `{}` as {} because it is already borrowed as {}",
                    var,
                    if new_is_mut { "mutable" } else { "immutable" },
                    if existing_is_mut { "mutable" } else { "immutable" }
                ).into(),
            },

            TypeError::FieldBorrowConflict { var, field, .. } => VerumError::Other {
                message: format!(
                    "E311: cannot borrow `{}` because field `{}` is already borrowed",
                    var, field
                ).into(),
            },

            TypeError::DanglingReference { var, .. } => VerumError::Other {
                message: format!(
                    "E312: `{}` does not live long enough - reference outlives referent",
                    var
                ).into(),
            },

            TypeError::CheckedRefEscape { var, .. } => VerumError::Other {
                message: format!(
                    "E310: `&checked` reference to `{}` may escape through function call",
                    var
                ).into(),
            },

            TypeError::MoveWhileBorrowed { var, .. } => VerumError::Other {
                message: format!(
                    "E313: cannot move `{}` while it is borrowed",
                    var
                ).into(),
            },

            TypeError::AssignWhileBorrowed { var, .. } => VerumError::Other {
                message: format!(
                    "E314: cannot assign to `{}` while it is borrowed",
                    var
                ).into(),
            },

            // Stack Safety Errors (E320-E321)
            // Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
            TypeError::StackAllocationExceedsLimit { size, limit, .. } => VerumError::Other {
                message: format!(
                    "E320: stack allocation exceeds safe limit ({} bytes exceeds {} byte limit)",
                    size, limit
                ).into(),
            },

            TypeError::UnboundedRecursionDetected { func_name, cycle, .. } => VerumError::Other {
                message: format!(
                    "E321: potential stack overflow from unbounded recursion in function `{}` (cycle: {})",
                    func_name,
                    cycle.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(" -> ")
                ).into(),
            },

            // Meta Function Purity Violation (E501)
            // Meta function purity: meta functions are implicitly pure (no IO, no mutation of non-meta state) — Meta functions are implicitly pure
            TypeError::ImpureMetaFunction { func_name, properties, .. } => VerumError::Other {
                message: format!(
                    "E501: meta function `{}` must be pure but has side effects: {}",
                    func_name, properties
                ).into(),
            },

            // Pure Function Purity Violation (E503)
            TypeError::ImpurePureFunction { func_name, properties, .. } => VerumError::Other {
                message: format!(
                    "E503: pure function `{}` has side effects: {}",
                    func_name, properties
                ).into(),
            },

            // Async Property Violation (E504)
            TypeError::AsyncPropertyViolation { message, .. } => VerumError::Other {
                message: format!("E504: {}", message).into(),
            },

            // Invalid Meta Context Usage (E502)
            // Meta contexts: meta functions have restricted context access (only compile-time-safe contexts) — Meta contexts
            TypeError::InvalidMetaContext { func_name, invalid_contexts, .. } => VerumError::Other {
                message: format!(
                    "E502: meta function `{}` uses runtime context(s) `{}` which are not available at compile-time",
                    func_name, invalid_contexts
                ).into(),
            },

            // Quote Hygiene Errors (M400-M409)
            // Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene

            TypeError::UnboundSpliceVariable { var_name, .. } => VerumError::Other {
                message: format!(
                    "M400: unbound splice variable `{}` - not in scope at quote evaluation",
                    var_name
                ).into(),
            },

            TypeError::UnquoteOutsideQuote { expr, .. } => VerumError::Other {
                message: format!(
                    "M401: splice/unquote `${{{}}}` used outside of quote block",
                    expr
                ).into(),
            },

            TypeError::AccidentalCapture { var_name, .. } => VerumError::Other {
                message: format!(
                    "M402: accidental variable capture - `{}` in quote shadows outer binding",
                    var_name
                ).into(),
            },

            TypeError::GensymCollision { symbol, .. } => VerumError::Other {
                message: format!(
                    "M403: gensym collision - generated symbol `{}` collides with user-defined name",
                    symbol
                ).into(),
            },

            TypeError::ScopeResolutionFailure { name, .. } => VerumError::Other {
                message: format!(
                    "M404: scope resolution failure - cannot resolve `{}` in quote",
                    name
                ).into(),
            },

            TypeError::StageMismatch { expected, actual, .. } => VerumError::Other {
                message: format!(
                    "M405: stage mismatch - expected stage {}, found stage {}",
                    expected, actual
                ).into(),
            },

            TypeError::LiftTypeMismatch { ty, reason, .. } => VerumError::Other {
                message: format!(
                    "M406: cannot lift type `{}` - {}",
                    ty, reason
                ).into(),
            },

            TypeError::InvalidStageEscape { reason, .. } => VerumError::Other {
                message: format!(
                    "M407: invalid stage escape - {}",
                    reason
                ).into(),
            },

            TypeError::UndeclaredCapture { var_name, .. } => VerumError::Other {
                message: format!(
                    "M408: undeclared capture of meta-level binding `{}`",
                    var_name
                ).into(),
            },

            TypeError::RepetitionMismatch { reason, .. } => VerumError::Other {
                message: format!(
                    "M409: repetition mismatch - {}",
                    reason
                ).into(),
            },

            // Inline assembly errors
            TypeError::InvalidAsmConstType { found, .. } => VerumError::Other {
                message: format!(
                    "invalid type for inline assembly const operand: {}",
                    found
                ).into(),
            },

            TypeError::AsmOutputNotLvalue { .. } => VerumError::Other {
                message: "inline assembly output operand must be an lvalue".into(),
            },

            TypeError::RecursionLimit(msg) => VerumError::Other {
                message: format!("recursion limit exceeded: {}", msg).into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::Span;
    use verum_common::Text;

    fn dummy_span() -> Span {
        use verum_ast::FileId;
        Span::new(0, 1, FileId::new(0))
    }

    #[test]
    fn test_type_mismatch_conversion() {
        let type_err = TypeError::Mismatch {
            expected: "Int".into(),
            actual: "Text".into(),
            span: dummy_span(),
        };

        let verum_err: VerumError = type_err.into();

        match verum_err {
            VerumError::TypeMismatch { expected, actual } => {
                assert_eq!(expected, Text::from("Int"));
                assert_eq!(actual, Text::from("Text"));
            }
            _ => panic!("Expected TypeMismatch variant"),
        }
    }

    #[test]
    fn test_unbound_variable_conversion() {
        let type_err = TypeError::UnboundVariable {
            name: "x".into(),
            span: dummy_span(),
        };

        let verum_err: VerumError = type_err.into();

        match verum_err {
            VerumError::UnboundVariable { name } => {
                assert_eq!(name, Text::from("x"));
            }
            _ => panic!("Expected UnboundVariable variant"),
        }
    }

    #[test]
    fn test_protocol_not_satisfied_conversion() {
        let type_err = TypeError::ProtocolNotSatisfied {
            ty: "MyType".into(),
            protocol: "Display".into(),
            span: dummy_span(),
        };

        let verum_err: VerumError = type_err.into();

        match verum_err {
            VerumError::ProtocolNotSatisfied { ty, protocol } => {
                assert_eq!(ty, Text::from("MyType"));
                assert_eq!(protocol, Text::from("Display"));
            }
            _ => panic!("Expected ProtocolNotSatisfied variant"),
        }
    }

    #[test]
    fn test_affine_violation_conversion() {
        let type_err = TypeError::AffineViolation {
            ty: "Socket".into(),
            first_use: dummy_span(),
            second_use: dummy_span(),
        };

        let verum_err: VerumError = type_err.into();

        match verum_err {
            VerumError::AffineViolation { ty } => {
                assert_eq!(ty, Text::from("Socket"));
            }
            _ => panic!("Expected AffineViolation variant"),
        }
    }
}
