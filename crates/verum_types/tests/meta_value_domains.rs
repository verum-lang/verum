//! Value-sort admission and predicate entailment use the declaring domain.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("domains");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(format!("{d:?}"))),
    );
    errors
}
fn source(consumer: &str) -> Text {
    format!("module domains; type Extent<N: meta Int{{>= 0}}> is {{ value: () }}; {consumer}")
        .into()
}
#[test]
fn equivalent_and_stronger_symbolic_domains_survive_renaming() {
    for domain in [">= 0", "> 0", ">= 7", "it >= 0 && it <= 9"] {
        let input = source(&format!(
            "fn consume<M: meta Int{{{domain}}}>(x: &Extent<M>) -> Int {{ M }}"
        ));
        let errors = errors(&input);
        assert!(errors.is_empty(), "{domain}: {errors:?}");
    }
}
#[test]
fn valid_constant_arguments_are_checked_in_the_declared_domain() {
    for value in ["0", "1", "19", "2 + 3"] {
        let errors = errors(&source(&format!("fn consume(x: &Extent<{value}>) {{}}")));
        assert!(errors.is_empty(), "{value}: {errors:?}");
    }
}
#[test]
fn invalid_constant_cannot_inhabit_the_refined_value_domain() {
    let errors = errors(&source("fn consume(x: &Extent<-1>) {}"));
    assert!(
        errors.iter().any(|e| e.contains("MetaRefinementViolation")),
        "{errors:?}"
    );
}
#[test]
fn weaker_and_unconstrained_symbolic_domains_are_not_admitted() {
    for domain in ["Int{>= -1}", "Int", "Int{it >= -3 && it <= 5}"] {
        let errors = errors(&source(&format!(
            "fn consume<M: meta {domain}>(x: &Extent<M>) {{}}"
        )));
        assert!(
            errors
                .iter()
                .any(|e| e.contains("cannot establish the declared value domain")),
            "{domain}: {errors:?}"
        );
    }
}
#[test]
fn a_different_value_sort_is_still_rejected() {
    let errors = errors(&source("fn consume<M: meta Bool>(x: &Extent<M>) {}"));
    assert!(
        errors.iter().any(|e| e.contains("wrong kind")),
        "{errors:?}"
    );
}
#[test]
fn unproved_index_arithmetic_does_not_erase_its_obligation() {
    let errors = errors(&source(
        "fn consume<M: meta Int{>= 0}>(x: &Extent<M - 1>) {}",
    ));
    assert!(
        errors.iter().any(
            |e| e.contains("generic value expression has no represented compile-time identity")
        ),
        "{errors:?}"
    );
}
#[test]
fn identical_opaque_predicates_are_not_proofs() {
    let errors = errors(
        "module domains; fn opaque(x: Int) -> Bool { x >= 0 } type Checked<N: meta Int{opaque(it)}> is { value: () }; fn consume<M: meta Int{opaque(it)}>(x: &Checked<M>) {}",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("cannot establish the declared value domain")),
        "{errors:?}"
    );
}

#[test]
fn forwarding_aliases_cannot_erase_a_required_value_domain() {
    let errors = errors(&source("type Forward<N: meta Int> is Extent<N>;"));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("cannot establish the declared value domain")),
        "{errors:?}"
    );
}

#[test]
fn a_refined_alias_preserves_the_domain_across_renamed_parameters() {
    let errors = errors(
        "module domains; type NonNegative is Int{>=0}; type Extent<N: meta NonNegative> is {value: ()}; fn consume<M: meta NonNegative>(x: &Extent<M>) {}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}
