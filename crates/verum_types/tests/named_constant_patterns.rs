//! Nullary pattern binding collection must not require a sum-type scrutinee.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors: List<Text> = checker.prepare_module_declarations(&ast)
        .into_iter().map(|error| format!("{error:?}").into()).collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| Text::from(format!("{error:?}"))));
    errors.extend(checker.diagnostics().iter().filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| Text::from(diagnostic.message())));
    errors
}

#[test]
fn declared_scalar_constants_work_in_single_and_or_patterns() {
    for arms in ["FIRST => 7, SECOND => 9", "FIRST | SECOND => 7"] {
        let source = format!("const FIRST: Int = 3; const SECOND: Int = 5;
            fn classify(value: Int) -> Int {{ match value {{ {arms}, _ => 0 }} }}");
        assert!(errors(&source).is_empty(), "{:?}", errors(&source));
    }
}

#[test]
fn collecting_no_bindings_does_not_authorize_invalid_patterns() {
    for source in [
        "fn classify(value: Int) -> Int { match value { MISSING => 1, _ => 0 } }",
        "const WRONG: Bool = true; fn classify(value: Int) -> Int { match value { WRONG => 1, _ => 0 } }",
        "type Choice is Present | Absent; fn classify(value: Choice) -> Int { match value { Missing => 1, _ => 0 } }",
        "const FIRST: Int = 3; fn classify(value: Int) -> Int { match value { FIRST | MISSING => 1, _ => 0 } }",
    ] {
        assert!(!errors(source).is_empty(), "invalid pattern accepted: {source}");
    }
}
