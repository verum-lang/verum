#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! debug_assert @cfg dispatch validator (#51).
//!
//! `core/base/panic.vr` defines `debug_assert`, `debug_assert_eq`, and
//! `debug_assert_ne` as **two separate overloads** gated by `@cfg`:
//!
//!   * `@cfg(debug)` version — calls `assert(...)` so it panics in debug builds
//!   * `@cfg(not(debug))` version — no-op (`_condition` / `_left` / `_right`)
//!
//! This validator pins:
//!   1. Both `@cfg(debug)` and `@cfg(not(debug))` overloads exist for each function.
//!   2. The debug overload's body actually calls the underlying `assert` / `assert_eq`
//!      / `assert_ne` primitive, not just returns silently.
//!   3. The release overload's parameter names start with `_` (compiler hint for
//!      intentional no-op: `_condition`, `_left`, `_right`, `_msg`).
//!   4. The three function names are pinned (no accidental rename without a drift alert).
//!
//! Baking the source in with `include_str!` means the test will fail immediately
//! if the file is renamed or if either overload is removed.

const PANIC_VR: &str = include_str!("../../../core/base/panic.vr");

fn count_occurrences(src: &str, pattern: &str) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = src[start..].find(pattern) {
        count += 1;
        start += pos + pattern.len();
    }
    count
}

/// Returns the text of the function body (from `{` to the matching `}`)
/// following `header` in `src`, or an empty string if not found.
fn extract_fn_body<'a>(src: &'a str, header: &str) -> &'a str {
    let Some(h_pos) = src.find(header) else { return "" };
    let after = &src[h_pos + header.len()..];
    let Some(open) = after.find('{') else { return "" };
    let body_start = h_pos + header.len() + open;
    // Walk forward to find the matching closing brace.
    let body_src = &src[body_start..];
    let mut depth = 0usize;
    for (i, ch) in body_src.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &body_src[..i + 1];
                }
            }
            _ => {}
        }
    }
    ""
}

// =============================================================================
// 1. Both @cfg overloads exist for debug_assert
// =============================================================================

#[test]
fn debug_assert_has_cfg_debug_overload() {
    assert!(
        PANIC_VR.contains("@cfg(debug)\npublic fn debug_assert("),
        "debug_assert must have a @cfg(debug) overload in panic.vr"
    );
}

#[test]
fn debug_assert_has_cfg_not_debug_overload() {
    assert!(
        PANIC_VR.contains("@cfg(not(debug))\npublic fn debug_assert("),
        "debug_assert must have a @cfg(not(debug)) no-op overload in panic.vr"
    );
}

// =============================================================================
// 2. Both @cfg overloads exist for debug_assert_eq
// =============================================================================

#[test]
fn debug_assert_eq_has_cfg_debug_overload() {
    assert!(
        PANIC_VR.contains("@cfg(debug)\npublic fn debug_assert_eq<"),
        "debug_assert_eq must have a @cfg(debug) overload in panic.vr"
    );
}

#[test]
fn debug_assert_eq_has_cfg_not_debug_overload() {
    assert!(
        PANIC_VR.contains("@cfg(not(debug))\npublic fn debug_assert_eq<"),
        "debug_assert_eq must have a @cfg(not(debug)) no-op overload in panic.vr"
    );
}

// =============================================================================
// 3. Both @cfg overloads exist for debug_assert_ne
// =============================================================================

#[test]
fn debug_assert_ne_has_cfg_debug_overload() {
    assert!(
        PANIC_VR.contains("@cfg(debug)\npublic fn debug_assert_ne<"),
        "debug_assert_ne must have a @cfg(debug) overload in panic.vr"
    );
}

#[test]
fn debug_assert_ne_has_cfg_not_debug_overload() {
    assert!(
        PANIC_VR.contains("@cfg(not(debug))\npublic fn debug_assert_ne<"),
        "debug_assert_ne must have a @cfg(not(debug)) no-op overload in panic.vr"
    );
}

// =============================================================================
// 4. Debug bodies delegate to the underlying assert primitive
// =============================================================================

#[test]
fn debug_assert_debug_body_calls_assert() {
    // Find the @cfg(debug) overload body.
    let header = "@cfg(debug)\npublic fn debug_assert(condition: Bool,";
    let body = extract_fn_body(PANIC_VR, header);
    assert!(
        !body.is_empty(),
        "could not extract @cfg(debug) debug_assert body"
    );
    assert!(
        body.contains("assert(condition"),
        "@cfg(debug) debug_assert must call assert(condition, ...), got: {body}"
    );
}

#[test]
fn debug_assert_eq_debug_body_calls_assert_eq() {
    let header = "@cfg(debug)\npublic fn debug_assert_eq<";
    let body = extract_fn_body(PANIC_VR, header);
    assert!(!body.is_empty(), "could not extract @cfg(debug) debug_assert_eq body");
    assert!(
        body.contains("assert_eq(left"),
        "@cfg(debug) debug_assert_eq must call assert_eq(left, ...), got: {body}"
    );
}

#[test]
fn debug_assert_ne_debug_body_calls_assert_ne() {
    let header = "@cfg(debug)\npublic fn debug_assert_ne<";
    let body = extract_fn_body(PANIC_VR, header);
    assert!(!body.is_empty(), "could not extract @cfg(debug) debug_assert_ne body");
    assert!(
        body.contains("assert_ne(left"),
        "@cfg(debug) debug_assert_ne must call assert_ne(left, ...), got: {body}"
    );
}

// =============================================================================
// 5. Release bodies use _ prefixed params (intentional no-op)
// =============================================================================

#[test]
fn debug_assert_release_body_uses_ignored_param() {
    let header = "@cfg(not(debug))\npublic fn debug_assert(_condition: Bool,";
    assert!(
        PANIC_VR.contains(header),
        "release debug_assert must have `_condition` (ignored) parameter, not `condition`"
    );
}

#[test]
fn debug_assert_eq_release_signature_uses_ignored_params() {
    // The release no-op has `_left` and `_right` in its SIGNATURE (parameter list),
    // not in the empty body `{}`.
    let header = "@cfg(not(debug))\npublic fn debug_assert_eq<";
    let Some(h_pos) = PANIC_VR.find(header) else {
        panic!("release debug_assert_eq not found in panic.vr");
    };
    // Grab text up to the opening brace of the body.
    let sig_slice = &PANIC_VR[h_pos..];
    let sig_end = sig_slice.find('{').expect("no body brace found");
    let sig = &sig_slice[..sig_end];
    assert!(
        sig.contains("_left") && sig.contains("_right"),
        "release debug_assert_eq signature must use _left/_right (ignored params), got: {sig}"
    );
}

#[test]
fn debug_assert_ne_release_signature_uses_ignored_params() {
    let header = "@cfg(not(debug))\npublic fn debug_assert_ne<";
    let Some(h_pos) = PANIC_VR.find(header) else {
        panic!("release debug_assert_ne not found in panic.vr");
    };
    let sig_slice = &PANIC_VR[h_pos..];
    let sig_end = sig_slice.find('{').expect("no body brace found");
    let sig = &sig_slice[..sig_end];
    assert!(
        sig.contains("_left") && sig.contains("_right"),
        "release debug_assert_ne signature must use _left/_right (ignored params), got: {sig}"
    );
}

// =============================================================================
// 6. Exactly two overloads per function name (one debug, one not)
// =============================================================================

#[test]
fn debug_assert_overload_count_is_exactly_two() {
    let count = count_occurrences(PANIC_VR, "public fn debug_assert(");
    assert_eq!(
        count, 2,
        "expected exactly 2 debug_assert overloads (debug + not-debug), found {count}"
    );
}

#[test]
fn debug_assert_eq_overload_count_is_exactly_two() {
    let count = count_occurrences(PANIC_VR, "public fn debug_assert_eq<");
    assert_eq!(
        count, 2,
        "expected exactly 2 debug_assert_eq overloads, found {count}"
    );
}

#[test]
fn debug_assert_ne_overload_count_is_exactly_two() {
    let count = count_occurrences(PANIC_VR, "public fn debug_assert_ne<");
    assert_eq!(
        count, 2,
        "expected exactly 2 debug_assert_ne overloads, found {count}"
    );
}
