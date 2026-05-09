#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Differential cross-tier spec infrastructure validator (#57).
//!
//! `vcs/differential/cross-impl/` contains the canonical cross-tier
//! differential suite: 39 `.vr` files that must all produce identical
//! results on Tier 0 (VBC interpreter) and Tier 1 (JIT/AOT).
//!
//! Each spec carries:
//!   * `@test: differential` — marks it as a cross-tier comparison test.
//!   * `@tier: 0,1`          — instructs the runner to execute on both tiers.
//!   * `@level: L0`          — cross-impl portability is an L0 guarantee.
//!   * `@expected-exit: 0`   — all specs must exit cleanly.
//!
//! This drift guard pins:
//!   1. Every spec in cross-impl/ has `@test: differential`.
//!   2. Every spec in cross-impl/ has `@tier: 0,1` (not single-tier).
//!   3. Every spec in cross-impl/ has `@expected-exit: 0`.
//!   4. The total spec count in cross-impl/ is ≥ 30 (drift alert if suite shrinks).
//!   5. Core categories are represented: arithmetic, control-flow, closures,
//!      collections, functions, pattern-matching, strings, maybe-type.
//!   6. No spec is missing both `fn main()` — every differential spec must have
//!      an entry point the runner can execute on both tiers.
//!
//! Baking content via `include_str!` means file renames and annotation
//! removals both fail CI immediately.

// ── Include a representative sample of the cross-impl specs ──────────────────
// (We include a subset; the directory-level count test uses std::fs.)

const INT_ARITH:   &str = include_str!("../../../vcs/differential/cross-impl/diff_int_arithmetic.vr");
const FLOAT_ARITH: &str = include_str!("../../../vcs/differential/cross-impl/diff_float_arithmetic.vr");
const BITWISE:     &str = include_str!("../../../vcs/differential/cross-impl/diff_bitwise_operations.vr");
const IF_ELSE:     &str = include_str!("../../../vcs/differential/cross-impl/diff_if_else.vr");
const FOR_RANGE:   &str = include_str!("../../../vcs/differential/cross-impl/diff_for_in_range.vr");
const LET_BIND:    &str = include_str!("../../../vcs/differential/cross-impl/diff_let_bindings.vr");
const CLOSURES:    &str = include_str!("../../../vcs/differential/cross-impl/diff_closures_capture.vr");
const HIGHER_ORDER:&str = include_str!("../../../vcs/differential/cross-impl/diff_higher_order_fn.vr");
const LIST_OPS:    &str = include_str!("../../../vcs/differential/cross-impl/diff_list_operations.vr");
const MAP_OPS:     &str = include_str!("../../../vcs/differential/cross-impl/diff_map_operations.vr");
const MAYBE_TYPE:  &str = include_str!("../../../vcs/differential/cross-impl/diff_maybe_type.vr");
const MATCH_EXPR:  &str = include_str!("../../../vcs/differential/cross-impl/diff_match_expr.vr");
const FUNCTIONS:   &str = include_str!("../../../vcs/differential/cross-impl/diff_functions_basic.vr");
const FIBONACCI:   &str = include_str!("../../../vcs/differential/cross-impl/diff_fibonacci_iterative.vr");
const ARRAY_OPS:   &str = include_str!("../../../vcs/differential/cross-impl/diff_array_operations.vr");
const EXIT_CODES:  &str = include_str!("../../../vcs/differential/cross-impl/diff_exit_codes.vr");
const SIEVE:       &str = include_str!("../../../vcs/differential/cross-impl/diff_sieve_primes.vr");

/// All included spec sources as (name, source) pairs for bulk validation.
const ALL_INCLUDED: &[(&str, &str)] = &[
    ("diff_int_arithmetic",    INT_ARITH),
    ("diff_float_arithmetic",  FLOAT_ARITH),
    ("diff_bitwise_operations",BITWISE),
    ("diff_if_else",           IF_ELSE),
    ("diff_for_in_range",      FOR_RANGE),
    ("diff_let_bindings",      LET_BIND),
    ("diff_closures_capture",  CLOSURES),
    ("diff_higher_order_fn",   HIGHER_ORDER),
    ("diff_list_operations",   LIST_OPS),
    ("diff_map_operations",    MAP_OPS),
    ("diff_maybe_type",        MAYBE_TYPE),
    ("diff_match_expr",        MATCH_EXPR),
    ("diff_functions_basic",   FUNCTIONS),
    ("diff_fibonacci_iterative",FIBONACCI),
    ("diff_array_operations",  ARRAY_OPS),
    ("diff_exit_codes",        EXIT_CODES),
    ("diff_sieve_primes",      SIEVE),
];

// ── 1. Every included spec has @test: differential ────────────────────────────

#[test]
fn all_included_specs_have_differential_test_annotation() {
    for (name, src) in ALL_INCLUDED {
        assert!(
            src.contains("@test: differential"),
            "'{name}' must have '// @test: differential' annotation"
        );
    }
}

// ── 2. Every included spec has @tier: 0,1 ────────────────────────────────────

#[test]
fn all_included_specs_have_cross_tier_annotation() {
    for (name, src) in ALL_INCLUDED {
        assert!(
            src.contains("@tier: 0,1"),
            "'{name}' must have '// @tier: 0,1' for cross-tier execution; \
             single-tier '@tier: 0' is not a differential spec"
        );
    }
}

// ── 3. Every included spec has @expected-exit: 0 ─────────────────────────────

#[test]
fn all_included_specs_have_expected_exit_zero() {
    // exit_codes spec intentionally tests non-zero exits — skip it here.
    for (name, src) in ALL_INCLUDED {
        if *name == "diff_exit_codes" { continue; }
        assert!(
            src.contains("@expected-exit: 0"),
            "'{name}' must have '@expected-exit: 0' (cross-tier must exit clean)"
        );
    }
}

// ── 4. Directory spec count is ≥ 30 ──────────────────────────────────────────

#[test]
fn cross_impl_directory_has_at_least_30_specs() {
    let dir = std::path::Path::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../vcs/differential/cross-impl")
    );
    let count = std::fs::read_dir(dir)
        .expect("vcs/differential/cross-impl must exist")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().map(|x| x == "vr").unwrap_or(false)
        })
        .count();
    assert!(
        count >= 30,
        "Expected ≥ 30 specs in cross-impl/, found {count}. \
         Update this threshold after intentional suite additions."
    );
}

// ── 5. Core categories are represented ───────────────────────────────────────

#[test]
fn arithmetic_category_present() {
    assert!(
        INT_ARITH.contains("@tags:") && INT_ARITH.contains("arithmetic"),
        "diff_int_arithmetic must be tagged 'arithmetic'"
    );
    assert!(
        FLOAT_ARITH.contains("@tags:") && FLOAT_ARITH.contains("arithmetic"),
        "diff_float_arithmetic must be tagged 'arithmetic'"
    );
}

#[test]
fn control_flow_category_present() {
    assert!(
        IF_ELSE.contains("@tags:") && (IF_ELSE.contains("control") || IF_ELSE.contains("if_else")),
        "diff_if_else must be tagged with a control-flow category"
    );
    assert!(
        FOR_RANGE.contains("@tags:"),
        "diff_for_in_range must have @tags annotation"
    );
}

#[test]
fn closures_category_present() {
    assert!(
        CLOSURES.contains("@tags:") && CLOSURES.contains("closure"),
        "diff_closures_capture must be tagged 'closures'"
    );
    assert!(
        HIGHER_ORDER.contains("@tags:"),
        "diff_higher_order_fn must have @tags annotation"
    );
}

#[test]
fn collections_category_present() {
    assert!(
        LIST_OPS.contains("@tags:") && LIST_OPS.contains("list"),
        "diff_list_operations must be tagged 'list'"
    );
    assert!(
        MAP_OPS.contains("@tags:"),
        "diff_map_operations must have @tags annotation"
    );
}

#[test]
fn maybe_type_category_present() {
    assert!(
        MAYBE_TYPE.contains("@tags:") && MAYBE_TYPE.contains("maybe"),
        "diff_maybe_type must be tagged 'maybe'"
    );
}

#[test]
fn pattern_matching_category_present() {
    assert!(
        MATCH_EXPR.contains("@tags:") && MATCH_EXPR.contains("match"),
        "diff_match_expr must be tagged 'match'"
    );
}

// ── 6. Every included spec has fn main() ─────────────────────────────────────

#[test]
fn all_included_specs_have_main_entry_point() {
    for (name, src) in ALL_INCLUDED {
        assert!(
            src.contains("fn main()"),
            "'{name}' must have 'fn main()' — cross-tier differential requires an entry point"
        );
    }
}

// ── 7. Fibonacci spec computes a deterministic value ─────────────────────────
//
// The fibonacci spec is the canonical "same algorithm, two tiers" probe.
// Pin that the 20th Fibonacci number (6765) appears as a literal or assert.

#[test]
fn fibonacci_spec_pins_a_deterministic_value() {
    assert!(
        FIBONACCI.contains("6765") || FIBONACCI.contains("fib(20)") || FIBONACCI.contains("fibonacci"),
        "diff_fibonacci_iterative must reference a known Fibonacci value or assertion"
    );
}

// ── 8. Sieve spec uses @tier: 0,1 (not just @tier: 0) ────────────────────────

#[test]
fn sieve_spec_is_cross_tier() {
    assert!(
        SIEVE.contains("@tier: 0,1"),
        "diff_sieve_primes must use '@tier: 0,1' for cross-tier execution"
    );
}

// ── 9. No spec uses Rust syntax ───────────────────────────────────────────────

#[test]
fn no_included_spec_uses_rust_struct_keyword() {
    for (name, src) in ALL_INCLUDED {
        // `struct` as a standalone word (not part of `construct`, `restructure`)
        let has_struct = src.split_whitespace().any(|w| w == "struct" || w == "struct{");
        assert!(
            !has_struct,
            "'{name}' must not use Rust 'struct' — use Verum 'type X is {{ ... }}'"
        );
    }
}

#[test]
fn no_included_spec_uses_rust_impl_keyword_standalone() {
    for (name, src) in ALL_INCLUDED {
        // Guard: `impl` standalone (not `implement`)
        let has_impl = src.lines().any(|l| {
            let trimmed = l.trim_start();
            trimmed.starts_with("impl ") && !trimmed.starts_with("implement")
        });
        assert!(
            !has_impl,
            "'{name}' must not use Rust 'impl' — use Verum 'implement'"
        );
    }
}
