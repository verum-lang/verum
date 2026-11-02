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
    assert!(
        cs.is_empty(),
        "the wrapper sits under the reference: {cs:?}"
    );
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

#[test]
fn restrictions_survive_value_boundaries() {
    for body in [
        "fn full(s: Store) -> Int { s.n } fn leak(s: Store with [Read]) -> Int { full(s) }",
        "fn leak(s: Store with [Read]) -> Store { s }",
        "fn leak(s: Store with [Read]) -> Store { let full: Store = s; full }",
        "fn leak(s: Store with [Read]) -> Store { Store { ..s } }",
        "fn leak(s: (Store with [Read], Int)) -> (Store, Int) { s }",
        "fn leak(f: fn(Store) -> Int) -> fn(Store with [Read]) -> Int { f }",
        "fn leak(s: &mut Store with [Read]) -> &mut Store { s }",
        "fn leak(s: Store with [Read]) -> unknown { s }",
        "fn leak(s: Store with [Read]) -> Store { s as Store }",
        "type View is Store with [Read]; fn leak(s: View) -> Store { s }",
        "fn same<T>(x: T) -> T { x } fn leak(s: Store with [Read]) -> Store { same(s) }",
        "fn leak(s: &mut Store) -> &mut Store with [Read] { s }",
        "type Cell<T> is { value: T }; fn leak(s: Cell<Store with [Read]>) -> Cell<Store> { s }",
    ] {
        let errors = complaints(&format!("{STORE}{body}"));
        assert!(
            errors.iter().any(|error| {
                error.contains("ReadOnly")
                    && ["E411", "E401", "E400"]
                        .iter()
                        .any(|code| error.contains(code))
            }),
            "expected a permission or carrier mismatch for {body}: {errors:?}"
        );
    }
}

#[test]
fn safe_variance_preserves_authority_through_aggregates_and_callbacks() {
    for body in [
        "fn fresh() -> Store with [Read] { Store { n: 1, tag: \"fresh\" } }",
        "fn narrow(s: (Store, Int)) -> (Store with [Read], Int) { s }",
        "fn narrow(f: fn(Store with [Read]) -> Int) -> fn(Store) -> Int { f }",
        "fn same(s: &mut Store with [Read]) -> &mut Store with [Read] { s }",
        "fn identity<T>(s: T) -> T { s } fn keep(s: Store with [Read]) -> Store with [Read] { identity(s) }",
    ] {
        let errors = complaints(&format!("{STORE}{body}"));
        assert!(errors.is_empty(), "rejected {body}: {errors:?}");
    }
}

#[test]
#[ignore = "requires the pinned Lean toolchain"]
fn native_permissions_and_equality_correspond_to_the_lean_model() {
    use std::{fmt::Write, path::Path, process::Command};
    use verum_ast::{Capability::*, Span};
    use verum_types::{Unifier, capability::TypeCapabilitySet, subtype::Subtyping, ty::Type};

    let scopes = [
        (".full", None),
        (".restricted []", Some(vec![])),
        (".restricted [.read]", Some(vec![ReadOnly])),
        (".restricted [.write]", Some(vec![WriteOnly])),
        (".restricted [.readWrite]", Some(vec![ReadWrite])),
        (
            ".restricted [.read, .write]",
            Some(vec![ReadOnly, WriteOnly]),
        ),
        (".restricted [.admin]", Some(vec![Admin])),
        (".restricted [.admin, .read]", Some(vec![Admin, ReadOnly])),
        (".restricted [.other 0]", Some(vec![Execute])),
        (
            ".restricted [.other 1]",
            Some(vec![Custom("publish".into())]),
        ),
    ]
    .map(|(name, rights)| {
        (
            name,
            rights.map_or(Type::Int, |rights| Type::CapabilityRestricted {
                base: Box::new(Type::Int),
                capabilities: TypeCapabilitySet::from_list(rights.into()),
            }),
        )
    });
    let mut source = include_str!(
        "../../../verification/external/lean/VerumExternalReplay/CapabilityAttenuation.lean"
    )
    .to_owned();
    source.push_str("\nopen Verum.CapabilityAttenuation\n");
    let checker = Subtyping::new();
    for (source_name, actual) in &scopes {
        for (target_name, expected) in &scopes {
            let forward = format!("accepts ({source_name}) ({target_name})");
            let reverse = format!("accepts ({target_name}) ({source_name})");
            let accepted = checker.is_subtype(actual, expected);
            let equal = Unifier::new()
                .unify(actual, expected, Span::dummy())
                .is_ok();
            writeln!(source, "example : {forward} = {accepted} := by decide").unwrap();
            writeln!(
                source,
                "example : ({forward} && {reverse}) = {equal} := by decide"
            )
            .unwrap();
        }
    }
    source.push_str("#print axioms preserves_permissions\n");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Check.lean");
    std::fs::write(&path, source).unwrap();
    let output = Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(&path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output()
        .unwrap();
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{diagnostics}");
    assert!(
        diagnostics.contains("Verum.CapabilityAttenuation.preserves_permissions"),
        "{diagnostics}"
    );
    assert!(
        !diagnostics.contains("sorryAx") && !diagnostics.contains("warning:"),
        "{diagnostics}"
    );
}
