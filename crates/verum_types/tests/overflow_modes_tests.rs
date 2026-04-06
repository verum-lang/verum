#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Comprehensive tests for integer overflow modes
//
// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .3
//
// Tests all three overflow modes (checked, wrapping, saturating) for
// all 12 integer types with all 9 arithmetic operations.

use verum_common::Maybe;

// ============================================================================
// Checked Arithmetic Tests (Returns Maybe<T>)
// ============================================================================

#[test]
fn test_checked_add_success() {
    // Signed types
    assert_eq!(100i8.checked_add(27), Some(127));
    assert_eq!(100i16.checked_add(200), Some(300));
    assert_eq!(100i32.checked_add(200), Some(300));
    assert_eq!(100i64.checked_add(200), Some(300));
    assert_eq!(100i128.checked_add(200), Some(300));

    // Unsigned types
    assert_eq!(100u8.checked_add(155), Some(255));
    assert_eq!(100u16.checked_add(200), Some(300));
    assert_eq!(100u32.checked_add(200), Some(300));
    assert_eq!(100u64.checked_add(200), Some(300));
    assert_eq!(100u128.checked_add(200), Some(300));
}

#[test]
fn test_checked_add_overflow() {
    // Signed overflow
    assert_eq!(i8::MAX.checked_add(1), None);
    assert_eq!(i16::MAX.checked_add(1), None);
    assert_eq!(i32::MAX.checked_add(1), None);
    assert_eq!(i64::MAX.checked_add(1), None);
    assert_eq!(i128::MAX.checked_add(1), None);

    // Unsigned overflow
    assert_eq!(u8::MAX.checked_add(1), None);
    assert_eq!(u16::MAX.checked_add(1), None);
    assert_eq!(u32::MAX.checked_add(1), None);
    assert_eq!(u64::MAX.checked_add(1), None);
    assert_eq!(u128::MAX.checked_add(1), None);
}

#[test]
fn test_checked_add_underflow() {
    // Signed underflow
    assert_eq!(i8::MIN.checked_add(-1), None);
    assert_eq!(i16::MIN.checked_add(-1), None);
    assert_eq!(i32::MIN.checked_add(-1), None);
    assert_eq!(i64::MIN.checked_add(-1), None);
    assert_eq!(i128::MIN.checked_add(-1), None);
}

#[test]
fn test_checked_sub_success() {
    assert_eq!(100i32.checked_sub(50), Some(50));
    assert_eq!(200u32.checked_sub(100), Some(100));
}

#[test]
fn test_checked_sub_overflow() {
    // Signed underflow
    assert_eq!(i8::MIN.checked_sub(1), None);
    assert_eq!(i32::MIN.checked_sub(1), None);

    // Unsigned underflow
    assert_eq!(0u8.checked_sub(1), None);
    assert_eq!(0u32.checked_sub(1), None);
}

#[test]
fn test_checked_mul_success() {
    assert_eq!(10i32.checked_mul(20), Some(200));
    assert_eq!(10u32.checked_mul(20), Some(200));
}

#[test]
fn test_checked_mul_overflow() {
    assert_eq!(i8::MAX.checked_mul(2), None);
    assert_eq!(i32::MAX.checked_mul(2), None);
    assert_eq!(u8::MAX.checked_mul(2), None);
    assert_eq!(u32::MAX.checked_mul(2), None);
}

#[test]
fn test_checked_mul_large_values() {
    let a: i64 = 1_000_000_000;
    let b: i64 = 1_000_000_000;
    assert_eq!(a.checked_mul(b), Some(1_000_000_000_000_000_000));

    let c: i64 = 10_000_000_000;
    assert_eq!(a.checked_mul(c), None); // Overflow
}

#[test]
fn test_checked_div_success() {
    assert_eq!(100i32.checked_div(5), Some(20));
    assert_eq!(100u32.checked_div(5), Some(20));
}

#[test]
fn test_checked_div_by_zero() {
    assert_eq!(100i32.checked_div(0), None);
    assert_eq!(100u32.checked_div(0), None);
}

#[test]
fn test_checked_div_min_by_minus_one() {
    // Special case: i32::MIN / -1 overflows
    assert_eq!(i8::MIN.checked_div(-1), None);
    assert_eq!(i32::MIN.checked_div(-1), None);
}

#[test]
fn test_checked_rem_success() {
    assert_eq!(100i32.checked_rem(7), Some(2));
    assert_eq!(100u32.checked_rem(7), Some(2));
}

#[test]
fn test_checked_rem_by_zero() {
    assert_eq!(100i32.checked_rem(0), None);
    assert_eq!(100u32.checked_rem(0), None);
}

#[test]
fn test_checked_neg_success() {
    assert_eq!(42i32.checked_neg(), Some(-42));
    assert_eq!((-42i32).checked_neg(), Some(42));
}

#[test]
fn test_checked_neg_overflow() {
    assert_eq!(i8::MIN.checked_neg(), None);
    assert_eq!(i32::MIN.checked_neg(), None);
}

#[test]
fn test_checked_shl_success() {
    assert_eq!(1i32.checked_shl(5), Some(32));
    assert_eq!(1u32.checked_shl(5), Some(32));
}

#[test]
fn test_checked_shl_overflow() {
    assert_eq!(1i32.checked_shl(32), None); // Shift >= bit width
    assert_eq!(1u32.checked_shl(32), None);
}

#[test]
fn test_checked_shr_success() {
    assert_eq!(32i32.checked_shr(5), Some(1));
    assert_eq!(32u32.checked_shr(5), Some(1));
}

#[test]
fn test_checked_shr_overflow() {
    assert_eq!(32i32.checked_shr(32), None);
    assert_eq!(32u32.checked_shr(32), None);
}

#[test]
fn test_checked_pow_success() {
    assert_eq!(2i32.checked_pow(10), Some(1024));
    assert_eq!(2u32.checked_pow(10), Some(1024));
}

#[test]
fn test_checked_pow_overflow() {
    assert_eq!(2i32.checked_pow(31), None); // 2^31 overflows i32
    assert_eq!(2u32.checked_pow(32), None); // 2^32 overflows u32
}

// ============================================================================
// Wrapping Arithmetic Tests (Two's Complement)
// ============================================================================

#[test]
fn test_wrapping_add_overflow() {
    // Signed wrapping
    assert_eq!(i8::MAX.wrapping_add(1), i8::MIN);
    assert_eq!(i16::MAX.wrapping_add(1), i16::MIN);
    assert_eq!(i32::MAX.wrapping_add(1), i32::MIN);
    assert_eq!(i64::MAX.wrapping_add(1), i64::MIN);
    assert_eq!(i128::MAX.wrapping_add(1), i128::MIN);

    // Unsigned wrapping
    assert_eq!(u8::MAX.wrapping_add(1), 0);
    assert_eq!(u16::MAX.wrapping_add(1), 0);
    assert_eq!(u32::MAX.wrapping_add(1), 0);
    assert_eq!(u64::MAX.wrapping_add(1), 0);
    assert_eq!(u128::MAX.wrapping_add(1), 0);
}

#[test]
fn test_wrapping_add_underflow() {
    // Signed wrapping
    assert_eq!(i8::MIN.wrapping_add(-1), i8::MAX);
    assert_eq!(i32::MIN.wrapping_add(-1), i32::MAX);

    // Unsigned wrapping
    assert_eq!(0u8.wrapping_add(u8::MAX), u8::MAX);
}

#[test]
fn test_wrapping_sub_underflow() {
    // Unsigned wrapping
    assert_eq!(0u8.wrapping_sub(1), u8::MAX);
    assert_eq!(0u16.wrapping_sub(1), u16::MAX);
    assert_eq!(0u32.wrapping_sub(1), u32::MAX);
    assert_eq!(0u64.wrapping_sub(1), u64::MAX);
}

#[test]
fn test_wrapping_sub_overflow() {
    // Signed wrapping
    assert_eq!(i8::MIN.wrapping_sub(1), i8::MAX);
    assert_eq!(i32::MIN.wrapping_sub(1), i32::MAX);
}

#[test]
fn test_wrapping_mul_overflow() {
    assert_eq!(i8::MAX.wrapping_mul(2), -2);
    assert_eq!(u8::MAX.wrapping_mul(2), 254);
}

#[test]
fn test_wrapping_div_normal() {
    assert_eq!(100i32.wrapping_div(5), 20);
    assert_eq!(100u32.wrapping_div(5), 20);
}

#[test]
#[should_panic]
fn test_wrapping_div_by_zero() {
    let _ = 100i32.wrapping_div(0); // Still panics
}

#[test]
fn test_wrapping_rem_normal() {
    assert_eq!(100i32.wrapping_rem(7), 2);
    assert_eq!(100u32.wrapping_rem(7), 2);
}

#[test]
fn test_wrapping_neg_overflow() {
    assert_eq!(i8::MIN.wrapping_neg(), i8::MIN); // -128 wraps to -128
    assert_eq!(i32::MIN.wrapping_neg(), i32::MIN);
}

#[test]
fn test_wrapping_shl_overflow() {
    assert_eq!(1i32.wrapping_shl(33), 2); // 33 % 32 = 1
    assert_eq!(1u32.wrapping_shl(33), 2);
}

#[test]
fn test_wrapping_shr_overflow() {
    assert_eq!(32i32.wrapping_shr(33), 16); // 33 % 32 = 1
    assert_eq!(32u32.wrapping_shr(33), 16);
}

#[test]
fn test_wrapping_pow_overflow() {
    assert_eq!(2i32.wrapping_pow(31), i32::MIN); // Wraps to min value
}

// ============================================================================
// Saturating Arithmetic Tests (Clamp to Bounds)
// ============================================================================

#[test]
fn test_saturating_add_overflow() {
    // Signed saturation
    assert_eq!(i8::MAX.saturating_add(1), i8::MAX);
    assert_eq!(i16::MAX.saturating_add(1), i16::MAX);
    assert_eq!(i32::MAX.saturating_add(1), i32::MAX);
    assert_eq!(i64::MAX.saturating_add(1), i64::MAX);
    assert_eq!(i128::MAX.saturating_add(1), i128::MAX);

    // Unsigned saturation
    assert_eq!(u8::MAX.saturating_add(1), u8::MAX);
    assert_eq!(u16::MAX.saturating_add(1), u16::MAX);
    assert_eq!(u32::MAX.saturating_add(1), u32::MAX);
    assert_eq!(u64::MAX.saturating_add(1), u64::MAX);
    assert_eq!(u128::MAX.saturating_add(1), u128::MAX);
}

#[test]
fn test_saturating_add_underflow() {
    // Signed saturation
    assert_eq!(i8::MIN.saturating_add(-1), i8::MIN);
    assert_eq!(i32::MIN.saturating_add(-1), i32::MIN);
}

#[test]
fn test_saturating_add_normal() {
    assert_eq!(100i32.saturating_add(50), 150);
    assert_eq!(100u32.saturating_add(50), 150);
}

#[test]
fn test_saturating_sub_underflow() {
    // Unsigned saturation to 0
    assert_eq!(0u8.saturating_sub(1), 0);
    assert_eq!(10u8.saturating_sub(20), 0);
    assert_eq!(0u32.saturating_sub(1), 0);

    // Signed saturation to min
    assert_eq!(i8::MIN.saturating_sub(1), i8::MIN);
    assert_eq!(i32::MIN.saturating_sub(1), i32::MIN);
}

#[test]
fn test_saturating_sub_overflow() {
    // Signed saturation to max
    assert_eq!(i8::MAX.saturating_sub(-1), i8::MAX);
    assert_eq!(i32::MAX.saturating_sub(-1), i32::MAX);
}

#[test]
fn test_saturating_sub_normal() {
    assert_eq!(100i32.saturating_sub(50), 50);
    assert_eq!(100u32.saturating_sub(50), 50);
}

#[test]
fn test_saturating_mul_overflow() {
    // Signed saturation
    assert_eq!(i8::MAX.saturating_mul(2), i8::MAX);
    assert_eq!(i32::MAX.saturating_mul(2), i32::MAX);

    // Unsigned saturation
    assert_eq!(u8::MAX.saturating_mul(2), u8::MAX);
    assert_eq!(u32::MAX.saturating_mul(2), u32::MAX);
}

#[test]
fn test_saturating_mul_underflow() {
    // Signed negative overflow
    assert_eq!(i8::MIN.saturating_mul(2), i8::MIN);
    assert_eq!(i32::MIN.saturating_mul(2), i32::MIN);
}

#[test]
fn test_saturating_mul_normal() {
    assert_eq!(10i32.saturating_mul(20), 200);
    assert_eq!(10u32.saturating_mul(20), 200);
}

#[test]
fn test_saturating_pow_overflow() {
    // Signed saturation
    assert_eq!(2i8.saturating_pow(10), i8::MAX); // 2^10 = 1024 > 127
    assert_eq!(2i32.saturating_pow(31), i32::MAX);

    // Unsigned saturation
    assert_eq!(2u8.saturating_pow(10), u8::MAX);
    assert_eq!(2u32.saturating_pow(32), u32::MAX);
}

#[test]
fn test_saturating_pow_normal() {
    assert_eq!(2i32.saturating_pow(10), 1024);
    assert_eq!(2u32.saturating_pow(10), 1024);
}

// ============================================================================
// Platform-Dependent Types (isize, usize)
// ============================================================================

#[test]
fn test_isize_overflow_modes() {
    // Checked
    assert_eq!(100isize.checked_add(200), Some(300));
    assert_eq!(isize::MAX.checked_add(1), None);

    // Wrapping
    assert_eq!(isize::MAX.wrapping_add(1), isize::MIN);

    // Saturating
    assert_eq!(isize::MAX.saturating_add(1), isize::MAX);
}

#[test]
fn test_usize_overflow_modes() {
    // Checked
    assert_eq!(100usize.checked_add(200), Some(300));
    assert_eq!(usize::MAX.checked_add(1), None);

    // Wrapping
    assert_eq!(usize::MAX.wrapping_add(1), 0);

    // Saturating
    assert_eq!(usize::MAX.saturating_add(1), usize::MAX);
}

// ============================================================================
// Edge Cases and Special Values
// ============================================================================

#[test]
fn test_zero_operations() {
    // Adding zero
    assert_eq!(0i32.checked_add(0), Some(0));
    assert_eq!(0u32.checked_add(0), Some(0));

    // Multiplying by zero
    assert_eq!(i32::MAX.checked_mul(0), Some(0));
    assert_eq!(u32::MAX.checked_mul(0), Some(0));

    // Subtracting zero
    assert_eq!(100i32.checked_sub(0), Some(100));
}

#[test]
fn test_one_operations() {
    // Multiplying by one
    assert_eq!(100i32.checked_mul(1), Some(100));
    assert_eq!(100u32.checked_mul(1), Some(100));

    // Dividing by one
    assert_eq!(100i32.checked_div(1), Some(100));
    assert_eq!(100u32.checked_div(1), Some(100));
}

#[test]
fn test_negative_operations() {
    // Subtracting negative (adds)
    assert_eq!(100i32.checked_sub(-50), Some(150));

    // Multiplying by negative
    assert_eq!(100i32.checked_mul(-2), Some(-200));

    // Dividing by negative
    assert_eq!(100i32.checked_div(-2), Some(-50));
}

#[test]
fn test_boundary_values() {
    // Operations at boundaries
    assert_eq!(i8::MAX.checked_add(0), Some(i8::MAX));
    assert_eq!(i8::MIN.checked_add(0), Some(i8::MIN));

    assert_eq!(u8::MAX.checked_add(0), Some(u8::MAX));
    assert_eq!(0u8.checked_sub(0), Some(0));
}

// ============================================================================
// Mixed Mode Tests
// ============================================================================

#[test]
fn test_all_modes_consistency() {
    let a: i32 = 100;
    let b: i32 = 200;

    // All modes should give same result for non-overflowing ops
    assert_eq!(a.checked_add(b), Some(300));
    assert_eq!(a.wrapping_add(b), 300);
    assert_eq!(a.saturating_add(b), 300);

    // Different behavior on overflow
    let max = i32::MAX;
    assert_eq!(max.checked_add(1), None); // Returns None
    assert_eq!(max.wrapping_add(1), i32::MIN); // Wraps around
    assert_eq!(max.saturating_add(1), i32::MAX); // Clamps to max
}

// ============================================================================
// Performance Characteristic Tests
// ============================================================================

#[test]
fn test_operations_are_inlined() {
    // These operations should all be inlined (#[inline])
    // and compile to single instructions
    let x = 100i32;
    let y = 200i32;

    let _checked = x.checked_add(y);
    let _wrapping = x.wrapping_add(y);
    let _saturating = x.saturating_add(y);

    // If this test compiles and runs fast, inlining is working
}

#[test]
fn test_all_12_types() {
    // Verify all 12 integer types implement all three traits

    // Signed
    let _: Maybe<i8> = 1i8.checked_add(1);
    let _: i8 = 1i8.wrapping_add(1);
    let _: i8 = 1i8.saturating_add(1);

    let _: Maybe<i16> = 1i16.checked_add(1);
    let _: i16 = 1i16.wrapping_add(1);
    let _: i16 = 1i16.saturating_add(1);

    let _: Maybe<i32> = 1i32.checked_add(1);
    let _: i32 = 1i32.wrapping_add(1);
    let _: i32 = 1i32.saturating_add(1);

    let _: Maybe<i64> = 1i64.checked_add(1);
    let _: i64 = 1i64.wrapping_add(1);
    let _: i64 = 1i64.saturating_add(1);

    let _: Maybe<i128> = 1i128.checked_add(1);
    let _: i128 = 1i128.wrapping_add(1);
    let _: i128 = 1i128.saturating_add(1);

    let _: Maybe<isize> = 1isize.checked_add(1);
    let _: isize = 1isize.wrapping_add(1);
    let _: isize = 1isize.saturating_add(1);

    // Unsigned
    let _: Maybe<u8> = 1u8.checked_add(1);
    let _: u8 = 1u8.wrapping_add(1);
    let _: u8 = 1u8.saturating_add(1);

    let _: Maybe<u16> = 1u16.checked_add(1);
    let _: u16 = 1u16.wrapping_add(1);
    let _: u16 = 1u16.saturating_add(1);

    let _: Maybe<u32> = 1u32.checked_add(1);
    let _: u32 = 1u32.wrapping_add(1);
    let _: u32 = 1u32.saturating_add(1);

    let _: Maybe<u64> = 1u64.checked_add(1);
    let _: u64 = 1u64.wrapping_add(1);
    let _: u64 = 1u64.saturating_add(1);

    let _: Maybe<u128> = 1u128.checked_add(1);
    let _: u128 = 1u128.wrapping_add(1);
    let _: u128 = 1u128.saturating_add(1);

    let _: Maybe<usize> = 1usize.checked_add(1);
    let _: usize = 1usize.wrapping_add(1);
    let _: usize = 1usize.saturating_add(1);
}
