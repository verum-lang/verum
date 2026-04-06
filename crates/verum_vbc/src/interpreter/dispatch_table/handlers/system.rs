//! System operation handlers for VBC interpreter dispatch.
//!
//! Handles: SyscallLinux (0xE0), Mmap (0xE1), Munmap (0xE2),
//! AtomicLoad (0xE3), AtomicStore (0xE4), AtomicCas (0xE5), AtomicFence (0xE6),
//! IoSubmit (0xE7), IoPoll (0xE8), TlsGet (0xE9), TlsSet (0xEA),
//! GradBegin (0xEB), GradEnd (0xEC), GradCheckpoint (0xED),
//! GradAccumulate (0xEE), GradStop (0xEF)

use crate::instruction::Reg;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::super::autodiff::GradMode as AutodiffGradMode;
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// Syscall (0xE0)
// ============================================================================

/// SyscallLinux (0xE0) - Raw syscall with up to 6 arguments.
pub(in super::super) fn handle_syscall(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let num_reg = read_reg(state)?;
    let a1_reg = read_reg(state)?;
    let a2_reg = read_reg(state)?;
    let a3_reg = read_reg(state)?;
    let a4_reg = read_reg(state)?;
    let a5_reg = read_reg(state)?;
    let a6_reg = read_reg(state)?;

    let num = state.get_reg(num_reg).as_i64();
    let a1 = state.get_reg(a1_reg).as_i64() as usize;
    let a2 = state.get_reg(a2_reg).as_i64() as usize;
    let a3 = state.get_reg(a3_reg).as_i64() as usize;
    let a4 = state.get_reg(a4_reg).as_i64() as usize;
    let a5 = state.get_reg(a5_reg).as_i64() as usize;
    let a6 = state.get_reg(a6_reg).as_i64() as usize;

    #[cfg(target_os = "linux")]
    {
        let result = unsafe {
            libc::syscall(num, a1, a2, a3, a4, a5, a6)
        };
        state.set_reg(dst, Value::from_i64(result as i64));
        Ok(DispatchResult::Continue)
    }

    #[cfg(target_os = "macos")]
    {
        let result = unsafe {
            libc::syscall(num as i32, a1, a2, a3, a4, a5, a6)
        };
        state.set_reg(dst, Value::from_i64(result as i64));
        Ok(DispatchResult::Continue)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (dst, num, a1, a2, a3, a4, a5, a6);
        Err(InterpreterError::NotImplemented {
            feature: "syscall: platform not supported",
            opcode: Some(0xE0),
        })
    }
}

// ============================================================================
// Atomic Operations (0xE3-0xE6)
// ============================================================================

/// AtomicLoad (0xE3)
pub(in super::super) fn handle_atomic_load(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr_reg = read_reg(state)?;
    let ordering = read_u8(state)?;
    let size = read_u8(state)?;

    let ptr = state.get_reg(ptr_reg).as_i64() as usize;

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
                    atomic.load(ord) as i64
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

/// AtomicStore (0xE4)
pub(in super::super) fn handle_atomic_store(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let ptr_reg = read_reg(state)?;
    let val_reg = read_reg(state)?;
    let ordering = read_u8(state)?;
    let size = read_u8(state)?;

    let ptr = state.get_reg(ptr_reg).as_i64() as usize;
    let val = state.get_reg(val_reg).as_i64();

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
                atomic.store(val as u64, ord);
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
pub(in super::super) fn handle_atomic_cas(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let ptr_reg = read_reg(state)?;
    let expected_reg = read_reg(state)?;
    let desired_reg = read_reg(state)?;
    let ordering = read_u8(state)?;
    let size = read_u8(state)?;

    let ptr = state.get_reg(ptr_reg).as_i64() as usize;
    let expected = state.get_reg(expected_reg).as_i64();
    let desired = state.get_reg(desired_reg).as_i64();

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
                match atomic.compare_exchange(expected as u8, desired as u8, success_ord, failure_ord) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            2 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU16);
                match atomic.compare_exchange(expected as u16, desired as u16, success_ord, failure_ord) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            4 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU32);
                match atomic.compare_exchange(expected as u32, desired as u32, success_ord, failure_ord) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            8 => {
                let atomic = &*(ptr as *const std::sync::atomic::AtomicU64);
                match atomic.compare_exchange(expected as u64, desired as u64, success_ord, failure_ord) {
                    Ok(old) => (old as i64, true),
                    Err(old) => (old as i64, false),
                }
            }
            _ => {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("invalid atomic size: {}", size),
                });
            }
        }
    };

    state.set_reg(dst, Value::from_i64(old_value));
    state.set_reg(Reg(dst.0 + 1), Value::from_bool(success));
    Ok(DispatchResult::Continue)
}

/// AtomicFence (0xE6)
pub(in super::super) fn handle_atomic_fence(_state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// TLS Operations (0xE9-0xEA)
// ============================================================================

/// TlsGet (0xE9) - Get thread-local storage value
pub(in super::super) fn handle_tls_get(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let slot_reg = read_reg(state)?;

    let slot = state.get_reg(slot_reg).as_i64() as usize;
    let value = state.tls_get(slot).unwrap_or_default();

    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// TlsSet (0xEA) - Set thread-local storage value
pub(in super::super) fn handle_tls_set(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_grad_begin(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _scope_id = read_u32(state)?;
    let mode_byte = read_u8(state)?;
    let num_wrt = read_u8(state)? as usize;

    for _ in 0..num_wrt {
        let _reg = read_reg(state)?;
    }

    let mode = match mode_byte {
        0 => AutodiffGradMode::Reverse,
        1 => AutodiffGradMode::Forward,
        2 => AutodiffGradMode::Auto,
        _ => AutodiffGradMode::Reverse,
    };

    state.grad_tape.begin_scope(mode);

    Ok(DispatchResult::Continue)
}

/// GradEnd (0xEC) - End gradient scope and compute gradients
pub(in super::super) fn handle_grad_end(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _scope_id = read_u32(state)?;
    let _output_reg = read_reg(state)?;
    let _grad_out_reg = read_reg(state)?;
    let num_grads = read_u8(state)? as usize;

    let mut grad_regs = Vec::with_capacity(num_grads);
    for _ in 0..num_grads {
        grad_regs.push(read_reg(state)?);
    }

    state.grad_tape.backward();

    // Collect gradient values from scope before mutating state
    let mut grad_values: Vec<f64> = Vec::with_capacity(grad_regs.len());
    if let Some(scope) = state.grad_tape.current_scope() {
        let tensor_ids: Vec<super::super::super::autodiff::TensorId> = scope.all_tensor_ids();
        for i in 0..grad_regs.len() {
            let val = if i < tensor_ids.len() {
                if let Some(grad_tensor) = scope.get_grad(tensor_ids[i]) {
                    if grad_tensor.numel == 1 {
                        if let Some(data) = &grad_tensor.data {
                            let ptr = data.as_ptr() as *const f64;
                            unsafe { *ptr }
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };
            grad_values.push(val);
        }
    }

    for (i, grad_reg) in grad_regs.iter().enumerate() {
        let val = grad_values.get(i).copied().unwrap_or(0.0);
        state.set_reg(*grad_reg, Value::from_f64(val));
    }

    state.grad_tape.end_scope();

    Ok(DispatchResult::Continue)
}

/// GradCheckpoint (0xED) - Create a gradient checkpoint
pub(in super::super) fn handle_grad_checkpoint(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _checkpoint_id = read_u32(state)?;
    let num_tensors = read_u8(state)? as usize;

    for _ in 0..num_tensors {
        let _reg = read_reg(state)?;
    }

    let _cp_id = state.grad_tape.checkpoint_all();

    Ok(DispatchResult::Continue)
}

/// GradAccumulate (0xEE) - Accumulate gradients
pub(in super::super) fn handle_grad_accumulate(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_grad_stop(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_mmap(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_mmap(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_munmap(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _addr = read_varint(state)?;
    let _len = read_varint(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

#[cfg(not(target_os = "linux"))]
pub(in super::super) fn handle_munmap(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_io_submit(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _ops_reg = read_reg(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

/// IoPoll (0xE8) - Poll IOEngine for completions.
pub(in super::super) fn handle_io_poll(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _timeout_reg = read_reg(state)?;
    let list_obj = state.heap.alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    unsafe {
        let base = list_obj.as_ptr() as *mut Value;
        base.write(Value::from_i64(0));
        base.add(1).write(Value::from_i64(0));
    }
    state.set_reg(dst, Value::from_ptr(list_obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}
