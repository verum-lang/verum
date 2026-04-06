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
//! Integration tests for end-to-end AST construction
//!
//! These tests verify that complex AST structures can be built correctly
//! and that all components work together properly.
//!
//! Comprehensive tests for statement AST nodes.

use verum_ast::decl::*;
use verum_ast::ty::GenericParamKind;
use verum_ast::*;
use verum_common::List;
use verum_common::{Heap, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name, test_span())
}

// ============================================================================
// MODULE CONSTRUCTION TESTS
// ============================================================================

#[test]
fn test_empty_module() {
    let file_id = FileId::new(0);
    let span = Span::new(0, 0, file_id);

    let module = Module::empty(file_id);

    assert_eq!(module.items.len(), 0);
    assert_eq!(module.file_id, file_id);
    assert_eq!(module.span, span);
}

#[test]
fn test_module_with_items() {
    let file_id = FileId::new(0);
    let span = test_span();

    let mut items = List::new();
    items.push(Item::new(
        ItemKind::Function(FunctionDecl {
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
            name: test_ident("main"),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Block(Block::empty(span))),
            span,
        }),
        span,
    ));

    let module = Module::new(items.clone(), file_id, span);

    assert_eq!(module.items.len(), 1);
    assert_eq!(module.file_id, file_id);
}

#[test]
fn test_compilation_unit_single_module() {
    let file_id = FileId::new(0);
    let module = Module::empty(file_id);

    let unit = CompilationUnit::single(module);

    assert_eq!(unit.modules.len(), 1);
}

#[test]
fn test_compilation_unit_multiple_modules() {
    let mut modules = List::new();
    modules.push(Module::empty(FileId::new(0)));
    modules.push(Module::empty(FileId::new(1)));

    let unit = CompilationUnit::new(modules);

    assert_eq!(unit.modules.len(), 2);
}

// ============================================================================
// COMPLEX EXPRESSION TREES
// ============================================================================

#[test]
fn test_complex_arithmetic_expression() {
    // Build: (a + b) * (c - d) / e
    let span = test_span();

    let add = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::ident(test_ident("a"))),
            right: Heap::new(Expr::ident(test_ident("b"))),
        },
        span,
    ));

    let sub = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Sub,
            left: Heap::new(Expr::ident(test_ident("c"))),
            right: Heap::new(Expr::ident(test_ident("d"))),
        },
        span,
    ));

    let mul = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: add,
            right: sub,
        },
        span,
    ));

    let div = Expr::new(
        ExprKind::Binary {
            op: BinOp::Div,
            left: mul,
            right: Heap::new(Expr::ident(test_ident("e"))),
        },
        span,
    );

    // Verify structure
    match div.kind {
        ExprKind::Binary { op: BinOp::Div, .. } => {}
        _ => panic!("Expected division at root"),
    }
}

// ============================================================================
// FUNCTION WITH REFINEMENT TYPES
// ============================================================================

#[test]
fn test_function_with_refinement_return_type() {
    // fn abs(x: Int) -> Int{>= 0}
    let span = test_span();

    let mut params = List::new();
    params.push(FunctionParam::new(
        FunctionParamKind::Regular {
            pattern: Pattern::ident(test_ident("x"), false, span),
            ty: Type::int(span),
            default_value: Maybe::None,
        },
        span,
    ));

    // Return type: Int{>= 0}
    let base = Heap::new(Type::int(span));
    let predicate = Heap::new(RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Ge,
                left: Heap::new(Expr::ident(test_ident("it"))),
                right: Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        span,
    ));

    let return_type = Maybe::Some(Type::new(TypeKind::Refined { base, predicate }, span));

    let func_decl = FunctionDecl {
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
        name: test_ident("abs"),
        generics: List::new(),
        params,
        return_type,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Block(Block::empty(span))),
        span,
    };

    assert!(matches!(
        func_decl.return_type,
        Maybe::Some(Type {
            kind: TypeKind::Refined { .. },
            ..
        })
    ));
}

// ============================================================================
// RECORD TYPE WITH FIELDS
// ============================================================================

#[test]
fn test_record_type_declaration() {
    // type Point is { x: Float, y: Float }
    let span = test_span();

    let mut fields = List::new();
    fields.push(RecordField::new(
        Visibility::Public,
        test_ident("x"),
        Type::float(span),
        span,
    ));
    fields.push(RecordField::new(
        Visibility::Public,
        test_ident("y"),
        Type::float(span),
        span,
    ));

    let type_decl = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Point"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Record(fields.clone()),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    match type_decl.body {
        TypeDeclBody::Record(ref f) => {
            assert_eq!(f.len(), 2);
        }
        _ => panic!("Expected Record type body"),
    }
}

// ============================================================================
// VARIANT TYPE DECLARATION
// ============================================================================

#[test]
fn test_variant_type_declaration() {
    // type Option<T> is Some(T) | None
    let span = test_span();

    let mut generics = List::new();
    generics.push(GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: List::new(),
            default: Maybe::None,
        },
        is_implicit: false,
        span,
    });

    let mut variants = List::new();
    let mut some_data = List::new();
    some_data.push(Type::new(
        TypeKind::Path(Path::single(test_ident("T"))),
        span,
    ));

    variants.push(Variant::new(
        test_ident("Some"),
        Some(VariantData::Tuple(some_data)),
        span,
    ));

    variants.push(Variant::new(test_ident("None"), None, span));

    let type_decl = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Option"),
        generics,
        attributes: List::new(),
        body: TypeDeclBody::Variant(variants.clone()),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    match type_decl.body {
        TypeDeclBody::Variant(ref v) => {
            assert_eq!(v.len(), 2);
        }
        _ => panic!("Expected Variant type body"),
    }
}

// ============================================================================
// AFFINE TYPE DECLARATION
// ============================================================================

#[test]
fn test_affine_type_declaration() {
    // type affine FileHandle is { fd: Int }
    let span = test_span();

    let mut fields = List::new();
    fields.push(RecordField::new(
        Visibility::Public,
        test_ident("fd"),
        Type::int(span),
        span,
    ));

    let type_decl = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("FileHandle"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Record(fields),
        resource_modifier: Maybe::Some(ResourceModifier::Affine),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(matches!(
        type_decl.resource_modifier,
        Maybe::Some(ResourceModifier::Affine)
    ));
    assert!(type_decl.resource_modifier.unwrap().is_at_most_once());
}

// ============================================================================
// CONTEXT DECLARATION
// ============================================================================

#[test]
fn test_context_declaration() {
    // context Database {
    //     fn query(sql: Text) -> Result<Rows>
    // }
    let span = test_span();

    let mut methods = List::new();
    methods.push(FunctionDecl {
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
        name: test_ident("query"),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::None,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span,
    });

    let ctx_decl = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Database"),
        generics: List::new(),
        methods,
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    assert_eq!(ctx_decl.methods.len(), 1);
}

// ============================================================================
// COMPLETE PROGRAM STRUCTURE
// ============================================================================

#[test]
fn test_complete_program() {
    // Build a complete small program with multiple declarations
    let file_id = FileId::new(0);
    let span = test_span();

    let mut items = List::new();

    // Mount statement
    items.push(Item::new(
        ItemKind::Mount(MountDecl {
            visibility: Visibility::Private,
            tree: MountTree { alias: Maybe::None,
                kind: MountTreeKind::Path(Path::single(test_ident("std"))),
                span,
            },
            alias: Maybe::None,
            span,
        }),
        span,
    ));

    // Type declaration
    items.push(Item::new(
        ItemKind::Type(TypeDecl {
            visibility: Visibility::Public,
            name: test_ident("MyType"),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Newtype(Type::int(span)),
            resource_modifier: Maybe::None,
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            span,
        }),
        span,
    ));

    // Function declaration
    items.push(Item::new(
        ItemKind::Function(FunctionDecl {
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
            name: test_ident("main"),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Block(Block::empty(span))),
            span,
        }),
        span,
    ));

    let module = Module::new(items, file_id, span);
    let compilation_unit = CompilationUnit::single(module);

    assert_eq!(compilation_unit.modules.len(), 1);
    assert_eq!(compilation_unit.modules[0].items.len(), 3);
}

// ============================================================================
// SERIALIZATION/DESERIALIZATION TESTS
// ============================================================================

#[test]
fn test_module_serialization() {
    let file_id = FileId::new(0);
    let module = Module::empty(file_id);

    // Should be able to serialize
    let json = serde_json::to_string(&module).expect("Failed to serialize");
    assert!(!json.is_empty());

    // Should be able to deserialize
    let deserialized: Module = serde_json::from_str(&json).expect("Failed to deserialize");
    assert_eq!(deserialized.file_id, file_id);
    assert_eq!(deserialized.items.len(), 0);
}

#[test]
fn test_expr_serialization() {
    let span = test_span();
    let expr = Expr::literal(Literal::int(42, span));

    let json = serde_json::to_string(&expr).expect("Failed to serialize");
    assert!(!json.is_empty());

    let deserialized: Expr = serde_json::from_str(&json).expect("Failed to deserialize");
    assert_eq!(deserialized, expr);
}

#[test]
fn test_type_serialization() {
    let span = test_span();
    let ty = Type::int(span);

    let json = serde_json::to_string(&ty).expect("Failed to serialize");
    assert!(!json.is_empty());

    let deserialized: Type = serde_json::from_str(&json).expect("Failed to deserialize");
    assert_eq!(deserialized, ty);
}

// ============================================================================
// SPANNED TRAIT TESTS
// ============================================================================

#[test]
fn test_all_nodes_implement_spanned() {
    let span = test_span();

    // All AST nodes should implement Spanned
    let _: &dyn Spanned = &Module::empty(FileId::new(0));
    let _: &dyn Spanned = &Expr::literal(Literal::int(42, span));
    let _: &dyn Spanned = &Type::int(span);
    let _: &dyn Spanned = &Pattern::wildcard(span);
    let _: &dyn Spanned = &Block::empty(span);
    let _: &dyn Spanned = &Literal::int(42, span);
}

// ============================================================================
// SAFETY TESTS - Complex structures don't panic
// ============================================================================

#[test]
fn test_complex_nesting_never_panics() {
    let span = test_span();

    // Build deeply nested structures
    let mut expr = Expr::literal(Literal::int(0, span));
    for i in 1..50 {
        expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(expr),
                right: Heap::new(Expr::literal(Literal::int(i, span))),
            },
            span,
        );
    }

    // Should be able to serialize even deeply nested structures
    let _ = serde_json::to_string(&expr).expect("Failed to serialize deep expression");
}
