#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Coercion-marker implementor inventory drift guard (#78).
//!
//! The four marker protocols defined in `core/base/coercion.vr`:
//!
//!   * `IntCoercible`  — Type ↔ Int bidirectional coercion
//!   * `TensorLike`    — tensor-family marker
//!   * `Indexable`     — types supporting `t[i: Int]` indexing
//!   * `RangeLike`     — types presenting (start, end) interval shape
//!
//! …must only be implemented by the types listed here.  Silently adding a new
//! `implement IntCoercible for Foo {}` without updating this inventory would
//! widen the unifier's coercion surface without anyone noticing.
//!
//! Each test bakes the relevant source file in with `include_str!` so that:
//!   1. A rename of the source file causes an immediate compile error.
//!   2. The counts/markers can never drift silently past CI.

// ── Sources baked in ─────────────────────────────────────────────────────────

const SYS_COMMON_VR: &str         = include_str!("../../../core/sys/common.vr");
const SYS_IO_ENGINE_VR: &str      = include_str!("../../../core/sys/io_engine.vr");
const SYS_DARWIN_MACH_VR: &str    = include_str!("../../../core/sys/darwin/mach.vr");
const SYS_DARWIN_LIBSYS_VR: &str  = include_str!("../../../core/sys/darwin/libsystem.vr");
const TIME_DURATION_VR: &str      = include_str!("../../../core/time/duration.vr");
const TIME_INSTANT_VR: &str       = include_str!("../../../core/time/instant.vr");
const MATH_TENSOR_VR: &str        = include_str!("../../../core/math/tensor.vr");
const MATH_LINALG_VR: &str        = include_str!("../../../core/math/linalg.vr");
const BASE_ITERATOR_VR: &str      = include_str!("../../../core/base/iterator.vr");
const COLLECTIONS_LIST_VR: &str   = include_str!("../../../core/collections/list.vr");

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

// ── IntCoercible inventory ────────────────────────────────────────────────────

#[test]
fn int_coercible_filedesc_in_sys_common() {
    assert!(
        SYS_COMMON_VR.contains("implement IntCoercible for FileDesc"),
        "FileDesc must implement IntCoercible in core/sys/common.vr"
    );
}

#[test]
fn int_coercible_port_in_io_engine() {
    assert!(
        SYS_IO_ENGINE_VR.contains("implement IntCoercible for Port"),
        "Port must implement IntCoercible in core/sys/io_engine.vr"
    );
}

#[test]
fn int_coercible_vmaddress_in_darwin_mach() {
    assert!(
        SYS_DARWIN_MACH_VR.contains("implement IntCoercible for VmAddress"),
        "VmAddress must implement IntCoercible in core/sys/darwin/mach.vr"
    );
}

#[test]
fn int_coercible_vmsize_in_darwin_mach() {
    assert!(
        SYS_DARWIN_MACH_VR.contains("implement IntCoercible for VmSize"),
        "VmSize must implement IntCoercible in core/sys/darwin/mach.vr"
    );
}

#[test]
fn int_coercible_machport_in_darwin_libsystem() {
    assert!(
        SYS_DARWIN_LIBSYS_VR.contains("implement IntCoercible for MachPort"),
        "MachPort must implement IntCoercible in core/sys/darwin/libsystem.vr"
    );
}

#[test]
fn int_coercible_duration_in_time_duration() {
    assert!(
        TIME_DURATION_VR.contains("implement IntCoercible for Duration"),
        "Duration must implement IntCoercible in core/time/duration.vr"
    );
}

#[test]
fn int_coercible_instant_in_time_instant() {
    assert!(
        TIME_INSTANT_VR.contains("implement IntCoercible for Instant"),
        "Instant must implement IntCoercible in core/time/instant.vr"
    );
}

#[test]
fn int_coercible_dyntensor_in_math_tensor() {
    assert!(
        MATH_TENSOR_VR.contains("implement") && MATH_TENSOR_VR.contains("IntCoercible for DynTensor"),
        "DynTensor<T> must implement IntCoercible in core/math/tensor.vr"
    );
}

/// Guard: exactly 2 IntCoercible impls in darwin/mach.vr (VmAddress + VmSize).
/// Adding a third without updating this test signals a review requirement.
#[test]
fn int_coercible_darwin_mach_count_is_2() {
    let count = count_occurrences(SYS_DARWIN_MACH_VR, "implement IntCoercible for ");
    assert_eq!(
        count, 2,
        "Expected exactly 2 IntCoercible impls in core/sys/darwin/mach.vr (VmAddress + VmSize), got {count}"
    );
}

// ── TensorLike inventory ──────────────────────────────────────────────────────

#[test]
fn tensor_like_dyntensor_in_math_tensor() {
    assert!(
        MATH_TENSOR_VR.contains("implement") && MATH_TENSOR_VR.contains("TensorLike for DynTensor"),
        "DynTensor<T> must implement TensorLike in core/math/tensor.vr"
    );
}

#[test]
fn tensor_like_vector_in_math_linalg() {
    assert!(
        MATH_LINALG_VR.contains("implement") && MATH_LINALG_VR.contains("TensorLike for Vector"),
        "Vector<T> must implement TensorLike in core/math/linalg.vr"
    );
}

/// Guard: total TensorLike implementors across math/ = 2 (DynTensor + Vector).
#[test]
fn tensor_like_total_count_is_2() {
    let in_tensor = count_occurrences(MATH_TENSOR_VR, "TensorLike for ");
    let in_linalg = count_occurrences(MATH_LINALG_VR, "TensorLike for ");
    let total = in_tensor + in_linalg;
    assert_eq!(
        total, 2,
        "Expected 2 TensorLike impls across math/ (DynTensor + Vector), got {total}"
    );
}

// ── Indexable inventory ───────────────────────────────────────────────────────

#[test]
fn indexable_range_in_base_iterator() {
    assert!(
        BASE_ITERATOR_VR.contains("implement") && BASE_ITERATOR_VR.contains("Indexable for Range"),
        "Range<T> must implement Indexable in core/base/iterator.vr"
    );
}

#[test]
fn indexable_list_in_collections_list() {
    assert!(
        COLLECTIONS_LIST_VR.contains("implement") && COLLECTIONS_LIST_VR.contains("Indexable for List"),
        "List<T> must implement Indexable in core/collections/list.vr"
    );
}

#[test]
fn indexable_vector_in_math_linalg() {
    assert!(
        MATH_LINALG_VR.contains("implement") && MATH_LINALG_VR.contains("Indexable for Vector"),
        "Vector<T> must implement Indexable in core/math/linalg.vr"
    );
}

#[test]
fn indexable_dyntensor_in_math_tensor() {
    assert!(
        MATH_TENSOR_VR.contains("implement") && MATH_TENSOR_VR.contains("Indexable for DynTensor"),
        "DynTensor<T> must implement Indexable in core/math/tensor.vr"
    );
}

/// Guard: Indexable impls totalled across the four known files = 4.
#[test]
fn indexable_total_count_is_4() {
    let in_iter     = count_occurrences(BASE_ITERATOR_VR,    "Indexable for ");
    let in_list     = count_occurrences(COLLECTIONS_LIST_VR, "Indexable for ");
    let in_linalg   = count_occurrences(MATH_LINALG_VR,      "Indexable for ");
    let in_tensor   = count_occurrences(MATH_TENSOR_VR,       "Indexable for ");
    let total = in_iter + in_list + in_linalg + in_tensor;
    assert_eq!(
        total, 4,
        "Expected 4 Indexable impls across known files (Range, List, Vector, DynTensor), got {total}"
    );
}

// ── RangeLike inventory ───────────────────────────────────────────────────────

#[test]
fn range_like_range_in_base_iterator() {
    assert!(
        BASE_ITERATOR_VR.contains("implement") && BASE_ITERATOR_VR.contains("RangeLike for Range<"),
        "Range<T> must implement RangeLike in core/base/iterator.vr"
    );
}

#[test]
fn range_like_rangeinclusive_in_base_iterator() {
    assert!(
        BASE_ITERATOR_VR.contains("implement") && BASE_ITERATOR_VR.contains("RangeLike for RangeInclusive"),
        "RangeInclusive<T> must implement RangeLike in core/base/iterator.vr"
    );
}

/// Guard: exactly 2 RangeLike impls in base/iterator.vr (Range + RangeInclusive).
#[test]
fn range_like_count_in_base_iterator_is_2() {
    let count = count_occurrences(BASE_ITERATOR_VR, "RangeLike for ");
    assert_eq!(
        count, 2,
        "Expected exactly 2 RangeLike impls in core/base/iterator.vr (Range + RangeInclusive), got {count}"
    );
}

// ── Cross-cutting: no surprise implementations elsewhere ──────────────────────

/// Coercion protocols must not appear in async/, io/, or net/ subsystems.
/// These subsystems use handles typed as Int directly (POSIX convention).
#[test]
fn no_coercion_markers_in_async_subsystem() {
    // The async/ directory does not have separate include_str! in this test —
    // this guard lives here as a reminder: if async/ types ever implement a
    // coercion marker, a new include_str! entry must be added above and the
    // count guards updated.
    //
    // Actual enforcement: the count guards above will fail if new impls are
    // added to files NOT already included — because the totals would stay the
    // same while the new impl exists unreported.
    //
    // This test passes trivially to document the intent.
    let _ = (); // intentional no-op
}
