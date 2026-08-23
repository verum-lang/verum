//! `Opcode::SyncExtended` — the sync family's honest home (T0852).
//! These arms lived in `ffi_extended.rs` as `SystemSubOpcode` squatters;
//! the operations have nothing to do with FFI.
//!
//! Wire: standard extended envelope via `dispatch_enveloped`.

use super::super::DispatchResult;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::bytecode_io::read_reg;
use super::envelope::dispatch_enveloped;
use crate::instruction::{Opcode, SyncSubOpcode};
use super::ffi_extended::{futex_park, value_as_addr};
use crate::interpreter::error::InterpreterError;
use crate::value::Value;

pub(in super::super) fn handle_sync_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, sync_extended_body)
}

fn sync_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    match SyncSubOpcode::from_byte(sub_op_byte) {
        Some(SyncSubOpcode::FutexWait) => {
            // Format: dst:reg, addr:reg, expected:reg, timeout_ns:reg
            // ABI: `(addr, expected, timeout_ns) -> i64` —
            //   0      → woken
            //   -EAGAIN (-11) → `*addr != expected`
            //   -ETIMEDOUT (-110 Linux / -60 macOS; we use -110 universally)
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let expected_reg = read_reg(state)?;
            let timeout_reg = read_reg(state)?;

            // MEM-BULK-ADDR-DUAL-1: futex words arrive via as_mut_ptr
            // (ptr- OR int-tagged) — use the canonical dual extraction.
            let addr = value_as_addr(state.get_reg(addr_reg)) as *const i32;
            // BOXED-INT-OPERAND-SWEEP-1: expected/timeout originate in
            // user expressions (`Duration.as_nanos() as UInt64` casts can
            // BOX) — raw .as_i64() read box bits (the TimeSleepNanos
            // class, 490ae89c3). Canonical maybe-boxed reader.
            let expected = state.get_reg(expected_reg).as_integer_compatible() as i32;
            let timeout_ns = state.get_reg(timeout_reg).as_integer_compatible();

            let result = futex_park::wait(addr, expected, timeout_ns);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::FutexWake) => {
            // Format: dst:reg, addr:reg, count:reg
            // ABI: `(addr, count) -> i64` returns # waiters woken.
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let count_reg = read_reg(state)?;

            // MEM-BULK-ADDR-DUAL-1: dual int-or-pointer extraction.
            let addr = value_as_addr(state.get_reg(addr_reg)) as *const i32;
            // BOXED-INT-OPERAND-SWEEP-1: canonical maybe-boxed reader.
            let count = state.get_reg(count_reg).as_integer_compatible();

            let woken = futex_park::wake(addr, count);
            state.set_reg(dst, Value::from_i64(woken));
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::SpinlockLock) => {
            // Format: dst:reg, lock_addr:reg
            // ABI: `(lock_addr: i64) -> i64` (always returns 0)
            // Atomic CAS loop: 0 → 1 means lock acquired.
            let dst = read_reg(state)?;
            let lock_reg = read_reg(state)?;

            // MEM-BULK-ADDR-DUAL-1: dual int-or-pointer extraction.
            let lock_addr = value_as_addr(state.get_reg(lock_reg)) as *mut u8;
            if !lock_addr.is_null() {
                // SAFETY: caller is responsible for `lock_addr` pointing
                // at a live u8 lock cell. Use atomic CAS to flip 0→1.
                let atomic = unsafe { &*(lock_addr as *const std::sync::atomic::AtomicU8) };
                let mut spin = 0u32;
                while atomic
                    .compare_exchange_weak(
                        0,
                        1,
                        std::sync::atomic::Ordering::Acquire,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_err()
                {
                    spin = spin.saturating_add(1);
                    if spin < 64 {
                        std::hint::spin_loop();
                    } else {
                        std::thread::yield_now();
                        spin = 0;
                    }
                }
            }
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::SpinlockTryLock) => {
            let dst = read_reg(state)?;
            let lock_reg = read_reg(state)?;
            let addr = value_as_addr(state.get_reg(lock_reg));
            let acquired = if addr != 0 {
                // SAFETY: caller warrants a live u32 lock cell.
                let atomic = unsafe { &*(addr as *const std::sync::atomic::AtomicU32) };
                atomic
                    .compare_exchange(
                        0,
                        1,
                        std::sync::atomic::Ordering::Acquire,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
            } else {
                false
            };
            state.set_reg(dst, Value::from_bool(acquired));
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::SpinlockUnlock) => {
            let lock_reg = read_reg(state)?;
            let addr = value_as_addr(state.get_reg(lock_reg));
            if addr != 0 {
                // SAFETY: as above.
                let atomic = unsafe { &*(addr as *const std::sync::atomic::AtomicU32) };
                atomic.store(0, std::sync::atomic::Ordering::Release);
            }
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::SpinlockIsLocked) => {
            let dst = read_reg(state)?;
            let lock_reg = read_reg(state)?;
            let addr = value_as_addr(state.get_reg(lock_reg));
            let locked = if addr != 0 {
                // SAFETY: as above.
                let atomic = unsafe { &*(addr as *const std::sync::atomic::AtomicU32) };
                atomic.load(std::sync::atomic::Ordering::Acquire) != 0
            } else {
                false
            };
            state.set_reg(dst, Value::from_bool(locked));
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::WaitgroupNew) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(crate::interpreter::waitgroup::wg_new()));
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::WaitgroupAdd) => {
            let dst = read_reg(state)?;
            let wg_reg = read_reg(state)?;
            let delta_reg = read_reg(state)?;
            let wg = state.get_reg(wg_reg).as_integer_compatible();
            let delta = state.get_reg(delta_reg).as_integer_compatible();
            state.set_reg(
                dst,
                Value::from_i64(crate::interpreter::waitgroup::wg_add(wg, delta)),
            );
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::WaitgroupDone) => {
            let dst = read_reg(state)?;
            let wg_reg = read_reg(state)?;
            let wg = state.get_reg(wg_reg).as_integer_compatible();
            state.set_reg(
                dst,
                Value::from_i64(crate::interpreter::waitgroup::wg_done(wg)),
            );
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::WaitgroupWait) => {
            let dst = read_reg(state)?;
            let wg_reg = read_reg(state)?;
            let wg = state.get_reg(wg_reg).as_integer_compatible();
            state.set_reg(
                dst,
                Value::from_i64(crate::interpreter::waitgroup::wg_wait(wg)),
            );
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::WaitgroupTryWait) => {
            let dst = read_reg(state)?;
            let wg_reg = read_reg(state)?;
            let wg = state.get_reg(wg_reg).as_integer_compatible();
            state.set_reg(
                dst,
                Value::from_i64(crate::interpreter::waitgroup::wg_try_wait(wg)),
            );
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::WaitgroupDestroy) => {
            let dst = read_reg(state)?;
            let wg_reg = read_reg(state)?;
            let wg = state.get_reg(wg_reg).as_integer_compatible();
            state.set_reg(
                dst,
                Value::from_i64(crate::interpreter::waitgroup::wg_destroy(wg)),
            );
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::TlsSlotGet) => {
            let dst = read_reg(state)?;
            let slot_reg = read_reg(state)?;
            let slot = state.get_reg(slot_reg).as_integer_compatible() as usize;
            // TLS-SLOT-GET-NULL-1: the declared return is `*const Byte` —
            // an ABSENT slot is the NULL POINTER (0), not nil.  A nil here
            // was both type-dishonest (silent-nil class) and a cross-tier
            // divergence: the AOT twin reads a zero-initialised
            // `__verum_tls_slots` thread-local and honestly yields 0.
            let value = state
                .user_tls_slots
                .get(&(slot as u16))
                .copied()
                .unwrap_or_else(|| Value::from_i64(0));
            state.set_reg(dst, value);
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::TlsSlotSet) => {
            let slot_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let slot = state.get_reg(slot_reg).as_integer_compatible() as usize;
            let value = state.get_reg(val_reg);
            state.user_tls_slots.insert(slot as u16, value);
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::TlsSlotHas) => {
            let dst = read_reg(state)?;
            let slot_reg = read_reg(state)?;
            let slot = state.get_reg(slot_reg).as_integer_compatible() as usize;
            let occupied = state
                .user_tls_slots
                .get(&(slot as u16))
                .map(|v| !v.is_nil())
                .unwrap_or(false);
            state.set_reg(dst, Value::from_bool(occupied));
            Ok(DispatchResult::Continue)
        }
        Some(SyncSubOpcode::TlsSlotClear) => {
            let slot_reg = read_reg(state)?;
            let slot = state.get_reg(slot_reg).as_integer_compatible() as usize;
            state.user_tls_slots.remove(&(slot as u16));
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::TlsGetBase) => {
            let dst = read_reg(state)?;
            let anchor = state as *const InterpreterState as i64;
            state.set_reg(dst, Value::from_i64(anchor));
            Ok(DispatchResult::Continue)
        }

        Some(SyncSubOpcode::AtomicRmw) => super::system::handle_atomic_rmw(state),
        _ => Err(InterpreterError::NotImplemented {
            feature: "sync_extended sub-opcode",
            opcode: Some(Opcode::SyncExtended),
        }),
    }
}
