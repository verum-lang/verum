//! Heap memory management with CBGR integration.
//!

//! The heap provides memory allocation for interpreter objects with:
//! - Object headers for type info and GC
//! - Generation counters for CBGR memory safety
//! - Epoch tracking for cross-allocator validation
//! - Simple bump allocation
//!

//! # CBGR Integration
//!

//! This heap implements full CBGR (Compile-time Borrow checking with Generational
//! References) semantics:
//!

//! - **Generation**: 32-bit counter incremented on each allocation, used to detect
//!  use-after-free when a slot is reused.
//! - **Epoch**: 16-bit value from global epoch counter, prevents ABA problem when
//!  generation wraps around.
//! - **Capabilities**: 16-bit flags for read/write/delegate permissions.
//!

//! # Object Layout
//!

//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ OBJECT LAYOUT (24 bytes header) │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ ┌──────────────────────────────────────────────────────────┐ │
//! │ │ ObjectHeader (24 bytes) │ │
//! │ │ type_id: TypeId (4) │ │
//! │ │ generation: u32 (4) │ │
//! │ │ flags: ObjectFlags (2) │ │
//! │ │ refcount: u16 (2) │ │
//! │ │ size: u32 (4) │ │
//! │ │ epoch: u16 (2) + capabilities: u16 (2) + _pad: u32 (4) │ │
//! │ └──────────────────────────────────────────────────────────┘ │
//! │ ┌──────────────────────────────────────────────────────────┐ │
//! │ │ Object Data │ │
//! │ │ (type-specific fields, arrays, etc.) │ │
//! │ └──────────────────────────────────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use std::alloc::{Layout, alloc, dealloc};
use std::ptr::NonNull;

use bitflags::bitflags;

use super::error::{CbgrViolationKind, InterpreterError, InterpreterResult};
use crate::types::TypeId;
use crate::value::Value;

/// Size of object header in bytes.
///
/// Re-exports the canonical [`verum_common::layout::OBJECT_HEADER_SIZE`]
/// — the single source of truth shared with the Tier-1 LLVM codegen
/// (`verum_codegen::llvm::runtime::RuntimeLowering::OBJECT_HEADER_SIZE`).
/// Both tiers MUST agree byte-for-byte; the
/// `header_struct_size_matches_canonical` test below pins the equality
/// against the actual `#[repr(C)] ObjectHeader` Rust struct.
pub const OBJECT_HEADER_SIZE: usize = verum_common::layout::OBJECT_HEADER_SIZE as usize;

/// Default heap size (16 MB).
///
/// Re-exports the canonical [`verum_common::layout::DEFAULT_HEAP_SIZE`].
pub const DEFAULT_HEAP_SIZE: usize = verum_common::layout::DEFAULT_HEAP_SIZE;

/// Minimum alignment for objects.
///
/// Re-exports the canonical [`verum_common::layout::MIN_HEAP_ALIGNMENT`]
/// — the single source of truth shared with `cbgr_heap.rs` and the
/// AOT bump allocator emitted by `verum_codegen::llvm::platform_ir`.
pub const MIN_ALIGNMENT: usize = verum_common::layout::MIN_HEAP_ALIGNMENT;

/// Maximum single allocation size (1 GB).
///

/// Prevents DoS attacks via requesting extremely large allocations
/// (e.g., 2^63 element arrays). Any single allocation request exceeding
/// this limit is rejected with OutOfMemory.
pub const MAX_ALLOCATION_SIZE: usize = verum_common::layout::MAX_ALLOCATION_SIZE;

// ============================================================================
// Variant heap-object header read helpers
// ============================================================================
//
// Variant heap layout (matches `pattern_matching::alloc_variant_into` and
// `method_dispatch::alloc_variant_with_payload`):
//
//   [ObjectHeader (24)][tag: u32][field_count: u32][payload: Value * N]
//
// Offsets are pinned at `verum_common::layout::{VARIANT_TAG_OFFSET = 24,
// VARIANT_PAYLOAD_OFFSET = 32}`.  Pre-this-helper, 16+ dispatch handler
// sites each repeated the unsafe-pointer-arithmetic dance:
//
//   let tag = unsafe { *(ptr.add(OBJECT_HEADER_SIZE) as *const u32) };
//   let payload = unsafe { *(ptr.add(OBJECT_HEADER_SIZE + 8) as *const Value) };
//
// Each site carried its own bounds reasoning, the magic `+ 8` was duplicated
// (the `(tag, field_count)` pair occupies 8 bytes), and editing the variant
// header layout would have required touching every site.  The helpers below
// centralise the offsets via `verum_common::layout::VARIANT_*_OFFSET` and
// the safety reasoning lives once.

/// Read the variant `tag` from a heap variant pointer.
///
/// # Safety
/// `ptr` must point to a live heap object whose data section starts with
/// the variant `(tag: u32, field_count: u32)` pair — `MakeVariant` and
/// `MakeVariantTyped` both produce this shape.
#[inline]
pub unsafe fn variant_tag(ptr: *const u8) -> u32 {
    unsafe {
        *(ptr.add(verum_common::layout::VARIANT_TAG_OFFSET as usize) as *const u32)
    }
}

/// Read the `(tag, field_count)` pair from a heap variant pointer in
/// a single function call.  Two adjacent u32 slots read in one cache-
/// friendly access — callers that need only the field count
/// pattern-match `(_, fc)` rather than calling a dedicated helper, so
/// no `variant_field_count`-only function exists.
///
/// # Safety
/// Same contract as [`variant_tag`].
#[inline]
pub unsafe fn variant_header_pair(ptr: *const u8) -> (u32, u32) {
    let data = unsafe { ptr.add(verum_common::layout::VARIANT_TAG_OFFSET as usize) as *const u32 };
    unsafe { (*data, *data.add(1)) }
}

/// Read the `idx`-th payload `Value` from a heap variant pointer.
///
/// # Safety
/// `ptr` must satisfy [`variant_tag`]'s contract AND `idx` must be
/// strictly less than the variant's `field_count`.
#[inline]
pub unsafe fn variant_payload(ptr: *const u8, idx: usize) -> Value {
    unsafe { *variant_payload_ptr(ptr, idx) }
}

/// Get a `*const Value` to the `idx`-th payload slot of a heap variant
/// pointer.
///
/// # Safety
/// `ptr` must satisfy [`variant_tag`]'s contract; `idx` must be in
/// `0..field_count` of the underlying variant.
#[inline]
pub unsafe fn variant_payload_ptr(ptr: *const u8, idx: usize) -> *const Value {
    let base = unsafe { ptr.add(verum_common::layout::VARIANT_PAYLOAD_OFFSET as usize) as *const Value };
    unsafe { base.add(idx) }
}

/// Mutable counterpart to [`variant_payload_ptr`] — for codegen-side
/// payload writes (e.g. `make_some_value` materialising the inner value
/// after `alloc_with_init`).
///
/// # Safety
/// Same contract as [`variant_payload_ptr`]; the underlying memory must
/// be uniquely owned for the duration of the write.
#[inline]
pub unsafe fn variant_payload_ptr_mut(ptr: *mut u8, idx: usize) -> *mut Value {
    let base = unsafe { ptr.add(verum_common::layout::VARIANT_PAYLOAD_OFFSET as usize) as *mut Value };
    unsafe { base.add(idx) }
}

/// Write the `(tag, field_count)` pair into the data section of a freshly
/// allocated variant heap object.  The canonical writer used by
/// `pattern_matching::alloc_variant_into_with_type_id` and
/// `method_dispatch::alloc_variant_with_payload`'s init closures —
/// previously each callsite open-coded
/// `*(data.as_mut_ptr() as *mut u32) = tag; *(...).add(1) = fc`.
///
/// # Safety
/// `ptr` must point to a live heap object's data section
/// (`heap.alloc_with_init`'s closure receives this; the helper does the
/// pointer arithmetic). The write touches 8 bytes
/// `[VARIANT_TAG_OFFSET..VARIANT_PAYLOAD_OFFSET]`.
#[inline]
pub unsafe fn write_variant_data_header(data: *mut u8, tag: u32, field_count: u32) {
    let tag_ptr = data as *mut u32;
    unsafe {
        *tag_ptr = tag;
        *tag_ptr.add(1) = field_count;
    }
}

// ============================================================================
// Closure heap-object header read helpers
// ============================================================================
//
// Closure heap layout (matches `calls::handle_make_closure`):
//
//   [ObjectHeader (24)][func_id: u32][capture_count: u32][captures: Value * N]
//
// The (func_id, capture_count) pair occupies the same 8 bytes that the
// variant header uses for (tag, field_count) — `verum_common::layout::
// VARIANT_PAYLOAD_OFFSET` (= 32) doubles as the captures-array base
// because the structural layout is parallel.  Pre-this-helper, 3 sites
// (`call_closure_sync` in dispatch_table/mod.rs, `handle_call_closure`
// + `handle_make_closure` in calls.rs) each repeated the same
// raw-pointer arithmetic with hardcoded `header_offset + 4` / `+ 8`
// literals.  The drift-pin test
// `closure_captures_offset_matches_variant_payload_offset` enforces the
// shared-offset invariant.

/// Read the `(func_id, capture_count)` pair from a closure heap pointer.
///
/// # Safety
/// `ptr` must point to a live heap object whose data section starts
/// with the closure `(func_id: u32, capture_count: u32)` pair —
/// `handle_make_closure` is the canonical producer.
#[inline]
pub unsafe fn closure_header(ptr: *const u8) -> (u32, u32) {
    let data = unsafe { ptr.add(verum_common::layout::VARIANT_TAG_OFFSET as usize) as *const u32 };
    unsafe { (*data, *data.add(1)) }
}

/// Get a `*const Value` to the `idx`-th capture slot of a closure
/// heap pointer.  Mirror of [`variant_payload_ptr`] for the closure
/// layout; both share `VARIANT_PAYLOAD_OFFSET` (= 32) as the base.
///
/// # Safety
/// `ptr` must satisfy [`closure_header`]'s contract; `idx` must be in
/// `0..capture_count` of the closure.
#[inline]
pub unsafe fn closure_captures_ptr(ptr: *const u8, idx: usize) -> *const Value {
    let base =
        unsafe { ptr.add(verum_common::layout::VARIANT_PAYLOAD_OFFSET as usize) as *const Value };
    unsafe { base.add(idx) }
}

/// Mutable counterpart to [`closure_captures_ptr`] — for codegen-side
/// capture writes (e.g. `handle_make_closure` storing captured values
/// after `alloc_with_init`).
///
/// # Safety
/// Same contract as [`closure_captures_ptr`]; the underlying memory
/// must be uniquely owned for the duration of the write.
#[inline]
pub unsafe fn closure_captures_ptr_mut(ptr: *mut u8, idx: usize) -> *mut Value {
    let base =
        unsafe { ptr.add(verum_common::layout::VARIANT_PAYLOAD_OFFSET as usize) as *mut Value };
    unsafe { base.add(idx) }
}

/// Write the `(func_id, capture_count)` header pair into a freshly
/// allocated closure data section.  Canonical writer used by
/// `handle_make_closure`'s `alloc_with_init` closure.
///
/// # Safety
/// `data` must point to a closure data section (8-byte header + N
/// capture slots).  The helper writes exactly 8 bytes
/// `[0..VARIANT_PAYLOAD_OFFSET - VARIANT_TAG_OFFSET]`.
#[inline]
pub unsafe fn write_closure_data_header(data: *mut u8, func_id: u32, capture_count: u32) {
    let head_ptr = data as *mut u32;
    unsafe {
        *head_ptr = func_id;
        *head_ptr.add(1) = capture_count;
    }
}

// ============================================================================
// BYTE_SLICE (528) byte-view helpers (ARCH-P5)
// ============================================================================
//
// A BYTE_SLICE heap object is the representation-tagged borrowed byte
// view produced by `Text.as_bytes()` (TextExtended::AsBytes) and by
// re-slicing an existing BYTE_SLICE (CbgrExtended RefSlice /
// SliceSubslice / SliceSplitAt).  Layout:
//
//   [ObjectHeader (24, type_id = TypeId::BYTE_SLICE)][ptr: i64][len: i64]
//
// Both payload slots are RAW machine words (NOT NaN-boxed Values) —
// bit-identical to the Tier-1 (AOT) slice Pack `{ptr@24, len@32}`
// stamped by `lower_pack_typed`, so the two tiers share ONE object
// form.  Consumers dispatch on the header TypeId via
// [`value_as_byte_slice`]; this retires the `len <= 1_000_000`
// FatRef-as-Text heuristic at every former consumer arm.
//
// `ptr` is NEVER null: empty views point at [`empty_byte_slice_ptr`]
// (mirrors the AOT `verum_text_get_ptr` never-null contract).

/// Stable, never-null pointer for zero-length byte views.
#[inline]
pub fn empty_byte_slice_ptr() -> *mut u8 {
    static EMPTY: [u8; 1] = [0];
    EMPTY.as_ptr() as *mut u8
}

/// Read the `(ptr, len)` payload of a BYTE_SLICE object base pointer.
///
/// # Safety
/// `base` must point to a live heap object whose header TypeId is
/// `TypeId::BYTE_SLICE` (the two raw i64 payload slots must be
/// initialized — every producer writes both).
#[inline]
pub unsafe fn byte_slice_payload(base: *const u8) -> (*mut u8, u64) {
    let data = unsafe { base.add(OBJECT_HEADER_SIZE) };
    let ptr = unsafe { *(data as *const u64) } as *mut u8;
    let len = unsafe { *(data.add(8) as *const u64) };
    (ptr, len)
}

/// Canonical classifier: if `v` is a pointer to a BYTE_SLICE heap
/// object, return its `(ptr, len)` byte range; `None` for every other
/// value shape.  This is the single inspection API all typed
/// BYTE_SLICE consumer arms use — no site re-implements the header
/// probe.
#[inline]
/// TYPED-ARRAY-ITER-1 — ONE authority for a packed typed array's
/// element geometry and decode.
///
/// `NewTypedArray` allocates `[T; N]` as a RAW buffer stamped with the
/// SCALAR TypeId (`U8`/`U16`/`U32`/`U64` for integers by stride,
/// `F32`/`F64` for floats — see TYPED-ARRAY-FLOAT-1).  Every consumer
/// that walks such a buffer (index reads, the iterator protocol) must
/// agree on (stride, float-ness) and on the raw→Value decode; the
/// iterator classifier previously had NO leg for these ids, fell to
/// `ITER_TYPE_LIST`, read the raw payload as a `[len, cap, backing]`
/// list header and dereferenced a garbage "backing pointer"
/// (`Set.from([1, 2, 3])` → SIGSEGV in phase.interpret).
pub fn typed_array_element_spec(tid: crate::types::TypeId) -> Option<(usize, bool)> {
    use crate::types::TypeId;
    match tid {
        TypeId::U8 => Some((1, false)),
        TypeId::U16 => Some((2, false)),
        TypeId::U32 => Some((4, false)),
        TypeId::U64 => Some((8, false)),
        TypeId::F32 => Some((4, true)),
        TypeId::F64 => Some((8, true)),
        _ => None,
    }
}

/// Decode one element of a packed typed array as a boxed `Value`.
/// Mirrors `handle_get_index`'s per-width legs exactly: integer widths
/// ≤32 zero-extend, 64-bit reads are signed (`[Int; N]`), floats round
/// -trip their IEEE-754 bits through `from_f64`.
///
/// # Safety
/// `data_ptr` must point at the buffer's DATA area (past the object
/// header) with at least `(idx + 1) * stride` readable bytes, where
/// `stride` matches `typed_array_element_spec(tid)`.
pub unsafe fn typed_array_element(
    tid: crate::types::TypeId,
    data_ptr: *const u8,
    idx: usize,
) -> Option<Value> {
    let (stride, is_float) = typed_array_element_spec(tid)?;
    let at = unsafe { data_ptr.add(idx * stride) };
    Some(match (stride, is_float) {
        (1, false) => Value::from_i64(unsafe { *at } as i64),
        (2, false) => Value::from_i64(unsafe { *(at as *const u16) } as i64),
        (4, false) => Value::from_i64(unsafe { *(at as *const u32) } as i64),
        (8, false) => Value::from_i64(unsafe { *(at as *const i64) }),
        (4, true) => Value::from_f64(f32::from_bits(unsafe { *(at as *const u32) }) as f64),
        (8, true) => Value::from_f64(f64::from_bits(unsafe { *(at as *const u64) })),
        _ => return None,
    })
}

/// Write dual of [`typed_array_element`] — encode one boxed `Value`
/// into a packed typed array slot.  Mirrors `handle_set_index`'s
/// per-width legs exactly: integer widths truncate `as_i64`, floats
/// store their IEEE-754 bit pattern (F64 falls back to the raw value
/// bits when the slot receives an already-unboxed pattern, F32
/// narrows through `as f32`).
///
/// # Safety
/// Same contract as [`typed_array_element`], with WRITE access to the
/// element range.
pub unsafe fn typed_array_store_element(
    tid: crate::types::TypeId,
    data_ptr: *mut u8,
    idx: usize,
    value: Value,
) -> Option<()> {
    let (stride, is_float) = typed_array_element_spec(tid)?;
    let at = unsafe { data_ptr.add(idx * stride) };
    match (stride, is_float) {
        (1, false) => unsafe { *at = value.as_i64() as u8 },
        (2, false) => unsafe { *(at as *mut u16) = value.as_i64() as u16 },
        (4, false) => unsafe { *(at as *mut i32) = value.as_i64() as i32 },
        (8, false) => unsafe { *(at as *mut i64) = value.as_i64() },
        (8, true) => {
            let bits = value
                .try_as_f64()
                .map(|f| f.to_bits())
                .unwrap_or_else(|| value.bits());
            unsafe { *(at as *mut u64) = bits };
        }
        (4, true) => {
            let bits = (value.try_as_f64().unwrap_or(0.0) as f32).to_bits();
            unsafe { *(at as *mut u32) = bits };
        }
        _ => return None,
    }
    Some(())
}

pub fn value_as_byte_slice(v: &Value) -> Option<(*mut u8, u64)> {
    if !v.is_ptr() || v.is_nil() || v.is_boxed_int() {
        return None;
    }
    let base = v.as_ptr::<u8>();
    // SAFETY: try_type_id rejects null / misaligned / special-marker
    // payloads; a BYTE_SLICE-stamped header implies the 16-byte raw
    // payload contract documented on `byte_slice_payload`.
    match unsafe { ObjectHeader::try_type_id(base) } {
        Some(TypeId::BYTE_SLICE) => Some(unsafe { byte_slice_payload(base) }),
        _ => None,
    }
}

// ============================================================================
// TEXT (4) canonical heap-record helpers (ARCH-P5 final leg)
// ============================================================================
//
// ONE heap Text layout.  Every interpreter-side heap Text producer
// (`Interpreter::alloc_string`, `load_constant`'s string-constant
// realisation, concat / to_string / char_to_str, the FFI OSError
// message builder, the Text mutator intercepts, …) allocates a SINGLE
// self-contained object:
//
//   [ObjectHeader (24, type_id = TypeId::TEXT, size = 24 + storage)]
//   [ptr: Value (NaN-boxed *u8)] [len: Value (Int)] [cap: Value (Int)]
//   [utf8 bytes …]                                   ^ payload offset 24
//
// The three payload slots ARE the language-level `Text {ptr, len, cap}`
// record (core/text/text.vr:194) exactly as the stdlib's struct-literal
// builders produce it — GetF/SetF and every .vr method body read the
// same fields the runtime wrote.  `ptr` points at payload+24 INSIDE the
// same allocation (self-contained: the object carries its bytes), and
// Tier-1 (AOT) has always used this record (`verum_text_alloc` /
// `verum_text_get_ptr` read `{ptr@0, len@8, cap@16}`).
//
// Two producer flavours, distinguished ONLY by the `cap` field — the
// exact semantic text.vr pins at text.vr:25 ("cap == 0 indicates
// static/immutable string literal"):
//
//   * [`Heap::alloc_text`] — immutable: `size = 24 + byte_len`,
//     `cap = 0`.  `Text.grow` (text.vr:1069) only dealloc's `self.ptr`
//     when `cap > 0` and `push_*` grows BEFORE the first write when
//     `len >= cap`, so the interior pointer is never written past `len`
//     and never reaches the .vr allocator's dealloc.  Mutation
//     COW-promotes into a fresh allocator buffer (grow copies the
//     bytes, then repoints ptr/cap at owned storage) — the same
//     contract AOT rodata Text literals carry (`{ptr, len, cap=0}`).
//     Empty text uses [`empty_byte_slice_ptr`] (never-null, never
//     written: push grows first, truncate early-returns on len 0).
//
//   * [`Heap::alloc_text_with_capacity`] — capacity-carrying (the
//     `with_capacity` / `reserve` intercepts): `size = 24 + cap + 1`,
//     `cap > 0`.  The byte region reserves `cap + 1` bytes so
//     text.vr's owned-buffer convention (a `cap`-capacity buffer has
//     `cap + 1` bytes for the NUL terminator; `push_byte` writes at
//     `len` then NUL at `len + 1` with `len < cap`) holds in-bounds.
//     When .vr `grow` eventually runs past `cap` it allocates a FRESH
//     buffer, copies, and calls `dealloc(self.ptr, cap+1, 1)` on the
//     interior pointer — a no-op at Tier-0 (`CbgrDealloc` is an
//     intentional leak, ffi_extended.rs), so the record's inline
//     storage simply goes dormant and dies with the object.
//
// This retires the legacy `TypeId(0x0001)` `[len: u64][bytes…]` form —
// the LAST dual representation in the Text ABI (the FatRef-as-Text
// heuristic was retired by the BYTE_SLICE(528) campaign).  The legacy
// reader arms are DELETED, not deprecated: archives never carry live
// heap Text objects (`Constant::String` is a string-table id realised
// by `load_constant` at runtime — serialize.rs:656 encodes the id
// only) and Tier-1 never produced the form, so nothing can resurrect
// a 0x0001 object once the interpreter producers are converted.

/// Payload size of the `Text {ptr, len, cap}` record head (three
/// 8-byte slots).  Bytes of a self-contained Text start at this
/// offset into the payload.
pub const TEXT_RECORD_SIZE: usize = 24;

/// Read `(bytes_ptr, byte_len)` from a canonical TEXT record payload.
///
/// Field 0 (`ptr`) tolerates every encoding the struct-literal codegen
/// and the runtime producers are known to emit (the same tolerance the
/// former AsBytes dual-layout reader carried):
///   * `Value::nil()` — no buffer (`Text.new()`): returns null.
///   * NaN-boxed `Value::from_ptr(..)` — the typed-store / runtime path.
///   * NaN-boxed `Value::from_i64(addr)` — an address that flowed
///     through an Int-typed slot (the `cbgr_alloc` tuple path).
///   * RAW pointer bits — the historical struct-literal store for
///     `&unsafe Byte` fields (`Text.from_utf8_unchecked`).
///
/// Returns `None` when field 1 (`len`) does not classify as a
/// NaN-boxed Int — a builder record ALWAYS carries `Value::from_i64(len)`
/// in slot 1, so a non-Int slot means the object is not a Text record.
///
/// # Safety
/// `base` must point to a live heap object whose header TypeId is
/// `TypeId::TEXT` and whose payload is at least [`TEXT_RECORD_SIZE`]
/// bytes (every canonical producer guarantees both).
#[inline]
pub unsafe fn text_record_payload(base: *const u8) -> Option<(*const u8, usize)> {
    let data = unsafe { base.add(OBJECT_HEADER_SIZE) };
    let f0 = unsafe { *(data as *const Value) };
    let f1 = unsafe { *((data as *const Value).add(1)) };
    if !f1.is_int() {
        return None;
    }
    let len = f1.as_i64().max(0) as usize;
    let ptr: *const u8 = if f0.is_nil() {
        std::ptr::null()
    } else if f0.is_ptr() {
        f0.as_ptr::<u8>() as *const u8
    } else if f0.is_int() {
        f0.as_i64() as usize as *const u8
    } else {
        // Raw pointer bits stored without NaN-box.
        (unsafe { *(data as *const u64) }) as *const u8
    };
    Some((ptr, len))
}

/// Read the `cap` field (slot 2) of a canonical TEXT record payload.
/// Tolerates a NaN-boxed Int or raw machine-word storage.
///
/// # Safety
/// Same contract as [`text_record_payload`].
#[inline]
pub unsafe fn text_record_cap(base: *const u8) -> i64 {
    let data = unsafe { base.add(OBJECT_HEADER_SIZE) };
    let f2 = unsafe { *((data as *const Value).add(2)) };
    if f2.is_int() {
        f2.as_i64()
    } else {
        (unsafe { *((data as *const u64).add(2)) }) as i64
    }
}

/// Canonical classifier: if `v` is a pointer to a TEXT heap record,
/// return its `(bytes_ptr, byte_len)`; `None` for every other value
/// shape.  The single inspection API for heap-Text consumer arms —
/// no site re-implements the header probe or the field-0 tolerance.
#[inline]
pub fn value_as_text_record(v: &Value) -> Option<(*const u8, usize)> {
    if !v.is_ptr() || v.is_nil() || v.is_boxed_int() {
        return None;
    }
    let base = v.as_ptr::<u8>();
    // SAFETY: try_type_id rejects null / misaligned / special-marker
    // payloads; a TEXT-stamped header implies the record contract
    // documented on `text_record_payload`.
    match unsafe { ObjectHeader::try_type_id(base) } {
        Some(TypeId::TEXT) => unsafe { text_record_payload(base) },
        _ => None,
    }
}

bitflags! {
    /// Object flags for runtime state.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ObjectFlags: u16 {
        /// Object is mutable.
        const MUTABLE = 0b0000_0001;
        /// Object is currently borrowed.
        const BORROWED = 0b0000_0010;
        /// Object is mutably borrowed.
        const BORROWED_MUT = 0b0000_0100;
        /// Object has been marked for GC.
        const MARKED = 0b0000_1000;
        /// Object is pinned (cannot move).
        const PINNED = 0b0001_0000;
        /// Object has been freed (for debug).
        const FREED = 0b0010_0000;
        /// Object contains references (needs tracing).
        const HAS_REFS = 0b0100_0000;
        /// Object has a finalizer.
        const HAS_FINALIZER = 0b1000_0000;
    }
}

/// Object header placed before object data.
///
/// Layout matches CBGR requirements with generation, epoch, and
/// capabilities.  The explicit `align(8)` is load-bearing: the
/// header is followed by 8-byte `Value` slots in CBGR-tracked
/// objects, and the `header_struct_size_matches_canonical`
/// drift-contract test demands
/// `align_of::<ObjectHeader>() == verum_common::layout::POINTER_SIZE`
/// (8) so that field-offset GEPs emitted by
/// `verum_codegen::llvm::runtime::RuntimeLowering` are
/// pointer-aligned.  Without `align(8)` the struct's natural
/// alignment is 4 (max of u32 fields), and every per-object
/// field-access GEP silently bypasses an 8-byte alignment
/// invariant assumed by Tier-1 codegen.  The `_padding: u32`
/// field below already brings the size to 24 (a multiple of 8),
/// so adding `align(8)` does not grow the struct.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy)]
pub struct ObjectHeader {
    /// Type ID for this object.
    pub type_id: TypeId,

    /// CBGR generation counter (32-bit).
    /// Incremented on each allocation to detect use-after-free.
    pub generation: u32,

    /// Object flags.
    pub flags: ObjectFlags,

    /// Reference count (for non-CBGR objects).
    pub refcount: u16,

    /// Object size (data portion only).
    pub size: u32,

    /// CBGR epoch (16-bit) - prevents ABA problem on generation wrap.
    pub epoch: u16,

    /// CBGR capabilities (16-bit) - read/write/delegate permissions.
    pub capabilities: u16,

    /// Padding to round the struct to 24 bytes (a `POINTER_SIZE`
    /// multiple).  Required by the cross-tier drift contract
    /// against `verum_common::layout::OBJECT_HEADER_SIZE`.
    _padding: u32,
}

/// Default capabilities for new objects (READ + WRITE).
const DEFAULT_CAPS: u16 = 0x0003; // READ (0x01) | WRITE (0x02)

impl ObjectHeader {
    /// Creates a new object header with CBGR tracking.
    pub fn new(type_id: TypeId, generation: u32, size: u32) -> Self {
        // Get current epoch from global counter
        let epoch = (verum_common::cbgr::current_epoch() & 0xFFFF) as u16;
        Self {
            type_id,
            generation,
            flags: ObjectFlags::empty(),
            refcount: 1,
            size,
            epoch,
            capabilities: DEFAULT_CAPS,
            _padding: 0,
        }
    }

    /// Creates a new object header with specific epoch.
    pub fn with_epoch(type_id: TypeId, generation: u32, size: u32, epoch: u16) -> Self {
        Self {
            type_id,
            generation,
            flags: ObjectFlags::empty(),
            refcount: 1,
            size,
            epoch,
            capabilities: DEFAULT_CAPS,
            _padding: 0,
        }
    }

    /// Returns true if the object is valid (not freed).
    pub fn is_valid(&self) -> bool {
        !self.flags.contains(ObjectFlags::FREED)
    }

    /// DROP-GLUE-TYPEID-1 (runtime leg): layout-plausibility gate for
    /// drop-glue dispatch.  Returns true only when this header's `size`
    /// field is consistent with an allocation the interpreter could
    /// have produced FOR the given descriptor:
    ///
    /// * record (`New` / `NewG`): `size == max(fields, 1) * 8`
    ///   (`handle_new` allocates exactly `field_count.max(1)` Value
    ///   slots);
    /// * sum type (`MakeVariantTyped`): `size == 8 + payload * 8` for
    ///   SOME declared variant (`alloc_variant_into_with_type_id`
    ///   allocates the 8-byte tag header plus the payload slots;
    ///   payload is tuple `arity` or record `fields.len()`).
    ///
    /// Why this exists: `DropRef` classifies its operand only as
    /// "regular pointer"; an INTERIOR pointer (`&obj.field as *const T`
    /// in baked stdlib bytecode) or a stale/corrupt pointer passes that
    /// test, and the bytes at the pointee then read as a fake header
    /// whose garbage `type_id` word can coincide with a REAL descriptor
    /// that carries a `drop_fn`.  The name-guard on the resolved
    /// function ("must be `<T>.drop`") cannot catch this — the foreign
    /// descriptor's drop IS a genuine drop.  The size cross-check is
    /// the discriminating fact the garbage almost never satisfies
    /// (observed live: `WindowsCondvar` dispatch against a fake header
    /// with `size == 1`).  A gated-out genuine object merely falls
    /// through to the builtin CBGR cleanup — strictly safer than
    /// executing an arbitrary Drop body over foreign memory.
    ///
    /// Freed objects are never plausible drop targets.
    pub fn layout_matches_descriptor(
        &self,
        desc: &crate::types::TypeDescriptor,
    ) -> bool {
        if !self.is_valid() {
            return false;
        }
        let sz = self.size as usize;
        let slot = std::mem::size_of::<crate::value::Value>();
        if sz == 0 || sz % slot != 0 {
            return false;
        }
        if !desc.variants.is_empty() {
            desc.variants.iter().any(|v| {
                let payload = if v.fields.is_empty() {
                    v.arity as usize
                } else {
                    v.fields.len()
                };
                sz == 8 + payload * slot
            })
        } else {
            sz == desc.fields.len().max(1) * slot
        }
    }

    /// Increments the reference count.
    pub fn incref(&mut self) {
        self.refcount = self.refcount.saturating_add(1);
    }

    /// Decrements the reference count. Returns true if count reaches zero.
    pub fn decref(&mut self) -> bool {
        self.refcount = self.refcount.saturating_sub(1);
        self.refcount == 0
    }

    /// Validates a CBGR reference against this header.
    ///

    /// Returns Ok(()) if valid, or an error describing the violation.
    pub fn validate(&self, expected_gen: u32, expected_epoch: u16) -> InterpreterResult<()> {
        if !self.is_valid() {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::UseAfterFree,
                ptr: 0,
            });
        }

        if self.generation != expected_gen {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::GenerationMismatch,
                ptr: 0,
            });
        }

        // Epoch validation with window check
        let epoch_diff = self.epoch.wrapping_sub(expected_epoch);
        if epoch_diff > 0x7FFF && epoch_diff != 0 {
            return Err(InterpreterError::CbgrViolation {
                kind: CbgrViolationKind::EpochExpired,
                ptr: 0,
            });
        }

        Ok(())
    }

    /// Check if the header has a specific capability.
    pub fn has_capability(&self, cap: u16) -> bool {
        (self.capabilities & cap) == cap
    }

    /// Set capabilities.
    pub fn set_capabilities(&mut self, caps: u16) {
        self.capabilities = caps;
    }

    /// Bare alignment of the `ObjectHeader` struct (`#[repr(C, align(8))]`).
    pub const ALIGN: usize = std::mem::align_of::<Self>();

    /// Returns `true` iff `ptr` is non-null AND aligned to the header's
    /// canonical 8-byte alignment.  Pre-condition that every code path
    /// dereferencing a `*const ObjectHeader` MUST satisfy — Rust's
    /// `panic_misaligned_pointer_dereference` runtime check aborts the
    /// whole interpreter on violation, so this is a soundness gate, not
    /// an optimisation.
    #[inline(always)]
    pub fn ptr_is_aligned(ptr: *const u8) -> bool {
        !ptr.is_null() && (ptr as usize) % Self::ALIGN == 0
    }

    /// Safely read the `ObjectHeader` at `ptr`.
    ///
    /// Returns `None` when `ptr` is null OR misaligned for `ObjectHeader`
    /// (`#[repr(C, align(8))]`).  Use this at every site that casts a
    /// raw `*const u8` to `*const ObjectHeader` and dereferences — the
    /// `Option` discipline forces the caller to handle the
    /// "this-isn't-actually-a-header" case instead of aborting the
    /// interpreter through Rust's UB-level alignment check.
    ///
    /// **Safety:** when `Some` is returned, the caller still relies on
    /// the pointer pointing at a valid `ObjectHeader` — alignment
    /// alone doesn't prove that.  This helper closes only the
    /// alignment hole; the rest of the soundness boundary (`ptr` must
    /// originate from a `handle_new` / CBGR allocation that wrote a
    /// header at that address) is preserved by the calling
    /// invariants.
    ///
    /// # Safety
    /// The caller MUST guarantee that `ptr`, if aligned and non-null,
    /// points to a valid `ObjectHeader` whose backing allocation is
    /// still live for the duration of the returned reference.
    #[inline(always)]
    pub unsafe fn try_from_ptr<'a>(ptr: *const u8) -> Option<&'a Self> {
        // Reject NaN-box special-value marker payloads (FatRef / ThinRef /
        // Generator / boxed-int).  A real heap pointer is a canonical
        // user-space address with bit 47 clear; EVERY special-value marker
        // sets bit 47 (`SPECIAL_VALUE_MARKER`, value.rs) — that invariant is
        // exactly what `Value::is_regular_ptr` relies on to tell a real
        // pointer from a marker.  `as_ptr()` on a FatRef/ThinRef VALUE hands
        // back its marker payload (e.g. `FAT_REF_MARKER = 0xe00000000000`),
        // which is 8-aligned but points at an unmapped address, so reading a
        // header there SIGSEGVs.  This single guard makes EVERY
        // `is_ptr()`→`as_ptr()`→header-deref site (there are ~200 `is_ptr`
        // call sites) safe against the FatRef-as-pointer class in one place:
        // a special-value reaching a header-deref site is a mis-dispatch, and
        // None/stub is the correct benign result — identical to how a
        // misaligned pointer is already handled.  Legitimate interior derefs
        // go through `FatRef::ptr()` / `ThinRef.ptr`, which are real
        // bit-47-clear addresses and are unaffected.
        const SPECIAL_VALUE_MARKER: u64 = 1u64 << 47;
        if (ptr as u64) & SPECIAL_VALUE_MARKER != 0 {
            return None;
        }
        if Self::ptr_is_aligned(ptr) {
            // SAFETY: alignment proven above; lifetime is caller's
            // responsibility as documented in the trait-level Safety
            // section.
            unsafe { Some(&*(ptr as *const Self)) }
        } else {
            None
        }
    }

    /// Read just the `type_id` field at `ptr`, without retaining a
    /// reference to the header (avoids borrow-conflict patterns where
    /// the surrounding code wants to mutate adjacent state).  Returns
    /// `None` on misalignment / null — see [`Self::try_from_ptr`].
    ///
    /// # Safety
    /// Same as [`Self::try_from_ptr`].
    #[inline(always)]
    pub unsafe fn try_type_id(ptr: *const u8) -> Option<TypeId> {
        // SAFETY: try_from_ptr discharges the alignment gate; lifetime
        // is scoped to this fn so the caller's borrow tracker can't
        // observe the borrow.
        unsafe { Self::try_from_ptr(ptr).map(|h| h.type_id) }
    }

    /// Alignment-safe variant of `&*(ptr as *const ObjectHeader)` that
    /// returns a reference to a static all-zero stub when `ptr` is
    /// null or misaligned for `ObjectHeader`.
    ///
    /// The stub's `type_id` is `TypeId(0)` (not equal to any legitimate
    /// `FIRST_USER+` or canonical-builtin TypeId), so every `if header.type_id == X`
    /// dispatch on it deterministically falls through to its else-
    /// branch — yielding the same semantics callers would have got
    /// from a `if let Some(h) = try_from_ptr(ptr)` pattern, without
    /// the structural rewrite at every call site.
    ///
    /// **When to prefer `try_from_ptr` instead**: when the caller can
    /// usefully act on the *fact* that a pointer is misaligned (e.g.
    /// error out with a typed `InterpreterError` rather than silently
    /// returning a default).  `ref_or_stub` is the right primitive at
    /// dispatch-time discrimination sites; `try_from_ptr` is the
    /// right primitive at validation boundaries.
    ///
    /// # Safety
    /// When `ptr` is aligned and non-null, the caller MUST guarantee
    /// it points to a valid `ObjectHeader` whose backing allocation
    /// is still live for the duration of the returned reference.
    #[inline(always)]
    pub unsafe fn ref_or_stub<'a>(ptr: *const u8) -> &'a Self {
        // SAFETY: contract documented above; alignment-positive case
        // is correct by construction.
        match unsafe { Self::try_from_ptr(ptr) } {
            Some(h) => h,
            None => Self::stub_reference(),
        }
    }

    /// Static all-zero `ObjectHeader` used as a sentinel return value
    /// by [`Self::ref_or_stub`].  Its `type_id` is `TypeId(0)`, which
    /// no live allocation ever takes (`TypeId::FIRST_USER` starts well
    /// above 0 and every canonical builtin TypeId is also non-zero),
    /// so dispatch-time `header.type_id == X` checks deterministically
    /// route through their else-branches.
    fn stub_reference() -> &'static Self {
        // `align(8)` is satisfied by `'static` storage of a
        // `#[repr(C, align(8))]` struct.
        static STUB: ObjectHeader = ObjectHeader {
            type_id: TypeId(0),
            generation: 0,
            flags: ObjectFlags::empty(),
            refcount: 0,
            size: 0,
            epoch: 0,
            capabilities: 0,
            _padding: 0,
        };
        &STUB
    }

    /// Attenuate capabilities (can only remove, not add).
    pub fn attenuate_capabilities(&mut self, mask: u16) {
        self.capabilities &= mask;
    }
}

/// Heap-allocated object (type-erased).
///

/// An `Object` is a pointer to memory that starts with an `ObjectHeader`
/// followed by type-specific data.
#[repr(transparent)]
#[derive(Debug)]
pub struct Object {
    /// Pointer to object header.
    ptr: NonNull<ObjectHeader>,
}

impl Object {
    /// Creates a new object from a raw pointer.
    ///

    /// # Safety
    ///

    /// The pointer must point to valid ObjectHeader followed by data.
    pub unsafe fn from_raw(ptr: *mut ObjectHeader) -> Option<Self> {
        NonNull::new(ptr).map(|ptr| Self { ptr })
    }

    /// Returns a pointer to the header.
    pub fn header(&self) -> &ObjectHeader {
        unsafe { self.ptr.as_ref() }
    }

    /// Returns a mutable pointer to the header.
    pub fn header_mut(&mut self) -> &mut ObjectHeader {
        unsafe { self.ptr.as_mut() }
    }

    /// Returns a pointer to the data portion.
    pub fn data_ptr(&self) -> *mut u8 {
        unsafe { (self.ptr.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) }
    }

    /// Returns the raw pointer.
    pub fn as_ptr(&self) -> *mut ObjectHeader {
        self.ptr.as_ptr()
    }

    /// Returns the type ID.
    pub fn type_id(&self) -> TypeId {
        self.header().type_id
    }

    /// Returns the generation counter.
    pub fn generation(&self) -> u32 {
        self.header().generation
    }

    /// Returns the epoch for CBGR validation.
    pub fn epoch(&self) -> u16 {
        self.header().epoch
    }

    /// Returns the capabilities flags.
    pub fn capabilities(&self) -> u16 {
        self.header().capabilities
    }

    /// Returns the reference count.
    pub fn refcount(&self) -> u16 {
        self.header().refcount
    }

    /// Returns the data size.
    pub fn size(&self) -> u32 {
        self.header().size
    }

    /// Returns a safe slice over the data portion of this object.
    ///

    /// This bounds-checks the size field against a maximum to prevent
    /// reading uninitialized memory if the header is corrupted.
    pub fn data_slice(&self) -> &[u8] {
        let size = self.header().size as usize;
        let ptr = self.data_ptr();
        if ptr.is_null() || size == 0 {
            return &[];
        }
        // Sanity: cap at 256MB to prevent corrupted headers from causing huge reads
        let capped = size.min(256 * 1024 * 1024);
        // SAFETY: data_ptr points to OBJECT_HEADER_SIZE bytes after the allocation start.
        // The allocation was made via alloc_with_init which allocates header + size bytes.
        // We cap the size to prevent corrupted metadata from reading beyond the allocation.
        unsafe { std::slice::from_raw_parts(ptr, capped) }
    }

    /// Validates this object against expected generation and epoch.
    pub fn validate(&self, expected_gen: u32, expected_epoch: u16) -> InterpreterResult<()> {
        self.header().validate(expected_gen, expected_epoch)
    }

    /// Check if the object has a specific capability.
    pub fn has_capability(&self, cap: u16) -> bool {
        self.header().has_capability(cap)
    }
}

/// Heap allocator for interpreter objects.
///

/// Uses simple bump allocation for fast allocation.
/// Collection is mark-sweep when threshold is reached.
pub struct Heap {
    /// Current generation counter.
    generation: u32,

    /// All allocated objects (for GC tracing).
    objects: Vec<NonNull<ObjectHeader>>,

    /// Total allocated bytes.
    allocated: usize,

    /// Collection threshold.
    threshold: usize,

    /// Statistics.
    stats: HeapStats,
}

/// Heap statistics including CBGR validation metrics.
#[derive(Debug, Clone, Default)]
pub struct HeapStats {
    /// Total allocations.
    pub total_allocs: u64,

    /// Total frees.
    pub total_frees: u64,

    /// Total bytes allocated.
    pub total_bytes: u64,

    /// Peak memory usage.
    pub peak_bytes: usize,

    /// Number of GC collections.
    pub collections: u64,

    /// CBGR validation checks performed.
    pub cbgr_validations: u64,

    /// CBGR validation failures.
    pub cbgr_failures: u64,

    /// Generation mismatches detected.
    pub generation_mismatches: u64,

    /// Epoch violations detected.
    pub epoch_violations: u64,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    /// Creates a new heap with default settings.
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_HEAP_SIZE)
    }

    /// Creates a new heap with the specified collection threshold.
    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            generation: 1,
            objects: Vec::with_capacity(1024),
            allocated: 0,
            threshold,
            stats: HeapStats::default(),
        }
    }

    /// Allocates an object of the given type and size.
    pub fn alloc(&mut self, type_id: TypeId, size: usize) -> InterpreterResult<Object> {
        self.alloc_with_init(type_id, size, |_| {})
    }

    /// Allocates an object with custom initialization.
    pub fn alloc_with_init<F>(
        &mut self,
        type_id: TypeId,
        size: usize,
        init: F,
    ) -> InterpreterResult<Object>
    where
        F: FnOnce(&mut [u8]),
    {
        // Guard against unbounded allocation (DoS prevention)
        if size > MAX_ALLOCATION_SIZE {
            return Err(InterpreterError::OutOfMemory {
                requested: size,
                available: self.threshold.saturating_sub(self.allocated),
            });
        }

        // Guard against u32 truncation: ObjectHeader stores size as u32
        if size > u32::MAX as usize {
            return Err(InterpreterError::OutOfMemory {
                requested: size,
                available: self.threshold.saturating_sub(self.allocated),
            });
        }

        let total_size = OBJECT_HEADER_SIZE + size;
        let layout = Layout::from_size_align(total_size, MIN_ALIGNMENT).map_err(|_| {
            InterpreterError::OutOfMemory {
                requested: total_size,
                available: self.threshold.saturating_sub(self.allocated),
            }
        })?;

        // Check threshold (simple GC trigger)
        if self.allocated + total_size > self.threshold {
            // In a real implementation, trigger GC here
            // For now, just extend threshold
            self.threshold = (self.threshold * 2).max(self.allocated + total_size);
        }

        // Allocate memory
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err(InterpreterError::OutOfMemory {
                requested: total_size,
                available: 0,
            });
        }

        // Get next generation
        let generation = self.next_generation();

        // Initialize header
        let header_ptr = ptr as *mut ObjectHeader;
        unsafe {
            header_ptr.write(ObjectHeader::new(type_id, generation, size as u32));
        }

        // Initialize data
        let data_ptr = unsafe { ptr.add(OBJECT_HEADER_SIZE) };
        unsafe {
            std::ptr::write_bytes(data_ptr, 0, size);
        }
        init(unsafe { std::slice::from_raw_parts_mut(data_ptr, size) });

        // Track object
        let nn_ptr = NonNull::new(header_ptr).ok_or(InterpreterError::OutOfMemory {
            requested: total_size,
            available: 0,
        })?;
        self.objects.push(nn_ptr);

        // Update stats
        self.allocated += total_size;
        self.stats.total_allocs += 1;
        self.stats.total_bytes += total_size as u64;
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.allocated);

        Ok(Object { ptr: nn_ptr })
    }

    /// Allocates an array of values.
    pub fn alloc_array(
        &mut self,
        element_type: TypeId,
        length: usize,
    ) -> InterpreterResult<Object> {
        let size = length * std::mem::size_of::<Value>();
        self.alloc(element_type, size)
    }

    /// Allocates a BYTE_SLICE byte-view object (ARCH-P5) over an
    /// existing byte buffer: `[ObjectHeader(BYTE_SLICE)][ptr: i64]
    /// [len: i64]` with RAW (non-NaN-boxed) payload slots.  Canonical
    /// producer surface for `Text.as_bytes()` and BYTE_SLICE
    /// re-slicing — see the module-level BYTE_SLICE helper docs.
    ///
    /// A null `ptr` is normalized to [`empty_byte_slice_ptr`] with
    /// `len = 0` so the never-null contract holds at every producer.
    pub fn alloc_byte_slice(&mut self, ptr: *mut u8, len: u64) -> InterpreterResult<Object> {
        let (ptr, len) = if ptr.is_null() {
            (empty_byte_slice_ptr(), 0)
        } else {
            (ptr, len)
        };
        self.alloc_with_init(TypeId::BYTE_SLICE, 16, |data| {
            // SAFETY: `data` is the freshly-allocated 16-byte payload;
            // both raw i64 slots are within it.
            unsafe {
                let slots = data.as_mut_ptr() as *mut u64;
                *slots = ptr as u64;
                *slots.add(1) = len;
            }
        })
    }

    /// Allocates the canonical IMMUTABLE heap Text record (ARCH-P5
    /// final leg): ONE self-contained object
    /// `[ObjectHeader(TEXT, size = 24 + byte_len)]{ptr, len, cap=0}[bytes…]`
    /// with `ptr` addressing the bytes at payload offset
    /// [`TEXT_RECORD_SIZE`] inside the SAME allocation.  `cap == 0` is
    /// text.vr's static/immutable marker — mutation COW-promotes (see
    /// the module-level TEXT helper docs for the invariant argument).
    ///
    /// Empty input normalizes `ptr` to [`empty_byte_slice_ptr`]
    /// (never-null; never written — `push_*` grows before the first
    /// write and `truncate` early-returns at `len == 0`).
    ///
    /// The single producer surface replacing every legacy
    /// `TypeId(0x0001)` `[len:u64][bytes…]` allocation.
    pub fn alloc_text(&mut self, bytes: &[u8]) -> InterpreterResult<Object> {
        let len = bytes.len();
        let obj = self.alloc(TypeId::TEXT, TEXT_RECORD_SIZE + len)?;
        let base = obj.as_ptr() as *mut u8;
        // SAFETY: the object was just allocated with `24 + len` payload
        // bytes: three 8-byte record slots followed by `len` bytes of
        // UTF-8 storage.  All writes below are within that region.
        unsafe {
            let data = base.add(OBJECT_HEADER_SIZE);
            let bytes_dst = data.add(TEXT_RECORD_SIZE);
            let ptr_v = if len == 0 {
                Value::from_ptr(empty_byte_slice_ptr())
            } else {
                Value::from_ptr(bytes_dst)
            };
            let slots = data as *mut Value;
            *slots = ptr_v;
            *slots.add(1) = Value::from_i64(len as i64);
            *slots.add(2) = Value::from_i64(0); // cap == 0: immutable/static marker
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_dst, len);
        }
        Ok(obj)
    }

    /// Allocates the canonical CAPACITY-CARRYING heap Text record
    /// (the `Text.with_capacity` / `Text.reserve` intercept surface):
    /// `[ObjectHeader(TEXT, size = 24 + cap + 1)]{ptr, len, cap}[storage…]`
    /// where `cap >= max(len, 1)` and the byte region reserves
    /// `cap + 1` bytes, honouring text.vr's owned-buffer convention (a
    /// cap-capacity buffer has `cap + 1` bytes so `push_byte`'s
    /// write-at-`len` + NUL-at-`len+1` stays in-bounds for `len < cap`).
    ///
    /// `cap > 0` marks the record grow-capable; when .vr `grow`
    /// eventually reallocates past `cap` its `dealloc` of the interior
    /// pointer is a Tier-0 no-op (`CbgrDealloc` intentional leak) and
    /// the inline storage goes dormant.
    pub fn alloc_text_with_capacity(
        &mut self,
        bytes: &[u8],
        cap: usize,
    ) -> InterpreterResult<Object> {
        let len = bytes.len();
        let cap = cap.max(len).max(1);
        let obj = self.alloc(TypeId::TEXT, TEXT_RECORD_SIZE + cap + 1)?;
        let base = obj.as_ptr() as *mut u8;
        // SAFETY: the object was just allocated with `24 + cap + 1`
        // payload bytes (`cap >= len`); the record slots and the
        // `len`-byte copy below are within that region.  `alloc`
        // zero-fills, so the `[len, cap]` tail (incl. the NUL slot)
        // is already zeroed.
        unsafe {
            let data = base.add(OBJECT_HEADER_SIZE);
            let bytes_dst = data.add(TEXT_RECORD_SIZE);
            let slots = data as *mut Value;
            *slots = Value::from_ptr(bytes_dst);
            *slots.add(1) = Value::from_i64(len as i64);
            *slots.add(2) = Value::from_i64(cap as i64);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_dst, len);
        }
        Ok(obj)
    }

    /// Frees an object.
    ///

    /// # Safety
    ///

    /// The object must have been allocated by this heap and must not be
    /// accessed after freeing.
    pub unsafe fn free(&mut self, obj: Object) {
        let header = obj.header();
        let total_size = OBJECT_HEADER_SIZE + header.size as usize;

        // SAFETY: The caller guarantees the object was allocated by this heap
        // with the same alignment.
        unsafe {
            let layout = Layout::from_size_align_unchecked(total_size, MIN_ALIGNMENT);

            // Mark as freed
            (*obj.ptr.as_ptr()).flags |= ObjectFlags::FREED;

            // Deallocate
            dealloc(obj.ptr.as_ptr() as *mut u8, layout);
        }

        // Update stats
        self.allocated = self.allocated.saturating_sub(total_size);
        self.stats.total_frees += 1;

        // Remove from tracking (expensive, but correct)
        self.objects.retain(|p| *p != obj.ptr);
    }

    /// Returns the next generation number.
    ///

    /// When generation reaches GEN_MAX, advances the global epoch and
    /// resets to GEN_INITIAL to prevent generation counter reuse within
    /// the same epoch (ABA prevention).
    pub fn next_generation(&mut self) -> u32 {
        let result = self.generation;
        if self.generation >= verum_common::cbgr::GEN_MAX {
            // SAFETY: Force epoch advance before allowing generation reuse.
            // This invalidates all references from the current epoch, preventing
            // ABA attacks where a new allocation gets the same generation as a
            // freed object.
            let new_epoch = verum_common::cbgr::advance_epoch();

            // Verify epoch actually advanced (protects against epoch counter exhaustion)
            debug_assert!(new_epoch > 0, "Epoch advance must produce a non-zero epoch");

            self.generation = verum_common::cbgr::GEN_INITIAL;
        } else {
            // SAFETY: checked addition - if wrapping_add would exceed GEN_MAX,
            // the >= check above catches it on the next call.
            self.generation = self.generation.wrapping_add(1);
        }
        result
    }

    /// Returns current statistics.
    pub fn stats(&self) -> &HeapStats {
        &self.stats
    }

    /// Returns total allocated bytes.
    pub fn allocated(&self) -> usize {
        self.allocated
    }

    /// Returns the number of live objects.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Returns true iff `ptr` was produced by this heap's allocator (i.e. is a
    /// tracked object whose first 24 bytes are a real `ObjectHeader`).
    ///

    /// Pointers that satisfy `Value::is_ptr` may originate from either this
    /// heap or from the system allocator (via `MemExtended::Alloc`). The
    /// latter are opaque byte buffers with no header; code that needs to
    /// inspect headers safely — most importantly `handle_clone` — must
    /// consult this method first.
    pub fn contains(&self, ptr: *const ObjectHeader) -> bool {
        if ptr.is_null() {
            return false;
        }
        self.objects.iter().any(|nn| std::ptr::eq(nn.as_ptr(), ptr))
    }

    /// Validates a CBGR reference against an object.
    ///

    /// This performs full CBGR validation including generation and epoch checks.
    /// Stats are updated for monitoring.
    pub fn validate_reference(
        &mut self,
        obj: &Object,
        expected_gen: u32,
        expected_epoch: u16,
    ) -> InterpreterResult<()> {
        self.stats.cbgr_validations += 1;

        match obj.validate(expected_gen, expected_epoch) {
            Ok(()) => Ok(()),
            Err(InterpreterError::CbgrViolation { kind, .. }) => {
                self.stats.cbgr_failures += 1;
                match kind {
                    CbgrViolationKind::GenerationMismatch => {
                        self.stats.generation_mismatches += 1;
                    }
                    CbgrViolationKind::EpochExpired => {
                        self.stats.epoch_violations += 1;
                    }
                    _ => {}
                }
                Err(InterpreterError::CbgrViolation {
                    kind,
                    ptr: obj.as_ptr() as usize,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Gets the current epoch for new references.
    pub fn current_epoch(&self) -> u16 {
        (verum_common::cbgr::current_epoch() & 0xFFFF) as u16
    }

    /// Clears all objects (for reset).
    ///

    /// # Safety
    ///

    /// All references to heap objects become invalid.
    pub unsafe fn clear(&mut self) {
        for obj_ptr in self.objects.drain(..) {
            // SAFETY: All objects were allocated by this heap with MIN_ALIGNMENT.
            unsafe {
                let header = obj_ptr.as_ref();
                // T0202 TENSOR-HANDLE-OBJECT-1 (teardown leg): TENSOR
                // carriers hold one inline `TensorHandle` whose `Drop`
                // (decref → free TensorData) must run before the raw
                // bytes go away — otherwise every tensor that never hit
                // `DropRef` (expression temps, still-live bindings at
                // exit) leaks its TensorData for the process lifetime.
                // Idempotent with the DropRef-time reclaim: a payload
                // already dropped there is the empty handle, whose drop
                // is a no-op.
                if header.type_id == crate::types::TypeId::TENSOR {
                    let payload = (obj_ptr.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE)
                        as *mut super::tensor::TensorHandle;
                    super::tensor::take_and_drop_payload(payload);
                }
                let total_size = OBJECT_HEADER_SIZE + header.size as usize;
                let layout = Layout::from_size_align_unchecked(total_size, MIN_ALIGNMENT);
                dealloc(obj_ptr.as_ptr() as *mut u8, layout);
            }
        }
        self.allocated = 0;
        self.generation = 1;
    }

    /// Gets an Object from a data pointer.
    ///

    /// Given a pointer to the data portion of an object (after the header),
    /// this reconstructs the Object wrapper for CBGR operations.
    ///

    /// # Safety
    ///

    /// The pointer must have been returned by `Object::data_ptr()` for
    /// an object allocated from this heap.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn get_object(&self, data_ptr: *mut u8) -> Option<Object> {
        if data_ptr.is_null() {
            return None;
        }

        // SAFETY: Validate pointer arithmetic won't underflow.
        // ObjectHeader requires OBJECT_HEADER_SIZE bytes before the data pointer.
        if (data_ptr as usize) < OBJECT_HEADER_SIZE {
            return None;
        }

        // Calculate header pointer by subtracting header size
        let header_ptr = unsafe { data_ptr.sub(OBJECT_HEADER_SIZE) as *mut ObjectHeader };

        // SAFETY: Check alignment to prevent type confusion via misaligned pointers
        if !(header_ptr as usize).is_multiple_of(std::mem::align_of::<ObjectHeader>()) {
            return None;
        }

        // Verify this is a valid heap object by checking if it's in our object list
        // This is O(n) but provides safety; in production could use a hash set
        let header_nonnull = std::ptr::NonNull::new(header_ptr)?;
        if !self.objects.contains(&header_nonnull) {
            return None;
        }

        // SAFETY: Pointer is in our object list, so header is valid to read.
        // Validate type tag to prevent object forgery via pointer reconstruction.
        let header = unsafe { &*header_ptr };

        // Reject freed objects - prevents type confusion via dangling pointers
        if header.flags.contains(ObjectFlags::FREED) {
            return None;
        }

        // Validate generation is in valid range (not unallocated sentinel)
        if header.generation == 0 {
            return None;
        }

        // SAFETY: All validation checks passed - object is genuine and alive
        unsafe { Object::from_raw(header_ptr) }
    }

    /// Creates a TokenStream heap object from serialized bytes.
    ///

    /// This is used by the MetaQuote instruction handler to create TokenStream
    /// objects directly from pre-serialized bytes stored in the constant pool.
    ///

    /// # Arguments
    ///

    /// * `serialized_data` - Pre-serialized TokenStream bytes
    ///

    /// # Returns
    ///

    /// A heap-allocated Object containing the serialized TokenStream data.
    ///

    /// # Performance
    ///

    /// O(n) where n = serialized data size. Just a single memcpy.
    pub fn alloc_token_stream(&mut self, serialized_data: &[u8]) -> InterpreterResult<Object> {
        self.alloc_with_init(
            crate::types::TypeId::TOKEN_STREAM,
            serialized_data.len(),
            |buf| {
                buf.copy_from_slice(serialized_data);
            },
        )
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        // Free all remaining objects
        unsafe { self.clear() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_header_size() {
        assert_eq!(std::mem::size_of::<ObjectHeader>(), OBJECT_HEADER_SIZE);
    }

    /// DROP-GLUE-TYPEID-1 pins: `layout_matches_descriptor` accepts
    /// exactly the sizes the interpreter allocators produce for the
    /// descriptor and rejects everything else (the fake-header /
    /// interior-pointer class that dispatched foreign Drop impls).
    #[test]
    fn drop_layout_gate_record_sizes() {
        use crate::types::{FieldDescriptor, TypeDescriptor, TypeKind};
        // 1-field record (RwLockWriteGuard / CriticalSection shape):
        // handle_new allocates max(1,1)*8 = 8 data bytes.
        let mut desc = TypeDescriptor {
            kind: TypeKind::Record,
            ..Default::default()
        };
        desc.fields.push(FieldDescriptor::default());
        let ok = ObjectHeader::new(crate::types::TypeId(100), 1, 8);
        assert!(ok.layout_matches_descriptor(&desc));
        // The live WindowsCondvar failure: fake header claimed size=1.
        let fake = ObjectHeader::new(crate::types::TypeId(100), 1, 1);
        assert!(!fake.layout_matches_descriptor(&desc));
        // Wrong slot count for the descriptor.
        let wrong = ObjectHeader::new(crate::types::TypeId(100), 1, 24);
        assert!(!wrong.layout_matches_descriptor(&desc));
        // Zero size is never a real allocation (handle_new mins at 1 slot).
        let zero = ObjectHeader::new(crate::types::TypeId(100), 1, 0);
        assert!(!zero.layout_matches_descriptor(&desc));
        // Freed objects are never plausible drop targets.
        let mut freed = ObjectHeader::new(crate::types::TypeId(100), 1, 8);
        freed.flags |= ObjectFlags::FREED;
        assert!(!freed.layout_matches_descriptor(&desc));
        // Zero-field record: handle_new still allocates one slot.
        let unit_desc = TypeDescriptor {
            kind: TypeKind::Record,
            ..Default::default()
        };
        assert!(ObjectHeader::new(crate::types::TypeId(101), 1, 8)
            .layout_matches_descriptor(&unit_desc));
        assert!(!ObjectHeader::new(crate::types::TypeId(101), 1, 16)
            .layout_matches_descriptor(&unit_desc));
    }

    #[test]
    fn drop_layout_gate_variant_sizes() {
        use crate::types::{
            FieldDescriptor, TypeDescriptor, VariantDescriptor, VariantKind,
        };
        // Sum type with a unit variant (payload 0), a 2-tuple variant,
        // and a 1-field record variant. alloc_variant_into_with_type_id
        // allocates 8 (tag header) + payload*8.
        let mut desc = TypeDescriptor::default();
        desc.variants.push(VariantDescriptor {
            tag: 0,
            kind: VariantKind::Unit,
            arity: 0,
            ..Default::default()
        });
        desc.variants.push(VariantDescriptor {
            tag: 1,
            kind: VariantKind::Tuple,
            arity: 2,
            ..Default::default()
        });
        let mut rec_variant = VariantDescriptor {
            tag: 2,
            kind: VariantKind::Record,
            arity: 0,
            ..Default::default()
        };
        rec_variant.fields.push(FieldDescriptor::default());
        desc.variants.push(rec_variant);

        let tid = crate::types::TypeId(200);
        // Unit variant: 8 + 0.
        assert!(ObjectHeader::new(tid, 1, 8).layout_matches_descriptor(&desc));
        // Tuple(2): 8 + 16.
        assert!(ObjectHeader::new(tid, 1, 24).layout_matches_descriptor(&desc));
        // Record{1}: 8 + 8 — same as unit+1 slot; covered by the 16 case.
        assert!(ObjectHeader::new(tid, 1, 16).layout_matches_descriptor(&desc));
        // No declared variant yields 5 slots.
        assert!(!ObjectHeader::new(tid, 1, 40).layout_matches_descriptor(&desc));
        // Non-slot-granular garbage.
        assert!(!ObjectHeader::new(tid, 1, 13).layout_matches_descriptor(&desc));
    }

    /// Cross-tier drift contract: the Rust `#[repr(C)] ObjectHeader`
    /// struct, the canonical `verum_common::layout::OBJECT_HEADER_SIZE`,
    /// and the Tier-1 LLVM-codegen
    /// `verum_codegen::llvm::runtime::RuntimeLowering::OBJECT_HEADER_SIZE`
    /// MUST all agree. Drift between any two miscompiles every
    /// heap-object field access.
    #[test]
    fn header_struct_size_matches_canonical() {
        assert_eq!(
            std::mem::size_of::<ObjectHeader>() as u64,
            verum_common::layout::OBJECT_HEADER_SIZE,
            "ObjectHeader Rust layout drifted from canonical OBJECT_HEADER_SIZE \
             — check verum_codegen::llvm::runtime field-offset GEPs",
        );
        // Per-collection field-offset arithmetic must match the layout
        // module's derivations exactly.
        assert_eq!(
            std::mem::align_of::<ObjectHeader>() as u64,
            verum_common::layout::POINTER_SIZE,
            "ObjectHeader alignment must match a single machine word",
        );
    }

    #[test]
    fn test_heap_creation() {
        let heap = Heap::new();
        assert_eq!(heap.allocated(), 0);
        assert_eq!(heap.object_count(), 0);
    }

    /// `variant_tag` / `variant_header_pair` round-trip with
    /// `write_variant_data_header` — the canonical writer used by
    /// `pattern_matching::alloc_variant_into_with_type_id`'s
    /// `alloc_with_init` closure.
    #[test]
    fn variant_header_helpers_roundtrip() {
        let mut heap = Heap::new();
        let obj = heap
            .alloc_with_init(TypeId(0x8000), 8 + std::mem::size_of::<Value>(), |data| {
                // SAFETY: data is the variant data section, exactly
                // 16 bytes (8-byte (tag, fc) header + 8-byte payload).
                unsafe { write_variant_data_header(data.as_mut_ptr(), 7, 1) };
            })
            .unwrap();

        let obj_ptr = obj.as_ptr() as *const u8;
        // SAFETY: `obj_ptr` is the full heap-object pointer; we just wrote a
        // valid variant header at the data section.
        unsafe {
            assert_eq!(variant_tag(obj_ptr), 7);
            assert_eq!(variant_header_pair(obj_ptr), (7, 1));
        }
    }

    /// `variant_payload_ptr` / `variant_payload` correctly index into
    /// the payload area at `VARIANT_PAYLOAD_OFFSET = 32`.
    #[test]
    fn variant_payload_helpers_at_canonical_offset() {
        let mut heap = Heap::new();
        let obj = heap
            .alloc_with_init(TypeId(0x8001), 8 + 2 * std::mem::size_of::<Value>(), |data| {
                unsafe {
                    write_variant_data_header(data.as_mut_ptr(), 1, 2);
                    // Write payload[0] = 42, payload[1] = 99 directly so we
                    // can read them back via the helpers.
                    let payload = data.as_mut_ptr().add(8) as *mut Value;
                    *payload = Value::from_i64(42);
                    *payload.add(1) = Value::from_i64(99);
                }
            })
            .unwrap();

        let obj_ptr = obj.as_ptr() as *const u8;
        unsafe {
            assert_eq!(variant_payload(obj_ptr, 0).as_i64(), 42);
            assert_eq!(variant_payload(obj_ptr, 1).as_i64(), 99);
            assert!(!variant_payload_ptr(obj_ptr, 0).is_null());
        }
    }

    /// Drift-protection: the helpers compose `OBJECT_HEADER_SIZE +
    /// (8 = tag+fc width)` to find the payload base; this MUST equal
    /// `VARIANT_PAYLOAD_OFFSET` from the canonical layout.  Pre-this-pin,
    /// the magic `+ 8` was duplicated at every callsite and could drift
    /// from the layout module silently.
    #[test]
    fn variant_payload_offset_canonical() {
        assert_eq!(
            verum_common::layout::OBJECT_HEADER_SIZE + 8,
            verum_common::layout::VARIANT_PAYLOAD_OFFSET,
            "VARIANT_PAYLOAD_OFFSET must equal OBJECT_HEADER_SIZE + sizeof((tag,fc))",
        );
        assert_eq!(
            verum_common::layout::VARIANT_TAG_OFFSET,
            verum_common::layout::OBJECT_HEADER_SIZE,
            "VARIANT_TAG_OFFSET must equal OBJECT_HEADER_SIZE (tag is the first data field)",
        );
    }

    /// Closure layout shares offsets with the variant layout (header
    /// + 8 bytes for the (func_id, capture_count) pair, then captures).
    /// `closure_header` / `closure_captures_ptr` reuse `VARIANT_TAG_OFFSET`
    /// and `VARIANT_PAYLOAD_OFFSET` directly — pin the architectural
    /// alignment so a future closure-layout edit doesn't silently
    /// desync from the variant offsets.
    #[test]
    fn closure_helpers_share_variant_offsets() {
        assert_eq!(
            verum_common::layout::VARIANT_TAG_OFFSET,
            verum_common::layout::OBJECT_HEADER_SIZE,
        );
        assert_eq!(
            verum_common::layout::VARIANT_PAYLOAD_OFFSET,
            verum_common::layout::OBJECT_HEADER_SIZE + 8,
        );
    }

    /// `closure_header` / `closure_captures_ptr` round-trip with
    /// `write_closure_data_header` — produced bytes match what
    /// `handle_make_closure`'s `alloc_with_init` writes, and what
    /// `handle_call_closure` / `call_closure_sync` read back.
    #[test]
    fn closure_header_helpers_roundtrip() {
        let mut heap = Heap::new();
        let payload_a = Value::from_i64(11);
        let payload_b = Value::from_i64(22);
        let obj = heap
            .alloc_with_init(
                TypeId(0xC000),
                8 + 2 * std::mem::size_of::<Value>(),
                |data| unsafe {
                    write_closure_data_header(data.as_mut_ptr(), 42, 2);
                    let captures = data.as_mut_ptr().add(8) as *mut Value;
                    *captures = payload_a;
                    *captures.add(1) = payload_b;
                },
            )
            .unwrap();

        let obj_ptr = obj.as_ptr() as *const u8;
        unsafe {
            let (func_id, capture_count) = closure_header(obj_ptr);
            assert_eq!(func_id, 42);
            assert_eq!(capture_count, 2);
            assert_eq!((*closure_captures_ptr(obj_ptr, 0)).as_i64(), 11);
            assert_eq!((*closure_captures_ptr(obj_ptr, 1)).as_i64(), 22);
        }
    }

    #[test]
    fn test_alloc() {
        let mut heap = Heap::new();

        let obj = heap.alloc(TypeId::INT, 64).unwrap();
        assert_eq!(obj.type_id(), TypeId::INT);
        assert_eq!(obj.size(), 64);
        assert_eq!(heap.object_count(), 1);
        assert!(heap.allocated() > 0);
    }

    #[test]
    fn test_alloc_with_init() {
        let mut heap = Heap::new();

        let obj = heap
            .alloc_with_init(TypeId::TEXT, 16, |data| {
                data.copy_from_slice(b"Hello, World!!\0\0");
            })
            .unwrap();

        let data = unsafe { std::slice::from_raw_parts(obj.data_ptr(), 16) };
        assert_eq!(data, b"Hello, World!!\0\0");
    }

    #[test]
    fn test_generation_increment() {
        let mut heap = Heap::new();

        let obj1 = heap.alloc(TypeId::INT, 8).unwrap();
        let obj2 = heap.alloc(TypeId::INT, 8).unwrap();

        // Generations should be different
        assert_ne!(obj1.generation(), obj2.generation());
    }

    #[test]
    fn test_free() {
        let mut heap = Heap::new();

        let obj = heap.alloc(TypeId::INT, 64).unwrap();
        let initial_alloc = heap.allocated();

        unsafe { heap.free(obj) };

        assert!(heap.allocated() < initial_alloc);
        assert_eq!(heap.object_count(), 0);
    }

    #[test]
    fn test_object_flags() {
        let mut flags = ObjectFlags::empty();
        assert!(!flags.contains(ObjectFlags::MUTABLE));

        flags |= ObjectFlags::MUTABLE;
        assert!(flags.contains(ObjectFlags::MUTABLE));

        flags |= ObjectFlags::BORROWED;
        assert!(flags.contains(ObjectFlags::MUTABLE | ObjectFlags::BORROWED));
    }

    #[test]
    fn test_refcount() {
        let mut header = ObjectHeader::new(TypeId::INT, 0, 8);
        assert_eq!(header.refcount, 1);

        header.incref();
        assert_eq!(header.refcount, 2);

        header.incref();
        assert_eq!(header.refcount, 3);

        assert!(!header.decref());
        assert_eq!(header.refcount, 2);

        assert!(!header.decref());
        assert_eq!(header.refcount, 1);

        assert!(header.decref()); // Reaches zero
        assert_eq!(header.refcount, 0);
    }

    #[test]
    fn test_stats() {
        let mut heap = Heap::new();

        heap.alloc(TypeId::INT, 64).unwrap();
        heap.alloc(TypeId::FLOAT, 128).unwrap();

        let stats = heap.stats();
        assert_eq!(stats.total_allocs, 2);
        assert!(stats.total_bytes > 0);
    }

    #[test]
    fn test_clear() {
        let mut heap = Heap::new();

        for _ in 0..10 {
            heap.alloc(TypeId::INT, 64).unwrap();
        }

        assert_eq!(heap.object_count(), 10);

        unsafe { heap.clear() };

        assert_eq!(heap.object_count(), 0);
        assert_eq!(heap.allocated(), 0);
    }

    #[test]
    fn test_alloc_array() {
        let mut heap = Heap::new();

        let obj = heap.alloc_array(TypeId::INT, 100).unwrap();
        let expected_size = 100 * std::mem::size_of::<Value>();
        assert_eq!(obj.size() as usize, expected_size);
    }

    // ====================================================================
    // ObjectHeader alignment-safety primitives
    // — Task #14 SIGABRT closure: every cast of `*const u8 → *const
    // ObjectHeader` followed by a dereference now routes through
    // `try_from_ptr` / `try_type_id` / `ref_or_stub`.  The helpers
    // discharge Rust's UB-level alignment check, replacing
    // panic_misaligned_pointer_dereference (which aborts the whole
    // interpreter via SIGABRT) with deterministic Option / sentinel
    // semantics.  These tests pin the soundness invariant.
    // ====================================================================

    #[test]
    fn object_header_try_from_ptr_rejects_null() {
        assert!(unsafe { ObjectHeader::try_from_ptr(std::ptr::null()).is_none() });
    }

    #[test]
    fn object_header_try_from_ptr_rejects_misaligned() {
        // Allocate an 8-byte buffer aligned to 8, then offset by 1
        // to construct a guaranteed-misaligned pointer.  Any value
        // 1..7 works; 1 is the most adversarial.
        let buf: [u8; 16] = [0; 16];
        let base = buf.as_ptr();
        for offset in 1..ObjectHeader::ALIGN {
            let misaligned = unsafe { base.add(offset) };
            assert_eq!(
                (misaligned as usize) % ObjectHeader::ALIGN,
                offset,
                "offset {} is genuinely misaligned",
                offset
            );
            assert!(
                unsafe { ObjectHeader::try_from_ptr(misaligned).is_none() },
                "try_from_ptr({:p}) at offset {} must return None instead of panic-aborting",
                misaligned,
                offset
            );
        }
    }

    #[test]
    fn object_header_try_type_id_returns_none_on_misalignment() {
        let buf: [u8; 16] = [0; 16];
        let misaligned = unsafe { buf.as_ptr().add(3) };
        assert!(unsafe { ObjectHeader::try_type_id(misaligned).is_none() });
    }

    #[test]
    fn object_header_ref_or_stub_returns_zero_typeid_stub_on_misalignment() {
        let buf: [u8; 16] = [0; 16];
        let misaligned = unsafe { buf.as_ptr().add(3) };
        let stub = unsafe { ObjectHeader::ref_or_stub(misaligned) };
        // Stub MUST not collide with any live TypeId — `TypeId(0)`
        // is the canonical "no allocation has this ID" sentinel.
        assert_eq!(stub.type_id, TypeId(0));
        assert_eq!(stub.size, 0);
        assert_eq!(stub.refcount, 0);
    }

    #[test]
    fn object_header_ref_or_stub_reads_real_header_when_aligned() {
        let mut heap = Heap::new();
        let obj = heap.alloc(TypeId::TEXT, 32).unwrap();
        let ptr = obj.as_ptr() as *const u8;
        let header = unsafe { ObjectHeader::ref_or_stub(ptr) };
        assert_eq!(
            header.type_id,
            TypeId::TEXT,
            "aligned heap pointer must read its real type_id, not the stub"
        );
    }

    #[test]
    fn object_header_stub_reference_is_repeatable() {
        // `ref_or_stub` returns a reference to a `'static` stub — two
        // calls with the same misaligned input must produce the same
        // reference (no per-call allocation).
        let buf: [u8; 16] = [0; 16];
        let p1 = unsafe { buf.as_ptr().add(1) };
        let p2 = unsafe { buf.as_ptr().add(3) };
        let s1 = unsafe { ObjectHeader::ref_or_stub(p1) } as *const _;
        let s2 = unsafe { ObjectHeader::ref_or_stub(p2) } as *const _;
        assert_eq!(
            s1, s2,
            "all misaligned inputs must alias to the same static stub"
        );
    }

    /// T0202 (teardown leg): `clear` must run the TENSOR payload glue
    /// — the inline `TensorHandle`'s drop (decref) — before freeing
    /// the carrier bytes, so expression temps that never hit DropRef
    /// still release their TensorData at interpreter teardown.
    #[test]
    fn clear_runs_tensor_carrier_glue() {
        use super::super::tensor::{DType, TensorHandle};

        let mut heap = Heap::new();
        let handle = TensorHandle::zeros(&[16], DType::F64).unwrap();
        // Co-owning witness keeps the shared TensorData observable
        // after `clear` reclaims the carrier's payload.
        let witness = handle.clone();
        let rc = |h: &TensorHandle| -> u32 {
            // SAFETY: witness co-owns, so TensorData outlives this test.
            unsafe { (*h.data.unwrap().as_ptr()).refcount() }
        };
        assert_eq!(rc(&witness), 2);

        let size = std::mem::size_of::<TensorHandle>();
        let mut moved = Some(handle);
        heap.alloc_with_init(crate::types::TypeId::TENSOR, size, |data| {
            // SAFETY: fresh `size`-byte 8-aligned region; `ptr::write`
            // moves the handle in without reading the destination.
            let h = moved.take().expect("init runs once");
            unsafe { std::ptr::write(data.as_mut_ptr() as *mut TensorHandle, h) };
        })
        .unwrap();

        // SAFETY: no outstanding references into this test-local heap.
        unsafe { heap.clear() };
        assert_eq!(
            rc(&witness),
            1,
            "clear must drop the carrier payload (decref exactly once)"
        );
    }
}
