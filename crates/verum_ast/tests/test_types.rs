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
//! Tests for type system AST nodes.
//!
//! This module tests all type representations including refinement types,
//! which are the core innovation of Verum.

use proptest::prelude::*;
use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::{FileId, Span};
use verum_ast::ty::*;
use verum_ast::ContextList;
use verum_common::{Heap, List, Maybe, Text};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_primitive_types() {
    let span = test_span();

    // Unit type
    let unit = Type::unit(span);
    assert_eq!(unit.kind, TypeKind::Unit);
    assert_eq!(unit.span, span);

    // Boolean type
    let bool_ty = Type::bool(span);
    assert_eq!(bool_ty.kind, TypeKind::Bool);

    // Integer type
    let int_ty = Type::int(span);
    assert_eq!(int_ty.kind, TypeKind::Int);

    // Float type
    let float_ty = Type::float(span);
    assert_eq!(float_ty.kind, TypeKind::Float);

    // Character type
    let char_ty = Type::new(TypeKind::Char, span);
    assert_eq!(char_ty.kind, TypeKind::Char);

    // Text type
    let string_ty = Type::text(span);
    assert_eq!(string_ty.kind, TypeKind::Text);
}

#[test]
fn test_inferred_type() {
    let span = test_span();
    let inferred = Type::inferred(span);
    assert_eq!(inferred.kind, TypeKind::Inferred);
}

#[test]
fn test_path_types() {
    let span = test_span();

    // Simple path type: Vec
    let simple = Type::new(TypeKind::Path(Path::single(test_ident("Vec"))), span);
    match &simple.kind {
        TypeKind::Path(path) => {
            assert_eq!(path.segments.len(), 1);
            if let PathSegment::Name(ident) = &path.segments[0] {
                assert_eq!(ident.name.as_str(), "Vec");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("Expected path type"),
    }

    // Qualified path: std::collections::HashMap
    let qualified = Type::new(
        TypeKind::Path(Path::new(
            List::from(vec![
                PathSegment::Name(test_ident("std")),
                PathSegment::Name(test_ident("collections")),
                PathSegment::Name(test_ident("HashMap")),
            ]),
            span,
        )),
        span,
    );
    match &qualified.kind {
        TypeKind::Path(path) => {
            assert_eq!(path.segments.len(), 3);
            if let PathSegment::Name(ident) = &path.segments[2] {
                assert_eq!(ident.name.as_str(), "HashMap");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("Expected path type"),
    }
}

#[test]
fn test_tuple_types() {
    let span = test_span();

    // Unit type (empty tuple)
    let unit = Type::new(TypeKind::Tuple(List::from(vec![])), span);
    match &unit.kind {
        TypeKind::Tuple(types) => {
            assert!(types.is_empty());
        }
        _ => panic!("Expected tuple type"),
    }

    // Single element tuple: (Int,)
    let single = Type::new(TypeKind::Tuple(List::from(vec![Type::int(span)])), span);
    match &single.kind {
        TypeKind::Tuple(types) => {
            assert_eq!(types.len(), 1);
            let first = match types.first() {
                Maybe::Some(f) => f,
                Maybe::None => panic!("Expected first element"),
            };
            assert_eq!(first.kind, TypeKind::Int);
        }
        _ => panic!("Expected tuple type"),
    }

    // Multiple element tuple: (Int, String, Bool)
    let multi = Type::new(
        TypeKind::Tuple(List::from(vec![
            Type::int(span),
            Type::text(span),
            Type::bool(span),
        ])),
        span,
    );
    match &multi.kind {
        TypeKind::Tuple(types) => {
            assert_eq!(types.len(), 3);
            let mut iter = types.iter();
            assert_eq!(iter.next().unwrap().kind, TypeKind::Int);
            assert_eq!(iter.next().unwrap().kind, TypeKind::Text);
            assert_eq!(iter.next().unwrap().kind, TypeKind::Bool);
        }
        _ => panic!("Expected tuple type"),
    }
}

#[test]
fn test_array_types() {
    let span = test_span();

    // Fixed size array: [Int; 10]
    let fixed = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::int(span)),
            size: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
        },
        span,
    );
    match &fixed.kind {
        TypeKind::Array { element, size } => {
            assert_eq!(element.kind, TypeKind::Int);
            assert!(size.is_some());
        }
        _ => panic!("Expected array type"),
    }

    // Dynamic array: [String]
    let dynamic = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::text(span)),
            size: Maybe::None,
        },
        span,
    );
    match &dynamic.kind {
        TypeKind::Array { element, size } => {
            assert_eq!(element.kind, TypeKind::Text);
            assert!(size.is_none());
        }
        _ => panic!("Expected array type"),
    }

    // Const generic size: [Bool; N]
    let const_generic = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::bool(span)),
            size: Maybe::Some(Heap::new(Expr::ident(test_ident("N")))),
        },
        span,
    );
    match &const_generic.kind {
        TypeKind::Array { size, .. } => {
            assert!(size.is_some());
        }
        _ => panic!("Expected array type"),
    }
}

#[test]
fn test_function_types() {
    let span = test_span();

    // Simple function: fn(Int) -> Bool
    let simple = Type::new(
        TypeKind::Function {
            params: List::from(vec![Type::int(span)]),
            return_type: Heap::new(Type::bool(span)),
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        span,
    );
    match &simple.kind {
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            assert_eq!(params.len(), 1);
            assert_eq!(params.first().unwrap().kind, TypeKind::Int);
            assert_eq!(return_type.kind, TypeKind::Bool);
        }
        _ => panic!("Expected function type"),
    }

    // Multiple parameters: fn(Int, String, Bool) -> ()
    let multi_param = Type::new(
        TypeKind::Function {
            params: List::from(vec![Type::int(span), Type::text(span), Type::bool(span)]),
            return_type: Heap::new(Type::unit(span)),
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        span,
    );
    match &multi_param.kind {
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            assert_eq!(params.len(), 3);
            assert_eq!(return_type.kind, TypeKind::Unit);
        }
        _ => panic!("Expected function type"),
    }

    // Higher-order function: fn(fn(Int) -> Bool) -> fn() -> Int
    let higher_order = Type::new(
        TypeKind::Function {
            params: List::from(vec![Type::new(
                TypeKind::Function {
                    params: List::from(vec![Type::int(span)]),
                    return_type: Heap::new(Type::bool(span)),
                    calling_convention: Maybe::None,
                    contexts: ContextList::empty(),
                },
                span,
            )]),
            return_type: Heap::new(Type::new(
                TypeKind::Function {
                    params: List::from(vec![]),
                    return_type: Heap::new(Type::int(span)),
                    calling_convention: Maybe::None,
                    contexts: ContextList::empty(),
                },
                span,
            )),
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        span,
    );
    match &higher_order.kind {
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            assert_eq!(params.len(), 1);
            assert!(matches!(
                params.first().unwrap().kind,
                TypeKind::Function { .. }
            ));
            assert!(matches!(return_type.kind, TypeKind::Function { .. }));
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_reference_types() {
    let span = test_span();

    // Immutable reference: &Int
    let immut_ref = Type::new(
        TypeKind::Reference {
            mutable: false,
            inner: Heap::new(Type::int(span)),
        },
        span,
    );
    match &immut_ref.kind {
        TypeKind::Reference { mutable, inner } => {
            assert!(!mutable);
            assert_eq!(inner.kind, TypeKind::Int);
        }
        _ => panic!("Expected reference type"),
    }

    // Mutable reference: &mut String
    let mut_ref = Type::new(
        TypeKind::Reference {
            mutable: true,
            inner: Heap::new(Type::text(span)),
        },
        span,
    );
    match &mut_ref.kind {
        TypeKind::Reference { mutable, inner } => {
            assert!(mutable);
            assert_eq!(inner.kind, TypeKind::Text);
        }
        _ => panic!("Expected reference type"),
    }

    // Nested reference: &&Bool
    let nested_ref = Type::new(
        TypeKind::Reference {
            mutable: false,
            inner: Heap::new(Type::new(
                TypeKind::Reference {
                    mutable: false,
                    inner: Heap::new(Type::bool(span)),
                },
                span,
            )),
        },
        span,
    );
    match &nested_ref.kind {
        TypeKind::Reference { inner, .. } => {
            assert!(matches!(inner.kind, TypeKind::Reference { .. }));
        }
        _ => panic!("Expected reference type"),
    }
}

#[test]
fn test_ownership_types() {
    let span = test_span();

    // Immutable ownership: %Int
    let immut_own = Type::new(
        TypeKind::Ownership {
            mutable: false,
            inner: Heap::new(Type::int(span)),
        },
        span,
    );
    match &immut_own.kind {
        TypeKind::Ownership { mutable, inner } => {
            assert!(!mutable);
            assert_eq!(inner.kind, TypeKind::Int);
        }
        _ => panic!("Expected ownership type"),
    }

    // Mutable ownership: %mut Vec
    let mut_own = Type::new(
        TypeKind::Ownership {
            mutable: true,
            inner: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("Vec"))),
                span,
            )),
        },
        span,
    );
    match &mut_own.kind {
        TypeKind::Ownership { mutable, .. } => {
            assert!(mutable);
        }
        _ => panic!("Expected ownership type"),
    }
}

#[test]
fn test_refinement_types() {
    let span = test_span();

    // Simple refinement: Int{> 0}
    let positive = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span)),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Gt,
                        left: Heap::new(Expr::ident(test_ident("it"))),
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );
    match &positive.kind {
        TypeKind::Refined { base, predicate } => {
            assert_eq!(base.kind, TypeKind::Int);
            match &predicate.expr.kind {
                ExprKind::Binary { op: BinOp::Gt, .. } => {}
                _ => panic!("Expected greater-than comparison"),
            }
        }
        _ => panic!("Expected refined type"),
    }

    // Range refinement: Int{>= 0 && <= 100}
    let range = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span)),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        left: Heap::new(Expr::new(
                            ExprKind::Binary {
                                op: BinOp::Ge,
                                left: Heap::new(Expr::ident(test_ident("it"))),
                                right: Heap::new(Expr::literal(Literal::int(0, span))),
                            },
                            span,
                        )),
                        right: Heap::new(Expr::new(
                            ExprKind::Binary {
                                op: BinOp::Le,
                                left: Heap::new(Expr::ident(test_ident("it"))),
                                right: Heap::new(Expr::literal(Literal::int(100, span))),
                            },
                            span,
                        )),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );
    match &range.kind {
        TypeKind::Refined { predicate, .. } => match &predicate.expr.kind {
            ExprKind::Binary { op: BinOp::And, .. } => {}
            _ => panic!("Expected AND expression"),
        },
        _ => panic!("Expected refined type"),
    }

    // Function call refinement: String{is_email(it)}
    let email = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::text(span)),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::ident(test_ident("is_email"))),
                        type_args: List::new(),
                        args: List::from(vec![Expr::ident(test_ident("it"))]),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );
    match &email.kind {
        TypeKind::Refined { base, predicate } => {
            assert_eq!(base.kind, TypeKind::Text);
            assert!(matches!(predicate.expr.kind, ExprKind::Call { .. }));
        }
        _ => panic!("Expected refined type"),
    }

    // Nested refinement: Vec<Int{> 0}>{is_sorted(it)}
    let sorted_positives = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::new(
                TypeKind::Generic {
                    base: Heap::new(Type::new(
                        TypeKind::Path(Path::single(test_ident("Vec"))),
                        span,
                    )),
                    args: {
                        let mut args = List::new();
                        args.push(GenericArg::Type(Type::new(
                            TypeKind::Refined {
                                base: Heap::new(Type::int(span)),
                                predicate: Heap::new(RefinementPredicate {
                                    expr: Expr::new(
                                        ExprKind::Binary {
                                            op: BinOp::Gt,
                                            left: Heap::new(Expr::ident(test_ident("it"))),
                                            right: Heap::new(Expr::literal(Literal::int(0, span))),
                                        },
                                        span,
                                    ),
                                    binding: Maybe::None,
                                    span,
                                }),
                            },
                            span,
                        )));
                        args
                    },
                },
                span,
            )),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::ident(test_ident("is_sorted"))),
                        type_args: List::new(),
                        args: List::from(vec![Expr::ident(test_ident("it"))]),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );
    // Just verify it can be constructed
    assert!(matches!(sorted_positives.kind, TypeKind::Refined { .. }));
}

#[test]
fn test_generic_types() {
    let span = test_span();

    // Simple generic: Vec<Int>
    let simple = Type::new(
        TypeKind::Generic {
            base: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("Vec"))),
                span,
            )),
            args: {
                let mut args = List::new();
                args.push(GenericArg::Type(Type::int(span)));
                args
            },
        },
        span,
    );
    match &simple.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 1);
            match args.first() {
                Maybe::Some(GenericArg::Type(ty)) => assert_eq!(ty.kind, TypeKind::Int),
                _ => panic!("Expected type argument"),
            }
        }
        _ => panic!("Expected generic type"),
    }

    // Multiple type parameters: HashMap<String, Int>
    let multi = Type::new(
        TypeKind::Generic {
            base: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("HashMap"))),
                span,
            )),
            args: {
                let mut args = List::new();
                args.push(GenericArg::Type(Type::text(span)));
                args.push(GenericArg::Type(Type::int(span)));
                args
            },
        },
        span,
    );
    match &multi.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 2);
        }
        _ => panic!("Expected generic type"),
    }

    // Const generic: Array<Int, 10>
    let const_generic = Type::new(
        TypeKind::Generic {
            base: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("Array"))),
                span,
            )),
            args: {
                let mut args = List::new();
                args.push(GenericArg::Type(Type::int(span)));
                args.push(GenericArg::Const(Expr::literal(Literal::int(10, span))));
                args
            },
        },
        span,
    );
    match &const_generic.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 2);
            assert!(matches!(args.first(), Maybe::Some(GenericArg::Type(_))));
            assert!(matches!(args.get(1), Maybe::Some(GenericArg::Const(_))));
        }
        _ => panic!("Expected generic type"),
    }

    // Lifetime generic: Ref<'a, Int>
    let lifetime = Type::new(
        TypeKind::Generic {
            base: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("Ref"))),
                span,
            )),
            args: {
                let mut args = List::new();
                args.push(GenericArg::Lifetime(Lifetime {
                    name: Text::from("a"),
                    span,
                }));
                args.push(GenericArg::Type(Type::int(span)));
                args
            },
        },
        span,
    );
    match &lifetime.kind {
        TypeKind::Generic { args, .. } => {
            assert_eq!(args.len(), 2);
            match args.first() {
                Maybe::Some(GenericArg::Lifetime(lt)) => assert_eq!(lt.name.as_str(), "a"),
                _ => panic!("Expected lifetime argument"),
            }
        }
        _ => panic!("Expected generic type"),
    }
}

// Tests for ImplTrait and DynTrait have been removed as these types no longer exist in the AST
// Tests for Never type have been removed as this type no longer exists in the AST

#[test]
fn test_complex_nested_types() {
    let span = test_span();

    // Result<Vec<String{len(it) > 0}>, &dyn Error>
    let complex = Type::new(
        TypeKind::Generic {
            base: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("Result"))),
                span,
            )),
            args: {
                let mut args = List::new();
                args.push(GenericArg::Type(Type::new(
                    TypeKind::Generic {
                        base: Heap::new(Type::new(
                            TypeKind::Path(Path::single(test_ident("Vec"))),
                            span,
                        )),
                        args: {
                            let mut inner_args = List::new();
                            inner_args.push(GenericArg::Type(Type::new(
                                TypeKind::Refined {
                                    base: Heap::new(Type::text(span)),
                                    predicate: Heap::new(RefinementPredicate {
                                        expr: Expr::new(
                                            ExprKind::Binary {
                                                op: BinOp::Gt,
                                                left: Heap::new(Expr::new(
                                                    ExprKind::Call {
                                                        func: Heap::new(Expr::ident(test_ident(
                                                            "len",
                                                        ))),
                                                        type_args: List::new(),
                                                        args: List::from(vec![Expr::ident(
                                                            test_ident("it"),
                                                        )]),
                                                    },
                                                    span,
                                                )),
                                                right: Heap::new(Expr::literal(Literal::int(
                                                    0, span,
                                                ))),
                                            },
                                            span,
                                        ),
                                        binding: Maybe::None,
                                        span,
                                    }),
                                },
                                span,
                            )));
                            inner_args
                        },
                    },
                    span,
                )));
                args.push(GenericArg::Type(Type::new(
                    TypeKind::Reference {
                        mutable: false,
                        inner: Heap::new(Type::new(
                            TypeKind::Path(Path::single(test_ident("Error"))),
                            span,
                        )),
                    },
                    span,
                )));
                args
            },
        },
        span,
    );

    // Just verify it can be constructed
    assert!(matches!(complex.kind, TypeKind::Generic { .. }));
}

// Property-based tests
proptest! {
    #[test]
    fn prop_tuple_type_size(size in 0usize..20) {
        let span = test_span();
        let types: Vec<Type> = (0..size)
            .map(|i| {
                match i % 3 {
                    0 => Type::int(span),
                    1 => Type::bool(span),
                    _ => Type::text(span),
                }
            })
            .collect();

        let tuple = Type::new(TypeKind::Tuple(List::from(types)), span);

        match &tuple.kind {
            TypeKind::Tuple(tys) => {
                assert_eq!(tys.len(), size);
            }
            _ => panic!("Expected tuple type"),
        }
    }

    #[test]
    fn prop_generic_args_count(count in 1usize..10) {
        let span = test_span();
        let args: List<GenericArg> = (0..count)
            .map(|i| {
                if i % 2 == 0 {
                    GenericArg::Type(Type::int(span))
                } else {
                    GenericArg::Const(Expr::literal(Literal::int(i as i128, span)))
                }
            })
            .collect();

        let generic = Type::new(
            TypeKind::Generic {
                base: Heap::new(Type::new(
                    TypeKind::Path(Path::single(test_ident("Generic"))),
                    span,
                )),
                args,
            },
            span,
        );

        match &generic.kind {
            TypeKind::Generic { args, .. } => {
                assert_eq!(args.len(), count);
            }
            _ => panic!("Expected generic type"),
        }
    }

    #[test]
    fn prop_nested_reference_depth(depth in 1usize..10) {
        let span = test_span();
        let mut ty = Type::int(span);

        for _ in 0..depth {
            ty = Type::new(
                TypeKind::Reference {
                    mutable: false,
                    inner: Heap::new(ty),
                },
                span,
            );
        }

        // Count reference depth
        fn count_depth(ty: &Type) -> usize {
            match &ty.kind {
                TypeKind::Reference { inner, .. } => 1 + count_depth(inner),
                _ => 0,
            }
        }

        assert_eq!(count_depth(&ty), depth);
    }
}

#[test]
fn test_type_with_where_clause() {
    let span = test_span();

    // Generic with where clause representation
    // Using GenericParamKind::Type with bounds
    let generic_param = GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: {
                let mut bounds = List::new();
                bounds.push(TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(test_ident("Display"))),
                    span,
                });
                bounds
            },
            default: Maybe::None,
        },
        is_implicit: false,
        span,
    };

    match &generic_param.kind {
        GenericParamKind::Type {
            name,
            bounds,
            default,
        } => {
            assert_eq!(name.name.as_str(), "T");
            assert_eq!(bounds.len(), 1);
            assert!(default.is_none());
        }
        _ => panic!("Expected type parameter"),
    }
}

#[test]
fn test_generic_param_with_default() {
    let span = test_span();

    let param_with_default = GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: List::new(),
            default: Maybe::Some(Type::int(span)),
        },
        is_implicit: false,
        span,
    };

    match &param_with_default.kind {
        GenericParamKind::Type { default, .. } => {
            assert!(default.is_some());
            if let Maybe::Some(ty) = default {
                assert_eq!(ty.kind, TypeKind::Int);
            }
        }
        _ => panic!("Expected type parameter"),
    }
}

#[test]
fn test_lifetime() {
    let span = test_span();

    let lifetime = Lifetime {
        name: Text::from("a"),
        span,
    };

    assert_eq!(lifetime.name.as_str(), "a");
    assert_eq!(lifetime.span, span);

    // Static lifetime
    let static_lifetime = Lifetime {
        name: Text::from("static"),
        span,
    };

    assert_eq!(static_lifetime.name.as_str(), "static");
}
