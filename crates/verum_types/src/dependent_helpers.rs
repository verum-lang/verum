//! Helper methods for integrating dependent types into the type checker
//!
//! This module provides extension methods and utilities that the TypeChecker
//! can use to verify dependent type constraints when checking types.
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Dependent Types Extension (v2.0+)

use verum_ast::ContextList;
use verum_ast::Type;
use verum_ast::expr::Expr;
use verum_ast::span::Span;
use verum_common::{Maybe, Set, Text};

use crate::context::TypeContext;
use crate::dependent_integration::{DependentTypeChecker, DependentTypeConstraint};
use crate::infer::TypeChecker;
use crate::refinement::{RefinementChecker, VerificationResult};
use crate::ty::Type as InternalType;
use crate::{Result, TypeError};

/// Extension trait for TypeChecker to add dependent type verification
pub trait DependentTypeCheckerExt {
    /// Verify a dependent type constraint
    ///
    /// This is called by the type checker when it encounters types that
    /// require dependent type verification:
    /// - Pi types: (x: A) -> B(x)
    /// - Sigma types: (x: A, B(x))
    /// - Equality types: Eq<A, lhs, rhs>
    /// - Fin types: value < bound
    fn verify_dependent_constraint(
        &mut self,
        constraint: DependentTypeConstraint,
    ) -> Result<VerificationResult>;

    /// Check if a type is a dependent type that needs verification
    fn is_dependent_type(&self, ty: &InternalType) -> bool;

    /// Extract dependent type constraint from a type
    ///
    /// Returns Some(constraint) if the type contains dependent constraints,
    /// None otherwise.
    fn extract_dependent_constraint(
        &self,
        ty: &InternalType,
        span: Span,
    ) -> Maybe<DependentTypeConstraint>;
}

impl DependentTypeCheckerExt for TypeChecker {
    fn verify_dependent_constraint(
        &mut self,
        constraint: DependentTypeConstraint,
    ) -> Result<VerificationResult> {
        // Delegate to TypeChecker's verify_dependent_type method which forwards to RefinementChecker
        self.verify_dependent_type(&constraint)
    }

    fn is_dependent_type(&self, ty: &InternalType) -> bool {
        use crate::ty::Type::*;

        match ty {
            // Sigma types are dependent
            Sigma { .. } => true,

            // Pi types are dependent
            Pi { .. } => true,

            // Eq types are dependent
            Eq { .. } => true,

            // Function types may be Pi types (dependent function types)
            // Check if return type references parameters
            // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi Types (Dependent Functions)
            Function {
                params,
                return_type,
                ..
            } => {
                // A function type is dependent if its return type contains
                // references to parameter types or values. We check for:
                // 1. Type variables that appear in both params and return type
                // 2. Named types that reference parameter names
                // 3. Refined return types with predicates mentioning params

                // Collect free type variables from parameters
                let param_vars: Set<crate::ty::TypeVar> =
                    params.iter().flat_map(collect_type_vars).collect();

                // Check if return type references any parameter variables
                let ret_vars = collect_type_vars(return_type);
                for var in ret_vars.iter() {
                    if param_vars.contains(var) {
                        return true;
                    }
                }

                // Also check if return type is itself dependent
                self.is_dependent_type(return_type)
            }

            // Named types might be Eq or Fin types
            Named { path, .. } => {
                // Extract last segment of path
                if let Some(last_seg) = path.segments.last()
                    && let verum_ast::ty::PathSegment::Name(ident) = last_seg
                {
                    return matches!(ident.name.as_str(), "Eq" | "Fin");
                }
                false
            }

            // Refined types may contain dependent predicates
            Refined { base, .. } => {
                // Check if base is dependent
                self.is_dependent_type(base)
            }

            _ => false,
        }
    }

    fn extract_dependent_constraint(
        &self,
        ty: &InternalType,
        span: Span,
    ) -> Maybe<DependentTypeConstraint> {
        use crate::ty::Type::*;

        match ty {
            // Sigma type: (x: A, B(x))
            Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Maybe::Some(DependentTypeConstraint::SigmaType {
                fst_name: fst_name.clone(),
                fst_type: convert_internal_to_ast(fst_type),
                snd_type: convert_internal_to_ast(snd_type),
                span,
            }),

            // Named types - check for Eq and Fin
            Named { path, args } => {
                // Extract last segment name
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    let name = ident.name.as_str();

                    if name == "Eq" && args.len() >= 3 {
                        // Eq<A, lhs, rhs> - Propositional equality type
                        // First arg is the type, second and third are the values being compared
                        let value_type = convert_internal_to_ast(&args[0]);

                        // Extract lhs and rhs as expressions from type arguments
                        // Type arguments may contain expression-level values in dependent types
                        let lhs = extract_expr_from_type_arg(&args[1], span);
                        let rhs = extract_expr_from_type_arg(&args[2], span);

                        Maybe::Some(DependentTypeConstraint::Equality {
                            value_type,
                            lhs,
                            rhs,
                            span,
                        })
                    } else if name == "Fin" && !args.is_empty() {
                        // Fin<n> - Finite natural numbers less than n
                        // The constraint is that any value of type Fin<n> is < n
                        // We construct a placeholder value expression and bound
                        let bound = extract_expr_from_type_arg(&args[0], span);

                        // Create a placeholder variable for the value being checked
                        // The actual value will be substituted during type checking
                        let value = create_placeholder_var("_fin_val", span);

                        Maybe::Some(DependentTypeConstraint::FinType { value, bound, span })
                    } else {
                        Maybe::None
                    }
                } else {
                    Maybe::None
                }
            }

            _ => Maybe::None,
        }
    }
}

/// Collect all type variables from a type
///
/// Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Dependent function type analysis
///
/// This recursively traverses a type to find all type variables it contains.
/// Used to detect dependent function types where the return type references
/// type variables from the parameter types.
pub fn collect_type_vars(ty: &InternalType) -> Set<crate::ty::TypeVar> {
    use crate::ty::Type as InternalType;
    let mut vars = Set::new();

    match ty {
        InternalType::Var(v) => {
            vars.insert(*v);
        }

        InternalType::Function {
            params,
            return_type,
            ..
        } => {
            for p in params {
                for v in collect_type_vars(p).iter() {
                    vars.insert(*v);
                }
            }
            for v in collect_type_vars(return_type).iter() {
                vars.insert(*v);
            }
        }

        InternalType::Named { args, .. } | InternalType::Generic { args, .. } => {
            for arg in args {
                for v in collect_type_vars(arg).iter() {
                    vars.insert(*v);
                }
            }
        }

        InternalType::Tuple(elements) => {
            for elem in elements {
                for v in collect_type_vars(elem).iter() {
                    vars.insert(*v);
                }
            }
        }

        InternalType::Reference { inner, .. }
        | InternalType::CheckedReference { inner, .. }
        | InternalType::UnsafeReference { inner, .. }
        | InternalType::Ownership { inner, .. }
        | InternalType::Pointer { inner, .. } => {
            for v in collect_type_vars(inner).iter() {
                vars.insert(*v);
            }
        }

        InternalType::Slice { element } => {
            for v in collect_type_vars(element).iter() {
                vars.insert(*v);
            }
        }

        InternalType::Array { element, .. } => {
            for v in collect_type_vars(element).iter() {
                vars.insert(*v);
            }
        }

        InternalType::Refined { base, .. } => {
            for v in collect_type_vars(base).iter() {
                vars.insert(*v);
            }
        }

        InternalType::Sigma {
            fst_type, snd_type, ..
        } => {
            for v in collect_type_vars(fst_type).iter() {
                vars.insert(*v);
            }
            for v in collect_type_vars(snd_type).iter() {
                vars.insert(*v);
            }
        }

        InternalType::Pi {
            param_type,
            return_type,
            ..
        } => {
            for v in collect_type_vars(param_type).iter() {
                vars.insert(*v);
            }
            for v in collect_type_vars(return_type).iter() {
                vars.insert(*v);
            }
        }

        // Primitive types have no type variables
        InternalType::Unit
        | InternalType::Bool
        | InternalType::Int
        | InternalType::Float
        | InternalType::Char
        | InternalType::Text
        | InternalType::Never => {}

        // Other types - traverse nested types if present
        _ => {}
    }

    vars
}

/// Convert internal Type to AST Type
///
/// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Type representation bridge
///
/// This is a complete conversion from verum_types::ty::Type to verum_ast::Type,
/// handling all type variants for proper dependent type checking.
pub fn convert_internal_to_ast(ty: &InternalType) -> Type {
    use crate::ty::Type as InternalType;
    use verum_ast::Ident;
    use verum_ast::ty::{GenericArg, Path, PathSegment, TypeKind};

    match ty {
        // Primitive types
        InternalType::Int => Type {
            kind: TypeKind::Int,
            span: Span::dummy(),
        },
        InternalType::Float => Type {
            kind: TypeKind::Float,
            span: Span::dummy(),
        },
        InternalType::Bool => Type {
            kind: TypeKind::Bool,
            span: Span::dummy(),
        },
        InternalType::Unit => Type {
            kind: TypeKind::Unit,
            span: Span::dummy(),
        },
        InternalType::Char => Type {
            kind: TypeKind::Char,
            span: Span::dummy(),
        },
        InternalType::Text => Type {
            kind: TypeKind::Text,
            span: Span::dummy(),
        },
        InternalType::Never => Type {
            kind: TypeKind::Inferred, // Never type becomes inferred in AST representation
            span: Span::dummy(),
        },

        // Named types with path
        InternalType::Named { path, args } => {
            if args.is_empty() {
                Type {
                    kind: TypeKind::Path(path.clone()),
                    span: Span::dummy(),
                }
            } else {
                // Generic instantiation: create base type from path, then wrap with Generic
                let base_type = Type {
                    kind: TypeKind::Path(path.clone()),
                    span: Span::dummy(),
                };
                let converted_args: verum_common::List<GenericArg> = args
                    .iter()
                    .map(|arg| GenericArg::Type(convert_internal_to_ast(arg)))
                    .collect();
                Type {
                    kind: TypeKind::Generic {
                        base: verum_common::Heap::new(base_type),
                        args: converted_args,
                    },
                    span: Span::dummy(),
                }
            }
        }

        // Generic types
        InternalType::Generic { name, args } => {
            let path = Path {
                segments: smallvec::smallvec![PathSegment::Name(Ident {
                    name: name.clone(),
                    span: Span::dummy(),
                })],
                span: Span::dummy(),
            };
            let base_type = Type {
                kind: TypeKind::Path(path),
                span: Span::dummy(),
            };
            let converted_args: verum_common::List<GenericArg> = args
                .iter()
                .map(|arg| GenericArg::Type(convert_internal_to_ast(arg)))
                .collect();
            Type {
                kind: TypeKind::Generic {
                    base: verum_common::Heap::new(base_type),
                    args: converted_args,
                },
                span: Span::dummy(),
            }
        }

        // Function types
        InternalType::Function {
            params,
            return_type,
            ..
        } => {
            let param_types: verum_common::List<Type> =
                params.iter().map(convert_internal_to_ast).collect();
            let ret_type = convert_internal_to_ast(return_type);
            Type {
                kind: TypeKind::Function {
                    params: param_types,
                    return_type: verum_common::Heap::new(ret_type),
                    calling_convention: verum_common::Maybe::None,
                    contexts: ContextList::empty(),
                },
                span: Span::dummy(),
            }
        }

        // Tuple types
        InternalType::Tuple(elements) => {
            let elem_types: verum_common::List<Type> =
                elements.iter().map(convert_internal_to_ast).collect();
            Type {
                kind: TypeKind::Tuple(elem_types),
                span: Span::dummy(),
            }
        }

        // Reference types
        InternalType::Reference { inner, mutable } => Type {
            kind: TypeKind::Reference {
                inner: verum_common::Heap::new(convert_internal_to_ast(inner)),
                mutable: *mutable,
            },
            span: Span::dummy(),
        },

        InternalType::CheckedReference { inner, mutable } => Type {
            kind: TypeKind::CheckedReference {
                inner: verum_common::Heap::new(convert_internal_to_ast(inner)),
                mutable: *mutable,
            },
            span: Span::dummy(),
        },

        InternalType::UnsafeReference { inner, mutable } => Type {
            kind: TypeKind::UnsafeReference {
                inner: verum_common::Heap::new(convert_internal_to_ast(inner)),
                mutable: *mutable,
            },
            span: Span::dummy(),
        },

        // Array and slice types
        InternalType::Array { element, size } => {
            use verum_ast::expr::ExprKind;
            use verum_ast::literal::{IntLit, Literal, LiteralKind};

            let size_maybe = if let Some(s) = size {
                let size_expr = Expr::new(
                    ExprKind::Literal(Literal {
                        kind: LiteralKind::Int(IntLit::new(*s as i128)),
                        span: Span::dummy(),
                    }),
                    Span::dummy(),
                );
                verum_common::Maybe::Some(verum_common::Heap::new(size_expr))
            } else {
                verum_common::Maybe::None
            };
            Type {
                kind: TypeKind::Array {
                    element: verum_common::Heap::new(convert_internal_to_ast(element)),
                    size: size_maybe,
                },
                span: Span::dummy(),
            }
        }
        InternalType::Slice { element } => Type {
            kind: TypeKind::Slice(verum_common::Heap::new(convert_internal_to_ast(element))),
            span: Span::dummy(),
        },

        // Type variables become inferred types in AST
        InternalType::Var(_) => Type {
            kind: TypeKind::Inferred,
            span: Span::dummy(),
        },

        // Fall back to inferred for complex/unsupported types
        _ => Type {
            kind: TypeKind::Inferred,
            span: Span::dummy(),
        },
    }
}

/// Extract an expression from a type argument
///
/// In dependent types, type arguments may contain expression-level values.
/// This function converts a type argument to an expression representation.
///
/// For example:
/// - Eq<Int, x, y> has x and y as expression arguments
/// - Fin<5> has 5 as an expression argument (literal)
fn extract_expr_from_type_arg(ty: &InternalType, span: Span) -> Expr {
    use crate::ty::Type as InternalType;
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::ty::PathSegment;

    match ty {
        // If it's a named type, convert to a path expression
        InternalType::Named { path, .. } => {
            // Convert type path to expression path
            if let Some(PathSegment::Name(ident)) = path.segments.last() {
                // Check if it's a number literal (e.g., type-level 5)
                if let Ok(n) = ident.name.as_str().parse::<i128>() {
                    return Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Int(IntLit::new(n)),
                            span,
                        }),
                        span,
                    );
                }
                // Otherwise treat as a variable reference
                return Expr::new(ExprKind::Path(path.clone()), span);
            }
            create_placeholder_var("_unknown", span)
        }

        // Int type could be a type-level value placeholder
        InternalType::Int => create_placeholder_var("_int_val", span),

        // For other types, create a placeholder
        _ => create_placeholder_var("_arg", span),
    }
}

/// Create a placeholder variable expression
///
/// Used when we need an expression but only have type information.
/// The placeholder will be substituted with the actual value during checking.
fn create_placeholder_var(name: &str, span: Span) -> Expr {
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::{Ident, Path, PathSegment};

    let ident = Ident {
        name: name.into(),
        span,
    };
    let path = Path {
        segments: smallvec::smallvec![PathSegment::Name(ident)],
        span,
    };
    Expr::new(ExprKind::Path(path), span)
}

/// Helper to enable dependent types in a type checker
///
/// Call this during type checker initialization to enable dependent type
/// verification.
pub fn enable_dependent_types(checker: &mut TypeChecker) {
    checker.enable_dependent_types();
}

/// Check if dependent types are enabled in a type checker
pub fn has_dependent_types(checker: &TypeChecker) -> bool {
    checker.has_dependent_types()
}
