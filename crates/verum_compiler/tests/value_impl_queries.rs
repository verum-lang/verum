//! Actual-source obligations delimit value-indexed implementation support.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("value_impl_queries");
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
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}").into()),
    );
    errors
}

const GENERIC: &str = r#"
module value_impl_queries;
type Evidence is protocol {};
type Cell<N: meta Int> is { value: () };
implement<N: meta Int> Evidence for Cell<N> {}
fn require<T: Evidence>(cell: T) -> T { cell }
"#;

#[test]
fn owned_value_implementation_binder_accepts_concrete_and_symbolic_receivers() {
    for tail in [
        "fn probe(cell: Cell<7>) -> Cell<7> { require(cell) }",
        "fn probe<M: meta Int>(cell: Cell<M>) -> Cell<M> { require(cell) }",
    ] {
        let errors = errors(&format!("{GENERIC} {tail}"));
        assert!(
            errors.is_empty(),
            "actual generic implementation must discharge receiver: {errors:?}"
        );
    }
}

const REPEATED: &str = r#"
module value_impl_queries;
type Evidence is protocol {};
type Pair<N: meta Int, M: meta Int> is { value: () };
implement<N: meta Int> Evidence for Pair<N, N> {}
fn require<T: Evidence>(cell: T) -> T { cell }
"#;

#[test]
fn repeated_owned_value_implementation_binder_accepts_equal_indices() {
    let errors = errors(&format!(
        "{REPEATED} fn probe(cell: Pair<7,7>) -> Pair<7,7> {{ require(cell) }}"
    ));
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn repeated_owned_value_implementation_binder_rejects_distinct_indices() {
    let errors = errors(&format!(
        "{REPEATED} fn probe(cell: Pair<7,8>) -> Pair<7,8> {{ require(cell) }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("Evidence")
            && (error.contains("bound")
                || error.contains("Bound")
                || error.contains("protocol")
                || error.contains("Protocol"))),
        "must refuse actual Evidence obligation, not an unrelated error: {errors:?}"
    );
}

#[test]
fn distinct_rigid_caller_values_cannot_be_unified_to_satisfy_an_implementation() {
    let errors = errors(&format!(
        "{REPEATED} fn probe<A: meta Int, B: meta Int>(cell: Pair<A,B>) -> Pair<A,B> {{ require(cell) }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("Evidence")
            && (error.contains("bound")
                || error.contains("Bound")
                || error.contains("protocol")
                || error.contains("Protocol"))),
        "must retain caller value identities: {errors:?}"
    );
}

const BOUNDED: &str = r#"
module value_impl_queries;
type Evidence is protocol {};
type Cell<N: meta Int> is { value: () };
implement<N: meta Int{>=0}> Evidence for Cell<N> {}
fn require<T: Evidence>(cell: T) -> T { cell }
"#;

#[test]
fn owned_value_implementation_domain_accepts_constant_and_stronger_symbolic_domain() {
    for tail in [
        "fn probe(cell: Cell<7>) -> Cell<7> { require(cell) }",
        "fn probe<N: meta Int{>=7}>(cell: Cell<N>) -> Cell<N> { require(cell) }",
    ] {
        let errors = errors(&format!("{BOUNDED} {tail}"));
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn owned_value_implementation_domain_refuses_negative_and_unproved_symbolic_value() {
    for tail in [
        "fn probe(cell: Cell<-1>) -> Cell<-1> { require(cell) }",
        "fn probe<N: meta Int>(cell: Cell<N>) -> Cell<N> { require(cell) }",
    ] {
        let errors = errors(&format!("{BOUNDED} {tail}"));
        assert!(
            errors.iter().any(|error| error.contains("Evidence")
                && (error.contains("bound")
                    || error.contains("Bound")
                    || error.contains("protocol"))),
            "domain refusal must belong to the actual obligation: {errors:?}"
        );
    }
}

const ASSOCIATED: &str = r#"
module value_impl_queries;
type Evidence is protocol { type Carrier; };
type Cell<N: meta Int> is { value: () };
implement<N: meta Int> Evidence for Cell<N> { type Carrier = Cell<N>; }
type Concrete is Cell<7>;
"#;

#[test]
fn associated_result_retains_its_selected_value_owner() {
    let errors = errors(&format!(
        "{ASSOCIATED} fn probe(cell: Concrete.Carrier) -> Cell<7> {{ cell }}"
    ));
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn associated_result_cannot_change_the_selected_value_index() {
    let errors = errors(&format!(
        "{ASSOCIATED} fn probe(cell: Concrete.Carrier) -> Cell<8> {{ cell }}"
    ));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Mismatch") || error.contains("E0204")),
        "must reject the actual distinct result index: {errors:?}"
    );
}

#[test]
fn declared_value_owner_reaches_real_protocol_method_calls() {
    let declarations = r#"
        module value_impl_queries;
        type Evidence is protocol { fn index(&self) -> Int; };
        type Cell<N: meta Int> is { value: () };
        implement<N: meta Int{>=0}> Evidence for Cell<N> { fn index(&self) -> Int { N } }
    "#;
    let valid = errors(&format!(
        "{declarations} fn probe(cell:Cell<7>)->Int{{cell.index()}}"
    ));
    assert!(valid.is_empty(), "actual protocol method call: {valid:?}");
    let invalid = errors(&format!(
        "{declarations} fn probe(cell:Cell<7>)->Bool{{cell.index()}}"
    ));
    assert!(
        invalid
            .iter()
            .any(|error| error.contains("Int") && error.contains("Bool")),
        "actual selected method result mismatch: {invalid:?}"
    );
}

#[test]
fn associated_result_proves_the_implementation_value_domain() {
    let declarations = ASSOCIATED.replace("implement<N: meta Int>", "implement<N: meta Int{>=0}>");
    let valid = errors(&format!("{declarations} fn probe(cell: Concrete.Carrier) -> Cell<7> {{ cell }}"));
    assert!(valid.is_empty(), "{valid:?}");
    let invalid = errors(&format!("{declarations} fn probe(cell: Concrete.Carrier) -> Cell<8> {{ cell }}"));
    assert!(invalid.iter().any(|error| error.contains("Mismatch") || error.contains("E0204")), "{invalid:?}");
}
