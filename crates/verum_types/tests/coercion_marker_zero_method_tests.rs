#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Marker-protocol zero-method invariant validator (#41).
//!
//! The four coercion-marker protocols defined in `core/base/coercion.vr`:
//!
//!   * `IntCoercible`  — `public type IntCoercible is protocol {};`
//!   * `TensorLike`    — `public type TensorLike is protocol {};`
//!   * `Indexable`     — `public type Indexable is protocol {};`
//!   * `RangeLike`     — `public type RangeLike is protocol {};`
//!
//! …are MARKER protocols.  They carry NO methods.  The compiler only cares
//! that an `implement X for T {}` block exists; the protocol body must remain
//! empty forever.
//!
//! Rationale: a method on a marker protocol would force every implementor to
//! provide an impl body, breaking the one-line opt-in contract and making the
//! compiler's "is this type in the coercion set" check depend on method
//! resolution rather than presence alone.
//!
//! Each test verifies:
//!   1. The protocol declaration uses the `is protocol {}` form (empty body).
//!   2. No `fn ` or `type ` items exist inside the protocol body.
//!   3. The protocol name appears exactly once as a `public type X is protocol`.

const COERCION_VR: &str = include_str!("../../../core/base/coercion.vr");

// ── Helpers ───────────────────────────────────────────────────────────────────

fn count_occurrences(src: &str, pattern: &str) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = src[start..].find(pattern) {
        count += 1;
        start += pos + pattern.len();
    }
    count
}

/// Extracts the text between `{` and matching `}` that immediately follows
/// `header` in `src`.  Returns `""` if not found.
fn extract_protocol_body<'a>(src: &'a str, header: &str) -> &'a str {
    let Some(h_pos) = src.find(header) else { return "" };
    let after = &src[h_pos + header.len()..];
    let Some(open_rel) = after.find('{') else { return "" };
    let body_start = h_pos + header.len() + open_rel + 1; // after '{'
    let body_src = &src[body_start..];
    let mut depth = 1usize;
    let mut end = 0;
    for (i, ch) in body_src.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    body_src[..end].trim()
}

// ── IntCoercible ──────────────────────────────────────────────────────────────

#[test]
fn int_coercible_is_protocol_declaration_present() {
    assert!(
        COERCION_VR.contains("public type IntCoercible is protocol"),
        "IntCoercible must be declared as 'public type IntCoercible is protocol'"
    );
}

#[test]
fn int_coercible_declared_exactly_once() {
    let count = count_occurrences(COERCION_VR, "public type IntCoercible is protocol");
    assert_eq!(
        count, 1,
        "Expected exactly one declaration of IntCoercible, got {count}"
    );
}

#[test]
fn int_coercible_body_has_no_fn() {
    let body = extract_protocol_body(COERCION_VR, "public type IntCoercible is protocol");
    assert!(
        !body.contains("fn "),
        "IntCoercible protocol body must contain no methods; found 'fn ' in: {body:?}"
    );
}

#[test]
fn int_coercible_body_has_no_type_item() {
    let body = extract_protocol_body(COERCION_VR, "public type IntCoercible is protocol");
    // "type " in the body would mean an associated type — not allowed in a marker.
    assert!(
        !body.contains("type "),
        "IntCoercible protocol body must contain no associated types; found 'type ' in: {body:?}"
    );
}

// ── TensorLike ────────────────────────────────────────────────────────────────

#[test]
fn tensor_like_is_protocol_declaration_present() {
    assert!(
        COERCION_VR.contains("public type TensorLike is protocol"),
        "TensorLike must be declared as 'public type TensorLike is protocol'"
    );
}

#[test]
fn tensor_like_declared_exactly_once() {
    let count = count_occurrences(COERCION_VR, "public type TensorLike is protocol");
    assert_eq!(count, 1, "Expected exactly one declaration of TensorLike, got {count}");
}

#[test]
fn tensor_like_body_has_no_fn() {
    let body = extract_protocol_body(COERCION_VR, "public type TensorLike is protocol");
    assert!(
        !body.contains("fn "),
        "TensorLike protocol body must contain no methods; found 'fn ' in: {body:?}"
    );
}

// ── Indexable ─────────────────────────────────────────────────────────────────

#[test]
fn indexable_is_protocol_declaration_present() {
    assert!(
        COERCION_VR.contains("public type Indexable is protocol"),
        "Indexable must be declared as 'public type Indexable is protocol'"
    );
}

#[test]
fn indexable_declared_exactly_once() {
    let count = count_occurrences(COERCION_VR, "public type Indexable is protocol");
    assert_eq!(count, 1, "Expected exactly one declaration of Indexable, got {count}");
}

#[test]
fn indexable_body_has_no_fn() {
    let body = extract_protocol_body(COERCION_VR, "public type Indexable is protocol");
    assert!(
        !body.contains("fn "),
        "Indexable protocol body must contain no methods; found 'fn ' in: {body:?}"
    );
}

// ── RangeLike ─────────────────────────────────────────────────────────────────

#[test]
fn range_like_is_protocol_declaration_present() {
    assert!(
        COERCION_VR.contains("public type RangeLike is protocol"),
        "RangeLike must be declared as 'public type RangeLike is protocol'"
    );
}

#[test]
fn range_like_declared_exactly_once() {
    let count = count_occurrences(COERCION_VR, "public type RangeLike is protocol");
    assert_eq!(count, 1, "Expected exactly one declaration of RangeLike, got {count}");
}

#[test]
fn range_like_body_has_no_fn() {
    let body = extract_protocol_body(COERCION_VR, "public type RangeLike is protocol");
    assert!(
        !body.contains("fn "),
        "RangeLike protocol body must contain no methods; found 'fn ' in: {body:?}"
    );
}

// ── Cross-cutting: total marker count ────────────────────────────────────────

/// Exactly 4 marker protocols exist in coercion.vr — no secret fifth marker.
#[test]
fn exactly_four_marker_protocols_in_coercion_vr() {
    let count = count_occurrences(COERCION_VR, "is protocol {}");
    assert_eq!(
        count, 4,
        "Expected exactly 4 marker protocol definitions in coercion.vr, got {count}"
    );
}

/// All four protocol names appear in the doc header section (lines 1-55).
/// If one is removed from the header but kept as a live type, this catches it.
#[test]
fn all_four_markers_documented_in_header() {
    let header_region = &COERCION_VR[..COERCION_VR.len().min(2500)];
    for name in &["IntCoercible", "TensorLike", "Indexable", "RangeLike"] {
        assert!(
            header_region.contains(name),
            "Marker protocol {name} must be mentioned in the file header/doc region"
        );
    }
}
