//! `as_f64` on an INTEGER used to disagree between builds ABOUT THE VALUE:
//! the `debug_assert!` aborted a debug build, and release — where that
//! assertion is compiled out — read the integer's NaN-box through
//! `f64::from_bits` and returned a quiet NaN that went on being used as a
//! number.
//!
//! Every call site measured was a float method reading an ARGUMENT (`min`,
//! `max`, `clamp`, `pow`), where an Int argument means 2, not NaN. So the
//! integer is widened, and the two builds now agree.
//!
//! Task: T1048.

use verum_vbc::value::Value;

#[test]
fn an_integer_widens_instead_of_becoming_a_quiet_nan() {
    assert_eq!(Value::from_i64(2).as_f64(), 2.0);
    assert_eq!(Value::from_i64(-7).as_f64(), -7.0);
    assert_eq!(Value::from_i64(0).as_f64(), 0.0);
}

/// The differentiator: a real float must be untouched, or "fix the integer
/// case" could quietly become "read everything as an integer".
#[test]
fn a_float_is_returned_unchanged() {
    assert_eq!(Value::from_f64(2.5).as_f64(), 2.5);
    assert_eq!(Value::from_f64(-0.125).as_f64(), -0.125);
}

/// And the property that made the old behaviour dangerous: the result is a
/// NUMBER, not a NaN. A test asserting only "no panic" would have passed
/// against the defect.
#[test]
fn the_result_is_never_nan_for_an_integer() {
    for i in [-3i64, 0, 1, 42, 1_000_000] {
        let got = Value::from_i64(i).as_f64();
        assert!(!got.is_nan(), "integer {i} read as f64 gave NaN");
        assert_eq!(got, i as f64);
    }
}
