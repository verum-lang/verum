#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Drift guard for the five numeric-protocol definitions (#54).
//!
//! Pinned facts:
//!   • `Zero`  — 2 required methods (`zero`, `is_zero`); non-marker; 16 primitive impls.
//!   • `One`   — 2 required methods (`one`,  `is_one`);  non-marker; 16 primitive impls.
//!   • `Integer`      — marker (0 methods); 13 implementations (all integer primitives).
//!   • `SignedInteger` — marker (0 methods);  7 implementations (signed integers only).
//!   • `Numeric`      — marker (0 methods); 16 implementations (integers + floats).
//!
//! All counts are verified by scanning the stdlib source files that are the
//! single source of truth.  Any refactor that adds, removes, or renames a
//! protocol method or implementation block will be caught here before it
//! can silently change the numeric-protocol surface.

// Source text baked in at compile time — no runtime I/O, no flakiness.
const PROTOCOLS_SRC: &str = include_str!("../../../core/base/protocols.vr");
const PRIMITIVES_SRC: &str = include_str!("../../../core/base/primitives.vr");

// ─── helpers ─────────────────────────────────────────────────────────────────

fn count_lines_matching(src: &str, pattern: &str) -> usize {
    src.lines().filter(|l| l.contains(pattern)).count()
}

// ─── Zero protocol ───────────────────────────────────────────────────────────

/// `Zero` must declare exactly `fn zero() -> Self` and `fn is_zero(&self) -> Bool`.
/// If the method table is widened or narrowed, downstream callers (generic
/// numeric algorithms) break silently unless this test fires first.
#[test]
fn zero_protocol_has_exactly_two_method_declarations() {
    let zero_methods = [
        "fn zero() -> Self",
        "fn is_zero(&self) -> Bool",
    ];
    for sig in &zero_methods {
        assert!(
            PROTOCOLS_SRC.contains(sig),
            "Zero protocol must declare `{sig}` — not found in core/base/protocols.vr",
        );
    }
    // Ensure the count inside the protocol block is exactly 2.
    // We anchor on the `Zero is protocol {` block: the two method declarations
    // must both appear between that header and the closing `};`.
    let block_start = PROTOCOLS_SRC
        .find("type Zero is protocol {")
        .expect("Zero protocol declaration not found");
    let block_end = PROTOCOLS_SRC[block_start..]
        .find("};")
        .expect("Zero protocol closing `};` not found")
        + block_start;
    let zero_block = &PROTOCOLS_SRC[block_start..block_end];
    let fn_count = zero_block.matches("fn ").count();
    assert_eq!(
        fn_count, 2,
        "Zero protocol must have exactly 2 method declarations, found {fn_count}",
    );
}

/// `Zero` must be implemented for all 16 numeric primitive types.
/// Integers (signed + unsigned) + 3 float types = 16.
#[test]
fn zero_protocol_has_sixteen_primitive_implementations() {
    let expected: &[&str] = &[
        "implement Zero for Int ",
        "implement Zero for Int8 ",
        "implement Zero for Int16 ",
        "implement Zero for Int32 ",
        "implement Zero for Int64 ",
        "implement Zero for Int128 ",
        "implement Zero for ISize ",
        "implement Zero for UInt8 ",
        "implement Zero for UInt16 ",
        "implement Zero for UInt32 ",
        "implement Zero for UInt64 ",
        "implement Zero for UInt128 ",
        "implement Zero for USize ",
        "implement Zero for Float ",
        "implement Zero for Float32 ",
        "implement Zero for Float64 ",
    ];
    for pat in expected {
        assert!(
            PRIMITIVES_SRC.contains(pat.trim()),
            "Missing implementation: `{pat}` in core/base/primitives.vr",
        );
    }
    let total = count_lines_matching(PRIMITIVES_SRC, "implement Zero for ");
    assert_eq!(
        total,
        expected.len(),
        "implement Zero for: expected {} impls, found {total}",
        expected.len(),
    );
}

// ─── One protocol ────────────────────────────────────────────────────────────

/// `One` must declare exactly `fn one() -> Self` and `fn is_one(&self) -> Bool`.
#[test]
fn one_protocol_has_exactly_two_method_declarations() {
    let one_methods = [
        "fn one() -> Self",
        "fn is_one(&self) -> Bool",
    ];
    for sig in &one_methods {
        assert!(
            PROTOCOLS_SRC.contains(sig),
            "One protocol must declare `{sig}` — not found in core/base/protocols.vr",
        );
    }
    let block_start = PROTOCOLS_SRC
        .find("type One is protocol {")
        .expect("One protocol declaration not found");
    let block_end = PROTOCOLS_SRC[block_start..]
        .find("};")
        .expect("One protocol closing `};` not found")
        + block_start;
    let one_block = &PROTOCOLS_SRC[block_start..block_end];
    let fn_count = one_block.matches("fn ").count();
    assert_eq!(
        fn_count, 2,
        "One protocol must have exactly 2 method declarations, found {fn_count}",
    );
}

/// `One` must be implemented for the same 16 numeric primitive types as `Zero`.
#[test]
fn one_protocol_has_sixteen_primitive_implementations() {
    let expected: &[&str] = &[
        "implement One for Int ",
        "implement One for Int8 ",
        "implement One for Int16 ",
        "implement One for Int32 ",
        "implement One for Int64 ",
        "implement One for Int128 ",
        "implement One for ISize ",
        "implement One for UInt8 ",
        "implement One for UInt16 ",
        "implement One for UInt32 ",
        "implement One for UInt64 ",
        "implement One for UInt128 ",
        "implement One for USize ",
        "implement One for Float ",
        "implement One for Float32 ",
        "implement One for Float64 ",
    ];
    for pat in expected {
        assert!(
            PRIMITIVES_SRC.contains(pat.trim()),
            "Missing implementation: `{pat}` in core/base/primitives.vr",
        );
    }
    let total = count_lines_matching(PRIMITIVES_SRC, "implement One for ");
    assert_eq!(
        total,
        expected.len(),
        "implement One for: expected {} impls, found {total}",
        expected.len(),
    );
}

// ─── Zero / One symmetry ─────────────────────────────────────────────────────

/// Both Zero and One must cover exactly the same set of types.
/// A type that can produce an additive identity must also be able to produce
/// a multiplicative identity, and vice versa.
#[test]
fn zero_and_one_cover_symmetric_type_sets() {
    let zero_count = count_lines_matching(PRIMITIVES_SRC, "implement Zero for ");
    let one_count = count_lines_matching(PRIMITIVES_SRC, "implement One for ");
    assert_eq!(
        zero_count, one_count,
        "Zero ({zero_count} impls) and One ({one_count} impls) must cover the same types",
    );
}

// ─── Integer (marker) ────────────────────────────────────────────────────────

/// `Integer` must be a marker protocol: empty body (no `fn` declarations).
#[test]
fn integer_protocol_is_a_marker_with_zero_methods() {
    let block_start = PROTOCOLS_SRC
        .find("type Integer is protocol extends Atomic")
        .expect("Integer protocol declaration not found");
    let block_end = PROTOCOLS_SRC[block_start..]
        .find("};")
        .expect("Integer protocol closing `};` not found")
        + block_start;
    let block = &PROTOCOLS_SRC[block_start..block_end];
    let fn_count = block.matches("fn ").count();
    assert_eq!(
        fn_count, 0,
        "Integer is a marker protocol — it must have 0 method declarations, found {fn_count}",
    );
}

/// `Integer` must be implemented for all 13 integer primitive types.
#[test]
fn integer_protocol_has_thirteen_implementations() {
    let expected: &[&str] = &[
        "implement Integer for UInt8 ",
        "implement Integer for UInt16 ",
        "implement Integer for UInt32 ",
        "implement Integer for UInt64 ",
        "implement Integer for UInt128 ",
        "implement Integer for USize ",
        "implement Integer for Int8 ",
        "implement Integer for Int16 ",
        "implement Integer for Int32 ",
        "implement Integer for Int64 ",
        "implement Integer for Int128 ",
        "implement Integer for Int ",
        "implement Integer for ISize ",
    ];
    for pat in expected {
        assert!(
            PROTOCOLS_SRC.contains(pat.trim()),
            "Missing implementation: `{pat}` in core/base/protocols.vr",
        );
    }
    let total = count_lines_matching(PROTOCOLS_SRC, "implement Integer for ");
    assert_eq!(
        total,
        expected.len(),
        "implement Integer for: expected {} impls, found {total}",
        expected.len(),
    );
}

// ─── SignedInteger (marker) ───────────────────────────────────────────────────

/// `SignedInteger` must be a marker protocol: empty body.
#[test]
fn signed_integer_protocol_is_a_marker_with_zero_methods() {
    let block_start = PROTOCOLS_SRC
        .find("type SignedInteger is protocol extends Integer")
        .expect("SignedInteger protocol declaration not found");
    let block_end = PROTOCOLS_SRC[block_start..]
        .find("};")
        .expect("SignedInteger protocol closing `};` not found")
        + block_start;
    let block = &PROTOCOLS_SRC[block_start..block_end];
    let fn_count = block.matches("fn ").count();
    assert_eq!(
        fn_count, 0,
        "SignedInteger is a marker protocol — it must have 0 methods, found {fn_count}",
    );
}

/// `SignedInteger` must cover exactly the 7 signed integer primitives.
#[test]
fn signed_integer_protocol_has_seven_implementations() {
    let expected: &[&str] = &[
        "implement SignedInteger for Int8 ",
        "implement SignedInteger for Int16 ",
        "implement SignedInteger for Int32 ",
        "implement SignedInteger for Int64 ",
        "implement SignedInteger for Int128 ",
        "implement SignedInteger for Int ",
        "implement SignedInteger for ISize ",
    ];
    for pat in expected {
        assert!(
            PROTOCOLS_SRC.contains(pat.trim()),
            "Missing implementation: `{pat}` in core/base/protocols.vr",
        );
    }
    let total = count_lines_matching(PROTOCOLS_SRC, "implement SignedInteger for ");
    assert_eq!(
        total,
        expected.len(),
        "implement SignedInteger for: expected {} impls, found {total}",
        expected.len(),
    );
}

// ─── Numeric (marker) ────────────────────────────────────────────────────────

/// `Numeric` must be a marker protocol: empty body.
#[test]
fn numeric_protocol_is_a_marker_with_zero_methods() {
    let block_start = PROTOCOLS_SRC
        .find("type Numeric is protocol extends Copy + Sized")
        .expect("Numeric protocol declaration not found");
    let block_end = PROTOCOLS_SRC[block_start..]
        .find("};")
        .expect("Numeric protocol closing `};` not found")
        + block_start;
    let block = &PROTOCOLS_SRC[block_start..block_end];
    let fn_count = block.matches("fn ").count();
    assert_eq!(
        fn_count, 0,
        "Numeric is a marker protocol — it must have 0 methods, found {fn_count}",
    );
}

/// `Numeric` must cover all 16 numeric primitive types (integers + floats).
#[test]
fn numeric_protocol_has_sixteen_implementations() {
    let expected: &[&str] = &[
        "implement Numeric for UInt8 ",
        "implement Numeric for UInt16 ",
        "implement Numeric for UInt32 ",
        "implement Numeric for UInt64 ",
        "implement Numeric for UInt128 ",
        "implement Numeric for USize ",
        "implement Numeric for Int8 ",
        "implement Numeric for Int16 ",
        "implement Numeric for Int32 ",
        "implement Numeric for Int64 ",
        "implement Numeric for Int128 ",
        "implement Numeric for Int ",
        "implement Numeric for ISize ",
        "implement Numeric for Float ",
        "implement Numeric for Float32 ",
        "implement Numeric for Float64 ",
    ];
    for pat in expected {
        assert!(
            PROTOCOLS_SRC.contains(pat.trim()),
            "Missing implementation: `{pat}` in core/base/protocols.vr",
        );
    }
    let total = count_lines_matching(PROTOCOLS_SRC, "implement Numeric for ");
    assert_eq!(
        total,
        expected.len(),
        "implement Numeric for: expected {} impls, found {total}",
        expected.len(),
    );
}

// ─── Subset invariants ───────────────────────────────────────────────────────

/// Every `SignedInteger` implementor must also implement `Integer` (subset invariant).
/// Numeric includes all Integer types plus floats.
#[test]
fn signed_integer_is_subset_of_integer_and_integer_is_subset_of_numeric() {
    let signed_types = [
        "Int8", "Int16", "Int32", "Int64", "Int128", "Int", "ISize",
    ];
    for ty in &signed_types {
        let signed_pat = format!("implement SignedInteger for {ty} ");
        let integer_pat = format!("implement Integer for {ty} ");
        let numeric_pat = format!("implement Numeric for {ty} ");
        assert!(
            PROTOCOLS_SRC.contains(&signed_pat),
            "{ty} must implement SignedInteger",
        );
        assert!(
            PROTOCOLS_SRC.contains(&integer_pat),
            "{ty} must implement Integer (SignedInteger ⊆ Integer invariant)",
        );
        assert!(
            PROTOCOLS_SRC.contains(&numeric_pat),
            "{ty} must implement Numeric (Integer ⊆ Numeric invariant)",
        );
    }
}
