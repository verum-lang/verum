//! T2851: conditional methods use the caller's declared lexical premises.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("conditional");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .iter()
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
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|error| error.is_error())
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

fn accepts(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{source}\n{errors:#?}");
}

fn rejects_method(source: &str) {
    let errors = errors(source);
    assert!(
        !errors.is_empty(),
        "unproved conditional method admitted: {source}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("MethodNotFound") || error.contains("E405")),
        "expected an unavailable method or unsatisfied call obligation: {errors:#?}"
    );
}

const DECLARATIONS: &str = r#"
    module conditional;
    type Allowed is protocol {};
    type Other is protocol {};
    type Readout is protocol { fn measure(&self) -> Int; };
    type Packet<T> is { value: T };
    implement<T: Allowed> Allowed for Packet<T> {}
    implement<T: Allowed> Readout for Packet<T> {
        fn measure(&self) -> Int { 17 }
    }
"#;

#[test]
fn nested_conditional_method_uses_its_function_premise() {
    accepts(&format!(
        "{DECLARATIONS}
        fn probe<T: Allowed>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
    accepts(&format!(
        "{DECLARATIONS}
        fn probe<T>(value: &Packet<Packet<T>>) -> Int where type T: Allowed {{ value.measure() }}"
    ));
}

#[test]
fn nested_conditional_method_uses_its_implementation_premise() {
    accepts(&format!(
        "{DECLARATIONS}
        type Holder<T> is {{ packet: Packet<Packet<T>> }};
        implement<T: Allowed> Holder<T> {{
            fn probe(&self) -> Int {{ self.packet.measure() }}
        }}"
    ));
}

#[test]
fn an_inherited_default_retains_the_implementation_conditions() {
    let declarations = r#"
        module conditional;
        type Allowed is protocol {};
        type Readout is protocol { fn measure(&self) -> Int { 17 } };
        type Packet<T> is { value: T };
        implement<T: Allowed> Allowed for Packet<T> {}
        implement<T: Allowed> Readout for Packet<T> {}
    "#;
    accepts(&format!(
        "{declarations}
        fn probe<T: Allowed>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
    rejects_method(&format!(
        "{declarations}
        fn probe<T>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
}

#[test]
fn missing_and_unrelated_premises_cannot_supply_a_method() {
    for parameters in ["T", "T: Other", "T, U: Allowed"] {
        rejects_method(&format!(
            "{DECLARATIONS}
            fn probe<{parameters}>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
        ));
    }
}

#[test]
fn a_prior_generic_body_cannot_supply_the_next_bodys_premise() {
    let bounded = "fn bounded<T: Allowed>(value: &Packet<T>) -> Int { value.measure() }";
    let unbounded = "fn unbounded<T>(value: &Packet<T>) -> Int { value.measure() }";
    for (first, second) in [(bounded, unbounded), (unbounded, bounded)] {
        rejects_method(&format!("{DECLARATIONS} {first} {second}"));
    }
}

#[test]
fn all_nested_conditions_must_be_proved() {
    let declarations = DECLARATIONS.replace(
        "implement<T: Allowed> Readout",
        "implement<T: Allowed + Other> Readout",
    );
    let declarations = format!("{declarations} implement<T: Other> Other for Packet<T> {{}}");
    accepts(&format!(
        "{declarations}
        fn probe<T: Allowed + Other>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
    rejects_method(&format!(
        "{declarations}
        fn probe<T: Allowed>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
}

#[test]
fn associated_equalities_are_checked_in_the_same_environment() {
    let declarations = r#"
        module conditional;
        type Source is protocol { type Item; };
        type Readout is protocol { fn measure(&self) -> Int; };
        type Packet<T> is { value: T };
        implement<T: Source<Item = Int>> Readout for Packet<T> {
            fn measure(&self) -> Int { 17 }
        }
    "#;
    accepts(&format!(
        "{declarations}
        fn probe<T: Source<Item = Int>>(value: &Packet<T>) -> Int {{ value.measure() }}"
    ));
    rejects_method(&format!(
        "{declarations}
        fn probe<T: Source<Item = Bool>>(value: &Packet<T>) -> Int {{ value.measure() }}"
    ));
}

#[test]
fn selected_method_keeps_its_declared_result() {
    let errors = errors(&format!(
        "{DECLARATIONS}
        fn probe<T: Allowed>(value: &Packet<Packet<T>>) -> Bool {{ value.measure() }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")),
        "{errors:#?}"
    );
    assert!(
        !errors.iter().any(|error| error.contains("MethodNotFound")),
        "{errors:#?}"
    );
}

fn overloaded_declarations() -> Text {
    format!(
        "{DECLARATIONS}
        type OtherReadout is protocol {{ fn measure(&self) -> Bool; }};
        implement<T: Other> Other for Packet<T> {{}}
        implement<T: Other> OtherReadout for Packet<T> {{
            fn measure(&self) -> Bool {{ false }}
        }}"
    )
    .into()
}

#[test]
fn protocol_candidate_search_uses_the_actual_nested_premise() {
    // Same-name protocol methods cannot use the single inherent-method slot.
    // Selection must prove the candidate's conditions in the caller's frame.
    let declarations = overloaded_declarations();
    accepts(&format!(
        "{declarations}
        fn probe<T: Allowed>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
    accepts(&format!(
        "{declarations}
        fn probe<T: Other>(value: &Packet<Packet<T>>) -> Bool {{ value.measure() }}"
    ));
    rejects_method(&format!(
        "{declarations}
        fn probe<T>(value: &Packet<Packet<T>>) -> Int {{ value.measure() }}"
    ));
}

#[test]
fn a_declared_clone_does_not_grant_structural_raw_pointer_copy() {
    let declarations = overloaded_declarations()
        .replace("measure", "clone")
        .replace("value: T", "value: &unsafe T");
    accepts(&format!(
        "{declarations}
        fn probe<T: Allowed>(value: &Packet<Packet<T>>) -> Int {{ value.clone() }}"
    ));
    let errors = errors(&format!(
        "{declarations}
        fn probe<T>(value: &Packet<Packet<T>>) -> Int {{ value.clone() }}"
    ));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("No implicit clone")
                && error.contains("requires a declared clone")),
        "{errors:#?}"
    );
}
