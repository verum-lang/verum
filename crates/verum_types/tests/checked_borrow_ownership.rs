//! Expected-type checking preserves the resource mode of borrowed operands.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(14))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("borrow");
    checker.set_current_module_path("borrow.owner");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| format!("{error:?}").into())
        .collect()
}

const DECLARATIONS: &str = "type affine Token is {value:Int};
    fn take(value:Token)->Int {value.value}";

#[test]
fn checked_shared_borrows_keep_affine_owners_available() {
    for body in [
        "let borrowed:&Token=&first; let again:&Token=&first; first.value",
        "let borrowed:&Token=&((first)); let again=&((first)); first.value",
        "let borrowed:&Int=&first.value; let again=&first.value; first.value",
        "let references=[&first,&second]; first.value+second.value",
        "for index in [0,1] {for token in [&first,&second] {let observed=token.value;}} first.value+second.value",
        "for index in [0,1] {let borrowed:&Token=&second; let observed=borrowed.value;} second.value",
    ] {
        let source = format!("{DECLARATIONS} fn probe(first:Token,second:Token)->Int {{{body}}}");
        let found = errors(&source);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}

#[test]
fn checked_borrows_do_not_hide_a_prior_move() {
    for body in [
        "let moved=first; let borrowed:&Token=&first; borrowed.value",
        "let moved=second; let references=[&first,&second]; 0",
    ] {
        let source = format!("{DECLARATIONS} fn probe(first:Token,second:Token)->Int {{{body}}}");
        let found = errors(&source);
        assert!(
            found.iter().any(|error| error.contains("MovedValueUsed")),
            "{source}: {found:?}"
        );
    }
}

#[test]
fn borrowing_restores_consumption_for_later_value_uses() {
    let source = format!(
        "{DECLARATIONS} fn probe(first:Token)->Int {{
        let borrowed:&Token=&first;
        let observed=borrowed.value;
        let moved=first;
        take(first)
    }}"
    );
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("MovedValueUsed")),
        "{found:?}"
    );
}

#[test]
fn nested_consumers_still_move_their_affine_arguments() {
    for body in [
        "for index in [0,1] {let moved=first;}",
        "for index in [0,1] {let borrowed:&Int=&take(first);}",
        "for index in [0,1] {let borrowed:&Int=&{let moved=first; moved.value};}",
        "for index in [0,1] {let borrowed=&{let moved=first; moved.value};}",
        "for index in [0,1] {let borrowed:&Int=&[73][{let moved=first; 0}];}",
        "for index in [0,1] {let borrowed=&[73][{let moved=first; 0}];}",
        "for index in [0,1] {let borrowed=&[first];}",
    ] {
        let source = format!("{DECLARATIONS} fn probe(first:Token) {{{body}}}");
        let found = errors(&source);
        assert!(
            found
                .iter()
                .any(|error| error.contains("AffineValueInLoop")),
            "{source}: {found:?}"
        );
    }
}

#[test]
fn shared_reference_checking_retains_expected_constructor_identity() {
    let source = "type First is None|Some(Int); type Second is None|Some(Bool);
        fn probe()->Int {let value:&Second=&None; match value {None=>0,Some(flag)=>1}}";
    let found = errors(source);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn reference_tiers_preserve_affine_operand_modes() {
    for tier in ["", "checked ", "unsafe "] {
        let source = format!(
            "{DECLARATIONS} fn probe(first:Token)->Int {{unsafe {{
            for index in [0,1] {{
                let borrowed:&{tier}Token=&{tier}first;
                let observed=borrowed.value;
            }}
            first.value
        }}}}"
        );
        let found = errors(&source);
        assert!(found.is_empty(), "{source}: {found:?}");

        for annotation in [Text::new(), Text::from(format!(":&{tier}Int"))] {
            let source = format!(
                "{DECLARATIONS} fn probe(first:Token) {{unsafe {{
                for index in [0,1] {{
                    let borrowed{annotation}=&{tier}{{let moved=first; moved.value}};
                }}
            }}}}"
            );
            let found = errors(&source);
            assert!(
                found
                    .iter()
                    .any(|error| error.contains("AffineValueInLoop")),
                "{source}: {found:?}"
            );
        }
    }
}
