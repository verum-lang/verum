//! FFI Test Library for Verum Conformance Suite
//!
//! This library provides C-compatible functions for testing the VBC FFI system.
//! It covers a wide range of FFI patterns including:
//!
//! - Basic type passing (integers, floats, booleans)
//! - Pointer types (raw pointers, C strings)
//! - Struct passing (by value and by reference)
//! - Callbacks (function pointers)
//! - Variadic functions
//! - Error handling (errno)
//! - Thread-local storage
//! - Memory operations (allocation, alignment)
//! - Array operations with callbacks (qsort-style)

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_float, c_int, c_long, c_ulong, c_void};
use std::ptr;
use std::slice;

// =============================================================================
// Basic Type Operations
// =============================================================================

/// Returns the sum of two i32 values.
#[no_mangle]
pub extern "C" fn ffi_add_i32(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

/// Returns the sum of two i64 values.
#[no_mangle]
pub extern "C" fn ffi_add_i64(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

/// Returns the sum of two u32 values.
#[no_mangle]
pub extern "C" fn ffi_add_u32(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}

/// Returns the sum of two u64 values.
#[no_mangle]
pub extern "C" fn ffi_add_u64(a: u64, b: u64) -> u64 {
    a.wrapping_add(b)
}

/// Returns the sum of two f32 values.
#[no_mangle]
pub extern "C" fn ffi_add_f32(a: c_float, b: c_float) -> c_float {
    a + b
}

/// Returns the sum of two f64 values.
#[no_mangle]
pub extern "C" fn ffi_add_f64(a: c_double, b: c_double) -> c_double {
    a + b
}

/// Multiply two i32 values.
#[no_mangle]
pub extern "C" fn ffi_mul_i32(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}

/// Divide two i32 values (returns 0 on division by zero).
#[no_mangle]
pub extern "C" fn ffi_div_i32(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a / b
    }
}

/// Returns the maximum of two i64 values.
#[no_mangle]
pub extern "C" fn ffi_max_i64(a: i64, b: i64) -> i64 {
    if a > b { a } else { b }
}

/// Returns the minimum of two i64 values.
#[no_mangle]
pub extern "C" fn ffi_min_i64(a: i64, b: i64) -> i64 {
    if a < b { a } else { b }
}

/// Returns the absolute value of an i64.
#[no_mangle]
pub extern "C" fn ffi_abs_i64(a: i64) -> i64 {
    a.abs()
}

/// Clamps a value to a range.
#[no_mangle]
pub extern "C" fn ffi_clamp_i64(value: i64, min: i64, max: i64) -> i64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

// =============================================================================
// Floating Point Operations
// =============================================================================

/// Returns the square root of a f64.
#[no_mangle]
pub extern "C" fn ffi_sqrt_f64(x: c_double) -> c_double {
    x.sqrt()
}

/// Returns the floor of a f64.
#[no_mangle]
pub extern "C" fn ffi_floor_f64(x: c_double) -> c_double {
    x.floor()
}

/// Returns the ceiling of a f64.
#[no_mangle]
pub extern "C" fn ffi_ceil_f64(x: c_double) -> c_double {
    x.ceil()
}

/// Returns the rounded value of a f64.
#[no_mangle]
pub extern "C" fn ffi_round_f64(x: c_double) -> c_double {
    x.round()
}

/// Returns sin(x).
#[no_mangle]
pub extern "C" fn ffi_sin_f64(x: c_double) -> c_double {
    x.sin()
}

/// Returns cos(x).
#[no_mangle]
pub extern "C" fn ffi_cos_f64(x: c_double) -> c_double {
    x.cos()
}

/// Returns pow(base, exp).
#[no_mangle]
pub extern "C" fn ffi_pow_f64(base: c_double, exp: c_double) -> c_double {
    base.powf(exp)
}

/// Check if f64 is NaN.
#[no_mangle]
pub extern "C" fn ffi_is_nan_f64(x: c_double) -> i32 {
    if x.is_nan() { 1 } else { 0 }
}

/// Check if f64 is infinite.
#[no_mangle]
pub extern "C" fn ffi_is_inf_f64(x: c_double) -> i32 {
    if x.is_infinite() { 1 } else { 0 }
}

// =============================================================================
// Bitwise Operations
// =============================================================================

/// Count leading zeros.
#[no_mangle]
pub extern "C" fn ffi_clz_u64(x: u64) -> i32 {
    x.leading_zeros() as i32
}

/// Count trailing zeros.
#[no_mangle]
pub extern "C" fn ffi_ctz_u64(x: u64) -> i32 {
    x.trailing_zeros() as i32
}

/// Population count (number of 1 bits).
#[no_mangle]
pub extern "C" fn ffi_popcnt_u64(x: u64) -> i32 {
    x.count_ones() as i32
}

/// Byte swap.
#[no_mangle]
pub extern "C" fn ffi_bswap_u32(x: u32) -> u32 {
    x.swap_bytes()
}

/// Byte swap 64-bit.
#[no_mangle]
pub extern "C" fn ffi_bswap_u64(x: u64) -> u64 {
    x.swap_bytes()
}

/// Rotate left.
#[no_mangle]
pub extern "C" fn ffi_rotl_u64(x: u64, n: u32) -> u64 {
    x.rotate_left(n)
}

/// Rotate right.
#[no_mangle]
pub extern "C" fn ffi_rotr_u64(x: u64, n: u32) -> u64 {
    x.rotate_right(n)
}

// =============================================================================
// Many Arguments (tests register spilling to stack)
// =============================================================================

/// Add 8 integers (tests argument passing ABI).
#[no_mangle]
pub extern "C" fn ffi_add_8_i64(
    a: i64, b: i64, c: i64, d: i64,
    e: i64, f: i64, g: i64, h: i64,
) -> i64 {
    a + b + c + d + e + f + g + h
}

/// Add 12 integers (tests stack-based arguments on most ABIs).
#[no_mangle]
pub extern "C" fn ffi_add_12_i64(
    a: i64, b: i64, c: i64, d: i64,
    e: i64, f: i64, g: i64, h: i64,
    i: i64, j: i64, k: i64, l: i64,
) -> i64 {
    a + b + c + d + e + f + g + h + i + j + k + l
}

/// Mixed integer and float arguments.
#[no_mangle]
pub extern "C" fn ffi_mixed_args(
    a: i64, b: f64, c: i64, d: f64,
    e: i64, f: f64, g: i64, h: f64,
) -> f64 {
    (a as f64) + b + (c as f64) + d + (e as f64) + f + (g as f64) + h
}

// =============================================================================
// Pointer Operations
// =============================================================================

/// Dereference an i32 pointer and return the value.
#[no_mangle]
pub extern "C" fn ffi_deref_i32(ptr: *const i32) -> i32 {
    if ptr.is_null() {
        0
    } else {
        unsafe { *ptr }
    }
}

/// Write a value through a pointer.
#[no_mangle]
pub extern "C" fn ffi_write_i32(ptr: *mut i32, value: i32) {
    if !ptr.is_null() {
        unsafe { *ptr = value };
    }
}

/// Increment value at pointer and return new value.
#[no_mangle]
pub extern "C" fn ffi_inc_i32(ptr: *mut i32) -> i32 {
    if ptr.is_null() {
        0
    } else {
        unsafe {
            *ptr += 1;
            *ptr
        }
    }
}

/// Swap two i32 values.
#[no_mangle]
pub extern "C" fn ffi_swap_i32(a: *mut i32, b: *mut i32) {
    if !a.is_null() && !b.is_null() {
        unsafe {
            std::ptr::swap(a, b);
        }
    }
}

/// Return a pointer to a static constant.
#[no_mangle]
pub extern "C" fn ffi_get_magic_ptr() -> *const i32 {
    static MAGIC: i32 = 0xDEADBEEFu32 as i32;
    &MAGIC
}

// =============================================================================
// String Operations
// =============================================================================

/// Return the length of a C string.
#[no_mangle]
pub extern "C" fn ffi_strlen(s: *const c_char) -> usize {
    if s.is_null() {
        0
    } else {
        unsafe { CStr::from_ptr(s).to_bytes().len() }
    }
}

/// Compare two C strings.
#[no_mangle]
pub extern "C" fn ffi_strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    if s1.is_null() || s2.is_null() {
        if s1 == s2 {
            0
        } else if s1.is_null() {
            -1
        } else {
            1
        }
    } else {
        unsafe {
            let str1 = CStr::from_ptr(s1);
            let str2 = CStr::from_ptr(s2);
            match str1.cmp(str2) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
    }
}

/// Calculate hash of a string.
#[no_mangle]
pub extern "C" fn ffi_hash_string(s: *const c_char) -> u64 {
    if s.is_null() {
        0
    } else {
        unsafe {
            let bytes = CStr::from_ptr(s).to_bytes();
            // Simple FNV-1a hash
            let mut hash: u64 = 0xcbf29ce484222325;
            for &byte in bytes {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
            hash
        }
    }
}

/// Returns a pointer to a static string.
#[no_mangle]
pub extern "C" fn ffi_get_greeting() -> *const c_char {
    static GREETING: &[u8] = b"Hello from FFI!\0";
    GREETING.as_ptr() as *const c_char
}

/// Concatenate two strings into a newly allocated buffer.
/// Caller must free with ffi_free.
#[no_mangle]
pub extern "C" fn ffi_concat(s1: *const c_char, s2: *const c_char) -> *mut c_char {
    let str1 = if s1.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(s1).to_string_lossy().into_owned() }
    };
    let str2 = if s2.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(s2).to_string_lossy().into_owned() }
    };

    match CString::new(str1 + &str2) {
        Ok(cstring) => cstring.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

// =============================================================================
// Struct Operations
// =============================================================================

/// A simple point struct.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Point2D {
    pub x: i32,
    pub y: i32,
}

/// A 3D point struct.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A complex struct with various field types.
#[repr(C)]
pub struct ComplexStruct {
    pub id: u64,
    pub name: *const c_char,
    pub value: f64,
    pub flags: u32,
    pub data: [u8; 16],
}

/// Create a Point2D.
#[no_mangle]
pub extern "C" fn ffi_make_point2d(x: i32, y: i32) -> Point2D {
    Point2D { x, y }
}

/// Add two Point2D values.
#[no_mangle]
pub extern "C" fn ffi_add_point2d(a: Point2D, b: Point2D) -> Point2D {
    Point2D {
        x: a.x + b.x,
        y: a.y + b.y,
    }
}

/// Calculate distance from origin for Point2D.
#[no_mangle]
pub extern "C" fn ffi_point2d_magnitude(p: Point2D) -> f64 {
    ((p.x as f64).powi(2) + (p.y as f64).powi(2)).sqrt()
}

/// Get x coordinate of Point2D by pointer.
#[no_mangle]
pub extern "C" fn ffi_point2d_get_x(p: *const Point2D) -> i32 {
    if p.is_null() {
        0
    } else {
        unsafe { (*p).x }
    }
}

/// Modify Point2D in place.
#[no_mangle]
pub extern "C" fn ffi_point2d_scale(p: *mut Point2D, factor: i32) {
    if !p.is_null() {
        unsafe {
            (*p).x *= factor;
            (*p).y *= factor;
        }
    }
}

/// Create a Point3D.
#[no_mangle]
pub extern "C" fn ffi_make_point3d(x: f64, y: f64, z: f64) -> Point3D {
    Point3D { x, y, z }
}

/// Add two Point3D values.
#[no_mangle]
pub extern "C" fn ffi_add_point3d(a: Point3D, b: Point3D) -> Point3D {
    Point3D {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Dot product of two Point3D.
#[no_mangle]
pub extern "C" fn ffi_dot_point3d(a: Point3D, b: Point3D) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Calculate magnitude of Point3D.
#[no_mangle]
pub extern "C" fn ffi_point3d_magnitude(p: Point3D) -> f64 {
    (p.x.powi(2) + p.y.powi(2) + p.z.powi(2)).sqrt()
}

/// Get size of ComplexStruct (for verification).
#[no_mangle]
pub extern "C" fn ffi_sizeof_complex_struct() -> usize {
    std::mem::size_of::<ComplexStruct>()
}

/// Initialize a ComplexStruct.
#[no_mangle]
pub extern "C" fn ffi_init_complex_struct(s: *mut ComplexStruct, id: u64, value: f64) {
    if !s.is_null() {
        unsafe {
            (*s).id = id;
            (*s).name = ptr::null();
            (*s).value = value;
            (*s).flags = 0;
            (*s).data = [0u8; 16];
        }
    }
}

// =============================================================================
// Array Operations
// =============================================================================

/// Sum an array of i32 values.
#[no_mangle]
pub extern "C" fn ffi_sum_array_i32(arr: *const i32, len: usize) -> i64 {
    if arr.is_null() || len == 0 {
        0
    } else {
        unsafe {
            slice::from_raw_parts(arr, len)
                .iter()
                .map(|&x| x as i64)
                .sum()
        }
    }
}

/// Sum an array of f64 values.
#[no_mangle]
pub extern "C" fn ffi_sum_array_f64(arr: *const f64, len: usize) -> f64 {
    if arr.is_null() || len == 0 {
        0.0
    } else {
        unsafe { slice::from_raw_parts(arr, len).iter().sum() }
    }
}

/// Find maximum in array.
#[no_mangle]
pub extern "C" fn ffi_max_array_i32(arr: *const i32, len: usize) -> i32 {
    if arr.is_null() || len == 0 {
        i32::MIN
    } else {
        unsafe {
            slice::from_raw_parts(arr, len)
                .iter()
                .copied()
                .max()
                .unwrap_or(i32::MIN)
        }
    }
}

/// Find minimum in array.
#[no_mangle]
pub extern "C" fn ffi_min_array_i32(arr: *const i32, len: usize) -> i32 {
    if arr.is_null() || len == 0 {
        i32::MAX
    } else {
        unsafe {
            slice::from_raw_parts(arr, len)
                .iter()
                .copied()
                .min()
                .unwrap_or(i32::MAX)
        }
    }
}

/// Fill an array with a value.
#[no_mangle]
pub extern "C" fn ffi_fill_array_i32(arr: *mut i32, len: usize, value: i32) {
    if !arr.is_null() && len > 0 {
        unsafe {
            slice::from_raw_parts_mut(arr, len).fill(value);
        }
    }
}

/// Copy array (byte-level copy, len is byte count).
#[no_mangle]
pub extern "C" fn ffi_copy_array(dst: *mut u8, src: *const u8, len: usize) {
    if !dst.is_null() && !src.is_null() && len > 0 {
        unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }
}

/// Copy array of i64 values (element-level copy, len is element count).
#[no_mangle]
pub extern "C" fn ffi_copy_array_i64(dst: *mut i64, src: *const i64, len: usize) {
    if !dst.is_null() && !src.is_null() && len > 0 {
        unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }
}

/// Reverse an array in place.
#[no_mangle]
pub extern "C" fn ffi_reverse_array_i32(arr: *mut i32, len: usize) {
    if !arr.is_null() && len > 1 {
        unsafe {
            slice::from_raw_parts_mut(arr, len).reverse();
        }
    }
}

// =============================================================================
// Callback Operations
// =============================================================================

/// Type for a comparator callback.
pub type ComparatorFn = extern "C" fn(i32, i32) -> i32;

/// Type for a unary operation callback.
pub type UnaryOpFn = extern "C" fn(i32) -> i32;

/// Type for a binary operation callback.
pub type BinaryOpFn = extern "C" fn(i32, i32) -> i32;

/// Type for a predicate callback.
pub type PredicateFn = extern "C" fn(i32) -> i32;

/// Call a comparator callback with two values.
#[no_mangle]
pub extern "C" fn ffi_call_comparator(f: ComparatorFn, a: i32, b: i32) -> i32 {
    f(a, b)
}

/// Apply a unary operation to a value.
#[no_mangle]
pub extern "C" fn ffi_apply_unary(f: UnaryOpFn, x: i32) -> i32 {
    f(x)
}

/// Apply a binary operation.
#[no_mangle]
pub extern "C" fn ffi_apply_binary(f: BinaryOpFn, a: i32, b: i32) -> i32 {
    f(a, b)
}

/// Sort an array using a callback comparator (qsort-style).
#[no_mangle]
pub extern "C" fn ffi_sort_array_i32(arr: *mut i32, len: usize, cmp: ComparatorFn) {
    if arr.is_null() || len < 2 {
        return;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(arr, len);
        slice.sort_by(|a, b| {
            let result = cmp(*a, *b);
            if result < 0 {
                std::cmp::Ordering::Less
            } else if result > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    }
}

/// Map a function over an array.
#[no_mangle]
pub extern "C" fn ffi_map_array_i32(
    arr: *mut i32,
    len: usize,
    f: UnaryOpFn,
) {
    if arr.is_null() || len == 0 {
        return;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(arr, len);
        for elem in slice.iter_mut() {
            *elem = f(*elem);
        }
    }
}

/// Filter array elements by predicate, returns new length.
#[no_mangle]
pub extern "C" fn ffi_filter_array_i32(
    arr: *mut i32,
    len: usize,
    pred: PredicateFn,
) -> usize {
    if arr.is_null() || len == 0 {
        return 0;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(arr, len);
        let mut write_idx = 0;

        for read_idx in 0..len {
            if pred(slice[read_idx]) != 0 {
                slice[write_idx] = slice[read_idx];
                write_idx += 1;
            }
        }

        write_idx
    }
}

/// Fold/reduce an array with a callback.
#[no_mangle]
pub extern "C" fn ffi_fold_array_i32(
    arr: *const i32,
    len: usize,
    init: i32,
    f: BinaryOpFn,
) -> i32 {
    if arr.is_null() || len == 0 {
        return init;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        slice.iter().fold(init, |acc, &x| f(acc, x))
    }
}

/// Find first element matching predicate, returns index or -1.
#[no_mangle]
pub extern "C" fn ffi_find_array_i32(
    arr: *const i32,
    len: usize,
    pred: PredicateFn,
) -> i64 {
    if arr.is_null() || len == 0 {
        return -1;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        for (i, &x) in slice.iter().enumerate() {
            if pred(x) != 0 {
                return i as i64;
            }
        }
        -1
    }
}

/// Count elements matching predicate.
#[no_mangle]
pub extern "C" fn ffi_count_array_i32(
    arr: *const i32,
    len: usize,
    pred: PredicateFn,
) -> usize {
    if arr.is_null() || len == 0 {
        return 0;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        slice.iter().filter(|&&x| pred(x) != 0).count()
    }
}

/// Check if any element matches predicate.
#[no_mangle]
pub extern "C" fn ffi_any_array_i32(
    arr: *const i32,
    len: usize,
    pred: PredicateFn,
) -> i32 {
    if arr.is_null() || len == 0 {
        return 0;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        if slice.iter().any(|&x| pred(x) != 0) { 1 } else { 0 }
    }
}

/// Check if all elements match predicate.
#[no_mangle]
pub extern "C" fn ffi_all_array_i32(
    arr: *const i32,
    len: usize,
    pred: PredicateFn,
) -> i32 {
    if arr.is_null() || len == 0 {
        return 1; // vacuously true
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        if slice.iter().all(|&x| pred(x) != 0) { 1 } else { 0 }
    }
}

// =============================================================================
// Advanced Callback Operations (libSystem patterns)
// =============================================================================

// --- Callbacks with context pointer (qsort_r / signal handler style) ---

/// Type for a comparator with context (qsort_r style).
pub type ComparatorWithContextFn = extern "C" fn(*const c_void, i32, i32) -> i32;

/// Type for a callback with void context (pthread_create style).
pub type ThreadFn = extern "C" fn(*mut c_void) -> *mut c_void;

/// Type for a simple void callback (atexit style).
pub type VoidCallbackFn = extern "C" fn();

/// Type for a callback with context returning void.
pub type ContextCallbackFn = extern "C" fn(*mut c_void);

/// Type for a callback receiving f64 and returning f64.
pub type FloatCallbackFn = extern "C" fn(c_double) -> c_double;

/// Type for a callback receiving two f64 and returning f64.
pub type FloatBinaryCallbackFn = extern "C" fn(c_double, c_double) -> c_double;

/// Type for an error callback with code and message.
pub type ErrorCallbackFn = extern "C" fn(i32, *const c_char);

/// Sort array with context (qsort_r style).
#[no_mangle]
pub extern "C" fn ffi_sort_with_context(
    arr: *mut i32,
    len: usize,
    ctx: *const c_void,
    cmp: ComparatorWithContextFn,
) {
    if arr.is_null() || len < 2 {
        return;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(arr, len);
        slice.sort_by(|a, b| {
            let result = cmp(ctx, *a, *b);
            if result < 0 {
                std::cmp::Ordering::Less
            } else if result > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    }
}

/// Binary search with context (bsearch_r style).
#[no_mangle]
pub extern "C" fn ffi_bsearch_with_context(
    arr: *const i32,
    len: usize,
    key: i32,
    ctx: *const c_void,
    cmp: ComparatorWithContextFn,
) -> i64 {
    if arr.is_null() || len == 0 {
        return -1;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        match slice.binary_search_by(|probe| {
            let result = cmp(ctx, *probe, key);
            if result < 0 {
                std::cmp::Ordering::Less
            } else if result > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        }) {
            Ok(idx) => idx as i64,
            Err(_) => -1,
        }
    }
}

/// Fold with context.
#[no_mangle]
pub extern "C" fn ffi_fold_with_context(
    arr: *const i32,
    len: usize,
    init: i32,
    ctx: *mut c_void,
    f: extern "C" fn(*mut c_void, i32, i32) -> i32,
) -> i32 {
    if arr.is_null() || len == 0 {
        return init;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        slice.iter().fold(init, |acc, &x| f(ctx, acc, x))
    }
}

// --- Thread-like callbacks (pthread_create pattern) ---

/// Execute a thread-like callback with context.
#[no_mangle]
pub extern "C" fn ffi_run_thread_callback(
    f: ThreadFn,
    arg: *mut c_void,
) -> *mut c_void {
    f(arg)
}

/// Execute callback and return status code.
#[no_mangle]
pub extern "C" fn ffi_run_with_status(
    f: extern "C" fn(*mut c_void) -> i32,
    arg: *mut c_void,
) -> i32 {
    f(arg)
}

// --- Void callbacks (atexit / cleanup pattern) ---

thread_local! {
    static ATEXIT_CALLBACKS: RefCell<Vec<VoidCallbackFn>> = const { RefCell::new(Vec::new()) };
}

/// Register an atexit-style callback.
#[no_mangle]
pub extern "C" fn ffi_register_atexit(f: VoidCallbackFn) -> i32 {
    ATEXIT_CALLBACKS.with(|cbs| {
        cbs.borrow_mut().push(f);
    });
    0 // success
}

/// Run all registered atexit callbacks (in reverse order).
#[no_mangle]
pub extern "C" fn ffi_run_atexit_callbacks() {
    ATEXIT_CALLBACKS.with(|cbs| {
        let callbacks: Vec<_> = cbs.borrow_mut().drain(..).collect();
        for f in callbacks.into_iter().rev() {
            f();
        }
    });
}

/// Clear all atexit callbacks without running.
#[no_mangle]
pub extern "C" fn ffi_clear_atexit_callbacks() {
    ATEXIT_CALLBACKS.with(|cbs| {
        cbs.borrow_mut().clear();
    });
}

// --- Floating-point callbacks ---

/// Apply a float transformation to all elements.
#[no_mangle]
pub extern "C" fn ffi_map_array_f64(
    arr: *mut c_double,
    len: usize,
    f: FloatCallbackFn,
) {
    if arr.is_null() || len == 0 {
        return;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(arr, len);
        for elem in slice.iter_mut() {
            *elem = f(*elem);
        }
    }
}

/// Reduce array with float binary callback.
#[no_mangle]
pub extern "C" fn ffi_reduce_f64(
    arr: *const c_double,
    len: usize,
    init: c_double,
    f: FloatBinaryCallbackFn,
) -> c_double {
    if arr.is_null() || len == 0 {
        return init;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        slice.iter().fold(init, |acc, &x| f(acc, x))
    }
}

/// Numerical integration using Simpson's rule with callback.
#[no_mangle]
pub extern "C" fn ffi_integrate_simpson(
    f: FloatCallbackFn,
    a: c_double,
    b: c_double,
    n: i32, // number of intervals (must be even)
) -> c_double {
    if n <= 0 || n % 2 != 0 {
        return 0.0;
    }

    let n = n as usize;
    let h = (b - a) / (n as f64);
    let mut sum = f(a) + f(b);

    for i in 1..n {
        let x = a + (i as f64) * h;
        if i % 2 == 0 {
            sum += 2.0 * f(x);
        } else {
            sum += 4.0 * f(x);
        }
    }

    sum * h / 3.0
}

/// Root finding using bisection method with callback.
#[no_mangle]
pub extern "C" fn ffi_bisection(
    f: FloatCallbackFn,
    a: c_double,
    b: c_double,
    tolerance: c_double,
    max_iterations: i32,
) -> c_double {
    let mut low = a;
    let mut high = b;
    let mut f_low = f(low);

    for _ in 0..max_iterations {
        let mid = (low + high) / 2.0;
        let f_mid = f(mid);

        if f_mid.abs() < tolerance || (high - low) / 2.0 < tolerance {
            return mid;
        }

        if f_mid.signum() == f_low.signum() {
            low = mid;
            f_low = f_mid;
        } else {
            high = mid;
        }
    }

    (low + high) / 2.0
}

// --- Error handling with callbacks ---

thread_local! {
    static ERROR_HANDLER: RefCell<Option<ErrorCallbackFn>> = const { RefCell::new(None) };
}

/// Set an error handler callback.
#[no_mangle]
pub extern "C" fn ffi_set_error_handler(handler: ErrorCallbackFn) {
    ERROR_HANDLER.with(|h| {
        *h.borrow_mut() = Some(handler);
    });
}

/// Clear the error handler.
#[no_mangle]
pub extern "C" fn ffi_clear_error_handler() {
    ERROR_HANDLER.with(|h| {
        *h.borrow_mut() = None;
    });
}

/// Report an error (invokes handler if set).
#[no_mangle]
pub extern "C" fn ffi_report_error(code: i32, message: *const c_char) {
    ERROR_HANDLER.with(|h| {
        if let Some(handler) = *h.borrow() {
            handler(code, message);
        }
    });
}

/// Division with error callback on failure.
#[no_mangle]
pub extern "C" fn ffi_safe_div(a: i32, b: i32) -> i32 {
    if b == 0 {
        static MSG: &[u8] = b"Division by zero\0";
        ffi_report_error(-1, MSG.as_ptr() as *const c_char);
        return 0;
    }
    a / b
}

// --- Recursive callbacks (callback calls back into FFI) ---

/// Recursively apply callback n times.
#[no_mangle]
pub extern "C" fn ffi_iterate(
    f: UnaryOpFn,
    initial: i32,
    n: i32,
) -> i32 {
    let mut value = initial;
    for _ in 0..n {
        value = f(value);
    }
    value
}

/// Fixed-point iteration (keeps applying f until value stabilizes or max iterations).
#[no_mangle]
pub extern "C" fn ffi_fixed_point(
    f: UnaryOpFn,
    initial: i32,
    max_iterations: i32,
) -> i32 {
    let mut value = initial;
    for _ in 0..max_iterations {
        let next = f(value);
        if next == value {
            return value;
        }
        value = next;
    }
    value
}

// --- Struct callbacks ---

/// Type for a callback that processes Point2D.
pub type Point2DCallbackFn = extern "C" fn(Point2D) -> Point2D;

/// Type for a callback that creates Point2D from index.
pub type Point2DGeneratorFn = extern "C" fn(i32) -> Point2D;

/// Apply a transformation to a Point2D.
#[no_mangle]
pub extern "C" fn ffi_transform_point2d(
    p: Point2D,
    f: Point2DCallbackFn,
) -> Point2D {
    f(p)
}

/// Generate an array of points using a callback.
#[no_mangle]
pub extern "C" fn ffi_generate_points(
    dst: *mut Point2D,
    len: usize,
    generator: Point2DGeneratorFn,
) {
    if dst.is_null() || len == 0 {
        return;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(dst, len);
        for (i, point) in slice.iter_mut().enumerate() {
            *point = generator(i as i32);
        }
    }
}

/// Sum of magnitude of all points using callback for magnitude.
#[no_mangle]
pub extern "C" fn ffi_sum_point_magnitudes(
    arr: *const Point2D,
    len: usize,
    magnitude_fn: extern "C" fn(Point2D) -> c_double,
) -> c_double {
    if arr.is_null() || len == 0 {
        return 0.0;
    }

    unsafe {
        let slice = slice::from_raw_parts(arr, len);
        slice.iter().map(|&p| magnitude_fn(p)).sum()
    }
}

// --- Multi-callback orchestration ---

/// State machine type for callbacks.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StateMachine {
    pub state: i32,
    pub data: *mut c_void,
}

/// State transition callback type.
pub type StateTransitionFn = extern "C" fn(*mut StateMachine, i32) -> i32;

/// Run a state machine until terminal state (negative).
#[no_mangle]
pub extern "C" fn ffi_run_state_machine(
    sm: *mut StateMachine,
    transition: StateTransitionFn,
    input: *const i32,
    input_len: usize,
) -> i32 {
    if sm.is_null() {
        return -1;
    }

    unsafe {
        if input.is_null() || input_len == 0 {
            // No input, just return current state
            return (*sm).state;
        }

        let inputs = slice::from_raw_parts(input, input_len);
        for &event in inputs {
            let new_state = transition(sm, event);
            (*sm).state = new_state;
            if new_state < 0 {
                break; // Terminal state
            }
        }

        (*sm).state
    }
}

// --- Continuation-passing style callbacks ---

/// Type for continuation callback.
pub type ContinuationFn = extern "C" fn(i32, *mut c_void);

/// Async-like computation with continuation.
#[no_mangle]
pub extern "C" fn ffi_compute_async(
    value: i32,
    cont: ContinuationFn,
    ctx: *mut c_void,
) {
    // Simulate some computation
    let result = value * 2 + 1;
    cont(result, ctx);
}

/// Chain multiple computations.
#[no_mangle]
pub extern "C" fn ffi_chain_compute(
    initial: i32,
    f1: UnaryOpFn,
    f2: UnaryOpFn,
    f3: UnaryOpFn,
) -> i32 {
    f3(f2(f1(initial)))
}

// --- Callbacks with multiple return values (via out pointers) ---

/// Type for divmod-style callback.
pub type DivModCallbackFn = extern "C" fn(i32, i32, *mut i32, *mut i32);

/// Execute divmod callback and verify results.
#[no_mangle]
pub extern "C" fn ffi_test_divmod_callback(
    a: i32,
    b: i32,
    divmod: DivModCallbackFn,
    quot_out: *mut i32,
    rem_out: *mut i32,
) {
    if b == 0 || quot_out.is_null() || rem_out.is_null() {
        return;
    }
    divmod(a, b, quot_out, rem_out);
}

// --- Comparison callbacks with different signatures ---

/// Type for comparing pointers to values.
pub type PtrComparatorFn = extern "C" fn(*const i32, *const i32) -> i32;

/// qsort-compatible sorting using pointer comparator.
#[no_mangle]
pub extern "C" fn ffi_qsort_style(
    arr: *mut i32,
    len: usize,
    cmp: PtrComparatorFn,
) {
    if arr.is_null() || len < 2 {
        return;
    }

    unsafe {
        let slice = slice::from_raw_parts_mut(arr, len);
        slice.sort_by(|a, b| {
            let result = cmp(a, b);
            if result < 0 {
                std::cmp::Ordering::Less
            } else if result > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    }
}

// =============================================================================
// Memory Allocation
// =============================================================================

/// Allocate memory (returns null-initialized).
#[no_mangle]
pub extern "C" fn ffi_alloc(size: usize) -> *mut c_void {
    if size == 0 {
        return ptr::null_mut();
    }

    let layout = match std::alloc::Layout::from_size_align(size, 8) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };

    unsafe {
        let ptr = std::alloc::alloc_zeroed(layout);
        if ptr.is_null() {
            ptr::null_mut()
        } else {
            ptr as *mut c_void
        }
    }
}

/// Allocate aligned memory.
#[no_mangle]
pub extern "C" fn ffi_alloc_aligned(size: usize, align: usize) -> *mut c_void {
    if size == 0 || !align.is_power_of_two() {
        return ptr::null_mut();
    }

    let layout = match std::alloc::Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };

    unsafe {
        let ptr = std::alloc::alloc_zeroed(layout);
        if ptr.is_null() {
            ptr::null_mut()
        } else {
            ptr as *mut c_void
        }
    }
}

/// Free memory allocated by ffi_alloc or ffi_alloc_aligned.
/// Note: This is unsafe as we don't track the layout.
/// In production, we'd use a more sophisticated allocator.
#[no_mangle]
pub extern "C" fn ffi_free(ptr: *mut c_void, size: usize) {
    if ptr.is_null() || size == 0 {
        return;
    }

    let layout = match std::alloc::Layout::from_size_align(size, 8) {
        Ok(l) => l,
        Err(_) => return,
    };

    unsafe {
        std::alloc::dealloc(ptr as *mut u8, layout);
    }
}

// =============================================================================
// Thread-Local Storage
// =============================================================================

thread_local! {
    static TLS_VALUE: RefCell<i64> = const { RefCell::new(0) };
    static TLS_DATA: RefCell<Option<*mut c_void>> = const { RefCell::new(None) };
}

/// Set thread-local value.
#[no_mangle]
pub extern "C" fn ffi_tls_set(value: i64) {
    TLS_VALUE.with(|v| *v.borrow_mut() = value);
}

/// Get thread-local value.
#[no_mangle]
pub extern "C" fn ffi_tls_get() -> i64 {
    TLS_VALUE.with(|v| *v.borrow())
}

/// Set thread-local data pointer.
#[no_mangle]
pub extern "C" fn ffi_tls_set_data(data: *mut c_void) {
    TLS_DATA.with(|d| *d.borrow_mut() = Some(data));
}

/// Get thread-local data pointer.
#[no_mangle]
pub extern "C" fn ffi_tls_get_data() -> *mut c_void {
    TLS_DATA.with(|d| d.borrow().unwrap_or(ptr::null_mut()))
}

// =============================================================================
// Error Handling
// =============================================================================

thread_local! {
    static LAST_ERROR: RefCell<i32> = const { RefCell::new(0) };
}

/// Set the last error code.
#[no_mangle]
pub extern "C" fn ffi_set_error(code: i32) {
    LAST_ERROR.with(|e| *e.borrow_mut() = code);
}

/// Get the last error code.
#[no_mangle]
pub extern "C" fn ffi_get_error() -> i32 {
    LAST_ERROR.with(|e| *e.borrow())
}

/// Clear the last error code.
#[no_mangle]
pub extern "C" fn ffi_clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = 0);
}

/// Divide with error handling.
#[no_mangle]
pub extern "C" fn ffi_div_with_error(a: i32, b: i32) -> i32 {
    if b == 0 {
        ffi_set_error(-1); // Division by zero
        0
    } else {
        ffi_clear_error();
        a / b
    }
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Get the library version.
#[no_mangle]
pub extern "C" fn ffi_get_version() -> i32 {
    // Version 1.0.0 encoded as 0x010000
    0x010000
}

/// Return a constant for testing.
#[no_mangle]
pub extern "C" fn ffi_get_magic() -> u64 {
    0xDEADBEEFCAFEBABE
}

/// Identity function (useful for testing calling convention).
#[no_mangle]
pub extern "C" fn ffi_identity_i64(x: i64) -> i64 {
    x
}

/// No-op function.
#[no_mangle]
pub extern "C" fn ffi_noop() {}

/// Sleep for milliseconds (for testing async interop).
#[no_mangle]
pub extern "C" fn ffi_sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// Get current time in nanoseconds (monotonic).
#[no_mangle]
pub extern "C" fn ffi_time_ns() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_i32() {
        assert_eq!(ffi_add_i32(10, 32), 42);
        assert_eq!(ffi_add_i32(-10, 10), 0);
        assert_eq!(ffi_add_i32(i32::MAX, 1), i32::MIN); // wrapping
    }

    #[test]
    fn test_sqrt() {
        assert!((ffi_sqrt_f64(4.0) - 2.0).abs() < 1e-10);
        assert!((ffi_sqrt_f64(2.0) - std::f64::consts::SQRT_2).abs() < 1e-10);
    }

    #[test]
    fn test_point2d() {
        let p1 = ffi_make_point2d(3, 4);
        let p2 = ffi_make_point2d(1, 1);
        let sum = ffi_add_point2d(p1, p2);
        assert_eq!(sum.x, 4);
        assert_eq!(sum.y, 5);

        let mag = ffi_point2d_magnitude(p1);
        assert!((mag - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_callback() {
        extern "C" fn add(a: i32, b: i32) -> i32 { a + b }
        extern "C" fn double(x: i32) -> i32 { x * 2 }

        assert_eq!(ffi_apply_binary(add, 10, 32), 42);
        assert_eq!(ffi_apply_unary(double, 21), 42);
    }

    #[test]
    fn test_tls() {
        ffi_tls_set(42);
        assert_eq!(ffi_tls_get(), 42);
        ffi_tls_set(100);
        assert_eq!(ffi_tls_get(), 100);
    }
}
