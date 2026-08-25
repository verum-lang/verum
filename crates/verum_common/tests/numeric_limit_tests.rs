//! The limit constants of the primitive numeric types have VALUES, and
//! this is the one place that knows them.
//!
//! Before this table existed, each consumer re-derived them. The SMT
//! translator did not, and translated `Int.MAX` to a free symbol: the
//! solver was then free to model it as a negative number and reported
//! `fn abs_value(n: Int) -> Int{>= 0} { … Int.MAX … }` as violating its
//! own refinement, naming an impossible counterexample. A wrong answer
//! delivered with a counterexample reads as authoritative, which is why
//! the exact values are pinned here rather than left to each caller.

use verum_common::well_known_types::type_names;

#[test]
fn signed_limits_are_exact_at_every_width() {
    let cases: &[(&str, i128, i128)] = &[
        ("Int8", i8::MIN as i128, i8::MAX as i128),
        ("Int16", i16::MIN as i128, i16::MAX as i128),
        ("Int32", i32::MIN as i128, i32::MAX as i128),
        ("Int", i64::MIN as i128, i64::MAX as i128),
        ("Int64", i64::MIN as i128, i64::MAX as i128),
        ("Int128", i128::MIN, i128::MAX),
    ];
    for (name, min, max) in cases {
        assert_eq!(
            type_names::integer_limit(name, "MIN"),
            Some(*min),
            "{name}.MIN"
        );
        assert_eq!(
            type_names::integer_limit(name, "MAX"),
            Some(*max),
            "{name}.MAX"
        );
    }
}

#[test]
fn unsigned_limits_start_at_zero() {
    for name in ["UInt8", "UInt16", "UInt32", "UInt", "UInt64"] {
        assert_eq!(type_names::integer_limit(name, "MIN"), Some(0), "{name}.MIN");
    }
    assert_eq!(
        type_names::integer_limit("UInt8", "MAX"),
        Some(u8::MAX as i128)
    );
    assert_eq!(
        type_names::integer_limit("UInt64", "MAX"),
        Some(u64::MAX as i128)
    );
}

/// `UInt128.MAX` does not fit in the return type. Reporting `None` is
/// honest; the previous bytecode-side table answered `u128::MAX as i128`,
/// which is **-1** — a wrong value wearing the right shape, and exactly
/// the kind of answer a caller cannot detect.
#[test]
fn the_unsigned_128_bit_maximum_is_refused_rather_than_truncated() {
    assert_eq!(type_names::integer_limit("UInt128", "MAX"), None);
    assert_eq!(type_names::integer_limit("UInt128", "MIN"), Some(0));
}

#[test]
fn aliases_agree_with_their_canonical_spelling() {
    for (alias, canonical) in [
        ("i64", "Int"),
        ("u8", "UInt8"),
        ("I32", "Int32"),
        ("usize", "UInt"),
    ] {
        assert_eq!(
            type_names::integer_limit(alias, "MAX"),
            type_names::integer_limit(canonical, "MAX"),
            "{alias} must mean the same as {canonical}"
        );
    }
}

#[test]
fn float_constants_carry_real_values_not_bit_patterns() {
    assert_eq!(
        type_names::float_constant("Float", "EPSILON"),
        Some(f64::EPSILON)
    );
    assert_eq!(type_names::float_constant("Float", "MAX"), Some(f64::MAX));
    assert_eq!(
        type_names::float_constant("Float32", "MAX"),
        Some(f32::MAX as f64)
    );
    assert_eq!(
        type_names::float_constant("Float", "PI"),
        Some(std::f64::consts::PI)
    );
    assert!(
        type_names::float_constant("Float", "NAN")
            .expect("NAN is a float constant")
            .is_nan()
    );
}

/// The negative poles. Without these, a table that answered `Some(0)`
/// to everything would pass every assertion above.
#[test]
fn non_numeric_types_and_unknown_constants_have_no_limit() {
    assert_eq!(type_names::integer_limit("Text", "MAX"), None);
    assert_eq!(type_names::integer_limit("List", "MAX"), None);
    assert_eq!(type_names::integer_limit("Int", "LARGEST"), None);
    assert_eq!(type_names::float_constant("Int", "PI"), None);
    assert_eq!(type_names::float_constant("Text", "EPSILON"), None);
    // A float type has no integer limit and vice versa — the two tables
    // partition the numeric types rather than overlapping.
    assert_eq!(type_names::integer_limit("Float", "MAX"), None);
}

#[test]
fn bits_reports_the_width_not_a_limit() {
    assert_eq!(type_names::integer_limit("Int", "BITS"), Some(64));
    assert_eq!(type_names::integer_limit("Int8", "BITS"), Some(8));
    assert_eq!(type_names::float_constant("Float32", "BITS"), Some(32.0));
}
