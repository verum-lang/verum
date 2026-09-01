//! The ambient bare-name constructor table is the OUTERMOST binding.
//!
//! `variant_constructor_parents` is flat and bare-keyed: every sum type
//! in the loaded stdlib publishes each of its variant spellings into it
//! with no regard for the call site's scope.  It is therefore a
//! prelude, and must lose to every explicit binding —
//! `try_resolve_variant_constructor_with_arity_for` says so in its own
//! T0525 comment.
//!
//! T0704 enforced that for an explicitly MOUNTED function.  The sibling
//! case — a function declared in the very file being checked — was not
//! covered, and the Call fast path in `infer/expr.rs` took the
//! constructor unconditionally whenever the bare name appeared in the
//! table, then died on the arity it could not satisfy:
//!
//! ```text
//! error: push expects 1 argument(s), got 2
//! ```
//!
//! `core/math/hott.vr` declares `Pushout`'s `push(c: C)`, so eight
//! `core/database/**` files that spell their own two-argument `push`
//! helper lost it to a constructor from the homotopy-type-theory
//! module — 117 messages, the largest single homogeneous class in the
//! `core/` corpus (T1034).  T0704's own comment names the same file and
//! the same type for the sibling constructor `inr`; only the mounted
//! leg was repaired then.
//!
//! The repair is confined to the arity-MISMATCH leg, which previously
//! always returned `Err`.  Both directions are pinned below: the local
//! declaration must win, and a genuine arity error with no local
//! declaration must still be reported.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Register the module's declarations in the pipeline's order, then
/// check every item and return the errors as strings.
fn check_errors(code: &str) -> Vec<String> {
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
    module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect()
}

/// The declaration in this very file wins over the ambient constructor.
#[test]
fn a_local_function_beats_a_same_named_ambient_constructor() {
    let errs = check_errors(
        r#"
type Pushout is inl(Text) | inr(Text) | push(Text);

fn push(dst: Text, s: Text) -> Text { dst }

fn go() -> Text {
    push("a", "b")
}
"#,
    );
    assert!(
        !errs.iter().any(|e| e.contains("push expects")),
        "the file's own two-argument `push` must answer the call, \
         not `Pushout.push`: {:?}",
        errs
    );
}

/// The control that keeps the repair from being a disabled arity check:
/// with NO local declaration, over-applying the constructor is still an
/// error.  Without this the test above passes for the wrong reason.
#[test]
fn without_a_local_declaration_the_arity_error_still_fires() {
    let errs = check_errors(
        r#"
type Pushout is inl(Text) | inr(Text) | push(Text);

fn go() -> Pushout {
    push("a", "b")
}
"#,
    );
    assert!(
        errs.iter().any(|e| e.contains("push expects")),
        "an over-applied constructor with no local binding to prefer \
         must still be reported: {:?}",
        errs
    );
}

/// The renamed twin of the first case.  It passed before the repair
/// too — its job is to show that the first test's subject differs from
/// this one ONLY in the name, so a failure there is about the
/// collision and nothing else.
#[test]
fn the_renamed_twin_was_never_affected() {
    let errs = check_errors(
        r#"
type Pushout is inl(Text) | inr(Text) | push(Text);

fn pushx(dst: Text, s: Text) -> Text { dst }

fn go() -> Text {
    pushx("a", "b")
}
"#,
    );
    assert!(
        !errs.iter().any(|e| e.contains("expects")),
        "the non-colliding name must check cleanly: {:?}",
        errs
    );
}
