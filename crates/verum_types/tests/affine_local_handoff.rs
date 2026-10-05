//! T1602: source diagnostics agree with consuming local initialization.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn diagnostics(source: &str) -> List<Text> {
    let ast = Parser::new(source)
        .parse_module()
        .expect("grammar-valid source");
    let mut checker = TypeChecker::new();
    for item in &ast.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker.register_type_declaration(decl).unwrap();
        }
    }
    for item in &ast.items {
        if let ItemKind::Function(decl) = &item.kind {
            checker.register_function_signature(decl).unwrap();
        }
    }
    let mut errors = List::new();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(Text::from(format!("{error:?}")));
        }
    }
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
const RESOURCE: &str = "type affine Token is { value: Int };";
#[test]
fn affine_source_is_unavailable_after_local_transfer() {
    for body in [
        "let moved = original; original.value",
        "if flag { let moved = original; } original.value",
        "let moved = original; let second = original; second.value",
    ] {
        let source: Text =
            format!("{RESOURCE} fn probe(original: Token, flag: Bool) -> Int {{ {body} }}").into();
        let errors = diagnostics(&source);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("MovedValueUsed") || e.contains("PartiallyMovedValue")),
            "{source}: {errors:?}"
        );
    }
}
#[test]
fn exclusive_moves_and_borrows_keep_their_source_rules() {
    for body in [
        "if flag { let moved = original; moved.value } else { let other = original; other.value }",
        "let borrowed = &original; borrowed.value + original.value",
    ] {
        let source: Text =
            format!("{RESOURCE} fn probe(original: Token, flag: Bool) -> Int {{ {body} }}").into();
        let errors = diagnostics(&source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}
#[test]
fn ordinary_copy_does_not_apply_affine_consumption() {
    let errors = diagnostics(
        "type Token is { value: Int }; fn probe(original: Token) -> Int { let copied = original; copied.value + original.value }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}
