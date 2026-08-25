//! A refinement says WHICH values inhabit a type. It never removes an
//! operation the base type supports.
//!
//! Indexing consulted the type's structure to decide indexability and
//! stopped at the first wrapper it did not recognise. A refinement
//! nested with a reference — `&List<Int>{len > 0}`, the natural way to
//! say "a borrowed non-empty list" and the exact shape the refinement
//! chapter of the documentation uses — reached the final arm and was
//! rejected with "cannot index non-indexable type". Either wrapper
//! ALONE was accepted, which is what kept this invisible: the two
//! obvious probes both pass.
//!
//! `peel_refinement` in `infer/expr.rs` now strips refinements on both
//! sides of the reference before indexability is decided.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Type-check a module and report every rejection, on either channel.
///
/// Errors surface as a hard `Err` OR as an Error-severity diagnostic
/// (statement recovery pushes a diagnostic and continues), so a test
/// that reads only one channel can call a rejected program accepted.
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

fn indexes_through(param_ty: &str) -> Vec<String> {
    rejections(&format!(
        r#"
fn nth(xs: {param_ty}, i: Int) -> Int {{
    xs[i]
}}
"#
    ))
}

fn assert_indexable(param_ty: &str) {
    let found = indexes_through(param_ty);
    let index_complaints: Vec<&String> = found
        .iter()
        .filter(|m| m.contains("index") || m.contains("Index"))
        .collect();
    assert!(
        index_complaints.is_empty(),
        "`{param_ty}` must be indexable; got {index_complaints:?}"
    );
}

#[test]
fn a_plain_reference_to_a_list_is_indexable() {
    // Control A — passed before the fix, and pins the harness.
    assert_indexable("&List<Int>");
}

#[test]
fn a_refined_list_is_indexable() {
    // Control B — also passed before the fix. A and B together are why
    // the defect needed BOTH wrappers to reproduce.
    assert_indexable("List<Int>{len > 0}");
}

#[test]
fn a_refined_reference_to_a_list_is_indexable() {
    // The case that failed.
    assert_indexable("&List<Int>{len > 0}");
}

#[test]
fn tiered_refined_references_are_indexable() {
    for spelling in ["&checked List<Int>{len > 0}", "&unsafe List<Int>{len > 0}"] {
        assert_indexable(spelling);
    }
}

/// The negative pole: peeling refinements must not turn everything into
/// something indexable. A refined Int has no elements, and saying so is
/// the whole value of the check.
#[test]
fn a_refined_scalar_is_still_not_indexable() {
    let found = indexes_through("&Int{it > 0}");
    assert!(
        found.iter().any(|m| m.contains("index") || m.contains("Index")),
        "`&Int{{it > 0}}` has no elements — indexing it must still be \
         rejected, otherwise the peel has simply disabled the check; \
         got {found:?}"
    );
}
