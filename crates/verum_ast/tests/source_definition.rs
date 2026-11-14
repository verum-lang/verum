use verum_ast::decl::Visibility;
use verum_ast::source_definition::{self, Error, Limits};
use verum_ast::{Expr, ExprKind, FileId, FunctionBody, FunctionDecl, Ident, Literal, Span};
use verum_common::{Heap, List};

fn ident(name: &str) -> Ident {
    Ident::new(name, Span::dummy())
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
fn encoded(function: &FunctionDecl, owner: &str) -> List<u8> {
    let mut bytes = List::new();
    source_definition::encode(
        function,
        owner,
        None,
        None,
        &[],
        Limits::default(),
        &mut |part| bytes.extend_from_slice(part),
    )
    .unwrap();
    bytes
}
fn integer(value: i64) -> Expr {
    Expr::literal(Literal::int(value.into(), Span::dummy()))
}

#[test]
fn complete_body_and_each_contract_are_significant() {
    let mut original = function();
    original.body = Some(FunctionBody::Expr(integer(73)));
    let baseline = encoded(&original, "owner");
    let mut changed = original.clone();
    changed.body = Some(FunctionBody::Expr(integer(74)));
    assert_ne!(baseline, encoded(&changed, "owner"));
    for clause in 0..3 {
        let mut changed = original.clone();
        match clause {
            0 => changed.requires.push(integer(1)),
            1 => changed.ensures.push(integer(1)),
            _ => changed.decreases.push(integer(1)),
        }
        assert_ne!(baseline, encoded(&changed, "owner"));
    }
    assert_ne!(baseline, encoded(&original, "foreign"));
    let mut changed = original.clone();
    changed.attributes.push(verum_ast::Attribute::new(
        "custom".into(),
        Some([integer(1)].into_iter().collect()),
        Span::dummy(),
    ));
    let with_attribute = encoded(&changed, "owner");
    changed.attributes[0].args.as_mut().unwrap()[0] = integer(2);
    assert_ne!(baseline, with_attribute);
    assert_ne!(with_attribute, encoded(&changed, "owner"));
}

#[test]
fn file_ids_offsets_and_allocation_addresses_are_not_definition_inputs() {
    let mut first = function();
    first.body = Some(FunctionBody::Expr(integer(73)));
    let mut relocated = first.clone();
    relocated.span = Span::new(100, 200, FileId::new(99));
    relocated.name.span = Span::new(101, 107, FileId::new(99));
    let Some(FunctionBody::Expr(expression)) = &mut relocated.body else {
        panic!("body");
    };
    expression.span = Span::new(170, 172, FileId::new(99));
    let ExprKind::Literal(literal) = &mut expression.kind else {
        panic!("literal");
    };
    literal.span = expression.span;
    assert_eq!(encoded(&first, "owner"), encoded(&relocated, "owner"));
}

#[test]
fn float_encoding_preserves_zero_sign_and_nan_payloads() {
    let mut function = function();
    let mut encodings = List::new();
    for bits in [0u64, 1u64 << 63, 0x7ff8000000000001, 0x7ff8000000000002] {
        function.body = Some(FunctionBody::Expr(Expr::literal(Literal::float(
            f64::from_bits(bits),
            Span::dummy(),
        ))));
        let encoded = encoded(&function, "owner");
        assert!(!encodings.contains(&encoded));
        encodings.push(encoded);
    }
}

#[test]
fn a_deep_definition_uses_an_ordinary_thread_stack() {
    std::thread::spawn(|| {
        let mut expression = integer(73);
        for _ in 0..10_000 {
            expression = Expr::new(
                ExprKind::Unary {
                    op: verum_ast::UnOp::Neg,
                    expr: Heap::new(expression),
                },
                Span::dummy(),
            );
        }
        let mut function = function();
        function.body = Some(FunctionBody::Expr(expression));
        let mut bytes = 0;
        source_definition::encode(
            &function,
            "owner",
            None,
            None,
            &[],
            Limits::default(),
            &mut |part| bytes += part.len(),
        )
        .unwrap();
        assert!(bytes > 10_000);
        // AST destruction has an independent recursive implementation. Dispose
        // this deliberately extreme fixture iteratively as well.
        let Some(FunctionBody::Expr(mut expression)) = function.body.take() else {
            panic!("body");
        };
        while let ExprKind::Unary { expr, .. } = expression.kind {
            expression = *expr;
        }
    })
    .join()
    .unwrap();
}

#[test]
fn resource_exhaustion_refuses_before_unbounded_work_or_output() {
    let mut function = function();
    function.body = Some(FunctionBody::Expr(integer(73)));
    for limits in [
        Limits {
            nodes: 4,
            bytes: 10000,
        },
        Limits {
            nodes: 10000,
            bytes: 32,
        },
    ] {
        let mut emitted = 0;
        let error =
            source_definition::encode(&function, "owner", None, None, &[], limits, &mut |part| {
                emitted += part.len()
            })
            .unwrap_err();
        assert!(matches!(
            error,
            Error::NodeLimit { limit: 4 } | Error::ByteLimit { limit: 32 }
        ));
        assert!(emitted <= limits.bytes);
    }
}

#[test]
fn lexical_imports_are_part_of_the_definition_input() {
    use source_definition::LexicalScope;
    let mut function = function();
    function.body = Some(FunctionBody::Expr(integer(73)));
    let baseline_scope = LexicalScope {
        owner: "owner".into(),
        known: true,
        named: [(
            "selected".into(),
            Some(["first.value".into()].into_iter().collect()),
        )]
        .into_iter()
        .collect(),
        globs: ["fallback".into()].into_iter().collect(),
    };
    let encode = |scope: &LexicalScope| {
        let mut bytes = List::new();
        source_definition::encode(
            &function,
            "owner",
            None,
            None,
            std::slice::from_ref(scope),
            Limits::default(),
            &mut |part| bytes.extend_from_slice(part),
        )
        .unwrap();
        bytes
    };
    let baseline = encode(&baseline_scope);
    let mut rebound = baseline_scope.clone();
    rebound.named[0].1.as_mut().unwrap()[0] = "second.value".into();
    assert_ne!(baseline, encode(&rebound));
    let mut unknown = baseline_scope.clone();
    unknown.known = false;
    assert_ne!(baseline, encode(&unknown));
    let mut glob = baseline_scope;
    glob.globs[0] = "other".into();
    assert_ne!(baseline, encode(&glob));
}

#[test]
fn implementation_selection_is_not_complete_definition_authority() {
    use verum_ast::decl::{ImplDecl, ImplItem, ImplItemKind, ImplKind};
    use verum_ast::{Item, ItemKind, Path, Type, TypeKind};
    let mut function = function();
    function.body = Some(FunctionBody::Expr(integer(73)));
    let item = Item::new(
        ItemKind::Impl(ImplDecl {
            is_unsafe: false,
            generics: List::new(),
            kind: ImplKind::Inherent(Type::new(
                TypeKind::Path(Path::single(ident("Token"))),
                Span::dummy(),
            )),
            generic_where_clause: None,
            meta_where_clause: None,
            specialize_attr: None,
            items: [ImplItem {
                attributes: List::new(),
                visibility: Visibility::Public,
                kind: ImplItemKind::Function(function),
                span: Span::dummy(),
            }]
            .into_iter()
            .collect(),
            span: Span::dummy(),
        }),
        Span::dummy(),
    );
    let encode = |item: &Item, selection_only| {
        let mut bytes = List::new();
        if selection_only {
            source_definition::encode_implementation_scope(
                item,
                "owner",
                Limits::default(),
                &mut |part| bytes.extend_from_slice(part),
            )
            .unwrap();
        } else {
            source_definition::encode_implementation(
                item,
                "owner",
                &[],
                Limits::default(),
                &mut |part| bytes.extend_from_slice(part),
            )
            .unwrap();
        }
        bytes
    };
    let mut changed = item.clone();
    let ItemKind::Impl(implementation) = &mut changed.kind else {
        panic!("impl");
    };
    let ImplItemKind::Function(function) = &mut implementation.items[0].kind else {
        panic!("method");
    };
    function.body = Some(FunctionBody::Expr(integer(74)));
    function.requires.push(integer(1));
    assert_eq!(encode(&item, true), encode(&changed, true));
    assert_ne!(encode(&item, false), encode(&changed, false));
    let ItemKind::Impl(implementation) = &mut changed.kind else {
        panic!("impl");
    };
    implementation.is_unsafe = true;
    assert_ne!(encode(&item, true), encode(&changed, true));
}
