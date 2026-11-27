//! A physical slot cannot keep a borrow alive after its value has moved out.
use verum_vbc::{
    bytecode::encode_instruction,
    deserialize::deserialize_module,
    instruction::{Instruction as I, MemSubOpcode as M, Reg},
    interpreter::{Interpreter, InterpreterError},
    module::{FunctionDescriptor, FunctionId, VbcModule},
    serialize::serialize_module,
};

#[test]
fn replacing_a_slot_revokes_both_shared_and_mutable_element_tickets() {
    for borrow in [M::DequeStorageRef, M::DequeStorageRefMut] {
        let mut module = VbcModule::new("ring_revocation".into());
        let mut function = FunctionDescriptor::new(module.strings.intern("probe"));
        function.register_count = 7;
        let memory = |op: M, registers: &[u8]| I::MemExtended {
            sub_op: op.to_byte(),
            operands: registers.into(),
        };
        for instruction in [
            I::NewDeque {
                dst: Reg(0),
                capacity_hint: 4,
            },
            I::LoadI {
                dst: Reg(1),
                value: 0,
            },
            I::LoadI {
                dst: Reg(2),
                value: 41,
            },
            memory(M::DequeStorageWrite, &[3, 0, 1, 2]),
            I::LoadI {
                dst: Reg(2),
                value: 1,
            },
            I::SetF {
                obj: Reg(0),
                field_idx: 2,
                value: Reg(2),
            },
            memory(borrow, &[4, 0, 1]),
            memory(M::DequeStorageTake, &[5, 0, 1]),
            memory(M::DequeStorageWrite, &[3, 0, 1, 2]),
            I::Deref {
                dst: Reg(6),
                ref_reg: Reg(4),
            },
            I::Ret { value: Reg(6) },
        ] {
            encode_instruction(&instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32;
        module.functions.push(function);
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let error = Interpreter::try_new_validated(wire.into())
            .unwrap()
            .execute_function(FunctionId(0))
            .unwrap_err();
        assert!(
            matches!(error, InterpreterError::CbgrViolation { .. }),
            "{error:?}"
        );
    }
}
