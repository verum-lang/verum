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
// Tests for new where clause support in declarations
// Tests for where clause disambiguation in declarations.
//
// This tests the separation of generic and meta where clauses in:
// - FunctionDecl
// - TypeDecl
// - ProtocolDecl
// - ImplDecl

use verum_ast::decl::{
    ContextRequirement, FunctionDecl, ImplDecl, ImplKind, ProtocolDecl, TypeDecl, TypeDeclBody,
    Visibility,
};
use verum_ast::ty::{
    Ident, Path, Type, TypeBound, TypeBoundKind, TypeKind, WhereClause, WherePredicate,
    WherePredicateKind,
};
use verum_ast::{Expr, ExprKind, Span};
use verum_common::{Heap, List, Maybe};

#[test]
fn test_function_with_generic_where_clause() {
    // Test: fn sort<T>(list: List<T>) where type T: Ord { ... }
    let dummy_span = Span::dummy();

    let generic_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Type {
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
                    dummy_span,
                ),
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(Ident::new("Ord", dummy_span))),
                    span: dummy_span,
                }]),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let func = FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("sort", dummy_span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::None,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::Some(generic_where.clone()),
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: dummy_span,
    };

    assert!(func.generic_where_clause.is_some());
    assert!(func.meta_where_clause.is_none());
}

#[test]
fn test_function_with_meta_where_clause() {
    // Test: fn array_op<N: meta usize>() where meta N > 0 { ... }
    let dummy_span = Span::dummy();

    let meta_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::expr::BinOp::Gt,
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                            dummy_span,
                        )),
                        right: Heap::new(Expr::new(
                            ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                            dummy_span,
                        )),
                    },
                    dummy_span,
                ),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let func = FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("array_op", dummy_span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::None,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::Some(meta_where.clone()),
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: dummy_span,
    };

    assert!(func.generic_where_clause.is_none());
    assert!(func.meta_where_clause.is_some());
}

#[test]
fn test_function_with_both_where_clauses() {
    // Test: fn process<T, N: meta usize>()
    //           where type T: Ord
    //           where meta N > 0
    let dummy_span = Span::dummy();

    let generic_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Type {
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
                    dummy_span,
                ),
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(Ident::new("Ord", dummy_span))),
                    span: dummy_span,
                }]),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let meta_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::expr::BinOp::Gt,
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                            dummy_span,
                        )),
                        right: Heap::new(Expr::new(
                            ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                            dummy_span,
                        )),
                    },
                    dummy_span,
                ),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let func = FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("process", dummy_span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::None,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::Some(generic_where),
        meta_where_clause: Maybe::Some(meta_where),
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: dummy_span,
    };

    assert!(func.generic_where_clause.is_some());
    assert!(func.meta_where_clause.is_some());
}

#[test]
fn test_type_decl_with_meta_where_clause() {
    // Test: type Matrix<M: meta usize, N: meta usize>
    //           where meta M > 0, meta N > 0
    //       is { data: [[Float; N]; M] }
    let dummy_span = Span::dummy();

    let meta_where = WhereClause::new(
        List::from(vec![
            WherePredicate {
                kind: WherePredicateKind::Meta {
                    constraint: Expr::new(
                        ExprKind::Binary {
                            op: verum_ast::expr::BinOp::Gt,
                            left: Heap::new(Expr::new(
                                ExprKind::Path(Path::single(Ident::new("M", dummy_span))),
                                dummy_span,
                            )),
                            right: Heap::new(Expr::new(
                                ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                                dummy_span,
                            )),
                        },
                        dummy_span,
                    ),
                },
                span: dummy_span,
            },
            WherePredicate {
                kind: WherePredicateKind::Meta {
                    constraint: Expr::new(
                        ExprKind::Binary {
                            op: verum_ast::expr::BinOp::Gt,
                            left: Heap::new(Expr::new(
                                ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                                dummy_span,
                            )),
                            right: Heap::new(Expr::new(
                                ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                                dummy_span,
                            )),
                        },
                        dummy_span,
                    ),
                },
                span: dummy_span,
            },
        ]),
        dummy_span,
    );

    let type_decl = TypeDecl {
        visibility: Visibility::Public,
        name: Ident::new("Matrix", dummy_span),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Unit,
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::Some(meta_where),
        span: dummy_span,
    };

    assert!(type_decl.meta_where_clause.is_some());
    if let Maybe::Some(where_clause) = &type_decl.meta_where_clause {
        assert_eq!(where_clause.predicates.len(), 2);
    }
}

#[test]
fn test_protocol_with_both_where_clauses() {
    // Test: protocol Container<T, N: meta usize>
    //           where type T: Clone
    //           where meta N > 0
    //       { ... }
    let dummy_span = Span::dummy();

    let generic_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Type {
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
                    dummy_span,
                ),
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(Ident::new("Clone", dummy_span))),
                    span: dummy_span,
                }]),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let meta_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::expr::BinOp::Gt,
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                            dummy_span,
                        )),
                        right: Heap::new(Expr::new(
                            ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                            dummy_span,
                        )),
                    },
                    dummy_span,
                ),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let protocol = ProtocolDecl {
        visibility: Visibility::Public,
        is_context: false,
        name: Ident::new("Container", dummy_span),
        generics: List::new(),
        bounds: List::new(),
        items: List::new(),
        generic_where_clause: Maybe::Some(generic_where),
        meta_where_clause: Maybe::Some(meta_where),
        span: dummy_span,
    };

    assert!(protocol.generic_where_clause.is_some());
    assert!(protocol.meta_where_clause.is_some());
}

#[test]
fn test_impl_with_both_where_clauses() {
    // Test: implement<T, N: meta usize> Container<T, N> for Array<T, N>
    //           where type T: Clone
    //           where meta N > 0
    //       { ... }
    let dummy_span = Span::dummy();

    let generic_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Type {
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
                    dummy_span,
                ),
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(Ident::new("Clone", dummy_span))),
                    span: dummy_span,
                }]),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let meta_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::expr::BinOp::Gt,
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                            dummy_span,
                        )),
                        right: Heap::new(Expr::new(
                            ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                            dummy_span,
                        )),
                    },
                    dummy_span,
                ),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let impl_decl = ImplDecl {
        is_unsafe: false,
        generics: List::new(),
        kind: ImplKind::Inherent(Type::new(
            TypeKind::Path(Path::single(Ident::new("Array", dummy_span))),
            dummy_span,
        )),
        generic_where_clause: Maybe::Some(generic_where),
        meta_where_clause: Maybe::Some(meta_where),
        specialize_attr: Maybe::None,
        items: List::new(),
        span: dummy_span,
    };

    assert!(impl_decl.generic_where_clause.is_some());
    assert!(impl_decl.meta_where_clause.is_some());
}

#[test]
fn test_function_with_context_requirements() {
    // Test: fn process() using [Database, Logger] { ... }
    let dummy_span = Span::dummy();

    let contexts = vec![
        ContextRequirement::simple(Path::single(Ident::new("Database", dummy_span)), List::new(), dummy_span),
        ContextRequirement::simple(Path::single(Ident::new("Logger", dummy_span)), List::new(), dummy_span),
    ];

    let func = FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("process", dummy_span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::None,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: contexts.into(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: dummy_span,
    };

    assert_eq!(func.contexts.len(), 2);
}

#[test]
fn test_complete_function_declaration() {
    // Test: fn process<T, N: meta usize>(data: T) -> Result<T>
    //           using [Database, Logger]
    //           where type T: Serialize
    //           where meta N > 0
    //       { ... }
    let dummy_span = Span::dummy();

    let contexts = vec![
        ContextRequirement::simple(Path::single(Ident::new("Database", dummy_span)), List::new(), dummy_span),
        ContextRequirement::simple(Path::single(Ident::new("Logger", dummy_span)), List::new(), dummy_span),
    ];

    let generic_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Type {
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
                    dummy_span,
                ),
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(Ident::new(
                        "Serialize",
                        dummy_span,
                    ))),
                    span: dummy_span,
                }]),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let meta_where = WhereClause::new(
        List::from(vec![WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::new(
                    ExprKind::Binary {
                        op: verum_ast::expr::BinOp::Gt,
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                            dummy_span,
                        )),
                        right: Heap::new(Expr::new(
                            ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                            dummy_span,
                        )),
                    },
                    dummy_span,
                ),
            },
            span: dummy_span,
        }]),
        dummy_span,
    );

    let func = FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("process", dummy_span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::new(
            TypeKind::Path(Path::single(Ident::new("Result", dummy_span))),
            dummy_span,
        )),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: contexts.into(),
        generic_where_clause: Maybe::Some(generic_where),
        meta_where_clause: Maybe::Some(meta_where),
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: dummy_span,
    };

    // Verify all components are present
    assert_eq!(func.contexts.len(), 2);
    assert!(func.generic_where_clause.is_some());
    assert!(func.meta_where_clause.is_some());
    assert!(func.return_type.is_some());
}
