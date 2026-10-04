use super::instantiate;
use verum_ast::ty::TypeKind;
use verum_fast_parser::Parser;

#[test]
fn projected_parameter_replaces_only_the_root_token() {
    assert_eq!(
        instantiate("Holder<Config>", &["T".into()], "T.Item"),
        Some("Config.Item".into())
    );
}

#[test]
fn callback_contexts_refuse_partial_substitution() {
    for source in [
        "fn(T) -> T using [Logger<T>]",
        "fn<R>(T) -> R using [Logger<T>]",
    ] {
        let ast = Parser::new(source)
            .parse_type()
            .expect("supported callback syntax");
        assert!(match ast.kind {
            TypeKind::Function { contexts, .. } | TypeKind::Rank2Function { contexts, .. } =>
                !contexts.requirements.is_empty(),
            _ => false,
        });
        assert_eq!(instantiate("Holder<Int>", &["T".into()], source), None);
    }
}

#[test]
fn callback_where_clause_refuses_partial_substitution() {
    let source = "fn<R>(T) -> R where type R: Clone";
    let ast = Parser::new(source)
        .parse_type()
        .expect("supported rank-two syntax");
    assert!(
        matches!(ast.kind, TypeKind::Rank2Function { where_clause, .. }
        if where_clause.is_some())
    );
    assert_eq!(instantiate("Holder<Int>", &["T".into()], source), None);
}

#[test]
fn array_element_and_const_length_are_both_bound() {
    assert_eq!(
        instantiate("Holder<Int, 3>", &["T".into(), "N".into()], "[T; N]"),
        Some("[Int; 3]".into())
    );
}

#[test]
fn substitution_respects_the_type_size_budget() {
    let owner = format!("Holder<{}>", "A".repeat(65_537));
    assert_eq!(instantiate(&owner, &["T".into()], "T"), None);
}
