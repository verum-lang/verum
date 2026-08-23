//! `Opcode::MachExtended` — the mach family's honest home (T0852).
//! These arms lived in `ffi_extended.rs` as `SystemSubOpcode` squatters;
//! the operations have nothing to do with FFI.
//!
//! Wire: standard extended envelope via `dispatch_enveloped`.

use super::super::DispatchResult;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::bytecode_io::read_reg;
use super::envelope::dispatch_enveloped;
use crate::instruction::{Opcode, MachSubOpcode};
use crate::interpreter::error::InterpreterError;

pub(in super::super) fn handle_mach_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, mach_extended_body)
}

fn mach_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    match MachSubOpcode::from_byte(sub_op_byte) {
        Some(MachSubOpcode::VmAllocate) => {
            // Format: dst:reg, size:reg, anywhere:reg
            let _dst = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            let _anywhere_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_vm_allocate: the interpreter has no Mach kernel binding — call \
                          vm_allocate through core.sys.darwin.mach (an @ffi(\"libSystem.B.dylib\") \
                          extern block), not the mach_vm_allocate intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::VmDeallocate) => {
            // Format: dst:reg, addr:reg, size:reg
            let _dst = read_reg(state)?;
            let _addr_reg = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_vm_deallocate: the interpreter has no Mach kernel binding — call \
                          vm_deallocate through core.sys.darwin.mach (an @ffi(\"libSystem.B.dylib\") \
                          extern block), not the mach_vm_deallocate intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::VmProtect) => {
            // Format: dst:reg, addr:reg, size:reg, prot:reg
            let _dst = read_reg(state)?;
            let _addr_reg = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            let _prot_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_vm_protect: the interpreter has no Mach kernel binding — call \
                          vm_protect through core.sys.darwin.mach (an @ffi(\"libSystem.B.dylib\") \
                          extern block), not the mach_vm_protect intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::SemCreate) => {
            // Format: dst:reg, initial_value:reg
            let _dst = read_reg(state)?;
            let _value_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_sem_create: the interpreter has no Mach kernel binding — call \
                          semaphore_create through core.sys.darwin.mach (an \
                          @ffi(\"libSystem.B.dylib\") extern block), not the mach_sem_create \
                          intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::SemDestroy) => {
            // Format: dst:reg, sem:reg
            let _dst = read_reg(state)?;
            let _sem_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_sem_destroy: the interpreter has no Mach kernel binding — call \
                          semaphore_destroy through core.sys.darwin.mach (an \
                          @ffi(\"libSystem.B.dylib\") extern block), not the mach_sem_destroy \
                          intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::SemSignal) => {
            // Format: dst:reg, sem:reg
            let _dst = read_reg(state)?;
            let _sem_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_sem_signal: the interpreter has no Mach kernel binding — call \
                          semaphore_signal through core.sys.darwin.mach (an \
                          @ffi(\"libSystem.B.dylib\") extern block), not the mach_sem_signal \
                          intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::SemWait) => {
            // Format: dst:reg, sem:reg
            let _dst = read_reg(state)?;
            let _sem_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_sem_wait: the interpreter has no Mach kernel binding — call \
                          semaphore_wait through core.sys.darwin.mach (an \
                          @ffi(\"libSystem.B.dylib\") extern block), not the mach_sem_wait \
                          intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::ErrorString) => {
            // Format: dst:reg, kern_return:reg
            let _dst = read_reg(state)?;
            let _err_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_error_string: the interpreter has no Mach kernel binding and will \
                          not guess a description — call error_string through \
                          core.sys.darwin.mach (an @ffi(\"libSystem.B.dylib\") extern block), not \
                          the mach_error_string intrinsic",
                opcode: None,
            })
        }

        Some(MachSubOpcode::SleepUntil) => {
            // Format: dst:reg, deadline:reg
            let _dst = read_reg(state)?;
            let _deadline_reg = read_reg(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "mach_sleep_until: the interpreter has no Mach kernel binding — call \
                          mach_wait_until through core.sys.darwin.mach (an \
                          @ffi(\"libSystem.B.dylib\") extern block), not the mach_sleep_until \
                          intrinsic",
                opcode: None,
            })
        }

        _ => Err(InterpreterError::NotImplemented {
            feature: "mach_extended sub-opcode",
            opcode: Some(Opcode::MachExtended),
        }),
    }
}


/// T0110 — the Tier-0 Mach arms must never fake success.
///
/// Every test asserts an OBSERVED behaviour of `mach_extended_body`: what the
/// handler returns, and what it leaves in the destination register. All of
/// them failed against the pre-fix arms, which reported `Continue` after
/// fabricating a value (a `std::alloc` block for `mach_vm_allocate`, an
/// incrementing fake handle for `mach_sem_create`, the string "success" for
/// EVERY code from `mach_error_string`) or after writing nothing at all.
#[cfg(test)]
mod mach_arms_never_fake_success {
    use super::*;
    use crate::instruction::Reg;
    use crate::value::Value;
    use crate::module::{FunctionDescriptor, FunctionId, VbcModule};
    use std::sync::Arc;

    const MACH_SUB_OPCODES: &[MachSubOpcode] = &[
        MachSubOpcode::VmAllocate,
        MachSubOpcode::VmDeallocate,
        MachSubOpcode::VmProtect,
        MachSubOpcode::SemCreate,
        MachSubOpcode::SemDestroy,
        MachSubOpcode::SemSignal,
        MachSubOpcode::SemWait,
        MachSubOpcode::ErrorString,
        MachSubOpcode::SleepUntil,
    ];

    /// An interpreter positioned at `operands`, with one frame of 16
    /// registers, ready for `mach_extended_body` to decode from.
    ///
    /// The operand registers hold plausible INTEGER arguments (a size, a
    /// flag, a protection mask) rather than the uninitialised nil a fresh
    /// frame starts with. That matters for the before/after measurement:
    /// with nil arguments the pre-fix `MachVmAllocate` panicked inside
    /// `Value::as_i64` and the tests would have "failed" on a crash instead
    /// of on the fabricated answer they exist to catch.
    fn state_reading(operands: &[u8]) -> InterpreterState {
        let mut module = VbcModule::new("t0110_mach".to_string());
        let name = module.strings.intern("t0110_mach_probe");
        module.bytecode.extend_from_slice(operands);
        module.add_function(FunctionDescriptor::new(name));

        let mut state = InterpreterState::new(Arc::new(module));
        state
            .call_stack
            .push_frame(FunctionId(0), 16, 0, Reg(0))
            .expect("probe frame");
        state.registers.push_frame(16);
        for (reg, arg) in [(2u16, 4096i64), (3, 1), (4, 3)] {
            state.set_reg(Reg(reg), Value::from_i64(arg));
        }
        state
    }

    /// Four operand bytes cover the widest arm (`MachVmProtect`); the
    /// narrower ones simply stop earlier.
    fn run(sub_op: MachSubOpcode) -> (InterpreterState, InterpreterResult<DispatchResult>) {
        let mut state = state_reading(&[1u8, 2, 3, 4]);
        let result = mach_extended_body(&mut state, sub_op as u8);
        (state, result)
    }

    /// The whole family refuses, and each refusal names ITSELF — a caller
    /// that gets "not implemented" has to know which of the nine failed.
    #[test]
    fn every_mach_sub_opcode_reports_not_implemented_naming_itself() {
        for &sub_op in MACH_SUB_OPCODES {
            let (_state, result) = run(sub_op);
            let err = result.expect_err("a Mach op with no binding must not report success");
            let text = err.to_string();
            let intrinsic = match sub_op {
                MachSubOpcode::VmAllocate => "mach_vm_allocate",
                MachSubOpcode::VmDeallocate => "mach_vm_deallocate",
                MachSubOpcode::VmProtect => "mach_vm_protect",
                MachSubOpcode::SemCreate => "mach_sem_create",
                MachSubOpcode::SemDestroy => "mach_sem_destroy",
                MachSubOpcode::SemSignal => "mach_sem_signal",
                MachSubOpcode::SemWait => "mach_sem_wait",
                MachSubOpcode::ErrorString => "mach_error_string",
                MachSubOpcode::SleepUntil => "mach_sleep_until",
            };
            assert!(
                text.contains(&intrinsic),
                "the diagnostic must name the operation that failed; {:?} said: {text}",
                sub_op
            );
            assert!(
                text.contains("core.sys.darwin.mach"),
                "the diagnostic must name the binding that works; {:?} said: {text}",
                sub_op
            );
        }
    }

    /// `mach_error_string` answered the string "success" for every code,
    /// including failures — the one arm that reported the OPPOSITE of what
    /// happened rather than merely skipping work.
    #[test]
    fn mach_error_string_never_answers_success_for_a_failure_code() {
        let (state, result) = run(MachSubOpcode::ErrorString);
        assert!(
            result.is_err(),
            "mach_error_string must not describe an unknown KernReturn"
        );
        let dst = state.get_reg(Reg(1));
        assert!(
            !dst.is_small_string() && !dst.is_ptr(),
            "no description may reach the destination register; it holds {dst:?}"
        );
    }

    /// The destination register must be left ALONE. Pre-fix, two arms wrote
    /// a fabricated value into it (a heap pointer, a fake semaphore handle)
    /// that the caller would have used as a Mach result.
    #[test]
    fn no_mach_arm_writes_a_fabricated_value_to_its_destination() {
        for &sub_op in MACH_SUB_OPCODES {
            let mut state = state_reading(&[1u8, 2, 3, 4]);
            let sentinel = Value::from_i64(0x5EED);
            state.set_reg(Reg(1), sentinel);
            let _ = mach_extended_body(&mut state, sub_op as u8);
            // Read the value WITHOUT `as_i64`: pre-fix, three of these arms
            // left a pointer or a string here, and the panicking accessor
            // would have hidden what was actually written.
            let after = state.get_reg(Reg(1));
            assert!(
                after.is_int() && after.as_i64() == 0x5EED,
                "{:?} wrote to the destination register instead of failing; it now holds {after:?}",
                sub_op
            );
        }
    }

    /// The operand cursor must end where the encoder said the instruction
    /// ends, whatever the caller does with the error — otherwise a caught
    /// error would leave the decoder mid-instruction.
    ///
    /// Unlike its siblings this one also held BEFORE the fix (the stub arms
    /// consumed their operands too). It is here to guard the new failure
    /// path, where returning `Err` early — the obvious way to write these
    /// arms — would silently break the invariant.
    #[test]
    fn every_mach_arm_consumes_exactly_its_operands_before_failing() {
        for (sub_op, operand_count) in [
            (MachSubOpcode::VmAllocate, 3u32),
            (MachSubOpcode::VmDeallocate, 3),
            (MachSubOpcode::VmProtect, 4),
            (MachSubOpcode::SemCreate, 2),
            (MachSubOpcode::SemDestroy, 2),
            (MachSubOpcode::SemSignal, 2),
            (MachSubOpcode::SemWait, 2),
            (MachSubOpcode::ErrorString, 2),
            (MachSubOpcode::SleepUntil, 2),
        ] {
            let mut state = state_reading(&[1u8, 2, 3, 4]);
            let _ = mach_extended_body(&mut state, sub_op as u8);
            assert_eq!(
                state.pc(),
                operand_count,
                "{:?} must consume exactly {operand_count} operand bytes before failing",
                sub_op
            );
        }
    }
}
