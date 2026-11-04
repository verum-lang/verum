//! Declaration-owned value binders instantiate at calls without changing identity.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(tail: &str) -> List<Text> {
    let source = format!("module value_calls; type Cell<N: meta Int> is {{ value: () }}; {tail}");
    let module = Parser::new(&source).parse_module().expect("source parses");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("value_calls");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| Text::from(format!("{error:?}")))
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
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|error| error.is_error())
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

fn accept(source: &str) {
    let errors = errors(source);
    assert!(
        errors.is_empty(),
        "valid declared value application failed: {errors:?}"
    );
}

fn reject_value_mismatch(source: &str) {
    let errors = errors(source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Mismatch") || error.contains("exact value index")),
        "must reject a represented value mismatch, not an unrelated unsupported feature: {errors:?}"
    );
}

const ID: &str = "fn id<N: meta Int>(cell: Cell<N>) -> Cell<N> { cell }";

#[test]
fn explicit_value_argument_instantiates_a_generic_function() {
    accept(&format!(
        "{ID} fn consume(cell: Cell<1>) -> Cell<1> {{ id<1>(cell) }}"
    ));
}

#[test]
fn inferred_value_argument_instantiates_a_generic_function() {
    accept(&format!(
        "{ID} fn consume(cell: Cell<1>) -> Cell<1> {{ id(cell) }}"
    ));
}

#[test]
fn each_call_has_fresh_value_inference() {
    accept(&format!(
        "{ID} fn consume(one: Cell<1>, two: Cell<2>) -> Cell<2> {{ let first = id(one); id(two) }}"
    ));
}

#[test]
fn explicit_argument_and_actual_index_must_agree() {
    reject_value_mismatch(&format!(
        "{ID} fn consume(cell: Cell<1>) -> Cell<2> {{ id<2>(cell) }}"
    ));
}

#[test]
fn one_quantified_index_cannot_accept_two_distinct_arguments() {
    reject_value_mismatch(
        "fn pair<N: meta Int>(one: Cell<N>, two: Cell<N>) -> Cell<N> { one } fn consume(one: Cell<1>, two: Cell<2>) -> Cell<1> { pair(one, two) }",
    );
}

#[test]
fn caller_owned_value_remains_rigid_when_forwarded() {
    accept(&format!(
        "{ID} fn consume<M: meta Int>(cell: Cell<M>) -> Cell<M> {{ id(cell) }}"
    ));
}

#[test]
fn forwarded_value_cannot_be_returned_with_another_caller_index() {
    reject_value_mismatch(&format!(
        "{ID} fn consume<N: meta Int, M: meta Int>(cell: Cell<N>) -> Cell<M> {{ id(cell) }}"
    ));
}

#[test]
fn mixed_type_value_type_function_slots_preserve_order() {
    accept(
        "fn keep<A, N: meta Int, B>(a: A, cell: Cell<N>, b: B) -> Cell<N> { cell } fn consume(cell: Cell<3>) -> Cell<3> { keep<Int, 3, Bool>(7, cell, true) }",
    );
}

#[test]
fn mixed_type_value_type_function_slots_preserve_value_mismatches() {
    reject_value_mismatch(
        "fn keep<A, N: meta Int, B>(a: A, cell: Cell<N>, b: B) -> Cell<N> { cell } fn consume(cell: Cell<3>) -> Cell<4> { keep<Int, 4, Bool>(7, cell, true) }",
    );
}

#[test]
fn receiver_owned_value_instantiates_from_the_actual_receiver() {
    accept(
        "type Holder<N: meta Int> is { value: () }; implement<N: meta Int> Holder<N> { fn same(&self, cell: Cell<N>) -> Cell<N> { cell } } fn consume(holder: Holder<3>, cell: Cell<3>) -> Cell<3> { holder.same(cell) }",
    );
}

#[test]
fn receiver_owned_value_rejects_a_different_argument_index() {
    reject_value_mismatch(
        "type Holder<N: meta Int> is { value: () }; implement<N: meta Int> Holder<N> { fn same(&self, cell: Cell<N>) -> Cell<N> { cell } } fn consume(holder: Holder<3>, cell: Cell<4>) -> Cell<3> { holder.same(cell) }",
    );
}

#[test]
fn method_value_parameter_is_separate_from_the_receiver_parameter() {
    accept(
        "type Holder<N: meta Int> is { value: () }; implement<N: meta Int> Holder<N> { fn same<M: meta Int>(&self, owner: Cell<N>, local: Cell<M>) -> Cell<M> { local } } fn consume(holder: Holder<3>, owner: Cell<3>, local: Cell<5>) -> Cell<5> { holder.same<5>(owner, local) }",
    );
}

#[test]
fn same_spelled_method_value_binder_does_not_capture_the_owner() {
    accept(
        "type Holder<N: meta Int> is { value: () }; implement<N: meta Int> Holder<N> { fn same<N: meta Int>(&self, cell: Cell<N>) -> Cell<N> { cell } } fn consume(holder: Holder<3>, cell: Cell<5>) -> Cell<5> { holder.same<5>(cell) }",
    );
}

#[test]
fn mixed_receiver_arguments_are_not_zipped_to_type_variables_alone() {
    accept(
        "type Holder<A, N: meta Int, B> is { first: A, last: B }; implement<A, N: meta Int, B> Holder<A, N, B> { fn same(&self, cell: Cell<N>, last: B) -> Cell<N> { cell } } fn consume(holder: Holder<Int, 3, Bool>, cell: Cell<3>) -> Cell<3> { holder.same(cell, true) }",
    );
}

const REFINED: &str = "fn positive<N: meta Int{>= 0}>(cell: Cell<N>) -> Cell<N> { cell }";

#[test]
fn explicit_value_respects_the_parameter_domain() {
    accept(&format!(
        "{REFINED} fn consume(cell: Cell<3>) -> Cell<3> {{ positive<3>(cell) }}"
    ));
}

#[test]
fn inferred_value_respects_the_parameter_domain() {
    accept(&format!(
        "{REFINED} fn consume(cell: Cell<3>) -> Cell<3> {{ positive(cell) }}"
    ));
}

#[test]
fn inferred_negative_value_does_not_escape_a_refined_domain() {
    let errors = errors(&format!(
        "{REFINED} fn consume(cell: Cell<-1>) -> Cell<-1> {{ positive(cell) }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("domain")
            || error.contains("MetaRefinementViolation")
            || error.contains("Mismatch")),
        "missing value domain refusal: {errors:?}"
    );
}

#[test]
fn explicit_negative_value_does_not_escape_a_refined_domain() {
    let errors = errors(&format!(
        "{REFINED} fn consume(cell: Cell<-1>) -> Cell<-1> {{ positive<-1>(cell) }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("domain")
            || error.contains("MetaRefinementViolation")
            || error.contains("Mismatch")),
        "missing value domain refusal: {errors:?}"
    );
}

#[test]
fn explicit_value_of_the_wrong_sort_is_rejected() {
    let errors = errors(&format!(
        "{ID} fn consume(cell: Cell<1>) -> Cell<1> {{ id<true>(cell) }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")
            || error.contains("wrong kind")
            || error.contains("sort")),
        "missing sort refusal: {errors:?}"
    );
}

#[test]
fn weaker_caller_domain_cannot_instantiate_a_stronger_callee() {
    let errors = errors(&format!(
        "{REFINED} fn consume<M: meta Int>(cell: Cell<M>) -> Cell<M> {{ positive(cell) }}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("domain")
            || error.contains("MetaRefinementViolation")
            || error.contains("Mismatch")),
        "missing symbolic domain refusal: {errors:?}"
    );
}

#[test]
fn stronger_caller_domain_instantiates_a_weaker_callee() {
    accept(&format!(
        "{REFINED} fn consume<M: meta Int{{>= 7}}>(cell: Cell<M>) -> Cell<M> {{ positive(cell) }}"
    ));
}

#[test]
fn floating_value_index_preserves_signed_zero_identity() {
    let errors = errors("type Sample<N: meta Float> is { value: () }; fn wrong(x: Sample<-0.0>) -> Sample<0.0> { x }");
    assert!(!errors.is_empty(), "distinct IEEE representations collapsed: {errors:?}");
}

#[test]
fn identical_floating_value_index_is_preserved() {
    accept("type Sample<N: meta Float> is { value: () }; fn same(x: Sample<0.0>) -> Sample<0.0> { x }");
}

#[test]
fn nested_function_value_binder_is_instantiated_in_the_outer_scope() {
    accept("fn outer<N: meta Int>(cell: Cell<N>) -> Cell<N> { fn inner<N: meta Int>(value: Cell<N>) -> Cell<N> { value } inner(cell) }");
}

#[test]
fn nested_same_spelled_binder_cannot_return_an_outer_value_as_its_own() {
    reject_value_mismatch("fn outer<N: meta Int>(cell: Cell<N>) -> Cell<N> { fn inner<N: meta Int>(value: Cell<N>) -> Cell<N> { cell } inner(cell) }");
}

// Declaration collection must refuse an invalid signature before checking bodies.
fn signature_errors(methods: &str) -> List<Text> {
    let source = format!("module value_calls; type Checked<N: meta Int{{>=0}}> is {{value: ()}}; type Owner is {{value: ()}}; implement Owner {{ {methods} }}");
    let module = Parser::new(&source).parse_module().expect("method signature grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("value_calls");
    let mut errors: List<Text> = checker.prepare_module_declarations(&module).iter()
        .map(|e| format!("{e:?}").into()).collect();
    errors.extend(checker.take_deferred_errors().iter().map(|e| format!("{e:?}").into()));
    errors.extend(checker.diagnostics().iter().filter(|e| e.is_error()).map(|e| format!("{e:?}").into()));
    errors
}

#[test]
fn method_signature_collection_rejects_a_negative_value_index() {
    let errors = signature_errors("fn invalid<M: meta Int>(&self, cell: Checked<-1>) {}");
    assert!(errors.iter().any(|e| e.contains("MetaRefinementViolation") || e.contains("declared value domain")), "{errors:?}");
}

#[test]
fn static_method_signature_collection_rejects_the_wrong_value_sort() {
    let errors = signature_errors("fn invalid<M: meta Int>(cell: Checked<true>) {}");
    assert!(errors.iter().any(|e| e.contains("wrong kind") || e.contains("sort") || e.contains("Mismatch")), "{errors:?}");
}

#[test]
fn valid_refined_method_signature_survives_collection() {
    let errors = signature_errors("fn valid<M: meta Int{>=0}>(&self, cell: Checked<M>) -> Checked<M> { cell }");
    assert!(errors.is_empty(), "{errors:?}");
}
