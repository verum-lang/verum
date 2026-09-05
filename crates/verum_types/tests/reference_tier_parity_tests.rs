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
//! Regression tests for A90 — the tier-1 reference arms kept their own copy
//! of the borrow-tracking rule and drifted from tier 0.
//!
//! `&checked T` is `&T` with the RUNTIME check elided. Its STATIC discipline
//! is meant to be identical: tier 1 is the compiler-PROVEN tier, so it may be
//! neither stricter nor looser than the default it optimises into. It was
//! both. `infer_unop`'s four reference arms each carried their own
//! `match &expr.kind` block — 90, 60, 9 and 13 lines — and the two tier-1
//! copies had fallen behind:
//!
//! * they matched only `ExprKind::Path`, so `&checked mut s.a` and
//!   `&checked mut xs[0]` registered NOTHING and tier 1 ACCEPTED a
//!   whole-value `&mut s` that tier 0 refuses (the UNSOUND direction);
//! * neither knew NLL-ARG-BORROW-1 — that a borrow taken as a call argument
//!   dies when the call returns — so a `&checked` argument stayed borrowed
//!   and the next use of the variable was refused with E310.
//!
//! The last one is what made the row P1: `&checked` is the zero-cost tier the
//! documentation recommends for hot paths, and a function taking one could be
//! called but its argument was unusable afterwards. Found in
//! `crates/verum_cli/examples/cbgr_references.vr`, the shipped example whose
//! subject is exactly this.
//!
//! WHAT THESE TESTS PIN: that the two spellings AGREE, not what either one
//! decides. Asserting "accepted" or "refused" per shape would freeze today's
//! aliasing policy; asserting agreement pins the tier invariant, which is the
//! property the three-tier model actually promises.
//!
//! FIX-SENSITIVITY, MEASURED — not assumed. The two tier-1 arms were
//! reverted to their own Path-only blocks and the samples re-run. Pre-fix,
//! FOUR of the five shapes diverge (tier-0 count vs tier-1 count):
//!
//! ```text
//!   immut-through-call   0  1     tier 1 refuses what tier 0 allows
//!   mut-through-call     0  1     the A90 subject
//!   whole-after-field    1  0     tier 1 ALLOWS what tier 0 refuses
//!   whole-after-index    1  0     tier 1 ALLOWS what tier 0 refuses
//!   held-then-read       1  1     agree — the negative control
//! ```
//!
//! With the fix, all five read the same on both sides. `held-then-read` is
//! the control that catches a "fix" which merely stops tracking tier-1
//! borrows: it must stay REFUSED at both tiers.
//!
//! TWO EARLIER REVISIONS OF THIS FILE WERE INERT, and both passed the
//! positive control — recorded here because the failure mode is not visible
//! from a green run:
//!
//! * the samples used `List`, `print` and f-strings. The bare `TypeChecker`
//!   has no stdlib, so every one reported `TypeNotFound: List` and E100
//!   (undefined variable) — and the assertion compared `is_empty()`, so both
//!   tiers were non-empty for reasons having nothing to do with borrows.
//!   Hence: stdlib-free samples, and a harness that filters to the borrow
//!   diagnostics.
//! * the read-back was `let r = n;`, which is a USE, not a borrow, and
//!   tripped nothing at either tier — including in the negative control.
//!   Hence: `let r = &n;`.
//!
//! Gated by CI's `integration` job (`cargo test -p verum_types --tests`).

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Type-check a module and return the collected errors as strings.
///
/// BOTH error channels are drained: a borrow conflict surfaces through `Err`
/// on one path and through the collected DIAGNOSTICS on another, and a
/// harness reading only one of them reports agreement the language does not
/// have.
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
    // ONLY the borrow verdict. An earlier revision of this file compared
    // "did anything go wrong at all" and was measured INERT: the bare
    // `TypeChecker` has no stdlib, so every sample reported
    // `TypeNotFound: List` plus E100 (undefined variable) for `print`, and
    // both tiers were non-empty for reasons having nothing to do with
    // borrows. The samples below are stdlib-free for the same reason.
    errs.retain(|e| e.contains("E310") || e.contains("BorrowConflict"));
    errs
}

/// Write one program at both tiers and require the same verdict.
///
/// `program` carries two placeholders so the two spellings differ by nothing
/// else: `REFMUT` becomes `&mut` / `&checked mut`, `REF` becomes `&` /
/// `&checked `. Substituting REFMUT first keeps `REF` from eating its prefix.
fn assert_tiers_agree(shape: &str, program: &str) {
    let tier0 = program.replace("REFMUT", "&mut").replace("REF", "&");
    let tier1 = program
        .replace("REFMUT", "&checked mut")
        .replace("REF", "&checked ");
    assert!(!tier0.contains("REF") && !tier1.contains("REF"));

    let e0 = typecheck(&tier0);
    let e1 = typecheck(&tier1);
    assert_eq!(
        e0.len(),
        e1.len(),
        "tier divergence on `{shape}`: tier 0 gave {} borrow error(s), tier 1 gave {}.\n\
         `&checked` is `&` with the runtime check elided — the static verdict \
         must be the same.\n  tier0: {e0:?}\n  tier1: {e1:?}",
        e0.len(),
        e1.len(),
    );
}

/// An immutable borrow taken as a CALL ARGUMENT dies when the call returns
/// (NLL-ARG-BORROW-1). Tier 1 registered it persistently, so the later
/// `&mut n` phantom-conflicted with a borrow no longer held.
#[test]
fn immutable_borrow_through_a_call_agrees_across_tiers() {
    assert_tiers_agree(
        "immut-through-call",
        r#"
fn take(x: REF Int) { }

fn main() {
    let mut n = 1;
    take(REF n);
    let m = &mut n;
}
"#,
    );
}

/// A90's own reproducer: `&checked mut n` passed to a function, then `n`
/// read. One token apart from a program that compiles.
#[test]
fn mutable_borrow_through_a_call_agrees_across_tiers() {
    assert_tiers_agree(
        "mut-through-call",
        r#"
fn take(x: REFMUT Int) { }

fn main() {
    let mut n = 1;
    take(REFMUT n);
    let r = &n;
}
"#,
    );
}

/// A borrow of ONE field implicitly borrows the whole value, which is what
/// forbids a later `&mut s`. The tier-1 arm never matched `ExprKind::Field`,
/// so it registered nothing and accepted the conflict.
#[test]
fn field_borrow_blocks_a_whole_value_borrow_at_both_tiers() {
    assert_tiers_agree(
        "whole-after-field",
        r#"
type S is { a: Int, b: Int };

fn main() {
    let mut s = S { a: 1, b: 2 };
    let held = REFMUT s.a;
    let whole = &mut s;
}
"#,
    );
}

/// Same rule one shape over: a constant-index element borrow is tracked like
/// a field borrow, and blocks a borrow of the whole collection.
#[test]
fn index_borrow_blocks_a_whole_collection_borrow_at_both_tiers() {
    assert_tiers_agree(
        "whole-after-index",
        r#"
fn main() {
    let mut xs: [Int; 3] = [1, 2, 3];
    let held = REFMUT xs[0];
    let whole = &mut xs;
}
"#,
    );
}

/// NEGATIVE CONTROL. A borrow that is genuinely still HELD must keep
/// refusing the read — at both tiers. Without this, "make tier 1 agree with
/// tier 0" is satisfiable by tracking nothing at all.
#[test]
fn a_held_mutable_borrow_still_refuses_a_read_at_both_tiers() {
    let program = r#"
fn main() {
    let mut n = 1;
    let held = REFMUT n;
    let r = &n;
}
"#;
    assert_tiers_agree("held-then-read", program);

    for (tier, spelling) in [("tier 0", "&mut"), ("tier 1", "&checked mut")] {
        let errs = typecheck(&program.replace("REFMUT", spelling));
        assert!(
            !errs.is_empty(),
            "{tier}: a HELD mutable borrow must still refuse the read — \
             agreement reached by tracking nothing is not the fix"
        );
    }
}
