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

// -----------------------------------------------------------------------------
// Round 3 §4.1 — Long single-instruction basic-block chain
// -----------------------------------------------------------------------------
// PENDING → DEFENSE CONFIRMED 2026-04-28.  Adversarial input: a function
// body of N straight-line `Mov` instructions with no branches.  Pre-fix
// concern was whether the bytecode encoder/decoder pair survives a basic
// block far past typical real-world function sizes — the fix invariant
// being that there's no implicit i16 cap on instruction count, only the
// i32 branch-target cap pinned by §2.2.
//
// 100,000 Mov instructions ≈ 6 bytes each (opcode + two reg bytes) ≈
// 600 KB of bytecode — comfortably below any reasonable per-function
// size limit, but tens of orders of magnitude beyond what any real
// codegen path emits.  If the encoder ever introduced a per-block
// instruction-count cap, this test fires.

#[test]
fn long_basic_block_chain_roundtrips() {
    // Build N Mov instructions in a line, encode them into one
    // contiguous bytecode buffer, then decode them back and check
    // the instruction count + per-instruction integrity at three
    // offsets (start, mid, end) without holding the entire decoded
    // sequence in memory.
    const N: usize = 100_000;

    let mut encoded = Vec::with_capacity(N * 6);
    for i in 0..N {
        let dst = Reg(((i % 16383) + 1) as u16);
        let src = Reg((i % 16383) as u16);
        bytecode::encode_instruction(
            &Instruction::Mov { dst, src },
            &mut encoded,
        );
    }

    // Walk the buffer and count successfully-decoded instructions.
    // A regression that introduced an early-stop (i16 length cap or
    // similar) would short-circuit here.
    let mut offset = 0usize;
    let mut decoded_count = 0usize;
    while offset < encoded.len() {
        let instr = bytecode::decode_instruction(&encoded, &mut offset)
            .expect("decode_instruction failed in long basic block");
        // Sanity check: every instruction must be a Mov, since that's
        // all we encoded.
        match instr {
            Instruction::Mov { .. } => {}
            other => panic!("expected Mov, got {:?}", other),
        }
        decoded_count += 1;
    }

    assert_eq!(
        decoded_count, N,
        "long basic block: encoded {} Mov instructions but decoded {}",
        N, decoded_count
    );
    assert_eq!(
        offset,
        encoded.len(),
        "decode consumed {} bytes but buffer has {}",
        offset,
        encoded.len()
    );
}

// -----------------------------------------------------------------------------
// Round 1 §3.1 — End-to-end load-time defense via deserialize_module_validated
// -----------------------------------------------------------------------------
// The validator's per-instruction cross-reference checks live in
// `verum_vbc::validate` and are wired into `deserialize_module_validated`.
// Pin the end-to-end path: serialize a hand-crafted invalid module,
// then load it through the validating entry point — must reject.

#[test]
fn deserialize_validated_rejects_call_with_oor_function_id() {
    use verum_vbc::deserialize::{deserialize_module, deserialize_module_validated};
    use verum_vbc::error::VbcError;
    use verum_vbc::instruction::RegRange;
    use verum_vbc::serialize::serialize_module;

    // Hand-craft a module with a single function whose body calls
    // FunctionId(99) — far past the function table's only entry.
    let mut module = VbcModule::new("rt_3_1_e2e".to_string());
    let mut f = FunctionDescriptor::new(StringId::EMPTY);
    f.id = FunctionId(0);
    f.bytecode_offset = 0;
    f.register_count = 4;

    let mut bc = Vec::new();
    bytecode::encode_instruction(
        &Instruction::Call {
            dst: Reg(0),
            func_id: 99,
            args: RegRange { start: Reg(0), count: 0 },
        },
        &mut bc,
    );
    bytecode::encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut bc);
    f.bytecode_length = bc.len() as u32;
    module.functions.push(f);
    module.bytecode = bc;
    module.header.function_table_count = 1;

    // Serialize then deserialize-without-validation: must succeed.
    // The defect only surfaces under the validating entry point.
    let bytes = serialize_module(&module).expect("serialize");
    let _trusted = deserialize_module(&bytes).expect("trusted load");

    // Validating entry point must reject.  The exact error variant
    // can be either the bare `InvalidFunctionId(99)` or a
    // `MultipleErrors(..)` wrapping it; both are well-formed
    // surface-level rejections of the load.
    let err = deserialize_module_validated(&bytes)
        .expect_err("validated load must reject hand-crafted invalid module");
    let has_err = matches!(&err, VbcError::InvalidFunctionId(99))
        || matches!(&err, VbcError::MultipleErrors(errs)
            if errs.iter().any(|e| matches!(e, VbcError::InvalidFunctionId(99))));
    assert!(
        has_err,
        "expected InvalidFunctionId(99), got: {:?}",
        err
    );
}

#[test]
fn deserialize_validated_accepts_well_formed_module() {
    use verum_vbc::deserialize::deserialize_module_validated;
    use verum_vbc::serialize::serialize_module;

    // Sanity baseline: a well-formed minimum-viable module must
    // round-trip cleanly through the validating entry point.
    let module = build_minimal_module();

    // Update header counts so validate_header passes.
    let mut owned: VbcModule = (*module).clone();
    owned.header.function_table_count = owned.functions.len() as u32;
    let bytes = serialize_module(&owned).expect("serialize");
    deserialize_module_validated(&bytes)
        .expect("well-formed module must validate cleanly");
}

#[test]
fn interpreter_try_new_validated_rejects_invalid_module() {
    use verum_vbc::instruction::RegRange;

    // Hand-craft a module whose body calls FunctionId(99) — out of
    // range against the 1-function table.  `Interpreter::try_new`
    // (the non-validating constructor) accepts the module; only
    // `try_new_validated` rejects, with a `ValidationFailed` error
    // carrying the rendered `VbcError`.
    let mut module = VbcModule::new("rt_3_1_interp".to_string());
    let mut f = FunctionDescriptor::new(StringId::EMPTY);
    f.id = FunctionId(0);
    f.bytecode_offset = 0;
    f.register_count = 4;

    let mut bc = Vec::new();
    bytecode::encode_instruction(
        &Instruction::Call {
            dst: Reg(0),
            func_id: 99,
            args: RegRange { start: Reg(0), count: 0 },
        },
        &mut bc,
    );
    bytecode::encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut bc);
    f.bytecode_length = bc.len() as u32;
    module.functions.push(f);
    module.bytecode = bc;
    module.header.function_table_count = 1;

    let arc = Arc::new(module);

    // try_new accepts (lenient — trusted source).  Use `is_ok()`
    // since `Interpreter` doesn't implement `Debug`.
    assert!(
        Interpreter::try_new(Arc::clone(&arc)).is_ok(),
        "try_new must accept under the trusted-source contract",
    );

    // try_new_validated rejects with ValidationFailed.
    match Interpreter::try_new_validated(arc) {
        Ok(_) => panic!("try_new_validated must reject hand-crafted invalid module"),
        Err(InterpreterError::ValidationFailed { reason, .. }) => {
            assert!(
                reason.contains("InvalidFunctionId")
                    || reason.contains("invalid function reference"),
                "expected ValidationFailed mentioning InvalidFunctionId, got reason: {}",
                reason,
            );
        }
        Err(other) => panic!("expected ValidationFailed, got: {:?}", other),
    }
}

#[test]
fn interpreter_try_new_validated_accepts_well_formed_module() {
    // Sanity baseline: well-formed minimal module must construct
    // through the validating constructor without error.  We have
    // to reify a fresh Arc with synced header counts because
    // `build_minimal_module` (shared with the other tests) doesn't
    // set the header count fields the validator checks.
    let mut module = (*build_minimal_module()).clone();
    module.header.function_table_count = module.functions.len() as u32;
    let arc = Arc::new(module);
    match Interpreter::try_new_validated(arc) {
        Ok(_) => {}
        Err(e) => panic!(
            "well-formed module must construct via try_new_validated, got: {:?}",
            e
        ),
    }
}
