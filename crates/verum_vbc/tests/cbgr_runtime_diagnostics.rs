//! Runtime observations must originate in the actual dispatcher, and missing
//! coverage must never be presented as a successful memory-safety check.
use std::sync::Arc;
use verum_common::{List, runtime_diagnostics::CbgrTargetKind};
use verum_vbc::{
    bytecode,
    instruction::{Instruction, Opcode, Reg},
    interpreter::{
        CbgrTarget, CbgrViolationKind, Interpreter, InterpreterError, diagnostics::MAX_CBGR_EVENTS,
    },
    module::{FunctionDescriptor, FunctionId, SourceMap, SourceMapEntry, VbcModule},
};

fn stale_reference(exact_source: bool) -> (Arc<VbcModule>, u32) {
    let mut module = VbcModule::new("cbgr_diagnostic_test".into());
    let name = module.intern_string("stale_reference");
    let path = module.intern_string("example.vr");
    let instructions = [
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 7,
        },
        Instruction::Ref {
            dst: Reg(1),
            src: Reg(0),
        },
        Instruction::DropRef { src: Reg(0) },
    ];
    for instruction in instructions {
        bytecode::encode_instruction(&instruction, &mut module.bytecode);
    }
    let fault_pc = module.bytecode.len() as u32;
    bytecode::encode_instruction(
        &Instruction::Deref {
            dst: Reg(2),
            ref_reg: Reg(1),
        },
        &mut module.bytecode,
    );
    bytecode::encode_instruction(&Instruction::Ret { value: Reg(2) }, &mut module.bytecode);
    let mut function = FunctionDescriptor::new(name);
    function.id = FunctionId(0);
    function.bytecode_length = module.bytecode.len() as u32;
    function.register_count = 4;
    module.functions.push(function);
    module.source_map = Some(SourceMap {
        files: vec![path],
        entries: vec![SourceMapEntry {
            bytecode_offset: if exact_source { fault_pc } else { 0 },
            file_idx: 0,
            line: 4,
            column: 3,
        }],
    });
    (Arc::new(module), fault_pc)
}

#[test]
fn actual_stale_reference_has_structured_target_and_exact_dispatch_location() {
    let (module, pc) = stale_reference(true);
    let mut interpreter = Interpreter::new(module);
    interpreter.state.enable_cbgr_diagnostics(8).unwrap();
    let error = interpreter.execute_function(FunctionId(0)).unwrap_err();
    let InterpreterError::CbgrPanic { failure, message } = error else {
        panic!("actual stale borrow must fail")
    };
    assert_eq!(failure.kind, CbgrViolationKind::GenerationMismatch);
    assert!(matches!(failure.target, CbgrTarget::Register(_)));
    assert_ne!(failure.expected, failure.actual);
    assert!(message.contains("CBGR use-after-free detected"));
    let report = interpreter.state.cbgr_report().unwrap();
    assert!(!report.truncated);
    assert_eq!(report.events.len(), 1);
    let event = &report.events[0];
    assert_eq!(event.kind, failure.kind);
    assert_eq!(event.target_kind, CbgrTargetKind::Register);
    assert_eq!(event.pc, pc);
    assert_eq!(event.opcode, Opcode::Deref as u8);
    assert_eq!(event.function_id, Some(0));
    assert_eq!(event.function.as_ref().unwrap().as_str(), "stale_reference");
    let source = event.source.as_ref().unwrap();
    assert_eq!(
        (source.path.as_str(), source.line, source.column),
        ("example.vr", 4, 3)
    );
}

#[test]
fn disabled_capture_preserves_failure_and_cannot_return_a_clean_report() {
    let (module, _) = stale_reference(true);
    let mut interpreter = Interpreter::new(module);
    assert!(matches!(
        interpreter.execute_function(FunctionId(0)),
        Err(InterpreterError::CbgrPanic { .. })
    ));
    assert!(interpreter.state.cbgr_report().is_none());
}

#[test]
fn exhausted_capture_reports_loss_and_reset_removes_prior_run_evidence() {
    let (module, _) = stale_reference(true);
    let mut interpreter = Interpreter::new(module);
    interpreter.state.enable_cbgr_diagnostics(0).unwrap();
    assert!(interpreter.execute_function(FunctionId(0)).is_err());
    let report = interpreter.state.cbgr_report().unwrap();
    assert!(report.truncated);
    assert!(report.events.is_empty());
    interpreter.reset();
    assert!(interpreter.state.cbgr_report().is_none());
    interpreter
        .state
        .enable_cbgr_diagnostics(usize::MAX)
        .unwrap();
    assert!(interpreter.state.cbgr_report().is_none());
    assert!(interpreter.execute_function(FunctionId(0)).is_err());
    let fresh = interpreter.state.cbgr_report().unwrap();
    assert_eq!(fresh.capacity, MAX_CBGR_EVENTS);
    assert!(!fresh.truncated);
    assert_eq!(fresh.events.len(), 1);
    interpreter.state.enable_cbgr_diagnostics(1).unwrap();
    assert!(interpreter.state.cbgr_report().is_none());
}

#[test]
fn source_location_is_not_borrowed_from_a_different_instruction() {
    let (module, _) = stale_reference(false);
    let mut interpreter = Interpreter::new(module);
    interpreter.state.enable_cbgr_diagnostics(1).unwrap();
    assert!(interpreter.execute_function(FunctionId(0)).is_err());
    let report = interpreter.state.cbgr_report().unwrap();
    assert_eq!(report.events.len(), 1);
    assert!(report.events[0].source.is_none());
}

#[test]
fn fallible_event_reservation_preserves_existing_elements() {
    let mut values = List::new();
    values.push(7u64);
    assert!(values.try_reserve_exact(usize::MAX).is_err());
    assert_eq!(values.as_slice(), &[7]);
    assert!(values.try_reserve(usize::MAX).is_err());
    assert_eq!(values.as_slice(), &[7]);
}

#[test]
fn capture_reports_disabled_checks_without_inventing_validation_coverage() {
    let (module, _) = stale_reference(false);
    let mut interpreter = Interpreter::new(module);
    interpreter.state.config.cbgr_enabled = false;
    interpreter.state.enable_cbgr_diagnostics(4).unwrap();
    let _ = interpreter.execute_function(FunctionId(0));
    let report = interpreter.state.cbgr_report().unwrap();
    assert!(!report.checks_enabled_at_start);
    assert!(report.events.is_empty());
}

#[test]
fn missing_entry_is_unavailable_not_an_empty_capture() {
    let (module, _) = stale_reference(false);
    let mut interpreter = Interpreter::new(module);
    interpreter.state.enable_cbgr_diagnostics(4).unwrap();
    assert!(interpreter.execute_function(FunctionId(999)).is_err());
    assert!(interpreter.state.cbgr_report().is_none());
}

#[test]
fn a_caught_cbgr_panic_still_has_runtime_evidence() {
    let (module, _) = stale_reference(false);
    let mut module = Arc::try_unwrap(module).unwrap();
    let mut wrapped = Default::default();
    // TryBegin offsets are relative to the PC after its operand. No fixed
    // opcode width is assumed: the handler follows the complete old body.
    bytecode::encode_instruction(
        &Instruction::TryBegin {
            handler_offset: module.bytecode.len().try_into().unwrap(),
        },
        &mut wrapped,
    );
    wrapped.extend_from_slice(&module.bytecode);
    bytecode::encode_instruction(
        &Instruction::LoadSmallI {
            dst: Reg(3),
            value: 9,
        },
        &mut wrapped,
    );
    bytecode::encode_instruction(&Instruction::Ret { value: Reg(3) }, &mut wrapped);
    module.bytecode = wrapped;
    module.functions[0].bytecode_length = module.bytecode.len() as u32;
    module.source_map = None;
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.state.enable_cbgr_diagnostics(4).unwrap();
    assert_eq!(
        interpreter
            .execute_function(FunctionId(0))
            .unwrap()
            .as_i64(),
        9
    );
    assert!(interpreter.state.exception_handlers.is_empty());
    let report = interpreter.state.cbgr_report().unwrap();
    assert_eq!(report.events.len(), 1);
    assert_eq!(report.events[0].kind, CbgrViolationKind::GenerationMismatch);
}

#[test]
fn conflicting_source_entries_never_choose_an_arbitrary_editor_location() {
    let (module, pc) = stale_reference(true);
    let mut module = Arc::try_unwrap(module).unwrap();
    module
        .source_map
        .as_mut()
        .unwrap()
        .entries
        .push(SourceMapEntry {
            bytecode_offset: pc,
            file_idx: 0,
            line: 900,
            column: 1,
        });
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.state.enable_cbgr_diagnostics(4).unwrap();
    assert!(interpreter.execute_function(FunctionId(0)).is_err());
    assert!(
        interpreter.state.cbgr_report().unwrap().events[0]
            .source
            .is_none()
    );
}
