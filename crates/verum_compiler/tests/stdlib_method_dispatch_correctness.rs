//! Method-dispatch receiver-type-narrowing guardrail (#169 / method-dispatch narrowing).
//!
//! Multiple stdlib types declare a method with the same simple name —
//! most notably `unwrap`, declared on Result, Maybe, Poll, and (historic)
//! generic-T paths.  The codegen in
//! `crates/verum_vbc/src/codegen/expressions.rs::compile_method_call`
//! resolves the call by first building an `effective_method_name` from
//! the receiver expression's shape; before method-dispatch narrowing the catch-all fell
//! through to bare `unwrap` whenever the expression shape wasn't
//! covered, and the dispatcher picked WHICHEVER candidate was
//! registered first by name+arity.
//!
//! For `Result<Int, Int>::Err(99).unwrap()` that frequently resolved to
//! a candidate whose `unwrap` does NOT panic on Err — the call returned
//! 99 silently and downstream code saw a malformed value, surfacing
//! later as `field index N (offset M) exceeds object data size K` or
//! `null pointer dereference` far from the cause.
//!
//! method-dispatch narrowing added a safety net: after the giant match, if the
//! resulting `effective_method_name` is still bare (no `.`, no `$`),
//! retry via `infer_expr_type_name(receiver)` — a generic best-effort
//! type-inference helper — and prefix as `Type.method` when the
//! inference succeeds with an Uppercase-typed base name.
//!
//! This test pins the contract: the `unwrap` family on each of its
//! stdlib types panics correctly on the receiver-type's bottom
//! element.  A regression that re-introduces the silent fallback fails
//! one of these fixtures, and the failure message points at
//! `compile_method_call` directly.
//!
//! Each fixture lives under
//! `vcs/specs/L0-critical/_codegen_regressions/` with the
//! `@test: run-interpreter-panic` directive and an `@expected-panic`
//! substring assertion. The vtest binary itself does the panic-message
//! matching; this guardrail just runs vtest and asserts the spec
//! passed.
//!
//! Adding a new method-dispatch contract:
//!
//!   1. Build a fixture under `vcs/specs/L0-critical/_codegen_regressions/`
//!      that exercises the receiver type's bottom-element method call
//!      and uses `@expected-panic: <substring>`.
//!   2. Add a four-line test below: `assert_dispatch_panic(&fixture)`.

mod stdlib_support;

use stdlib_support::{vtest_run_capture, workspace_root};

/// Run the given fixture spec via `vtest` and assert it produced
/// `RESULT: PASSED` — meaning the `@test: run-interpreter-panic`
/// directive was satisfied (panic fired AND message matched the
/// `@expected-panic` substring).
///
/// `scenario` names the fixture in the failure message so a CI diff
/// identifies which dispatch contract regressed.
fn assert_dispatch_panic(scenario: &str, fixture: &std::path::Path) {
    if !fixture.is_file() {
        panic!(
            "method-dispatch fixture missing at {} — was it moved or \
             deleted without updating this guardrail? Restore the \
             fixture or update the test.",
            fixture.display(),
        );
    }
    let out = vtest_run_capture(fixture);
    let merged: Vec<&str> = out.merged_lines().collect();
    let passed = merged.iter().any(|l| l.contains("RESULT: PASSED"));
    assert!(
        passed,
        "method-dispatch contract `{}` regressed.  vtest did not \
         produce `RESULT: PASSED` on {}.\n\n\
         This means `compile_method_call` in \
         `crates/verum_vbc/src/codegen/expressions.rs` no longer \
         narrows the receiver type before resolving `unwrap` (or the \
         analogous method) — the dispatcher picked a candidate from \
         a different type's variant table, and the body that should \
         have panicked silently returned a malformed value or panicked \
         with the wrong message.\n\n\
         Check method-dispatch narrowing fix: the safety net after the giant \
         expression-shape match that retries via \
         `infer_expr_type_name(receiver)` and prefixes as \
         `Type.method` when inference succeeds.\n\n\
         exit code: {:?}\n\n\
         tail of vtest output (stdout + stderr merged):\n{}",
        scenario,
        fixture.display(),
        out.exit_code,
        merged
            .iter()
            .rev()
            .take(20)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// `Result<T, E>::Err(_).unwrap()` must panic with a message that
/// identifies the receiver as a `Result` (i.e. mentions `Err` in the
/// message — the canonical phrasing is "called `unwrap` on an Err
/// value").  The `@expected-panic` directive in the fixture verifies
/// the message; this guardrail verifies the test passed.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn result_unwrap_on_err_panics() {
    let fixture = workspace_root().join(
        "vcs/specs/L0-critical/_codegen_regressions/result_unwrap_err_receiver_narrow.vr",
    );
    assert_dispatch_panic("Result.unwrap on Err", &fixture);
}

/// `Maybe<T>::None.unwrap()` must panic with a message that identifies
/// the receiver as a `Maybe` (canonical phrasing "called `unwrap` on
/// a None value").  The pair of this test + the Result test exercises
/// the receiver-narrowing's discrimination between the two siblings.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn maybe_unwrap_on_none_panics() {
    let fixture = workspace_root().join(
        "vcs/specs/L0-critical/_codegen_regressions/maybe_unwrap_none_panics.vr",
    );
    assert_dispatch_panic("Maybe.unwrap on None", &fixture);
}
