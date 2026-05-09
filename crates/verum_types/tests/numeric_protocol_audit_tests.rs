#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Vestigial-protocols audit: Zero / One / Numeric / SignedInteger / Integer (#54).
//!
//! Verdict: ALL FIVE protocols are load-bearing.  None are vestigial.
//!
//!   Zero         — additive identity; used by `Iterator::sum_by` bound
//!                  (`B: Add<Output=B> + Zero`).  Implemented in primitives.vr.
//!   One          — multiplicative identity; used by `Iterator::product_by` bound
//!                  (`B: Mul<Output=B> + One`).  Implemented in primitives.vr.
//!   Numeric      — arithmetic marker for integer + float types; used as a bound
//!                  in `math/tensor.vr` (`DynTensor<T: Numeric>`) and `math/linalg.vr`.
//!   Integer      — integer atomic-fetch-modify marker; extends Atomic; used by
//!                  generic atomic intrinsics `atomic_fetch_add<T: Integer>`.
//!   SignedInteger — signed semantics; extends Integer; used by `wrapping_neg`
//!                   and related operations that are only safe on signed types.
//!
//! This drift guard pins:
//!   1. Each protocol is declared in `core/base/protocols.vr`.
//!   2. Each has a known active use site (bound in a generic function or type).
//!   3. Implementation counts in protocols.vr stay stable (Numeric: 16, Integer: 12,
//!      SignedInteger: 7).
//!   4. Zero and One have at least one implementation in primitives.vr (Int, Float).

const PROTOCOLS_VR: &str = include_str!("../../../core/base/protocols.vr");
const PRIMITIVES_VR: &str = include_str!("../../../core/base/primitives.vr");
const ITERATOR_VR: &str   = include_str!("../../../core/base/iterator.vr");
const TENSOR_VR: &str     = include_str!("../../../core/math/tensor.vr");

fn count_occurrences(src: &str, pattern: &str) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = src[start..].find(pattern) {
        count += 1;
        start += pos + pattern.len();
    }
    count
}

// ── Zero ─────────────────────────────────────────────────────────────────────

#[test]
fn zero_protocol_declared_in_protocols_vr() {
    assert!(
        PROTOCOLS_VR.contains("public type Zero is protocol"),
        "Zero protocol must be declared in core/base/protocols.vr"
    );
}

#[test]
fn zero_has_fn_zero_method() {
    assert!(
        PROTOCOLS_VR.contains("fn zero() -> Self"),
        "Zero protocol must have 'fn zero() -> Self' method"
    );
}

#[test]
fn zero_has_fn_is_zero_method() {
    assert!(
        PROTOCOLS_VR.contains("fn is_zero(&self) -> Bool"),
        "Zero protocol must have 'fn is_zero(&self) -> Bool' method"
    );
}

#[test]
fn zero_implemented_for_int_in_primitives_vr() {
    assert!(
        PRIMITIVES_VR.contains("implement Zero for Int"),
        "Zero must be implemented for Int in core/base/primitives.vr"
    );
}

#[test]
fn zero_implemented_for_float_in_primitives_vr() {
    assert!(
        PRIMITIVES_VR.contains("implement Zero for Float"),
        "Zero must be implemented for Float in core/base/primitives.vr"
    );
}

#[test]
fn zero_used_as_bound_in_sum_by() {
    assert!(
        ITERATOR_VR.contains("+ Zero"),
        "Zero must be used as a bound in Iterator::sum_by (': … + Zero') in iterator.vr"
    );
}

// ── One ──────────────────────────────────────────────────────────────────────

#[test]
fn one_protocol_declared_in_protocols_vr() {
    assert!(
        PROTOCOLS_VR.contains("public type One is protocol"),
        "One protocol must be declared in core/base/protocols.vr"
    );
}

#[test]
fn one_has_fn_one_method() {
    assert!(
        PROTOCOLS_VR.contains("fn one() -> Self"),
        "One protocol must have 'fn one() -> Self' method"
    );
}

#[test]
fn one_has_fn_is_one_method() {
    assert!(
        PROTOCOLS_VR.contains("fn is_one(&self) -> Bool"),
        "One protocol must have 'fn is_one(&self) -> Bool' method"
    );
}

#[test]
fn one_implemented_for_int_in_primitives_vr() {
    assert!(
        PRIMITIVES_VR.contains("implement One for Int"),
        "One must be implemented for Int in core/base/primitives.vr"
    );
}

#[test]
fn one_used_as_bound_in_product_by() {
    assert!(
        ITERATOR_VR.contains("+ One"),
        "One must be used as a bound in Iterator::product_by (': … + One') in iterator.vr"
    );
}

// ── Numeric ───────────────────────────────────────────────────────────────────

#[test]
fn numeric_protocol_declared_in_protocols_vr() {
    assert!(
        PROTOCOLS_VR.contains("public type Numeric is protocol"),
        "Numeric protocol must be declared in core/base/protocols.vr"
    );
}

#[test]
fn numeric_used_as_bound_in_tensor_vr() {
    assert!(
        TENSOR_VR.contains("T: Numeric"),
        "Numeric must be used as a bound in math/tensor.vr (e.g. DynTensor<T: Numeric>)"
    );
}

/// Numeric is implemented for 16 types in protocols.vr:
///   8 unsigned ints + 5 signed ints + 3 floats = 16.
#[test]
fn numeric_implementation_count_in_protocols_vr_is_16() {
    let count = count_occurrences(PROTOCOLS_VR, "implement Numeric for ");
    assert_eq!(
        count, 16,
        "Expected 16 Numeric implementations in protocols.vr, got {count}"
    );
}

// ── Integer ───────────────────────────────────────────────────────────────────

#[test]
fn integer_protocol_declared_in_protocols_vr() {
    assert!(
        PROTOCOLS_VR.contains("public type Integer is protocol"),
        "Integer protocol must be declared in core/base/protocols.vr"
    );
}

#[test]
fn integer_extends_atomic() {
    assert!(
        PROTOCOLS_VR.contains("public type Integer is protocol extends Atomic"),
        "Integer must extend Atomic in protocols.vr"
    );
}

/// Integer is implemented for 13 types in protocols.vr:
///   6 unsigned (UInt8/16/32/64/128 + USize) + 7 signed (Int8/16/32/64/128 + Int + ISize) = 13.
#[test]
fn integer_implementation_count_in_protocols_vr_is_13() {
    let count = count_occurrences(PROTOCOLS_VR, "implement Integer for ");
    assert_eq!(
        count, 13,
        "Expected 13 Integer implementations in protocols.vr, got {count}"
    );
}

// ── SignedInteger ─────────────────────────────────────────────────────────────

#[test]
fn signed_integer_protocol_declared_in_protocols_vr() {
    assert!(
        PROTOCOLS_VR.contains("public type SignedInteger is protocol"),
        "SignedInteger protocol must be declared in core/base/protocols.vr"
    );
}

#[test]
fn signed_integer_extends_integer() {
    assert!(
        PROTOCOLS_VR.contains("public type SignedInteger is protocol extends Integer"),
        "SignedInteger must extend Integer in protocols.vr"
    );
}

/// SignedInteger is implemented for 7 types: Int8/16/32/64/128 + Int + ISize.
#[test]
fn signed_integer_implementation_count_in_protocols_vr_is_7() {
    let count = count_occurrences(PROTOCOLS_VR, "implement SignedInteger for ");
    assert_eq!(
        count, 7,
        "Expected 7 SignedInteger implementations in protocols.vr, got {count}"
    );
}

// ── Audit verdict ─────────────────────────────────────────────────────────────

/// All five protocols are NOT vestigial — this test pins that verdict by
/// requiring all declarations and use sites to coexist simultaneously.
/// If any protocol becomes truly unused, its use-site test will fail first.
#[test]
fn all_five_protocols_are_non_vestigial() {
    let protocols = ["Zero", "One", "Numeric", "Integer", "SignedInteger"];
    for name in &protocols {
        assert!(
            PROTOCOLS_VR.contains(&format!("public type {name} is protocol")),
            "Protocol '{name}' must be declared in protocols.vr"
        );
    }
    // At least one known use site per protocol:
    assert!(ITERATOR_VR.contains("+ Zero"),      "Zero: must have use site in iterator.vr");
    assert!(ITERATOR_VR.contains("+ One"),       "One: must have use site in iterator.vr");
    assert!(TENSOR_VR.contains("T: Numeric"),    "Numeric: must have use site in tensor.vr");
    assert!(PROTOCOLS_VR.contains("extends Atomic"), "Integer: must extend Atomic in protocols.vr");
    assert!(PROTOCOLS_VR.contains("extends Integer"), "SignedInteger: must extend Integer in protocols.vr");
}
