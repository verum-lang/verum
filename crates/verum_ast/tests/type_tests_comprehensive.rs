#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Comprehensive tests for type AST nodes including refinement types
//!
//! Tests cover:
//! - Primitive types
//! - Compound types (tuples, arrays, functions)
//! - Refinement types (the core innovation of Verum!)
//! - Generic types with type arguments
//! - Three-tier reference model (&T, &checked T, &unsafe T)
//! - Type bounds and where clauses
//! - Sigma types and dependent refinements
//!
//! Comprehensive tests for Verum type AST nodes.

use verum_ast::ty::*;
use verum_ast::*;
use verum_common::List;
use verum_common::{Heap, Maybe, Text};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name, test_span())
}

// ============================================================================
// PRIMITIVE TYPES TESTS
// ============================================================================

#[test]
fn test_unit_type() {
    let span = test_span();
    let ty = Type::unit(span);

    assert!(matches!(ty.kind, TypeKind::Unit));
    assert_eq!(ty.span, span);
}

#[test]
fn test_bool_type() {
    let span = test_span();
    let ty = Type::bool(span);

    assert!(matches!(ty.kind, TypeKind::Bool));
}

#[test]
fn test_int_type() {
    let span = test_span();
    let ty = Type::int(span);

    assert!(matches!(ty.kind, TypeKind::Int));
}

#[test]
fn test_float_type() {
    let span = test_span();
    let ty = Type::float(span);

    assert!(matches!(ty.kind, TypeKind::Float));
}

#[test]
fn test_text_type() {
    let span = test_span();
    let ty = Type::text(span);

    assert!(matches!(ty.kind, TypeKind::Text));
}

#[test]
fn test_char_type() {
    let span = test_span();
    let ty = Type::new(TypeKind::Char, span);

    assert!(matches!(ty.kind, TypeKind::Char));
}

#[test]
fn test_inferred_type() {
    let span = test_span();
    let ty = Type::inferred(span);

    assert!(matches!(ty.kind, TypeKind::Inferred));
}

// ============================================================================
// PATH TYPES TESTS
// ============================================================================

#[test]
fn test_path_type() {
    let span = test_span();
    let path = Path::single(test_ident("MyType"));
    let ty = Type::new(TypeKind::Path(path.clone()), span);

    match ty.kind {
        TypeKind::Path(ref p) => {
            assert_eq!(*p, path);
        }
        _ => panic!("Expected Path type"),
    }
}

#[test]
fn test_path_single() {
    let ident = test_ident("Foo");
    let path = Path::single(ident.clone());

    assert!(path.is_single());
    assert_eq!(path.as_ident().unwrap().name, ident.name);
}

#[test]
fn test_path_from_ident() {
    let ident = test_ident("Foo");
    let path = Path::from_ident(ident.clone());

    assert!(path.is_single());
    assert_eq!(path.as_ident().unwrap().name, ident.name);
}

#[test]
fn test_path_segments() {
    let span = test_span();
    let mut segments = List::new();
    segments.push(PathSegment::Name(test_ident("std")));
    segments.push(PathSegment::Name(test_ident("io")));
    segments.push(PathSegment::Name(test_ident("File")));

    let path = Path::new(segments, span);

    assert!(!path.is_single());
    assert_eq!(path.segments.len(), 3);
}

#[test]
fn test_path_segment_self() {
    let span = test_span();
    let mut segments = List::new();
    segments.push(PathSegment::SelfValue);

    let path = Path::new(segments, span);

    assert!(matches!(path.segments[0], PathSegment::SelfValue));
}

#[test]
fn test_path_segment_super() {
    let span = test_span();
    let mut segments = List::new();
    segments.push(PathSegment::Super);

    let path = Path::new(segments, span);

    assert!(matches!(path.segments[0], PathSegment::Super));
}

#[test]
fn test_path_segment_crate() {
    let span = test_span();
    let mut segments = List::new();
    segments.push(PathSegment::Cog);

    let path = Path::new(segments, span);

    assert!(matches!(path.segments[0], PathSegment::Cog));
}

// ============================================================================
// COMPOUND TYPES TESTS
// ============================================================================

#[test]
fn test_tuple_type_empty() {
    let span = test_span();
    let types = List::new();
    let ty = Type::new(TypeKind::Tuple(types.clone()), span);

    match ty.kind {
        TypeKind::Tuple(ref ts) => {
            assert_eq!(ts.len(), 0);
        }
        _ => panic!("Expected Tuple type"),
    }
}

#[test]
fn test_tuple_type_multiple_elements() {
    let span = test_span();
    let mut types = List::new();
    types.push(Type::int(span));
    types.push(Type::text(span));
    types.push(Type::bool(span));

    let ty = Type::new(TypeKind::Tuple(types.clone()), span);

    match ty.kind {
        TypeKind::Tuple(ref ts) => {
            assert_eq!(ts.len(), 3);
        }
        _ => panic!("Expected Tuple type"),
    }
}

#[test]
fn test_array_type_with_size() {
    let span = test_span();
    let element = Heap::new(Type::int(span));
    let size = Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span))));

    let ty = Type::new(TypeKind::Array { element, size }, span);

    match ty.kind {
        TypeKind::Array { ref size, .. } => {
            assert!(matches!(size, Maybe::Some(_)));
        }
        _ => panic!("Expected Array type"),
    }
}

#[test]
fn test_array_type_without_size() {
    let span = test_span();
    let element = Heap::new(Type::int(span));
    let size = Maybe::None;

    let ty = Type::new(TypeKind::Array { element, size }, span);

    match ty.kind {
        TypeKind::Array { ref size, .. } => {
            assert!(matches!(size, Maybe::None));
        }
        _ => panic!("Expected Array type"),
    }
}

#[test]
fn test_slice_type() {
    let span = test_span();
    let element = Heap::new(Type::int(span));

    let ty = Type::new(TypeKind::Slice(element), span);

    match ty.kind {
        TypeKind::Slice(_) => {}
        _ => panic!("Expected Slice type"),
    }
}

#[test]
fn test_function_type() {
    let span = test_span();
    let mut params = List::new();
    params.push(Type::int(span));
    params.push(Type::text(span));

    let return_type = Heap::new(Type::bool(span));

    let ty = Type::new(
        TypeKind::Function {
            params: params.clone(),
            return_type,
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        span,
    );

    match ty.kind {
        TypeKind::Function { ref params, .. } => {
            assert_eq!(params.len(), 2);
        }
        _ => panic!("Expected Function type"),
    }
}

// ============================================================================
// REFERENCE TYPES TESTS - Three-Tier Reference Model
// ============================================================================

#[test]
fn test_safe_reference_immutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::Reference {
            mutable: false,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::Reference { mutable, .. } => {
            assert!(!mutable);
        }
        _ => panic!("Expected Reference type"),
    }
}

#[test]
fn test_safe_reference_mutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::Reference {
            mutable: true,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::Reference { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected Reference type"),
    }
}

#[test]
fn test_checked_reference_immutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::CheckedReference {
            mutable: false,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::CheckedReference { mutable, .. } => {
            assert!(!mutable);
        }
        _ => panic!("Expected CheckedReference type"),
    }
}

#[test]
fn test_checked_reference_mutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::CheckedReference {
            mutable: true,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::CheckedReference { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected CheckedReference type"),
    }
}

#[test]
fn test_unsafe_reference_immutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::UnsafeReference {
            mutable: false,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::UnsafeReference { mutable, .. } => {
            assert!(!mutable);
        }
        _ => panic!("Expected UnsafeReference type"),
    }
}

#[test]
fn test_unsafe_reference_mutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::UnsafeReference {
            mutable: true,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::UnsafeReference { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected UnsafeReference type"),
    }
}

#[test]
fn test_raw_pointer_const() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::Pointer {
            mutable: false,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::Pointer { mutable, .. } => {
            assert!(!mutable);
        }
        _ => panic!("Expected Pointer type"),
    }
}

#[test]
fn test_raw_pointer_mut() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::Pointer {
            mutable: true,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::Pointer { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected Pointer type"),
    }
}

#[test]
fn test_ownership_reference_immutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::Ownership {
            mutable: false,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::Ownership { mutable, .. } => {
            assert!(!mutable);
        }
        _ => panic!("Expected Ownership type"),
    }
}

#[test]
fn test_ownership_reference_mutable() {
    let span = test_span();
    let inner = Heap::new(Type::int(span));

    let ty = Type::new(
        TypeKind::Ownership {
            mutable: true,
            inner,
        },
        span,
    );

    match ty.kind {
        TypeKind::Ownership { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected Ownership type"),
    }
}

// ============================================================================
// REFINEMENT TYPES TESTS - THE CORE INNOVATION OF VERUM!
// Tests for refinement predicate construction and the five binding rules.
// ============================================================================

#[test]
fn test_refined_type_inline() {
    // Rule 1 (Inline): Int{> 0}
    let span = test_span();
    let base = Heap::new(Type::int(span));

    // Predicate: it > 0
    let predicate = Heap::new(RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Heap::new(Expr::ident(test_ident("it"))),
                right: Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        span,
    ));

    let ty = Type::new(TypeKind::Refined { base, predicate }, span);

    match ty.kind {
        TypeKind::Refined { .. } => {}
        _ => panic!("Expected Refined type"),
    }
}

#[test]
fn test_refined_type_with_lambda() {
    // Rule 2 (Lambda): Int where |x| x > 0
    let span = test_span();
    let base = Heap::new(Type::int(span));

    let predicate = Heap::new(RefinementPredicate::with_binding(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Heap::new(Expr::ident(test_ident("x"))),
                right: Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Maybe::Some(test_ident("x")),
        span,
    ));

    let ty = Type::new(TypeKind::Refined { base, predicate }, span);

    match ty.kind {
        TypeKind::Refined { ref predicate, .. } => {
            assert!(matches!(predicate.binding, Maybe::Some(_)));
        }
        _ => panic!("Expected Refined type"),
    }
}

#[test]
fn test_sigma_type() {
    // Rule 3 (Sigma): x: Int where x > 0
    let span = test_span();
    let name = test_ident("x");
    let base = Heap::new(Type::int(span));

    let predicate = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Heap::new(Expr::ident(test_ident("x"))),
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    ));

    let ty = Type::new(
        TypeKind::Sigma {
            name: name.clone(),
            base,
            predicate,
        },
        span,
    );

    match ty.kind {
        TypeKind::Sigma { ref name, .. } => {
            assert_eq!(name.name.as_str(), "x");
        }
        _ => panic!("Expected Sigma type"),
    }
}

#[test]
fn test_refinement_predicate_implicit_binding() {
    let span = test_span();
    let expr = Expr::ident(test_ident("it"));
    let pred = RefinementPredicate::new(expr, span);

    assert!(matches!(pred.binding, Maybe::None));
}

#[test]
fn test_refinement_predicate_explicit_binding() {
    let span = test_span();
    let expr = Expr::ident(test_ident("x"));
    let binding = Maybe::Some(test_ident("x"));
    let pred = RefinementPredicate::with_binding(expr, binding.clone(), span);

    assert!(matches!(pred.binding, Maybe::Some(_)));
}

// ============================================================================
// GENERIC TYPES TESTS
// ============================================================================

#[test]
fn test_generic_type_with_type_args() {
    let span = test_span();
    let base = Heap::new(Type::new(
        TypeKind::Path(Path::single(test_ident("List"))),
        span,
    ));

    let mut args = List::new();
    args.push(GenericArg::Type(Type::int(span)));

    let ty = Type::new(TypeKind::Generic { base, args }, span);

    match ty.kind {
        TypeKind::Generic { ref args, .. } => {
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0], GenericArg::Type(_)));
        }
        _ => panic!("Expected Generic type"),
    }
}

#[test]
fn test_generic_type_with_const_args() {
    let span = test_span();
    let base = Heap::new(Type::new(
        TypeKind::Path(Path::single(test_ident("Array"))),
        span,
    ));

    let mut args = List::new();
    args.push(GenericArg::Type(Type::int(span)));
    args.push(GenericArg::Const(Expr::literal(Literal::int(10, span))));

    let ty = Type::new(TypeKind::Generic { base, args }, span);

    match ty.kind {
        TypeKind::Generic { ref args, .. } => {
            assert_eq!(args.len(), 2);
            assert!(matches!(args[1], GenericArg::Const(_)));
        }
        _ => panic!("Expected Generic type"),
    }
}

#[test]
fn test_qualified_type() {
    let span = test_span();
    let self_ty = Heap::new(Type::new(
        TypeKind::Path(Path::single(test_ident("T"))),
        span,
    ));
    let trait_ref = Path::single(test_ident("Iterator"));
    let assoc_name = test_ident("Item");

    let ty = Type::new(
        TypeKind::Qualified {
            self_ty,
            trait_ref,
            assoc_name: assoc_name.clone(),
        },
        span,
    );

    match ty.kind {
        TypeKind::Qualified { ref assoc_name, .. } => {
            assert_eq!(assoc_name.name.as_str(), "Item");
        }
        _ => panic!("Expected Qualified type"),
    }
}

// ============================================================================
// TYPE BOUNDS TESTS
// ============================================================================

#[test]
fn test_bounded_type() {
    let span = test_span();
    let base = Heap::new(Type::new(
        TypeKind::Path(Path::single(test_ident("T"))),
        span,
    ));

    let mut bounds = List::new();
    bounds.push(TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(test_ident("Display"))),
        span,
    });

    let ty = Type::new(TypeKind::Bounded { base, bounds }, span);

    match ty.kind {
        TypeKind::Bounded { ref bounds, .. } => {
            assert_eq!(bounds.len(), 1);
        }
        _ => panic!("Expected Bounded type"),
    }
}

#[test]
fn test_dyn_protocol_type() {
    let span = test_span();
    let mut bounds = List::new();
    bounds.push(TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(test_ident("Display"))),
        span,
    });
    bounds.push(TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(test_ident("Debug"))),
        span,
    });

    let ty = Type::new(
        TypeKind::DynProtocol {
            bounds,
            bindings: Maybe::None,
        },
        span,
    );

    match ty.kind {
        TypeKind::DynProtocol { ref bounds, .. } => {
            assert_eq!(bounds.len(), 2);
        }
        _ => panic!("Expected DynProtocol type"),
    }
}

#[test]
fn test_type_bound_protocol() {
    let span = test_span();
    let bound = TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(test_ident("Clone"))),
        span,
    };

    assert!(matches!(bound.kind, TypeBoundKind::Protocol(_)));
}

#[test]
fn test_type_bound_equality() {
    let span = test_span();
    let bound = TypeBound {
        kind: TypeBoundKind::Equality(Type::int(span)),
        span,
    };

    assert!(matches!(bound.kind, TypeBoundKind::Equality(_)));
}

// ============================================================================
// GENERIC PARAMETERS TESTS
// ============================================================================

#[test]
fn test_generic_param_type() {
    let span = test_span();
    let param = GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: List::new(),
            default: Maybe::None,
        },
        is_implicit: false,
        span,
    };

    match param.kind {
        GenericParamKind::Type { ref name, .. } => {
            assert_eq!(name.name.as_str(), "T");
        }
        _ => panic!("Expected Type generic parameter"),
    }
}

#[test]
fn test_generic_param_type_with_bounds() {
    let span = test_span();
    let mut bounds = List::new();
    bounds.push(TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(test_ident("Display"))),
        span,
    });

    let param = GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: bounds.clone(),
            default: Maybe::None,
        },
        is_implicit: false,
        span,
    };

    match param.kind {
        GenericParamKind::Type { ref bounds, .. } => {
            assert_eq!(bounds.len(), 1);
        }
        _ => panic!("Expected Type generic parameter"),
    }
}

#[test]
fn test_generic_param_type_with_default() {
    let span = test_span();
    let param = GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: List::new(),
            default: Maybe::Some(Type::int(span)),
        },
        is_implicit: false,
        span,
    };

    match param.kind {
        GenericParamKind::Type { ref default, .. } => {
            assert!(matches!(default, Maybe::Some(_)));
        }
        _ => panic!("Expected Type generic parameter"),
    }
}

// test_generic_param_const() removed in v5.1 - GenericParamKind::Const is deprecated
// Use GenericParamKind::Meta instead for compile-time parameters

#[test]
fn test_generic_param_meta() {
    // Meta parameter: unified compile-time computation under the meta keyword
    let span = test_span();
    let param = GenericParam {
        kind: GenericParamKind::Meta {
            name: test_ident("N"),
            ty: Type::new(TypeKind::Path(Path::single(test_ident("usize"))), span),
            refinement: Maybe::None,
        },
        is_implicit: false,
        span,
    };

    match param.kind {
        GenericParamKind::Meta { ref name, .. } => {
            assert_eq!(name.name.as_str(), "N");
        }
        _ => panic!("Expected Meta generic parameter"),
    }
}

#[test]
fn test_generic_param_meta_with_refinement() {
    let span = test_span();
    let refinement = Maybe::Some(Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Heap::new(Expr::ident(test_ident("N"))),
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    )));

    let param = GenericParam {
        kind: GenericParamKind::Meta {
            name: test_ident("N"),
            ty: Type::new(TypeKind::Path(Path::single(test_ident("usize"))), span),
            refinement,
        },
        is_implicit: false,
        span,
    };

    match param.kind {
        GenericParamKind::Meta { ref refinement, .. } => {
            assert!(matches!(refinement, Maybe::Some(_)));
        }
        _ => panic!("Expected Meta generic parameter"),
    }
}

// ============================================================================
// WHERE CLAUSE TESTS - disambiguation between type, meta, value, postcondition
// ============================================================================

#[test]
fn test_where_predicate_type() {
    // where type T: Protocol
    let span = test_span();
    let mut bounds = List::new();
    bounds.push(TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(test_ident("Ord"))),
        span,
    });

    let pred = WherePredicate {
        kind: WherePredicateKind::Type {
            ty: Type::new(TypeKind::Path(Path::single(test_ident("T"))), span),
            bounds,
        },
        span,
    };

    match pred.kind {
        WherePredicateKind::Type { .. } => {}
        _ => panic!("Expected Type where predicate"),
    }
}

#[test]
fn test_where_predicate_meta() {
    // where meta N > 0
    let span = test_span();
    let pred = WherePredicate {
        kind: WherePredicateKind::Meta {
            constraint: Expr::new(
                ExprKind::Binary {
                    op: BinOp::Gt,
                    left: Heap::new(Expr::ident(test_ident("N"))),
                    right: Heap::new(Expr::literal(Literal::int(0, span))),
                },
                span,
            ),
        },
        span,
    };

    match pred.kind {
        WherePredicateKind::Meta { .. } => {}
        _ => panic!("Expected Meta where predicate"),
    }
}

#[test]
fn test_where_predicate_value() {
    // where value it > 0
    let span = test_span();
    let pred = WherePredicate {
        kind: WherePredicateKind::Value {
            predicate: Expr::new(
                ExprKind::Binary {
                    op: BinOp::Gt,
                    left: Heap::new(Expr::ident(test_ident("it"))),
                    right: Heap::new(Expr::literal(Literal::int(0, span))),
                },
                span,
            ),
        },
        span,
    };

    match pred.kind {
        WherePredicateKind::Value { .. } => {}
        _ => panic!("Expected Value where predicate"),
    }
}

#[test]
fn test_where_predicate_ensures() {
    // where ensures result >= 0
    let span = test_span();
    let pred = WherePredicate {
        kind: WherePredicateKind::Ensures {
            postcondition: Expr::new(
                ExprKind::Binary {
                    op: BinOp::Ge,
                    left: Heap::new(Expr::ident(test_ident("result"))),
                    right: Heap::new(Expr::literal(Literal::int(0, span))),
                },
                span,
            ),
        },
        span,
    };

    match pred.kind {
        WherePredicateKind::Ensures { .. } => {}
        _ => panic!("Expected Ensures where predicate"),
    }
}

#[test]
fn test_where_clause() {
    let span = test_span();
    let mut predicates = List::new();
    predicates.push(WherePredicate {
        kind: WherePredicateKind::Type {
            ty: Type::new(TypeKind::Path(Path::single(test_ident("T"))), span),
            bounds: List::new(),
        },
        span,
    });

    let clause = WhereClause { predicates, span };

    assert_eq!(clause.predicates.len(), 1);
    assert_eq!(clause.span, span);
}

// ============================================================================
// IDENTIFIER TESTS
// ============================================================================

#[test]
fn test_ident_creation() {
    let ident = test_ident("foo");

    assert_eq!(ident.name.as_str(), "foo");
    assert_eq!(ident.as_str(), "foo");
}

#[test]
fn test_ident_display() {
    let ident = test_ident("bar");

    assert_eq!(format!("{}", ident), "bar");
}

// ============================================================================
// LIFETIME TESTS (Future expansion)
// ============================================================================

#[test]
fn test_lifetime() {
    let span = test_span();
    let lifetime = Lifetime {
        name: Text::from("a"),
        span,
    };

    assert_eq!(lifetime.name.as_str(), "a");
}

// ============================================================================
// SAFETY TESTS - No panics
// ============================================================================

#[test]
fn test_type_construction_never_panics() {
    let span = test_span();

    // All primitive types
    let _ = Type::unit(span);
    let _ = Type::bool(span);
    let _ = Type::int(span);
    let _ = Type::float(span);
    let _ = Type::text(span);
    let _ = Type::inferred(span);

    // Compound types with empty collections
    let _ = Type::new(TypeKind::Tuple(List::new()), span);
    let _ = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::int(span)),
            size: Maybe::None,
        },
        span,
    );
}

#[test]
fn test_deeply_nested_types() {
    let span = test_span();

    // Build a deeply nested type: &&&&&Int
    let mut ty = Type::int(span);
    for _ in 0..10 {
        ty = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: Heap::new(ty),
            },
            span,
        );
    }

    // Should be able to create deeply nested types
    fn count_depth(ty: &Type) -> usize {
        match &ty.kind {
            TypeKind::Reference { inner, .. } => 1 + count_depth(inner),
            _ => 0,
        }
    }

    assert_eq!(count_depth(&ty), 10);
}

// ============================================================================
// EDGE CASE TESTS
// ============================================================================

#[test]
fn test_empty_tuple_type_is_unit() {
    let span = test_span();
    let ty = Type::new(TypeKind::Tuple(List::new()), span);

    match ty.kind {
        TypeKind::Tuple(ref types) => {
            assert_eq!(types.len(), 0);
        }
        _ => panic!("Expected empty Tuple type"),
    }
}

#[test]
fn test_single_element_tuple_type() {
    let span = test_span();
    let mut types = List::new();
    types.push(Type::int(span));

    let ty = Type::new(TypeKind::Tuple(types), span);

    match ty.kind {
        TypeKind::Tuple(ref types) => {
            assert_eq!(types.len(), 1);
        }
        _ => panic!("Expected single-element Tuple type"),
    }
}
