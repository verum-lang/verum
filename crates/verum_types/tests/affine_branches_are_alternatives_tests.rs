//! The arms of a `match` and the branches of an `if` are ALTERNATIVES.
//!
//! An affine value may be consumed in each of them, because exactly one
//! runs — the commit-or-rollback shape every affine resource appears in.
//! It must still be refused when consumed in ONE arm and used after, and a
//! `linear` value must still be required on EVERY path.
//!
//! The three questions are answered by two different lattices over the same
//! bindings (`AffineTracker::merge_alternatives`), which is why they are
//! pinned together: a fix that satisfies one by collapsing the two into a
//! single bit breaks another.
//!
//! Spec: L0-critical/memory-safety/affine_consumed_once_per_exclusive_arm.vr
//! and its three siblings.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

/// Type-check a whole module and return every complaint, from both the
/// `Err` channel and the diagnostics channel (STMT-RECOVERY-1: a checker
/// may push a diagnostic and keep going, so an `Err`-only read is a stale
/// pin).
fn complaints(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse");
    let mut checker = TypeChecker::new();

    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            let _ = checker.register_type_declaration(type_decl);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }

    let mut out: Vec<String> = Vec::new();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            out.push(format!("{}", e));
        }
    }
    for d in checker.diagnostics().iter() {
        out.push(format!("{:?}", d));
    }
    out
}

const DECLS: &str = r#"
type affine Tx is { id: Int };

fn commit(t: Tx) -> Int { t.id }
fn rollback(t: Tx) -> Int { 0 - t.id }
"#;

fn moved(cs: &[String]) -> bool {
    cs.iter()
        .any(|c| c.contains("used after") || c.contains("moved"))
}

#[test]
fn match_arms_may_each_consume_the_same_affine_value() {
    let code = format!(
        "{DECLS}
fn finish(t: Tx, ok: Bool) -> Int {{
    match ok {{
        true  => commit(t),
        false => rollback(t),
    }}
}}
"
    );
    let cs = complaints(&code);
    assert!(
        !moved(&cs),
        "exactly one arm runs, so `t` is consumed exactly once: {cs:?}"
    );
}

#[test]
fn if_branches_may_each_consume_the_same_affine_value() {
    let code = format!(
        "{DECLS}
fn finish(t: Tx, ok: Bool) -> Int {{
    if ok {{ commit(t) }} else {{ rollback(t) }}
}}
"
    );
    let cs = complaints(&code);
    assert!(
        !moved(&cs),
        "exactly one branch runs, so `t` is consumed exactly once: {cs:?}"
    );
}

#[test]
fn consuming_in_one_arm_then_using_after_is_still_refused() {
    // The pole that keeps the fix from being a hole: per-branch restoration
    // WITHOUT the union merge turns this program green and introduces a
    // double free.
    let code = format!(
        "{DECLS}
fn twice(t: Tx, ok: Bool) -> Int {{
    let first = match ok {{
        true  => commit(t),
        false => 0,
    }};
    first + commit(t)
}}
"
    );
    let cs = complaints(&code);
    assert!(
        moved(&cs),
        "one execution reaches both consumptions, so this must be refused: {cs:?}"
    );
}

#[test]
fn a_branch_that_cannot_fall_through_does_not_reach_the_continuation() {
    // `bail` consumes and RETURNS, so no execution arrives at the later use
    // with `t` already gone. Merging a branch that cannot fall through would
    // refuse this correct program.
    let code = format!(
        "{DECLS}
fn guard(t: Tx, bail: Bool) -> Int {{
    if bail {{
        return commit(t);
    }}
    commit(t) - 2
}}
"
    );
    let cs = complaints(&code);
    assert!(
        !moved(&cs),
        "the consuming branch returns, so the later use is on a disjoint path: {cs:?}"
    );
}

#[test]
fn a_linear_value_must_be_consumed_on_every_path() {
    // The INTERSECTION lattice. Answering this question with the union bit
    // accepts the leak whenever `ok` is false.
    // The obligation is on a `let`-bound local: a parameter arrives
    // already moved in, and its scope end is its destruction site.
    let code = r#"
type linear Handle is { fd: Int };

fn close(h: Handle) -> Int { h.fd }

fn maybe_close(ok: Bool) -> Int {
    let h = Handle { fd: 3 };
    match ok {
        true  => close(h),
        false => 0,
    }
}
"#;
    let cs = complaints(code);
    assert!(
        cs.iter().any(|c| c.contains("exactly once")),
        "the `false` path leaves the obligation unmet: {cs:?}"
    );
}

#[test]
fn a_linear_value_consumed_in_every_arm_is_satisfied() {
    let code = r#"
type linear Handle is { fd: Int };

fn close(h: Handle) -> Int { h.fd }

fn always_close(ok: Bool) -> Int {
    let h = Handle { fd: 3 };
    match ok {
        true  => close(h),
        false => close(h),
    }
}
"#;
    let cs = complaints(code);
    assert!(
        !cs.iter().any(|c| c.contains("exactly once")),
        "every path consumes it exactly once: {cs:?}"
    );
}

#[test]
fn a_return_is_a_scope_end_for_a_linear_obligation() {
    // The hole the branch merge opens if only the end of the body is
    // checked: the leaving branch is (correctly) excluded from the merge
    // that forms the continuation, so by the end of the function the
    // obligation looks met on the only path that survived. Excluded from
    // the CONTINUATION is right; excluded from the CHECK is not.
    let code = r#"
type linear Session is { id: Int };

fn open() -> Session { Session { id: 1 } }
fn close(s: Session) -> Int { s.id }

fn work(fail: Bool) -> Int {
    let s = open();
    if fail {
        return 0;
    }
    close(s)
}
"#;
    let cs = complaints(code);
    assert!(
        cs.iter().any(|c| c.contains("exactly once")),
        "the early return leaves `s` unclosed on that path: {cs:?}"
    );
}

#[test]
fn a_return_that_discharges_the_obligation_is_accepted() {
    // The control. A rule that fired at every `return` would pass the
    // test above while refusing every correct early exit.
    let code = r#"
type linear Session is { id: Int };

fn open() -> Session { Session { id: 1 } }
fn close(s: Session) -> Int { s.id }

fn work(fail: Bool) -> Int {
    let s = open();
    if fail {
        return close(s);
    }
    close(s)
}
"#;
    let cs = complaints(code);
    assert!(
        !cs.iter().any(|c| c.contains("exactly once")),
        "both paths close the session exactly once: {cs:?}"
    );
}
