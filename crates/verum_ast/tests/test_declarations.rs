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
    unused_assignments,
    clippy::approx_constant
)]
//! Tests for declaration AST nodes.
//!
//! This module tests all declaration types including functions, types,
//! protocols (traits), implementations, modules, and more.

use verum_ast::decl::*;
use verum_ast::ty::{GenericArg, GenericParamKind, TypeBound, TypeBoundKind};
use verum_ast::*;
use verum_common::{Heap, List, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_visibility() {
    // Test visibility modifiers: public, public(crate), public(super), public(in path), private
    let public = Visibility::Public;
    assert!(matches!(public, Visibility::Public));
    assert!(public.is_public());

    let private = Visibility::Private;
    assert!(matches!(private, Visibility::Private));
    assert!(!private.is_public());

    let public_crate = Visibility::PublicCrate;
    assert!(matches!(public_crate, Visibility::PublicCrate));
    assert!(public_crate.is_crate_visible());

    let public_super = Visibility::PublicSuper;
    assert!(matches!(public_super, Visibility::PublicSuper));
}

#[test]
fn test_item_construction() {
    let span = test_span();

    // Mount item (Use was renamed to Import, then to Link, then to Mount)
    let mount_item = Item::new(
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
    );
    assert!(matches!(mount_item.kind, ItemKind::Mount(_)));
    assert_eq!(mount_item.span, span);
}

#[test]
fn test_function_declaration() {
    let span = test_span();

    // Simple function: pub fn foo() -> Int
    let simple_fn = FunctionDecl {
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
        name: test_ident("foo"),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::int(span)),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Block(Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(Expr::literal(Literal::int(42, span)))),
            span,
        })),
        span,
    };

    assert_eq!(simple_fn.name.name.as_str(), "foo");
    assert!(!simple_fn.is_async);
    assert!(simple_fn.params.is_empty());
    assert!(simple_fn.body.is_some());
}

#[test]
fn test_function_with_parameters() {
    let span = test_span();

    // Function with parameters: fn add(x: Int, y: Int) -> Int
    let params = List::from(vec![
        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        ),
        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("y"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        ),
    ]);

    let add_fn = FunctionDecl {
        visibility: Visibility::Private,
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
        name: test_ident("add"),
        generics: List::new(),
        params,
        return_type: Maybe::Some(Type::int(span)),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Block(Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Heap::new(Expr::ident(test_ident("x"))),
                    right: Heap::new(Expr::ident(test_ident("y"))),
                },
                span,
            ))),
            span,
        })),
        span,
    };

    assert_eq!(add_fn.params.len(), 2);
    assert!(add_fn.return_type.is_some());
}

#[test]
fn test_async_function() {
    let span = test_span();

    let async_fn = FunctionDecl {
        visibility: Visibility::Public,
        is_async: true,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: test_ident("fetch"),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::new(
            TypeKind::Path(Path::single(test_ident("Result"))),
            span,
        )),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None, // Abstract function
        span,
    };

    assert!(async_fn.is_async);
    assert!(async_fn.body.is_none());
}

#[test]
fn test_generic_function() {
    let span = test_span();

    // Generic function: fn identity<T>(x: T) -> T
    let generics = List::from(vec![GenericParam {
        kind: GenericParamKind::Type {
            name: test_ident("T"),
            bounds: List::new(),
            default: Maybe::None,
        },
        is_implicit: false,
        span,
    }]);

    let params = List::from(vec![FunctionParam {
        kind: FunctionParamKind::Regular {
            pattern: Pattern::ident(test_ident("x"), false, span),
            ty: Type::new(TypeKind::Path(Path::single(test_ident("T"))), span),
            default_value: Maybe::None,
        },
        attributes: List::new(),
        span,
    }]);

    let generic_fn = FunctionDecl {
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
        name: test_ident("identity"),
        generics,
        params,
        return_type: Maybe::Some(Type::new(
            TypeKind::Path(Path::single(test_ident("T"))),
            span,
        )),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Block(Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(Expr::ident(test_ident("x")))),
            span,
        })),
        span,
    };

    assert_eq!(generic_fn.generics.len(), 1);
    // Check the name from the generic param kind
    match &generic_fn.generics[0].kind {
        GenericParamKind::Type { name, .. } => assert_eq!(name.name.as_str(), "T"),
        _ => panic!("Expected type parameter"),
    }
}

#[test]
fn test_method_declaration() {
    let span = test_span();

    // Method with self: fn len(&self) -> Int
    let self_param = FunctionParam {
        kind: FunctionParamKind::SelfRef,
        attributes: List::new(),
        span,
    };

    let method = FunctionDecl {
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
        name: test_ident("len"),
        generics: List::new(),
        params: List::from(vec![self_param]),
        return_type: Maybe::Some(Type::int(span)),
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
    };

    // Check if it's a method by examining first param
    assert!(method.params.first().is_some_and(|p| p.is_self()));
    assert_eq!(method.params.len(), 1);
}

#[test]
fn test_type_declaration() {
    let span = test_span();

    // Simple type alias: type UserId = Int
    let alias = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("UserId"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Alias(Type::int(span)),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert_eq!(alias.name.name.as_str(), "UserId");
    assert!(matches!(alias.body, TypeDeclBody::Alias(_)));
}

#[test]
fn test_struct_declaration() {
    let span = test_span();

    // Record: type Point { x: Float, y: Float }
    let point_struct = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Point"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Record(List::from(vec![
            RecordField {
                visibility: Visibility::Public,
                name: test_ident("x"),
                ty: Type::float(span),
                attributes: List::new(),
                default_value: Maybe::None,
                bit_spec: Maybe::None,
                span,
            },
            RecordField {
                visibility: Visibility::Public,
                name: test_ident("y"),
                ty: Type::float(span),
                attributes: List::new(),
                default_value: Maybe::None,
                bit_spec: Maybe::None,
                span,
            },
        ])),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    match &point_struct.body {
        TypeDeclBody::Record(fields) => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name.name.as_str(), "x");
            assert_eq!(fields[1].name.name.as_str(), "y");
        }
        _ => panic!("Expected record body"),
    }
}

#[test]
fn test_enum_declaration() {
    let span = test_span();

    // Variant: type Option<T> { Some(T), None }
    let option_enum = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Option"),
        generics: List::from(vec![GenericParam {
            kind: GenericParamKind::Type {
                name: test_ident("T"),
                bounds: List::new(),
                default: Maybe::None,
            },
            is_implicit: false,
            span,
        }]),
        attributes: List::new(),
        body: TypeDeclBody::Variant(List::from(vec![
            Variant {
                generic_params: List::new(),
                name: test_ident("Some"),
                data: Some(VariantData::Tuple(List::from(vec![Type::new(
                    TypeKind::Path(Path::single(test_ident("T"))),
                    span,
                )]))),
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
            Variant {
                generic_params: List::new(),
                name: test_ident("None"),
                data: None,
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
        ])),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    match &option_enum.body {
        TypeDeclBody::Variant(variants) => {
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].name.name.as_str(), "Some");
            assert_eq!(variants[1].name.name.as_str(), "None");
            assert!(variants[0].data.is_some());
            assert!(variants[1].data.is_none());
        }
        _ => panic!("Expected variant body"),
    }
}

#[test]
fn test_newtype_declaration() {
    let span = test_span();

    // Newtype: type Email(String{is_email(it)})
    let email_newtype = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Email"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Newtype(Type::new(
            TypeKind::Refined {
                base: Heap::new(Type::text(span)),
                predicate: Heap::new(RefinementPredicate {
                    expr: Expr::new(
                        ExprKind::Call {
                            func: Heap::new(Expr::ident(test_ident("is_email"))),
                            args: List::from(vec![Expr::ident(test_ident("it"))]),
                            type_args: List::new(),
                        },
                        span,
                    ),
                    binding: Maybe::None,
                    span,
                }),
            },
            span,
        )),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(matches!(email_newtype.body, TypeDeclBody::Newtype(_)));
}

#[test]
fn test_protocol_declaration() {
    let span = test_span();

    // Protocol: protocol Display { fn fmt(&self) -> String }
    let display_protocol = ProtocolDecl {
        visibility: Visibility::Public,
        is_context: false,
        name: test_ident("Display"),
        generics: List::new(),
        bounds: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        items: List::from(vec![ProtocolItem {
            kind: ProtocolItemKind::Function {
                decl: FunctionDecl {
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
                    name: test_ident("fmt"),
                    generics: List::new(),
                    params: List::from(vec![FunctionParam {
                        kind: FunctionParamKind::SelfRef,
                        attributes: List::new(),
                        span,
                    }]),
                    return_type: Maybe::Some(Type::text(span)),
                    throws_clause: Maybe::None,
                    std_attr: Maybe::None,
                    contexts: List::new(),
                    generic_where_clause: Maybe::None,
                    meta_where_clause: Maybe::None,
                    requires: List::new(),
                    ensures: List::new(),
                    attributes: List::new(),
                    body: Maybe::None, // Abstract method
                    span,
                },
                default_impl: Maybe::None,
            },
            span,
        }]),
        span,
    };

    assert_eq!(display_protocol.name.name.as_str(), "Display");
    assert_eq!(display_protocol.items.len(), 1);
    assert!(display_protocol.bounds.is_empty());
}

#[test]
fn test_protocol_with_supertraits() {
    let span = test_span();

    // Protocol with bounds: protocol Eq: PartialEq { ... }
    let eq_protocol = ProtocolDecl {
        visibility: Visibility::Public,
        is_context: false,
        name: test_ident("Eq"),
        generics: List::new(),
        bounds: List::from(vec![Type::new(TypeKind::Path(Path::single(test_ident("PartialEq"))), test_span())]),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        items: List::new(),
        span,
    };

    assert_eq!(eq_protocol.bounds.len(), 1);
    match &eq_protocol.bounds[0].kind {
        TypeKind::Path(path) => {
            assert_eq!(path.as_ident().unwrap().name.as_str(), "PartialEq");
        }
        _ => panic!("Expected path type"),
    }
}

#[test]
fn test_impl_block() {
    let span = test_span();

    // Simple impl: impl Point { ... }
    let point_impl = ImplDecl {
        is_unsafe: false,
        generics: List::new(),
        kind: ImplKind::Inherent(Type::new(
            TypeKind::Path(Path::single(test_ident("Point"))),
            span,
        )),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        specialize_attr: Maybe::None,
        items: List::from(vec![ImplItem {
            attributes: List::new(),
            visibility: Visibility::Public,
            kind: ImplItemKind::Function(FunctionDecl {
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
                name: test_ident("distance"),
                generics: List::new(),
                params: List::from(vec![
                    FunctionParam {
                        kind: FunctionParamKind::SelfRef,
                        attributes: List::new(),
                        span,
                    },
                    FunctionParam {
                        kind: FunctionParamKind::Regular {
                            pattern: Pattern::ident(test_ident("other"), false, span),
                            ty: Type::new(
                                TypeKind::Reference {
                                    mutable: false,
                                    inner: Heap::new(Type::new(
                                        TypeKind::Path(Path::single(test_ident("Point"))),
                                        span,
                                    )),
                                },
                                span,
                            ),
                            default_value: Maybe::None,
                        },
                        attributes: List::new(),
                        span,
                    },
                ]),
                return_type: Maybe::Some(Type::float(span)),
                throws_clause: Maybe::None,
                std_attr: Maybe::None,
                contexts: List::new(),
                generic_where_clause: Maybe::None,
                meta_where_clause: Maybe::None,
                requires: List::new(),
                ensures: List::new(),
                attributes: List::new(),
                body: Maybe::None, // Simplified
                span,
            }),
            span,
        }]),
        span,
    };

    match &point_impl.kind {
        ImplKind::Inherent(ty) => match &ty.kind {
            TypeKind::Path(path) => {
                assert_eq!(path.as_ident().unwrap().name.as_str(), "Point");
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected inherent impl"),
    }
    assert_eq!(point_impl.items.len(), 1);
}

#[test]
fn test_trait_impl() {
    let span = test_span();

    // Trait impl: impl Display for Point { ... }
    let display_impl = ImplDecl {
        is_unsafe: false,
        generics: List::new(),
        kind: ImplKind::Protocol {
            protocol: Path::single(test_ident("Display")),
            protocol_args: List::new(),
            for_type: Type::new(TypeKind::Path(Path::single(test_ident("Point"))), span),
        },
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        specialize_attr: Maybe::None,
        items: List::from(vec![ImplItem {
            attributes: List::new(),
            visibility: Visibility::Public,
            kind: ImplItemKind::Function(FunctionDecl {
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
                name: test_ident("fmt"),
                generics: List::new(),
                params: List::from(vec![FunctionParam {
                    kind: FunctionParamKind::SelfRef,
                    attributes: List::new(),
                    span,
                }]),
                return_type: Maybe::Some(Type::text(span)),
                throws_clause: Maybe::None,
                std_attr: Maybe::None,
                contexts: List::new(),
                generic_where_clause: Maybe::None,
                meta_where_clause: Maybe::None,
                requires: List::new(),
                ensures: List::new(),
                attributes: List::new(),
                body: Maybe::Some(FunctionBody::Block(Block {
                    stmts: List::new(),
                    expr: Maybe::None,
                    span,
                })),
                span,
            }),
            span,
        }]),
        span,
    };

    match &display_impl.kind {
        ImplKind::Protocol {
            protocol,
            protocol_args,
            for_type,
        } => {
            assert_eq!(protocol.as_ident().unwrap().name.as_str(), "Display");
            assert!(protocol_args.is_empty());
            match &for_type.kind {
                TypeKind::Path(path) => {
                    assert_eq!(path.as_ident().unwrap().name.as_str(), "Point");
                }
                _ => panic!("Expected path type"),
            }
        }
        _ => panic!("Expected protocol impl"),
    }
}

#[test]
fn test_const_declaration() {
    let span = test_span();

    // Const: pub const PI: Float = 3.14159
    let pi_const = ConstDecl {
        visibility: Visibility::Public,
        name: test_ident("PI"),
        generics: List::new(),
        ty: Type::float(span),
        value: Expr::literal(Literal::float(3.14159, span)),
        span,
    };

    assert_eq!(pi_const.name.name.as_str(), "PI");
    match &pi_const.value.kind {
        ExprKind::Literal(lit) => {
            assert!(matches!(lit.kind, LiteralKind::Float(_)));
        }
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_static_declaration() {
    let span = test_span();

    // Static: static mut COUNTER: Int = 0
    let counter_static = StaticDecl {
        visibility: Visibility::Private,
        is_mut: true,
        name: test_ident("COUNTER"),
        ty: Type::int(span),
        value: Expr::literal(Literal::int(0, span)),
        span,
    };

    assert_eq!(counter_static.name.name.as_str(), "COUNTER");
    assert!(counter_static.is_mut);
}

#[test]
fn test_module_declaration() {
    let span = test_span();

    // Module: mod utils { ... }
    let utils_module = ModuleDecl {
        visibility: Visibility::Public,
        name: test_ident("utils"),
        items: Maybe::Some(List::from(vec![Item::new(
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
                name: test_ident("helper"),
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
                body: Maybe::Some(FunctionBody::Block(Block {
                    stmts: List::new(),
                    expr: Maybe::None,
                    span,
                })),
                span,
            }),
            span,
        )])),
        profile: Maybe::None,
        features: Maybe::None,
        contexts: List::new(),
        span,
    };

    assert_eq!(utils_module.name.name.as_str(), "utils");
    assert_eq!(utils_module.items.as_ref().unwrap().len(), 1);
}

#[test]
fn test_mount_declaration() {
    let span = test_span();

    // Simple mount: mount std.io
    let simple_mount = MountDecl {
        visibility: Visibility::Private,
        tree: MountTree { alias: Maybe::None,
            kind: MountTreeKind::Path(Path::new(
                {
                    let mut segments = List::new();
                    segments.push(PathSegment::Name(test_ident("std")));
                    segments.push(PathSegment::Name(test_ident("io")));
                    segments
                },
                span,
            )),
            span,
        },
        alias: Maybe::None,
        span,
    };

    // Check the mount tree
    match &simple_mount.tree.kind {
        MountTreeKind::Path(path) => assert_eq!(path.segments.len(), 2),
        _ => panic!("Expected path mount"),
    }
    assert!(simple_mount.alias.is_none());

    // Mount with alias: mount std.collections.HashMap as Map
    let aliased_mount = MountDecl {
        visibility: Visibility::Private,
        tree: MountTree { alias: Maybe::None,
            kind: MountTreeKind::Path(Path::new(
                {
                    let mut segments = List::new();
                    segments.push(PathSegment::Name(test_ident("std")));
                    segments.push(PathSegment::Name(test_ident("collections")));
                    segments.push(PathSegment::Name(test_ident("HashMap")));
                    segments
                },
                span,
            )),
            span,
        },
        alias: Maybe::Some(test_ident("Map")),
        span,
    };

    assert!(aliased_mount.alias.is_some());
    assert_eq!(aliased_mount.alias.as_ref().unwrap().name.as_str(), "Map");

    // Mount with specific items: mount std.io.{print, println}
    let specific_mount = MountDecl {
        visibility: Visibility::Private,
        tree: MountTree { alias: Maybe::None,
            kind: MountTreeKind::Nested {
                prefix: Path::new(
                    {
                        let mut segments = List::new();
                        segments.push(PathSegment::Name(test_ident("std")));
                        segments.push(PathSegment::Name(test_ident("io")));
                        segments
                    },
                    span,
                ),
                trees: List::from(vec![
                    MountTree { alias: Maybe::None,
                        kind: MountTreeKind::Path(Path::single(test_ident("print"))),
                        span,
                    },
                    MountTree { alias: Maybe::None,
                        kind: MountTreeKind::Path(Path::single(test_ident("println"))),
                        span,
                    },
                ]),
            },
            span,
        },
        alias: Maybe::None,
        span,
    };

    match &specific_mount.tree.kind {
        MountTreeKind::Nested { trees, .. } => {
            assert_eq!(trees.len(), 2);
        }
        _ => panic!("Expected nested mounts"),
    }
}

#[test]
fn test_predicate_declaration() {
    let span = test_span();

    // Named predicate: predicate IsPositive(x: Int): Bool = x > 0
    let is_positive = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("IsPositive"),
        generics: List::new(),
        params: List::from(vec![FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span,
        }]),
        return_type: Type::bool(span),
        body: Heap::new(Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Heap::new(Expr::ident(test_ident("x"))),
                right: Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        )),
        span,
    };

    assert_eq!(is_positive.name.name.as_str(), "IsPositive");
    assert_eq!(is_positive.params.len(), 1);
}

#[test]
fn test_meta_declaration() {
    let span = test_span();

    // Meta (macro) declaration: meta! stringify { ... }
    let stringify_meta = MetaDecl {
        visibility: Visibility::Public,
        name: test_ident("stringify"),
        params: List::from(vec![MetaParam {
            name: test_ident("tokens"),
            fragment: Maybe::Some(MetaFragment::TokenTree),
            span,
        }]),
        rules: List::new(), // Empty for built-in macro
        span,
    };

    assert_eq!(stringify_meta.name.name.as_str(), "stringify");
    assert_eq!(stringify_meta.params.len(), 1);
    assert!(stringify_meta.rules.is_empty());
}

#[test]
fn test_complex_generic_declaration() {
    let span = test_span();

    // Complex generic type:
    // type Container<T, N>
    let container = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Container"),
        generics: List::from(vec![
            GenericParam {
                kind: GenericParamKind::Type {
                    name: test_ident("T"),
                    bounds: {
                        let mut bounds = List::new();
                        bounds.push(TypeBound {
                            kind: TypeBoundKind::Protocol(Path::single(test_ident("Display"))),
                            span,
                        });
                        bounds.push(TypeBound {
                            kind: TypeBoundKind::Protocol(Path::single(test_ident("Clone"))),
                            span,
                        });
                        bounds
                    },
                    default: Maybe::None,
                },
                is_implicit: false,
                span,
            },
            GenericParam {
                kind: GenericParamKind::Const {
                    name: test_ident("N"),
                    ty: Type::int(span),
                },
                is_implicit: false,
                span,
            },
        ]),
        attributes: List::new(),
        body: TypeDeclBody::Record(List::from(vec![RecordField {
            visibility: Visibility::Private,
            name: test_ident("items"),
            ty: Type::new(
                TypeKind::Array {
                    element: Heap::new(Type::new(
                        TypeKind::Path(Path::single(test_ident("T"))),
                        span,
                    )),
                    size: Maybe::Some(Heap::new(Expr::ident(test_ident("N")))),
                },
                span,
            ),
            attributes: List::new(),
            default_value: Maybe::None,
            bit_spec: Maybe::None,
            span,
        }])),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert_eq!(container.generics.len(), 2);
}

#[test]
fn test_context_declaration() {
    let span = test_span();

    // Function with contexts: fn risky() -> Int using [IO, Error]
    // Verum uses Context System for dependency injection, NOT algebraic effects
    // Context declarations: context Name { methods... }
    let contexts = List::from(vec![
        ContextRequirement::simple(Path::single(test_ident("IO")), List::new(), span),
        ContextRequirement::simple(Path::single(test_ident("Error")), List::new(), span),
    ]);

    let risky_fn = FunctionDecl {
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
        name: test_ident("risky"),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::int(span)),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span,
    };

    assert_eq!(risky_fn.contexts.len(), 2);
    assert_eq!(
        risky_fn.contexts[0].path.as_ident().unwrap().name.as_str(),
        "IO"
    );
}

#[test]
fn test_enum_with_discriminants() {
    let span = test_span();

    // Variant with no data
    let flags_enum = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Flags"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Variant(List::from(vec![
            Variant {
                generic_params: List::new(),
                name: test_ident("None"),
                data: None,
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
            Variant {
                generic_params: List::new(),
                name: test_ident("Read"),
                data: None,
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
            Variant {
                generic_params: List::new(),
                name: test_ident("Write"),
                data: None,
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
            Variant {
                generic_params: List::new(),
                name: test_ident("Execute"),
                data: None,
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
        ])),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    match &flags_enum.body {
        TypeDeclBody::Variant(variants) => {
            assert_eq!(variants.len(), 4);
            for variant in variants {
                assert!(variant.data.is_none());
            }
        }
        _ => panic!("Expected variant body"),
    }
}

#[test]
fn test_associated_type_in_protocol() {
    let span = test_span();

    // Protocol with associated type
    let iterator_protocol = ProtocolDecl {
        visibility: Visibility::Public,
        is_context: false,
        name: test_ident("Iterator"),
        generics: List::new(),
        bounds: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        items: List::from(vec![
            ProtocolItem {
                kind: ProtocolItemKind::Type {
                    name: test_ident("Item"),
                    type_params: List::new(),
                    bounds: List::new(),
                    where_clause: Maybe::None,
                    default_type: Maybe::None,
                },
                span,
            },
            ProtocolItem {
                kind: ProtocolItemKind::Function {
                    decl: FunctionDecl {
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
                        name: test_ident("next"),
                        generics: List::new(),
                        params: List::from(vec![FunctionParam {
                            kind: FunctionParamKind::SelfRefMut,
                            attributes: List::new(),
                            span,
                        }]),
                        return_type: Maybe::Some(Type::new(
                            TypeKind::Generic {
                                base: Heap::new(Type::new(
                                    TypeKind::Path(Path::single(test_ident("Option"))),
                                    span,
                                )),
                                args: {
                                    let mut args = List::new();
                                    args.push(GenericArg::Type(Type::new(
                                        TypeKind::Path(Path::single(test_ident("Item"))),
                                        span,
                                    )));
                                    args
                                },
                            },
                            span,
                        )),
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
                    },
                    default_impl: Maybe::None,
                },
                span,
            },
        ]),
        span,
    };

    assert_eq!(iterator_protocol.items.len(), 2);
    match &iterator_protocol.items[0].kind {
        ProtocolItemKind::Type { name, .. } => {
            assert_eq!(name.name.as_str(), "Item");
        }
        _ => panic!("Expected associated type"),
    }
}

// ============================================================================
// Resource Modifier Tests
// ============================================================================

#[test]
fn test_resource_modifier_affine() {
    // Affine types: use at most once (resource safety modifier)
    // type affine FileHandle is { fd: Int }
    let span = test_span();

    let file_handle = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("FileHandle"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Record(List::from(vec![RecordField {
            visibility: Visibility::Private,
            name: test_ident("fd"),
            ty: Type::int(span),
            attributes: List::new(),
            default_value: Maybe::None,
            bit_spec: Maybe::None,
            span,
        }])),
        resource_modifier: Maybe::Some(ResourceModifier::Affine),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(file_handle.resource_modifier.is_some());
    match file_handle.resource_modifier {
        Maybe::Some(ResourceModifier::Affine) => {
            assert!(ResourceModifier::Affine.is_at_most_once());
            assert!(!ResourceModifier::Affine.is_exactly_once());
        }
        _ => panic!("Expected Affine resource modifier"),
    }
}

#[test]
fn test_resource_modifier_linear() {
    // type linear Token is { value: Int }
    let span = test_span();

    let token = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Token"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Record(List::from(vec![RecordField {
            visibility: Visibility::Private,
            name: test_ident("value"),
            ty: Type::int(span),
            attributes: List::new(),
            default_value: Maybe::None,
            bit_spec: Maybe::None,
            span,
        }])),
        resource_modifier: Maybe::Some(ResourceModifier::Linear),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(token.resource_modifier.is_some());
    match token.resource_modifier {
        Maybe::Some(ResourceModifier::Linear) => {
            assert!(ResourceModifier::Linear.is_at_most_once());
            assert!(ResourceModifier::Linear.is_exactly_once());
        }
        _ => panic!("Expected Linear resource modifier"),
    }
}

#[test]
fn test_resource_modifier_none() {
    // type Point is { x: Float, y: Float }
    let span = test_span();

    let point = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Point"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Record(List::from(vec![
            RecordField {
                visibility: Visibility::Private,
                name: test_ident("x"),
                ty: Type::float(span),
                attributes: List::new(),
                default_value: Maybe::None,
                bit_spec: Maybe::None,
                span,
            },
            RecordField {
                visibility: Visibility::Private,
                name: test_ident("y"),
                ty: Type::float(span),
                attributes: List::new(),
                default_value: Maybe::None,
                bit_spec: Maybe::None,
                span,
            },
        ])),
        resource_modifier: Maybe::None,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(point.resource_modifier.is_none());
}

#[test]
fn test_resource_modifier_as_str() {
    assert_eq!(ResourceModifier::Affine.as_str(), "affine");
    assert_eq!(ResourceModifier::Linear.as_str(), "linear");
}

#[test]
fn test_resource_modifier_display() {
    let affine_str = format!("{}", ResourceModifier::Affine);
    let linear_str = format!("{}", ResourceModifier::Linear);

    assert_eq!(affine_str, "affine");
    assert_eq!(linear_str, "linear");
}

#[test]
fn test_resource_modifier_affine_newtype() {
    // type affine Handle is Int
    let span = test_span();

    let handle = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Handle"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Newtype(Type::int(span)),
        resource_modifier: Maybe::Some(ResourceModifier::Affine),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(matches!(
        handle.resource_modifier,
        Maybe::Some(ResourceModifier::Affine)
    ));
}

#[test]
fn test_resource_modifier_affine_variant() {
    // type affine Resource is File(FileHandle) | Socket(SocketHandle)
    let span = test_span();

    let resource = TypeDecl {
        visibility: Visibility::Public,
        name: test_ident("Resource"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Variant(List::from(vec![
            Variant {
                generic_params: List::new(),
                name: test_ident("File"),
                data: Some(VariantData::Tuple(List::from(vec![Type::new(
                    TypeKind::Path(Path::single(test_ident("FileHandle"))),
                    span,
                )]))),
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
            Variant {
                generic_params: List::new(),
                name: test_ident("Socket"),
                data: Some(VariantData::Tuple(List::from(vec![Type::new(
                    TypeKind::Path(Path::single(test_ident("SocketHandle"))),
                    span,
                )]))),
                where_clause: Maybe::None,
                attributes: List::new(),
                span,
            },
        ])),
        resource_modifier: Maybe::Some(ResourceModifier::Affine),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        span,
    };

    assert!(matches!(
        resource.resource_modifier,
        Maybe::Some(ResourceModifier::Affine)
    ));
}

// ============================================================================
// Sub-Context Tests
// ============================================================================

#[test]
fn test_context_decl_with_sub_contexts() {
    // Sub-context declarations for fine-grained capability control
    // context FileSystem {
    //     context Read { fn read(path: Text) -> Result<List<u8>> }
    //     context Write { fn write(path: Text, data: List<u8>) -> Result<()> }
    // }
    let span = test_span();

    let read_method = FunctionDecl {
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
        name: test_ident("read"),
        generics: List::new(),
        params: List::from(vec![FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("path"), false, span),
                ty: Type::text(span),
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span,
        }]),
        return_type: Maybe::Some(Type::new(
            TypeKind::Generic {
                base: Heap::new(Type::new(
                    TypeKind::Path(Path::single(test_ident("Result"))),
                    span,
                )),
                args: List::from(vec![GenericArg::Type(Type::new(
                    TypeKind::Generic {
                        base: Heap::new(Type::new(
                            TypeKind::Path(Path::single(test_ident("List"))),
                            span,
                        )),
                        args: List::from(vec![GenericArg::Type(Type::new(
                            TypeKind::Path(Path::single(test_ident("u8"))),
                            span,
                        ))]),
                    },
                    span,
                ))]),
            },
            span,
        )),
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
    };

    let write_method = FunctionDecl {
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
        name: test_ident("write"),
        generics: List::new(),
        params: List::from(vec![
            FunctionParam {
                kind: FunctionParamKind::Regular {
                    pattern: Pattern::ident(test_ident("path"), false, span),
                    ty: Type::text(span),
                    default_value: Maybe::None,
                },
                attributes: List::new(),
                span,
            },
            FunctionParam {
                kind: FunctionParamKind::Regular {
                    pattern: Pattern::ident(test_ident("data"), false, span),
                    ty: Type::new(
                        TypeKind::Generic {
                            base: Heap::new(Type::new(
                                TypeKind::Path(Path::single(test_ident("List"))),
                                span,
                            )),
                            args: List::from(vec![GenericArg::Type(Type::new(
                                TypeKind::Path(Path::single(test_ident("u8"))),
                                span,
                            ))]),
                        },
                        span,
                    ),
                    default_value: Maybe::None,
                },
                attributes: List::new(),
                span,
            },
        ]),
        return_type: Maybe::Some(Type::unit(span)),
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
    };

    let read_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Read"),
        generics: List::new(),
        methods: List::from(vec![read_method]),
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    let write_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Write"),
        generics: List::new(),
        methods: List::from(vec![write_method]),
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    let filesystem_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("FileSystem"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::from(vec![read_context, write_context]),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    assert_eq!(filesystem_context.sub_contexts.len(), 2);
    assert_eq!(
        filesystem_context.sub_contexts[0].name.name.as_str(),
        "Read"
    );
    assert_eq!(
        filesystem_context.sub_contexts[1].name.name.as_str(),
        "Write"
    );
    assert_eq!(filesystem_context.sub_contexts[0].methods.len(), 1);
    assert_eq!(filesystem_context.sub_contexts[1].methods.len(), 1);
}

#[test]
fn test_context_decl_nested_sub_contexts() {
    // Nested sub-contexts: FileSystem.IO.Read
    let span = test_span();

    let read_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Read"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    let io_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("IO"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::from(vec![read_context]),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    let filesystem_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("FileSystem"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::from(vec![io_context]),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    assert_eq!(filesystem_context.sub_contexts.len(), 1);
    assert_eq!(filesystem_context.sub_contexts[0].sub_contexts.len(), 1);
    assert_eq!(
        filesystem_context.sub_contexts[0].sub_contexts[0]
            .name
            .name
            .as_str(),
        "Read"
    );
}

#[test]
fn test_context_decl_methods_and_sub_contexts() {
    // Context with both methods and sub-contexts
    let span = test_span();

    let main_method = FunctionDecl {
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
        name: test_ident("status"),
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
    };

    let sub_context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Advanced"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    let context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Database"),
        generics: List::new(),
        methods: List::from(vec![main_method]),
        sub_contexts: List::from(vec![sub_context]),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    assert_eq!(context.methods.len(), 1);
    assert_eq!(context.sub_contexts.len(), 1);
    assert_eq!(context.methods[0].name.name.as_str(), "status");
    assert_eq!(context.sub_contexts[0].name.name.as_str(), "Advanced");
}

#[test]
fn test_context_decl_empty_sub_contexts() {
    // Context with no sub-contexts (default case)
    let span = test_span();

    let context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Logger"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    assert!(context.sub_contexts.is_empty());
}

#[test]
fn test_context_decl_spanned() {
    let span = test_span();

    let context = ContextDecl {
        visibility: Visibility::Public,
        is_async: false,
        name: test_ident("Test"),
        generics: List::new(),
        methods: List::new(),
        sub_contexts: List::new(),
        associated_types: List::new(),
        associated_consts: List::new(),
        span,
    };

    assert_eq!(context.span(), span);
}
