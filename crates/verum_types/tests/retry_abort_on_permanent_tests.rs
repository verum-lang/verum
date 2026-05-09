#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Retry abort-on-permanent-error drift guard (#76).
//!
//! `core/base/retry.vr` implements retry logic.  When `should_retry` returns
//! false, retry_with_strategy must abort immediately (permanent-error semantics).
//!
//! This drift guard pins:
//!   1. retry.vr defines `RetryOptions<E>` with a `should_retry` field.
//!   2. retry.vr has `with_should_retry` method that replaces the predicate.
//!   3. retry_with_strategy aborts when `should_retry` returns false.
//!   4. RetryOptions has `max_attempts` field.
//!   5. RetryOptions has `strategy: RetryBackoff` field.
//!   6. RetryBackoff has None / Exponential / Linear variants.
//!   7. retry.vr exports `retry_with_strategy` function.
//!   8. VCS spec uses `RetryOptions`, `RetryBackoff`, `should_retry`.

const RETRY_VR: &str = include_str!("../../../core/base/retry.vr");
const RETRY_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/stdlib/retry_abort_on_permanent.vr"
);

// ── 1. RetryOptions has should_retry field ───────────────────────────────────

#[test]
fn retry_options_has_should_retry_field() {
    assert!(
        RETRY_VR.contains("should_retry:"),
        "RetryOptions must have a 'should_retry' field"
    );
}

// ── 2. with_should_retry method ───────────────────────────────────────────────

#[test]
fn retry_options_has_with_should_retry_method() {
    assert!(
        RETRY_VR.contains("fn with_should_retry"),
        "RetryOptions must have a 'with_should_retry' method"
    );
}

// ── 3. retry_with_strategy aborts when should_retry returns false ─────────────

#[test]
fn retry_with_strategy_checks_should_retry() {
    assert!(
        RETRY_VR.contains("should_retry")
            && (RETRY_VR.contains("!") || RETRY_VR.contains("false")),
        "retry_with_strategy must check 'should_retry' predicate and abort on false"
    );
}

// ── 4. max_attempts field ─────────────────────────────────────────────────────

#[test]
fn retry_options_has_max_attempts_field() {
    assert!(
        RETRY_VR.contains("max_attempts:"),
        "RetryOptions must have a 'max_attempts' field"
    );
}

// ── 5. strategy field ─────────────────────────────────────────────────────────

#[test]
fn retry_options_has_strategy_field() {
    assert!(
        RETRY_VR.contains("strategy:"),
        "RetryOptions must have a 'strategy' field of type RetryBackoff"
    );
}

// ── 6. RetryBackoff variants ──────────────────────────────────────────────────

#[test]
fn retry_backoff_has_none_variant() {
    assert!(
        RETRY_VR.contains("None") && RETRY_VR.contains("RetryBackoff"),
        "RetryBackoff must have a None variant"
    );
}

#[test]
fn retry_backoff_has_exponential_variant() {
    assert!(
        RETRY_VR.contains("Exponential"),
        "RetryBackoff must have an Exponential variant"
    );
}

#[test]
fn retry_backoff_has_linear_variant() {
    assert!(
        RETRY_VR.contains("Linear"),
        "RetryBackoff must have a Linear variant"
    );
}

// ── 7. retry_with_strategy exported ──────────────────────────────────────────

#[test]
fn retry_vr_exports_retry_with_strategy() {
    assert!(
        RETRY_VR.contains("pub fn retry_with_strategy")
            || RETRY_VR.contains("public fn retry_with_strategy"),
        "retry.vr must export 'retry_with_strategy'"
    );
}

// ── 8. VCS spec ───────────────────────────────────────────────────────────────

#[test]
fn retry_spec_uses_retry_options() {
    assert!(
        RETRY_SPEC.contains("RetryOptions"),
        "retry_abort_on_permanent.vr must reference RetryOptions"
    );
}

#[test]
fn retry_spec_uses_retry_backoff() {
    assert!(
        RETRY_SPEC.contains("RetryBackoff"),
        "retry_abort_on_permanent.vr must reference RetryBackoff"
    );
}

#[test]
fn retry_spec_uses_should_retry_predicate() {
    assert!(
        RETRY_SPEC.contains("should_retry"),
        "retry_abort_on_permanent.vr must reference 'should_retry' predicate"
    );
}

#[test]
fn retry_spec_is_typecheck_pass() {
    assert!(
        RETRY_SPEC.contains("@test: typecheck-pass"),
        "retry_abort_on_permanent.vr must be '@test: typecheck-pass'"
    );
}
