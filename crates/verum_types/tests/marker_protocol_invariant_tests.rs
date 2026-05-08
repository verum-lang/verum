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
//! A *marker protocol* in Verum is one whose declaration has an empty body
//! (`{}`).  Marker protocols convey properties through the type system without
//! adding new required methods — they are essentially capability tags.
//!
//! The invariant: the set of known marker protocols in the core stdlib must
//! remain marker protocols.  If someone accidentally converts a marker to a
//! method-bearing protocol (or adds a method to a currently empty body), that
//! changes the conformance burden for all implementors silently.
//!
//! Tests check:
//!   1. Each known marker protocol still appears as a single-line `{};` form.
//!   2. No `fn ` declaration appears between `type X is protocol` and `};` for
//!      any listed marker.
//!   3. The total count of single-line marker protocols in `protocols.vr` is
//!      pinned so that accidental additions are also caught.
//!
//! Covered protocols (from `core/base/protocols.vr`):
//!   Sized, Send, Sync, Unpin                                    (root markers)
//!   Eq (extends PartialEq), Copy (extends Clone)                (refinement markers)
//!   Atomic, Integer, SignedInteger, Numeric, SignedNumeric       (numeric hierarchy)
//!
//! Covered protocols (from `core/base/iterator.vr`):
//!   FusedIterator (extends Iterator)                            (iterator marker)

const PROTOCOLS_SRC: &str = include_str!("../../../core/base/protocols.vr");
const ITERATOR_SRC: &str = include_str!("../../../core/base/iterator.vr");

// ─── helper ──────────────────────────────────────────────────────────────────

/// Returns the text of the protocol block starting at the first occurrence of
/// `header` and ending at the first `};` after it.  Panics if not found.
fn extract_protocol_block<'a>(src: &'a str, header: &str) -> &'a str {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("protocol header not found: `{header}`"));
    let after = &src[start..];
    let end = after
        .find("};")
        .unwrap_or_else(|| panic!("closing `}};` not found after `{header}`"));
    &after[..end]
}

// ─── Root markers ─────────────────────────────────────────────────────────────

/// `Sized`, `Send`, `Sync`, `Unpin` are the four foundational marker protocols.
/// They carry no required methods; the compiler enforces the constraint purely
/// via the presence of the `implement X for T {}` block.
#[test]
fn root_markers_have_zero_method_declarations() {
    let root_markers = [
        ("Sized", "type Sized is protocol {}"),
        ("Send",  "type Send is protocol {}"),
        ("Sync",  "type Sync is protocol {}"),
        ("Unpin", "type Unpin is protocol {}"),
    ];
    for (name, expected_line) in &root_markers {
        assert!(
            PROTOCOLS_SRC.contains(expected_line),
            "{name} must appear as a single-line empty-body protocol `{expected_line}`",
        );
        let block = extract_protocol_block(PROTOCOLS_SRC, &format!("type {name} is protocol"));
        assert_eq!(
            block.matches("fn ").count(), 0,
            "{name} is a marker protocol — must have 0 method declarations",
        );
    }
}

// ─── Refinement markers ───────────────────────────────────────────────────────

/// `Eq` and `Copy` are protocols that inherit method requirements from a super
/// protocol but add none of their own.  The inherited requirements (`eq`,
/// `clone`) are satisfied transitively; no additional method body is needed.
#[test]
fn refinement_markers_have_zero_own_method_declarations() {
    let refinement_markers = [
        ("Eq",   "type Eq is protocol extends PartialEq {}"),
        ("Copy", "type Copy is protocol extends Clone {}"),
    ];
    for (name, expected_line) in &refinement_markers {
        assert!(
            PROTOCOLS_SRC.contains(expected_line),
            "{name} must appear as a single-line empty-body protocol `{expected_line}`",
        );
        let block = extract_protocol_block(PROTOCOLS_SRC, &format!("type {name} is protocol"));
        assert_eq!(
            block.matches("fn ").count(), 0,
            "{name} is a refinement marker — must have 0 own method declarations",
        );
    }
}

// ─── Numeric hierarchy markers ────────────────────────────────────────────────

/// The five numeric-hierarchy marker protocols must remain empty-body protocols.
/// Adding methods here would require ALL existing implementors to provide bodies.
#[test]
fn numeric_hierarchy_markers_have_zero_method_declarations() {
    let numeric_markers = [
        ("Atomic",       "type Atomic is protocol extends Copy + Sized {}"),
        ("Integer",      "type Integer is protocol extends Atomic {}"),
        ("SignedInteger","type SignedInteger is protocol extends Integer {}"),
        ("Numeric",      "type Numeric is protocol extends Copy + Sized {}"),
        ("SignedNumeric", "type SignedNumeric is protocol extends Numeric {}"),
    ];
    for (name, expected_line) in &numeric_markers {
        assert!(
            PROTOCOLS_SRC.contains(expected_line),
            "{name} must appear as a single-line empty-body protocol `{expected_line}`",
        );
        let block = extract_protocol_block(PROTOCOLS_SRC, &format!("type {name} is protocol"));
        assert_eq!(
            block.matches("fn ").count(), 0,
            "{name} is a numeric-hierarchy marker — must have 0 method declarations",
        );
    }
}

// ─── Iterator marker ──────────────────────────────────────────────────────────

/// `FusedIterator` tags iterators that are guaranteed to return `None` forever
/// once exhausted.  It must remain a zero-method marker.
#[test]
fn fused_iterator_is_a_zero_method_marker() {
    let expected_line = "type FusedIterator is protocol extends Iterator {}";
    assert!(
        ITERATOR_SRC.contains(expected_line),
        "FusedIterator must appear as single-line empty-body protocol `{expected_line}`",
    );
    let block = extract_protocol_block(ITERATOR_SRC, "type FusedIterator is protocol");
    assert_eq!(
        block.matches("fn ").count(), 0,
        "FusedIterator is a marker protocol — must have 0 method declarations",
    );
}

// ─── Total count pins ─────────────────────────────────────────────────────────

/// Pins the total number of single-line marker protocols declared in
/// `protocols.vr`.  Any new marker added without updating this count will be
/// caught; any marker promoted to a non-marker will also be caught by the
/// individual tests above.
#[test]
fn protocols_vr_total_single_line_marker_count_is_pinned() {
    // Count lines that contain `is protocol {};` (no-super markers)
    // or `is protocol extends ... {};` (super markers).
    // Both patterns end with `{};` on the same line as the header.
    let no_super: usize = PROTOCOLS_SRC
        .lines()
        .filter(|l| l.contains("is protocol {};") && !l.trim_start().starts_with("//"))
        .count();
    let with_super: usize = PROTOCOLS_SRC
        .lines()
        .filter(|l| l.contains("is protocol extends") && l.contains("{};") && !l.trim_start().starts_with("//"))
        .count();
    // Root markers (4) + Eq + Copy + Atomic + Integer + SignedInteger + Numeric + SignedNumeric = 11
    assert_eq!(
        no_super, 4,
        "protocols.vr must have exactly 4 root (no-super) marker protocols, found {no_super}",
    );
    assert_eq!(
        with_super, 7,
        "protocols.vr must have exactly 7 extends-only marker protocols, found {with_super}",
    );
}
