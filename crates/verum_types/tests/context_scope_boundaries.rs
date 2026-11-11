//! A provider authorizes calls only within the lexical scope that installed it.
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(body: &str) -> Vec<String> {
    let source = format!(
        "context Gate {{fn ping();}}
         type Channel is {{value: Int}};
         implement Channel {{fn ping(&self) {{}}}}
         fn needs() using [Gate] {{Gate.ping();}}
         {body}"
    );
    let module = Parser::new(&source).parse_module().expect("valid grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let registration = checker.prepare_module_declarations(&module);
    assert!(registration.is_empty(), "{registration:?}");
    let mut errors = Vec::new();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}")),
    );
    errors
}

#[test]
fn providers_end_with_their_lexical_block() {
    for provision in [
        "provide Gate = provider in {needs();}",
        "{provide Gate = provider; needs();}",
    ] {
        let errors = errors(&format!(
            "fn bad(provider: Channel) {{{provision} needs();}}"
        ));
        assert!(
            errors.iter().any(|error| error.contains("MissingContext")),
            "{provision}: {errors:?}"
        );
    }
}

#[test]
fn failed_provider_blocks_do_not_authorize_later_calls() {
    for provision in [
        "provide Gate = provider in {missing}",
        "{provide Gate = provider; missing}",
    ] {
        let errors = errors(&format!(
            "fn bad(provider: Channel) {{{provision} needs();}}"
        ));
        assert!(
            errors.iter().any(|error| error.contains("missing")),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|error| error.contains("MissingContext")),
            "{provision}: {errors:?}"
        );
    }
}

#[test]
fn scoped_provide_rejects_undeclared_contexts() {
    for provision in [
        "provide Missing = provider;",
        "provide Missing = provider in {}",
    ] {
        let errors = errors(&format!("fn bad(provider: Channel) {{{provision}}}"));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("UndefinedContext")),
            "{provision}: {errors:?}"
        );
    }
}

#[test]
fn providers_and_nested_shadowing_remain_available_inside_their_scope() {
    for body in [
        "fn valid(provider: Channel) {provide Gate = provider in {needs();}}",
        "fn valid(provider: Channel) {{provide Gate = provider; needs();}}",
        "fn valid(provider: Channel) {provide Gate = provider;
            {provide Gate = provider; needs();} needs();}",
        "fn valid(provider: Channel) {provide Gate = provider;
            provide Gate = provider in {needs();} needs();}",
        "fn valid(provider: Channel) using [Gate] {
            provide Gate = provider in {needs();} needs();}",
        "fn valid(provider: Channel) {
            {provide Gate = provider; needs();}
            {provide Gate = provider; needs();}}",
        "fn valid(provider: Channel) -> Int {let Gate = 7;
            provide Gate = provider in {needs();} Gate}",
        "fn valid(provider: Channel) -> Int {
            provide Gate = provider in {return 7;}}",
    ] {
        let errors = errors(body);
        assert!(errors.is_empty(), "{body}: {errors:?}");
    }
}

#[test]
fn same_scope_duplicate_providers_are_rejected() {
    let errors =
        errors("fn bad(provider: Channel) {provide Gate = provider; provide Gate = provider;}");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("DuplicateProvide")),
        "{errors:?}"
    );
}

#[test]
fn failed_inner_providers_preserve_the_enclosing_context() {
    for body in [
        "fn bad(provider: Channel) using [Gate] {
            provide Gate = provider in {missing} needs();}",
        "fn bad(provider: Channel) {provide Gate = provider;
            provide Gate = provider in {missing} needs();}",
    ] {
        let errors = errors(body);
        assert!(
            errors.iter().any(|error| error.contains("missing")),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| error.contains("UnboundVariable")),
            "{errors:?}"
        );
    }
}

#[test]
fn provider_initializers_cannot_use_the_context_they_will_install() {
    let errors =
        errors("fn bad(provider: Channel) {provide Gate = {needs(); provider} in {needs();}}");
    assert!(
        errors.iter().any(|error| error.contains("MissingContext")),
        "{errors:?}"
    );
}
