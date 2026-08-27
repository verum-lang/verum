//! A type parameter may declare a DEFAULT, and an application may omit
//! the trailing slots that have one.
//!
//!     type Pair<A, B = Text> is { a: A, b: B };
//!     fn take(p: Pair<Int>) -> Text { p.b }
//!
//! The default was already in the AST (`GenericParamKind::Type.default`)
//! and the grammar did not mention it; the parser read it only AFTER a
//! bounds clause, so `<B: Named = Text>` parsed and `<B = Text>` did not;
//! and nothing substituted it, so an omitted slot survived as the bare
//! parameter name and a mismatch reported `found 'B'` — a type the
//! program never wrote.
//!
//! WHY IT MATTERS BEYOND CONVENIENCE: the stdlib already assumes it. 194
//! uses of `Result<X>` across 31 core/ files apply a two-parameter
//! `Result<T, E>` to one argument, which is why generic arity cannot be
//! checked in both directions until this exists (T0922).
//!
//! Task: T0922.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

fn complaints(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(t) = &item.kind {
            let _ = checker.register_type_declaration(t);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(f);
        }
    }
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

const PAIR: &str = "type Pair<A, B = Text> is { a: A, b: B };\n";

#[test]
fn an_omitted_trailing_slot_takes_its_default() {
    // Reading the defaulted field at its declared default type must check.
    let cs = complaints(&format!("{PAIR}fn take(p: Pair<Int>) -> Text {{ p.b }}"));
    assert!(cs.is_empty(), "`B` defaults to Text: {cs:?}");
}

#[test]
fn the_default_is_a_real_type_not_a_free_parameter() {
    // THE CONTROL THAT MATTERS. Without it, a checker that simply ignores
    // the omitted slot passes the test above — the field's type stays the
    // unresolved parameter and unifies with anything. Asking for the
    // WRONG type must name the DEFAULT, not the parameter.
    let cs = complaints(&format!("{PAIR}fn take(p: Pair<Int>) -> Int {{ p.b }}"));
    assert!(
        cs.iter().any(|c| c.contains("Text")),
        "the mismatch must name Text, the type the default supplies: {cs:?}"
    );
    assert!(
        !cs.iter().any(|c| c.contains("found 'B'")),
        "`B` is what the program did NOT write: {cs:?}"
    );
}

#[test]
fn a_default_can_be_written_without_bounds() {
    // The parser reached the default only through the bounds branch, so
    // this spelling — the common one — failed to parse at the `=`.
    let mut parser = Parser::new("type Pair<A, B = Text> is { a: A, b: B };");
    let module = parser.parse_module().expect("`<B = Text>` must parse");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn a_default_still_works_alongside_bounds() {
    // The spelling that already parsed, pinned so the fix to its sibling
    // cannot take it away.
    let mut parser = Parser::new(
        "type Named is protocol { fn name(&self) -> Text; };\n\
         type Pair<A, B: Named = Text> is { a: A, b: B };",
    );
    let module = parser.parse_module().expect("`<B: Named = Text>` must parse");
    assert_eq!(module.items.len(), 2);
}

#[test]
fn a_slot_with_no_default_is_not_filled() {
    // Only a trailing run of DEFAULTED slots may be omitted. `Pair<A, B>`
    // has no defaults, so `Pair<Int>` supplies one argument for two and
    // nothing invents the second.
    let cs = complaints(
        "type Plain<A, B> is { a: A, b: B };\n\
         fn take(p: Plain<Int>) -> Int { p.a }",
    );
    // The arity refusal itself is T0922's blocked half; what this pins is
    // that the DEFAULT machinery does not silently paper over the hole.
    assert!(
        !cs.iter().any(|c| c.contains("Text")),
        "no default exists to fill `B` with: {cs:?}"
    );
}

#[test]
fn an_explicit_argument_beats_the_default() {
    // Supplying the slot must override, not be overridden.
    let cs = complaints(&format!("{PAIR}fn take(p: Pair<Int, Int>) -> Int {{ p.b }}"));
    assert!(
        cs.is_empty(),
        "`B` was supplied as Int, so the default does not apply: {cs:?}"
    );
}
