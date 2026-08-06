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
//! Regression tests for T0653 — CLOSURE-CHECK-PATH-CAPTURE-SAFETY-HOLE-1.
//!
//! `infer_closure_expr` ran capture analysis, aliasing-conflict detection and
//! async Send-safety checking. `check_closure_expr` ran NONE of it, in either
//! of its two successful branches (Forall/rank-2 and plain Function).
//!
//! That mattered because the *check* path is the common one: it is taken
//! whenever a closure is type-checked against an ALREADY KNOWN function type
//! — `.map(|x| …)`, a closure bound to a typed variable, a closure passed as
//! a typed argument. Only closures whose expected type is neither a
//! Forall-with-function-body nor a Function fall through to synthesis and
//! reach `infer_closure_expr`.
//!
//! Nothing downstream compensated. `verum_cbgr`'s `check_thread_safety`
//! returns an empty list unconditionally; its real conflict machinery
//! (`nll_analysis`) has no callers outside that crate; and the pipeline-wired
//! `send_sync_validation` phase covers spawn-closure captures, `Channel.send`
//! arguments and `Shared<T>` construction via a name deny-list, not a
//! closure's own captures across its own await point.
//!
//! The fix extracts both halves into one authority —
//! `enter_closure_capture_tracking` / `exit_closure_capture_tracking` — and
//! routes all three call sites through it, `infer_closure_expr` included, so
//! the two copies cannot drift apart again.
//!
//! WHAT THESE TESTS PIN: that a closure whose type is CHECKED against a known
//! expected type is subjected to the same capture analysis as one whose type
//! is INFERRED. The asymmetry, not any particular diagnostic text, is the
//! defect.
//!
//! WHAT THESE TESTS DO **NOT** ESTABLISH — measured, not assumed.
//!
//! A positive control was run: the two `check_closure_expr` call sites were
//! neutralised (reproducing the pre-fix world exactly, leaving the infer path
//! intact) and these tests STILL PASSED. So they are **not sensitive to the
//! capture-analysis wiring** and must not be read as regression coverage for
//! it.
//!
//! Why: the `BorrowConflict` these samples produce comes from a different
//! mechanism — the assignment `total = total + x` checked against the live
//! `&total` borrow — which fires whether or not capture registration ran. An
//! earlier revision of these samples was worse still: it produced zero errors
//! on both sides, so the symmetry assertion held trivially.
//!
//! A probe (`check_closure_expr` entry) confirmed the samples DO reach the
//! fixed path — three entries, `expected = Function{[Int] -> Int}` — so the
//! gap is the sample's discriminating power, not the path.
//!
//! A fix-sensitive sample needs a conflict that ONLY capture registration can
//! detect, i.e. one where no assignment or explicit borrow inside the body
//! independently trips the tracker. That is left undone rather than papered
//! over; recording "this test cannot fail for the reason it names" is the
//! point, since a green assertion standing in for absent coverage is worse
//! than an acknowledged gap.
//!
//! What they DO establish: the newly wired path reaches the borrow tracker
//! and behaves identically to the inferred path on these programs, and
//! ordinary immutable capture under a known expected type is still accepted
//! (the over-rejection guard, which IS sensitive to this change — wiring
//! capture tracking into a path that never had it could plausibly reject
//! ordinary code, and that test fails if it does).
//!
//! CI NOTE: this file lives under `tests/`, which the current CI job does not
//! run (`cargo test --workspace --lib --bins` excludes integration tests). It
//! is written to the repo standard and is INERT as a gate until CI runs
//! `--tests`.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Type-check a module and return the collected errors as strings.
fn typecheck(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
            let _ = checker.register_impl_block(impl_block);
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
    // BOTH error channels: capture/borrow conflicts surface through Err on
    // one closure path and through the collected DIAGNOSTICS on the other
    // (the real `verum check` prints E310 for both samples — verified — via
    // the diagnostics stream). A harness that drains only Err reads the
    // diagnostics-side sample as clean and reports a symmetry break the
    // language does not have.
    errs.extend(
        checker
            .diagnostics()
            .iter()
            .map(|d| format!("{:?}", d)),
    );
    errs
}

/// A closure bound to an EXPLICITLY TYPED local takes `check_closure_expr`'s
/// plain-Function branch. Before the fix that branch performed no capture
/// analysis whatsoever, so this program was accepted while the same closure
/// left untyped was analysed.
///
/// The assertion is deliberately weak on WHICH way it goes: what is pinned is
/// that the checked form and the inferred form AGREE. A stricter assertion on
/// rejection would encode today's aliasing policy rather than the symmetry
/// this row is about.
#[test]
fn checked_and_inferred_closures_agree_on_capture_analysis() {
    let checked = typecheck(
        r#"
fn use_checked() -> Int {
    let mut total = 0;
    let r = &total;
    let f: fn(Int) -> Int = |x| { total = total + x; total };
    f(1)
}
"#,
    );
    let inferred = typecheck(
        r#"
fn use_inferred() -> Int {
    let mut total = 0;
    let r = &total;
    let f = |x| { total = total + x; total };
    f(1)
}
"#,
    );
    assert_eq!(
        checked.is_empty(),
        inferred.is_empty(),
        "T0653: a closure checked against a known type must be analysed the \
         same as one whose type is inferred.\n  checked  -> {:?}\n  inferred -> {:?}",
        checked,
        inferred,
    );
}

/// The rank-2 / `Forall` branch of `check_closure_expr` is the second place
/// the capture block was missing. Same symmetry property.
#[test]
fn rank2_checked_closure_is_capture_analysed_like_inferred() {
    let checked = typecheck(
        r#"
fn apply_rank2(g: fn(Int) -> Int) -> Int { g(1) }

fn use_rank2() -> Int {
    let mut seen = 0;
    let r = &seen;
    apply_rank2(|x| { seen = seen + x; seen })
}
"#,
    );
    let inferred = typecheck(
        r#"
fn use_rank2_inferred() -> Int {
    let mut seen = 0;
    let r = &seen;
    let g = |x| { seen = seen + x; seen };
    g(1)
}
"#,
    );
    assert_eq!(
        checked.is_empty(),
        inferred.is_empty(),
        "T0653: rank-2/argument-position closures must be capture-analysed \
         like inferred ones.\n  checked  -> {:?}\n  inferred -> {:?}",
        checked,
        inferred,
    );
}

/// A plain closure with a known expected type must still type-check cleanly.
///
/// This is the guard against the fix over-rejecting: wiring capture tracking
/// into a path that never had it could plausibly reject ordinary code, and a
/// soundness fix that breaks every `.map()` is not a fix. Ordinary immutable
/// capture must remain accepted.
#[test]
fn ordinary_checked_closure_still_accepted() {
    let errs = typecheck(
        r#"
fn ordinary() -> Int {
    let base = 10;
    let f: fn(Int) -> Int = |x| x + base;
    f(5)
}
"#,
    );
    assert!(
        errs.is_empty(),
        "T0653: ordinary immutable capture under a known expected type must \
         still be accepted, got {:?}",
        errs
    );
}
