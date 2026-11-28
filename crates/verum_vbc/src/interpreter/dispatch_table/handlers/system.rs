//! System operation handlers for VBC interpreter dispatch.
//!
//! Handles: SyscallLinux (0xE0), Mmap (0xE1), Munmap (0xE2),
//! AtomicLoad (0xE3), AtomicStore (0xE4), AtomicCas (0xE5), AtomicFence (0xE6),
//! IoSubmit (0xE7), IoPoll (0xE8), TlsGet (0xE9), TlsSet (0xEA),
//! GradBegin (0xEB), GradEnd (0xEC), GradCheckpoint (0xED),
//! GradAccumulate (0xEE), GradStop (0xEF)

use super::super::super::autodiff::GradMode as AutodiffGradMode;
use super::super::super::autodiff_record::{begin_recording, finish_recording};
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use crate::types::TypeId;
use crate::value::Value;

// ============================================================================
// Syscall (0xE0)
// ============================================================================

/// SyscallLinux (0xE0) - Raw syscall with up to 6 arguments.
pub(in super::super) fn handle_syscall(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let mut read_word = || {
        let reg = read_reg(state)?;
        state.get_reg(reg).try_as_i64().ok_or(InterpreterError::TypeMismatch {
            expected: "Int",
            got: "non-integer",
            operation: "raw syscall word",
        })
    };
    let num = read_word()?;
    let mut args = [0usize; 6];
    for arg in &mut args {
        *arg = read_word()? as usize;
    }
    let [a1, a2, a3, a4, a5, a6] = args;

    #[cfg(target_os = "linux")]
    {
        // clone/clone3 would resume the same VM stack and mutable interpreter
        // state in another host thread. Never invoke them without isolation.
        let thread_operation = if num == libc::SYS_clone {
            Some("clone")
        } else if num == libc::SYS_clone3 {
            Some("clone3")
        } else {
            None
        };
        if let Some(operation) = thread_operation {
            return Err(InterpreterError::UnsupportedRuntimeOperation {
                operation: operation.into(),
                reason: "native threads require isolated concurrent VM execution".into(),
            });
        }
        // SAFETY: the raw syscall intrinsic supplies the host ABI words; its
        // caller owns pointer validity. Read the calling thread's errno before
        // performing any further host operation on the failure path.
        let result = unsafe {
            match libc::syscall(num, a1, a2, a3, a4, a5, a6) {
                -1 => -i64::from(*libc::__errno_location()),
                value => value as i64,
            }
        };
        state.set_reg(dst, Value::from_i64(result));
        Ok(DispatchResult::Continue)
    }

    #[cfg(target_os = "macos")]
    {
        let result = unsafe { libc::syscall(num as i32, a1, a2, a3, a4, a5, a6) };
        state.set_reg(dst, Value::from_i64(result as i64));
        Ok(DispatchResult::Continue)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (dst, num, a1, a2, a3, a4, a5, a6);
        Err(InterpreterError::NotImplemented {
            feature: "syscall: platform not supported",
            opcode: Some(crate::instruction::Opcode::SyscallLinux),
        })
    }
}

// ============================================================================
// Atomic Operations (0xE3-0xE6)
// ============================================================================

/// AtomicLoad (0xE3)
pub(in super::super) fn handle_atomic_load(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr_reg = read_reg(state)?;
    let ordering = read_u8(state)?;
    let size = read_u8(state)?;

    // Accept both Pointer-tagged and Int-tagged values as raw addresses
    // — StructFieldAddr (#37) returns Pointer-tagged via Value::from_ptr
    // for true heap addresses; legacy callers may pass an Int. Same
    // pattern as DerefRaw / DerefMutRaw.
    let val = state.get_reg(ptr_reg);
    let ptr = if val.is_ptr() {
        val.as_ptr::<u8>() as usize
    } else {
        val.as_i64() as usize
    };

    use std::sync::atomic::Ordering;
    let ord = match ordering {
        0 => Ordering::Relaxed,
        1 => Ordering::Acquire,
        2 | 4 => Ordering::SeqCst,
        _ => Ordering::SeqCst,
    };

    let value = if ptr == 0 || ptr < 0x1000 || (size > 1 && !ptr.is_multiple_of(size as usize)) {
        0i64
    } else {
        unsafe {
            match size {
                1 => {
                    let atomic = &*(ptr as *const std::sync::atomic::AtomicU8);
                    atomic.load(ord) as i64
                }
                2 => {
                    let atomic = &*(ptr as *const std::sync::atomic::AtomicU16);
                    atomic.load(ord) as i64
                }
                4 => {
                    let atomic = &*(ptr as *const std::sync::atomic::AtomicU32);
                    atomic.load(ord) as i64
                }
                8 => {
                    let atomic = &*(ptr as *const std::sync::atomic::AtomicU64);
                    AtomicWordStorage::at(state, ptr).decode(atomic.load(ord))?
                }
                _ => {
                    return Err(InterpreterError::InvalidOperand {
                        message: format!("invalid atomic size: {}", size),
                    });
                }
            }
        }
    };

    state.set_reg(dst, Value::from_i64(value));
    Ok(DispatchResult::Continue)
}

/// A declared record field stores a Value. Static, native and raw allocation
/// cells store machine words, including words whose bits resemble Value tags.
#[derive(Clone, Copy)]
enum AtomicWordStorage {
    ValueField,
    MachineWord,
}

impl AtomicWordStorage {
    fn at(state: &InterpreterState, address: usize) -> Self {
        if state.heap.is_exposed_value_field(address) {
            Self::ValueField
        } else {
            Self::MachineWord
        }
    }

    fn decode(self, bits: u64) -> InterpreterResult<i64> {
        match self {
            Self::MachineWord => Ok(bits as i64),
            Self::ValueField => Value::from_bits(bits).try_as_i64().ok_or(
                InterpreterError::TypeMismatch {
                    expected: "64-bit integer",
                    got: "non-integer record field",
                    operation: "atomic word access",
                },
            ),
        }
    }

    fn encode(self, value: i64) -> u64 {
        match self {
            Self::MachineWord => value as u64,
            Self::ValueField => Value::from_i64(value).bits(),
        }
    }

    fn encode_operand(self, operand: Value, integer: i64) -> u64 {
        match self {
            Self::MachineWord => integer as u64,
            Self::ValueField if operand.is_inline_int() || operand.is_boxed_int() => operand.bits(),
            Self::ValueField => Value::from_i64(integer).bits(),
        }
    }
}

/// AtomicStore (0xE4)
pub(in super::super) fn handle_atomic_store(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let ptr_reg = read_reg(state)?;
    let val_reg = read_reg(state)?;
    let ordering = read_u8(state)?;
    let size = read_u8(state)?;

    // Same pointer-extraction pattern as handle_atomic_load (#37).
    let ptr_val = state.get_reg(ptr_reg);
    let ptr = if ptr_val.is_ptr() {
        ptr_val.as_ptr::<u8>() as usize
    } else {
        ptr_val.as_i64() as usize
    };
    let operand = state.get_reg(val_reg);
    let val = operand.as_i64();

    if ptr == 0 || ptr < 0x1000 {
        return Err(InterpreterError::NullPointer);
    }

    if size > 1 && !ptr.is_multiple_of(size as usize) {
        return Err(InterpreterError::InvalidOperand {
            message: format!("misaligned atomic store: ptr=0x{:x}, size={}", ptr, size),
        });
    }

    use std::sync::atomic::Ordering;
    let ord = match ordering {
        0 => Ordering::Relaxed,
        1 => Ordering::Release,
        2 | 4 => Ordering::SeqCst,
        _ => Ordering::SeqCst,
    };

    unsafe {
        match size {
            1 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU8);
                atomic.store(val as u8, ord);
            }
            2 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU16);
                atomic.store(val as u16, ord);
            }
            4 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU32);
                atomic.store(val as u32, ord);
            }
            8 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU64);
                atomic.store(AtomicWordStorage::at(state, ptr).encode_operand(operand, val), ord);
            }
            _ => {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("invalid atomic size: {}", size),
                });
            }
        }
    }

    Ok(DispatchResult::Continue)
}

/// AtomicCas (0xE5)
pub(in super::super) fn handle_atomic_cas(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr_reg = read_reg(state)?;
    let expected_reg = read_reg(state)?;
    let desired_reg = read_reg(state)?;
    let ordering = read_u8(state)?;
    let size = read_u8(state)?;

    // Same pointer-extraction pattern as handle_atomic_load (#37).
    let ptr_val = state.get_reg(ptr_reg);
    let ptr = if ptr_val.is_ptr() {
        ptr_val.as_ptr::<u8>() as usize
    } else {
        ptr_val.as_i64() as usize
    };
    let expected = state.get_reg(expected_reg).as_i64();
    let desired_value = state.get_reg(desired_reg);
    let desired = desired_value.as_i64();

    if ptr == 0 || ptr < 0x1000 {
        return Err(InterpreterError::NullPointer);
    }

    if size > 1 && !ptr.is_multiple_of(size as usize) {
        return Err(InterpreterError::InvalidOperand {
            message: format!("misaligned atomic CAS: ptr=0x{:x}, size={}", ptr, size),
        });
    }

    use std::sync::atomic::Ordering;
    let (success_ord, failure_ord) = match ordering {
        0 => (Ordering::Relaxed, Ordering::Relaxed),
        1 => (Ordering::Acquire, Ordering::Acquire),
        2 => (Ordering::Release, Ordering::Relaxed),
        3 => (Ordering::AcqRel, Ordering::Acquire),
        4 => (Ordering::SeqCst, Ordering::SeqCst),
        _ => (Ordering::SeqCst, Ordering::SeqCst),
    };

    let (old_value, success) = unsafe {
        match size {
            1 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU8);
                match atomic.compare_exchange(
                    expected as u8,
                    desired as u8,
                    success_ord,
                    failure_ord,
                ) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            2 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU16);
                match atomic.compare_exchange(
                    expected as u16,
                    desired as u16,
                    success_ord,
                    failure_ord,
                ) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            4 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU32);
                match atomic.compare_exchange(
                    expected as u32,
                    desired as u32,
                    success_ord,
                    failure_ord,
                ) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            8 => {
                // Equal wide integers can have distinct Value encodings.
                // Compare their complete values, then swap the observed bits.
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU64);
                let storage = AtomicWordStorage::at(state, ptr);
                let replacement = storage.encode_operand(desired_value, desired);
                let mut raw = atomic.load(failure_ord);
                loop {
                    let observed = storage.decode(raw)?;
                    if observed != expected {
                        break (observed, false);
                    }
                    match atomic.compare_exchange(
                        raw,
                        replacement,
                        success_ord,
                        failure_ord,
                    ) {
                        Ok(_) => break (expected, true),
                        Err(actual) => raw = actual,
                    }
                }
            }
            _ => {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("invalid atomic size: {}", size),
                });
            }
        }
    };

    // Pack (old_value, success) as a 2-slot Tuple heap object so
    // the destructuring `let (actual, did_swap) = atomic_cas_*` in
    // user code can Unpack it correctly. The previous convention
    // wrote dst (i64) and dst+1 (Bool) directly, but the codegen
    // for intrinsic call sites doesn't allocate a paired register
    // pair — it allocates ONE dst — so the dst+1 write was
    // unreachable and `did_swap` arrived as nil. Discovered while
    // validating task #39's NaN-box CAS fix: the underlying CAS
    // succeeded but the result tuple destructure read garbage.
    let data_size = 2 * std::mem::size_of::<Value>();
    let obj = state
        .heap
        .alloc_with_init(TypeId::TUPLE, data_size, |_| {})?;
    let data_ptr = obj.data_ptr();
    unsafe {
        let slot_ptr = data_ptr as *mut Value;
        std::ptr::write(slot_ptr, Value::from_i64(old_value));
        std::ptr::write(slot_ptr.add(1), Value::from_bool(success));
    }
    state.set_reg(dst, Value::from_ptr(obj.as_ptr()));
    Ok(DispatchResult::Continue)
}

/// AtomicRmw — `FfiExtended` sub-op `SystemSubOpcode::AtomicRmw` (0xBC).
///
/// Format: `dst:reg, ptr:reg, val:reg, op:u8, size:u8`.  `dst` receives
/// the value read BEFORE the update (the C11 / LLVM `atomicrmw`
/// convention shared with Tier-1).
///
/// The whole point of this opcode is that the read-modify-write is
/// INDIVISIBLE.  Its predecessor — an `AtomicLoad`, an arithmetic
/// instruction and one `AtomicCas` emitted inline, with the CAS result
/// thrown away — is atomic in each of its three steps and racy as a
/// whole: when two threads interleave, both read the same old value and
/// one update is silently lost.  Nothing observes the failure, and a
/// single-threaded test can never see it.
pub(in super::super) fn handle_atomic_rmw(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    use crate::instruction::AtomicRmwOp;
    use std::sync::atomic::Ordering;

    let dst = read_reg(state)?;
    let ptr_reg = read_reg(state)?;
    let val_reg = read_reg(state)?;
    let op_byte = read_u8(state)?;
    let size = read_u8(state)?;

    let op = AtomicRmwOp::from_byte(op_byte).ok_or(InterpreterError::InvalidOperand {
        message: format!("invalid atomic RMW op: {}", op_byte),
    })?;

    // Same dual pointer extraction as the sibling atomic handlers:
    // `StructFieldAddr` yields a Pointer-tagged value, older callers an Int.
    let ptr_val = state.get_reg(ptr_reg);
    let ptr = if ptr_val.is_ptr() {
        ptr_val.as_ptr::<u8>() as usize
    } else {
        ptr_val.as_i64() as usize
    };
    // BOXED-INT-OPERAND-SWEEP-1: the operand comes from user
    // expressions (`val as UInt64`) which may arrive boxed.
    let operand = state.get_reg(val_reg).as_integer_compatible();

    if ptr == 0 || ptr < 0x1000 {
        return Err(InterpreterError::NullPointer);
    }
    if size > 1 && !ptr.is_multiple_of(size as usize) {
        return Err(InterpreterError::InvalidOperand {
            message: format!("misaligned atomic RMW: ptr=0x{:x}, size={}", ptr, size),
        });
    }

    // SAFETY: `ptr` is non-null, above the reserved low page and
    // aligned for `size`; the caller owns a live atomic cell of that
    // width there, which is the same contract the sibling
    // AtomicLoad/Store/Cas handlers rely on.
    let old = unsafe {
        match size {
            1 => {
                let a = &*(ptr as *const std::sync::atomic::AtomicU8);
                let v = operand as u8;
                match op {
                    AtomicRmwOp::Add => a.fetch_add(v, Ordering::SeqCst),
                    AtomicRmwOp::Sub => a.fetch_sub(v, Ordering::SeqCst),
                    AtomicRmwOp::And => a.fetch_and(v, Ordering::SeqCst),
                    AtomicRmwOp::Or => a.fetch_or(v, Ordering::SeqCst),
                    AtomicRmwOp::Xor => a.fetch_xor(v, Ordering::SeqCst),
                    AtomicRmwOp::Xchg => a.swap(v, Ordering::SeqCst),
                }
                .into()
            }
            2 => {
                let a = &*(ptr as *const std::sync::atomic::AtomicU16);
                let v = operand as u16;
                match op {
                    AtomicRmwOp::Add => a.fetch_add(v, Ordering::SeqCst),
                    AtomicRmwOp::Sub => a.fetch_sub(v, Ordering::SeqCst),
                    AtomicRmwOp::And => a.fetch_and(v, Ordering::SeqCst),
                    AtomicRmwOp::Or => a.fetch_or(v, Ordering::SeqCst),
                    AtomicRmwOp::Xor => a.fetch_xor(v, Ordering::SeqCst),
                    AtomicRmwOp::Xchg => a.swap(v, Ordering::SeqCst),
                }
                .into()
            }
            4 => {
                let a = &*(ptr as *const std::sync::atomic::AtomicU32);
                let v = operand as u32;
                match op {
                    AtomicRmwOp::Add => a.fetch_add(v, Ordering::SeqCst),
                    AtomicRmwOp::Sub => a.fetch_sub(v, Ordering::SeqCst),
                    AtomicRmwOp::And => a.fetch_and(v, Ordering::SeqCst),
                    AtomicRmwOp::Or => a.fetch_or(v, Ordering::SeqCst),
                    AtomicRmwOp::Xor => a.fetch_xor(v, Ordering::SeqCst),
                    AtomicRmwOp::Xchg => a.swap(v, Ordering::SeqCst),
                }
                .into()
            }
            8 => {
                let a = &*(ptr as *const std::sync::atomic::AtomicU64);
                let storage = AtomicWordStorage::at(state, ptr);
                let mut raw = a.load(Ordering::SeqCst);
                loop {
                    let current = storage.decode(raw)?;
                    let next = storage.encode(op.apply(current, operand));
                    match a.compare_exchange_weak(
                        raw,
                        next,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break current,
                        Err(actual) => raw = actual,
                    }
                }
            }
            _ => {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("invalid atomic RMW size: {}", size),
                });
            }
        }
    };

    state.set_reg(dst, Value::from_i64(old));
    Ok(DispatchResult::Continue)
}

/// AtomicFence (0xE6)
pub(in super::super) fn handle_atomic_fence(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let immediate = read_u8(state)?;
    if immediate == 0xff {
        std::hint::spin_loop();
    } else {
        let order = crate::instruction::MemoryOrdering::from_immediate(immediate)
            .ok_or_else(|| invalid_fence_order(i64::from(immediate)))?;
        execute_fence(order, false);
    }
    Ok(DispatchResult::Continue)
}

pub(super) fn invalid_fence_order(value: i64) -> InterpreterError {
    InterpreterError::Panic { message: format!("invalid memory fence ordering {value}") }
}

pub(super) fn execute_fence(order: crate::instruction::MemoryOrdering, compiler_only: bool) {
    use crate::instruction::MemoryOrdering;
    use std::sync::atomic::{Ordering, compiler_fence, fence};
    let ordering = match order {
        MemoryOrdering::Relaxed => return,
        MemoryOrdering::Acquire => Ordering::Acquire,
        MemoryOrdering::Release => Ordering::Release,
        MemoryOrdering::AcqRel => Ordering::AcqRel,
        MemoryOrdering::SeqCst => Ordering::SeqCst,
    };
    if compiler_only { compiler_fence(ordering); } else { fence(ordering); }
}

// ============================================================================
// TLS Operations (0xE9-0xEA)
// ============================================================================

/// TlsGet (0xE9) - Get thread-local storage value
pub(in super::super) fn handle_tls_get(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let slot_reg = read_reg(state)?;

    let slot = state.get_reg(slot_reg).as_i64() as usize;
    let value = state.tls_get(slot).unwrap_or_default();

    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// TlsSet (0xEA) - Set thread-local storage value
pub(in super::super) fn handle_tls_set(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let slot_reg = read_reg(state)?;
    let val_reg = read_reg(state)?;

    let slot = state.get_reg(slot_reg).as_i64() as usize;
    let value = state.get_reg(val_reg);

    state.tls_set(slot, value);

    Ok(DispatchResult::Continue)
}

// ============================================================================
// Autodiff Operations (0xEB-0xEF)
// ============================================================================

/// GradBegin (0xEB) - Begin a gradient computation scope
///
/// The `wrt` registers name the values being differentiated with respect to.
/// Each becomes a leaf node on the tape, and forward arithmetic records against
/// them until the matching `GradEnd`.
pub(in super::super) fn handle_grad_begin(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    // scope_id and the wrt count are varint-encoded by the serializer
    // (encode_varint / encode_reg_vec at bytecode.rs GradBegin); read them at
    // the same width or the stream desyncs (a 1-byte varint read as 4-byte u32
    // shifted the stream and surfaced as "invalid TypeRef tag" — this handler
    // was never exercised until grad() was wired end-to-end).
    let _scope_id = read_varint(state)? as u32;
    let mode_byte = read_u8(state)?;
    let num_wrt = read_varint(state)? as usize;

    let mut wrt = Vec::with_capacity(num_wrt);
    for _ in 0..num_wrt {
        wrt.push(read_reg(state)?);
    }

    let mode = match mode_byte {
        0 => AutodiffGradMode::Reverse,
        1 => AutodiffGradMode::Forward,
        2 => AutodiffGradMode::Auto,
        _ => AutodiffGradMode::Reverse,
    };

    state.grad_tape.begin_scope(mode);
    begin_recording(state, &wrt);

    Ok(DispatchResult::Continue)
}

/// GradEnd (0xEC) - Stop recording and hand back a pullback handle
///
/// Reverse mode separates the forward recording from the backward sweep, so
/// this does not run backward. It closes the scope, retains the tape, and
/// writes the handle that reaches it into `handle_reg`; the pullback closure
/// carries that handle to `GRAD_BACKWARD` when it is finally applied to a
/// cotangent seed.
///
/// When the caller supplied explicit gradient destinations, the eager form
/// applies the pullback immediately with a unit seed instead.
pub(in super::super) fn handle_grad_end(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    // Same varint contract as GradBegin (bytecode.rs GradEnd serializer).
    let _scope_id = read_varint(state)? as u32;
    let output_reg = read_reg(state)?;
    let handle_reg = read_reg(state)?;
    let num_grads = read_varint(state)? as usize;

    let mut grad_regs = Vec::with_capacity(num_grads);
    for _ in 0..num_grads {
        grad_regs.push(read_reg(state)?);
    }

    let handle = finish_recording(state, output_reg).unwrap_or(0);
    state.set_reg(handle_reg, Value::from_i64(handle as i64));

    if !grad_regs.is_empty() {
        let grads = state
            .grad_tape
            .run_pullback(handle, 1.0)
            .unwrap_or_default();
        for (i, grad_reg) in grad_regs.iter().enumerate() {
            let val = grads.get(i).copied().unwrap_or(0.0);
            state.set_reg(*grad_reg, Value::from_f64(val));
        }
    }

    Ok(DispatchResult::Continue)
}

/// GradCheckpoint (0xED) - Create a gradient checkpoint
pub(in super::super) fn handle_grad_checkpoint(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    // Varint-encoded by the serializer (bytecode.rs GradCheckpoint), like the
    // other grad scope-ops.
    let _checkpoint_id = read_varint(state)? as u32;
    let num_tensors = read_varint(state)? as usize;

    for _ in 0..num_tensors {
        let _reg = read_reg(state)?;
    }

    let _cp_id = state.grad_tape.checkpoint_all();

    Ok(DispatchResult::Continue)
}

/// GradAccumulate (0xEE) - Accumulate gradients
pub(in super::super) fn handle_grad_accumulate(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    let dst_val = state.get_reg(dst);
    let src_val = state.get_reg(src);

    if dst_val.is_int() && src_val.is_int() {
        let dst_i = dst_val.as_i64();
        let src_i = src_val.as_i64();
        state.set_reg(dst, Value::from_i64(dst_i + src_i));
    } else {
        let dst_f = dst_val.as_f64();
        let src_f = src_val.as_f64();
        state.set_reg(dst, Value::from_f64(dst_f + src_f));
    }

    Ok(DispatchResult::Continue)
}

/// GradStop (0xEF) - Stop gradient flow (detach tensor)
pub(in super::super) fn handle_grad_stop(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    let src_val = state.get_reg(src);
    state.set_reg(dst, src_val);

    Ok(DispatchResult::Continue)
}

// ============================================================================
// Memory Mapping Operations (0xE1-0xE2)
// ============================================================================

/// Mmap (0xE1) - Memory map a region.
#[cfg(target_os = "linux")]
pub(in super::super) fn handle_mmap(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _addr = read_varint(state)?;
    let _len = read_varint(state)?;
    let _prot = read_varint(state)?;
    let _flags = read_varint(state)?;
    let _fd = read_signed_varint(state)?;
    let _offset = read_varint(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

#[cfg(not(target_os = "linux"))]
pub(in super::super) fn handle_mmap(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _ = read_varint(state)?;
    let _ = read_varint(state)?;
    let _ = read_varint(state)?;
    let _ = read_varint(state)?;
    let _ = read_signed_varint(state)?;
    let _ = read_varint(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

/// Munmap (0xE2) - Unmap a memory region.
#[cfg(target_os = "linux")]
pub(in super::super) fn handle_munmap(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _addr = read_varint(state)?;
    let _len = read_varint(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

#[cfg(not(target_os = "linux"))]
pub(in super::super) fn handle_munmap(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _ = read_varint(state)?;
    let _ = read_varint(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// IO Operations (0xE7-0xE8)
// ============================================================================

/// IoSubmit (0xE7) - Submit I/O operation to IOEngine.
pub(in super::super) fn handle_io_submit(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _ops_reg = read_reg(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

/// IoPoll (0xE8) - Poll IOEngine for completions.
pub(in super::super) fn handle_io_poll(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _timeout_reg = read_reg(state)?;
    let list_obj = state
        .heap
        .alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    unsafe {
        let base = list_obj.as_ptr() as *mut Value;
        base.write(Value::from_i64(0));
        base.add(1).write(Value::from_i64(0));
    }
    state.set_reg(dst, Value::from_ptr(list_obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}
