//! A capability restricts what may be DONE with a value. It does not
//! change what the value IS, and it may be given away, never acquired.
//!
//! Both halves were broken, and the pair is why: a restricted record
//! stopped being a record, so `s.n` reported
//!
//!     error<E103>: Cannot access field 'n' on non-record type:
//!                  Store with [ReadOnly]
//!
//! which made the feature unusable on the shape it exists for — while
//! WIDENING, the thing it exists to refuse, compiled silently. So there
//! was no program that distinguished a working attenuation check from a
//! broken one: the negative pole looked exactly like the positive one.
//!
//! Task: T0918.

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
        out.push(format!("{:?} {}", d.code(), d.message()));
    }
    out
}

const STORE: &str = "public type Store is { n: Int, tag: Text };\n";

#[test]
fn a_restricted_record_is_still_a_record() {
    let cs = complaints(&format!(
        "{STORE}public fn reads(s: Store with [Read]) -> Int {{ s.n }}"
    ));
    assert!(
        cs.is_empty(),
        "a capability does not change what the value is: {cs:?}"
    );
}

#[test]
fn a_restricted_record_keeps_every_field() {
    // A fix that special-cased one field type would pass the test above.
    let cs = complaints(&format!(
        "{STORE}public fn tag_of(s: Store with [Read]) -> Text {{ s.tag.clone() }}"
    ));
    assert!(cs.is_empty(), "both fields, both types: {cs:?}");
}

#[test]
fn a_restriction_behind_a_reference_still_reaches_the_fields() {
    let cs = complaints(&format!(
        "{STORE}public fn reads(s: &Store with [Read]) -> Int {{ s.n }}"
    ));
    assert!(cs.is_empty(), "the wrapper sits under the reference: {cs:?}");
}

#[test]
fn attenuation_is_allowed() {
    // Giving away rights is always safe, and it is the direction the
    // feature exists to make ordinary.
    let cs = complaints(&format!(
        "{STORE}\
         public fn narrow(s: Store with [Read]) -> Int {{ s.n }}\n\
         public fn wide(s: Store with [Read, Write]) -> Int {{ narrow(s) }}"
    ));
    assert!(cs.is_empty(), "[Read, Write] satisfies [Read]: {cs:?}");
}

#[test]
fn an_unrestricted_value_satisfies_any_restriction() {
    // A plain `Store` states no restriction, i.e. full rights, so passing
    // it is itself an attenuation. Refusing this would make every
    // capability parameter unreachable from ordinary code.
    let cs = complaints(&format!(
        "{STORE}\
         public fn needs_write(s: Store with [Read, Write]) -> Int {{ s.n }}\n\
         public fn plain(s: Store) -> Int {{ needs_write(s) }}"
    ));
    assert!(cs.is_empty(), "unrestricted holds every right: {cs:?}");
}

#[test]
fn widening_is_refused() {
    // THE POLE THAT MAKES THE OTHERS MEAN SOMETHING. Without it, the peel
    // above is indistinguishable from deleting the check.
    let cs = complaints(&format!(
        "{STORE}\
         public fn needs_write(s: Store with [Read, Write]) -> Int {{ s.n }}\n\
         public fn only_read(s: Store with [Read]) -> Int {{ needs_write(s) }}"
    ));
    assert!(
        cs.iter().any(|c| c.contains("E411")),
        "[Read] cannot satisfy [Read, Write]: {cs:?}"
    );
}

#[test]
fn the_widening_refusal_names_both_sets() {
    // "cannot be widened" alone leaves the reader to work out which right
    // is missing and where from.
    let cs = complaints(&format!(
        "{STORE}\
         public fn needs_write(s: Store with [Read, Write]) -> Int {{ s.n }}\n\
         public fn only_read(s: Store with [Read]) -> Int {{ needs_write(s) }}"
    ));
    let msg = cs.join("\n");
    assert!(
        msg.contains("carries") && msg.contains("required"),
        "the refusal must name what is held and what is needed: {cs:?}"
    );
}
