//! Writing brackets is applying a type constructor, so the number of
//! arguments has to match the declaration — in BOTH directions.
//!
//! The arity check existed and asked only `provided > expected`, so
//! `Pair<Int>` for a two-parameter `Pair` went through. An under-applied
//! constructor is the worse half: the free parameter is then unified with
//! whatever arrives first, so the program does not fail, it quietly means
//! something else.
//!
//! Under-application is not itself the error. A BARE `List` is the
//! unapplied constructor and higher-kinded positions want exactly that —
//! which is why the rule keys on the brackets, not on saturation.
//!
//! Task: T0922.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

fn complaints(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse");
    let mut checker = TypeChecker::new();

    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            let _ = checker.register_type_declaration(type_decl);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }

    // Read the error the way a user sees it: through its Diagnostic, which
    // is where the code lives. `Display` on a TypeError prints the message
    // only, so a test reading that channel cannot tell a coded refusal from
    // an uncoded one.
    let mut out: Vec<String> = Vec::new();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            let d = e.to_diagnostic();
            out.push(format!("{:?} {}", d.code(), d.message()));
        }
    }
    for d in checker.diagnostics().iter() {
        out.push(format!("{:?}", d));
    }
    out
}

const PAIR: &str = "type Pair<A, B> is { a: A, b: B };\n";

fn arity_complaint(cs: &[String]) -> bool {
    cs.iter().any(|c| c.contains("type argument"))
}

#[test]
#[ignore = "T0922: `provided < expected` is blocked on default type parameters. \
Turning it on refuses 194 uses of `Result<T>` in 31 core/ files, where `Result<T, E>` \
takes two and the stdlib is written as if `E` defaulted. The grammar has no \
`= Type` on a type parameter yet. Un-ignore with `type Result<T, E = Error>`."]
fn too_few_type_arguments_in_a_let_annotation_is_refused() {
    let cs = complaints(&format!(
        "{PAIR}fn main() {{ let p: Pair<Int> = Pair {{ a: 1, b: 2 }}; }}"
    ));
    assert!(
        arity_complaint(&cs),
        "`Pair<Int>` applies a two-parameter constructor to one argument: {cs:?}"
    );
}

#[test]
#[ignore = "T0922: `provided < expected` is blocked on default type parameters. \
Turning it on refuses 194 uses of `Result<T>` in 31 core/ files, where `Result<T, E>` \
takes two and the stdlib is written as if `E` defaulted. The grammar has no \
`= Type` on a type parameter yet. Un-ignore with `type Result<T, E = Error>`."]
fn too_few_type_arguments_in_a_signature_is_refused() {
    let cs = complaints(&format!("{PAIR}fn take(p: Pair<Int>) -> Int {{ 0 }}"));
    assert!(
        arity_complaint(&cs),
        "a parameter type is an annotation like any other: {cs:?}"
    );
}

#[test]
fn too_many_type_arguments_is_still_refused() {
    // The half that already worked. It is pinned here so the two-sided
    // rewrite cannot lose it.
    let cs = complaints(&format!(
        "{PAIR}fn take(p: Pair<Int, Text, Bool>) -> Int {{ 0 }}"
    ));
    assert!(arity_complaint(&cs), "3 arguments for 2 parameters: {cs:?}");
}

#[test]
fn the_right_number_of_type_arguments_is_accepted() {
    // The control. Without it, a check that refuses everything would pass
    // the three tests above.
    let cs = complaints(&format!(
        "{PAIR}fn take(p: Pair<Int, Text>) -> Int {{ p.a }}"
    ));
    assert!(
        !arity_complaint(&cs),
        "two arguments for two parameters: {cs:?}"
    );
}

#[test]
fn a_bare_constructor_name_is_not_an_application() {
    // No brackets, so nothing was applied and there is nothing to count.
    // This is how a higher-kinded position names a constructor, and it must
    // stay legal for the rule above to be about brackets rather than about
    // saturation.
    let cs = complaints(&format!(
        "{PAIR}type Holder<F> is {{ f: F }};\nfn main() {{}}"
    ));
    assert!(
        !arity_complaint(&cs),
        "an unapplied constructor supplies no arguments: {cs:?}"
    );
}

#[test]
fn the_arity_diagnostic_carries_a_code() {
    // A refusal with no code scores zero on any gate that filters on
    // `error<E...>`, which is how this one stayed invisible.
    let cs = complaints(&format!(
        "{PAIR}fn take(p: Pair<Int, Text, Bool>) -> Int {{ 0 }}"
    ));
    assert!(
        cs.iter().any(|c| c.contains("E407")),
        "the diagnostic must be coded: {cs:?}"
    );
}
