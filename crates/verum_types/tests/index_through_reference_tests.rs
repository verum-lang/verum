#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Regression tests for A89 — indexing through a reference to a type ALIAS.
//!
//! Measured 2026-09-05, one variable changed at a time:
//!
//! ```text
//!   type M is List<List<Float>>;
//!   fn f(m: &M) -> Float { m[0][0] }        Cannot index non-indexable type
//!   fn f(m: M)  -> Float { m[0][0] }        ok    (by value)
//!   fn f(m: &List<List<Float>>) …           ok    (reference, no alias)
//! ```
//!
//! So it was neither references nor aliases nor indexing, but a reference
//! whose POINTEE is a named type the Index-protocol lookup cannot resolve.
//! `&checked M` and `&unsafe M` failed identically — all three tiers.
//!
//! Cause: the lenient fallback ("a named type may implement Index; give it a
//! fresh element type") matched on the OUTER type. `Type::Reference{..}`
//! matches none of its arms, so every reference whose pointee needed that
//! leniency fell through to the error — while the same pointee written by
//! value took the lenient arm. The fallback now decides on the pointee.
//!
//! Found in `crates/verum_cli/examples/cbgr_references.vr`, the shipped
//! example whose whole subject is the three reference tiers: five
//! occurrences, every tier.
//!
//! WHAT THESE TESTS PIN: that by-value and by-reference AGREE, at all three
//! tiers — not that indexing is always allowed. The negative control keeps a
//! genuinely non-indexable pointee refused.
//!
//! A FIRST REVISION ASSERTED ACCEPTANCE and failed for a reason that is not
//! the language: this harness builds a bare `TypeChecker` with no stdlib, so
//! `type M is List<List<Float>>` registers M as `<placeholder:M>` and the
//! lenient arm does not fire for the by-value form either. Through the CLI,
//! where `List` resolves, both forms are accepted — measured, 0 errors each.
//! The absolute claim belongs to a CLI probe; the agreement belongs here.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Type-check a module and return the INDEX diagnostics only. A bare
/// `TypeChecker` has no stdlib, so an unfiltered comparison measures
/// `TypeNotFound` on both sides and agrees for the wrong reason.
fn index_errors(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(f);
        }
    }
    let mut errs: Vec<String> = module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect();
    errs.extend(checker.diagnostics().iter().map(|d| format!("{:?}", d)));
    errs.retain(|e| e.contains("non-indexable") || e.contains("Cannot index"));
    errs
}

const ALIAS: &str = "type M is List<List<Float>>;\n\n";

/// The control that made the row precise: by value, the alias indexes — but
/// only where `List` RESOLVES. Measured 2026-09-05: in this stdlib-less
/// harness `type M is List<List<Float>>` registers M as `<placeholder:M>`,
/// which is neither Named nor Generic nor Unknown nor Var, so the lenient arm
/// does not fire and the BY-VALUE form is refused too. Through the CLI, with
/// the stdlib present, both forms are accepted (measured: 0 errors each).
///
/// So the absolute claim belongs to the CLI probe and the AGREEMENT belongs
/// here. Asserting acceptance in this harness would pin the harness's missing
/// stdlib, not the language.
#[test]
fn by_value_and_by_reference_are_refused_or_accepted_together() {
    let by_value = format!("{ALIAS}fn f(m: M) -> Float {{ m[0][0] }}\n");
    let by_ref = format!("{ALIAS}fn f(m: &M) -> Float {{ m[0][0] }}\n");
    assert_eq!(
        index_errors(&by_value).is_empty(),
        index_errors(&by_ref).is_empty(),
        "a reference is indexable exactly when its pointee is.\n  value: {:?}\n  ref:   {:?}",
        index_errors(&by_value),
        index_errors(&by_ref),
    );
}

/// The second control: a reference WITHOUT the alias indexes.
#[test]
fn indexing_through_a_reference_without_an_alias_is_accepted() {
    let src = "fn f(m: &List<List<Float>>) -> Float { m[0][0] }\n";
    assert!(
        index_errors(src).is_empty(),
        "reference, no alias: {:?}",
        index_errors(src)
    );
}

/// The subject, at all three reference tiers. Each is one token from a
/// program that already worked.
#[test]
fn indexing_through_a_reference_to_an_alias_agrees_with_by_value() {
    let by_value = format!("{ALIAS}fn f(m: M) -> Float {{ m[0][0] }}\n");
    for tier in ["&", "&checked ", "&unsafe "] {
        let src = format!("{ALIAS}fn f(m: {tier}M) -> Float {{ m[0][0] }}\n");
        let by_ref = index_errors(&src);
        assert_eq!(
            index_errors(&by_value).is_empty(),
            by_ref.is_empty(),
            "`{tier}M` disagrees with the same pointee taken by value. \
             A reference is indexable exactly when its pointee is.\n  {by_ref:?}",
        );
    }
}

/// NEGATIVE CONTROL. A pointee that genuinely cannot be indexed must stay
/// refused — otherwise "make the reference agree" is satisfiable by allowing
/// everything.
#[test]
fn a_reference_to_a_non_indexable_type_is_still_refused() {
    let by_value = "fn f(x: Int) -> Int { x[0] }\n";
    let by_ref = "fn f(x: &Int) -> Int { x[0] }\n";
    assert!(
        !index_errors(by_value).is_empty(),
        "indexing an Int by value must be refused"
    );
    assert!(
        !index_errors(by_ref).is_empty(),
        "indexing an Int through a reference must be refused too"
    );
}
