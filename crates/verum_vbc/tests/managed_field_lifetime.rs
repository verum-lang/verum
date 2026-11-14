//! Field references are tied to their exact allocation, not a call frame or address.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    bytecode::{decode_instructions, encode_instruction},
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    encoding::{decode_reg, decode_varint, encode_reg, encode_varint},
    instruction::{CbgrSubOpcode, Instruction},
    interpreter::{Interpreter, InterpreterError},
    serialize::serialize_module,
    types::TypeId,
    validate::validate_module,
    value::Value,
};

fn program(named: bool) -> (Interpreter, TypeId) {
    let source = "type Record is { first: Int, second: Int };\n\
        fn first(record: &Record) -> &Int { &record.first }\n\
        fn second(record: &mut Record) -> &mut Int { &mut record.second }\n\
        fn read(field: &Int) -> Int { *field }\n\
        fn write(field: &mut Int) { *field = 99; }";
    let ast = Parser::new(source).parse_module().unwrap();
    let mut module = VbcCodegen::new().compile_module(&ast).unwrap();
    let owner = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Record"))
        .unwrap()
        .id;
    if named {
        let names = [
            module.intern_string("first"),
            module.intern_string("second"),
        ];
        let original = std::mem::take(&mut module.bytecode);
        for function in &mut module.functions {
            let start = function.bytecode_offset as usize;
            let instructions =
                decode_instructions(&original[start..start + function.bytecode_length as usize])
                    .unwrap();
            function.bytecode_offset = module.bytecode.len() as u32;
            for mut instruction in instructions {
                if let Instruction::CbgrExtended { sub_op, operands } = &mut instruction
                    && *sub_op == CbgrSubOpcode::RefField as u8
                {
                    let mut offset = 0;
                    let dst = decode_reg(operands, &mut offset).unwrap();
                    let base = decode_reg(operands, &mut offset).unwrap();
                    let field = decode_varint(operands, &mut offset).unwrap() as usize;
                    operands.clear();
                    encode_reg(dst, operands);
                    encode_reg(base, operands);
                    encode_varint(u64::from(names[field].0), operands);
                    *sub_op = CbgrSubOpcode::RefFieldNamed as u8;
                }
                encode_instruction(&instruction, &mut module.bytecode);
            }
            function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        }
    }
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&module).unwrap();
    (Interpreter::new(module.into()), owner)
}

fn call(vm: &mut Interpreter, name: &str, argument: Value) -> Result<Value, InterpreterError> {
    let function = vm.state.module.find_function_by_name(name).unwrap();
    vm.execute_function_with_args(function, &[argument])
}

#[test]
fn sibling_fields_survive_their_producing_frames_and_preserve_capabilities() {
    for named in [false, true] {
        let (mut vm, owner) = program(named);
        let object = vm
            .state
            .heap
            .alloc(owner, 2 * std::mem::size_of::<Value>())
            .unwrap();
        let slots = object.data_ptr().cast::<Value>();
        // SAFETY: the new allocation contains these two initialized Value slots.
        unsafe {
            slots.write(Value::from_i64(31));
            slots.add(1).write(Value::from_i64(42));
        }
        let record = Value::from_ptr(object.as_ptr().cast::<u8>());
        let first = call(&mut vm, "first", record).unwrap();
        let second = call(&mut vm, "second", record).unwrap();
        assert_eq!(call(&mut vm, "read", first).unwrap().try_as_i64(), Some(31));
        assert_eq!(
            call(&mut vm, "read", second).unwrap().try_as_i64(),
            Some(42)
        );
        call(&mut vm, "write", second).unwrap();
        assert_eq!(
            call(&mut vm, "read", second).unwrap().try_as_i64(),
            Some(99)
        );
        assert!(matches!(
            call(&mut vm, "write", first),
            Err(InterpreterError::CapabilityDenied { .. })
        ));
        // SAFETY: the exact owner remains live and the VM calls have completed.
        assert_eq!(unsafe { slots.read() }.try_as_i64(), Some(31));
    }
}

#[test]
fn freed_owner_cannot_be_revived_by_a_new_allocation() {
    for named in [false, true] {
        let (mut vm, owner) = program(named);
        let object = vm
            .state
            .heap
            .alloc(owner, 2 * std::mem::size_of::<Value>())
            .unwrap();
        // SAFETY: both Value fields fit in the allocation.
        unsafe {
            object.data_ptr().cast::<Value>().write(Value::from_i64(31));
            object
                .data_ptr()
                .cast::<Value>()
                .add(1)
                .write(Value::from_i64(42));
        }
        let record = Value::from_ptr(object.as_ptr().cast::<u8>());
        let field = call(&mut vm, "second", record).unwrap();
        // SAFETY: deliberately retire the exact owner to test runtime refusal.
        unsafe {
            vm.state.heap.free(object);
        }
        let replacement = vm
            .state
            .heap
            .alloc(owner, 2 * std::mem::size_of::<Value>())
            .unwrap();
        // SAFETY: both replacement fields fit in the separate live allocation.
        unsafe {
            replacement
                .data_ptr()
                .cast::<Value>()
                .write(Value::from_i64(7));
            replacement
                .data_ptr()
                .cast::<Value>()
                .add(1)
                .write(Value::from_i64(73));
        }
        for operation in ["read", "write"] {
            let error =
                call(&mut vm, operation, field).expect_err("a retired owner cannot grant access");
            assert!(
                matches!(error, InterpreterError::CbgrViolation { .. }),
                "{operation}: {error:?}"
            );
        }
        // SAFETY: the replacement remains live, and rejected access cannot mutate it.
        assert_eq!(
            unsafe { replacement.data_ptr().cast::<Value>().add(1).read() }.try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn a_readonly_owned_field_cannot_authorize_an_object_write() {
    let source = "type Inner is { value: Int }; type Outer is { inner: Inner };\n\
        fn borrow(owner: &Outer) -> &Inner { &owner.inner }\n\
        fn mutate(value: &mut Inner) { value.value = 99; }";
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&module).unwrap();
    let inner_id = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Inner"))
        .unwrap()
        .id;
    let outer_id = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Outer"))
        .unwrap()
        .id;
    let mut vm = Interpreter::new(module.into());
    let inner = vm
        .state
        .heap
        .alloc(inner_id, std::mem::size_of::<Value>())
        .unwrap();
    let outer = vm
        .state
        .heap
        .alloc(outer_id, std::mem::size_of::<Value>())
        .unwrap();
    // SAFETY: both exact allocations own one initialized Value slot.
    unsafe {
        inner.data_ptr().cast::<Value>().write(Value::from_i64(73));
        outer
            .data_ptr()
            .cast::<Value>()
            .write(Value::from_ptr(inner.as_ptr().cast::<u8>()));
    }
    let borrow = call(
        &mut vm,
        "borrow",
        Value::from_ptr(outer.as_ptr().cast::<u8>()),
    )
    .unwrap();
    let error = call(&mut vm, "mutate", borrow)
        .expect_err("a forged caller cannot strengthen a readonly field");
    assert!(
        matches!(error, InterpreterError::CapabilityDenied { .. }),
        "{error:?}"
    );
    // SAFETY: the rejected write cannot mutate or free either live owner.
    assert_eq!(
        unsafe { inner.data_ptr().cast::<Value>().read() }.try_as_i64(),
        Some(73)
    );
}
