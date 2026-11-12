use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<Text> = errors.iter().map(|e| e.to_string().into()).collect();
    messages.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| d.message().into()));
    messages
}

#[test]
fn iteration_releases_its_temporary_borrow_after_the_loop() {
    let source = "fn run() -> Int {
        let mut values = [1, 2];
        for item in &values { let _read = *item; }
        values[0] = 7;
        values[0]
    }";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn mutable_iteration_releases_its_temporary_loan() {
    let errors = errors("fn run() -> Int { let mut values = [1, 2]; for item in &mut values { *item = 3; } values[0] = 7; values[0] }");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_escaped_reference_can_still_read_its_live_source() {
    let errors = errors("fn run() -> Int { let values = [1, 2]; let spare = 0; let mut held = &spare; for item in &values { held = item; } *held }");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_escaped_mutable_reference_preserves_exclusive_access() {
    let errors = errors("fn run() -> Int { let mut values = [1, 2]; let mut spare = 0; let mut held = &mut spare; for item in &mut values { held = item; } *held = 7; *held }");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn iteration_cannot_release_an_outer_or_escaped_loan() {
    for source in [
        "fn bad() { let mut values = [1, 2]; for item in &values { values[0] = 7; let _n = *item; } }",
        "fn bad() { let mut values = [1, 2]; let held = &values; for item in &values {} values[0] = 7; let _n = held[0]; }",
        "fn bad() { let mut values = [1, 2]; for outer in &values { for inner in &values {} values[0] = 7; let _n = *outer; } }",
        "fn bad() -> Int { let mut values = [1, 2]; let spare = 0; let mut held = &spare; for item in &values { held = item; } values[0] = 7; *held }",
        "fn identity(x: &Int) -> &Int { x } fn bad() -> Int { let mut values = [1, 2]; let spare = 0; let mut held = &spare; for item in &values { held = identity(item); } values[0] = 7; *held }",
        "fn bad() -> Int { let mut values = [1, 2]; let spare = 0; let mut held = (&spare,); for item in &values { held = (item,); } values[0] = 7; *held.0 }",
        "type Holder is { value: &Int }; fn bad() -> Int { let mut values = [1, 2]; let spare = 0; let mut held = Holder { value: &spare }; for item in &values { held.value = item; } values[0] = 7; *held.value }",
        "fn bad(flag: Bool) -> Int { let mut values = [1, 2]; let spare = 0; let mut held = &spare; for item in &values { if flag { held = item; } } values[0] = 7; *held }",
        "fn bad() -> Int { let values = [1, 2]; let spare = 0; let mut held = &spare; for item in &values { let local = 3; held = &local; } *held }",
        "fn bad() -> Int { let spare = 0; let mut held = &spare; for item in &[1, 2] { held = item; } *held }",
        "fn bad() -> Int { let mut values = [1, 2]; let mut spare = 0; let mut held = &mut spare; for item in &mut values { held = item; } values[0] = 7; *held }",
        "fn bad() -> Int { let mut values = [1, 2]; let spare = 0; let mut held = &spare; { held = &values[0]; } values[0] = 7; *held }",
    ] {
        let errors = errors(source);
        assert!(!errors.is_empty(), "released a live borrow: {source}");
        assert!(errors.iter().any(|e| e.contains("borrow") || e.contains("lifetime") || e.contains("outlive") || e.contains("reference")), "unrelated refusal: {source}: {errors:?}");
    }
}
