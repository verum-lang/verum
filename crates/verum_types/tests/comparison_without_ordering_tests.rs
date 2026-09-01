//! `<` on a type that declares no ordering compiles to a POINTER
//! comparison, so the answer is where the values happen to be allocated:
//! consistent inside a process, inverted between processes (T1017).
//!
//! W0506 says so. Its FIRST version also said so about `UInt64`, `Int32`,
//! `USize` and every other sized numeric alias — 140 times over 200 core/
//! files, all of them wrong. A sized alias is a `Type::Named`, not
//! `Type::Int`, and the runtime orders it natively; a ten-run probe on
//! `UInt64 < UInt64` gives the right answer ten times out of ten.
//!
//! The warning was reporting the CHECKER's gap as the author's mistake —
//! and the 140 read as confirmation that the check worked. **Many identical
//! findings in one place mean ONE cause, not many.**
//!
//! What was missing was not the check but its second half: there was a
//! control for FIRING and none for STAYING SILENT. A detector that shouts
//! at everything is indistinguishable from a working one.
//!
//! Task: T1017 / T1046.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

fn warnings(code: &str) -> String {
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
    let mut out = String::new();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            out.push_str(&format!("{:?} ", e.to_diagnostic().code()));
        }
    }
    for d in checker.diagnostics().iter() {
        out.push_str(&format!("{:?} ", d.code()));
    }
    out
}

/// FIRES: a record with no ordering, compared. This is the real defect.
#[test]
fn a_record_with_no_ordering_is_warned_about() {
    let w = warnings(
        "\
type Id is { v: Int };
fn f() -> Bool {
    let a = Id { v: 1 };
    let b = Id { v: 2 };
    a < b
}
",
    );
    assert!(
        w.contains("W0506"),
        "comparing a record with no ordering compiles to a pointer \
         comparison and must be warned about; got: {w}"
    );
}

/// STAYS SILENT: sized numeric aliases. This is the half that was missing,
/// and its absence let 140 false positives ship.
#[test]
fn sized_numeric_aliases_are_not_warned_about() {
    for ty in [
        "UInt64", "Int32", "UInt32", "USize", "UInt16", "UInt8", "Int16", "Int64", "ISize",
        "Float32", "Float64", "Byte",
    ] {
        let w = warnings(&format!(
            "fn f() -> Bool {{ let a: {ty} = 1; let b: {ty} = 2; a < b }}\n"
        ));
        assert!(
            !w.contains("W0506"),
            "`{ty}` is ordered natively by the runtime — warning about it \
             reports the checker's own gap as the author's mistake; got: {w}"
        );
    }
}

/// STAYS SILENT: a type that DOES declare an ordering.
#[test]
fn a_type_with_ord_is_not_warned_about() {
    let w = warnings(
        "\
type Id is { v: Int };
implement Ord for Id {
    fn cmp(&self, other: &Id) -> Ordering { self.v.cmp(&other.v) }
}
fn f() -> Bool {
    let a = Id { v: 1 };
    let b = Id { v: 2 };
    a < b
}
",
    );
    assert!(
        !w.contains("W0506"),
        "a declared ordering is exactly what the warning asks for; got: {w}"
    );
}
