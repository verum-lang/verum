//! Inspecting a sum's cases must not erase its declared identity at a value boundary.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

#[test]
fn alias_resolution_stops_at_nominal_identity_and_rejects_cycles() {
    use verum_ast::{Ident, Span, ty::Path};
    use verum_types::{Type, context::TypeContext};
    let named = |name: &str| Type::Named {
        path: Path::single(Ident::new(name, Span::default())),
        args: List::new(),
    };
    let mut context = TypeContext::new();
    context.define_type("owner.First", Type::Variant(Default::default()));
    context.define_alias("Alias", named("owner.First"));
    context.define_alias("AnotherAlias", named("Alias"));
    assert!(context.resolve_alias("owner.First").is_none());
    assert_eq!(context.resolve_alias("Alias"), Some(&named("owner.First")));
    assert_eq!(
        context.resolve_alias("AnotherAlias"),
        Some(&named("owner.First"))
    );
    context.define_alias("CycleA", named("CycleB"));
    context.define_alias("CycleB", named("CycleA"));
    assert!(context.resolve_alias("CycleA").is_none());
}

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

const SUMS: &str = "type First<T> is Take(T) | Skip; type Second<T> is Take(T) | Skip;";

#[test]
fn matching_cases_do_not_identify_distinct_sums() {
    for body in [
        "fn convert(value:First<Int>)->Second<Int> {value}",
        "fn convert(value:First<Int>) {let other:Second<Int> = value;}",
        "fn convert(value:&First<Int>)->&Second<Int> {value}",
        "fn convert(value:fn()->First<Int>)->fn()->Second<Int> {value}",
        "fn convert(value:(First<Int>,Bool))->(Second<Int>,Bool) {value}",
    ] {
        let source = format!("{SUMS} {body}");
        let errors = errors(&source);
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn partially_overlapping_cases_do_not_authorize_nominal_conversion() {
    let source =
        "type First is A | B; type Second is A | B | C; fn convert(value:First)->Second {value}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")),
        "{errors:?}"
    );
}

#[test]
fn aliases_and_declared_case_inspection_preserve_identity() {
    for body in [
        "type Alias<T> is First<T>; fn convert(value:First<Int>)->Alias<Int> {value}",
        "type Alias<T> is First<T>; fn convert(value:Alias<Int>)->First<Int> {value}",
        "fn make()->First<Int> {First.Take(7)} fn inspect(value:First<Int>)->Int {match value {First.Take(n)=>n, First.Skip=>0}}",
        "fn convert(value:fn()->First<Int>)->fn()->First<Int> {value}",
    ] {
        let source = format!("{SUMS} {body}");
        let errors = errors(&source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}
