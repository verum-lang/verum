use verum_ast::{FileId, ItemKind, TypeKind, ty::{GenericParamKind, TypeBoundKind, WherePredicateKind}};
use verum_parser::LosslessParser;

#[test]
fn editor_ast_preserves_callable_capabilities_and_exact_function_equations() {
    for (signature, asynchronous, rank2) in [
        ("fn(Int) -> Int", false, false),
        ("fn<A>(A) -> A", false, true),
        ("async fn(Int) -> Int", true, false),
        ("async fn<A>(A) -> A using [Clock]", true, true),
    ] {
        for prefix in ["", "type "] {
            let source = format!("fn capability<F: {signature}>(callback: F) -> Int {{ 0 }}\nfn bounded<F>(callback: F) -> Int where {prefix}F: {signature} {{ 0 }}\nfn exact<F>(callback: F) -> Int where {prefix}F = {signature} {{ 0 }}");
            let parsed = LosslessParser::new().parse(&source, FileId::new(43));
            assert!(parsed.ok(), "{source}: {:?}", parsed.errors);
            assert_eq!(parsed.text(), source);
            assert_eq!(parsed.module.items.len(), 3);
            for (index, item) in parsed.module.items.iter().enumerate() {
                let ItemKind::Function(function) = &item.kind else { panic!("function") };
                let bounds = if index == 0 {
                    let GenericParamKind::Type { bounds, .. } = &function.generics[0].kind else { panic!("parameter") };
                    bounds
                } else {
                    let WherePredicateKind::Type { bounds, .. } = &function.generic_where_clause.as_ref().unwrap().predicates[0].kind else { panic!("predicate") };
                    bounds
                };
                let [bound] = bounds.as_slice() else { panic!("one bound: {bounds:?}") };
                let ty = match &bound.kind {
                    TypeBoundKind::Callable(ty) if index < 2 => ty,
                    TypeBoundKind::Equality(ty) if index == 2 => ty,
                    other => panic!("wrong bound identity: {other:?}"),
                };
                match &ty.kind {
                    TypeKind::Function { is_async, .. } if !rank2 => assert_eq!(*is_async, asynchronous),
                    TypeKind::Rank2Function { is_async, type_params, .. } if rank2 => {
                        assert_eq!(*is_async, asynchronous);
                        assert_eq!(type_params.len(), 1);
                    }
                    other => panic!("wrong callable shape: {other:?}"),
                }
                assert_eq!(ty.span.file_id, FileId::new(43));
                assert_eq!(&source[ty.span.start as usize..ty.span.end as usize], signature);
            }
        }
    }
}

#[test]
fn editor_bound_lists_keep_protocols_defaults_and_refuse_missing_types() {
    let source = "fn chosen<F: Family + fn(Int) -> Int + !Other = fn(Int) -> Int>(callback: F) -> Int { 0 }";
    let parsed = LosslessParser::new().parse(source, FileId::new(44));
    assert!(parsed.ok(), "{:?}", parsed.errors);
    assert_eq!(parsed.text(), source);
    let ItemKind::Function(function) = &parsed.module.items[0].kind else { panic!("function") };
    let GenericParamKind::Type { bounds, default, .. } = &function.generics[0].kind else { panic!("parameter") };
    assert!(matches!(bounds.as_slice(), [protocol, callable, negative]
        if matches!(protocol.kind, TypeBoundKind::Protocol(_))
        && matches!(callable.kind, TypeBoundKind::Callable(_))
        && matches!(negative.kind, TypeBoundKind::NegativeProtocol(_))));
    assert!(matches!(default.as_ref().unwrap().kind, TypeKind::Function { .. }));
    for source in [
        "fn broken<F:>(callback: F) -> Int { 0 }",
        "fn broken<F: fn(Int) ->>(callback: F) -> Int { 0 }",
        "fn broken<F>(callback: F) -> Int where F = { 0 }",
        "fn broken<F>(callback: F) -> Int where F: Family + { 0 }",
    ] {
        assert!(!LosslessParser::new().parse(source, FileId::new(45)).ok(), "{source}");
    }
}
