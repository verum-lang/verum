//! Public checker accepts the structural argument parsed at the call site.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("signature");
        }
    }
    let mut errors: List<_> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn structural_layout_calls_typecheck_from_source() {
    for ty in [
        "&Byte",
        "&mut Byte",
        "&checked Byte",
        "&unsafe Byte",
        "Byte",
    ] {
        let source =
            format!("fn size<T>() -> Int {{ T.size }} fn probe() -> Int {{ size<{ty}>() }}");
        assert!(
            errors(&source).is_empty(),
            "{source}: {:?}",
            errors(&source)
        );
    }
}

#[test]
fn structural_argument_rejects_an_unrelated_value() {
    for ty in ["Bool", "&Byte"] {
        let source = format!(
            "fn accept<T>(value: T) -> Int {{ 1 }} fn probe() -> Int {{ accept<{ty}>(\"wrong\") }}"
        );
        assert!(!errors(&source).is_empty(), "{source}");
    }
}
