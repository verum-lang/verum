use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, ProtocolBody, ProtocolItem,
    ProtocolItemKind, RecordField, TypeDecl, TypeDeclBody, Variant, VariantData, Visibility,
};
use verum_ast::source_signature::{
    signature_uses_universes, source_signature, source_type_declaration,
    type_declaration_uses_universes, type_uses_universes, validate_source_signature,
    validate_source_type_declaration,
};
use verum_ast::ty::{
    GenericParam, GenericParamKind, Ident, Path, Type, TypeBound, TypeBoundKind, TypeKind,
    UniverseLevelExpr, WhereClause, WherePredicate, WherePredicateKind,
};
use verum_ast::{Expr, ExprKind, Literal, Span};
use verum_common::{Heap, List};

fn ident(name: &str) -> Ident {
    Ident::new(name, Span::dummy())
}
fn universe(level: UniverseLevelExpr) -> Type {
    Type::new(TypeKind::Universe { level: Some(level) }, Span::dummy())
}
fn variable(name: &str) -> UniverseLevelExpr {
    UniverseLevelExpr::Variable(ident(name))
}
fn level(name: &str) -> GenericParam {
    GenericParam {
        kind: GenericParamKind::Level { name: ident(name) },
        is_implicit: false,
        span: Span::dummy(),
    }
}
fn ordinary(name: &str) -> GenericParam {
    GenericParam {
        kind: GenericParamKind::Type {
            name: ident(name),
            bounds: List::new(),
            default: None,
        },
        is_implicit: false,
        span: Span::dummy(),
    }
}
fn function() -> FunctionDecl {
    FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: true,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        extern_abi: None,
        is_variadic: false,
        name: ident("retain"),
        generics: List::new(),
        params: List::new(),
        return_type: None,
        throws_clause: None,
        std_attr: None,
        contexts: List::new(),
        generic_where_clause: None,
        meta_where_clause: None,
        decreases: List::new(),
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: None,
        span: Span::dummy(),
    }
}
fn parameter(ty: Type) -> FunctionParam {
    FunctionParam::new(
        FunctionParamKind::Regular {
            pattern: verum_ast::pattern::Pattern::new(
                verum_ast::pattern::PatternKind::Wildcard,
                Span::dummy(),
            ),
            ty,
            default_value: None,
        },
        Span::dummy(),
    )
}
fn retained() -> FunctionDecl {
    let mut declaration = function();
    declaration.generics.push(level("u"));
    declaration.params.push(parameter(universe(variable("u"))));
    declaration.return_type = Some(universe(UniverseLevelExpr::Max(
        Heap::new(variable("u")),
        Heap::new(UniverseLevelExpr::Concrete(1)),
    )));
    declaration
}

#[test]
fn whole_signature_roundtrips_without_erasing_binder_kinds_or_constraints() {
    let mut declaration = retained();
    declaration.is_async = true;
    declaration.is_unsafe = true;
    declaration.generics.push(GenericParam {
        kind: GenericParamKind::Type {
            name: ident("A"),
            bounds: List::from_iter([TypeBound {
                kind: TypeBoundKind::Equality(Type::new(TypeKind::Int, Span::dummy())),
                span: Span::dummy(),
            }]),
            default: Some(Type::new(TypeKind::Int, Span::dummy())),
        },
        is_implicit: true,
        span: Span::dummy(),
    });
    declaration
        .requires
        .push(Expr::literal(Literal::bool(true, Span::dummy())));
    declaration
        .ensures
        .push(Expr::literal(Literal::bool(true, Span::dummy())));
    declaration.body = Some(FunctionBody::Expr(Expr::new(
        ExprKind::Path(Path::single(ident("value"))),
        Span::dummy(),
    )));
    let carrier = source_signature(&declaration, false).unwrap().unwrap();
    declaration.body = None;
    assert_eq!(carrier, declaration);
    let roundtrip: FunctionDecl =
        serde_json::from_slice(&serde_json::to_vec(&carrier).unwrap()).unwrap();
    validate_source_signature(&roundtrip, false).unwrap();
    assert_eq!(roundtrip, carrier);
    assert!(matches!(
        roundtrip.generics[0].kind,
        GenericParamKind::Level { .. }
    ));
}

#[test]
fn bodies_do_not_create_or_consume_signature_evidence() {
    let mut declaration = function();
    let mut body = Expr::new(
        ExprKind::TypeExpr(universe(variable("not_in_scope"))),
        Span::dummy(),
    );
    for _ in 0..300 {
        body = Expr::new(ExprKind::Paren(Heap::new(body)), Span::dummy());
    }
    declaration.body = Some(FunctionBody::Expr(body));
    assert!(!signature_uses_universes(&declaration).unwrap());
    assert!(source_signature(&declaration, true).unwrap().is_none());
    declaration.return_type = Some(universe(UniverseLevelExpr::Concrete(1)));
    let carrier = source_signature(&declaration, false).unwrap().unwrap();
    assert!(carrier.body.is_none());
    assert!(validate_source_signature(&declaration, false).is_err());
}

#[test]
fn invalid_binding_and_wrong_kind_never_produce_a_carrier() {
    let mut unbound = retained();
    unbound.generics.clear();
    assert!(source_signature(&unbound, false).is_err());
    let mut wrong_kind = retained();
    wrong_kind.generics[0] = ordinary("u");
    assert!(source_signature(&wrong_kind, false).is_err());
    let mut duplicate = retained();
    duplicate.generics.push(ordinary("u"));
    assert!(source_signature(&duplicate, false).is_err());
    let mut level_as_type = retained();
    level_as_type.return_type = Some(Type::new(
        TypeKind::Path(Path::single(ident("u"))),
        Span::dummy(),
    ));
    assert!(source_signature(&level_as_type, false).is_err());
    let mut changed = retained();
    changed.return_type = Some(universe(variable("v")));
    assert!(validate_source_signature(&changed, false).is_err());
}

#[test]
fn concrete_universes_and_generic_bounds_are_detected() {
    let mut concrete = function();
    concrete.return_type = Some(universe(UniverseLevelExpr::Concrete(0)));
    assert!(source_signature(&concrete, false).unwrap().is_some());
    let mut bound_only = function();
    bound_only.generics.push(ordinary("A"));
    let GenericParamKind::Type { bounds, .. } = &mut bound_only.generics[0].kind else {
        panic!("type generic")
    };
    bounds.push(TypeBound {
        kind: TypeBoundKind::Equality(universe(variable("missing"))),
        span: Span::dummy(),
    });
    assert!(signature_uses_universes(&bound_only).unwrap());
    assert!(source_signature(&bound_only, false).is_err());
}

#[test]
fn callable_result_universes_keep_their_lexical_binders() {
    for result_level in [variable("u"), UniverseLevelExpr::Concrete(1)] {
        let mut declaration = retained();
        let mut callable = ordinary("F");
        let GenericParamKind::Type { bounds, .. } = &mut callable.kind else {
            panic!("type")
        };
        bounds.push(TypeBound {
            kind: TypeBoundKind::Equality(Type::new(
                TypeKind::Function {
                    is_async: false,
                    params: List::new(),
                    return_type: Heap::new(universe(result_level)),
                    calling_convention: None,
                    contexts: Default::default(),
                },
                Span::dummy(),
            )),
            span: Span::dummy(),
        });
        declaration.generics.push(callable);
        let carrier = source_signature(&declaration, false).unwrap().unwrap();
        validate_source_signature(&carrier, false).unwrap();
        assert_eq!(carrier.generics, declaration.generics);
    }
}

#[test]
fn universe_kind_bounds_are_detected_but_refused_until_formation_is_checked() {
    for level_expression in [variable("u"), UniverseLevelExpr::Concrete(1)] {
        for kind in [
            TypeBoundKind::Equality(universe(level_expression.clone())),
            TypeBoundKind::GenericProtocol(universe(level_expression.clone())),
        ] {
            let bound = TypeBound {
                kind,
                span: Span::dummy(),
            };
            let mut generic = function();
            generic.generics.push(level("u"));
            generic.generics.push(GenericParam {
                kind: GenericParamKind::Type {
                    name: ident("A"),
                    bounds: List::from_iter([bound.clone()]),
                    default: None,
                },
                is_implicit: false,
                span: Span::dummy(),
            });
            assert!(signature_uses_universes(&generic).unwrap());
            assert!(
                source_signature(&generic, false)
                    .unwrap_err()
                    .contains("formation checking")
            );
            assert!(validate_source_signature(&generic, false).is_err());

            let mut where_bound = function();
            where_bound.generics.push(level("u"));
            where_bound.generics.push(ordinary("A"));
            where_bound.generic_where_clause = Some(WhereClause::new(
                List::from_iter([WherePredicate {
                    kind: WherePredicateKind::Type {
                        ty: Type::new(TypeKind::Path(Path::single(ident("A"))), Span::dummy()),
                        bounds: List::from_iter([bound]),
                    },
                    span: Span::dummy(),
                }]),
                Span::dummy(),
            ));
            assert!(signature_uses_universes(&where_bound).unwrap());
            assert!(
                source_signature(&where_bound, false)
                    .unwrap_err()
                    .contains("formation checking")
            );
            assert!(validate_source_signature(&where_bound, false).is_err());
        }
    }
}

#[test]
fn methods_and_rank_two_universes_refuse_instead_of_losing_scopes() {
    assert!(source_signature(&retained(), true).is_err());
    let mut declaration = function();
    declaration.params.push(parameter(Type::new(
        TypeKind::Rank2Function {
            is_async: false,
            type_params: List::from_iter([level("u")]),
            params: List::from_iter([universe(variable("u"))]),
            return_type: Heap::new(universe(variable("u"))),
            contexts: Default::default(),
            calling_convention: None,
            where_clause: None,
        },
        Span::dummy(),
    )));
    assert!(source_signature(&declaration, false).is_err());
    let mut ordinary_method = function();
    ordinary_method
        .params
        .push(parameter(Type::new(TypeKind::Int, Span::dummy())));
    assert!(source_signature(&ordinary_method, true).unwrap().is_none());
}

#[test]
fn retained_ast_has_structural_and_text_budgets() {
    let mut declaration = function();
    let mut expression = UniverseLevelExpr::Concrete(0);
    for _ in 0..200 {
        expression = UniverseLevelExpr::Succ(Heap::new(expression));
    }
    declaration.return_type = Some(universe(expression));
    assert!(source_signature(&declaration, false).is_err());
    let mut oversized = retained();
    oversized.name = ident(&"x".repeat(1_048_577));
    assert!(source_signature(&oversized, false).is_err());
    assert!(source_signature(&retained(), false).is_ok());
}

#[test]
fn cache_type_detection_uses_ast_kinds_not_rendered_names() {
    assert!(
        !type_uses_universes(&Type::new(
            TypeKind::Path(Path::single(ident("Type(0)"))),
            Span::dummy(),
        ))
        .unwrap()
    );
    assert!(type_uses_universes(&universe(UniverseLevelExpr::Concrete(0))).unwrap());
    let mut declaration = TypeDecl {
        visibility: Visibility::Public,
        name: ident("Carrier"),
        generics: List::new(),
        attributes: List::new(),
        body: TypeDeclBody::Alias(Type::new(TypeKind::Int, Span::dummy())),
        value_params: List::new(),
        resource_modifier: None,
        generic_where_clause: None,
        meta_where_clause: None,
        span: Span::dummy(),
    };
    assert!(!type_declaration_uses_universes(&declaration).unwrap());
    declaration.generics.push(ordinary("Level"));
    assert!(!type_declaration_uses_universes(&declaration).unwrap());
    declaration.body = TypeDeclBody::Alias(universe(UniverseLevelExpr::Concrete(0)));
    assert!(type_declaration_uses_universes(&declaration).unwrap());
    declaration.body = TypeDeclBody::Alias(Type::new(TypeKind::Int, Span::dummy()));
    declaration.generics.push(level("u"));
    assert!(type_declaration_uses_universes(&declaration).unwrap());
}

fn type_declaration(body: TypeDeclBody) -> TypeDecl {
    TypeDecl {
        visibility: Visibility::Public,
        name: ident("Carrier"),
        generics: List::new(),
        attributes: List::new(),
        body,
        value_params: List::new(),
        resource_modifier: None,
        generic_where_clause: None,
        meta_where_clause: None,
        span: Span::dummy(),
    }
}

fn carrier_field() -> RecordField {
    RecordField::new(
        Visibility::Public,
        ident("carrier"),
        Type::new(TypeKind::Universe { level: None }, Span::dummy()),
        Span::dummy(),
    )
}

fn callable_universe() -> Type {
    Type::new(
        TypeKind::Function {
            is_async: false,
            params: List::from_iter([Type::new(TypeKind::Int, Span::dummy())]),
            return_type: Heap::new(universe(UniverseLevelExpr::Concrete(1))),
            calling_convention: None,
            contexts: Default::default(),
        },
        Span::dummy(),
    )
}

#[test]
fn data_carrier_roundtrips_complete_record_constraints_and_defaults() {
    let mut field = carrier_field();
    field.default_value = Some(Expr::new(
        ExprKind::TypeExpr(Type::new(TypeKind::Int, Span::dummy())),
        Span::dummy(),
    ));
    let mut declaration = type_declaration(TypeDeclBody::Record(List::from_iter([field])));
    declaration.name = ident("NType");
    declaration.resource_modifier = Some(verum_ast::decl::ResourceModifier::Affine);
    declaration.generics.push(GenericParam {
        kind: GenericParamKind::Meta {
            name: ident("N"),
            ty: Type::new(TypeKind::Int, Span::dummy()),
            refinement: Some(Heap::new(Expr::literal(Literal::bool(true, Span::dummy())))),
        },
        is_implicit: false,
        span: Span::dummy(),
    });
    declaration.meta_where_clause = Some(WhereClause::new(
        List::from_iter([WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::literal(Literal::bool(true, Span::dummy())),
            },
            span: Span::dummy(),
        }]),
        Span::dummy(),
    ));
    let carrier = source_type_declaration(&declaration).unwrap().unwrap();
    assert_eq!(carrier, declaration);
    let decoded: TypeDecl = serde_json::from_slice(&serde_json::to_vec(&carrier).unwrap()).unwrap();
    validate_source_type_declaration(&decoded).unwrap();
    assert_eq!(decoded, declaration);
}

#[test]
fn data_shapes_and_callable_bounds_preserve_full_declarations() {
    let ty = universe(UniverseLevelExpr::Concrete(0));
    let variant = Variant::new(
        ident("Pack"),
        Some(VariantData::Tuple(List::from_iter([ty.clone()]))),
        Span::dummy(),
    );
    for body in [
        TypeDeclBody::Alias(ty.clone()),
        TypeDeclBody::Newtype(ty.clone()),
        TypeDeclBody::Tuple(List::from_iter([ty.clone()])),
        TypeDeclBody::SigmaTuple(List::from_iter([ty])),
        TypeDeclBody::Variant(List::from_iter([variant.clone()])),
        TypeDeclBody::Inductive(List::from_iter([variant])),
    ] {
        let declaration = type_declaration(body);
        assert_eq!(
            source_type_declaration(&declaration).unwrap().unwrap(),
            declaration
        );
    }
    let mut declaration = type_declaration(TypeDeclBody::Unit);
    declaration.generics.push(GenericParam {
        kind: GenericParamKind::Type {
            name: ident("F"),
            bounds: List::from_iter([TypeBound {
                kind: TypeBoundKind::Equality(callable_universe()),
                span: Span::dummy(),
            }]),
            default: Some(callable_universe()),
        },
        is_implicit: false,
        span: Span::dummy(),
    });
    declaration
        .value_params
        .push(parameter(callable_universe()));
    assert_eq!(
        source_type_declaration(&declaration).unwrap().unwrap(),
        declaration
    );
    let ordinary = type_declaration(TypeDeclBody::Alias(Type::new(TypeKind::Int, Span::dummy())));
    assert!(source_type_declaration(&ordinary).unwrap().is_none());
}

fn method_item() -> ProtocolItem {
    let mut method = function();
    method.name = ident("sections");
    method.params.push(FunctionParam::new(
        FunctionParamKind::SelfRef,
        Span::dummy(),
    ));
    method.return_type = Some(universe(UniverseLevelExpr::Concrete(0)));
    ProtocolItem {
        kind: ProtocolItemKind::Function {
            decl: method,
            default_impl: None,
        },
        span: Span::dummy(),
    }
}

#[test]
fn protocol_carriers_strip_source_bodies_and_reject_bodies_on_decode() {
    let mut item = method_item();
    let ProtocolItemKind::Function { decl, default_impl } = &mut item.kind else {
        panic!("method")
    };
    let mut expression = Expr::new(
        ExprKind::TypeExpr(universe(variable("body_only_free_level"))),
        Span::dummy(),
    );
    for _ in 0..300 {
        expression = Expr::new(ExprKind::Paren(Heap::new(expression)), Span::dummy());
    }
    decl.body = Some(FunctionBody::Expr(expression.clone()));
    *default_impl = Some(FunctionBody::Expr(expression));
    assert!(source_signature(decl, true).is_err());
    let associated = ProtocolItem {
        kind: ProtocolItemKind::Type {
            name: ident("Item"),
            type_params: List::from_iter([ordinary("A")]),
            bounds: List::new(),
            where_clause: None,
            default_type: Some(universe(UniverseLevelExpr::Concrete(1))),
        },
        span: Span::dummy(),
    };
    let mut axiom = verum_ast::decl::AxiomDecl::new(
        ident("law"),
        Expr::literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    axiom.return_type = Some(universe(UniverseLevelExpr::Concrete(1)));
    let axiom = ProtocolItem {
        kind: ProtocolItemKind::Axiom(axiom),
        span: Span::dummy(),
    };
    let body = ProtocolBody::new(List::from_iter([item, associated, axiom]));
    for body in [
        TypeDeclBody::Protocol(body.clone()),
        TypeDeclBody::Coinductive(body),
    ] {
        let declaration = type_declaration(body);
        let original_error = validate_source_type_declaration(&declaration).unwrap_err();
        assert!(original_error.contains("Carrier") && original_error.contains("sections"));
        let carrier = source_type_declaration(&declaration).unwrap().unwrap();
        let (TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body)) = &carrier.body else {
            panic!("protocol")
        };
        let ProtocolItemKind::Function { decl, default_impl } = &body.items[0].kind else {
            panic!("method")
        };
        assert!(decl.body.is_none() && default_impl.is_none());
        validate_source_type_declaration(&carrier).unwrap();
        let decoded: TypeDecl =
            serde_json::from_slice(&serde_json::to_vec(&carrier).unwrap()).unwrap();
        assert_eq!(decoded, carrier);
        validate_source_type_declaration(&decoded).unwrap();
        let (TypeDeclBody::Protocol(original) | TypeDeclBody::Coinductive(original)) =
            &declaration.body
        else {
            panic!("protocol")
        };
        assert_eq!(&body.items.as_slice()[1..], &original.items.as_slice()[1..]);
        for into_default in [false, true] {
            let mut malicious = carrier.clone();
            let (TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body)) =
                &mut malicious.body
            else {
                panic!("protocol")
            };
            let ProtocolItemKind::Function { decl, default_impl } = &mut body.items[0].kind else {
                panic!("method")
            };
            let body = Some(FunctionBody::Expr(Expr::literal(Literal::bool(
                true,
                Span::dummy(),
            ))));
            if into_default {
                *default_impl = body;
            } else {
                decl.body = body;
            }
            assert!(
                validate_source_type_declaration(&malicious)
                    .unwrap_err()
                    .contains("Carrier")
            );
        }
    }
}

#[test]
fn nominal_level_binders_in_all_declaration_scopes_refuse() {
    let mut top = type_declaration(TypeDeclBody::Alias(universe(variable("u"))));
    top.generics.push(level("u"));
    let mut variant = Variant::new(ident("Pack"), None, Span::dummy());
    variant.generic_params.push(level("u"));
    let constructor = type_declaration(TypeDeclBody::Inductive(List::from_iter([variant])));
    let mut method = method_item();
    let ProtocolItemKind::Function { decl, .. } = &mut method.kind else {
        panic!("method")
    };
    decl.generics.push(level("u"));
    let method = type_declaration(TypeDeclBody::Protocol(ProtocolBody::new(List::from_iter(
        [method],
    ))));
    let associated = type_declaration(TypeDeclBody::Protocol(ProtocolBody::new(List::from_iter(
        [ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: ident("Item"),
                type_params: List::from_iter([level("u")]),
                bounds: List::new(),
                where_clause: None,
                default_type: None,
            },
            span: Span::dummy(),
        }],
    ))));
    let mut axiom = verum_ast::decl::AxiomDecl::new(
        ident("law"),
        Expr::literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    axiom.generics.push(level("u"));
    let axiom = type_declaration(TypeDeclBody::Protocol(ProtocolBody::new(List::from_iter(
        [ProtocolItem {
            kind: ProtocolItemKind::Axiom(axiom),
            span: Span::dummy(),
        }],
    ))));
    for declaration in [top, constructor, method, associated, axiom] {
        assert!(type_declaration_uses_universes(&declaration).unwrap());
        let error = source_type_declaration(&declaration).unwrap_err();
        assert!(
            error.contains("Carrier") && error.contains("level instantiation"),
            "{error}"
        );
        assert!(validate_source_type_declaration(&declaration).is_err());
    }
}

#[test]
fn nominal_carriers_reject_free_wrong_kind_duplicate_rank_two_and_oversize_annotations() {
    let free = type_declaration(TypeDeclBody::Alias(universe(variable("u"))));
    let mut wrong = free.clone();
    wrong.generics.push(ordinary("u"));
    let mut duplicate = type_declaration(TypeDeclBody::Alias(universe(
        UniverseLevelExpr::Concrete(0),
    )));
    duplicate.generics.extend([ordinary("A"), ordinary("A")]);
    let rank_two = type_declaration(TypeDeclBody::Alias(Type::new(
        TypeKind::Rank2Function {
            is_async: false,
            type_params: List::from_iter([ordinary("A")]),
            params: List::new(),
            return_type: Heap::new(universe(UniverseLevelExpr::Concrete(0))),
            contexts: Default::default(),
            calling_convention: None,
            where_clause: None,
        },
        Span::dummy(),
    )));
    let mut expression = UniverseLevelExpr::Concrete(0);
    for _ in 0..200 {
        expression = UniverseLevelExpr::Succ(Heap::new(expression));
    }
    let deep = type_declaration(TypeDeclBody::Alias(universe(expression)));
    for declaration in [free, wrong, duplicate, rank_two, deep] {
        assert!(
            source_type_declaration(&declaration)
                .unwrap_err()
                .contains("Carrier")
        );
        assert!(
            validate_source_type_declaration(&declaration)
                .unwrap_err()
                .contains("Carrier")
        );
    }
    for name in ["", "other.Carrier", "other::Carrier"] {
        let mut declaration = type_declaration(TypeDeclBody::Alias(universe(
            UniverseLevelExpr::Concrete(0),
        )));
        declaration.name = ident(name);
        let error = source_type_declaration(&declaration).unwrap_err();
        assert!(error.contains(format!("`{name}`").as_str()) && error.contains("unqualified"));
        assert!(validate_source_type_declaration(&declaration).is_err());
    }
}

#[test]
fn nominal_detection_and_validation_include_record_defaults_and_variant_constraints() {
    let mut field = RecordField::new(
        Visibility::Public,
        ident("value"),
        Type::new(TypeKind::Int, Span::dummy()),
        Span::dummy(),
    );
    field.default_value = Some(Expr::new(
        ExprKind::TypeExpr(universe(variable("free"))),
        Span::dummy(),
    ));
    let default = type_declaration(TypeDeclBody::Record(List::from_iter([field])));
    let mut variant = Variant::new(ident("Pack"), None, Span::dummy());
    let mut bound = ordinary("F");
    let GenericParamKind::Type { bounds, .. } = &mut bound.kind else {
        panic!("type")
    };
    bounds.push(TypeBound {
        kind: TypeBoundKind::Equality(Type::new(
            TypeKind::Function {
                is_async: false,
                params: List::new(),
                return_type: Heap::new(universe(variable("free"))),
                contexts: Default::default(),
                calling_convention: None,
            },
            Span::dummy(),
        )),
        span: Span::dummy(),
    });
    variant.generic_params.push(bound);
    let variant = type_declaration(TypeDeclBody::Variant(List::from_iter([variant])));
    let mut axiom = verum_ast::decl::AxiomDecl::new(
        ident("law"),
        Expr::literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    axiom.return_type = Some(universe(variable("free")));
    let axiom = type_declaration(TypeDeclBody::Protocol(ProtocolBody::new(List::from_iter(
        [ProtocolItem {
            kind: ProtocolItemKind::Axiom(axiom),
            span: Span::dummy(),
        }],
    ))));
    for declaration in [default, variant, axiom] {
        assert!(type_declaration_uses_universes(&declaration).unwrap());
        let error = source_type_declaration(&declaration).unwrap_err();
        assert!(
            error.contains("Carrier") && error.contains("free"),
            "{error}"
        );
    }
}

#[test]
fn nested_callable_contexts_retain_exact_declaration_carriers() {
    let callback = Type::new(TypeKind::Function {
        is_async: true, params: vec![Type::new(TypeKind::Int, Span::dummy())].into(),
        return_type: Heap::new(Type::new(TypeKind::Int, Span::dummy())),
        calling_convention: None,
        contexts: verum_ast::context::ContextList::new(vec![
            verum_ast::context::ContextRequirement::simple(Path::single(ident("Trace")), List::new(), Span::dummy()),
        ].into()),
    }, Span::dummy());
    let nested = Type::new(TypeKind::Tuple(vec![callback].into()), Span::dummy());
    let mut declaration = function();
    declaration.params.push(parameter(nested.clone()));
    let carrier = source_signature(&declaration, false).unwrap().expect("context declaration carrier");
    assert_eq!(carrier.params, declaration.params);
    validate_source_signature(&carrier, false).unwrap();
    let nominal = type_declaration(TypeDeclBody::Alias(nested));
    let nominal_carrier = source_type_declaration(&nominal).unwrap().expect("nested alias carrier");
    assert_eq!(nominal_carrier.body, nominal.body);
    validate_source_type_declaration(&nominal_carrier).unwrap();
}
