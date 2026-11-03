//! The profiler measures the interpreter handler actually invoked by the bytecode.
use std::sync::Arc;
use verum_common::runtime_diagnostics::MethodDispatchOutcome;
use verum_vbc::{
    FunctionId, Instruction, Reg, RegRange, VbcModule, bytecode::encode_instruction,
    interpreter::Interpreter, module::FunctionDescriptor,
};

fn fixture() -> (Interpreter, FunctionId, FunctionId, u32) {
    let mut module = VbcModule::new("meter".into());
    fn add(module: &mut VbcModule, name: &str, selector: &str) -> (FunctionId, u32) {
        let name = module.intern_string(name);
        let method_id = module.intern_string(selector).0;
        let start = module.bytecode.len() as u32;
        encode_instruction(
            &Instruction::LoadI {
                dst: Reg(0),
                value: 17,
            },
            &mut module.bytecode,
        );
        let pc = module.bytecode.len() as u32 - start;
        encode_instruction(
            &Instruction::CallM {
                dst: Reg(1),
                receiver: Reg(0),
                method_id,
                args: RegRange::new(Reg(0), 0),
            },
            &mut module.bytecode,
        );
        encode_instruction(&Instruction::Ret { value: Reg(1) }, &mut module.bytecode);
        let id = FunctionId(module.functions.len() as u32);
        let len = module.bytecode.len() as u32 - start;
        (
            module.add_function(FunctionDescriptor {
                name,
                id,
                bytecode_offset: start,
                bytecode_length: len,
                register_count: 2,
                ..Default::default()
            }),
            pc,
        )
    }
    let (ok, pc) = add(&mut module, "positive", "clone");
    let (bad, _) = add(&mut module, "negative", "absent_method");
    (Interpreter::new(Arc::new(module)), ok, bad, pc)
}

#[test]
fn captures_the_selector_actual_call_pc_and_outcome_with_bounded_counts() {
    let (mut vm, ok, bad, pc) = fixture();
    assert_eq!(vm.execute_function(ok).unwrap().as_i64(), 17);
    assert!(vm.state.method_dispatch_report().is_none());
    vm.state.enable_method_dispatch_profile(1).unwrap();
    for _ in 0..2 {
        assert_eq!(vm.execute_function(ok).unwrap().as_i64(), 17);
    }
    assert!(
        vm.execute_function(bad)
            .unwrap_err()
            .to_string()
            .contains("absent_method")
    );
    assert_eq!(vm.execute_function(ok).unwrap().as_i64(), 17);
    let report = vm.state.method_dispatch_report().unwrap();
    assert!(report.validate());
    assert_eq!(report.dropped_calls, 1);
    assert_eq!(report.rows.len(), 1);
    let row = &report.rows[0];
    assert_eq!(
        (
            row.function_id,
            row.function.as_deref(),
            row.pc,
            row.selector.as_deref()
        ),
        (ok.0, Some("positive"), pc, Some("clone"))
    );
    assert_eq!(row.outcome, MethodDispatchOutcome::CompletedInline);
    assert_eq!(row.count, 3);
    assert!(row.inclusive_ns >= row.max_ns);
    vm.state.enable_method_dispatch_profile(1).unwrap();
    assert!(vm.execute_function(bad).is_err());
    let report = vm.state.method_dispatch_report().unwrap();
    assert_eq!(report.rows[0].outcome, MethodDispatchOutcome::Failed);
    assert_eq!(report.rows[0].selector.as_deref(), Some("absent_method"));
    assert_eq!(report.rows[0].count, 1);
    assert_eq!(report.dropped_calls, 0);
    vm.state.enable_method_dispatch_profile(0).unwrap();
    assert_eq!(vm.execute_function(ok).unwrap().as_i64(), 17);
    let report = vm.state.method_dispatch_report().unwrap();
    assert!(report.rows.is_empty());
    assert_eq!(report.dropped_calls, 1);
    assert!(report.validate());
    vm.state.reset();
    assert!(vm.state.method_dispatch_report().is_none());
}

#[test]
fn a_receiver_bound_call_records_the_callee_and_the_adjacent_call_opcode() {
    use verum_vbc::{
        module::ParamDescriptor,
        types::{TypeId, TypeRef},
    };
    let mut module = VbcModule::new("packet".into());
    let method = module.intern_string("Int.measured_read");
    let self_name = module.intern_string("self");
    let callee = FunctionId(0);
    encode_instruction(
        &Instruction::LoadI {
            dst: Reg(0),
            value: 91,
        },
        &mut module.bytecode,
    );
    encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut module.bytecode);
    module.add_function(FunctionDescriptor {
        id: callee,
        name: method,
        register_count: 1,
        parent_type: Some(TypeId::INT),
        params: [ParamDescriptor {
            name: self_name,
            type_ref: TypeRef::Concrete(TypeId::INT),
            ..Default::default()
        }]
        .into_iter()
        .collect(),
        bytecode_length: module.bytecode.len() as u32,
        ..Default::default()
    });
    let start = module.bytecode.len() as u32;
    encode_instruction(
        &Instruction::LoadI {
            dst: Reg(0),
            value: 5,
        },
        &mut module.bytecode,
    );
    encode_instruction(
        &Instruction::BindMethodReceiver {
            type_ref: TypeRef::Concrete(TypeId::INT),
        },
        &mut module.bytecode,
    );
    let pc = module.bytecode.len() as u32 - start;
    encode_instruction(
        &Instruction::CallM {
            dst: Reg(1),
            receiver: Reg(0),
            method_id: method.0,
            args: RegRange::new(Reg(0), 0),
        },
        &mut module.bytecode,
    );
    encode_instruction(&Instruction::Ret { value: Reg(1) }, &mut module.bytecode);
    let name = module.intern_string("main");
    let entry = module.add_function(FunctionDescriptor {
        id: FunctionId(1),
        name,
        register_count: 2,
        bytecode_offset: start,
        bytecode_length: module.bytecode.len() as u32 - start,
        ..Default::default()
    });
    let mut vm = Interpreter::new(Arc::new(module));
    vm.state.enable_method_dispatch_profile(1).unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 91);
    let report = vm.state.method_dispatch_report().unwrap();
    let row = &report.rows[0];
    assert_eq!(row.pc, pc);
    assert_eq!(
        row.outcome,
        MethodDispatchOutcome::EnteredFrame {
            function_id: callee.0
        }
    );
}
