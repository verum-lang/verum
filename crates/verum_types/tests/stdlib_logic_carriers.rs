//! Check the actual empty/witness declarations, without substituting model types.
//! Other logic declarations and runtime execution have separate qualification.
use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn declarations() -> String {
    let source = include_str!("../../../core/math/logic.vr");
    let module = Parser::new(source).parse_module().expect("actual logic source");
    let mut declarations = Vec::new();
    for item in &module.items {
        let name = match &item.kind {
            ItemKind::Type(ty) => ty.name.name.as_str(),
            ItemKind::Function(function) => function.name.name.as_str(),
            _ => continue,
        };
        if ["Void", "Proof", "absurd"].contains(&name) {
            declarations.push(&source[item.span.start as usize..item.span.end as usize]);
        }
    }
    assert_eq!(declarations.len(), 3);
    declarations.join("\n")
}

fn errors(consumer: &str) -> Vec<String> {
    let source = format!("{}\n{consumer}", declarations());
    let module = Parser::new(&source).parse_module().expect("valid consumer syntax");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("core.math.logic");
    let mut errors: Vec<_> = checker.prepare_module_declarations(&module)
        .iter().map(|error| format!("{error:?}")).collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}")); }
    }
    errors.extend(checker.take_deferred_errors().iter().map(|error| format!("{error:?}")));
    errors.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| format!("{d:?}")));
    errors
}

#[test]
fn source_contracts_admit_exact_witnesses_and_empty_elimination() {
    for source in [
        "",
        "fn eliminate<A>(value: Void) -> A { value }",
        "fn identity(value: Int) -> Proof<Int> { Proof { witness: value } }",
        "fn identity(value: Bool) -> Proof<Bool> { Proof { witness: value } }",
        "fn project(value: Proof<Int>) -> Int { value.witness }",
    ] {
        let errors = errors(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn empty_carrier_has_no_unit_or_record_constructor() {
    assert!(errors("").is_empty(), "the declarations themselves must be valid");
    for source in [
        "fn forge() -> Void { () }",
        "fn forge() -> Void { Void {} }",
        "fn forge(value: unknown) -> Void { value }",
    ] {
        assert!(!errors(source).is_empty(), "accepted: {source}");
    }
}

#[test]
fn witness_must_have_its_declared_carrier() {
    assert!(errors("").is_empty(), "the declarations themselves must be valid");
    for source in [
        "fn forge() -> Proof<Bool> { Proof { witness: 7 } }",
        "fn forge() -> Proof<Void> { Proof { witness: () } }",
        "fn forge() -> Proof<Int> { Proof {} }",
    ] {
        assert!(!errors(source).is_empty(), "accepted: {source}");
    }
}
