//! T1530: explicit method arguments must reach the source type checker.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("parse source");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("register type");
        }
    }
    for item in &module.items {
        if let ItemKind::Impl(decl) = &item.kind {
            checker.register_impl_block(decl).expect("register impl");
        }
    }
    for item in &module.items {
        if let ItemKind::Function(decl) = &item.kind {
            checker
                .register_function_signature(decl)
                .expect("register function");
        }
    }
    let mut errors: List<_> = module
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

fn accepted(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}

const FACTORY: &str = r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Output>(self) -> Output { Output.from_value(7) }
}
"#;

#[test]
fn explicit_method_result_needs_no_let_annotation() {
    accepted(&format!(
        "{FACTORY}\n{}",
        r#"
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.build<Answer>();
    result.value
}
"#
    ));
}

#[test]
fn explicit_method_result_supports_direct_field_chain() {
    accepted(&format!(
        "{FACTORY}\n{}",
        r#"
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.build<Answer>().value
}
"#
    ));
}

#[test]
fn method_generic_shadow_is_distinct_from_receiver_parameter() {
    accepted(
        r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Item>(self) -> Item { Item.from_value(7) }
}
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.build<Answer>();
    result.value
}
"#,
    );
}

#[test]
fn explicit_method_argument_rejects_an_unrelated_value_type() {
    let errors = errors(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<Output>(self, value: Output) -> Output { value }
}
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.choose<Int>(true)
}
"#,
    );
    assert!(!errors.is_empty(), "explicit Int must reject Bool");
    assert!(errors.iter().any(|e| e.contains("Mismatch")), "{errors:?}");
}

#[test]
fn successive_explicit_instantiations_do_not_share_fresh_variables() {
    accepted(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<Output>(self, value: Output) -> Output { value }
}
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    let first = factory.choose<Int>(1);
    let second = factory.choose<Bool>(true);
    second
}
"#,
    );
}

#[test]
fn unused_declared_method_parameter_still_owns_its_explicit_slot() {
    accepted(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<Unused, Output>(self, value: Output) -> Output { value }
}
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.choose<Int, Bool>(true);
    result
}
"#,
    );
}

#[test]
fn excess_explicit_method_arguments_are_rejected_at_the_call() {
    let errors = errors(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<Output>(self, value: Output) -> Output { value }
}
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.choose<Int, Bool>(1)
}
"#,
    );
    assert!(errors.iter().any(|e| e.contains("E408")), "{errors:?}");
}

#[test]
fn implicit_method_parameter_does_not_consume_an_explicit_argument() {
    accepted(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<{Inferred}, Output>(self, left: Inferred, right: Output) -> Output { right }
}
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.choose<Bool>(1, true);
    result
}
"#,
    );
}

#[test]
fn explicit_return_argument_cannot_be_overridden_by_a_let_annotation() {
    let errors = errors(&format!(
        "{FACTORY}\n{}",
        r#"
fn probe() -> Text {
    let factory: Factory<Int> = Factory { value: 7 };
    let result: Text = factory.build<Answer>();
    result
}
"#
    ));
    assert!(errors.iter().any(|e| e.contains("Mismatch")), "{errors:?}");
}

#[test]
fn direct_field_chain_uses_the_explicit_results_field_type() {
    let errors = errors(&format!(
        "{FACTORY}\n{}",
        r#"
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.build<Answer>().value
}
"#
    ));
    assert!(errors.iter().any(|e| e.contains("Mismatch")), "{errors:?}");
}

#[test]
fn shadowed_method_does_not_replace_the_next_methods_owner_parameter() {
    accepted(
        r#"
type Answer is { value: Int };
implement Answer { fn from_value(value: Int) -> Answer { Answer { value } } }
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Item>(self) -> Item { Item.from_value(7) }
    fn original(self) -> Item { self.value }
}
fn probe() -> Int {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.build<Answer>();
    let original = factory.original();
    original + result.value
}
"#,
    );
}

#[test]
fn omitted_explicit_arguments_still_infer_from_method_values() {
    accepted(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<Output>(self, value: Output) -> Output { value }
}
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    let result = factory.choose(true);
    result
}
"#,
    );
}

#[test]
fn value_argument_does_not_satisfy_a_declared_type_parameter() {
    let errors = errors(
        r#"
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn choose<Output>(self, value: Output) -> Output { value }
}
fn probe() -> Bool {
    let factory: Factory<Int> = Factory { value: 7 };
    factory.choose<9>(true)
}
"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("requires a type argument")),
        "{errors:?}"
    );
}
