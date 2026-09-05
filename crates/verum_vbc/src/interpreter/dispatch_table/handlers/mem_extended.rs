//! `Opcode::MemExtended` (0xBF) — the heap/pointer family's honest
//! home.  The allocator verbs (0x00-0x06) moved here from
//! `log_extended.rs` (the file the family was misfiled in — its name
//! lied about the content, T0852); the pointer/raw/array/static-mut
//! bands migrated from `ffi_extended.rs`'s SystemSubOpcode pocket.
//! Sub-op bytes per `MemSubOpcode` (`instruction.rs`).

use super::super::DispatchResult;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::bytecode_io::*;
use super::envelope::dispatch_enveloped;
use super::super::super::heap;
use super::ffi_extended::{MAX_FFI_ALLOCATION_SIZE, value_as_addr};
use crate::instruction::Opcode;
use crate::types::TypeId;
use crate::value::Value;


/// MemExtended (0xBF) - Memory allocation operations.
///
/// Sub-opcodes:
/// - 0x00: Alloc - allocate heap memory
/// - 0x01: AllocZeroed - allocate zeroed heap memory
/// - 0x02: Dealloc - deallocate heap memory
/// - 0x03: Realloc - reallocate heap memory
/// - 0x04: Swap - swap two values in place
/// - 0x05: Replace - replace value and return old
/// - 0x06: NewByteList - allocate a `List<Byte>` with packed-byte
///   backing (1 byte/element, vs 8 for canonical `NewList`).
///   Closes red-team §4 runtime memory amplification: 10K connections
///   × 16-KiB read buffer drops from 1.28 GiB to 160 MiB.
///   Format: `[dst:reg, cap:reg]`.
pub(in super::super) fn handle_mem_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, mem_extended_body)
}

/// `MemExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition.
///
/// This family is the canonical instance of the defect the envelope exists to
/// kill, and the reason the authority is unconditional. Each arm reads the
/// register count of the **registry's** declared param shape (`AllocZeroed`
/// reads dst + size + align = 3), but a Verum-source forward declaration used
/// to bind a SUBSET of those params — `core/intrinsics/runtime/os.vr`'s
/// `__alloc_zeroed_raw(size: Int)` was annotated `@intrinsic("alloc_zeroed")`
/// with one argument while the registry declares two. Codegen then emits FEWER
/// operand bytes than the arm reads, the arm's `read_reg` overshoots into the
/// next instruction's opcode byte, and the pc stays misaligned for the rest of
/// the function: `GenerationalArena.new(N)` surfaced this as a "Null pointer
/// dereference" at a downstream `SetF` whose object register had become
/// garbage. That whole drift class is now pinned shut by
/// `declared_arities_match_the_registry` in
/// `tests/intrinsic_key_resolution_gate.rs` (CI-gated), and the `.vr`
/// declarations carry `align` for real — but the envelope stays
/// unconditional, because the gate guards the stdlib's declarations, not
/// every bytecode producer that will ever exist.
///
/// Arms may therefore read any number of bytes, in any order, and may `return`
/// early — the envelope re-establishes the instruction boundary afterwards, so
/// codegen drift can no longer leak past it.
/// The one layout rule for the whole `MemExtended` raw-memory family:
/// `size` is clamped to 1 (so `alloc(0)` returns a real, freeable
/// pointer) and `align` is used AS SENT. Alloc, dealloc and both
/// realloc legs must agree on this bit for bit — `dealloc`/`realloc`
/// with any layout other than the one the block was allocated with is
/// UB — so the rule lives once, here, instead of once per arm.
///
/// A zero or non-power-of-two `align` is refused loudly. The previous
/// shape — three arms hardcoding 8 while reading (and discarding) the
/// align register, one arm silently mapping 0 to 8 — was the
/// consumer-side twin of the producer-side arity drift documented on
/// [`mem_extended_body`]: a default that masks a caller that never set
/// the value. Every `.vr` caller now passes a real alignment, and the
/// arity gate pins that; a 0 arriving here again means a bytecode
/// producer defect, and the panic must name it, not paper over it.
fn raw_mem_layout(size: usize, align: usize, what: &str) -> InterpreterResult<std::alloc::Layout> {
    std::alloc::Layout::from_size_align(size.max(1), align).map_err(|_| InterpreterError::Panic {
        message: format!(
            "invalid {what} layout: size={size}, align={align} \
             (align must be a nonzero power of two — a 0 means the \
             bytecode producer never sent the align operand)"
        ),
    })
}

fn mem_extended_body(
    state: &mut InterpreterState,
    sub_op: u8,
) -> InterpreterResult<DispatchResult> {
    match sub_op {
        // Alloc: [dst, size, align]
        0x00 => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;
            let align = state.get_reg(align_reg).as_i64() as usize;

            let layout = raw_mem_layout(size, align, "allocation")?;
            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "allocation failed".into(),
                });
            }

            state.set_reg(dst, Value::from_ptr(ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // AllocZeroed: [dst, size, align]
        0x01 => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;
            let align = state.get_reg(align_reg).as_i64() as usize;

            let layout = raw_mem_layout(size, align, "allocation")?;
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            if ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "allocation failed".into(),
                });
            }

            state.set_reg(dst, Value::from_ptr(ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // Dealloc: [ptr, size, align]
        //
        // No `size > 0` skip: the alloc arms clamp size to 1, so a
        // block allocated as `alloc(0)` occupies real memory under
        // layout (1, align) and MUST be freed under that same layout.
        // The old guard leaked exactly those blocks.
        0x02 => {
            let ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let size = state.get_reg(size_reg).as_i64() as usize;
            let align = state.get_reg(align_reg).as_i64() as usize;

            if !ptr.is_null() {
                let layout = raw_mem_layout(size, align, "deallocation")?;
                unsafe { std::alloc::dealloc(ptr, layout) };
            }

            Ok(DispatchResult::Continue)
        }

        // Realloc: [dst, ptr, old_size, new_size, align]
        0x03 => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let old_size_reg = read_reg(state)?;
            let new_size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let old_size = state.get_reg(old_size_reg).as_i64() as usize;
            let new_size = state.get_reg(new_size_reg).as_i64() as usize;
            let align = state.get_reg(align_reg).as_i64() as usize;

            // LIST-REALLOC-CANONICAL-1: when `ptr` is an interpreter-heap
            // object, the .vr `resize_buffer`'s realloc reached a CANONICAL
            // collection backing (a NewList/ListPush array with an
            // ObjectHeader + Value/byte slots), NOT a std::alloc buffer.
            // `List.new`/`with_capacity`/`push`/`get` are intercepted onto
            // `state.heap`, but `reserve`/`resize` run the .vr body which
            // funnels through this realloc. Using std::alloc here freed the
            // heap-object pointer via `std::alloc::dealloc` (an address the
            // system allocator never returned -> heap corruption -> SIGABRT at
            // drop) and copied from the header offset (data loss). Grow via
            // `state.heap` exactly like `handle_list_push`: allocate a new
            // backing, copy the data region at +OBJECT_HEADER_SIZE, and leave
            // the old backing for the GC (never std::alloc::dealloc it).
            if !ptr.is_null() && state.heap.contains(ptr as *const heap::ObjectHeader) {
                let is_byte = {
                    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
                    header.type_id == TypeId::BYTE_LIST
                };
                let elem_size = if is_byte {
                    1usize
                } else {
                    std::mem::size_of::<Value>()
                };
                let new_cap_slots = new_size / elem_size.max(1);
                let new_backing = if is_byte {
                    state.heap.alloc(TypeId::BYTE_LIST, new_cap_slots)?
                } else {
                    state.heap.alloc_array(TypeId::UNIT, new_cap_slots)?
                };
                state.record_allocation();
                let new_ptr = new_backing.as_ptr() as *mut u8;
                let copy_bytes = old_size.min(new_size);
                if copy_bytes > 0 {
                    let old_data = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) };
                    let new_data = unsafe { new_ptr.add(heap::OBJECT_HEADER_SIZE) };
                    unsafe {
                        std::ptr::copy_nonoverlapping(old_data, new_data, copy_bytes);
                    }
                }
                state.set_reg(dst, Value::from_ptr(new_ptr as *mut ()));
                // T0429 — this early return is safe BY CONSTRUCTION, and the
                // reason is structural, not local: the pc reposition lives in
                // `dispatch_enveloped`, our CALLER, so returning from this
                // function cannot bypass it. When the correction still lived at
                // the tail of this handler, this exact `return` skipped it and
                // re-opened the desync the handler was written to close. Any
                // future fast path may return freely for the same reason —
                // just never reintroduce a pc fixup here.
                return Ok(DispatchResult::Continue);
            }

            let new_layout = raw_mem_layout(new_size, align, "reallocation")?;
            let new_ptr = unsafe { std::alloc::alloc(new_layout) };
            if new_ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "reallocation failed".into(),
                });
            }

            // Copy old data to new allocation (up to the smaller of old/new size)
            if !ptr.is_null() && old_size > 0 {
                let copy_size = old_size.min(new_size);
                unsafe { std::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size) };
                // Zero the extra bytes if growing
                if new_size > old_size {
                    unsafe {
                        std::ptr::write_bytes(new_ptr.add(old_size), 0, new_size - old_size);
                    }
                }
                // Free old allocation — same rule as the alloc arms,
                // or the layouts disagree and the free is UB.
                let old_layout = raw_mem_layout(old_size, align, "reallocation")?;
                unsafe { std::alloc::dealloc(ptr, old_layout) };
            }

            state.set_reg(dst, Value::from_ptr(new_ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // Swap: [a, b]
        //
        // The args are `&mut T` references which materialise as CBGR
        // register-refs (negative-i64-encoded `(abs_index, generation)`
        // pairs) — NOT raw pointers.  Pre-fix `as_ptr::<u64>()` on a
        // CBGR ref dereferenced the negative integer as a pointer and
        // SIGSEGV'd at runtime.  Resolve through `cbgr_helpers` so the
        // swap operates on the abs-register slots in the Value world,
        // not raw memory addresses.
        0x04 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            if is_cbgr_ref(&a_val) && is_cbgr_ref(&b_val) {
                let (a_abs, _) = decode_cbgr_ref(a_val);
                let (b_abs, _) = decode_cbgr_ref(b_val);
                let tmp = state.registers.get_absolute(a_abs);
                let b_inner = state.registers.get_absolute(b_abs);
                state.registers.set_absolute(a_abs, b_inner);
                state.registers.set_absolute(b_abs, tmp);
            } else {
                // Raw-pointer fallback (preserves the legacy path for
                // direct-pointer call sites that bypass CBGR encoding).
                let a_ptr = a_val.as_ptr::<u64>();
                let b_ptr = b_val.as_ptr::<u64>();
                if !a_ptr.is_null() && !b_ptr.is_null() {
                    unsafe { core::ptr::swap(a_ptr, b_ptr); }
                }
            }
            Ok(DispatchResult::Continue)
        }

        // Replace: [dst, dest, src]
        //
        // Same CBGR-ref handling as Swap above — `&mut T` materialises
        // as a CBGR register-ref, not a raw pointer.  Read the current
        // value out of the abs-register slot, store the new value, and
        // return the old value in `dst`.
        0x05 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            let dst = read_reg(state)?;
            let dest_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let dest_val = state.get_reg(dest_reg);
            let src_val = state.get_reg(src_reg);
            if is_cbgr_ref(&dest_val) {
                let (abs, _) = decode_cbgr_ref(dest_val);
                let old = state.registers.get_absolute(abs);
                state.registers.set_absolute(abs, src_val);
                state.set_reg(dst, old);
            } else {
                // Raw-pointer fallback (legacy).
                let dest_ptr = dest_val.as_ptr::<u64>();
                if dest_ptr.is_null() {
                    return Err(InterpreterError::Panic {
                        message: "replace: null destination pointer".into(),
                    });
                }
                let new_val_u64 = src_val.as_i64() as u64;
                let old_val = unsafe { *dest_ptr };
                unsafe { *dest_ptr = new_val_u64 };
                state.set_reg(dst, Value::from_i64(old_val as i64));
            }
            Ok(DispatchResult::Continue)
        }

        // NewByteList: [dst, cap] — allocate `List<Byte>` with packed
        // 1-byte-per-element backing (TypeId::BYTE_LIST).  Mirrors
        // `handle_new_list`'s 3-Value-header layout but tags both the
        // list and its backing with `BYTE_LIST` and sizes the backing
        // as `cap` raw bytes rather than `cap * sizeof(Value)`.
        // Closes red-team §4 runtime memory half.
        0x06 => {
            use crate::interpreter::heap::OBJECT_HEADER_SIZE;
            use crate::types::TypeId;

            let dst = read_reg(state)?;
            let cap_reg = read_reg(state)?;

            let cap_raw = state.get_reg(cap_reg).as_i64();
            let cap: usize = if cap_raw < 16 { 16 } else { cap_raw as usize };

            let backing = state.heap.alloc(TypeId::BYTE_LIST, cap)?;
            state.record_allocation();
            // Backing data is `cap` raw bytes — no per-element
            // initialisation needed (heap.alloc returns zeroed memory
            // for managed allocations; len = 0 means no slot is read).

            let list = state
                .heap
                .alloc(TypeId::BYTE_LIST, 3 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let data_ptr = unsafe {
                (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value
            };
            unsafe {
                *data_ptr = Value::from_i64(0);
                *data_ptr.add(1) = Value::from_i64(cap as i64);
                *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
            }
            state.set_reg(dst, Value::from_ptr(list.as_ptr() as *mut u8));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // T0852 Mem wave: pointer/deref leaves (0x10-0x1B), raw
        // fixed-width load/store (0x20-0x25), byte/typed arrays
        // (0x30-0x37), static-mut cells (0x40-0x41) — migrated here
        // from ffi_extended.rs (SystemSubOpcode squatters).  Byte
        // values per MemSubOpcode.
        // ================================================================
        0x10 => {
            // Read value through raw pointer
            // Format: dst:reg, ptr:reg, size:u8
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let size = read_u8(state)?;

            // Accept both Pointer-tagged and Int-tagged values as raw addresses
            let val = state.get_reg(ptr_reg);
            // #54 IS-REGULAR-PTR sweep: a FatRef/ThinRef VALUE is a
            // MARKER — `as_ptr()` on it hands back the marker payload
            // (unmapped). Deref-raw on a reference deref's its REAL
            // address (`fr.ptr()` / `tr.ptr`), exactly the raw-pointer
            // semantics the opcode contract promises.
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // SAFETY: `ptr` was null-checked above. The caller of the
            // `DerefRaw` opcode is responsible for supplying a pointer that is
            // valid for reads of `size` bytes — this mirrors Rust's raw-pointer
            // semantics and matches the AOT lowering. `read_unaligned` handles
            // arbitrary alignment.
            //

            // Extension policy (root fix for Issue #2 — continuation from
            // `handle_get_index`): `DerefRaw` is emitted by typed-array
            // reads and by any intrinsic that lowers to a raw memory read.
            // Widths 1/2/4 are *zero-extended* into the i64 NaN-box slot so
            // that a `[UInt32; N]` (or `[UInt8; N]` / `[UInt16; N]`) element
            // whose high bit is set is preserved as an unsigned value.
            // Sign-extension via `as i32 as i64` corrupted the upper 32 bits
            // of every u32 with bit 31 set and was the root cause of the
            // CRC32 divergence against zlib. Callers that actually need
            // signed semantics for a raw read can truncate (`as i32` etc.)
            // at the use site — zero-extension is the invariant-preserving
            // default for unsigned raw I/O, which is the vastly common case.
            let value = unsafe {
                match size {
                    1 => *ptr as i64,                                        // u8 → i64 (zero-extend)
                    2 => std::ptr::read_unaligned(ptr as *const u16) as i64, // u16 → i64
                    4 => std::ptr::read_unaligned(ptr as *const u32) as i64, // u32 → i64
                    8 => std::ptr::read_unaligned(ptr as *const i64), // 8 bytes fill the slot
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid deref size: {}", size),
                        });
                    }
                }
            };
            state.set_reg(dst, Value::from_i64(value));
            Ok(DispatchResult::Continue)
        }

        0x11 => {
            // Write value through raw pointer
            // Format: ptr:reg, value:reg, size:u8
            let ptr_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let size = read_u8(state)?;

            // ONE raw-address decode ladder — the same forms DerefRaw and
            // the volatile twins accept (fat/thin/regular-ptr AND the
            // int-tagged address a ptr_offset chain produces). The former
            // int-reject was asymmetric theater: the read side and both
            // volatile ops already accepted ints, raw writes are gated by
            // `unsafe` at the language level, and the reject broke every
            // ptr_write through pointer arithmetic (T0108).
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // Extract the raw bits of the Value to write — for sub-word
            // sizes we want the inline-int payload (low bits), but for
            // an 8-byte write we want the FULL 64-bit Value bit-pattern
            // including any NaN-box tag header so that Pointer-tagged
            // values (e.g. heap-allocated Variant Some(v) writes via
            // `*ptr = Some(value)` in user code) round-trip correctly.
            //

            // The previous impl always called `.as_i64()`, which debug-
            // asserts the value is Int-tagged and therefore panicked
            // with "Expected int, got Some(0)" for any pointer-tagged
            // write — discovered while validating task #40.
            let val_value = state.get_reg(value_reg);
            // SAFETY: `ptr` was null-checked above AND rejected if it was not a
            // pointer-tagged Value (guards against arbitrary integer-to-pointer
            // writes). The caller is responsible for ensuring the target is
            // writable for `size` bytes per the FFI contract. `write_unaligned`
            // handles arbitrary alignment.
            unsafe {
                match size {
                    1 | 2 | 4 => {
                        // Sub-word writes use the inline-int payload.
                        // For sub-word atomic stores on Verum struct
                        // fields the payload's low bytes ARE the user-
                        // visible value; this matches handle_atomic_store.
                        let value = if val_value.is_int() {
                            val_value.as_i64()
                        } else {
                            // Pointer-tagged Value at sub-word size is
                            // an unusual case (typically the user is
                            // writing a NaN-boxed pointer to a non-aligned
                            // slot) — fall back to the raw bits.
                            val_value.bits() as i64
                        };
                        match size {
                            1 => *ptr = value as u8,
                            2 => std::ptr::write_unaligned(ptr as *mut i16, value as i16),
                            4 => std::ptr::write_unaligned(ptr as *mut i32, value as i32),
                            _ => unreachable!(),
                        }
                    }
                    8 => {
                        // Write the FULL 8-byte Value bit-pattern so
                        // Pointer-tagged writes (heap variant Some(v),
                        // boxed integers, ThinRef indices, etc.) survive
                        // round-trip through the raw-pointer storage.
                        std::ptr::write_unaligned(ptr as *mut u64, val_value.bits());
                    }
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid deref size: {}", size),
                        });
                    }
                }
            }
            Ok(DispatchResult::Continue)
        }

        0x12 => {
            // Read pointer through raw pointer (for pointer-to-pointer)
            // Format: dst:reg, ptr:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<*mut u8>();
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // SAFETY: `ptr` is non-null (checked above) and is a pointer-to-pointer
            // produced by the caller. Aligned reads of a `*mut u8` are sound so
            // long as the target holds a valid pointer bit-pattern, per the FFI
            // contract for `DerefRawPtr`.
            let value_ptr = unsafe { *ptr };
            state.set_reg(dst, Value::from_ptr(value_ptr));
            Ok(DispatchResult::Continue)
        }

        0x13 => {
            // Pointer arithmetic: add offset
            // Format: dst:reg, ptr:reg, offset:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let offset_reg = read_reg(state)?;

            let addr = value_as_addr(state.get_reg(ptr_reg));
            let offset = state.get_reg(offset_reg).as_i64();

            // Element-scaled: ptr_offset/ptr_add index by ELEMENT, not byte.
            // Every List/slice backing slot is an 8-byte NaN-boxed Value, so
            // scale the element count to a byte offset (matches the AOT i64 GEP
            // and the registry contract "ptr + count * 8"). A raw byte add here
            // under-advanced by 8x — ptr_offset(p, 1) read mid-slot garbage.
            let byte_offset = offset.checked_mul(8).ok_or(InterpreterError::IntegerOverflow {
                operation: "PtrAdd",
            })?;

            // SECURITY: `ptr.add(offset)` uses wrapping arithmetic on the raw
            // address, which can wrap around the address space when `offset`
            // is attacker-controlled, producing an arbitrary pointer. Use
            // checked arithmetic on the address bits and fail on overflow.
            let new_addr = if byte_offset >= 0 {
                addr.checked_add(byte_offset as usize)
            } else {
                // negative; subtract its absolute value from addr
                let abs = (byte_offset as i128).unsigned_abs() as usize;
                addr.checked_sub(abs)
            };
            let new_addr = new_addr.ok_or(InterpreterError::IntegerOverflow {
                operation: "PtrAdd",
            })?;
            // Int-tagged address — same rationale as the as_ptr intercept:
            // a pointer-tagged interior address becomes a droppable-looking
            // heap object and DropRef chases element bytes as a header.
            state.set_reg(dst, Value::from_i64(new_addr as i64));
            Ok(DispatchResult::Continue)
        }

        0x14 => {
            // Pointer arithmetic: subtract offset
            // Format: dst:reg, ptr:reg, offset:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let offset_reg = read_reg(state)?;

            let addr = value_as_addr(state.get_reg(ptr_reg));
            let offset = state.get_reg(offset_reg).as_i64();

            // Element-scaled (see PtrAdd): scale the element count to a byte
            // offset over 8-byte NaN-boxed slots.
            let byte_offset = offset.checked_mul(8).ok_or(InterpreterError::IntegerOverflow {
                operation: "PtrSub",
            })?;

            // SECURITY: raw pointer `.sub`/`.add` wrap around the address
            // space with an attacker-controlled offset. Perform checked
            // arithmetic on the integer address and fail on overflow.
            let new_addr = if byte_offset >= 0 {
                addr.checked_sub(byte_offset as usize)
            } else {
                let abs = (byte_offset as i128).unsigned_abs() as usize;
                addr.checked_add(abs)
            };
            let new_addr = new_addr.ok_or(InterpreterError::IntegerOverflow {
                operation: "PtrSub",
            })?;
            // Int-tagged address — same rationale as the as_ptr intercept:
            // a pointer-tagged interior address becomes a droppable-looking
            // heap object and DropRef chases element bytes as a header.
            state.set_reg(dst, Value::from_i64(new_addr as i64));
            Ok(DispatchResult::Continue)
        }

        0x15 => {
            // Pointer difference: compute distance in bytes
            // Format: dst:reg, ptr1:reg, ptr2:reg
            let dst = read_reg(state)?;
            let ptr1_reg = read_reg(state)?;
            let ptr2_reg = read_reg(state)?;

            let ptr1 = value_as_addr(state.get_reg(ptr1_reg));
            let ptr2 = value_as_addr(state.get_reg(ptr2_reg));

            let diff = (ptr1 as isize) - (ptr2 as isize);
            state.set_reg(dst, Value::from_i64(diff as i64));
            Ok(DispatchResult::Continue)
        }

        0x16 => {
            // Check if pointer is null
            // Format: dst:reg, ptr:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;

            let addr = value_as_addr(state.get_reg(ptr_reg));
            let is_null = addr == 0;
            state.set_reg(dst, Value::from_bool(is_null));
            Ok(DispatchResult::Continue)
        }

        0x17 => {
            // Sign-extending sibling of `DerefRaw`. Format and layout
            // identical (`dst:reg, ptr:reg, size:u8`) — only the
            // extension policy differs: we read `size` bytes through
            // the pointer and **sign-extend** the result to i64.
            //

            // This is the load-bearing read for typed signed C
            // primitives at FFI boundaries — `int8_t` / `int16_t` /
            // `int32_t` slots whose high bit can flip the sign of the
            // i64 representation. errno is the canonical user (a
            // positive errno fits either policy, but the engine should
            // not depend on errno staying non-negative — kernel APIs
            // that return signed i32 results in pointer-deref form
            // need this sign-fidelity).
            //

            // See `DerefRaw`'s comment block above for the historical
            // CRC32 zero-extension rationale that motivated keeping the
            // two opcodes separate.
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let size = read_u8(state)?;

            let val = state.get_reg(ptr_reg);
            // #54 IS-REGULAR-PTR sweep: a FatRef/ThinRef VALUE is a
            // MARKER — `as_ptr()` on it hands back the marker payload
            // (unmapped). Deref-raw on a reference deref's its REAL
            // address (`fr.ptr()` / `tr.ptr`), exactly the raw-pointer
            // semantics the opcode contract promises.
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // SAFETY: `ptr` was null-checked above. The caller is
            // responsible for the pointer being valid for reads of
            // `size` bytes per the FFI contract. `read_unaligned`
            // handles arbitrary alignment.
            let value = unsafe {
                match size {
                    1 => *(ptr as *const i8) as i64, // i8 → i64 (sign-extend)
                    2 => std::ptr::read_unaligned(ptr as *const i16) as i64, // i16 → i64 (sign-extend)
                    4 => std::ptr::read_unaligned(ptr as *const i32) as i64, // i32 → i64 (sign-extend)
                    8 => std::ptr::read_unaligned(ptr as *const i64), // 8 bytes fill the slot
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid signed deref size: {}", size),
                        });
                    }
                }
            };
            state.set_reg(dst, Value::from_i64(value));
            Ok(DispatchResult::Continue)
        }

        0x18 => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let size = read_u8(state)?;
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            // SAFETY: null-checked; caller supplies a pointer valid for
            // `size`-byte reads (raw-pointer contract, mirrors DerefRaw).
            let value = unsafe {
                match size {
                    1 => std::ptr::read_volatile(ptr) as i64,
                    2 => std::ptr::read_volatile(ptr as *const u16) as i64,
                    4 => std::ptr::read_volatile(ptr as *const u32) as i64,
                    8 => std::ptr::read_volatile(ptr as *const i64),
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid volatile read size: {}", size),
                        });
                    }
                }
            };
            state.set_reg(dst, Value::from_i64(value));
            Ok(DispatchResult::Continue)
        }

        0x19 => {
            let ptr_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let size = read_u8(state)?;
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            let v = state.get_reg(value_reg).as_i64();
            // SAFETY: null-checked; caller supplies a pointer valid for
            // `size`-byte writes (raw-pointer contract, mirrors DerefMutRaw).
            unsafe {
                match size {
                    1 => std::ptr::write_volatile(ptr, v as u8),
                    2 => std::ptr::write_volatile(ptr as *mut u16, v as u16),
                    4 => std::ptr::write_volatile(ptr as *mut u32, v as u32),
                    8 => std::ptr::write_volatile(ptr as *mut i64, v),
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid volatile write size: {}", size),
                        });
                    }
                }
            }
            Ok(DispatchResult::Continue)
        }

        0x1A => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let size = read_u8(state)?;
            // **FLAT-RECORD-RW-1 (T1160)** — a width other than 1/2/4/8
            // is a MULTI-FIELD RECORD read, and the emission that
            // produced it (`try_compile_flat_record_rw`) appended the
            // type id so the payload can be rebuilt into an object.
            // Reading the extra operand is gated on the width, so every
            // existing 8-byte instruction — including all baked stdlib
            // bytecode — decodes exactly as before.
            let record_type_id: Option<crate::types::TypeId> =
                if matches!(size, 1 | 2 | 4 | 8) {
                    None
                } else {
                    Some(crate::types::TypeId(read_u32(state)?))
                };
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            // SAFETY: null-checked; caller supplies a pointer valid for
            // `size`-byte reads (raw-pointer contract, mirrors DerefRaw).
            // A record read rebuilds the object from the flat payload the
            // store side laid down — the twin the tree was missing
            // (T1160). Bounded by the extent's room, exactly like
            // `bridge_flat_store`.
            if let Some(tid) = record_type_id {
                let room = super::cbgr::bridge_extent_room(state, ptr as usize)
                    .unwrap_or(size as usize);
                let v = super::cbgr::bridge_flat_load(
                    state,
                    ptr as *const u8,
                    room,
                    size as usize,
                    tid,
                    "ptr_read",
                )?;
                state.set_reg(dst, v);
                return Ok(DispatchResult::Continue);
            }
            let value = unsafe {
                match size {
                    1 => *ptr as i64,
                    2 => std::ptr::read_unaligned(ptr as *const u16) as i64,
                    4 => std::ptr::read_unaligned(ptr as *const u32) as i64,
                    8 => std::ptr::read_unaligned(ptr as *const i64),
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid ptr_read size: {}", size),
                        });
                    }
                }
            };
            state.set_reg(dst, Value::from_i64(value));
            Ok(DispatchResult::Continue)
        }
        0x1B => {
            let ptr_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let size = read_u8(state)?;
            // Symmetric with 0x1A (T1160): a non-1/2/4/8 width means the
            // emission appended a type id. The write side does not need
            // it — `bridge_flat_store` takes the shape from the VALUE's
            // own header — but the operand must be CONSUMED or every
            // instruction after this one decodes at the wrong offset.
            if !matches!(size, 1 | 2 | 4 | 8) {
                let _record_type_id = read_u32(state)?;
            }
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_fat_ref() {
                val.as_fat_ref().ptr()
            } else if val.is_thin_ref() {
                val.as_thin_ref().ptr
            } else if val.is_regular_ptr() {
                val.as_ptr()
            } else if val.is_int() {
                // A raw address fabricated as an Int and cast — the
                // documented minority case.
                val.as_i64() as *mut u8
            } else {
                // Anything else has no pointer reading. `as_i64()` on a
                // Bool or a variant decodes the NaN box's payload bits as
                // an address: `debug_assert!(self.is_int())` in a debug
                // build, a wild pointer in release. Null is the honest
                // answer, and every caller here already rejects null on
                // the next line.
                //
                // The correct shape was already in the tree —
                // `handlers/cbgr.rs:1718` guards with `is_int()` and
                // falls to `null_mut()`. Paired implementations are an
                // oracle: when one copy of a pattern is right, the
                // question is why the others differ.
                std::ptr::null_mut()
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            let v = state.get_reg(value_reg);
            // **T0705 — a RECORD into a BRIDGE block goes in FLAT.**
            // The machine form of a heap value is its POINTER, and
            // storing that is right for a cell that holds a reference.
            // A bridge allocation is not such a cell: `cbgr_alloc(T.size)`
            // + `ptr_write(p, record)` asks for the record's BYTES, and
            // that is what `memcpy` / `load_byte` / AOT code and the
            // read side (`(*p).field` -> StructFieldAddr) all expect.
            // Storing the pointer left the block holding one word that
            // the flat read walks straight past — `Shared.new` builds
            // exactly this way, which is why every baked `Shared` read
            // garbage.
            //
            // Gate: the destination must lie in a live bridge extent
            // (provenance, not the tag) AND the block must have room for
            // the WHOLE value — a block sized for a reference keeps the
            // pointer write.
            if crate::interpreter::env_flags::is_set(crate::interpreter::env_flags::Flag::TracePtrwrite) {
                let room = super::cbgr::bridge_extent_room(state, ptr as usize);
                let hsize = if v.is_regular_ptr() && !v.is_nil() {
                    unsafe {
                        super::super::super::heap::ObjectHeader::try_from_ptr(v.as_ptr::<u8>())
                    }
                    .map(|h| h.size)
                } else {
                    None
                };
                eprintln!(
                    "[ptrwrite] raw={:#x} raw_tag={:?} dst={:p} size_op={} val_tag={:?} regular_ptr={} room={:?} \
                     hdr_size={:?}",
                    val.to_bits(),
                    val.tag(),
                    ptr,
                    size,
                    v.tag(),
                    v.is_regular_ptr(),
                    room,
                    hsize
                );
            }
            if v.is_regular_ptr()
                && !v.is_nil()
                && let Some(room) = super::cbgr::bridge_extent_room(state, ptr as usize)
                && let Some(header) =
                    (unsafe { super::super::super::heap::ObjectHeader::try_from_ptr(v.as_ptr::<u8>()) })
                && room >= header.size as usize
            {
                return super::cbgr::bridge_flat_store(state, ptr, room, v, "ptr_write");
            }
            // MACHINE representation: the payload (as_i64 for ints,
            // bit-pattern for floats via to_f64 bits, raw ptr for heap
            // values) — an `*mut Int` cell holds 11, never the NaN box.
            let payload: i64 = if v.is_float() {
                v.as_f64().to_bits() as i64
            } else if v.is_regular_ptr() {
                v.as_ptr::<u8>() as i64
            } else if v.is_bool() {
                // A `*mut Bool` cell holds 0 or 1, and `as_i64` refuses
                // a boolean — `is_int()` does not include TAG_BOOLEAN.
                // `let y: Bool = true;` in a `static mut` reached the
                // catch-all below and tripped
                // `debug_assert!(self.is_int(), "Expected int, got {:?}")`
                // with tag 2 (TAG_BOOLEAN).
                i64::from(v.as_bool())
            } else if v.is_int() {
                v.as_i64()
            } else {
                // Everything else — variants and any other value whose
                // machine form IS its NaN box — is written verbatim.
                //
                // This arm used to be an unconditional `as_i64()`, on
                // the assumption that "not float, not pointer" means
                // integer. It is a CLOSED set treated as open with a
                // default, and the default decodes a box's payload bits
                // as if they were a number.
                //
                // In a debug build the assert fires — which is how this
                // was found, through
                // `verum_compiler/tests/core_stdlib_validation_test.rs`,
                // a suite that had never run in CI. In a release build
                // there is no assert: the write silently stores garbage
                // and the next read hands it back with the original
                // type's tag.
                //
                // A variant has no unboxed machine form, so the box IS
                // the representation and storing it verbatim is the only
                // value that round-trips.
                v.to_bits() as i64
            };
            // SAFETY: null-checked; caller supplies a pointer valid for
            // `size`-byte writes (raw-pointer contract).
                        if crate::interpreter::env_flags::is_set(crate::interpreter::env_flags::Flag::TraceStaticmut) {
                eprintln!("[staticmut-trace] 0x6C write ptr={:p} payload={} size={}", ptr, payload, size);
            }
unsafe {
                match size {
                    1 => *ptr = payload as u8,
                    2 => std::ptr::write_unaligned(ptr as *mut u16, payload as u16),
                    4 => std::ptr::write_unaligned(ptr as *mut u32, payload as u32),
                    8 => std::ptr::write_unaligned(ptr as *mut i64, payload),
                    _ => {
                        return Err(InterpreterError::InvalidOperand {
                            message: format!("invalid ptr_write size: {}", size),
                        });
                    }
                }
            }
            Ok(DispatchResult::Continue)
        }

        0x1C | 0x1D => {
            // PtrAddT / PtrSubT — width-aware pointer arithmetic
            // (T0108 byte-stride law): `ptr ± count × width`.
            // Format: dst, ptr, count, width:imm-u8.
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let count_reg = read_reg(state)?;
            let width = read_u8(state)? as i64;
            let ptr = state.get_reg(ptr_reg).as_integer_compatible();
            let count = state.get_reg(count_reg).as_integer_compatible();
            let delta = count.wrapping_mul(width);
            let out = if sub_op == 0x1C {
                ptr.wrapping_add(delta)
            } else {
                ptr.wrapping_sub(delta)
            };
            state.set_reg(dst, Value::from_i64(out));
            Ok(DispatchResult::Continue)
        }

        0x20 => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let addr = state.get_reg(addr_reg).as_integer_compatible();
            let v = if addr > 0 {
                // SAFETY: caller-supplied live address (allocation-harness
                // discipline); byte read.
                unsafe { *(addr as *const u8) as i64 }
            } else {
                0
            };
            state.set_reg(dst, Value::from_i64(v));
            Ok(DispatchResult::Continue)
        }
        0x21 => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let addr = state.get_reg(addr_reg).as_integer_compatible();
            let v = state.get_reg(val_reg).as_integer_compatible() as u8;
            if addr > 0 {
                // SAFETY: as above.
                unsafe { *(addr as *mut u8) = v };
            }
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }
        0x22 => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let addr = state.get_reg(addr_reg).as_integer_compatible();
            let v = if addr > 0 {
                // SAFETY: as above; sign-extended per the C `int` contract.
                unsafe { *(addr as *const i32) as i64 }
            } else {
                0
            };
            state.set_reg(dst, Value::from_i64(v));
            Ok(DispatchResult::Continue)
        }
        0x23 => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let addr = state.get_reg(addr_reg).as_integer_compatible();
            let v = state.get_reg(val_reg).as_integer_compatible() as i32;
            if addr > 0 {
                // SAFETY: as above.
                unsafe { *(addr as *mut i32) = v };
            }
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }
        0x24 => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let addr = state.get_reg(addr_reg).as_integer_compatible();
            let v = if addr > 0 {
                // SAFETY: as above.
                unsafe { *(addr as *const i64) }
            } else {
                0
            };
            state.set_reg(dst, Value::from_i64(v));
            Ok(DispatchResult::Continue)
        }
        0x25 => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let addr = state.get_reg(addr_reg).as_integer_compatible();
            let v = state.get_reg(val_reg).as_integer_compatible();
            if addr > 0 {
                // SAFETY: as above.
                unsafe { *(addr as *mut i64) = v };
            }
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        0x30 => {
            // Allocate a byte array (contiguous bytes, not Values)
            // Format: dst:reg, size:reg, init:reg
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let init_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;
            let init_byte = state.get_reg(init_reg).as_i64() as u8;

            // Allocate with TypeId::U8 to mark this as a byte array
            let obj = state.heap.alloc(TypeId::U8, size)?;
            state.record_allocation();

            // Initialize all bytes
            // SAFETY: `obj.as_ptr()` was just returned from `state.heap.alloc()`,
            // which yields a non-null pointer with `OBJECT_HEADER_SIZE + size`
            // bytes of valid storage. Skipping past the header lands on the
            // first data byte of the allocation, which stays valid until the
            // object is freed.
            let data_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(super::super::super::heap::OBJECT_HEADER_SIZE)
            };
            // SAFETY: `data_ptr` points to `size` bytes of uninitialized storage
            // inside the freshly allocated object. Writing `size` bytes stays in
            // bounds and leaves every byte initialized to `init_byte`.
            unsafe {
                std::ptr::write_bytes(data_ptr, init_byte, size);
            }

            // Return header pointer so GetE/SetE work correctly with byte arrays
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            Ok(DispatchResult::Continue)
        }

        0x31 => {
            // Get address of element in byte array (for &mut buf[idx] as *mut Byte)
            // Format: dst:reg, arr:reg, idx:reg
            // Returns: dst = arr_ptr + OBJECT_HEADER_SIZE + idx
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();

            // Verify this is a byte array
            // SAFETY: `arr_ptr` was null-checked above. Every pointer tagged via
            // `Value::from_ptr` that survives the heap's lifetime starts with an
            // `ObjectHeader`, so the cast is layout-compatible. The reference is
            // short-lived and does not outlive this dispatch call.
            let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id != TypeId::U8 {
                return Err(InterpreterError::TypeMismatch {
                    expected: "byte array (TypeId::U8)",
                    got: "non-byte-array",
                    operation: "ByteArrayElementAddr",
                });
            }

            // Bounds check
            let array_size = header.size as usize;
            if idx < 0 || idx as usize >= array_size {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_size,
                });
            }

            // Compute element address: arr_ptr + OBJECT_HEADER_SIZE + idx
            // SAFETY: `idx` was bounds-checked against `array_size`, so the
            // offset cannot exceed the allocation. The resulting pointer lies
            // within the live byte-array allocation.
            let elem_addr = unsafe {
                arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + idx as usize)
            };
            state.set_reg(dst, Value::from_ptr(elem_addr));
            Ok(DispatchResult::Continue)
        }

        0x32 => {
            // Load a byte from byte array
            // Format: dst:reg, arr:reg, idx:reg
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();

            // Verify this is a byte array
            // SAFETY: `arr_ptr` is non-null (checked above) and points to a live
            // heap object whose layout begins with `ObjectHeader`. The borrow
            // does not escape this handler.
            let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id != TypeId::U8 {
                return Err(InterpreterError::TypeMismatch {
                    expected: "byte array (TypeId::U8)",
                    got: "non-byte-array",
                    operation: "ByteArrayLoad",
                });
            }

            // Bounds check
            let array_size = header.size as usize;
            if idx < 0 || idx as usize >= array_size {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_size,
                });
            }

            // Load byte value
            // SAFETY: `idx` was bounds-checked against `array_size`, so the
            // computed byte address lies within the live allocation. The byte
            // is always initialized (arrays are zeroed or written before load).
            let byte_val = unsafe {
                *arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + idx as usize)
            };
            state.set_reg(dst, Value::from_i64(byte_val as i64));
            Ok(DispatchResult::Continue)
        }

        0x33 => {
            // Store a byte to byte array
            // Format: arr:reg, idx:reg, val:reg
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();
            let val = state.get_reg(val_reg).as_i64() as u8;

            // Verify this is a byte array
            // SAFETY: `arr_ptr` is non-null (checked above) and is a live heap
            // object whose layout begins with `ObjectHeader`. The reference
            // is dropped before any mutation.
            let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id != TypeId::U8 {
                return Err(InterpreterError::TypeMismatch {
                    expected: "byte array (TypeId::U8)",
                    got: "non-byte-array",
                    operation: "ByteArrayStore",
                });
            }

            // Bounds check
            let array_size = header.size as usize;
            if idx < 0 || idx as usize >= array_size {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_size,
                });
            }

            // Store byte value
            // SAFETY: `idx` was bounds-checked against `array_size`, so the
            // computed address is within the live allocation. There are no
            // outstanding references to this byte (the `header` borrow above
            // is dropped by now).
            unsafe {
                *arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + idx as usize) = val;
            }
            Ok(DispatchResult::Continue)
        }

        0x34 => {
            // Create new typed array with specified element size
            // Format: dst:reg, count:reg, elem_size:u8, init:reg
            let dst = read_reg(state)?;
            let count_reg = read_reg(state)?;
            // TYPED-ARRAY-FLOAT-1 (#28): bit 0x80 of the elem_size byte marks
            // a floating-point element array. The low 7 bits are the raw byte
            // stride (4 or 8); the flag routes the allocation to the float
            // heap TypeId (F64 / F32) so `handle_get_index` / `handle_set_index`
            // decode the raw IEEE-754 bytes through `from_f64` rather than
            // `from_i64` (which re-boxes a double's bit pattern as a bogus
            // integer — the `[Float; N]` → NaN symptom).
            let elem_size_byte = read_u8(state)?;
            let is_float = (elem_size_byte & 0x80) != 0;
            let elem_size = (elem_size_byte & 0x7f) as usize;
            let init_reg = read_reg(state)?;

            let count_val = state.get_reg(count_reg).as_i64();
            if count_val < 0 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!(
                        "NewTypedArray: expected non-negative count, got {}",
                        count_val
                    ),
                });
            }
            let count = count_val as usize;
            let init_value = state.get_reg(init_reg).as_i64();

            // Total size = count * elem_size (checked to prevent overflow)
            let total_size = count.checked_mul(elem_size).ok_or({
                InterpreterError::OutOfMemory {
                    requested: usize::MAX,
                    available: MAX_FFI_ALLOCATION_SIZE,
                }
            })?;
            if total_size > MAX_FFI_ALLOCATION_SIZE {
                return Err(InterpreterError::OutOfMemory {
                    requested: total_size,
                    available: MAX_FFI_ALLOCATION_SIZE,
                });
            }

            // Allocate array (using TypeId based on element size + float-ness).
            // Float arrays reuse the scalar float TypeIds (F64 == FLOAT == 3,
            // F32 == 13) exactly as integer arrays reuse the scalar int TypeIds
            // (U8/U16/U32/U64). The heap free path is a raw buffer dealloc for
            // every non-record type, so a float-typed packed array drops
            // identically to an integer one.
            let type_id = if is_float {
                match elem_size {
                    8 => TypeId::F64,
                    4 => TypeId::F32,
                    _ => TypeId::U8, // no narrower float width exists
                }
            } else {
                match elem_size {
                    1 => TypeId::U8,
                    2 => TypeId::U16,
                    4 => TypeId::U32,
                    8 => TypeId::U64,
                    _ => TypeId::U8, // Default to byte array for unknown sizes
                }
            };

            // Allocate using heap.alloc which returns an Object
            let obj = state.heap.alloc(type_id, total_size)?;
            state.record_allocation();

            // Get raw pointer for initialization
            let ptr = obj.as_ptr() as *mut u8;

            // Initialize elements
            // SAFETY: `ptr` was just returned from `state.heap.alloc(type_id,
            // total_size)` and has `OBJECT_HEADER_SIZE + total_size` bytes of
            // valid storage. `total_size = count * elem_size` and was checked
            // for overflow above. All subsequent writes iterate only `count`
            // elements of `elem_size` each, so they stay in bounds.
            unsafe {
                let data_ptr = ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE);
                match elem_size {
                    1 => {
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u8;
                        }
                    }
                    2 => {
                        let data_ptr = data_ptr as *mut u16;
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u16;
                        }
                    }
                    4 => {
                        let data_ptr = data_ptr as *mut u32;
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u32;
                        }
                    }
                    8 => {
                        let data_ptr = data_ptr as *mut u64;
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u64;
                        }
                    }
                    _ => {
                        // Byte-wise initialization for unknown sizes
                        for i in 0..total_size {
                            *data_ptr.add(i) = 0;
                        }
                    }
                }
            }

            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        0x35 => {
            // Get element address for typed array with specified element size
            // Format: dst:reg, arr:reg, idx:reg, elem_size:u8
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;
            let elem_size = read_u8(state)? as usize;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();
            if idx < 0 {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: 0,
                });
            }

            // Compute element address: base + header + (idx * elem_size) with overflow checks
            let idx_usize = idx as usize;
            let offset = idx_usize.checked_mul(elem_size).ok_or({
                InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: 0,
                }
            })?;
            let total_offset = super::super::super::heap::OBJECT_HEADER_SIZE
                .checked_add(offset)
                .ok_or({
                    InterpreterError::IndexOutOfBounds {
                        index: idx,
                        length: 0,
                    }
                })?;
            // SAFETY: `total_offset` was produced by checked arithmetic, so it
            // cannot wrap. `arr_ptr` is non-null and points to a live typed
            // array. The caller (Verum emitter) is responsible for ensuring
            // `idx * elem_size` stays within the allocation — this path mirrors
            // the AOT lowering, which applies the same contract.
            let elem_addr = unsafe { arr_ptr.add(total_offset) };

            state.set_reg(dst, Value::from_ptr(elem_addr));
            Ok(DispatchResult::Continue)
        }

        0x36 => {
            // Load one element from a packed typed array, DECODING by width.
            // Format: dst:reg, arr:reg, idx:reg, elem_size:u8 (bit 0x80 = float).
            // The read twin of `TypedArrayStore` and the typed analogue of
            // `ByteArrayLoad` (elem_size == 1): integer widths zero-extend, F32
            // widens `from_bits` → f64, F64 round-trips its IEEE bits. Reuses
            // the ONE decode authority `heap::typed_array_element` so this path
            // and the `GetE` typed-array branch stay bit-identical.
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;
            let elem_byte = read_u8(state)?;
            let elem_size = (elem_byte & 0x7F) as usize;
            let is_float = elem_byte & 0x80 != 0;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            let idx = state.get_reg(idx_reg).as_i64();
            if idx < 0 {
                return Err(InterpreterError::IndexOutOfBounds { index: idx, length: 0 });
            }
            // Bounds-check against `header.size` (BYTES), mirroring the store.
            let array_bytes = {
                // SAFETY: `arr_ptr` non-null (checked) and begins with an
                // `ObjectHeader`; the borrow does not escape this block.
                let header = unsafe {
                    &*(arr_ptr as *const super::super::super::heap::ObjectHeader)
                };
                header.size as usize
            };
            let elem_stride = elem_size.max(1);
            let offset = (idx as usize).checked_mul(elem_size).ok_or({
                InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_bytes / elem_stride,
                }
            })?;
            if offset.checked_add(elem_size).map_or(true, |end| end > array_bytes) {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_bytes / elem_stride,
                });
            }
            // Map (elem_size, is_float) → the packed-scalar TypeId, then decode
            // through the shared authority (keys `typed_array_element_spec`).
            let tid = match (elem_size, is_float) {
                (1, false) => TypeId::U8,
                (2, false) => TypeId::U16,
                (4, false) => TypeId::U32,
                (8, false) => TypeId::U64,
                (4, true) => TypeId::F32,
                (8, true) => TypeId::F64,
                // Widths outside {1,2,4,8}: fall back to a raw i64 read.
                _ => TypeId::U64,
            };
            // SAFETY: bounds-checked above; the data area starts at
            // `HEADER` and `typed_array_element` reads `idx*stride` within it.
            let data_ptr =
                unsafe { arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) };
            let value = unsafe {
                super::super::super::heap::typed_array_element(tid, data_ptr, idx as usize)
            }
            .unwrap_or_else(|| Value::from_i64(0));
            state.set_reg(dst, value);
            Ok(DispatchResult::Continue)
        }

        0x37 => {
            // Store one element into a packed typed array, UNBOXING the value.
            // Format: arr:reg, idx:reg, val:reg, elem_size:u8
            //
            // The typed-array twin of `ByteArrayStore` (elem_size == 1) and of
            // `NewTypedArray`'s raw fill: a `[Int; N]` element is a raw
            // `elem_size` integer, NOT a NaN-boxed `Value`. The prior init /
            // assignment path (`TypedArrayElementAddr` + `DerefMutRaw`) wrote
            // the FULL 64-bit `Value` bits (deliberate for `*ptr = Some(v)`
            // pointer round-trips, task #40), so the load — which reads the raw
            // `elem_size` bytes (`GetE` / `SliceGet reserved=elem_size` →
            // `from_i64`) — read back the NaN-box tag pattern as garbage
            // (`a[0]` of `[10,…]` → `0x7FF9…000A`). Storing raw makes store and
            // load coherent.
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let elem_size = read_u8(state)? as usize;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            let idx = state.get_reg(idx_reg).as_i64();
            if idx < 0 {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: 0,
                });
            }
            // Extract the raw element payload WITHOUT assuming Int — an
            // `as_i64()` alone panics ("Expected int") on a `[Float; N]`
            // element or any pointer-tagged value, which is what crashed the
            // stdlib bake's const-eval. Int → the unboxed i64; Float → the
            // f64 bit-pattern (raw double, since non-NaN floats are stored
            // untagged); everything else → the full 64-bit bits (matches
            // DerefMutRaw's task-#40 pointer round-trip). The raw bytes are
            // truncated to `elem_size` below, coherent with the raw load.
            let val_value = state.get_reg(val_reg);
            let val: u64 = if val_value.is_int() {
                val_value.as_i64() as u64
            } else if let Some(f) = val_value.try_as_f64() {
                // A 4-byte float slot ([Float32; N]) holds the IEEE-754 SINGLE
                // bit pattern — narrow the double first (TYPED-ARRAY-FLOAT-1
                // #28). Truncating the 64-bit double pattern to 32 bits would
                // store the low half of its mantissa, not the f32 value.
                if elem_size == 4 {
                    (f as f32).to_bits() as u64
                } else {
                    f.to_bits()
                }
            } else {
                val_value.bits()
            };

            // Bounds check against `header.size` (BYTES). The borrow is scoped
            // and dropped before the write.
            let array_bytes = {
                // SAFETY: `arr_ptr` is non-null (checked) and begins with an
                // `ObjectHeader`; the borrow does not escape this block.
                let header = unsafe {
                    &*(arr_ptr as *const super::super::super::heap::ObjectHeader)
                };
                header.size as usize
            };
            let elem_stride = elem_size.max(1);
            let offset = (idx as usize).checked_mul(elem_size).ok_or({
                InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_bytes / elem_stride,
                }
            })?;
            if offset.checked_add(elem_size).map_or(true, |end| end > array_bytes) {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_bytes / elem_stride,
                });
            }
            // SAFETY: `offset + elem_size <= array_bytes` (checked above), so
            // the write stays within the live allocation. No outstanding
            // references (the `header` borrow was dropped). `write_unaligned`
            // is unnecessary — typed-array elements are naturally aligned at
            // `HEADER + idx*elem_size` for power-of-two `elem_size`.
            unsafe {
                let data_ptr =
                    arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + offset);
                match elem_size {
                    1 => *data_ptr = val as u8,
                    2 => *(data_ptr as *mut u16) = val as u16,
                    4 => *(data_ptr as *mut u32) = val as u32,
                    8 => *(data_ptr as *mut u64) = val,
                    _ => {
                        // Unknown width: write the low bytes little-endian.
                        let bytes = val.to_le_bytes();
                        for (i, b) in bytes.iter().enumerate().take(elem_size.min(8)) {
                            *data_ptr.add(i) = *b;
                        }
                    }
                }
            }
            Ok(DispatchResult::Continue)
        }

        0x40 => {
            let dst = read_reg(state)?;
            let slot_lo = read_u8(state)? as u16;
            let slot_hi = read_u8(state)? as u16;
            let slot = (slot_hi << 8) | slot_lo;
            // Lazy allocation — first read of any static-mut slot in
            // this interpreter allocates a zero-initialised cell.
            // Subsequent reads return the same stable address.
            let cell_addr = state.static_mut_cell_addr(slot);
            if crate::interpreter::env_flags::is_set(crate::interpreter::env_flags::Flag::TraceStaticmut) {
                eprintln!("[staticmut-trace] 0x52 slot={} -> addr={:p}", slot, cell_addr);
            }
            state.set_reg(dst, Value::from_ptr::<u8>(cell_addr));
            Ok(DispatchResult::Continue)
        }

        0x41 => {
            // Wide twin of StaticMutAddr (T0133): [dst][slot16][size16].
            let dst = read_reg(state)?;
            let slot_lo = read_u8(state)? as u16;
            let slot_hi = read_u8(state)? as u16;
            let size_lo = read_u8(state)? as u16;
            let size_hi = read_u8(state)? as u16;
            let slot = (slot_hi << 8) | slot_lo;
            let size = (((size_hi << 8) | size_lo) as usize).max(1);
            let cell_addr = state.static_mut_cell_addr_sized(slot, size);
            state.set_reg(dst, Value::from_ptr::<u8>(cell_addr));
            Ok(DispatchResult::Continue)
        }


        _ => Err(InterpreterError::NotImplemented {
            feature: "mem_extended sub-opcode",
            opcode: Some(Opcode::MemExtended),
        }),
    }
}
