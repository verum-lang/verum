use verum_ast::{FileId, Type, TypeKind, pretty::format_type};
use verum_fast_parser::VerumParser;
fn parse(source: &str) -> Type {
    VerumParser::new()
        .parse_type_str(source, FileId::new(7))
        .expect("callable grammar")
}
#[test]
fn eager_async_callable_retains_output_instead_of_a_nominal_future() {
    let ty = parse("async fn(Int) -> Bool");
    let TypeKind::Function {
        is_async,
        params,
        return_type,
        ..
    } = &ty.kind
    else {
        panic!("{ty:?}")
    };
    assert!(*is_async);
    assert!(matches!(params[0].kind, TypeKind::Int));
    assert!(matches!(return_type.kind, TypeKind::Bool));
    assert_eq!(format_type(&ty).as_str(), "async fn(Int) -> Bool");
    assert!(!matches!(
        parse("fn(Int) -> Bool").kind,
        TypeKind::Function { is_async: true, .. }
    ));
}
#[test]
fn rank2_context_and_nested_callable_markers_survive_printing() {
    for source in [
        "async fn<T>(T) -> T using [Clock]",
        "fn(async fn(Int) -> Bool) -> async fn() -> Int",
        "async fn<T>(fn(T) -> T) -> T",
        "async fn()",
    ] {
        let printed = format_type(&parse(source));
        assert_eq!(format_type(&parse(printed.as_str())), printed);
    }
    let ty = parse("async fn<T>(T) -> T using [Clock]");
    let TypeKind::Rank2Function {
        is_async,
        type_params,
        params,
        return_type,
        contexts,
        ..
    } = ty.kind
    else {
        panic!("rank2")
    };
    assert!(is_async);
    assert_eq!(type_params.len(), 1);
    assert_eq!(contexts.requirements.len(), 1);
    assert!(matches!(params[0].kind, TypeKind::Path(_)));
    assert!(matches!(return_type.kind, TypeKind::Path(_)));
}
#[test]
fn user_declared_future_spelling_has_no_role_in_async_callable_parsing() {
    let ty = parse("async fn(Future) -> Future");
    let TypeKind::Function {
        is_async,
        return_type,
        ..
    } = ty.kind
    else {
        panic!("function")
    };
    assert!(is_async);
    let TypeKind::Path(path) = return_type.kind else {
        panic!("written Future must remain an ordinary path")
    };
    assert_eq!(path.to_string(), "Future");
}

#[test]
fn callable_bounds_after_plus_keep_their_complete_syntax() {
    for bound in ["fn(Int) -> Int", "async fn(Int) -> Int", "fn<T>(T) -> T"] {
        let source = format!(
            "type Carrier is protocol {{ fn apply<F: fn(Int) -> Int + {bound}>(&self, value: F); }};"
        );
        let module = verum_fast_parser::Parser::new(&source)
            .parse_module()
            .unwrap();
        let verum_ast::ItemKind::Type(decl) = &module.items[0].kind else {
            panic!()
        };
        let verum_ast::decl::TypeDeclBody::Protocol(protocol) = &decl.body else {
            panic!()
        };
        let verum_ast::decl::ProtocolItemKind::Function { decl: method, .. } = &protocol.items[0].kind
        else {
            panic!()
        };
        let verum_ast::ty::GenericParamKind::Type { bounds, .. } = &method.generics[0].kind else {
            panic!()
        };
        assert_eq!(bounds.len(), 2);
    }
}

#[test]
fn arrow_kind_syntax_does_not_capture_universe_bounds() {
    for source in [
        "fn use_kind<F: Type -> Type>() {}",
        "fn use_kind<F: (Type -> Type) -> Type>() {}",
        "fn use_kind<F: Type -> (Type -> Type)>() {}",
    ] {
        let module = verum_fast_parser::Parser::new(source)
            .parse_module()
            .unwrap();
        let verum_ast::ItemKind::Function(decl) = &module.items[0].kind else {
            panic!()
        };
        assert!(matches!(
            decl.generics[0].kind,
            verum_ast::ty::GenericParamKind::KindAnnotated { .. }
        ));
    }
    for source in [
        "fn keep<A: Type>() {}",
        "fn keep<u: Level, A: Type(u)>() {}",
    ] {
        let module = verum_fast_parser::Parser::new(source)
            .parse_module()
            .unwrap();
        let verum_ast::ItemKind::Function(decl) = &module.items[0].kind else {
            panic!()
        };
        assert!(matches!(
            decl.generics.last().unwrap().kind,
            verum_ast::ty::GenericParamKind::Type { .. }
        ));
    }
}
