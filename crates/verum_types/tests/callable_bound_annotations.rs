use verum_ast::{
    Expr, ExprKind, FileId, Span,
    visitor::{Visitor, walk_expr},
};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, ty::Type, type_registry::TypeRegistry};

#[derive(Default)]
struct Closures(List<Span>);

impl Visitor for Closures {
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr.kind, ExprKind::Closure { .. }) {
            self.0.push(expr.span);
        }
        walk_expr(self, expr);
    }
}

fn check(source: &str) -> (List<Text>, TypeRegistry, List<Span>) {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .expect("valid source grammar");
    let mut closures = Closures::default();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_universe_poly_enabled(true);
    let mut errors: List<_> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| Text::from(format!("{error}")))
        .collect();
    for item in &ast.items {
        closures.visit_item(item);
        if let Err(error) = checker.check_item(item) {
            errors.push(Text::from(format!("{error}")));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| Text::from(format!("{error}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| Text::from(format!("{diagnostic:?}"))),
    );
    (errors, checker.take_type_registry(), closures.0)
}

#[test]
fn explicit_return_annotation_cannot_replace_a_direct_or_generic_contract() {
    for source in [
        "fn accept(f: fn() -> Bool) -> Bool { f() } fn bad() -> Bool { accept(|| -> Int { 1 }) }",
        "fn accept<F: fn() -> Bool>(f: F) -> Bool { f() } fn bad() -> Bool { accept(|| -> Int { 1 }) }",
        "fn accept(f: fn<T>(T) -> Bool) -> Bool { true } fn bad() -> Bool { accept(|value| -> Int { 1 }) }",
    ] {
        let (errors, _, _) = check(source);
        assert!(
            !errors.is_empty(),
            "accepted incompatible annotation: {source}"
        );
    }
}

#[test]
fn matching_annotations_remain_valid_for_both_contract_forms() {
    for source in [
        "fn accept(f: fn() -> Int) -> Int { f() } fn good() -> Int { accept(|| -> Int { 1 }) }",
        "fn accept<F: fn() -> Int>(f: F) -> Int { f() } fn good() -> Int { accept(|| -> Int { 1 }) }",
        "fn accept(f: fn<T>(T) -> Bool) -> Bool { true } fn good() -> Bool { accept(|value| -> Bool { true }) }",
    ] {
        let (errors, _, _) = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn annotated_bindings_accept_stronger_callable_guarantees() {
    for source in [
        "fn good() -> Int {let base = 10; let f: fn(Int) -> Int = |x| x + base; f(5)}",
        "fn good() -> Int {let f: fn(Int) -> Int = |x| x else {return 0;}; f(5)}",
    ] {
        let (errors, _, _) = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn annotated_bindings_preserve_parameter_and_result_contracts() {
    for source in [
        "fn bad() {let f: fn(Int) -> Bool = |x| -> Int {x};}",
        "fn bad() {let f: fn(Int) -> Bool = |x| -> Int {x} else {return;};}",
        "fn bad() {let f: fn() = || -> Int {1};}",
        "fn text(value: Bool) -> Bool {value} fn bad() {let f: fn(Int) -> Bool = text;}",
    ] {
        let (errors, _, _) = check(source);
        assert!(
            !errors.is_empty(),
            "accepted incompatible binding: {source}"
        );
    }
}

#[test]
fn annotation_still_checks_its_body() {
    let (errors, _, _) = check(
        "fn accept(f: fn() -> Bool) -> Bool { f() } fn bad() -> Bool { accept(|| -> Bool { 1 }) }",
    );
    assert!(!errors.is_empty());
}

#[test]
fn universe_annotations_cannot_narrow_the_required_result() {
    let (errors, _, _) = check(
        "fn accept(f: fn() -> Type(1)) -> Type(1) { f() } fn bad(value: Type(2)) -> Type(1) { accept(|| -> Type(2) { value }) }",
    );
    assert!(!errors.is_empty());
    let (errors, _, _) = check(
        "fn accept(f: fn() -> Type(2)) -> Type(2) { f() } fn good(value: Type(1)) -> Type(2) { accept(|| -> Type(1) { value }) }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn unit_discard_keeps_the_actual_annotated_callable_result() {
    for source in [
        "fn consume(f: fn()) { f(); } fn good() { consume(|| -> Int { 7 }); }",
        "fn keep<F: fn()>(f: F) -> F { f } fn good() -> Int { let saved = keep(|| -> Int { 7 }); saved() }",
    ] {
        let (errors, registry, closures) = check(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert_eq!(closures.len(), 1);
        let Type::Function { return_type, .. } = registry
            .lookup_expr(closures[0])
            .expect("closure has a codegen type")
        else {
            panic!("closure must remain callable")
        };
        assert_eq!(**return_type, Type::Int);
    }
}
