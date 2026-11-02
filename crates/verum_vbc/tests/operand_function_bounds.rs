use std::sync::Arc;

use verum_vbc::instruction::Opcode;
use verum_vbc::interpreter::{Interpreter, InterpreterError};
use verum_vbc::module::{FunctionDescriptor, VbcModule};

/// Append a second executable body so an unbounded reader would have bytes
/// available past the first descriptor's declared end.
fn run(body: &[u8], following: &[u8]) -> Result<verum_vbc::value::Value, InterpreterError> {
    let mut module = VbcModule::new("operand_bounds".into());
    for (name, bytes) in [("main", body), ("adjacent", following)] {
        let mut function = FunctionDescriptor::new(module.intern_string(name));
        function.register_count = 2;
        function.bytecode_offset = module.append_bytecode(bytes);
        function.bytecode_length = bytes.len() as u32;
        module.add_function(function);
    }
    Interpreter::new(Arc::new(module)).run_main()
}

#[test]
fn truncated_operands_cannot_consume_the_next_function() {
    for (body, pc) in [
        (vec![Opcode::LoadTrue as u8, 0x80], 1),
        (vec![Opcode::LoadI as u8, 0, 0x80], 2),
        (vec![Opcode::LoadF as u8, 0, 0, 0, 0, 0, 0, 0, 0], 2),
    ] {
        assert!(matches!(
            run(&body, &[Opcode::RetV as u8]),
            Err(InterpreterError::InvalidBytecode { pc: at, .. }) if at == pc
        ));
    }
}

#[test]
fn interpreter_rejects_varint_bits_above_u64_and_preserves_full_width() {
    let load = |last| {
        let mut bytes = vec![Opcode::LoadI as u8, 0];
        bytes.extend_from_slice(&[0xff; 9]);
        bytes.extend_from_slice(&[last, Opcode::Ret as u8, 0]);
        bytes
    };
    // ZigZag(u64::MAX) is i64::MIN.
    assert_eq!(run(&load(1), &[]).unwrap().try_as_i64(), Some(i64::MIN));
    for last in [2, 3, 0x7f, 0x80, 0xff] {
        assert!(matches!(
            run(&load(last), &[]),
            Err(InterpreterError::InvalidBytecode { pc: 2, .. })
        ));
    }
}

#[test]
fn fixed_width_decoders_reject_an_unrepresentable_end_offset() {
    let mut offset = usize::MAX;
    assert!(verum_vbc::encoding::decode_u16(&[0], &mut offset).is_err());
    assert!(verum_vbc::encoding::decode_f64(&[0], &mut offset).is_err());
    assert_eq!(offset, usize::MAX);
}

#[test]
fn extended_lengths_must_fit_the_enclosing_function_in_both_paths() {
    use verum_vbc::instruction::MathSubOpcode;
    for length in [4, u64::from(u32::MAX) + 1, u64::MAX] {
        let mut body = vec![Opcode::MathExtended as u8, MathSubOpcode::SinF64 as u8];
        verum_vbc::encoding::encode_varint(length, &mut body);
        body.extend_from_slice(&[0, 1]);
        assert!(verum_vbc::bytecode::decode_instruction(&body, &mut 0).is_err());
        assert!(matches!(
            run(&body, &[0; 8]),
            Err(InterpreterError::InvalidBytecode { pc: 2, .. })
        ));
    }
}
