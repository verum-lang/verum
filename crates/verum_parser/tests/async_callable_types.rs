use verum_ast::{FileId, ItemKind, TypeDeclBody, TypeKind, pretty::format_type};
use verum_parser::LosslessParser;

#[test]
fn callable_tokens_are_available_at_every_editor_entry_point() {
    use verum_parser::EventBasedParser;
    for (source, parsed) in [
        ("async fn(Int) -> Int", EventBasedParser::parse_type("async fn(Int) -> Int")),
        ("type Callback is async fn(Int) -> Int;", EventBasedParser::parse_item("type Callback is async fn(Int) -> Int;")),
        ("let callback: async fn(Int) -> Int = ready;", EventBasedParser::parse_statement("let callback: async fn(Int) -> Int = ready;")),
        ("{ let callback: async fn(Int) -> Int = ready; }", EventBasedParser::parse_block("{ let callback: async fn(Int) -> Int = ready; }")),
        ("{ let callback: async fn(Int) -> Int = ready; }", EventBasedParser::parse_expression("{ let callback: async fn(Int) -> Int = ready; }")),
    ] {
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.green.text(), source);
    }
    let parser = EventBasedParser::new();
    for source in [
        "type First is async fn<T>(T) -> T;",
        "type Second is fn(Int) -> Bool;",
    ] {
        let parsed = parser.parse(source, FileId::new(14));
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.green.text(), source);
    }
}
#[test]
fn lossless_callable_preserves_rank2_context_and_exact_source_spans() {
    let source = "// leading text\n type Callback is async fn<T>(T) -> T using [Clock];\n";
    let parsed = LosslessParser::new().parse(source, FileId::new(11));
    assert!(parsed.ok(), "{:?}", parsed.errors);
    assert_eq!(parsed.text(), source);
    let ItemKind::Type(decl) = &parsed.module.items[0].kind else {
        panic!("type alias")
    };
    let TypeDeclBody::Alias(ty) = &decl.body else {
        panic!("alias")
    };
    let TypeKind::Rank2Function {
        is_async,
        type_params,
        contexts,
        return_type,
        ..
    } = &ty.kind
    else {
        panic!("{ty:?}")
    };
    assert!(*is_async);
    assert_eq!(type_params.len(), 1);
    assert_eq!(contexts.requirements.len(), 1);
    assert_eq!(return_type.span.file_id, FileId::new(11));
    assert_eq!(
        &source[return_type.span.start as usize..return_type.span.end as usize],
        "T"
    );
    assert_eq!(
        format_type(ty).as_str(),
        "async fn<T>(T) -> T using [Clock]"
    );
}
#[test]
fn lossless_nested_callable_retains_independent_markers() {
    let source = "type Callback is fn(async fn(Int) -> Bool) -> async fn() -> Int;";
    let parsed = LosslessParser::new().parse(source, FileId::new(12));
    assert!(parsed.ok(), "{:?}", parsed.errors);
    assert_eq!(parsed.text(), source);
    let ItemKind::Type(decl) = &parsed.module.items[0].kind else {
        panic!("type alias")
    };
    let TypeDeclBody::Alias(ty) = &decl.body else {
        panic!("alias")
    };
    let TypeKind::Function {
        is_async,
        params,
        return_type,
        ..
    } = &ty.kind
    else {
        panic!("function")
    };
    assert!(!is_async);
    assert!(matches!(
        params[0].kind,
        TypeKind::Function { is_async: true, .. }
    ));
    assert!(matches!(
        return_type.kind,
        TypeKind::Function { is_async: true, .. }
    ));
}

#[test]
fn many_callable_declarations_remain_lossless_with_exact_spans() {
    let mut source = String::new();
    for index in 0..512 {
        source.push_str(&format!("// callback {index}\ntype Callback{index} is async fn<T>(T, fn(Int) -> Int) -> T using [Clock];\n"));
    }
    let parsed = LosslessParser::new().parse(&source, FileId::new(13));
    assert!(parsed.ok(), "{:?}", parsed.errors);
    assert_eq!(parsed.text(), source);
    assert_eq!(parsed.module.items.len(), 512);
    for (index, item) in parsed.module.items.iter().enumerate() {
        let ItemKind::Type(decl) = &item.kind else {
            panic!("type alias")
        };
        assert_eq!(decl.name.name.as_str(), format!("Callback{index}"));
        let TypeDeclBody::Alias(ty) = &decl.body else {
            panic!("alias")
        };
        let TypeKind::Rank2Function {
            is_async,
            type_params,
            params,
            return_type,
            contexts,
            ..
        } = &ty.kind
        else {
            panic!("rank2")
        };
        assert!(*is_async);
        assert_eq!(type_params.len(), 1);
        assert_eq!(params.len(), 2);
        assert!(matches!(
            params[1].kind,
            TypeKind::Function {
                is_async: false,
                ..
            }
        ));
        assert_eq!(contexts.requirements.len(), 1);
        assert_eq!(return_type.span.file_id, FileId::new(13));
        assert_eq!(
            &source[return_type.span.start as usize..return_type.span.end as usize],
            "T"
        );
    }
}
