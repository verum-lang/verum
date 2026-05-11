//! Single source of truth for Verum type-layout constants.
//!
//! All compiler crates that need to answer the question "how many bytes
//! does a value of type `T` occupy at runtime?" consult this module —
//! the typechecker (`@sizeof` / stack-allocation budgets), the MIR
//! lowering (TypeProperty construction), the `@const` / `@property`
//! meta evaluators, and downstream codegen passes.
//!
//! Why a single module? CBGR's memory model has three reference tiers
//! with different on-stack footprints. Each tier's choice — **ThinRef
//! is 16 bytes**, **raw `&unsafe` pointers are 8 bytes** — is load-
//! bearing for stack-allocation analysis, escape analysis, and the
//! `@sizeof` reflection surface. Pre-this-module, those constants were
//! duplicated across four files with at least one site silently
//! disagreeing (the type-checker's `calculate_type_size` returned 8
//! for ALL reference tiers, including Tier-0/Tier-1, contradicting
//! the CBGR design and `type_props`'s 16). A misclassification flowed
//! into `@sizeof` and stack-budget violations.
//!
//! Every constant here is a mirror of an architectural decision
//! documented in `docs/detailed/cbgr-implementation.md` and
//! `docs/architecture/no-libc-architecture.md`. Editing a value here
//! is a load-bearing change requiring matching updates to the runtime
//! header layout in `verum_common::cbgr` and the LLVM lowering in
//! `verum_codegen::llvm::types`.

// ============================================================================
// CBGR reference layout
// ============================================================================

/// Default raw-pointer width (Verum currently targets 64-bit only).
///
/// Used for: function pointers, `*const T` / `*mut T`, untyped void
/// pointers in FFI lowering, plus the per-cell building block of the
/// fat pointer / ThinRef compounds below.
pub const POINTER_SIZE: u64 = 8;

/// Size of a CBGR Tier-0 / Tier-1 reference (`&T` and `&checked T`).
///
/// ```text
///     ThinRef<T> = { ptr: *T, generation: u32, epoch_caps: u32 }
/// ```
///
/// 16 bytes total. Both Tier 0 (default, runtime-validated) and Tier 1
/// (compiler-proven safe via escape analysis) share the same on-stack
/// representation — Tier 1 simply skips the validation read at runtime.
/// **Drift contract:** the runtime header layout in
/// `verum_common::cbgr::ThinRef` MUST agree with this constant; the
/// `cbgr_layout_invariants` test in this file pins the equality.
pub const THIN_REF_SIZE: u64 = 16;

/// Size of a CBGR FatRef (reference for unsized types — slices,
/// trait objects).
///
/// **Authoritative source:** `core/mem/fat_ref.vr` declares
/// `@repr(C, size(32), align(8))` with the layout
/// ```text
///     FatRef<T> = {
///         ptr: &unsafe Byte,        //  8
///         generation: UInt32,        //  4
///         epoch_and_caps: UInt32,    //  4
///         metadata: Int,             //  8  (len for slices, vtable* for dyn)
///         offset_from_base: UInt32,  //  4  (subslice view offset)
///         reserved: UInt32,          //  4
///     }
/// ```
/// **32 bytes total**, matching the stdlib `core/mem` declaration.
///
/// **Drift contract:** runtime LLVM lowering in
/// `verum_codegen::llvm::cbgr` MUST construct a struct whose byte
/// total equals this constant. The previous 4-field LLVM lowering
/// (24 bytes) was a real correctness bug — it caused ABI-boundary
/// corruption when stdlib `core/mem/fat_ref.vr` 32-byte methods
/// were called against the codegen's 24-byte struct. Fixed in tandem
/// with this constant (commit aligning LLVM cbgr emission to the
/// canonical 6-field layout).
pub const FAT_REF_SIZE: u64 = 32;

/// Size of a Tier-0 reference (`&T`). Alias of [`THIN_REF_SIZE`].
pub const REF_TIER0_SIZE: u64 = THIN_REF_SIZE;

/// Size of a Tier-1 reference (`&checked T`). Alias of [`THIN_REF_SIZE`]
/// — Tier 1 retains the same on-stack layout as Tier 0 to allow
/// transparent reuse of CBGR-validated callees; only the runtime
/// validation read is elided.
pub const REF_TIER1_SIZE: u64 = THIN_REF_SIZE;

/// Size of a Tier-2 reference (`&unsafe T`).
///
/// Tier 2 references opt out of CBGR validation entirely and lower to
/// a raw pointer — 8 bytes, no generation / epoch metadata. The unsafe
/// boundary requires the user to discharge soundness manually.
pub const REF_TIER2_SIZE: u64 = POINTER_SIZE;

/// Slice fat-pointer width (`{ ptr, len }`).
pub const SLICE_FAT_PTR_SIZE: u64 = POINTER_SIZE * 2;

// ============================================================================
// CBGR field offsets (per `core/mem/{thin,fat}_ref.vr` declaration order)
// ============================================================================
//
// These offsets are read by codegen (LLVM `verum_cbgr_check*` IR
// emission, MLIR equivalents) when GEPing into a runtime `ThinRef` /
// `FatRef` pointer to load the generation/epoch/capabilities for the
// dereference safety check. They MUST agree with the Rust struct
// layout in `verum_vbc::value::{ThinRef, FatRef}` and the canonical
// declaration in `core/mem/`. The drift-protection test in
// `verum_vbc::value::tests::cbgr_runtime_field_offsets_match_canonical`
// uses `std::mem::offset_of!` to pin the contract at unit-test time.

/// ThinRef field offsets (16 bytes total).
///
/// Layout (`core/mem/thin_ref.vr`):
/// ```text
///     ptr: *T            @ 0
///     generation: u32    @ 8
///     epoch_and_caps: u32@ 12
/// ```

/// Offset of the `ptr` field (always 0 — first field).
pub const THIN_REF_PTR_OFFSET: u64 = 0;

/// Offset of the 32-bit `generation` field. Read by `verum_cbgr_check`
/// during dereference validation.
pub const THIN_REF_GENERATION_OFFSET: u64 = POINTER_SIZE; // 8

/// Offset of the packed `epoch_and_caps` u32 field
/// (`epoch:hi16 | caps:lo16`).
pub const THIN_REF_EPOCH_CAPS_OFFSET: u64 = POINTER_SIZE + 4; // 12

/// FatRef shares its first three fields with ThinRef (same offsets);
/// the additional fields below extend it to 32 bytes.
///
/// Layout (`core/mem/fat_ref.vr`):
/// ```text
///     ptr               @  0
///     generation        @  8
///     epoch_and_caps    @ 12
///     metadata: i64     @ 16  (slice len / vtable pointer)
///     offset_from_base  @ 24  (subslice view offset)
///     reserved          @ 28  (padding for future extensions)
/// ```

/// Offset of the 64-bit `metadata` field (FatRef-only).
pub const FAT_REF_METADATA_OFFSET: u64 = THIN_REF_SIZE; // 16

/// Offset of the 32-bit `offset_from_base` field (FatRef-only).
pub const FAT_REF_OFFSET_FROM_BASE_OFFSET: u64 = FAT_REF_METADATA_OFFSET + 8; // 24

/// Offset of the 32-bit `reserved` padding field (FatRef-only).
pub const FAT_REF_RESERVED_OFFSET: u64 = FAT_REF_OFFSET_FROM_BASE_OFFSET + 4; // 28

// ============================================================================
// CBGR bit-packing constants (epoch_and_caps u32)
// ============================================================================
//
// Both ThinRef and FatRef pack a 16-bit `epoch` counter and a 16-bit
// `capabilities` mask into a single 32-bit `epoch_and_caps` field:
// `epoch` occupies the upper 16 bits, `caps` the lower 16. Validation
// IR shifts/masks through this field; centralising the widths and mask
// here keeps the wire format stable across LLVM, MLIR, and runtime.

/// Bit width of the epoch counter (upper half of `epoch_and_caps`).
pub const EPOCH_BITS: u32 = 16;

/// Bit width of the capabilities mask (lower half of `epoch_and_caps`).
pub const CAPS_BITS: u32 = 16;

/// Total bit width of the packed `epoch_and_caps` u32.
pub const EPOCH_CAPS_BITS: u32 = EPOCH_BITS + CAPS_BITS; // 32

/// Mask isolating the capabilities portion of `epoch_and_caps`.
pub const CAPS_MASK_U32: u32 = (1u32 << CAPS_BITS) - 1; // 0xFFFF

/// Mask isolating the epoch portion of `epoch_and_caps` after right-
/// shifting by `CAPS_BITS`.
pub const EPOCH_MASK_U32: u32 = (1u32 << EPOCH_BITS) - 1; // 0xFFFF

// ============================================================================
// CBGR allocation header
// ============================================================================

/// Size of the CBGR per-allocation header that precedes every heap
/// object on the data side.
///
/// **Authoritative source:** `core/mem/header.vr` declares
/// `HEADER_SIZE: Int = 32`. The runtime mirror is
/// `verum_common::cbgr::AllocationHeader::SIZE` (also 32).
///
/// Codegen back-pointer arithmetic (`user_ptr - ALLOCATION_HEADER_SIZE`)
/// recovers the header from a data pointer for runtime CBGR validation.
pub const ALLOCATION_HEADER_SIZE: u64 = 32;

// ----------------------------------------------------------------------------
// AllocationHeader per-field byte offsets
// ----------------------------------------------------------------------------
//
// Layout (`core/mem/header.vr`, mirrored by
// `verum_common::cbgr::AllocationHeader` `#[repr(C, align(32))]`):
// ```text
//     size:         u32        @  0
//     alignment:    u32        @  4
//     generation:   AtomicU32  @  8
//     epoch:        AtomicU16  @ 12
//     capabilities: AtomicU16  @ 14
//     type_id:      u32        @ 16
//     flags:        AtomicU32  @ 20
//     reserved:     [u32; 2]   @ 24
// ```
// Drift contract pinned by the `offset_of!`-based tests in
// `verum_common::cbgr::tests::allocation_header_field_offsets_pinned`.
// Codegen / SMT verifier / static analysis layers that materialise
// allocation headers MUST consult these constants — never magic
// numbers.

/// Offset of the `size` field in `AllocationHeader`.
pub const ALLOCATION_HEADER_SIZE_OFFSET: u64 = 0;

/// Offset of the `alignment` field.
pub const ALLOCATION_HEADER_ALIGNMENT_OFFSET: u64 = 4;

/// Offset of the atomic `generation` u32 field.
pub const ALLOCATION_HEADER_GENERATION_OFFSET: u64 = 8;

/// Offset of the atomic `epoch` u16 field.
pub const ALLOCATION_HEADER_EPOCH_OFFSET: u64 = 12;

/// Offset of the atomic `capabilities` u16 field.
pub const ALLOCATION_HEADER_CAPABILITIES_OFFSET: u64 = 14;

/// Offset of the `type_id` u32 field.
pub const ALLOCATION_HEADER_TYPE_ID_OFFSET: u64 = 16;

/// Offset of the atomic `flags` u32 field.
pub const ALLOCATION_HEADER_FLAGS_OFFSET: u64 = 20;

/// Offset of the `reserved` `[u32; 2]` padding field.
pub const ALLOCATION_HEADER_RESERVED_OFFSET: u64 = 24;

// ============================================================================
// Heap object header (Tier-0 interpreter / Tier-1 codegen shared)
// ============================================================================

/// Size of the per-heap-object header that precedes every Verum
/// runtime object (List, Map, Set, Deque, Variant, user records).
///
/// Layout (`verum_vbc::interpreter::heap::ObjectHeader`,
/// `#[repr(C)]`):
/// ```text
///     type_id:      TypeId u32  @ 0
///     generation:   u32         @ 4
///     flags:        ObjectFlags @ 8
///     refcount:     u16         @ 10  (note: 2-byte field)
///     size:         u32         @ 12
///     epoch:        u16         @ 16
///     capabilities: u16         @ 18
///     _padding:     u32         @ 20
/// ```
/// 24 bytes total, 8-byte aligned. Both the Tier-0 interpreter and
/// the Tier-1 LLVM codegen MUST use the same value — drift causes
/// silent miscompilation of every heap-object field access (the
/// codegen GEPs by `OBJECT_HEADER_SIZE + field_idx * VALUE_SLOT_SIZE`
/// while the interpreter reads at `OBJECT_HEADER_SIZE + field_idx * 8`).
///
/// **Drift contract:** verified at unit-test time via
/// `verum_vbc::interpreter::heap::tests` — `size_of::<ObjectHeader>()`
/// MUST equal this constant.
pub const OBJECT_HEADER_SIZE: u64 = 24;

/// Width of a single object-field slot in bytes.
///
/// Verum heap objects pack one NaN-boxed `Value` per field; the
/// runtime tag bits are fitted into 64-bit words. Codegen emits
/// `GEP(obj_ptr, OBJECT_HEADER_SIZE + field_idx * VALUE_SLOT_SIZE)`
/// for every field access, and the interpreter mirrors the same
/// stride.
pub const VALUE_SLOT_SIZE: u64 = 8;

/// Compute the byte offset of the *N*-th data field within a heap
/// object, accounting for the leading object header.
///
/// `object_field_offset(N) == OBJECT_HEADER_SIZE + N * VALUE_SLOT_SIZE`.
/// Use this in codegen GEPs and runtime field access in lieu of
/// hand-computed magic numbers.
pub const fn object_field_offset(field_index: u64) -> u64 {
    OBJECT_HEADER_SIZE + field_index * VALUE_SLOT_SIZE
}

// ----------------------------------------------------------------------------
// Canonical heap-object field offsets — derived from the per-collection
// declared field order so codegen and runtime cannot disagree on layout.
// ----------------------------------------------------------------------------

/// `List<T>` field layout: `{ ptr, len, cap }`.
pub const LIST_PTR_OFFSET: u64 = object_field_offset(0); // 24
pub const LIST_LEN_OFFSET: u64 = object_field_offset(1); // 32
pub const LIST_CAP_OFFSET: u64 = object_field_offset(2); // 40
/// Total `List<T>` object size: header + 3 fields = 48 bytes.
pub const LIST_OBJECT_SIZE: u64 = OBJECT_HEADER_SIZE + 3 * VALUE_SLOT_SIZE;

/// `Map<K, V>` field layout (C-runtime view): `{ entries, len, cap }`.
/// (The compiled `map.vr` carries an extra `tombstones` field but the
/// shared C runtime uses the 3-field projection.)
pub const MAP_ENTRIES_OFFSET: u64 = object_field_offset(0); // 24
pub const MAP_LEN_OFFSET: u64 = object_field_offset(1); // 32
pub const MAP_CAP_OFFSET: u64 = object_field_offset(2); // 40
/// Total `Map<K, V>` C-runtime object size: header + 3 fields = 48 bytes.
pub const MAP_HEADER_SIZE: u64 = OBJECT_HEADER_SIZE + 3 * VALUE_SLOT_SIZE;

/// `Set<T>` field layout: `{ len, cap, entries }`.
pub const SET_LEN_OFFSET: u64 = object_field_offset(0); // 24
pub const SET_CAP_OFFSET: u64 = object_field_offset(1); // 32
pub const SET_ENTRIES_OFFSET: u64 = object_field_offset(2); // 40

/// `Deque<T>` field layout: `{ data, head, len, cap }`.
pub const DEQUE_DATA_OFFSET: u64 = object_field_offset(0); // 24
pub const DEQUE_HEAD_OFFSET: u64 = object_field_offset(1); // 32
pub const DEQUE_LEN_OFFSET: u64 = object_field_offset(2); // 40
pub const DEQUE_CAP_OFFSET: u64 = object_field_offset(3); // 48

/// Variant tag offset (single 8-byte slot immediately after the header).
pub const VARIANT_TAG_OFFSET: u64 = OBJECT_HEADER_SIZE; // 24

/// Variant payload offset (one slot after the tag).
pub const VARIANT_PAYLOAD_OFFSET: u64 = OBJECT_HEADER_SIZE + VALUE_SLOT_SIZE; // 32

// ============================================================================
// Synthetic TypeId convention for legacy `MakeVariant`
// ============================================================================
//
// The Verum bytecode supports two variant-construction opcodes:
//
//   * `MakeVariantTyped { type_id, tag, field_count }` — modern form;
//     stores the parent sum-type's real TypeId in the heap header so
//     the runtime can resolve variant names type-scoped (O(1) lookup
//     in the type's variants list).
//
//   * `MakeVariant { tag, field_count }` — legacy fallback emitted
//     when codegen lacks a real TypeId for the parent type (e.g. the
//     archive descriptor hasn't been imported yet, or the type is
//     anonymous). The runtime synthesises a sentinel TypeId of
//     `SYNTHETIC_VARIANT_TYPE_ID_BASE + tag` so consumers can still
//     scan for a matching variant via the global tag-scan fallback in
//     `format_variant_for_print_depth`.
//
// Both producers (the runtime allocators that materialise variant
// values) AND consumers (predicates that need to detect "this is a
// variant heap object") must agree on the sentinel range. Pre-this
// section, the formula `0x8000 + tag` and the `>= 0x8000` predicate
// were duplicated across 5+ producer sites and 3+ consumer sites
// in `verum_vbc::interpreter` and `verum_vbc::codegen`. Editing the
// sentinel base required touching every duplicate; missed sites
// silently degraded to incorrect classification.
//
// Centralising the convention here makes the contract drift-protected
// by construction.

/// Base of the synthetic-variant TypeId range.
///
/// Variants emitted by the legacy `MakeVariant` opcode (no parent
/// type info available at codegen time) carry
/// `synthetic_variant_type_id(tag) == SYNTHETIC_VARIANT_TYPE_ID_BASE + tag`
/// in the heap header. Real (user / stdlib) TypeIds are bounded by
/// `verum_vbc::types::TypeId::LAST_SEMANTIC = 1023` plus the
/// `alloc_user_type_id` allocator's range below `0x8000`, so the
/// `>= 0x8000` predicate cleanly partitions synthetic vs typed
/// variants. The choice of `0x8000` (i.e. bit 15 set) makes the
/// predicate a single bit-test in tight loops.
pub const SYNTHETIC_VARIANT_TYPE_ID_BASE: u32 = 0x8000;

/// Synthetic TypeId for record fallback in `wrap_in_variant`-style
/// helpers when the type-name → real-id lookup misses.
///
/// Distinct from the variant base above so the consumer-side
/// `is_synthetic_variant_type_id` predicate (`>= 0x8000`) doesn't
/// false-positive on records. The gap `[0x8000..0x9000)` reserves
/// 4096 distinct synthetic variant tags; `0x9000` is far enough above
/// to avoid practical collision with high-tag variants.
pub const SYNTHETIC_RECORD_TYPE_ID: u32 = 0x9000;

/// Compose a synthetic variant TypeId from a `tag`.
///
/// Used by every runtime site that materialises a `MakeVariant` heap
/// object — see `verum_vbc::interpreter::dispatch_table::handlers::
/// pattern_matching::alloc_variant_into` for the canonical caller.
#[inline]
pub const fn synthetic_variant_type_id(tag: u32) -> u32 {
    SYNTHETIC_VARIANT_TYPE_ID_BASE + tag
}

/// Returns `true` iff `type_id` is in the synthetic-variant range
/// `[0x8000..)`.
///
/// Consumed by:
///  * the `format_variant_for_print_depth` global tag-scan fallback,
///  * the `deep_value_eq` cross-type variant check,
///  * the variant-detection guard in `string_helpers` and
///    `ffi_extended` runtime intercepts.
#[inline]
pub const fn is_synthetic_variant_type_id(type_id: u32) -> bool {
    type_id >= SYNTHETIC_VARIANT_TYPE_ID_BASE
}

// ============================================================================
// Heap configuration (allocator-wide invariants)
// ============================================================================
//
// Limits and defaults shared by every heap implementation in the
// toolchain — the Tier-0 interpreter heap (`verum_vbc::interpreter::heap`),
// the CBGR-tracked allocator (`verum_vbc::interpreter::cbgr_heap`), and
// the AOT-emitted bump allocator (`verum_codegen::llvm::platform_ir::
// emit_allocator`). Drift between any two would yield inconsistent
// alignment guarantees, allocation rejection thresholds, or initial
// collection capacities depending on which path serves a given call.

/// Minimum alignment for every heap allocation (bytes).
///
/// All Verum allocations align to at least this boundary. The value
/// matches the natural pointer alignment on the supported 64-bit
/// targets (x86_64 / aarch64) and is required for the `#[repr(C)]`
/// `ObjectHeader` / `AllocationHeader` to satisfy their internal
/// field alignments.
pub const MIN_HEAP_ALIGNMENT: usize = 8;

/// Hard ceiling on a single heap allocation (bytes).
///
/// 1 GiB. Prevents DoS from pathological allocations
/// (`array of 2^63 elements` style requests). Exceeding this ceiling
/// produces a structured `AllocationFailure` rather than aborting the
/// process. Both heap implementations enforce the same threshold.
pub const MAX_ALLOCATION_SIZE: usize = 1024 * 1024 * 1024;

/// Default capacity for the Tier-0 interpreter heap (bytes).
///
/// 16 MiB. Used when constructing a `Heap` with `Heap::new()` /
/// `Heap::default()`. Larger heaps can be requested via
/// `Heap::with_threshold(...)`. The value strikes a balance between
/// fast startup (small initial mmap on platforms that pre-fault) and
/// avoiding frequent threshold checks during typical program runs.
pub const DEFAULT_HEAP_SIZE: usize = 16 * 1024 * 1024;

/// Default initial capacity for collections (List / Map / Set / Deque).
///
/// 16 entries. Picks a power-of-two starting capacity friendly to the
/// hash-map probing scheme in `core/collections/map.vr` (which
/// invokes `next_power_of_two().max(INITIAL_CAPACITY)`). The codegen
/// allocator and the stdlib `INITIAL_CAPACITY` constant in
/// `core/collections/map.vr` MUST agree on this value — drift causes
/// the AOT path to allocate one initial size and the stdlib resize
/// helpers to grow from another.
pub const DEFAULT_COLLECTION_CAPACITY: u64 = 16;

// ============================================================================
// Built-in scalar layouts
// ============================================================================

/// `Bool` width (1 byte).
pub const BOOL_SIZE: u64 = 1;

/// `Char` width (4 bytes — Unicode scalar value, UTF-32 storage).
pub const CHAR_SIZE: u64 = 4;

/// Default `Int` width on the 64-bit target (8 bytes).
///
/// Width-tagged variants (`Int8`/`Int16`/...) override this — see
/// [`primitive_size_by_name`].
pub const INT_SIZE: u64 = 8;

/// Default `Float` width (8 bytes — IEEE 754 binary64).
pub const FLOAT_SIZE: u64 = 8;

/// `Text` value layout (`{ ptr, len, capacity }`).
///
/// Text is a value-typed UTF-8 string buffer with capacity tracking;
/// the on-stack footprint mirrors a triple of pointer-sized words.
pub const TEXT_SIZE: u64 = POINTER_SIZE * 3;

// ============================================================================
// Primitive lookup
// ============================================================================

/// Resolve a primitive type's runtime size by its canonical Verum name.
///
/// Returns `Some(size_in_bytes)` for any primitive type Verum models —
/// scalars (`Int` / `Float` / `Bool` / `Char` / `Unit` / `Never`),
/// width-tagged numerics (`Int8` … `Int128`, `UInt8` … `UInt128`,
/// `IntSize`, `USize`, `Float32`, `Float64`), the legacy uppercase-
/// short forms (`I8` … `I128`, `U8` … `U128`, `F32`, `F64`, `Usize`,
/// `Isize`), the Rust-style lowercase aliases (`i8` … `i128`, `u8` …
/// `u128`, `usize`, `isize`, `f32`, `f64`), and `Text`.
///
/// Returns `None` for compound types (records, sum types, generic
/// applications, function types, references) — those go through
/// shape-aware size computation in the typechecker / MIR lowering.
///
/// **Single source of truth:** every primitive width that appears in
/// the compiler reads through this function.
pub fn primitive_size_by_name(name: &str) -> Option<u64> {
    match name {
        // Unit-like — no payload
        "Unit" | "()" | "Never" => Some(0),

        // Boolean
        "Bool" | "bool" => Some(BOOL_SIZE),

        // Character
        "Char" | "char" => Some(CHAR_SIZE),

        // Default-width numerics (target-pointer-width)
        "Int" | "UInt" | "Float" => Some(INT_SIZE),

        // Width-tagged 1-byte
        "Int8" | "UInt8" | "Byte" | "I8" | "U8" | "i8" | "u8" => Some(1),

        // Width-tagged 2-byte
        "Int16" | "UInt16" | "I16" | "U16" | "i16" | "u16" => Some(2),

        // Width-tagged 4-byte
        "Int32" | "UInt32" | "Float32" | "I32" | "U32" | "F32" | "i32" | "u32" | "f32" => Some(4),

        // Width-tagged 8-byte (incl. canonical Int64/UInt64/Float64,
        // pointer-sized Int/UInt aliases, and lowercase forms).
        // `ISize` is the canonical capitalised-S signed pointer-width
        // form mirroring `USize`; `IntSize` is the prior canonical
        // spelling (both alias the same underlying width).
        "Int64" | "UInt64" | "Float64" | "IntSize" | "ISize" | "USize" | "UIntSize"
        | "I64" | "U64" | "F64" | "Usize" | "Isize"
        | "i64" | "u64" | "f64" | "isize" | "usize" => Some(POINTER_SIZE),

        // Width-tagged 16-byte
        "Int128" | "UInt128" | "I128" | "U128" | "i128" | "u128" => Some(16),

        // Text — value-typed string buffer
        "Text" => Some(TEXT_SIZE),

        // Compound or unknown
        _ => None,
    }
}

/// Resolve the alignment of a primitive type by its canonical Verum
/// name. For width-tagged scalars the alignment equals the width;
/// `Text` and other heap-backed values align to the pointer width.
pub fn primitive_alignment_by_name(name: &str) -> Option<u64> {
    match name {
        "Unit" | "()" | "Never" => Some(1),
        // Heap-backed value types align to pointer width (the first
        // field of the layout is a pointer).
        "Text" => Some(POINTER_SIZE),
        // Everything else: alignment == width.
        _ => primitive_size_by_name(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CBGR layout invariants — these constants are load-bearing for
    /// the reference-tier semantics and the runtime header layout.
    /// Drifting any of them is a non-trivial design change requiring
    /// updates to `verum_common::cbgr`, the LLVM lowering, and the
    /// reference-tier docs.
    #[test]
    fn cbgr_layout_invariants() {
        assert_eq!(POINTER_SIZE, 8, "Verum targets 64-bit only");
        assert_eq!(THIN_REF_SIZE, 16, "ThinRef = ptr + gen + epoch_caps");
        assert_eq!(
            FAT_REF_SIZE, 32,
            "FatRef = ThinRef + metadata:8 + offset:4 + reserved:4 (per core/mem/fat_ref.vr @repr(C, size(32), align(8)))",
        );
        assert_eq!(REF_TIER0_SIZE, THIN_REF_SIZE);
        assert_eq!(REF_TIER1_SIZE, THIN_REF_SIZE);
        assert_eq!(REF_TIER2_SIZE, POINTER_SIZE);
        assert_eq!(SLICE_FAT_PTR_SIZE, 16);
        assert_eq!(TEXT_SIZE, 24);
    }

    /// CBGR field offsets are derived from POINTER_SIZE / THIN_REF_SIZE
    /// and must remain monotonic and non-overlapping for both ref shapes.
    #[test]
    fn cbgr_field_offsets_pinned() {
        // ThinRef: ptr(0..8) + generation(8..12) + epoch_caps(12..16)
        assert_eq!(THIN_REF_PTR_OFFSET, 0);
        assert_eq!(THIN_REF_GENERATION_OFFSET, 8);
        assert_eq!(THIN_REF_EPOCH_CAPS_OFFSET, 12);
        // First three fields fit exactly into THIN_REF_SIZE.
        assert_eq!(THIN_REF_EPOCH_CAPS_OFFSET + 4, THIN_REF_SIZE);

        // FatRef extension: metadata(16..24) + offset_from_base(24..28) + reserved(28..32)
        assert_eq!(FAT_REF_METADATA_OFFSET, 16);
        assert_eq!(FAT_REF_OFFSET_FROM_BASE_OFFSET, 24);
        assert_eq!(FAT_REF_RESERVED_OFFSET, 28);
        // Last extra field fits exactly into FAT_REF_SIZE.
        assert_eq!(FAT_REF_RESERVED_OFFSET + 4, FAT_REF_SIZE);
    }

    /// Heap-object layout constants are derived from `OBJECT_HEADER_SIZE`
    /// and `VALUE_SLOT_SIZE`. A change in either must propagate to every
    /// per-collection offset; the per-collection offsets must remain
    /// monotonic and match `object_field_offset(idx)` arithmetic.
    #[test]
    fn heap_object_layout_pinned() {
        // Header sized 24 bytes (matches verum_vbc::interpreter::heap::ObjectHeader).
        assert_eq!(OBJECT_HEADER_SIZE, 24);
        assert_eq!(VALUE_SLOT_SIZE, POINTER_SIZE, "slot is one machine word");

        // object_field_offset() arithmetic — pure derivations.
        assert_eq!(object_field_offset(0), OBJECT_HEADER_SIZE);
        assert_eq!(object_field_offset(1), OBJECT_HEADER_SIZE + VALUE_SLOT_SIZE);
        assert_eq!(object_field_offset(2), OBJECT_HEADER_SIZE + 2 * VALUE_SLOT_SIZE);

        // List<T> { ptr, len, cap }: declared order matches index 0/1/2.
        assert_eq!(LIST_PTR_OFFSET, object_field_offset(0));
        assert_eq!(LIST_LEN_OFFSET, object_field_offset(1));
        assert_eq!(LIST_CAP_OFFSET, object_field_offset(2));
        // Object size = header + 3 slot widths.
        assert_eq!(LIST_OBJECT_SIZE, OBJECT_HEADER_SIZE + 3 * VALUE_SLOT_SIZE);
        assert_eq!(LIST_OBJECT_SIZE, 48);

        // Map<K, V> uses the same 3-slot projection.
        assert_eq!(MAP_ENTRIES_OFFSET, LIST_PTR_OFFSET);
        assert_eq!(MAP_LEN_OFFSET, LIST_LEN_OFFSET);
        assert_eq!(MAP_CAP_OFFSET, LIST_CAP_OFFSET);
        assert_eq!(MAP_HEADER_SIZE, LIST_OBJECT_SIZE);

        // Set<T>: same 3-slot stride, different field semantics.
        assert_eq!(SET_LEN_OFFSET, object_field_offset(0));
        assert_eq!(SET_CAP_OFFSET, object_field_offset(1));
        assert_eq!(SET_ENTRIES_OFFSET, object_field_offset(2));

        // Deque<T>: 4 slots.
        assert_eq!(DEQUE_DATA_OFFSET, object_field_offset(0));
        assert_eq!(DEQUE_HEAD_OFFSET, object_field_offset(1));
        assert_eq!(DEQUE_LEN_OFFSET, object_field_offset(2));
        assert_eq!(DEQUE_CAP_OFFSET, object_field_offset(3));

        // Variants — tag in slot 0, payload in slot 1.
        assert_eq!(VARIANT_TAG_OFFSET, OBJECT_HEADER_SIZE);
        assert_eq!(VARIANT_PAYLOAD_OFFSET, OBJECT_HEADER_SIZE + VALUE_SLOT_SIZE);
    }

    /// AllocationHeader field offsets are derived from
    /// `ALLOCATION_HEADER_SIZE_OFFSET = 0` plus declared field widths.
    /// Layout: `size(4) + alignment(4) + generation(4) + epoch(2) +
    /// capabilities(2) + type_id(4) + flags(4) + reserved(8)` = 32 bytes.
    /// Each next offset must equal previous offset + previous field
    /// width — pinning here guarantees the constants stay self-
    /// consistent independent of the actual `#[repr(C)]` struct.
    #[test]
    fn allocation_header_offsets_pinned() {
        // Derived offsets: each field starts where the previous ended.
        assert_eq!(ALLOCATION_HEADER_SIZE_OFFSET, 0);
        assert_eq!(ALLOCATION_HEADER_ALIGNMENT_OFFSET, 4);   // size: u32 → 4
        assert_eq!(ALLOCATION_HEADER_GENERATION_OFFSET, 8);  // alignment: u32 → 8
        assert_eq!(ALLOCATION_HEADER_EPOCH_OFFSET, 12);      // generation: u32 → 12
        assert_eq!(ALLOCATION_HEADER_CAPABILITIES_OFFSET, 14); // epoch: u16 → 14
        assert_eq!(ALLOCATION_HEADER_TYPE_ID_OFFSET, 16);    // capabilities: u16 → 16
        assert_eq!(ALLOCATION_HEADER_FLAGS_OFFSET, 20);      // type_id: u32 → 20
        assert_eq!(ALLOCATION_HEADER_RESERVED_OFFSET, 24);   // flags: u32 → 24
        // Reserved is 8 bytes (u32 × 2), filling out to 32.
        assert_eq!(
            ALLOCATION_HEADER_RESERVED_OFFSET + 8,
            ALLOCATION_HEADER_SIZE,
            "reserved [u32; 2] fits exactly into the 32-byte total",
        );
    }

    /// Heap-configuration invariants. Both interpreter heap impls and
    /// the AOT bump allocator MUST agree on these limits — duplication
    /// would let one path accept allocations the other rejects.
    #[test]
    fn heap_config_invariants() {
        // Alignment: ≥ pointer width, power of two.
        assert_eq!(MIN_HEAP_ALIGNMENT, 8);
        assert_eq!(MIN_HEAP_ALIGNMENT, POINTER_SIZE as usize);
        assert!(
            MIN_HEAP_ALIGNMENT.is_power_of_two(),
            "alignment must be power-of-two for Layout::from_size_align",
        );

        // Allocation ceiling: 1 GiB.
        assert_eq!(MAX_ALLOCATION_SIZE, 1 << 30);
        assert!(
            MAX_ALLOCATION_SIZE < usize::MAX / 2,
            "ceiling must leave headroom for header overhead",
        );

        // Default heap: 16 MiB, less than the per-allocation ceiling.
        assert_eq!(DEFAULT_HEAP_SIZE, 16 << 20);
        assert!(
            DEFAULT_HEAP_SIZE < MAX_ALLOCATION_SIZE,
            "default heap fits below the per-allocation ceiling",
        );

        // Default collection capacity: power of two for hash-probe.
        assert_eq!(DEFAULT_COLLECTION_CAPACITY, 16);
        assert!(
            (DEFAULT_COLLECTION_CAPACITY as u128).is_power_of_two(),
            "stdlib map probe scheme requires power-of-two cap",
        );
    }

    /// Bit-packing constants stay self-consistent: caps + epoch widths
    /// fill the u32 exactly, masks isolate only their respective halves.
    #[test]
    fn cbgr_bit_pack_constants_consistent() {
        assert_eq!(EPOCH_BITS + CAPS_BITS, EPOCH_CAPS_BITS);
        assert_eq!(EPOCH_CAPS_BITS, 32, "packed field is u32");
        assert_eq!(CAPS_MASK_U32, 0xFFFF);
        assert_eq!(EPOCH_MASK_U32, 0xFFFF);
        // Masks cover exactly half the packed u32.
        assert_eq!(CAPS_MASK_U32.count_ones(), CAPS_BITS);
        assert_eq!(EPOCH_MASK_U32.count_ones(), EPOCH_BITS);
        // Caps mask doesn't overlap epoch bits.
        assert_eq!(CAPS_MASK_U32 & (EPOCH_MASK_U32 << CAPS_BITS), 0);
    }

    /// All primitive scalars resolve to non-None sizes via the
    /// canonical names recognized by `well_known_types::type_names`.
    #[test]
    fn canonical_primitives_have_size() {
        for n in [
            "Bool", "Char", "Int", "Float", "Text", "Unit", "Never",
            "Int8", "Int16", "Int32", "Int64", "Int128", "IntSize",
            "UInt8", "UInt16", "UInt32", "UInt64", "UInt128", "USize",
            "Float32", "Float64", "Byte",
        ] {
            assert!(
                primitive_size_by_name(n).is_some(),
                "primitive '{}' must have a known size",
                n
            );
        }
    }

    /// Width-tagged numerics align with their declared widths.
    #[test]
    fn width_tagged_numeric_widths() {
        assert_eq!(primitive_size_by_name("Int8"), Some(1));
        assert_eq!(primitive_size_by_name("Int16"), Some(2));
        assert_eq!(primitive_size_by_name("Int32"), Some(4));
        assert_eq!(primitive_size_by_name("Int64"), Some(8));
        assert_eq!(primitive_size_by_name("Int128"), Some(16));
        assert_eq!(primitive_size_by_name("UInt8"), Some(1));
        assert_eq!(primitive_size_by_name("UInt16"), Some(2));
        assert_eq!(primitive_size_by_name("UInt32"), Some(4));
        assert_eq!(primitive_size_by_name("UInt64"), Some(8));
        assert_eq!(primitive_size_by_name("UInt128"), Some(16));
        assert_eq!(primitive_size_by_name("Float32"), Some(4));
        assert_eq!(primitive_size_by_name("Float64"), Some(8));
        assert_eq!(primitive_size_by_name("Byte"), Some(1));
        assert_eq!(primitive_size_by_name("USize"), Some(8));
        assert_eq!(primitive_size_by_name("IntSize"), Some(8));
    }

    /// Legacy uppercase-short and Rust-lowercase aliases agree with
    /// their canonical counterparts. Drift here would silently change
    /// `@sizeof` answers between source spelling forms.
    #[test]
    fn primitive_alias_consistency() {
        // Canonical ↔ Verum-uppercase-short ↔ Rust-lowercase matrix.
        let table: &[(&str, &str, &str)] = &[
            ("Int8",   "I8",   "i8"),
            ("Int16",  "I16",  "i16"),
            ("Int32",  "I32",  "i32"),
            ("Int64",  "I64",  "i64"),
            ("UInt8",  "U8",   "u8"),
            ("UInt16", "U16",  "u16"),
            ("UInt32", "U32",  "u32"),
            ("UInt64", "U64",  "u64"),
            ("Float32","F32",  "f32"),
            ("Float64","F64",  "f64"),
            ("USize",  "Usize","usize"),
            // IntSize -> Isize -> isize
            ("IntSize","Isize","isize"),
        ];
        for &(canon, short, lower) in table {
            let c = primitive_size_by_name(canon);
            let s = primitive_size_by_name(short);
            let l = primitive_size_by_name(lower);
            assert!(c.is_some(), "canonical '{}' missing", canon);
            assert_eq!(c, s, "short alias '{}' disagrees with '{}'", short, canon);
            assert_eq!(c, l, "lower alias '{}' disagrees with '{}'", lower, canon);
        }
    }

    /// Compound / non-primitive names return None.
    #[test]
    fn compound_types_return_none() {
        for n in ["List", "Map", "Set", "Maybe", "Result", "MyType", "T"] {
            assert_eq!(
                primitive_size_by_name(n),
                None,
                "'{}' should not be classified as a primitive",
                n
            );
        }
    }

    /// Alignment matches width for fixed scalars; pointer-sized for
    /// heap-backed values.
    #[test]
    fn primitive_alignment_rules() {
        assert_eq!(primitive_alignment_by_name("Int8"), Some(1));
        assert_eq!(primitive_alignment_by_name("Int16"), Some(2));
        assert_eq!(primitive_alignment_by_name("Int32"), Some(4));
        assert_eq!(primitive_alignment_by_name("Int64"), Some(8));
        assert_eq!(primitive_alignment_by_name("Int"), Some(8));
        assert_eq!(primitive_alignment_by_name("Float"), Some(8));
        assert_eq!(primitive_alignment_by_name("Bool"), Some(1));
        assert_eq!(primitive_alignment_by_name("Char"), Some(4));
        assert_eq!(primitive_alignment_by_name("Unit"), Some(1));
        // Text aligns to pointer, not to its 24-byte total width.
        assert_eq!(primitive_alignment_by_name("Text"), Some(POINTER_SIZE));
    }

    // ========================================================================
    // Synthetic-variant TypeId convention — drift contract
    // ========================================================================
    //
    // These pin the contract that runtime allocators (producer side) and
    // variant-detection predicates (consumer side) share. A drift here
    // would silently mis-classify variant heap objects across the runtime.

    /// Base sentinel matches the documented `0x8000` value.
    #[test]
    fn synthetic_variant_base_pinned() {
        assert_eq!(SYNTHETIC_VARIANT_TYPE_ID_BASE, 0x8000);
    }

    /// Record-fallback sentinel above the synthetic-variant range so
    /// `is_synthetic_variant_type_id` doesn't false-positive on records.
    #[test]
    fn synthetic_record_above_variant_range() {
        assert_eq!(SYNTHETIC_RECORD_TYPE_ID, 0x9000);
        assert!(SYNTHETIC_RECORD_TYPE_ID >= SYNTHETIC_VARIANT_TYPE_ID_BASE);
    }

    /// `synthetic_variant_type_id(tag)` matches the canonical formula.
    #[test]
    fn synthetic_variant_formula_canonical() {
        assert_eq!(synthetic_variant_type_id(0), 0x8000);
        assert_eq!(synthetic_variant_type_id(1), 0x8001);
        assert_eq!(synthetic_variant_type_id(0xFF), 0x80FF);
        // Round-trip: composing then classifying recovers the
        // synthetic-variant property.
        for tag in [0u32, 1, 7, 0x42, 0xFF, 0xFFF] {
            assert!(is_synthetic_variant_type_id(synthetic_variant_type_id(tag)));
        }
    }

    /// `is_synthetic_variant_type_id` partitions the TypeId space at
    /// `0x8000`. Real user/stdlib TypeIds sit below; everything at or
    /// above is synthetic.
    #[test]
    fn synthetic_variant_predicate_partitions() {
        // Real TypeId range (well below 0x8000).
        assert!(!is_synthetic_variant_type_id(0));
        assert!(!is_synthetic_variant_type_id(17));   // FIRST_USER
        assert!(!is_synthetic_variant_type_id(515));  // MAYBE
        assert!(!is_synthetic_variant_type_id(516));  // RESULT
        assert!(!is_synthetic_variant_type_id(1023)); // LAST_SEMANTIC
        assert!(!is_synthetic_variant_type_id(0x7FFF));
        // Synthetic range starts at 0x8000.
        assert!(is_synthetic_variant_type_id(0x8000));
        assert!(is_synthetic_variant_type_id(0x8001));
        assert!(is_synthetic_variant_type_id(SYNTHETIC_RECORD_TYPE_ID));
        assert!(is_synthetic_variant_type_id(u32::MAX));
    }
}
