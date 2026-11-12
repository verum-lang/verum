use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(body: &str) -> List<Text> {
    let source = format!(
        "type Pair is (Int, Bool);
        type Checked is Pair where |pair| pair.0 >= 0;
        type Alias is Checked;
        type Borrowed is &Checked;
        {body}"
    );
    let ast = Parser::new(&source)
        .parse_module()
        .expect("valid source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut result: List<Text> = errors
        .iter()
        .map(|error| error.to_string().into())
        .collect();
    result.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message().into()),
    );
    result
}

#[test]
fn aliases_and_reference_tiers_keep_the_refined_tuple_carrier() {
    for ty in [
        "Checked",
        "Alias",
        "&Checked",
        "&mut Alias",
        "&checked Checked",
        "&unsafe Alias",
        "&Borrowed",
    ] {
        let source = format!(
            "fn read(value: {ty}) -> Int {{ value.0 }}
                              fn flag(value: {ty}) -> Bool {{ value.1 }}"
        );
        let found = errors(&source);
        assert!(found.is_empty(), "{source}: {found:?}");
        for invalid in [
            format!("fn bad(value: {ty}) -> Bool {{ value.0 }}"),
            format!("fn bad(value: {ty}) -> Int {{ value.2 }}"),
        ] {
            assert!(
                !errors(&invalid).is_empty(),
                "accepted invalid projection: {invalid}"
            );
        }
    }
}

#[test]
fn a_refinement_does_not_invent_a_tuple_carrier() {
    assert!(
        !errors(
            "type Positive is Int where |n| n > 0;
                    fn bad(value: &Positive) -> Int { value.0 }"
        )
        .is_empty()
    );
}

#[test]
fn projected_mutation_cannot_invalidate_the_enclosing_refinement() {
    for body in [
        "fn bad(value: &mut Checked) { value.0 = -1; }",
        "fn bad(value: &mut Alias) -> &mut Int { &mut value.0 }",
        "fn bad(value: &mut Checked) { let alias = value; alias.0 = -1; }",
        "fn forward(value: &mut Checked) -> &mut Checked { value }
         fn bad(value: &mut Checked) { forward(value).0 = -1; }",
    ] {
        let found = errors(body);
        assert!(found.iter().any(|message| message.contains("enclosing refinement")),
            "missing refinement protection for {body}: {found:?}");
    }
    let found = errors("fn replace(value: &mut Checked, next: Checked) { *value = next; }");
    assert!(
        found.is_empty(),
        "whole-value replacement failed: {found:?}"
    );
}
