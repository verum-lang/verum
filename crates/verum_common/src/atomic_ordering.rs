//! Single source of truth for atomic memory ordering values.
//!
//! These values mirror `std::sync::atomic::Ordering` semantics and
//! the canonical Verum stdlib declarations in
//! `core/intrinsics/atomic.vr`. They are **target-independent** —
//! every supported architecture / OS uses the same numeric encoding
//! since this is a Verum-internal protocol between codegen-emitted
//! atomic instructions and the runtime atomic dispatcher, not a
//! syscall-ABI boundary.
//!
//! Codegen emits `LoadI(ORDERING_*)` to materialise an ordering value
//! at runtime; the interpreter's atomic-op handlers read it and
//! invoke the matching `std::sync::atomic` operation.

/// Relaxed ordering — no synchronization or ordering constraints.
/// Only the operation's atomicity is guaranteed.
pub const ORDERING_RELAXED: i64 = 0;

/// Acquire ordering — establishes a happens-before relation with
/// every prior `Release` store on the same atomic.
pub const ORDERING_ACQUIRE: i64 = 1;

/// Release ordering — every prior memory operation in this thread
/// is visible to threads doing an `Acquire` load.
pub const ORDERING_RELEASE: i64 = 2;

/// AcqRel ordering — combines `Acquire` (for loads) with `Release`
/// (for stores) on the same operation. Used by RMW (read-modify-write)
/// primitives like `fetch_add` / `compare_exchange`.
pub const ORDERING_ACQ_REL: i64 = 3;

/// Sequentially consistent ordering — strongest guarantee. All
/// SeqCst operations across all threads observe a single global
/// total order. Slowest but always correct.
pub const ORDERING_SEQ_CST: i64 = 4;

/// Resolve an atomic-ordering name to its canonical integer value.
/// Returns `None` for unknown names.
///
/// Used by codegen `resolve_stdlib_constant_value` and by atomic-op
/// dispatchers in the runtime.
pub fn ordering_value(name: &str) -> Option<i64> {
    match name {
        "ORDERING_RELAXED" => Some(ORDERING_RELAXED),
        "ORDERING_ACQUIRE" => Some(ORDERING_ACQUIRE),
        "ORDERING_RELEASE" => Some(ORDERING_RELEASE),
        "ORDERING_ACQ_REL" => Some(ORDERING_ACQ_REL),
        "ORDERING_SEQ_CST" => Some(ORDERING_SEQ_CST),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Atomic ordering values stay pinned at the canonical 0..4
    /// progression. These mirror `std::sync::atomic::Ordering`
    /// discriminant order and the stdlib `core/intrinsics/atomic.vr`
    /// declarations.
    #[test]
    fn atomic_ordering_values_pinned() {
        assert_eq!(ORDERING_RELAXED, 0);
        assert_eq!(ORDERING_ACQUIRE, 1);
        assert_eq!(ORDERING_RELEASE, 2);
        assert_eq!(ORDERING_ACQ_REL, 3);
        assert_eq!(ORDERING_SEQ_CST, 4);
    }

    /// Ordering values are strictly increasing — captures the
    /// "stronger ordering = larger value" invariant codegen relies
    /// on for integer-comparison-based ordering analysis.
    #[test]
    fn ordering_values_monotonic() {
        assert!(ORDERING_RELAXED < ORDERING_ACQUIRE);
        assert!(ORDERING_ACQUIRE < ORDERING_RELEASE);
        assert!(ORDERING_RELEASE < ORDERING_ACQ_REL);
        assert!(ORDERING_ACQ_REL < ORDERING_SEQ_CST);
    }

    /// Lookup helper resolves all canonical names + rejects unknowns.
    #[test]
    fn ordering_value_lookup() {
        assert_eq!(ordering_value("ORDERING_RELAXED"), Some(0));
        assert_eq!(ordering_value("ORDERING_ACQUIRE"), Some(1));
        assert_eq!(ordering_value("ORDERING_RELEASE"), Some(2));
        assert_eq!(ordering_value("ORDERING_ACQ_REL"), Some(3));
        assert_eq!(ordering_value("ORDERING_SEQ_CST"), Some(4));
        // Unknown names return None.
        assert_eq!(ordering_value("ORDERING_NONE"), None);
        assert_eq!(ordering_value("RELAXED"), None);
        assert_eq!(ordering_value(""), None);
    }
}
