//! Universal associated contracts must hold for every permitted family argument.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("families");
    let mut errors: Vec<String> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{:?}", error.to_diagnostic()))
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{:?}", error.to_diagnostic()));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| format!("{:?}", error.to_diagnostic())),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|error| format!("{error:?}")),
    );
    errors
}

fn accept(source: &str) { let errors = errors(source); assert!(errors.is_empty(), "{source}\n{errors:#?}"); }
fn reject(source: &str) { let errors = errors(source); assert!(!errors.is_empty(), "unproved contract accepted: {source}"); }
fn source(first: &str, second: &str, result: &str) -> String {
    format!(r#"module families;
        type Stream is protocol {{ type Item; }};
        type Factory is protocol {{ type Item; type Rows: Stream<Item=Self.Item>; }};
        type Rows<A,B> is {{ first:A, second:B }};
        implement<T,A:Stream<Item=T>,B:Stream<Item=T>> Stream for Rows<A,B> {{ type Item=T; }}
        type Db<A,B> is {{ first:A, second:B }};
        implement<T,A{first},B{second}> Factory for Db<A,B> {{ type Item={result}; type Rows=Rows<A,B>; }}
    "#)
}
#[test] fn associated_equalities_determine_the_hidden_implementation_parameter() {
    accept(&source(":Stream<Item=T>", ":Stream<Item=T>", "T"));
}
#[test] fn associated_equations_preserve_alpha_renaming_and_order() {
    let base = source(":Stream<Item=T>", ":Stream<Item=T>", "T");
    for declaration in ["Z,X:Stream<Item=Z>,Y:Stream<Item=Z>", "Y:Stream<Item=Z>,Z,X:Stream<Item=Z>"] {
        accept(&base.replace("implement<T,A:Stream<Item=T>,B:Stream<Item=T>> Stream for Rows<A,B> { type Item=T; }",
            &format!("implement<{declaration}> Stream for Rows<X,Y> {{ type Item=Z; }}")));
    }
}
#[test] fn missing_equality_is_not_inferred_from_the_requested_result() {
    reject(&source(":Stream", ":Stream", "T"));
}
#[test] fn another_required_protocol_premise_is_still_checked() {
    reject(&source(":Stream<Item=T>", "", "T"));
}
#[test] fn conflicting_actual_equalities_cannot_specialize_the_caller() {
    reject(&source(":Stream<Item=T>", ":Stream<Item=Int>", "T"));
}
#[test] fn associated_result_equality_is_still_checked() {
    reject(&source(":Stream<Item=T>", ":Stream<Item=T>", "Bool"));
}
#[test] fn unrelated_member_with_same_name_is_not_evidence() {
    let base = source(":Stream<Item=T>", ":Stream<Item=T>", "T");
    reject(&base.replace("type Factory", "type Foreign is protocol {type Item;}; type Factory")
        .replace("implement<T,A:Stream<Item=T>,B:Stream<Item=T>> Factory", "implement<T,A:Foreign<Item=T>,B:Foreign<Item=T>> Factory"));
}
#[test] fn inherited_equalities_preserve_the_exact_member_owner() {
    let base = source(":Derived<Item=T>", ":Derived<Item=T>", "T");
    accept(&base.replace("type Factory", "type Derived is protocol extends Stream {}; type Factory"));
}
#[test] fn generic_family_owner_arguments_are_part_of_the_evidence() {
    let base = source(":Stream<Bool,Item=T>", ":Stream<Bool,Item=T>", "T")
        .replace("type Stream is", "type Stream<X> is")
        .replace("type Rows: Stream<Item=Self.Item>", "type Rows: Stream<Bool,Item=Self.Item>")
        .replace("A:Stream<Item=T>,B:Stream<Item=T>", "A:Stream<Bool,Item=T>,B:Stream<Bool,Item=T>")
        .replace("Stream for Rows", "Stream<Bool> for Rows");
    accept(&base);
    reject(&base.replace("implement<T,A:Stream<Bool,Item=T>,B:Stream<Bool,Item=T>> Factory", "implement<T,A:Stream<Int,Item=T>,B:Stream<Int,Item=T>> Factory"));
}
#[test] fn protocol_and_callable_equations_share_one_worklist() {
    for bounds in ["T,G:fn(Int)->T,F:Stream<Item=G>", "F:Stream<Item=G>,T,G:fn(Int)->T"] {
        accept(&format!(r#"module families;
            type Stream is protocol{{type Item;}};
            type Factory is protocol{{type Item;type Rows:Stream<Item=Self.Item>;}};
            type Rows<F> is{{value:F}};
            implement<{bounds}> Stream for Rows<F>{{type Item=T;}}
            type Db<F> is{{value:F}};
            implement<{bounds}> Factory for Db<F>{{type Item=T;type Rows=Rows<F>;}}
        "#));
    }
}
#[test] fn callable_outputs_can_supply_the_associated_subject() {
    accept(r#"module families;
        type Stream is protocol{type Item;};type Factory is protocol{type Item;type Rows:Stream<Item=Self.Item>;};
        type Rows<F> is{value:F};implement<T,G:Stream<Item=T>,F:fn(Int)->G> Stream for Rows<F>{type Item=T;}
        type Db<F> is{value:F};implement<T,G:Stream<Item=T>,F:fn(Int)->G> Factory for Db<F>{type Item=T;type Rows=Rows<F>;}
    "#);
}
#[test] fn hidden_callable_contract_is_not_synthesized_from_associated_equality() {
    reject(r#"module families;
        type Stream is protocol{type Item;};type Factory is protocol{type Item;type Rows:Stream<Item=Self.Item>;};
        type Rows<F> is{value:F};implement<T,G:fn(Int)->T,F:Stream<Item=G>> Stream for Rows<F>{type Item=T;}
        type Db<F> is{value:F};implement<T,G,F:Stream<Item=G>> Factory for Db<F>{type Item=T;type Rows=Rows<F>;}
    "#);
}
#[test] fn independent_where_conditions_are_not_dropped() {
    let base = source(":Stream<Item=T>", ":Stream<Item=T>", "T").replace("type Factory", "type Marker is protocol {}; type Factory")
        .replace("Stream for Rows<A,B>", "Stream for Rows<A,B> where T:Marker");
    reject(&base);
    accept(&base.replace("implement<T,A:Stream<Item=T>,B:Stream<Item=T>> Factory", "implement<T:Marker,A:Stream<Item=T>,B:Stream<Item=T>> Factory"));
}

#[test] fn declared_abstract_member_proves_its_reflexive_equality() {
    accept(&source(":Stream", ":Stream<Item=A.Item>", "A.Item")
        .replace("implement<T,A:Stream,B:Stream<Item=A.Item>> Factory", "implement<A:Stream,B:Stream<Item=A.Item>> Factory"));
}
#[test] fn independent_abstract_members_are_not_equal() {
    reject(&source(":Stream", ":Stream", "A.Item")
        .replace("implement<T,A:Stream,B:Stream> Factory", "implement<A:Stream,B:Stream> Factory"));
}
#[test] fn associated_default_is_not_a_universal_equality_witness() {
    reject(&source(":Stream", ":Stream", "Int")
        .replace("type Stream is protocol { type Item; }", "type Stream is protocol { type Item=Int; }"));
}
#[test] fn distinct_generic_applications_do_not_conflict_or_supply_each_other() {
    let base = source(":Stream<Bool,Item=T>+Stream<Int,Item=Int>", ":Stream<Bool,Item=T>+Stream<Int,Item=Int>", "T")
        .replace("type Stream is", "type Stream<X> is")
        .replace("type Rows: Stream<Item=Self.Item>", "type Rows: Stream<Bool,Item=Self.Item>")
        .replace("A:Stream<Item=T>,B:Stream<Item=T>", "A:Stream<Bool,Item=T>,B:Stream<Bool,Item=T>")
        .replace("Stream for Rows", "Stream<Bool> for Rows");
    accept(&base);
    reject(&base.replace(":Stream<Bool,Item=T>+Stream<Int,Item=Int>", ":Stream<Int,Item=Int>"));
}
#[test] fn conflicting_parent_equality_is_not_overwritten() {
    let base = source(":Derived<Item=Bool>", ":Derived<Item=Bool>", "Bool")
        .replace("type Factory", "type Derived is protocol extends Stream<Item=Int> {}; type Factory");
    reject(&base);
}
