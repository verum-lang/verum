//! CBGR reference encoding/decoding/validation helpers.
//!
//! **SOLE authority** for the interpreter's Tier-0 register-reference encoding.
//! Every dispatch handler that creates, classifies, or decodes a register-ref
//! funnels through `encode_cbgr_ref{,_mut}` / `is_cbgr_ref` / `decode_cbgr_ref`
//! / `is_cbgr_ref_mutable` / `strip_cbgr_ref_mutability` here.
//!
//! # T0367 / CBGR-REGREF-TAG-1 — the soundness fix
//!
//! Register-refs used to be UNTAGGED negative inline `Int`s
//! (`-1 - abs - (gen << 32)`), squatting inside `TAG_INTEGER` space. A legal
//! user `Int` of the form `-(k·2^32 + m)` with `m ≤ 2^24` therefore
//! false-positived [`is_cbgr_ref`], decoded to a garbage absolute register
//! index, and hit `Registers::get_absolute` — a loud ICE in debug and a SILENT
//! substitution of a random register's value for the user's `Int` in release.
//! The `1 << 24` abs-index bound shrank the collision window but never closed
//! it — refs stayed forgeable from data, breaking the three-tier CBGR premise.
//!
//! The encoding now lives in a DISCRIMINATED tag (`Value::cbgr_regref`, under
//! `TAG_UNIT` with a marker bit — see `value.rs`), disjoint from `TAG_INTEGER`,
//! so `is_cbgr_ref` is an EXACT tag test and a user `Int` can never collide.
//!
//! `VERUM_CBGR_LEGACY_INT_REFS=1` restores the pre-T0367 int-space encoding
//! (with its soundness hole) for A/B differential comparison ONLY.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use crate::value::{Capabilities, Value};
use std::sync::OnceLock;

/// Sentinel generation value that disables generation checks on deref.
/// Fits in the tagged encoding's 22-bit generation field.
pub(super) const CBGR_NO_CHECK_GENERATION: u32 = 0x7FFE;

/// Default epoch window size for FatRef validation.
/// Half of 16-bit range allows ~32K epoch advances before invalidation.
pub(super) const EPOCH_WINDOW_SIZE: u16 = 0x7FFF;

/// Width of the generation field carried by a tagged register-ref (bits).
/// The slot generation is a full 32-bit counter (`registers::GEN_MAX`); the ref
/// carries its low 22 bits and [`validate_cbgr_generation`] compares modulo
/// 2^22 (the ThinRef 16-bit epoch-window precedent).
const CBGR_REGREF_GEN_BITS: u32 = 22;
/// Mask for the low `CBGR_REGREF_GEN_BITS` of a generation counter.
const CBGR_REGREF_GEN_MASK: u32 = (1u32 << CBGR_REGREF_GEN_BITS) - 1;

// --- Kill-switch: legacy int-space encoding (A/B only) ----------------------

/// `VERUM_CBGR_LEGACY_INT_REFS=1` (or `true`) restores the pre-T0367
/// negative-inline-`Int` register-ref encoding. Read once and cached. Default
/// off. Exists solely so a differential run can confirm the tagged encoding is
/// what closed the T0367 soundness hole.
#[inline(always)]
fn legacy_int_refs() -> bool {
    static CELL: OnceLock<bool> = OnceLock::new();
    *CELL.get_or_init(|| {
        crate::interpreter::env_flags::get(crate::interpreter::env_flags::Flag::CbgrLegacyIntRefs).map(std::ffi::OsString::from)
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false)
    })
}

/// Legacy mutability bit (bit 31 of the abs word) — legacy encoding only.
const LEGACY_CBGR_MUTABLE_BIT: u32 = 0x8000_0000;
/// Legacy abs-index sanity ceiling (the `1 << 24` window that never closed the
/// hole) — legacy `is_cbgr_ref` only.
const LEGACY_CBGR_REF_ABS_INDEX_MAX: u32 = 1 << 24;

#[inline(always)]
fn legacy_encode(abs_index: u32, generation: u32) -> i64 {
    -1i64 - (abs_index as i64) - ((generation as i64) << 32)
}

#[inline(always)]
fn legacy_decode(encoded: i64) -> (u32, u32) {
    let raw = -(encoded + 1);
    let abs_index = ((raw & 0xFFFF_FFFF) as u32) & !LEGACY_CBGR_MUTABLE_BIT;
    let generation = ((raw >> 32) & 0xFFFF_FFFF) as u32;
    (abs_index, generation)
}

// --- Encode / decode / classify (SOLE authority) ----------------------------

/// Encodes an immutable register-based reference with CBGR generation tracking.
///
/// Returns a discriminated tagged `Value` (never an `Int`) — see the module
/// docs. Under the kill-switch it returns the legacy negative-`Int` form.
#[inline(always)]
pub(super) fn encode_cbgr_ref(abs_index: u32, generation: u32) -> Value {
    if legacy_int_refs() {
        Value::from_i64(legacy_encode(abs_index, generation))
    } else {
        Value::cbgr_regref(abs_index, generation, false)
    }
}

/// Encodes a mutable (`&mut`) register-based reference.
#[inline(always)]
pub(super) fn encode_cbgr_ref_mut(abs_index: u32, generation: u32) -> Value {
    if legacy_int_refs() {
        Value::from_i64(legacy_encode(abs_index | LEGACY_CBGR_MUTABLE_BIT, generation))
    } else {
        Value::cbgr_regref(abs_index, generation, true)
    }
}

/// Decodes a register-based reference into `(abs_index, generation)`.
///
/// PRECONDITION: `encoded` is a register-ref ([`is_cbgr_ref`] returned true) —
/// every call site gates on that (the pinned contract). In the tagged form the
/// generation is the low 22 bits of the slot counter.
///
/// Visibility is `pub(in crate::interpreter)` so the collection-normaliser in
/// `interpreter::state` routes through this SOLE authority (T0367 deleted its
/// inlined copy) and so the `do_return` frame-escape stabilisation in
/// `dispatch_table::mod.rs` can read the ref's `abs_index`.
#[inline(always)]
pub(in crate::interpreter) fn decode_cbgr_ref(encoded: Value) -> (u32, u32) {
    if legacy_int_refs() {
        legacy_decode(encoded.as_i64())
    } else {
        (encoded.cbgr_regref_abs(), encoded.cbgr_regref_generation())
    }
}

/// Checks if a CBGR register reference is mutable (created via RefMut).
///
/// PRECONDITION: `encoded` is a register-ref ([`is_cbgr_ref`] returned true).
#[inline(always)]
pub(super) fn is_cbgr_ref_mutable(encoded: Value) -> bool {
    if legacy_int_refs() {
        let raw = -(encoded.as_i64() + 1);
        ((raw & 0xFFFF_FFFF) as u32) & LEGACY_CBGR_MUTABLE_BIT != 0
    } else {
        encoded.cbgr_regref_is_mut()
    }
}

/// Strips the mutability bit from a CBGR register reference, making it immutable.
/// Used for capability downgrade (`&mut` -> `&`).
#[inline(always)]
pub(super) fn strip_cbgr_ref_mutability(encoded: Value) -> Value {
    if is_cbgr_ref_mutable(encoded) {
        let (abs_index, generation) = decode_cbgr_ref(encoded);
        encode_cbgr_ref(abs_index, generation)
    } else {
        encoded
    }
}

/// Checks if a Value is a CBGR register-based reference.
///
/// EXACT tag test (T0367): a register-ref is `Value::cbgr_regref` — `TAG_UNIT`
/// with the register-ref marker bit set — which is disjoint from every `Int`,
/// so no user integer, of any value or sign, can be misclassified. There is no
/// arithmetic-range guessing and no residual collision window.
///
/// Under `VERUM_CBGR_LEGACY_INT_REFS=1` this falls back to the pre-T0367
/// negative-`Int` range guess (which carries the soundness hole BY DESIGN, for
/// A/B comparison only).
///
/// Visibility is `pub(in crate::interpreter)` for the `do_return` frame-escape
/// gate in `dispatch_table::mod.rs` and the collection normaliser in
/// `interpreter::state`.
#[inline(always)]
pub(in crate::interpreter) fn is_cbgr_ref(val: &Value) -> bool {
    if legacy_int_refs() {
        if !val.is_inline_int() {
            return false;
        }
        let encoded = val.as_i64();
        if encoded >= -(1i64 << 32) {
            return false;
        }
        let (abs_index, _) = legacy_decode(encoded);
        abs_index <= LEGACY_CBGR_REF_ABS_INDEX_MAX
    } else {
        val.is_cbgr_regref()
    }
}

/// Resolve a method-call argument Value to its underlying scalar Value,
/// regardless of which reference shape the caller passed.
///
/// Verum lowers `&expr` to one of three runtime shapes depending on the
/// shape of `expr`:
///
///   * **CBGR register-ref** — emitted by `UnOp::Ref` on a bare variable
///     (`&x` where `x: Int`). A discriminated tagged `Value` (T0367,
///     `Value::cbgr_regref`) packing `(abs_index, generation, mutable)`;
///     classified by [`is_cbgr_ref`] and decoded via [`decode_cbgr_ref`].
///   * **Heap-interior pointer** — emitted by `RefListElement` / `RefField`
///     when the borrow target lives inside a heap object (`&xs[i]`,
///     `&record.field`). Encoded as `Value::from_ptr(elem_ptr)` and
///     tracked in `state.cbgr_mutable_ptrs` so the generic Deref handler
///     reads through it instead of returning the pointer as a value.
///   * **ThinRef** — 16-byte CBGR reference produced by some FFI / mem
///     paths; pointer + generation + epoch in heap. Auto-derefed via
///     `*const Value` when non-null.
///
/// Every primitive-method intercept that consumes `other: &T` MUST funnel
/// the argument through this helper. The buggy alternative — checking
/// only `is_cbgr_ref` and otherwise calling `val.as_i64()` — silently
/// returns the raw pointer address when the caller borrowed a list
/// element or a field, producing nonsense comparisons (e.g. for
/// `xs[0].cmp(&xs[1])` the bubble-sort decision flips).
///
/// Register references follow the caller's CBGR generation-check policy.
/// Heap slot projections always validate their current owner and storage.
#[inline]
pub(super) fn resolve_arg_value(
    state: &super::super::super::state::InterpreterState,
    val: Value,
) -> InterpreterResult<Value> {
    if is_cbgr_ref(&val) {
        let (abs_index, _gen) = decode_cbgr_ref(val);
        return Ok(state.registers.get_absolute(abs_index));
    }
    if let Some(slot) = reference_value_slot(state, val)? {
        // SAFETY: the reference carrier establishes an initialized Value slot.
        return Ok(unsafe { *slot });
    }
    Ok(val)
}

/// Read exactly one declared reference, retaining register generation validation.
pub(super) fn read_live_reference(state: &InterpreterState, reference: Value) -> InterpreterResult<Value> {
    if is_cbgr_ref(&reference) {
        let (index, generation) = decode_cbgr_ref(reference);
        if index as usize >= state.registers.top() {
            return Err(InterpreterError::InvalidOperand { message: "reference is outside its register scope".into() });
        }
        validate_cbgr_generation(state, index, generation)?;
        Ok(state.registers.get_absolute(index))
    } else {
        require_reference_capability(state, reference, Capabilities::READ)?;
        resolve_arg_value(state, reference)
    }
}

/// Pointer references to one Value slot. Bridge allocations carry flat data,
/// so their extent authority takes precedence over the interior-slot registry.
#[inline]
fn reference_value_slot(
    state: &super::super::super::state::InterpreterState,
    value: Value,
) -> InterpreterResult<Option<*mut Value>> {
    if value.is_regular_ptr() && !value.is_nil() {
        let address = value.as_ptr::<u8>() as usize;
        if address % std::mem::align_of::<Value>() == 0
            && super::cbgr::bridge_extent_room(state, address).is_none()
            && state.cbgr_mutable_ptrs.contains(&address)
        {
            return Ok(Some(address as *mut Value));
        }
    }
    if value.is_thin_ref() {
        let reference = state.heap.thin_ref(value)?;
        if !reference.ptr.is_null()
            && reference.ptr as usize % std::mem::align_of::<Value>() == 0
        {
            return Ok(Some(reference.ptr.cast::<Value>()));
        }
    }
    Ok(None)
}

/// Resolve one inline record field within its live raw allocation. Record
/// fields use Value slots in the interpreter, without an ObjectHeader prefix.
pub(super) fn raw_record_field_slot(
    state: &InterpreterState,
    receiver: Value,
    byte_offset: usize,
) -> InterpreterResult<Option<*mut Value>> {
    let address = if receiver.is_regular_ptr() && !receiver.is_nil() {
        receiver.as_ptr::<u8>() as usize
    } else if receiver.is_int() && receiver.as_i64() > 0 {
        receiver.as_i64() as usize
    } else {
        return Ok(None);
    };
    let Some(room) = super::cbgr::bridge_extent_room(state, address) else {
        return Ok(None);
    };
    let slot_size = std::mem::size_of::<Value>();
    if byte_offset.checked_add(slot_size).is_none_or(|end| end > room) {
        return Err(InterpreterError::InvalidOperand {
            message: format!(
                "record field at byte offset {byte_offset} requires {slot_size} bytes, \
                 but the live raw allocation has {room} bytes remaining"
            ),
        });
    }
    let field = address.checked_add(byte_offset).ok_or_else(|| {
        InterpreterError::InvalidOperand { message: "record field address overflow".into() }
    })?;
    if !field.is_multiple_of(std::mem::align_of::<Value>()) {
        return Err(InterpreterError::InvalidOperand {
            message: "record field address is not aligned for a Value slot".into(),
        });
    }
    Ok(Some(field as *mut Value))
}

/// A validated, single Value slot. Register indices stay in the register file;
/// managed addresses are resolved once, with no callback or allocation in the exchange.
pub(super) enum MutableValueSlot {
    Register(u32),
    Memory(*mut Value),
}

impl MutableValueSlot {
    pub(super) fn resolve(state: &InterpreterState, reference: Value) -> InterpreterResult<Self> {
        let invalid = || InterpreterError::InvalidOperand {
            message: "expected a live, aligned mutable Value slot".into(),
        };
        if is_cbgr_ref(&reference) {
            if !is_cbgr_ref_mutable(reference) {
                return Err(InterpreterError::CapabilityDenied {
                    required: u64::from(Capabilities::WRITE),
                    actual: u64::from(Capabilities::READ_ONLY.0),
                });
            }
            let (index, generation) = decode_cbgr_ref(reference);
            if index as usize >= state.registers.top() {
                return Err(invalid());
            }
            validate_cbgr_generation(state, index, generation)?;
            return Ok(Self::Register(index));
        }
        require_reference_write(state, reference)?;
        reference_value_slot(state, reference)?.map(Self::Memory).ok_or_else(invalid)
    }

    /// Increment only a live nonzero UInt32. No allocation occurs after CAS.
    pub(super) fn retain_u32(&self, state: &mut InterpreterState) -> InterpreterResult<()> {
        use std::sync::atomic::{AtomicU32, Ordering};
        let next = |value: Value| -> InterpreterResult<Value> {
            let value = value.try_as_i64().filter(|v| *v > 0 && *v < i64::from(u32::MAX))
                .ok_or_else(|| InterpreterError::Panic { message: "retain_copy requires a nonzero, non-overflowing UInt32 counter".into() })?;
            Ok(Value::from_i64(value + 1))
        };
        match *self {
            Self::Register(index) => {
                // Registers are local to this interpreter; the reference was
                // checked before allocation, which cannot relocate register slots.
                let value = next(state.registers.get_absolute(index))?;
                state.registers.set_absolute(index, value);
            }
            Self::Memory(address) => {
                // Value's tag word is immutable for this UInt32 storage. Use
                // exactly the existing UInt32 atomic width, never an overlapping
                // AtomicU64 access while other owners perform 32-bit release.
                let little_endian = 1u64.to_ne_bytes()[0] == 1;
                let low = usize::from(!little_endian);
                let words = address.cast::<u32>();
                // SAFETY: resolve checked an aligned live Value slot. The tag
                // occupies disjoint immutable bytes; the payload is atomic.
                let tag = unsafe { words.add(1 - low).read() };
                if tag != (Value::from_i64(1).bits() >> 32) as u32 {
                    return Err(InterpreterError::InvalidOperand { message: "retain_copy counter has no UInt32 Value representation".into() });
                }
                // SAFETY: the unsafe contract requires all owners to use this
                // same UInt32 atomic counter and keep its containing slot alive.
                let counter = unsafe { &*words.add(low).cast::<AtomicU32>() };
                let mut observed = counter.load(Ordering::Relaxed);
                loop {
                    if observed == 0 || observed == u32::MAX {
                        return Err(InterpreterError::Panic { message: "retain_copy requires a nonzero, non-overflowing UInt32 counter".into() });
                    }
                    match counter.compare_exchange_weak(observed, observed + 1, Ordering::Relaxed, Ordering::Relaxed) {
                        Ok(_) => break,
                        Err(current) => observed = current,
                    }
                }
            }
        }
        Ok(())
    }

    fn read(&self, state: &InterpreterState) -> Value {
        match *self {
            Self::Register(index) => state.registers.get_absolute(index),
            // SAFETY: resolve established an aligned initialized slot, still live.
            Self::Memory(address) => unsafe { address.read() },
        }
    }

    fn write(&self, state: &mut InterpreterState, value: Value) {
        match *self {
            Self::Register(index) => state.registers.set_absolute(index, value),
            // SAFETY: resolve established write access to this exact slot.
            Self::Memory(address) => unsafe { address.write(value); },
        }
    }
}

pub(super) fn swap_referents(
    state: &mut InterpreterState, left: Value, right: Value,
) -> InterpreterResult<()> {
    let first = MutableValueSlot::resolve(state, left)?;
    let second = MutableValueSlot::resolve(state, right)?;
    let a = first.read(state);
    let b = second.read(state);
    first.write(state, b);
    second.write(state, a);
    state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
    Ok(())
}

/// Check the capability carried by a heap reference before a store or writeback.
pub(super) fn require_reference_write(state: &super::super::super::state::InterpreterState, reference: Value) -> InterpreterResult<()> {
    require_reference_capability(state, reference, Capabilities::WRITE)
}

fn require_reference_capability(state: &InterpreterState, reference: Value, required: u16) -> InterpreterResult<()> {
    let actual = if reference.is_thin_ref() {
        state.heap.thin_ref(reference)?.capabilities()
    } else if reference.is_fat_ref() {
        reference.as_fat_ref().capabilities()
    } else {
        return Ok(());
    };
    if actual.has(required) {
        Ok(())
    } else {
        Err(InterpreterError::CapabilityDenied {
            required: u64::from(required),
            actual: u64::from(actual.0),
        })
    }
}

/// A receiver value together with its original writeback place and the
/// intersection of every reference capability traversed to reach it.
pub(super) struct ReceiverPlace {
    pub value: Value,
    pub reference: Value,
    pub capabilities: Capabilities,
}

impl ReceiverPlace {
    pub(super) fn require_write(&self) -> InterpreterResult<()> {
        if self.capabilities.has(Capabilities::WRITE) {
            Ok(())
        } else {
            Err(InterpreterError::CapabilityDenied {
                required: u64::from(Capabilities::WRITE),
                actual: u64::from(self.capabilities.0),
            })
        }
    }
}

/// Select a receiver's current value without discarding its access authority.
/// Register-reference forwarding retains every incoming restriction. A heap
/// slot is loaded once: its value can itself be an owned record pointer, whose
/// first field must not be confused with another indirection.
pub(super) fn resolve_receiver_place(
    state: &InterpreterState,
    receiver: Value,
) -> InterpreterResult<ReceiverPlace> {
    let mut place = receiver;
    let mut capabilities = Capabilities::FULL;
    for _ in 0..8 {
        let incoming = if is_cbgr_ref(&place) {
            if is_cbgr_ref_mutable(place) {
                Capabilities::MUT_EXCLUSIVE
            } else {
                Capabilities::READ_ONLY
            }
        } else if place.is_thin_ref() {
            state.heap.thin_ref(place)?.capabilities()
        } else if place.is_fat_ref() {
            place.as_fat_ref().capabilities()
        } else {
            Capabilities::FULL
        };
        capabilities = capabilities.intersect(&incoming);
        if !capabilities.has(Capabilities::READ) {
            return Err(InterpreterError::CapabilityDenied {
                required: u64::from(Capabilities::READ),
                actual: u64::from(capabilities.0),
            });
        }
        if !is_cbgr_ref(&place) {
            return Ok(ReceiverPlace {
                value: resolve_arg_value(state, place)?,
                reference: place,
                capabilities,
            });
        }
        let (absolute, generation) = decode_cbgr_ref(place);
        if absolute as usize >= state.registers.top() {
            return Err(InterpreterError::InvalidOperand {
                message: "method receiver register reference is out of bounds".into(),
            });
        }
        validate_cbgr_generation(state, absolute, generation)?;
        let next = state.registers.get_absolute(absolute);
        if !is_cbgr_ref(&next) && !next.is_fat_ref()
            && reference_value_slot(state, next)?.is_none()
        {
            return Ok(ReceiverPlace { value: next, reference: place, capabilities });
        }
        place = next;
    }
    Err(InterpreterError::InvalidOperand {
        message: "method receiver reference chain is cyclic or exceeds its depth limit".into(),
    })
}

/// Resolve a RECEIVER to its object value, peeling LAYERED indirection.
///
/// `resolve_arg_value` peels exactly one hop of each encoding. A
/// receiver forwarded through nested method frames can arrive as a
/// register-ref whose target is ITSELF still an indirection —
/// `handle_deref_mut` already carries a per-site second peel "for
/// layered register-ref chains the single resolve above may still
/// surface", which is the tell that one hop is not a contract. Measured
/// casualty (T0705): `grab(&mut self) -> mid(&self) -> deep(&self)`
/// reading a reference-typed field at depth 2 got NIL where the same
/// read at depth 1 worked, and every baked `Shared` &mut-self chain
/// (get_mut, make_unique) died on exactly that — 10 of
/// base/memory/cbgr_test's failures are this class.
///
/// ONE authority, loop-until-stable with a bounded hop count (cycles
/// are a codegen bug, not a semantics; refusing beats spinning). Every
/// receiver-consuming handler (GetF, RefField, StructFieldAddr, the
/// deref family) should resolve through THIS, not through private
/// one-hop copies — the copies are how the gap keeps reopening.
/// A COMPARISON operand, resolved to the value it names (T1491).
///
/// `==` and `<` take whatever the registers hold, and two of the three
/// reference encodings arrive unpeeled: a CBGR register-ref (`&x` on a
/// local) and a heap-INTERIOR pointer (`Maybe.Some(ref l)` lowers to
/// `GetVariantDataRef`, which addresses the payload SLOT). Neither is
/// the thing being compared, and `handle_eqg` then probed the interior
/// pointer's bytes as an `ObjectHeader` to pick a type to dispatch on:
///
///     match self.last { Maybe.Some(ref l) if l == &x => continue, … }
///
/// read `type_id 1` = `Bool` out of a payload slot, looked up `Bool.eq`
/// by a linear scan over the module's functions, called the first match
/// with (interior-ptr, register-ref), and the guard answered a TUPLE —
/// `{1, None}`, the shape of `Iterator.size_hint` — which `if` then read
/// as true. `DedupIter` kept only the first element of every input, and
/// so did any user impl of the same shape in a programme that also
/// called it, because the scan's winner moves with the decode set.
///
/// NOT `resolve_receiver`, and the difference cost a suite run. That
/// function composes `resolve_arg_value`, whose `cbgr_mutable_ptrs` peel
/// is unguarded against a STALE entry — the set is never cleaned on free
/// (task #48), so an address it still holds can have been recycled into
/// a live object. `Data.Object(o1) == Data.Object(o2)` duly peeled both
/// VARIANTS to their payload `Map`s, dispatched `Map.eq` and answered
/// false where it had answered true.
///
/// So the interior hop yields only a SCALAR slot. A heap-object base
/// that a stale entry still names holds a pointer, not a scalar, and is
/// left alone; a payload slot holding an `Int` has no other reading.
/// Gating on "not an object base" was tried first and did NOT hold —
/// the recycled address is not in the heap's object index either.
///
/// One hop for the slot, because the slot holds exactly one `Value`;
/// looping would turn a compared POINTER into its first field, the
/// casualty `resolve_receiver` documents just below.
pub(in crate::interpreter) fn resolve_comparison_operand(
    state: &super::super::super::state::InterpreterState,
    val: Value,
) -> Value {
    // Register-ref chains peel to a fixpoint: a reference forwarded
    // through nested frames can name another reference.
    let mut v = val;
    let mut hops = 0u8;
    while hops < 8 && is_cbgr_ref(&v) {
        let (abs_index, _gen) = decode_cbgr_ref(v);
        let next = state.registers.get_absolute(abs_index);
        if next.bits() == v.bits() {
            break;
        }
        v = next;
        hops += 1;
    }
    if v.is_ptr() && !v.is_nil() {
        let addr = v.as_ptr::<u8>() as usize;
        if state.cbgr_mutable_ptrs.contains(&addr)
            && addr.is_multiple_of(std::mem::align_of::<Value>())
            // Bridge memory is DATA, not a cell — the same precedence
            // `resolve_arg_value` states: provenance outranks the
            // cell registry.
            && super::cbgr::bridge_extent_room(state, addr).is_none()
        {
            // SAFETY: the address is tracked as an interior reference,
            // is `Value`-aligned and belongs to no bridge extent — so it
            // addresses a live `Value` slot, the same contract
            // `handle_get_field`'s own peel relies on.
            let slot = unsafe { *(addr as *const Value) };
            // ONLY A SCALAR SLOT, and this is the guard that makes the
            // peel safe against a STALE entry. `cbgr_mutable_ptrs` is
            // never cleaned on free (task #48), so an address it still
            // holds can have been recycled into a live object — and
            // `Data.Object(o1) == Data.Object(o2)` duly peeled both
            // VARIANTS, dispatched `Map.eq` on their payloads and
            // answered false where it had answered true.
            //
            // A slot holding a POINTER is the case the comparison
            // already gets right without help: `deep_value_eq` walks it
            // structurally. A slot holding a SCALAR is the one that has
            // no other reading — an Int payload compared as an object
            // is what fabricated `Bool` out of a payload slot. So this
            // is `handle_get_field`'s own rule inverted: GetF follows
            // the slot only when it holds a pointer, because it wants
            // an object; equality peels it only when it does not.
            if !slot.is_ptr() {
                return slot;
            }
        }
    }
    v
}

pub(in crate::interpreter) fn resolve_receiver(
    state: &super::super::super::state::InterpreterState,
    val: Value,
) -> InterpreterResult<Value> {
    // REGISTER-REF chains peel to a fixpoint; the value-cell / thin-ref
    // peel then happens EXACTLY ONCE. The asymmetry is the contract, not
    // an accident: a register-ref forwarded through nested frames can
    // point at another register-ref, so that chain must be followed to
    // its end, whereas a POINTER is peeled at most once — `cbgr_alloc` +
    // `ptr_write` stores a record as one boxed Value, so one load
    // reaches the object and a SECOND load reaches its first field.
    // Looping the whole peel handed `Shared.strong_count`'s address
    // computation `Int(1)` — the value of the very field it was trying
    // to address (measured verbatim: `[fieldaddr-ok] tag=Some(1)
    // is_int=true obj=0x1 field_addr=0x19 off=0`).
    let mut v = val;
    let mut hops = 0u8;
    while hops < 8 && is_cbgr_ref(&v) {
        let (abs_index, _gen) = decode_cbgr_ref(v);
        let next = state.registers.get_absolute(abs_index);
        if next.bits() == v.bits() {
            break;
        }
        v = next;
        hops += 1;
    }
    resolve_arg_value(state, v)
}

/// Peel one `Heap<T>` CBGR cell, yielding the boxed value.
///
/// **SOLE authority** for the IMPLICIT cell peel — the one every object reader
/// performs on the way to a tag or a field. (`handle_deref`, the explicit
/// `Deref` opcode, keeps its own longer form: it validates the header's `FREED`
/// flag to surface use-after-free and records `cbgr_deref_source` for a
/// following `RefCreate`. Those are deref semantics, not a peel.)
///
/// A `Heap<T>` reaches a handler as the cell's DATA pointer — 8 bytes holding
/// the boxed `Value`, sitting `ALLOCATION_HEADER_SIZE` past an
/// `AllocationHeader` registered in `state.cbgr_allocations`. Those 8 bytes are
/// NOT an object header, so every reader that expects an object — variant tag,
/// variant payload field, record field — addresses the cell instead of the
/// object unless it peels first.
///
/// The consequence is not a fault but a silent out-of-bounds read: the cell
/// allocation is `ALLOCATION_HEADER_SIZE + 8` bytes, while a payload read
/// lands at `data + OBJECT_HEADER_SIZE + 8 + field * 8`, past its end. That is
/// how `alpha_eq(replicate(send(..)), ..)` returned `false` for structurally
/// identical terms: the tag test peeled (so the arm fired) and the payload
/// bindings did not (so the arm compared garbage).
///
/// One level only — the boxed value is the object itself, or nil, never
/// another cell. Values that are not a live cell data pointer come back
/// unchanged, so callers apply this unconditionally and keep whatever
/// `is_ptr` / `is_regular_ptr` gate they already had.
#[inline]
pub(super) fn peel_heap_cell(
    state: &super::super::super::state::InterpreterState,
    value: Value,
) -> Value {
    if !value.is_regular_ptr() || value.is_nil() {
        return value;
    }
    let base_ptr = value.as_ptr::<u8>();
    if base_ptr.is_null() {
        return value;
    }
    let header_addr = (base_ptr as usize)
        .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
    if !state.cbgr_allocations.contains(&header_addr) {
        return value;
    }
    // SAFETY: a data pointer whose header is live in `cbgr_allocations`
    // addresses the cell's inner `Value`; the cell is exactly one `Value` wide.
    unsafe { *(base_ptr as *const Value) }
}

/// Peel a variant's record payload for field access (T1571).
/// The allocation flag comes from the variant producer, never a TypeId range.
/// This also distinguishes a legacy variant and a nominal record with the
/// same numeric ID. Unit variants and scalar payloads remain unpeeled.
///
/// # Safety
/// `ptr`, if aligned and non-null, must address a live ObjectHeader from
/// the field receiver's CBGR/heap/Shared resolution. ref_or_stub rejects
/// unaligned and special-value pointers without reading them.
pub(super) unsafe fn variant_record_inner(ptr: *mut u8) -> InterpreterResult<*mut u8> {
    use super::super::super::heap;
    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
    if !header.flags.contains(heap::ObjectFlags::VARIANT) {
        return Ok(ptr);
    }
    if header.size < 8 {
        return Err(InterpreterError::Panic {
            message: "variant field receiver lacks tag/count prefix".into(),
        });
    }
    // SAFETY: the producer marker and size prove the prefix is present.
    let (_, count) = unsafe { heap::variant_header_pair(ptr) };
    let required = (count as usize)
        .checked_mul(std::mem::size_of::<Value>())
        .and_then(|bytes| bytes.checked_add(8));
    if required.is_none_or(|size| size > header.size as usize) {
        return Err(InterpreterError::Panic {
            message: "variant field receiver payload exceeds allocation".into(),
        });
    }
    if count == 0 {
        return Ok(ptr);
    }
    // SAFETY: at least one payload slot is within the checked allocation.
    let inner = unsafe { heap::variant_payload(ptr, 0) };
    if inner.is_ptr() && !inner.is_nil() {
        let inner_ptr = inner.as_ptr::<u8>();
        if inner_ptr.is_null() {
            return Err(InterpreterError::NullPointer);
        }
        Ok(inner_ptr)
    } else {
        Ok(ptr)
    }
}

/// Write `value` through whatever reference shape `slot_val` holds, mutating
/// the ORIGIN storage the reference points at. Write-side twin of
/// [`resolve_arg_value`] — the same three reference shapes, resolved for a
/// store instead of a load:
///
///   * **CBGR register-ref** → `Registers::set_absolute(abs_index, value)`.
///     A CBGR ref encodes the ABSOLUTE origin register index and is copied
///     verbatim across call frames, so a single resolution reaches the true
///     origin no matter how many wrapper frames the `&mut` passed through
///     (`safe_getsockname(addr_len: &mut UInt32) { getsockname(…, addr_len) }`
///     — the ref in `addr_len`'s slot still points at the caller's variable).
///   * **Heap-interior pointer** (tracked in `cbgr_mutable_ptrs`) → write the
///     `Value` at the interior address (`&record.field` / `&xs[i]`).
///   * **ThinRef** → write the `Value` at the ref's heap pointer.
///
/// Returns `true` if the value was written through a reference; `false` if
/// `slot_val` is not a reference (the caller should then store into the local
/// register directly — the same-frame `&mut local` case where the source
/// register already IS the origin scalar slot). A capability denial is an
/// error, so callers cannot interpret it as permission to try another store.
///
/// FFI-WRITEBACK-THRU-REF-1 (#25): the previous write-back unconditionally
/// did `set_reg(source_reg)`, which for a `&mut` OUT-param passed through a
/// wrapper frame overwrote the REFERENCE in the wrapper's parameter slot with
/// the returned scalar and never touched the caller's variable — so a C
/// OUT-param (`socklen_t*`) write was silently dropped one frame up. Following
/// the ref to its origin makes the C write land on the caller's storage.
#[inline]
pub(in super::super) fn write_through_ref(
    state: &mut super::super::super::state::InterpreterState,
    slot_val: Value,
    value: Value,
) -> InterpreterResult<bool> {
    require_reference_write(state, slot_val)?;
    if is_cbgr_ref(&slot_val) {
        let (abs_index, _gen) = decode_cbgr_ref(slot_val);
        state.registers.set_absolute(abs_index, value);
        return Ok(true);
    }
    if let Some(slot) = reference_value_slot(state, slot_val)? {
        // SAFETY: the reference carrier establishes a writable Value slot;
        // bridge data was excluded before considering interior-slot tracking.
        unsafe { *slot = value };
        return Ok(true);
    }
    Ok(false)
}

/// FFI-WRITEBACK-THRU-REF-1 (#25): normalize `&mut` scalar OUT-param args
/// that arrive as bare mutable references — a wrapper's `&mut` parameter
/// forwarded straight to an FFI call (`safe_getsockname(addr_len: &mut
/// UInt32) { getsockname(…, addr_len) }`), as opposed to an explicit
/// `&mut expr` the codegen already stripped and tagged in `source_reg_map`.
///
/// For each such argument this both:
///   * **derefs** it to the referent scalar, so the value marshalled into
///     the C call is the real number (128) — the raw CBGR-ref bits would
///     otherwise marshal as a garbage `socklen_t` input; and
///   * **registers** its source register in `source_reg_map`, so the
///     post-call write-back fires and — via [`write_through_ref`] — lands
///     the C store on the caller's origin variable one (or more) frames up.
///
/// Immutable references are deliberately left untouched: they are never
/// write-back targets, and following one would corrupt the origin with the
/// unmodified temp value. An explicit `&mut x` on a same-frame local is
/// already reduced to a plain scalar by the codegen, so it does not match
/// here and keeps its existing same-frame write-back path.
pub(in super::super) fn normalize_ffi_ref_args(
    state: &super::super::super::state::InterpreterState,
    arg_regs: &[crate::instruction::Reg],
    args: &mut [Value],
    source_reg_map: &mut std::collections::HashMap<u8, u16>,
) -> InterpreterResult<()> {
    for (i, arg_reg) in arg_regs.iter().enumerate() {
        if i >= args.len() {
            break;
        }
        let v = args[i];
        let is_mut_ref = if is_cbgr_ref(&v) {
            is_cbgr_ref_mutable(v)
        } else if v.is_ptr() && !v.is_nil() {
            state
                .cbgr_mutable_ptrs
                .contains(&(v.as_ptr::<u8>() as usize))
        } else {
            v.is_thin_ref()
        };
        if is_mut_ref {
            args[i] = resolve_arg_value(state, v)?;
            source_reg_map.entry(i as u8).or_insert(arg_reg.0);
        }
    }
    Ok(())
}

/// True iff a decoded register-ref generation matches the live slot generation.
///
/// SOLE authority for register-ref generation equality. A tagged register-ref
/// carries only the low `CBGR_REGREF_GEN_BITS` (22) of the 32-bit slot counter
/// (`registers::GEN_MAX`), so the comparison is modulo 2^22 — the 16-bit
/// `ThinRef` epoch-window precedent. Without the mask, a ref into any slot
/// whose generation has passed 2^22 would spuriously read as stale. Used by the
/// deref validator ([`validate_cbgr_generation`]) and the `is_valid` /
/// `validate_epoch` register-ref introspection intrinsics, so all three agree.
///
/// The sentinels (`CBGR_NO_CHECK_GENERATION`, `GEN_NO_CHECK` = 0x7FFE) fit in
/// 22 bits and are handled by the callers before reaching this comparison.
#[inline(always)]
pub(super) fn regref_generation_matches(ref_generation: u32, current_generation: u32) -> bool {
    (ref_generation & CBGR_REGREF_GEN_MASK) == (current_generation & CBGR_REGREF_GEN_MASK)
}

/// Validates CBGR generation and epoch for a register-based reference.
///
/// Returns `Ok(())` if the reference is valid, or `Err(Panic)` if
/// use-after-free is detected (generation or epoch mismatch).
///
/// The epoch check prevents the ABA problem where generation wraps around
/// to a previously-used value after 2^32 allocations.
#[inline(always)]
pub(super) fn validate_cbgr_generation(
    state: &super::super::super::state::InterpreterState,
    abs_index: u32,
    ref_generation: u32,
) -> InterpreterResult<()> {
    use super::super::super::registers::GEN_NO_CHECK;

    // Honour `InterpreterConfig.cbgr_enabled`: when the caller
    // has explicitly disabled CBGR validation (e.g. for a
    // max-throughput build that ran the static escape analyzer
    // upfront and proved every reference safe), the dispatch
    // loop must skip the per-deref generation check. This
    // short-circuit closes the inert-defense pattern: prior to
    // wiring, the documented opt-out had no observable effect —
    // every dereference paid the ~15 ns validation cost
    // regardless of caller intent.
    if !state.config.cbgr_enabled {
        return Ok(());
    }

    if ref_generation != CBGR_NO_CHECK_GENERATION && ref_generation != GEN_NO_CHECK {
        let current_gen = state.registers.get_generation(abs_index);
        // Modulo-2^22 comparison via the SOLE authority (see
        // `regref_generation_matches`): the tagged register-ref carries only 22
        // generation bits. Short-lived refs — the overwhelming majority —
        // capture and deref within one unchanged generation, so this is exact in
        // practice; divergence would need a slot bumped an exact multiple of
        // 2^22 times between borrow and use.
        if !regref_generation_matches(ref_generation, current_gen) {
            return Err(InterpreterError::CbgrPanic {
                failure: crate::interpreter::CbgrFailure {
                    kind: crate::interpreter::CbgrViolationKind::GenerationMismatch,
                    target: crate::interpreter::CbgrTarget::Register(abs_index),
                    expected: Some(u64::from(ref_generation)),
                    actual: Some(u64::from(current_gen)),
                },
                message: format!(
                    "CBGR use-after-free detected: expected generation {}, found {}",
                    ref_generation, current_gen
                ),
            });
        }
    }
    Ok(())
}

/// Validates epoch using window comparison to handle 16-bit truncation.
///
/// The 64-bit global epoch is truncated to 16-bit when stored in references
/// (ThinRef, FatRef). This creates a potential ABA problem where the epoch
/// wraps every 65536 operations.
///
/// Window validation solves this by checking if the reference epoch is within
/// a valid window of the current epoch. References are considered valid if:
/// 1. The truncated epochs match exactly, OR
/// 2. The reference epoch is within the valid window (accounting for wraparound)
///
/// The window size is configurable but defaults to 32768 (half of 16-bit range),
/// allowing references to remain valid for ~32K epoch advances.
#[inline(always)]
pub(super) fn validate_epoch_window(ref_epoch: u16, current_epoch: u64, window_size: u16) -> bool {
    let current_epoch_16 = (current_epoch & 0xFFFF) as u16;

    // Direct match
    if ref_epoch == current_epoch_16 {
        return true;
    }

    // Window comparison: reference must be within [current - window, current]
    // Use wrapping arithmetic to handle 16-bit overflow
    let diff = current_epoch_16.wrapping_sub(ref_epoch);
    diff <= window_size
}

#[cfg(test)]
mod tests {
    //! Authority-level tests for the T0367 tagged register-ref encoding.
    //! These run in the DEFAULT (tagged) mode — the kill-switch is off unless
    //! `VERUM_CBGR_LEGACY_INT_REFS` is set in the environment.
    use super::*;

    /// The soundness property at the authority boundary: no user `Int` — of any
    /// value or sign — is classified as a register-ref. This is the exact
    /// function (and the exact seed value) from the T0367 acceptance.
    #[test]
    fn is_cbgr_ref_rejects_user_ints() {
        assert!(!is_cbgr_ref(&Value::from_i64(-131008744790)));
        // The full collision family -(k·2^32 + m) - 1 the legacy encoding
        // decoded to a small abs-index, plus assorted user ints and both
        // inline boundaries.
        for k in 0..64i64 {
            for m in [0i64, 1, 2, 1647571, 12242261, (1 << 24) - 1] {
                let bad = -(k * (1i64 << 32) + m) - 1;
                assert!(
                    !is_cbgr_ref(&Value::from_i64(bad)),
                    "user Int {} (k={}, m={}) misclassified as a register-ref",
                    bad,
                    k,
                    m
                );
            }
        }
        for &i in &[0i64, 1, -1, i64::MIN, i64::MAX, -4296614867] {
            assert!(!is_cbgr_ref(&Value::from_i64(i)), "user Int {} misread", i);
        }
        // Other Value kinds are not register-refs either.
        assert!(!is_cbgr_ref(&Value::unit()));
        assert!(!is_cbgr_ref(&Value::from_bool(true)));
        assert!(!is_cbgr_ref(&Value::from_f64(1.5)));
    }

    /// A genuinely-encoded register-ref round-trips through the authority:
    /// encode → is_cbgr_ref true → decode → same (abs, gen); mutability
    /// preserved and strippable.
    #[test]
    fn encoded_regref_roundtrips() {
        let cases = [
            (0u32, 1u32, false),
            (7, 3, true),
            (1023, CBGR_NO_CHECK_GENERATION, false),
            (12242261, 42, true), // the T0367 abs — a *legitimate* ref here
            ((1 << 24) - 1, (1 << 22) - 1, true),
        ];
        for (abs, generation, mutable) in cases {
            let enc = if mutable {
                encode_cbgr_ref_mut(abs, generation)
            } else {
                encode_cbgr_ref(abs, generation)
            };
            assert!(is_cbgr_ref(&enc), "encoded regref not classified (abs={})", abs);
            assert!(!enc.is_int(), "a tagged regref must not be an Int");
            let (dabs, dgen) = decode_cbgr_ref(enc);
            assert_eq!(dabs, abs, "abs round-trip");
            assert_eq!(dgen, generation & ((1 << 22) - 1), "gen round-trip mod 2^22");
            assert_eq!(is_cbgr_ref_mutable(enc), mutable, "mutability round-trip");

            let shared = strip_cbgr_ref_mutability(enc);
            assert!(is_cbgr_ref(&shared));
            assert!(!is_cbgr_ref_mutable(shared), "strip must clear mutability");
            let (sabs, sgen) = decode_cbgr_ref(shared);
            assert_eq!((sabs, sgen), (abs, generation & ((1 << 22) - 1)));
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/receiver_writeback.rs"]
mod receiver_writeback;
