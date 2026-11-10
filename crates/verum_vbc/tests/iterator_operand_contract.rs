//! Native iteration rejects operands without the corresponding runtime representation.
use std::sync::Arc;
use verum_vbc::{
    bytecode,
    instruction::{Instruction, Opcode, Reg},
    interpreter::{Interpreter, InterpreterError},
    module::{Constant, FunctionDescriptor, FunctionId, VbcModule},
    types::{StringId, TypeId},
    value::Value,
};

fn execute(instructions: &[Instruction]) -> Result<Value, InterpreterError> {
    let mut module = VbcModule::new("iteration".into());
    for text in ["é", "long UTF-8 λ"] {
        let value = module.strings.intern(text);
        module.constants.push(Constant::String(value));
    }
    for instruction in instructions {
        bytecode::encode_instruction(instruction, &mut module.bytecode);
    }
    let mut function = FunctionDescriptor::new(StringId::EMPTY);
    function.id = FunctionId(0);
    function.register_count = 8;
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    Interpreter::new(Arc::new(module)).execute_function(FunctionId(0))
}

#[test]
fn iter_new_rejects_an_arbitrary_record_and_a_scalar() {
    for source in [
        Instruction::New {
            dst: Reg(0),
            type_id: TypeId::FIRST_USER,
            field_count: 1,
        },
        Instruction::LoadI {
            dst: Reg(0),
            value: 17,
        },
    ] {
        let result = execute(&[
            source,
            Instruction::IterNew {
                dst: Reg(1),
                iterable: Reg(0),
            },
            Instruction::Ret { value: Reg(1) },
        ]);
        assert!(
            matches!(result, Err(InterpreterError::InvalidOperand { ref message }) if message.contains("IterNew")),
            "{result:?}"
        );
    }
}

#[test]
fn iter_next_rejects_an_unrelated_record_of_the_same_size() {
    let result = execute(&[
        Instruction::New {
            dst: Reg(0),
            type_id: TypeId::UNIT.0,
            field_count: 3,
        },
        Instruction::IterNext {
            dst: Reg(1),
            has_next: Reg(2),
            iter: Reg(0),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(
        matches!(result, Err(InterpreterError::InvalidOperand { ref message }) if message.contains("IterNext")),
        "{result:?}"
    );
}

#[test]
fn value_arrays_iterate_from_the_first_element_to_the_allocation_end() {
    let values = execute(&[
        Instruction::LoadI {
            dst: Reg(0),
            value: 73,
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Raw {
            opcode: Opcode::NewArray,
            data: vec![2, 0, 1],
        },
        Instruction::LoadI {
            dst: Reg(0),
            value: 85,
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 1,
        },
        Instruction::SetE {
            arr: Reg(2),
            idx: Reg(1),
            value: Reg(0),
        },
        Instruction::IterNew {
            dst: Reg(3),
            iterable: Reg(2),
        },
        Instruction::IterNext {
            dst: Reg(4),
            has_next: Reg(1),
            iter: Reg(3),
        },
        Instruction::IterNext {
            dst: Reg(5),
            has_next: Reg(1),
            iter: Reg(3),
        },
        Instruction::IterNext {
            dst: Reg(0),
            has_next: Reg(6),
            iter: Reg(3),
        },
        Instruction::BinaryI {
            op: verum_vbc::instruction::BinaryIntOp::Sub,
            dst: Reg(7),
            a: Reg(5),
            b: Reg(4),
        },
        Instruction::Ret { value: Reg(7) },
    ])
    .unwrap();
    assert_eq!(values.try_as_i64(), Some(12));
}

#[test]
fn text_iteration_uses_utf8_bytes_in_each_storage_form() {
    for (constant, first) in [(0, 0xc3), (1, i64::from(b'l'))] {
        let value = execute(&[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: constant,
            },
            Instruction::IterNew {
                dst: Reg(1),
                iterable: Reg(0),
            },
            Instruction::IterNext {
                dst: Reg(2),
                has_next: Reg(3),
                iter: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ])
        .unwrap();
        assert_eq!(value.try_as_i64(), Some(first));
    }
}

#[test]
fn field_stores_cannot_change_private_cursor_state() {
    let result = execute(&[
        Instruction::LoadK {
            dst: Reg(0),
            const_id: 0,
        },
        Instruction::IterNew {
            dst: Reg(1),
            iterable: Reg(0),
        },
        Instruction::SetF {
            obj: Reg(1),
            field_idx: 0,
            value: Reg(0),
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(
        matches!(result, Err(InterpreterError::InvalidOperand { ref message }) if message.contains("SetF"))
    );
}
