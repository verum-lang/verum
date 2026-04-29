//! Regression guard for the `[runtime].futures` and
//! `[runtime].nurseries` opt-out gates on the VBC interpreter
//! dispatch surface.
//!
//! Pin: when the user disables a feature via `verum.toml` the
//! corresponding dispatch handler must REJECT, not silently honour
//! the operation. Before the wire-up,
//! `InterpreterConfig.futures_enabled` and
//! `InterpreterConfig.nurseries_enabled` were inert — the pipeline
//! propagated the manifest values into the slots but the handlers
//! never consulted them, so opt-out was advisory at best.
//!
//! The check shape is the cheapest possible: a single boolean read
//! at the head of `handle_spawn` / `handle_nursery_init` before any
//! operand decode. Cost is one branch on the warm path; nothing on
//! the cold (denied) path past the message construction.

use std::sync::Arc;

use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::InterpreterError;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;

/// Build a single-function module whose body issues the requested
/// instructions. The function reserves four registers — enough for
/// the trivial test bodies' operands.
fn module_with_body(body: &[Instruction]) -> Arc<VbcModule> {
    let mut module = VbcModule::new("runtime_gate_test".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    let mut bytecode_bytes = Vec::new();
    for instr in body {
        bytecode::encode_instruction(instr, &mut bytecode_bytes);
    }
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode_bytes.len() as u32;
    func.register_count = 4;
    func.instructions = Some(body.to_vec());
    module.functions.push(func);
    module.bytecode = bytecode_bytes;
    Arc::new(module)
}

#[test]
fn spawn_rejected_when_futures_disabled() {
    let body = [
        Instruction::Spawn {
            dst: Reg(0),
            func_id: 0,
            args: RegRange { start: Reg(1), count: 0 },
        },
        Instruction::Ret { value: Reg(0) },
    ];
    let module = module_with_body(&body);
    let mut interp = Interpreter::new(module);
    interp.state.config.futures_enabled = false;

    let res = interp.execute_function(FunctionId(0));
    match res {
        Err(InterpreterError::Panic { ref message }) => {
            assert!(
                message.contains("futures disabled"),
                "panic message must name the disabled feature: {message}"
            );
        }
        other => panic!("expected Panic, got {other:?}"),
    }
}

#[test]
fn nursery_init_rejected_when_nurseries_disabled() {
    let body = [
        Instruction::NurseryInit { dst: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ];
    let module = module_with_body(&body);
    let mut interp = Interpreter::new(module);
    interp.state.config.nurseries_enabled = false;

    let res = interp.execute_function(FunctionId(0));
    match res {
        Err(InterpreterError::Panic { ref message }) => {
            assert!(
                message.contains("nurseries disabled"),
                "panic message must name the disabled feature: {message}"
            );
        }
        other => panic!("expected Panic, got {other:?}"),
    }
}

#[test]
fn spawn_passes_when_futures_enabled() {
    // Default config has futures_enabled = true; the gate must let
    // the operation through. The handler is upstream of any function
    // resolution work, so the trivial body can be a Spawn followed
    // by a Ret of the task-handle register — the spawn returns a
    // task-id sentinel that we don't otherwise inspect.
    let body = [
        Instruction::Spawn {
            dst: Reg(0),
            func_id: 0,
            args: RegRange { start: Reg(1), count: 0 },
        },
        Instruction::Ret { value: Reg(0) },
    ];
    let module = module_with_body(&body);
    let mut interp = Interpreter::new(module);
    assert!(interp.state.config.futures_enabled, "default must be on");

    // The handler succeeds (Spawn just records a deferred task).
    // Whether downstream Ret can resolve the sentinel value is
    // independent of the gate this test is pinning.
    let res = interp.execute_function(FunctionId(0));
    assert!(
        res.is_ok(),
        "spawn must pass under default futures-enabled config: {res:?}"
    );
}

#[test]
fn nursery_init_passes_when_nurseries_enabled() {
    let body = [
        Instruction::NurseryInit { dst: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ];
    let module = module_with_body(&body);
    let mut interp = Interpreter::new(module);
    assert!(interp.state.config.nurseries_enabled, "default must be on");

    let res = interp.execute_function(FunctionId(0));
    assert!(
        res.is_ok(),
        "nursery init must pass under default nurseries-enabled config: {res:?}"
    );
}
