//! `@derive(Ord)` was parsed, accepted as a known attribute, and applied
//! by nobody: the dispatcher that would apply it had ZERO callers, and
//! the subtree below it is descriptive — `derive_ord` returns a
//! `DerivedBody` that has no consumers outside its own file.
//!
//! So a user who wrote `@derive(Ord)` got exactly what a user who wrote
//! nothing got: `a < b` compiled to a POINTER comparison and printed the
//! wrong answer in a third of runs.
//!
//! WIRING THE DISPATCHER WOULD HAVE MADE IT WORSE. Registering a
//! `ProtocolImpl` makes `implements_protocol(T, "Ord")` true, which
//! silences W0506 — the diagnostic that detects the real defect — while
//! no `T.cmp` exists and the value stays dice. The attribute would look
//! like it worked and switch off the detector for what it did not fix.
//!
//! Hence generation, and generation of SOURCE: everything downstream sees
//! an ordinary `implement` block, and lexicographic order is spelled with
//! the standard library's own `Ordering.then` rather than reinvented.
//!
//! These tests pin the SHAPE of the generated source. The value —
//! `@derive(Ord)` makes `a < b` correct across ten runs — is measured
//! end-to-end in the task; a unit test cannot run a program.
//!
//! Task: T1018.

use verum_fast_parser::FastParser;

fn injected_source(code: &str) -> String {
    let parser = FastParser::new();
    let mut module = parser
        .parse_module_str(code, verum_ast::FileId::new(0))
        .expect("parse");
    let before = module.items.len();
    let _ = verum_compiler::pipeline::inject_derived_impls_for_test(&mut module);
    let mut out = String::new();
    for item in module.items.iter().skip(before) {
        out.push_str(&verum_ast::pretty::format_item(item).as_str().to_string());
        out.push('\n');
    }
    out
}

#[test]
fn derive_ord_on_a_one_field_record_generates_a_cmp() {
    let src = injected_source("@derive(Ord)\ntype Id is { v: Int };\n");
    assert!(
        src.contains("Ord") && src.contains("cmp"),
        "an `implement Ord` with a `cmp` must be generated; got:\n{src}"
    );
}

/// Lexicographic order over several fields, chained with the standard
/// library's own combinator — not a second definition of what
/// lexicographic means.
#[test]
fn derive_ord_chains_several_fields_with_then() {
    let src = injected_source("@derive(Ord)\ntype P is { a: Int, b: Int };\n");
    assert!(
        src.contains("then"),
        "two fields must be chained with `Ordering.then`; got:\n{src}"
    );
}

/// The differentiator: a type with NO derive must gain nothing. Without
/// this, a pass that appended an impl to every record would satisfy both
/// tests above.
#[test]
fn a_type_without_derive_gains_nothing() {
    let src = injected_source("type Q is { v: Int };\n");
    assert!(
        src.trim().is_empty(),
        "nothing may be generated for a type that asked for nothing; got:\n{src}"
    );
}

/// A derive this pass cannot honour is REPORTED, not dropped. Silence is
/// the behaviour being replaced.
#[test]
fn an_underivable_protocol_is_reported_rather_than_dropped() {
    let parser = FastParser::new();
    let mut module = parser
        .parse_module_str(
            "@derive(Hash)\ntype Id is { v: Int };\n",
            verum_ast::FileId::new(0),
        )
        .expect("parse");
    let unsupported = verum_compiler::pipeline::inject_derived_impls_for_test(&mut module);
    assert_eq!(
        unsupported.len(),
        1,
        "a derive with no generator must be reported"
    );
    assert_eq!(unsupported[0].1, "Hash");
}
