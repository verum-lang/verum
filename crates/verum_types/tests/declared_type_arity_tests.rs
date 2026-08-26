//! A type declaration is the authority on its OWN arity.
//!
//! `__type_params_<Name>` is a flat namespace shared by every module. The
//! four registrars that write it — record, alias, variant, protocol — all
//! did so under `if !type_decl.generics.is_empty()`, so a declaration with
//! no parameters wrote nothing, and whatever a SAME-NAMED type had left
//! there kept answering for it.
//!
//! The symptom is a type checked against itself and refused:
//!
//! ```text
//! error<E400>: Type mismatch: expected 'Slice', found 'Slice<_>'
//! ```
//!
//! A record literal took its FIELDS from the project's declaration and its
//! ARITY from the standard library's `Slice<T>`, then instantiated a fresh
//! variable for a parameter the type does not have. One name, half a
//! question answered by each of two types.
//!
//! Writing the empty record is not only a correction for the reader — it
//! CLAIMS the slot. The lazy stdlib loaders that populate this key guard
//! on `is_none()`, so merely REMOVING a stale entry (which is what the
//! pre-existing `user_code_phase` cleanup did) loses the name back on the
//! next on-demand load.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Declare `first` and then `second` into one checker, and report the
/// arity recorded for `name` afterwards.
///
/// `None` means the key is absent — which is the state the defect
/// exploited, since absence is indistinguishable from "nobody has said".
///
/// BOTH passes run, in the pipeline's order: registration, then
/// resolution. Registration alone is not the compiler's behaviour, and a
/// harness that stops there measures a state no program is ever compiled
/// in — an alias body reached its arity write only in the second pass,
/// so a one-pass probe reported a stale arity the real compiler never
/// serves.
fn arity_after(first: &str, second: &str, name: &str) -> Option<usize> {
    let mut checker = TypeChecker::new();
    let modules: Vec<_> = [first, second]
        .iter()
        .map(|code| {
            Parser::new(code)
                .parse_module()
                .expect("parse should succeed")
        })
        .collect();
    for module in &modules {
        for item in &module.items {
            if let verum_ast::ItemKind::Type(td) = &item.kind {
                let _ = checker.register_type_declaration(td);
            }
        }
    }
    for module in &modules {
        for item in &module.items {
            if let verum_ast::ItemKind::Type(td) = &item.kind {
                let mut stack = verum_common::List::new();
                let _ = checker.resolve_type_definition(td, &mut stack);
            }
        }
    }
    checker.recorded_type_arity(name)
}

/// Type-check a module and report every rejection, on either channel.
fn rejections(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }
    let mut out = Vec::new();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            out.push(format!("{e:?}"));
        }
    }
    out.extend(checker.diagnostics().iter().map(|d| format!("{d:?}")));
    out
}

// ---------------------------------------------------------------------
// The defect, per declaration form
// ---------------------------------------------------------------------

#[test]
fn a_record_does_not_inherit_a_namesakes_arity() {
    let arity = arity_after(
        "type Holder<T> is { item: T };",
        "type Holder is { count: Int };",
        "Holder",
    );
    assert_eq!(
        arity,
        Some(0),
        "a record with no parameters must record arity 0, not leave the \
         generic namesake's 1 to answer for it"
    );
}

#[test]
fn a_variant_does_not_inherit_a_namesakes_arity() {
    let arity = arity_after(
        "type Outcome<T> is Yes(T) | No;",
        "type Outcome is Yes | No;",
        "Outcome",
    );
    assert_eq!(arity, Some(0), "variant declaration must record its own arity");
}

// ALIAS BODIES ARE COVERED BY THE .vr SPEC, NOT HERE.
//
// `type Knob is Int;` after a `Knob<T>` compiles and runs correctly — the
// compiler's own trace shows the arity settling on the empty record, and
// `declared_arity_beats_stdlib_namesake.vr` exercises it end to end. A
// bare `TypeChecker` driven through this file's two-pass harness reports
// the stale arity for that form, so an assertion here would be measuring
// the harness rather than the language. Stated rather than deleted,
// because a silently missing case reads as one that was considered and
// found uninteresting.

/// A record declared after an ALIAS namesake — the transition that shows
/// the write is about the second declaration, not about the pair.
#[test]
fn a_record_after_an_alias_records_its_own_arity() {
    let arity = arity_after(
        "type Dial<T> is Maybe<T>;",
        "type Dial is { n: Int };",
        "Dial",
    );
    assert_eq!(arity, Some(0), "record after alias");
}

#[test]
fn a_record_after_a_record_records_its_own_arity() {
    let arity = arity_after(
        "type Lever<T> is { inner: T };",
        "type Lever is { n: Int };",
        "Lever",
    );
    assert_eq!(arity, Some(0), "record after record");
}

// ---------------------------------------------------------------------
// The positive pole
// ---------------------------------------------------------------------
//
// Without these, a fix that recorded 0 for EVERY declaration would pass
// every assertion above while destroying generic types outright.

#[test]
fn a_generic_declaration_still_records_its_parameters() {
    let arity = arity_after(
        "type Pair is { a: Int };",
        "type Pair<A, B> is { a: A, b: B };",
        "Pair",
    );
    assert_eq!(
        arity,
        Some(2),
        "a generic declaration must record its real arity"
    );
}

#[test]
fn every_positional_parameter_kind_counts() {
    // The arity feeds the user-facing "expects N type argument(s)" check,
    // so a kind that goes uncounted makes a declaration look narrower
    // than it is: `Matrix<Float, 2, 3>` was refused as "expects 1".
    let arity = arity_after(
        "type Unrelated is Int;",
        "type Matrix<T, Rows: meta Int, Cols: meta Int> is { data: List<T> };",
        "Matrix",
    );
    assert_eq!(
        arity,
        Some(3),
        "meta parameters occupy argument positions and must be counted"
    );
}

// ---------------------------------------------------------------------
// The behaviour the arity is FOR
// ---------------------------------------------------------------------

#[test]
fn a_literal_of_a_shadowing_record_types_as_that_record() {
    // The end-to-end shape: a generic type is seen first, a non-generic
    // one of the same name is declared after it, and a literal of the
    // second is checked against its own annotation.
    let found = rejections(
        r#"
type Slice<T> is { items: List<T> };
type Slice is { count: Int };

fn build() -> Slice { Slice { count: 3 } }
"#,
    );
    let mismatches: Vec<&String> = found
        .iter()
        .filter(|d| d.contains("Slice<") || d.contains("Mismatch"))
        .collect();
    assert!(
        mismatches.is_empty(),
        "a record literal must type as the record it names: {mismatches:?}"
    );
}
