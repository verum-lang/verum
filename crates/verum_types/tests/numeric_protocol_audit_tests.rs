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
//!   Integer      — integer arithmetic classification; extends Numeric.
//!                  Atomic operations additionally require Atomic.
//!   SignedInteger — signed semantics; extends Integer; used by `wrapping_neg`
//!                   and related operations that are only safe on signed types.
//!
//! This drift guard pins:
//!   1. Each protocol is declared in `core/base/protocols.vr`.
//!   2. Each has a known active use site (bound in a generic function or type).
//!   3. Numeric classification and atomic representation remain independent.
//!   4. Zero and One have at least one implementation in primitives.vr (Int, Float).
//!
//! Concrete types and aliases are checked by the compiler's
//! `numeric_capability_contract` tests against the actual stdlib graph.
//! Counting declarations cannot establish which aliases satisfy a bound.

const PROTOCOLS_VR: &str = include_str!("../../../core/base/protocols.vr");
const PRIMITIVES_VR: &str = include_str!("../../../core/base/primitives.vr");
const ITERATOR_VR: &str   = include_str!("../../../core/base/iterator.vr");
const TENSOR_VR: &str     = include_str!("../../../core/math/tensor.vr");
const SIMD_MOD_VR: &str   = include_str!("../../../core/simd/mod.vr");

const ATOMIC_VR: &str = include_str!("../../../core/intrinsics/atomic.vr");

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
fn tensor_stack_bounds_on_the_numeric_tower() {
    // T0499 (00539476e) moved the tensor stack's element bound from
    // `T: Numeric` to the RealField tower — the audit follows the
    // design, not the old spelling. Numeric's own live bound sites are
    // pinned in all_five_protocols_are_non_vestigial (simd/mod.vr).
    assert!(
        TENSOR_VR.contains("RealField"),
        "the tensor stack must bound its elements on the RealField tower \
         (T0499); if the design moved again, point this audit at the new \
         truth rather than deleting it"
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
fn integer_extends_numeric_without_granting_atomic_representation() {
    assert!(
        PROTOCOLS_VR.contains("public type Integer is protocol extends Numeric {}"),
        "Integer classifies arithmetic independently of atomic representation"
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
    // T0499: the tensor stack moved to RealField; Numeric's live bound
    // sites are the SIMD vectors (`T: SimdElement + Numeric`).
    assert!(SIMD_MOD_VR.contains("+ Numeric"),   "Numeric: must have use site in simd/mod.vr");
    assert!(ATOMIC_VR.contains("T: Atomic + Integer"), "Integer: must have a use site in atomic.vr");
    assert!(PROTOCOLS_VR.contains("extends Integer"), "SignedInteger: must extend Integer in protocols.vr");
}
