//! Arithmetic helper functions for VBC interpreter dispatch.
//! Includes wrapping and saturating arithmetic with bit-width support.

// ============================================================================
// Bit Mask and Sign Extension
// ============================================================================

/// Computes the mask for a given bit width.
#[inline(always)]
pub(super) fn type_mask(width: u8) -> u64 {
    match width {
        8 => 0xFF,
        16 => 0xFFFF,
        32 => 0xFFFF_FFFF,
        64 | 128 => u64::MAX,
        _ => u64::MAX, // Default to 64-bit
    }
}

/// Sign-extends a value from the given bit width to i64.
#[inline(always)]
pub(super) fn sign_extend(value: u64, width: u8, mask: u64) -> i64 {
    let sign_bit = 1u64 << (width - 1);
    if value & sign_bit != 0 {
        (value | !mask) as i64
    } else {
        value as i64
    }
}

// ============================================================================
// Wrapping Arithmetic Helpers
// ============================================================================

/// Wrapping addition with truncation to bit width.
/// If signed=true, sign-extends the result.
#[inline(always)]
pub(super) fn wrapping_add(a: i64, b: i64, width: u8, signed: bool) -> i64 {
    let mask = type_mask(width);
    let result = (a as u64).wrapping_add(b as u64) & mask;
    if signed {
        sign_extend(result, width, mask)
    } else {
        result as i64
    }
}

/// Wrapping subtraction with truncation to bit width.
/// If signed=true, sign-extends the result.
#[inline(always)]
pub(super) fn wrapping_sub(a: i64, b: i64, width: u8, signed: bool) -> i64 {
    let mask = type_mask(width);
    let result = (a as u64).wrapping_sub(b as u64) & mask;
    if signed {
        sign_extend(result, width, mask)
    } else {
        result as i64
    }
}

/// Wrapping multiplication with truncation to bit width.
/// If signed=true, sign-extends the result.
#[inline(always)]
pub(super) fn wrapping_mul(a: i64, b: i64, width: u8, signed: bool) -> i64 {
    let mask = type_mask(width);
    let result = (a as u64).wrapping_mul(b as u64) & mask;
    if signed {
        sign_extend(result, width, mask)
    } else {
        result as i64
    }
}

/// Wrapping negation with truncation to bit width.
#[inline(always)]
pub(super) fn wrapping_neg(a: i64, width: u8, signed: bool) -> i64 {
    let mask = type_mask(width);
    if signed {
        // For signed, negate and truncate
        let result = (a as u64).wrapping_neg() & mask;
        // Sign extend if necessary
        let sign_bit = 1u64 << (width - 1);
        if result & sign_bit != 0 {
            (result | !mask) as i64
        } else {
            result as i64
        }
    } else {
        // Unsigned negation: -x mod 2^width = (2^width - x) mod 2^width
        ((0u64.wrapping_sub(a as u64)) & mask) as i64
    }
}

/// Wrapping left shift with shift amount mod width.
/// If signed=true, sign-extends the result.
#[inline(always)]
pub(super) fn wrapping_shl(a: i64, b: u32, width: u8, signed: bool) -> i64 {
    let shift = b % (width as u32);
    let mask = type_mask(width);
    let result = ((a as u64) << shift) & mask;
    if signed {
        sign_extend(result, width, mask)
    } else {
        result as i64
    }
}

/// Wrapping right shift with shift amount mod width.
/// For signed types, performs arithmetic shift (sign-extends).
/// For unsigned types, performs logical shift (zero-extends).
#[inline(always)]
pub(super) fn wrapping_shr(a: i64, b: u32, width: u8, signed: bool) -> i64 {
    let shift = b % (width as u32);
    let mask = type_mask(width);

    if signed {
        // For signed, need to sign-extend before shift, then truncate
        let sign_bit = 1i64 << (width as i64 - 1);
        let val = a & (mask as i64);
        // Sign extend to full width if negative
        let extended = if val & sign_bit != 0 {
            val | !(mask as i64)
        } else {
            val
        };
        // Arithmetic shift right
        (extended >> shift) & (mask as i64)
    } else {
        // Unsigned: logical shift
        (((a as u64) & mask) >> shift) as i64
    }
}

// ============================================================================
// Saturating Arithmetic Helpers
// ============================================================================

/// Returns (min, max) bounds for a given bit width and signedness.
#[inline(always)]
pub(super) fn type_bounds(width: u8, signed: bool) -> (i64, i64) {
    if signed {
        match width {
            8 => (i8::MIN as i64, i8::MAX as i64),
            16 => (i16::MIN as i64, i16::MAX as i64),
            32 => (i32::MIN as i64, i32::MAX as i64),
            64 | 128 => (i64::MIN, i64::MAX),
            _ => (i64::MIN, i64::MAX),
        }
    } else {
        match width {
            8 => (0, u8::MAX as i64),
            16 => (0, u16::MAX as i64),
            32 => (0, u32::MAX as i64),
            64 | 128 => (0, i64::MAX), // Can't represent full u64 max in i64
            _ => (0, i64::MAX),
        }
    }
}

/// Saturating addition.
#[inline(always)]
pub(super) fn saturating_add(a: i64, b: i64, width: u8, signed: bool) -> i64 {
    let (min_val, max_val) = type_bounds(width, signed);

    if signed {
        // Check for signed overflow/underflow
        match a.checked_add(b) {
            Some(result) if result > max_val => max_val,
            Some(result) if result < min_val => min_val,
            Some(result) => result,
            None => {
                // Overflow occurred
                if b > 0 { max_val } else { min_val }
            }
        }
    } else {
        // Unsigned: treat as unsigned and saturate
        let au = a as u64;
        let bu = b as u64;
        match au.checked_add(bu) {
            Some(result) if result > max_val as u64 => max_val,
            Some(result) => result as i64,
            None => max_val,
        }
    }
}

/// Saturating subtraction.
#[inline(always)]
pub(super) fn saturating_sub(a: i64, b: i64, width: u8, signed: bool) -> i64 {
    let (min_val, max_val) = type_bounds(width, signed);

    if signed {
        match a.checked_sub(b) {
            Some(result) if result > max_val => max_val,
            Some(result) if result < min_val => min_val,
            Some(result) => result,
            None => {
                // Overflow occurred
                if b < 0 { max_val } else { min_val }
            }
        }
    } else {
        // Unsigned: saturate at 0
        let au = a as u64;
        let bu = b as u64;
        if bu > au { 0 } else { (au - bu) as i64 }
    }
}

/// Saturating multiplication.
#[inline(always)]
pub(super) fn saturating_mul(a: i64, b: i64, width: u8, signed: bool) -> i64 {
    let (min_val, max_val) = type_bounds(width, signed);

    if signed {
        match a.checked_mul(b) {
            Some(result) if result > max_val => max_val,
            Some(result) if result < min_val => min_val,
            Some(result) => result,
            None => {
                // Overflow - determine direction
                if (a > 0 && b > 0) || (a < 0 && b < 0) {
                    max_val
                } else {
                    min_val
                }
            }
        }
    } else {
        let au = a as u64;
        let bu = b as u64;
        match au.checked_mul(bu) {
            Some(result) if result > max_val as u64 => max_val,
            Some(result) => result as i64,
            None => max_val,
        }
    }
}
