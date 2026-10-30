//! Associated equalities belong to generic declarations and freshen together.
use verum_ast::{FileId, ItemKind};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

const DECLARATIONS: &str = r#"
type Channel is protocol { type Item; fn get(&self) -> Self.Item; };
type Ready<T> is { value: T };
implement<T> Channel for Ready<T> {
    type Item = T;
    fn get(&self) -> T { self.value }
}
"#;

fn check(declarations: &str, probe: &str, accepted: bool) {
    check_with_protocol(DECLARATIONS, declarations, probe, accepted);
}

fn check_with_protocol(protocol: &str, declarations: &str, probe: &str, accepted: bool) {
    let source = format!("{protocol}\n{declarations}\n{probe}");
    let ast = FastParser::new()
        .parse_module_str(&source, FileId::new(0))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
            ItemKind::Impl(decl) => checker.register_impl_block(decl).unwrap(),
            ItemKind::Function(decl) => checker.register_function_signature(decl).unwrap(),
            _ => (),
        }
    }
    for item in &ast.items {
        if matches!(&item.kind, ItemKind::Function(decl) if decl.name.name.as_str() == "probe") {
            continue;
        }
        checker
            .check_item(item)
            .expect("generic declaration must independently typecheck");
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostic_sources().is_empty());
    let probe = ast
        .items
        .iter()
        .find(|item| {
            matches!(&item.kind,
        ItemKind::Function(decl) if decl.name.name.as_str() == "probe")
        })
        .unwrap();
    let result = checker.check_item(probe);
    let deferred = checker.take_deferred_errors();
    let diagnostics = checker.diagnostic_sources();
    assert_eq!(
        result.is_ok() && deferred.is_empty() && diagnostics.is_empty(),
        accepted,
        "{result:?}; {deferred:?}; {diagnostics:?}"
    );
}

#[test]
fn parameterized_protocol_forwarding_requires_the_same_arguments() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    let exact = "fn exact<F: Channel<Int, Item = Int>>(value: F) -> Int { value.get() }";
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<Int, Item = Int>>(value: F) -> Int { exact(value) }",
        true,
    );
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<Text, Item = Int>>(value: F) -> Int { exact(value) }",
        false,
    );
}

#[test]
fn universal_protocol_argument_cannot_be_specialized_by_forwarding() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    let exact = "fn exact<F: Channel<Int, Item = Int>>(value: F) -> Int { value.get() }";
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<T, Item = Int>, T>(value: F) -> Int { exact(value) }",
        false,
    );
}

#[test]
fn concrete_protocol_argument_is_part_of_associated_equality_evidence() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    check_with_protocol(
        &protocol,
        "fn exact<F: Channel<Int, Item = Int>>(value: F) -> Int { value.get() }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        true,
    );
    check_with_protocol(
        &protocol,
        "fn exact<F: Channel<Text, Item = Int>>(value: F) -> Int { value.get() }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        false,
    );
}

const EXACT: &str = "fn exact<T, F: Channel<Item = T>>(value: F) -> T { value.get() }";

#[test]
fn independent_calls_freshen_the_associated_equality_with_the_scheme() {
    check(
        EXACT,
        r#"fn probe() -> Int {
        let first: Int = exact(Ready { value: 7 });
        let second: Text = exact(Ready { value: "independent" });
        first
    }"#,
        true,
    );
}

#[test]
fn explicit_generic_arguments_enforce_the_associated_equality() {
    check(
        EXACT,
        "fn probe() -> Int { exact<Int, Ready<Int>>(Ready { value: 7 }) }",
        true,
    );
    check(
        EXACT,
        r#"fn probe() -> Int { exact<Int, Ready<Text>>(Ready { value: "wrong" }) }"#,
        false,
    );
}

#[test]
fn generic_forwarding_preserves_associated_equality_evidence() {
    let declarations =
        format!("{EXACT}\nfn relay<T, F: Channel<Item = T>>(value: F) -> T {{ exact(value) }}");
    check(
        &declarations,
        "fn probe() -> Int { relay(Ready { value: 7 }) }",
        true,
    );
    check(
        &declarations,
        r#"fn probe() -> Int { relay(Ready { value: "wrong" }) }"#,
        false,
    );
}

#[test]
fn generic_forwarding_cannot_borrow_a_different_parameters_bound() {
    check(
        EXACT,
        "fn probe<T, F>(value: F) -> T { exact(value) }",
        false,
    );
    check(
        EXACT,
        "fn probe<T, F, G: Channel<Item = T>>(value: F, other: G) -> T { exact(value) }",
        false,
    );
}

#[test]
fn forward_declared_binder_is_the_same_type_variable_in_an_equality() {
    let declarations = "fn exact<F: Channel<Item = T>, T>(value: F) -> T { value.get() }";
    check(
        declarations,
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        true,
    );
    check(
        declarations,
        r#"fn probe() -> Int { exact(Ready { value: "wrong" }) }"#,
        false,
    );
}

#[test]
fn protocol_arguments_are_checked_without_an_associated_equality() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    let exact = "fn exact<F: Channel<Text>>(value: F) -> Int { 7 }";
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<Text>>(value: F) -> Int { exact(value) }",
        true,
    );
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<Int>>(value: F) -> Int { exact(value) }",
        false,
    );
    check_with_protocol(
        &protocol,
        exact,
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        false,
    );
}

#[test]
fn fresh_callee_arguments_can_match_universal_caller_evidence() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    check_with_protocol(
        &protocol,
        "fn exact<P, F: Channel<P, Item = Int>>(value: F) -> Int { value.get() }",
        "fn probe<T, F: Channel<T, Item = Int>>(value: F) -> Int { exact(value) }",
        true,
    );
}

#[test]
fn receiver_and_protocol_arguments_share_implementation_substitution() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<T> for Ready");
    check_with_protocol(
        &protocol,
        "fn exact<F: Channel<Int, Item = Int>>(value: F) -> Int { value.get() }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        true,
    );
    check_with_protocol(
        &protocol,
        "fn exact<F: Channel<Text, Item = Int>>(value: F) -> Int { value.get() }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        false,
    );
}

#[test]
fn associated_equality_comes_from_the_selected_protocol_instantiation() {
    let protocol = r#"
        type Channel<P> is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel<Int> for Ready { type Item = Int; }
        implement Channel<Text> for Ready { type Item = Text; }
    "#;
    check_with_protocol(
        protocol,
        "fn exact<F: Channel<Text, Item = Text>>(value: F) -> Int { 7 }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        true,
    );
    check_with_protocol(
        protocol,
        "fn exact<F: Channel<Text, Item = Int>>(value: F) -> Int { 7 }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        false,
    );
}

#[test]
fn conditional_implementations_require_associated_equality_evidence() {
    for condition in [
        "implement<T> Checked for Container<T> where T: Channel<Item = Int> {}",
        "implement<T: Channel<Item = Int>> Checked for Container<T> {}",
    ] {
        let protocol = format!(
            r#"
            type Channel is protocol {{ type Item; }};
            type Ready is {{ value: Int }};
            implement Channel for Ready {{ type Item = Int; }}
            type Checked is protocol {{}};
            type Container<T> is {{ value: T }};
            {condition}
        "#
        );
        let exact = "fn exact<F: Checked>(value: F) -> Int { 7 }";
        let probe = "fn probe() -> Int { exact(Container { value: Ready { value: 7 } }) }";
        check_with_protocol(&protocol, exact, probe, true);
        check_with_protocol(
            &protocol.replace("type Item = Int;", "type Item = Text;"),
            exact,
            probe,
            false,
        );
    }
}

#[test]
fn blanket_implementations_require_associated_equality_evidence() {
    let protocol = r#"
        type Channel is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel for Ready { type Item = Int; }
        type Checked is protocol {};
        implement<T: Channel<Item = Int>> Checked for T {}
    "#;
    let exact = "fn exact<F: Checked>(value: F) -> Int { 7 }";
    let probe = "fn probe() -> Int { exact(Ready { value: 7 }) }";
    check_with_protocol(protocol, exact, probe, true);
    check_with_protocol(
        &protocol.replace("type Item = Int;", "type Item = Text;"),
        exact,
        probe,
        false,
    );
}

#[test]
fn conditional_equalities_substitute_the_implementation_binders() {
    let protocol = r#"
        type Channel is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel for Ready { type Item = Int; }
        type Checked is protocol {};
        type Container<T, U> is { value: T, expected: U };
        implement<T, U> Checked for Container<T, U> where T: Channel<Item = U> {}
    "#;
    let exact = "fn exact<F: Checked>(value: F) -> Int { 7 }";
    check_with_protocol(
        protocol,
        exact,
        "fn probe() -> Int { exact(Container { value: Ready { value: 7 }, expected: 9 }) }",
        true,
    );
    check_with_protocol(
        protocol,
        exact,
        r#"fn probe() -> Int { exact(Container { value: Ready { value: 7 }, expected: "wrong" }) }"#,
        false,
    );
}

#[test]
fn indexed_implementations_still_have_to_discharge_their_conditions() {
    let protocol = r#"
        type Channel is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel for Ready { type Item = Int; }
        type Checked is protocol {};
        implement Checked for Ready where Ready: Channel<Item = Int> {}
    "#;
    let exact = "fn exact<F: Checked>(value: F) -> Int { 7 }";
    let probe = "fn probe() -> Int { exact(Ready { value: 7 }) }";
    check_with_protocol(protocol, exact, probe, true);
    check_with_protocol(
        &protocol.replace("type Item = Int;", "type Item = Text;"),
        exact,
        probe,
        false,
    );
}

#[test]
fn a_conditional_equality_cannot_justify_itself() {
    check_with_protocol(
        r#"
        type Channel is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel for Ready where Ready: Channel<Item = Int> { type Item = Int; }
    "#,
        "fn exact<F: Channel<Item = Int>>(value: F) -> Int { 7 }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        false,
    );
}

#[test]
fn nested_conditional_evidence_cannot_bypass_an_indexed_condition() {
    let protocol = r#"
        type Channel is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel for Ready { type Item = Int; }
        type Checked is protocol {};
        implement Checked for Ready where Ready: Channel<Item = Int> {}
        type Approved is protocol {};
        type Container<T> is { value: T };
        implement<T: Checked> Approved for Container<T> {}
    "#;
    let exact = "fn exact<F: Approved>(value: F) -> Int { 7 }";
    let probe = "fn probe() -> Int { exact(Container { value: Ready { value: 7 } }) }";
    check_with_protocol(protocol, exact, probe, true);
    check_with_protocol(
        &protocol.replace("type Item = Int;", "type Item = Text;"),
        exact,
        probe,
        false,
    );
}

#[test]
fn assignment_coercions_do_not_establish_protocol_argument_identity() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    let exact = "fn exact<F: Channel<Text, Item = Int>>(value: F) -> Int { value.get() }";
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<Text, Item = Int>>(value: F) -> Int { exact(value) }",
        true,
    );
    check_with_protocol(
        &protocol,
        exact,
        "fn probe<F: Channel<Char, Item = Int>>(value: F) -> Int { exact(value) }",
        false,
    );
}

#[test]
fn assignment_coercions_do_not_establish_associated_equality() {
    let exact = "fn exact<F: Channel<Item = Text>>(value: F) -> Text { value.get() }";
    check(
        exact,
        "fn probe<F: Channel<Item = Text>>(value: F) -> Text { exact(value) }",
        true,
    );
    check(
        exact,
        "fn probe<F: Channel<Item = Char>>(value: F) -> Text { exact(value) }",
        false,
    );
}

#[test]
fn source_aliases_keep_the_same_protocol_evidence_identity() {
    check(
        "type Scalar is Int; fn exact<F: Channel<Item = Scalar>>(value: F) -> Scalar { value.get() }",
        "fn probe<F: Channel<Item = Int>>(value: F) -> Int { exact(value) }",
        true,
    );
}

#[test]
fn callable_bound_does_not_specialize_a_universal_protocol_argument() {
    let protocol = DECLARATIONS
        .replace("type Channel is", "type Channel<P> is")
        .replace("Channel for Ready", "Channel<Int> for Ready");
    check_with_protocol(
        &protocol,
        "fn exact<F: Channel<fn(Int) -> Int, Item = Int>>(value: F) -> Int { value.get() }",
        "fn probe<T: fn(Int) -> Int, F: Channel<T, Item = Int>>(value: F) -> Int { exact(value) }",
        false,
    );
}

#[test]
fn nested_aliases_preserve_evidence_without_erasing_nominal_owners() {
    let protocol = r#"
        type Scalar is Int;
        type Number is Scalar;
        type Boxed<T> is { value: T };
        type Other<T> is { value: T };
        type Channel<P> is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel<Boxed<Number>> for Ready { type Item = (Scalar, Int); }
    "#;
    let exact = "fn exact<F: Channel<Boxed<Int>, Item = (Int, Int)>>(value: F) -> Int { 7 }";
    check_with_protocol(
        protocol,
        exact,
        "fn probe<F: Channel<Boxed<Number>, Item = (Scalar, Number)>>(value: F) -> Int { exact(value) }",
        true,
    );
    check_with_protocol(
        protocol,
        exact,
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        true,
    );
    check_with_protocol(
        protocol,
        exact,
        "fn probe<F: Channel<Other<Int>, Item = (Int, Int)>>(value: F) -> Int { exact(value) }",
        false,
    );
}

#[test]
fn callable_associated_equality_normalizes_only_transparent_aliases() {
    check_with_protocol(
        r#"
        type Scalar is Int;
        type Channel is protocol { type Item; };
        type Ready is { value: Int };
        implement Channel for Ready { type Item = fn(Scalar) -> Scalar; }
    "#,
        "fn exact<F: Channel<Item = fn(Int) -> Int>>(value: F) -> Int { 7 }",
        "fn probe() -> Int { exact(Ready { value: 7 }) }",
        true,
    );
}
