//! Native leaf of SysExtended.LinuxSignalWatch. The kernel may invoke the
//! handler on any thread, even while another thread is dropping an interpreter.
//! Only permanently allocated atomic counters are reachable from that handler.
use crate::signal_watch::{SignalWatchOp, abi};
use std::sync::{
    Mutex, TryLockError,
    atomic::{AtomicU64, Ordering::Relaxed},
};
use verum_common::{Heap, Maybe};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Action {
    handler: usize,
    flags: u64,
    restorer: usize,
    mask: u64,
}
const _: () = assert!(size_of::<Action>() == 32);

struct Disposition {
    ticket: i64,
    previous: Action,
}
struct Slot {
    owner: Mutex<Disposition>,
    pending: AtomicU64,
}
static SLOTS: [Slot; 64] = [const {
    Slot {
        owner: Mutex::new(Disposition {
            ticket: 0,
            previous: Action {
                handler: 0,
                flags: 0,
                restorer: 0,
                mask: 0,
            },
        }),
        pending: AtomicU64::new(0),
    }
}; 64];

extern "C" fn count_delivery(signal: i32) {
    if let Some(slot) = SLOTS.get((signal as usize).wrapping_sub(1)) {
        // The first unrepresentable count is a sticky overflow until drained.
        let mut n = slot.pending.load(Relaxed);
        while n <= i64::MAX as u64 {
            match slot
                .pending
                .compare_exchange_weak(n, n + 1, Relaxed, Relaxed)
            {
                Ok(_) => break,
                Err(observed) => n = observed,
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "C" fn restore_context() -> ! {
    core::arch::naked_asm!("mov rax, {}", "syscall", const abi::X86_SIGRETURN)
}

fn observer() -> Action {
    #[cfg(target_arch = "x86_64")]
    let (flags, restorer) = (
        abi::SA_RESTART | abi::SA_RESTORER,
        restore_context as *const () as usize,
    );
    #[cfg(target_arch = "aarch64")]
    let (flags, restorer) = (abi::SA_RESTART, 0);
    Action {
        handler: count_delivery as *const () as usize,
        flags,
        restorer,
        mask: 0,
    }
}

fn exchange(signal: usize, action: &Action, previous: Maybe<&mut Action>) -> i64 {
    let old = previous.map_or(std::ptr::null_mut(), std::ptr::from_mut);
    let action = std::ptr::from_ref(action);
    let result: i64;
    // SAFETY: the kernel reads/writes the LP64 UAPI structures for this call
    // only. The installed callback and (where required) restorer live forever.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("syscall", inlateout("rax") abi::X86_SIGACTION => result,
            in("rdi") signal, in("rsi") action, in("rdx") old, in("r10") abi::MASK_BYTES,
            lateout("rcx") _, lateout("r11") _, options(nostack));
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("svc 0", in("x8") abi::ARM_SIGACTION, inlateout("x0") signal => result,
            in("x1") action, in("x2") old, in("x3") abi::MASK_BYTES, options(nostack));
    }
    result
}

/// A positive ticket owns the disposition; its negative remembers the last
/// generation when free. Tickets are never reused, including after overflow.
fn apply(op: SignalWatchOp, value: i64, cleanup: bool) -> i64 {
    let Some(signal) = op.signal(value) else {
        return -22;
    };
    let slot = &SLOTS[signal - 1];
    // Teardown waits for a concurrent owner to leave the control section;
    // never retry a kernel error (including a seccomp refusal) in Drop.
    let lock = if cleanup {
        slot.owner.lock().map_err(TryLockError::Poisoned)
    } else {
        slot.owner.try_lock()
    };
    let mut owner = match lock {
        Ok(owner) => owner,
        Err(TryLockError::WouldBlock) => return -16,
        Err(TryLockError::Poisoned(_)) => return -5,
    };
    if op == SignalWatchOp::Install {
        if owner.ticket > 0 {
            return -16;
        }
        let ticket = if owner.ticket == 0 {
            value
        } else {
            let Some(next) = (-owner.ticket).checked_add(64) else {
                return -75;
            };
            next
        };
        slot.pending.store(0, Relaxed);
        let result = exchange(signal, &observer(), Some(&mut owner.previous));
        if result < 0 {
            return result;
        }
        owner.ticket = ticket;
        return ticket;
    }
    if owner.ticket != value {
        return -9;
    }
    if op == SignalWatchOp::Take {
        return i64::try_from(slot.pending.swap(0, Relaxed)).unwrap_or(-75);
    }
    let result = exchange(signal, &owner.previous, None);
    if result == 0 {
        owner.ticket = -value;
    }
    result
}

/// An interpreter cannot drain or restore another interpreter's registration.
/// An unused VM allocates no watch table. Reset and destruction return any
/// remaining dispositions to the host, even after a guest trap.
#[derive(Default)]
pub(super) struct OwnedWatches(Maybe<Heap<[i64; 64]>>);
impl OwnedWatches {
    pub(super) fn execute(
        &mut self,
        op: SignalWatchOp,
        value: i64,
    ) -> super::InterpreterResult<i64> {
        let Some(signal) = op.signal(value) else {
            return Ok(-22);
        };
        let index = signal - 1;
        if op != SignalWatchOp::Install && self.0.as_ref().is_none_or(|t| t[index] != value) {
            return Ok(-9);
        }
        let result = apply(op, value, false);
        if result >= 0 {
            match op {
                SignalWatchOp::Install => {
                    self.0.get_or_insert_with(|| Heap::new([0; 64]))[index] = result
                }
                SignalWatchOp::Restore => {
                    if let Some(t) = &mut self.0 {
                        t[index] = 0;
                    }
                }
                SignalWatchOp::Take => {}
            }
        }
        Ok(result)
    }
}
impl Drop for OwnedWatches {
    fn drop(&mut self) {
        if let Some(tickets) = &self.0 {
            for &ticket in tickets.iter().filter(|&&t| t > 0) {
                let result = apply(SignalWatchOp::Restore, ticket, true);
                if result < 0 {
                    tracing::error!(
                        ticket,
                        errno = -result,
                        "could not restore Linux signal disposition on guest teardown"
                    );
                }
            }
        }
    }
}
