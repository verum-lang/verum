//! Whether `*x` is legal must not depend on the field's WIDTH (T1338).
//!
//! The `Deref` arm of `infer_unary` dispatches on the `Type` VARIANT, and
//! its bottom `_ =>` arm grants transparent deref — "deref on value types
//! is identity", the CBGR model's own rule. `Bool`, `Int`, `Float`, `Char`
//! and `Text` each own a `Type` variant and reach it. Every sized integer
//! is carried as `Type::Named { path, args: [] }`, hit the `Named` arm
//! instead, and was refused:
//!
//! ```text
//! error<E409>: Cannot dereference non-reference type: UInt64
//! ```
//!
//! Measured before the fix, one shape, only the field type varying:
//!
//! ```text
//!   Int v=1   Float v=1.0   Bool v=true   Text v=a
//!   Int8  Int32  Int64  UInt8  UInt32  UInt64   ->  E409
//! ```
//!
//! Same declaration, same operator, same scrutinee — the verdict followed
//! the declared width, which is the representation answering a language
//! question. `core/net/quic/path.vr:145` (`(*sent_bytes)`, a `UInt64`
//! field) depends on the accepting answer and compiles in-tree only
//! because `core/` is checked in the lenient `stdlib_single_file_mode`;
//! under `VERUM_STRICT_STDLIB=1` the same file reports E409 at that line.
//!
//! The last test is the control that keeps E409 meaningful: a `Named`
//! type that is NOT a primitive value must still be refused. A fix that
//! silenced the diagnostic everywhere would pass the sweep and be wrong.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// The checker must be SEEDED, and the first version of this file was
/// not. A bare `TypeChecker::new()` knows `Int`, `Float`, `Bool` and
/// `Text` — they own `Type` variants — but not `Int8`…`UInt64`, which
/// are ordinary named types it has never been told about. Every sized
/// integer therefore failed with
///
/// ```text
/// TypeNotFound { name: "UInt64", … }
/// ```
///
/// BEFORE the deref rule ran at all, and an assertion that filtered
/// errors for the substring "Cannot dereference" saw none — so the test
/// passed on the fixed tree AND on the unfixed one. Measured, both
/// polarities, which is the only reason it was caught. See
/// `phases_orchestration.rs`: the production path builds the checker
/// with `with_minimal_context()` and then `register_builtins()`.
fn check_errors(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
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
    module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect()
}

/// `(*x)` on a binding taken through a `&` scrutinee, one field type per
/// call.
fn deref_a_field_of_type(ty: &str, lit: &str) -> Vec<String> {
    check_errors(&format!(
        r#"
type Sum is Pair {{ x: {ty} }} | Nil;

fn read(s: &Sum) -> {ty} {{
    match s {{
        Sum.Pair {{ x }} => (*x),
        Sum.Nil => {lit},
    }}
}}
"#
    ))
}

const CASES: &[(&str, &str)] = &[
    ("Int", "1"),
    ("Int8", "1_i8"),
    ("Int32", "1_i32"),
    ("Int64", "1_i64"),
    ("UInt8", "1_u8"),
    ("UInt32", "1_u32"),
    ("UInt64", "1_u64"),
    ("Float", "1.0"),
    ("Bool", "true"),
    ("Text", "\"a\""),
];

#[test]
fn every_primitive_field_type_gets_the_same_answer() {
    // ANY error fails this, not just a deref one. Filtering by substring
    // is what made the first version of this test vacuous: when the
    // subject never ran, the filter matched nothing and the absence read
    // as success. A clean program must type-check CLEANLY; if a case
    // starts failing for an unrelated reason, that is a broken harness
    // and it must say so rather than quietly stop measuring.
    let mut failed: Vec<(&str, Vec<String>)> = Vec::new();
    for (ty, lit) in CASES {
        let errs = deref_a_field_of_type(ty, lit);
        if !errs.is_empty() {
            failed.push((*ty, errs));
        }
    }
    assert!(
        failed.is_empty(),
        "`(*x)` on a binding taken through a `&` scrutinee must type-check \
         for every primitive field type. Failing: {:#?}\n\nA \
         `Cannot dereference` here means the rule is reading the field's \
         declared WIDTH — types with their own `Type` variant reach the \
         transparent-deref arm and sized integers, carried as \
         `Type::Named`, do not. Any OTHER error here means this harness \
         is no longer exercising the deref rule.",
        failed
    );
}

/// The control. E409 exists to catch a real mistake and must keep
/// catching it.
#[test]
fn a_non_primitive_named_type_is_still_refused() {
    let errs = check_errors(
        r#"
type Thing is { a: Int };
type Sum is Pair { t: Thing } | Nil;

fn read(s: &Sum) -> Int {
    match s {
        Sum.Pair { t } => (*t).a,
        Sum.Nil => 0,
    }
}
"#,
    );
    assert!(
        errs.iter().any(|e| e.contains("Cannot dereference")),
        "dereferencing a `Thing` — a named type that is not a primitive \
         value — must still be E409; if this stops firing the fix has \
         silenced the diagnostic instead of correcting who it applies to. \
         Errors seen: {:?}",
        errs
    );
}
