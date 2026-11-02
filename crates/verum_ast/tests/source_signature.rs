use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, TypeDecl, TypeDeclBody,
    Visibility,
};
use verum_ast::source_signature::{
    signature_uses_universes, source_signature, type_declaration_uses_universes,
    type_uses_universes, validate_source_signature,
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
