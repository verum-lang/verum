//! Protocol evidence belongs to the exact receiver, not its value coercions.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("valid Verum grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("receivers");
    let mut errors: List<Text> = checker.prepare_module_declarations(&module).iter()
        .map(|error| format!("{:?}", error.to_diagnostic()).into()).collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{:?}", error.to_diagnostic()).into());
        }
    }
    errors.extend(checker.take_deferred_errors().iter()
        .map(|error| Text::from(format!("{:?}", error.to_diagnostic()))));
    errors.extend(checker.diagnostics().iter().filter(|error| error.is_error())
        .map(|error| Text::from(format!("{error:?}"))));
    errors
}

fn source(implementations: &str, result: &str) -> Text {
    format!(r#"module receivers;
        type Observe is protocol {{ type Output; }};
        type Positive is Int {{ self > 0 }};
        type Packet is {{ payload: Int }};
        {implementations}
        type Factory is protocol {{ type Value: Observe<Output=Bool>; }};
        type Owner is ();
        implement Factory for Owner {{ type Value = {result}; }}
    "#).into()
}

fn accept(source: &str) {
    let found = errors(source);
    assert!(found.is_empty(), "{source}\n{found:#?}");
}

fn reject(source: &str) {
    let found = errors(source);
    assert!(found.iter().any(|error| error.contains("E405")),
        "expected a failed protocol obligation: {source}\n{found:#?}");
}

#[test]
fn refined_implementation_does_not_obstruct_unrelated_receiver_evidence() {
    for receiver in ["Bool", "Text", "Packet", "Int", "Positive"] {
        let mut implementations = Text::from("implement Observe for Positive {type Output=Bool;}");
        if receiver != "Positive" {
            implementations.push_str(&format!("implement Observe for {receiver} {{type Output=Bool;}}"));
        }
        accept(&source(&implementations, receiver));
    }
}

#[test]
fn base_and_refined_receivers_keep_independent_associated_outputs() {
    accept(&source(
        "implement Observe for Int {type Output=Int;} implement Observe for Positive {type Output=Bool;}",
        "Positive",
    ));
    accept(&source(
        "implement Observe for Int {type Output=Bool;} implement Observe for Positive {type Output=Int;}",
        "Int",
    ));
}

#[test]
fn refinement_is_not_evidence_for_its_carrier() {
    reject(&source("implement Observe for Positive {type Output=Bool;}", "Int"));
}

#[test]
fn carrier_is_not_evidence_for_its_refinement() {
    reject(&source("implement Observe for Int {type Output=Bool;}", "Positive"));
}

#[test]
fn wrong_associated_output_is_not_hidden_by_a_refinement() {
    reject(&source("implement Observe for Positive {type Output=Int;}", "Positive"));
}

#[test]
fn uninstantiated_receiver_cannot_borrow_a_concrete_refined_witness() {
    reject(&source("implement Observe for Positive {type Output=Bool;}", "T")
        .replace("type Owner is ();", "type Owner<T> is { value: T };")
        .replace("implement Factory for Owner", "implement<T> Factory for Owner<T>"));
}
