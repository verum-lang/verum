//! Stdlib coercion-protocol registry — temporary scaffold pending
//! `#101` protocol-based replacement.
//!
//! # Background
//!
//! `verum_types/src/CLAUDE.md` lays down a hard architectural rule:
//!
//! > **NEVER hardcode stdlib/core type knowledge in the compiler.**
//! > The compiler implementation (verum_types, verum_vbc, etc.) must
//! > have ZERO knowledge of stdlib (`core/`) types.
//!
//! Until protocol-based discovery is wired up (each stdlib type
//! declares `implement IntCoercible / TensorLike / Indexable /
//! RangeLike for X` and the compiler scans implement-blocks to drive
//! unifier registration), we centralise the hardcoded scaffolding
//! HERE so:
//!
//!   * The violation is contained in one identifiable location, not
//!     scattered through `pipeline.rs` Pass 5.5.
//!   * A grep for the protocol name (`int_coercible_stdlib_names`,
//!     etc.) shows the entire list of stdlib types the compiler
//!     "knows about" — no hidden sites to forget when ripping the
//!     list out.
//!   * The follow-up replacement (#101) only has to delete this
//!     module and replace `register_stdlib_coercions` with a
//!     protocol-walking pass.
//!
//! # Migration plan (for #101 follow-up)
//!
//! Step 1: define the four coercion protocols in `core/base/coercion.vr`:
//!
//! ```verum
//! public type IntCoercible is protocol {};
//! public type TensorLike is protocol { /* tensor-shape methods */ };
//! public type Indexable is protocol { fn index(&self, i: Int) -> _; };
//! public type RangeLike is protocol { fn start(&self) -> Int; fn end(&self) -> Int; };
//! ```
//!
//! Step 2: have stdlib types `implement` the relevant protocol(s):
//!
//! ```verum
//! implement IntCoercible for Duration {};
//! implement IntCoercible for Port {};
//! implement TensorLike for DynTensor<T> { /* ... */ };
//! ```
//!
//! Step 3: in pipeline.rs replace the call to `register_stdlib_coercions`
//! with a pass that walks `module.items` for `ItemKind::Impl(impl_decl)`
//! whose `protocol_path` matches one of the four protocols, and calls
//! the corresponding `register_*_type` method on the unifier with the
//! impl block's target type name.
//!
//! Step 4: delete this module.

/// Stdlib type names that participate in tensor-family coercions.
/// Becomes obsolete once `TensorLike` protocol is wired (#101 step 3).
pub const TENSOR_FAMILY_STDLIB_NAMES: &[&str] = &[
    "DynTensor",
    "Tensor",
    "Vector",
    "Cotangent",
    "Tangent",
];

/// Stdlib type names that support integer indexing.
/// Becomes obsolete once `Indexable` protocol is wired (#101 step 3).
pub const INDEXABLE_STDLIB_NAMES: &[&str] = &[
    "Range",
    "Slice",
];

/// Stdlib type names that match the range-like shape (start, end).
/// Becomes obsolete once `RangeLike` protocol is wired (#101 step 3).
pub const RANGE_LIKE_STDLIB_NAMES: &[&str] = &[
    "Range",
];

/// Stdlib type names that cross-coerce with `Int` in unification.
///
/// Categories:
///   * Scalar wrappers (Duration, Instant, Epoch) — sized-numeric
///     value types where the underlying representation is i64
///   * FFI handles (Port, FileDesc, MachPort, VmAddress, VmSize,
///     ClockId, ...) — POSIX/syscall integer typedefs
///   * Bitflags (MemProt, MapFlags, Sockaddr) — packed-bit i64
///   * Path types (Path, PathBuf) — coerce because indexing/slicing
///     produces Int, not because they're integers themselves
///   * Resource handles (GPUBuffer, DeviceRegistry, ProcessGroup) —
///     opaque-i64 handles
///   * Tensor family (DynTensor, Tensor, Vector) — for index ops
///   * Container family (List, Range, Slice, Maybe, Lazy, Once) —
///     for length/index coercion targeting Int
///
/// Becomes obsolete once `IntCoercible` protocol is wired (#101 step 3).
pub const INT_COERCIBLE_STDLIB_NAMES: &[&str] = &[
    // FFI integer typedefs
    "Port", "FileDesc", "MachPort", "VmAddress", "VmSize",
    "Timespec", "TimeSpec", "ClockId",
    // Bitflags / sockaddr
    "MemProt", "MapFlags", "Sockaddr", "Path", "PathBuf",
    // Resource handles
    "GPUBuffer", "DeviceRegistry", "ProcessGroup",
    // Time/duration scalars
    "Duration", "Instant", "Epoch",
    // Tensor family (for index ops)
    "DynTensor", "Tensor", "Vector",
    // Containers (for len/index coercion)
    "List", "Range", "Slice", "Maybe", "Lazy", "Once",
];

/// Stdlib type names that cross-coerce with each other in
/// Named<->Named unification (e.g. Duration ↔ Instant via i64 backing
/// representation). Language-level numeric primitives (Int8 / U64 /
/// Float64) live in `unify.rs::Unifier::new` and are NOT included
/// here — those are part of the language definition, not stdlib.
///
/// Becomes obsolete once `Numeric` protocol query lands (separate
/// follow-up — `Numeric` exists at `core/base/protocols.vr` but isn't
/// queryable from the unifier yet).
pub const SIZED_NUMERIC_STDLIB_NAMES: &[&str] = &[
    "Duration",
    "Instant",
    "Epoch",
];

/// Register every stdlib type in the four lists above with the
/// unifier. Single entry point so callers in `pipeline.rs` Pass 5.5
/// don't see any hardcoded names.
///
/// When #101 protocol-based discovery lands, this function's body
/// gets replaced by a `walk_implement_blocks_and_register` pass —
/// the call sites stay the same.
pub fn register_stdlib_coercions(unifier: &mut verum_types::unify::Unifier) {
    for name in TENSOR_FAMILY_STDLIB_NAMES {
        unifier.register_tensor_family_type(verum_common::Text::from(*name));
    }
    for name in INDEXABLE_STDLIB_NAMES {
        unifier.register_indexable_type(verum_common::Text::from(*name));
    }
    for name in RANGE_LIKE_STDLIB_NAMES {
        unifier.register_range_like_type(verum_common::Text::from(*name));
    }
    for name in SIZED_NUMERIC_STDLIB_NAMES {
        unifier.register_sized_numeric_type(verum_common::Text::from(*name));
    }
    for name in INT_COERCIBLE_STDLIB_NAMES {
        unifier.register_int_coercible_type(verum_common::Text::from(*name));
    }
}
