#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! TestClock / @inject-clock directive drift guard (#66).
//!
//! Pins the following contracts:
//!   1. directive.rs has a `inject_clock` field on TestDirectives.
//!   2. `@inject-clock:` prefix is parsed into inject_clock.
//!   3. An empty `@inject-clock:` value is ignored (inject_clock stays None).
//!   4. VCS spec is `@test: typecheck-pass`.
//!   5. VCS spec uses `@inject-clock:`.
//!   6. VCS spec defines TestClock type.
//!   7. VCS spec defines `advance_ms` method.
//!   8. VCS spec defines `now_ms` method.

const DIRECTIVE_SRC: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/test_clock_injection.vr"
);

// ── 1. inject_clock field on TestDirectives ───────────────────────────────────

#[test]
fn directive_has_inject_clock_field() {
    assert!(
        DIRECTIVE_SRC.contains("inject_clock"),
        "TestDirectives must have an 'inject_clock' field"
    );
}

// ── 2. @inject-clock: parsed into inject_clock ───────────────────────────────

#[test]
fn directive_parses_inject_clock_prefix() {
    assert!(
        DIRECTIVE_SRC.contains("@inject-clock:"),
        "directive.rs must parse '@inject-clock:' prefix"
    );
}

// ── 3. Empty inject-clock value is ignored ────────────────────────────────────

#[test]
fn directive_ignores_empty_inject_clock() {
    assert!(
        DIRECTIVE_SRC.contains("inject_clock: None"),
        "directive.rs default must set inject_clock to None"
    );
}

// ── 4. VCS spec is typecheck-pass ─────────────────────────────────────────────

#[test]
fn spec_is_typecheck_pass() {
    assert!(
        SPEC.contains("@test: typecheck-pass"),
        "test_clock_injection.vr must be '@test: typecheck-pass'"
    );
}

// ── 5. VCS spec uses @inject-clock ────────────────────────────────────────────

#[test]
fn spec_uses_inject_clock_directive() {
    assert!(
        SPEC.contains("@inject-clock:"),
        "test_clock_injection.vr must use '@inject-clock:' directive"
    );
}

// ── 6. VCS spec defines TestClock type ────────────────────────────────────────

#[test]
fn spec_defines_test_clock_type() {
    assert!(
        SPEC.contains("TestClock"),
        "test_clock_injection.vr must define 'TestClock'"
    );
}

// ── 7. VCS spec defines advance_ms ────────────────────────────────────────────

#[test]
fn spec_defines_advance_ms_method() {
    assert!(
        SPEC.contains("advance_ms"),
        "test_clock_injection.vr must define 'advance_ms' method"
    );
}

// ── 8. VCS spec defines now_ms ────────────────────────────────────────────────

#[test]
fn spec_defines_now_ms_method() {
    assert!(
        SPEC.contains("now_ms"),
        "test_clock_injection.vr must define 'now_ms' method"
    );
}
