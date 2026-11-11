use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &ast.items {
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
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

#[test]
fn a_named_value_domain_survives_function_and_implementation_scopes() {
    let source = r#"
type Projector<k: meta USize {k > 0}> is { public tag: Int };
implement<k: meta USize {k > 0}> Projector<k> {
    fn complement(&self) -> Projector<k> { Projector {tag: self.tag} }
}
fn retain<n: meta USize {n > 0}>(value: Projector<n>) -> Projector<n> { value }
"#;
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn all_declared_subject_spellings_share_the_same_domain() {
    for spelling in ["n", "it", "self"] {
        let source = format!(
            r#"
type Positive<n: meta USize {{n > 0}}> is {{ tag: Int }};
fn accept<n: meta USize {{{spelling} > 1}}>(value: Positive<n>) {{}}
"#
        );
        let errors = errors(&source);
        assert!(errors.is_empty(), "{spelling}: {errors:?}");
    }
}

#[test]
fn a_declared_name_does_not_discharge_a_stronger_or_unsupported_domain() {
    for required in ["n > 1", "opaque_predicate(n)", "n > another"] {
        let source = format!(
            r#"
type Restricted<n: meta USize {{{required}}}> is {{ tag: Int }};
fn refused<n: meta USize {{n > 0}}>(value: Restricted<n>) {{}}
"#
        );
        let errors = errors(&source);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("declared value domain")),
            "{required}: {errors:?}"
        );
    }
}

#[test]
fn constants_must_satisfy_the_named_predicate() {
    for subject in ["n", "it", "self"] {
        let source = format!(
            r#"
type Positive<n: meta USize {{{subject} > 0}}> is | Item(Int);
fn accept(value: Positive<1>) {{}}
"#
        );
        let found = errors(&source);
        assert!(found.is_empty(), "{subject}: {found:?}");
        let wrong = source.replace("Positive<1>", "Positive<0>");
        let errors = errors(&wrong);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("declared value domain")
                    || error.contains("MetaRefinementViolation")),
            "{errors:?}"
        );
    }
}

#[test]
fn an_alias_retains_its_own_refinement_subject() {
    let valid = r#"
type Positive is USize {it > 0};
type Wrapped<n: meta Positive> is | Item(Int);
fn accept<n: meta USize {n > 1}>(value: Wrapped<n>) {}
"#;
    let found = errors(valid);
    assert!(found.is_empty(), "{found:?}");
    let bad = valid.replace("it > 0", "n > 0");
    let found = errors(&bad);
    assert!(
        found
            .iter()
            .any(|error| error.contains("declared value domain")),
        "{found:?}"
    );
}
