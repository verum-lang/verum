//! Red-team Round 2 §3.3 + §2.2 — VBC bytecode trust-boundary invariants.
//!
//! Pins the interpreter's defenses on hand-crafted module input:
//!
//!   * §3.3 (PENDING → DEFENSE) — `FunctionId(N)` out of range must
//!     surface as `InterpreterError::FunctionNotFound`, never panic /
//!     segfault. The defense lives in `mod.rs:136` /  `mod.rs:407` /
//!     `mod.rs:516` where `state.module.get_function(func_id).ok_or(
//!     FunctionNotFound)?` is the canonical lookup pattern.
//!
//!   * §2.2 (PENDING → DEFENSE) — branch-target offsets are encoded as
//!     `i32` in `Instruction::{Jmp, JmpIf, JmpNot, JmpCmp}` (see
//!     `instruction.rs:8455`). Functions with 2^16+ instructions are
//!     not a cliff; the encoding has ~2.1 billion offsets of headroom.
//!     Pin this by exercising a Jmp with offset >= 65,536.
//!
//! **Audit reference:** vcs/red-team/round-2-implementation.md §2.2 + §3.3.

use std::sync::Arc;

use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::interpreter::{Interpreter, InterpreterError};
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;

/// Build a minimal module with a single function (FunctionId(0)) whose
/// body is a single Ret-Unit. Used as the "valid" baseline; OOR call
/// then targets FunctionId(1), FunctionId(99999) etc.
fn build_minimal_module() -> Arc<VbcModule> {
    let mut module = VbcModule::new("oor_test".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.register_count = 4;

    let mut bc = Vec::new();
    // Encode a no-op + return — minimum viable function body.
    bytecode::encode_instruction(&Instruction::Nop, &mut bc);
    bytecode::encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut bc);
    func.bytecode_length = bc.len() as u32;

    module.functions.push(func);
    module.bytecode = bc;
    Arc::new(module)
}

// -----------------------------------------------------------------------------
// §3.3 — FunctionId(N) out of range surfaces as FunctionNotFound
// -----------------------------------------------------------------------------

#[test]
fn function_id_one_past_end_returns_function_not_found() {
    let module = build_minimal_module();
    let mut interp = Interpreter::new(module);
    // Module has FunctionId(0) only; FunctionId(1) is one-past-end.
    let result = interp.execute_function(FunctionId(1));
    assert!(
        matches!(result, Err(InterpreterError::FunctionNotFound(_))),
        "Expected FunctionNotFound, got {:?}",
        result
    );
}

#[test]
fn function_id_far_out_of_range_returns_function_not_found() {
    let module = build_minimal_module();
    let mut interp = Interpreter::new(module);
    // Pathologically far OOR — would index past any reasonable Vec length.
    let result = interp.execute_function(FunctionId(0xFFFF_FF00));
    assert!(
        matches!(result, Err(InterpreterError::FunctionNotFound(_))),
        "Expected FunctionNotFound, got {:?}",
        result
    );
}

#[test]
fn function_id_max_returns_function_not_found() {
    let module = build_minimal_module();
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(u32::MAX));
    assert!(
        matches!(result, Err(InterpreterError::FunctionNotFound(_))),
        "Expected FunctionNotFound, got {:?}",
        result
    );
}

#[test]
fn valid_function_id_zero_executes_without_error() {
    // Sanity: the OOR tests above must not be passing because all
    // FunctionId calls fail. The valid baseline must succeed.
    let module = build_minimal_module();
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0));
    assert!(
        result.is_ok(),
        "Valid FunctionId(0) must execute, got {:?}",
        result
    );
}

// -----------------------------------------------------------------------------
// §2.2 — Branch-target offsets are i32; 2^16+ instruction headroom
// -----------------------------------------------------------------------------

#[test]
fn jmp_offset_can_express_beyond_2_pow_16() {
    // The VBC `Instruction::Jmp` carries `offset: i32`; constructing a
    // value with an offset of 100_000 (= 2^16 + 34_464) compiles. The
    // type-level invariant is that branch targets are NOT i16-bounded.
    //
    // We don't actually execute this jump (it would require a bytecode
    // function with 100K+ instructions); we only assert that the type
    // system accepts the i32 value.
    let big_jmp = Instruction::Jmp { offset: 100_000_i32 };
    match big_jmp {
        Instruction::Jmp { offset } => {
            assert_eq!(offset, 100_000);
            assert!(
                offset > i16::MAX as i32,
                "If branch offset were i16-bounded, this would panic on overflow"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn jmp_offset_can_express_negative_beyond_2_pow_15() {
    let big_back_jmp = Instruction::Jmp { offset: -100_000_i32 };
    match big_back_jmp {
        Instruction::Jmp { offset } => {
            assert_eq!(offset, -100_000);
            assert!(offset < i16::MIN as i32);
        }
        _ => unreachable!(),
    }
}

#[test]
fn jmp_offset_at_i32_max_compiles() {
    // i32::MAX = 2_147_483_647 — far past any plausible function size.
    let max_jmp = Instruction::Jmp { offset: i32::MAX };
    match max_jmp {
        Instruction::Jmp { offset } => assert_eq!(offset, i32::MAX),
        _ => unreachable!(),
    }
}

#[test]
fn jmp_offset_at_i32_min_compiles() {
    let min_jmp = Instruction::Jmp { offset: i32::MIN };
    match min_jmp {
        Instruction::Jmp { offset } => assert_eq!(offset, i32::MIN),
        _ => unreachable!(),
    }
}

#[test]
fn conditional_jmp_offsets_are_also_i32() {
    // JmpIf and JmpNot must follow the same i32-encoded offset
    // convention as Jmp; otherwise a conditional branch in a 100K-
    // instruction function would silently truncate.
    let jif = Instruction::JmpIf {
        cond: Reg(0),
        offset: 100_000_i32,
    };
    let jnot = Instruction::JmpNot {
        cond: Reg(0),
        offset: 100_000_i32,
    };

    match jif {
        Instruction::JmpIf { offset, .. } => {
            assert!(offset > i16::MAX as i32);
        }
        _ => unreachable!(),
    }
    match jnot {
        Instruction::JmpNot { offset, .. } => {
            assert!(offset > i16::MAX as i32);
        }
        _ => unreachable!(),
    }
}
