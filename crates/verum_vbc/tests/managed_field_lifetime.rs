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

fn name_field_projections(module: &mut verum_vbc::VbcModule, fields: &[&str]) {
    let names: verum_common::List<_> = fields.iter().map(|name| module.intern_string(name)).collect();
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
            if let Instruction::GetF { dst, obj, field_idx } = instruction {
                instruction = Instruction::GetFieldNamed {
                    dst,
                    obj,
                    name: names[field_idx as usize].0,
                };
            }
            if let Instruction::SetF { obj, field_idx, value } = instruction {
                instruction = Instruction::SetFieldNamed {
                    obj,
                    name: names[field_idx as usize].0,
                    value,
                };
            }
            encode_instruction(&instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
    }

}

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
        name_field_projections(&mut module, &["first", "second"]);
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

#[test]
fn nested_projection_cannot_strengthen_an_incoming_readonly_ticket() {
    for named in [false, true] {
        let source = "type Inner is { value: Int }; type Outer is { value: Inner };\n\
            fn borrow(owner: &Outer) -> &Inner { &owner.value }\n\
            fn borrow_mut(owner: &mut Outer) -> &mut Inner { &mut owner.value }\n\
            fn field(inner: &mut Inner) -> &mut Int { &mut inner.value }\n\
            fn forwarded(inner: &mut Inner) -> &mut Int { field(&mut *inner) }\n\
            fn read(field: &Int) -> Int { *field }\n\
            fn write(field: &mut Int) { *field = 99; }";
        let ast = Parser::new(source).parse_module().unwrap();
        let mut module = VbcCodegen::new().compile_module(&ast).unwrap();
        if named {
            name_field_projections(&mut module, &["value"]);
        }
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        validate_module(&module).unwrap();
        let type_id = |name: &str| module.types.iter()
            .find(|ty| module.get_string(ty.name) == Some(name)).unwrap().id;
        let (inner_id, outer_id) = (type_id("Inner"), type_id("Outer"));
        let mut vm = Interpreter::new(module.into());
        let inner = vm.state.heap.alloc(inner_id, std::mem::size_of::<Value>()).unwrap();
        let outer = vm.state.heap.alloc(outer_id, std::mem::size_of::<Value>()).unwrap();
        // SAFETY: both allocations contain exactly one initialized Value field.
        unsafe {
            inner.data_ptr().cast::<Value>().write(Value::from_i64(73));
            outer.data_ptr().cast::<Value>().write(Value::from_ptr(inner.as_ptr().cast::<u8>()));
        }
        let owner = Value::from_ptr(outer.as_ptr().cast::<u8>());
        let readonly = call(&mut vm, "borrow", owner).unwrap();
        for selector in ["field", "forwarded"] {
            let projected = call(&mut vm, selector, readonly).unwrap();
            assert_eq!(call(&mut vm, "read", projected).unwrap().try_as_i64(), Some(73));
            let result = call(&mut vm, "write", projected);
            assert!(result.is_err(), "named={named} selector={selector}: {result:?}");
            let error = result.unwrap_err();
            assert!(matches!(error, InterpreterError::CapabilityDenied { .. }),
                "named={named} selector={selector}: {error:?}");
        }
        let mutable = call(&mut vm, "borrow_mut", owner).unwrap();
        let projected = call(&mut vm, "forwarded", mutable).unwrap();
        call(&mut vm, "write", projected).unwrap();
        assert_eq!(call(&mut vm, "read", projected).unwrap().try_as_i64(), Some(99));
    }
}

#[test]
fn declared_readonly_reborrow_cannot_request_mutability() {
    let ast = Parser::new("fn invalid(value: &Int) -> &mut Int { &mut *value }")
        .parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast)
        .expect_err("readonly declaration cannot authorize a mutable reborrow");
    assert!(error.to_string().contains("readonly place"), "{error}");
}

#[test]
fn references_stored_in_heap_slots_retain_every_inner_capability() {
    use verum_vbc::value::{Capabilities, FatRef};
    for named in [false, true] {
        for fat in [true, false] {
            let source = "type Inner is { value: Int }; type Outer is { value: Inner };\n\
                type Forward is { value: &Inner };\n\
                fn borrowed(owner: &Outer)->&Inner { &owner.value }\n\
                fn mutable(owner: &mut Outer)->&mut Inner { &mut owner.value }\n\
                fn forward(owner: &mut Forward)->&mut &Inner { &mut owner.value }\n\
                fn field(inner:&mut Inner)->&mut Int { &mut inner.value }\n\
                fn update(inner:&mut Inner) { inner.value=47; }\n\
                fn read(value:&Int)->Int { *value }\n\
                fn write(value:&mut Int) { *value=99; }";
            let ast = Parser::new(source).parse_module().unwrap();
            let mut module = VbcCodegen::new().compile_module(&ast).unwrap();
            if named {
                name_field_projections(&mut module, &["value"]);
            }
            let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
            validate_module(&module).unwrap();
            let type_id = |name: &str| {
                module
                    .types
                    .iter()
                    .find(|ty| module.get_string(ty.name) == Some(name))
                    .unwrap()
                    .id
            };
            let (inner_id, outer_id, forward_id) =
                (type_id("Inner"), type_id("Outer"), type_id("Forward"));
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
            let forwarded = vm
                .state
                .heap
                .alloc(forward_id, std::mem::size_of::<Value>())
                .unwrap();
            // SAFETY: all three exact owners remain live; each has one Value slot.
            unsafe {
                inner.data_ptr().cast::<Value>().write(Value::from_i64(73));
                outer
                    .data_ptr()
                    .cast::<Value>()
                    .write(Value::from_ptr(inner.as_ptr().cast::<u8>()));
            }
            let readonly = if fat {
                Value::from_fat_ref(FatRef::new(
                    inner.as_ptr().cast::<u8>(),
                    inner.generation(),
                    inner.epoch(),
                    Capabilities::READ_ONLY,
                    1,
                ))
            } else {
                call(
                    &mut vm,
                    "borrowed",
                    Value::from_ptr(outer.as_ptr().cast::<u8>()),
                )
                .unwrap()
            };
            // SAFETY: the declared forwarding slot holds a live reference value.
            unsafe {
                forwarded.data_ptr().cast::<Value>().write(readonly);
            }
            let indirect = call(
                &mut vm,
                "forward",
                Value::from_ptr(forwarded.as_ptr().cast::<u8>()),
            )
            .unwrap();
            let field = call(&mut vm, "field", indirect)
                .unwrap_or_else(|error| panic!("named={named} fat={fat}: {error}"));
            assert_eq!(call(&mut vm, "read", field).unwrap().try_as_i64(), Some(73));
            let error = call(&mut vm, "write", field)
                .expect_err("outer mutable slot cannot strengthen its readonly reference value");
            assert!(
                matches!(error, InterpreterError::CapabilityDenied { .. }),
                "named={named} fat={fat}: {error}"
            );
            let error = call(&mut vm, "update", indirect)
                .expect_err("direct field writes must retain the same inner capability");
            assert!(
                matches!(error, InterpreterError::CapabilityDenied { .. }),
                "{error}"
            );
            // SAFETY: refusal preserves the live original owner.
            assert_eq!(
                unsafe { inner.data_ptr().cast::<Value>().read() }.try_as_i64(),
                Some(73)
            );
            let mutable = if fat {
                Value::from_fat_ref(FatRef::new(
                    inner.as_ptr().cast::<u8>(),
                    inner.generation(),
                    inner.epoch(),
                    Capabilities::MUT_EXCLUSIVE,
                    1,
                ))
            } else {
                call(
                    &mut vm,
                    "mutable",
                    Value::from_ptr(outer.as_ptr().cast::<u8>()),
                )
                .unwrap()
            };
            // SAFETY: replace only the forwarding reference, retaining its owner.
            unsafe {
                forwarded.data_ptr().cast::<Value>().write(mutable);
            }
            let field = call(&mut vm, "field", indirect).unwrap();
            call(&mut vm, "write", field).unwrap();
            assert_eq!(call(&mut vm, "read", field).unwrap().try_as_i64(), Some(99));
            call(&mut vm, "update", indirect).unwrap();
            assert_eq!(call(&mut vm, "read", field).unwrap().try_as_i64(), Some(47));
            // SAFETY: retire only the forwarding owner to test the outer ticket.
            unsafe {
                vm.state.heap.free(forwarded);
            }
            let error = call(&mut vm, "field", indirect)
                .expect_err("a live inner reference cannot revive its retired outer slot");
            assert!(
                matches!(error, InterpreterError::CbgrViolation { .. }),
                "{error}"
            );
            assert_eq!(call(&mut vm, "read", field).unwrap().try_as_i64(), Some(47));
        }
    }
}

#[test]
fn receiver_slot_cycles_and_exhausted_depth_refuse_without_granting_write() {
    let source = "type Record is { value: Int }; type Forward is { value: &Record };\n\
        fn field(record:&mut Record)->&mut Int { &mut record.value }\n\
        fn forward(owner:&mut Forward)->&mut &Record { &mut owner.value }\n\
        fn borrowed(owner:&Forward)->&Record { owner.value }\n\
        fn write(value:&mut Int){*value=99;}";
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&module).unwrap();
    let type_id = |name: &str| {
        module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap()
            .id
    };
    let (record_id, forward_id) = (type_id("Record"), type_id("Forward"));
    let mut vm = Interpreter::new(module.into());
    let record = vm
        .state
        .heap
        .alloc(record_id, std::mem::size_of::<Value>())
        .unwrap();
    // SAFETY: the exact record allocation owns one initialized Value slot.
    unsafe {
        record.data_ptr().cast::<Value>().write(Value::from_i64(73));
    }
    let mut chain = Value::from_ptr(record.as_ptr().cast::<u8>());
    for depth in 1..=9 {
        let owner = vm
            .state
            .heap
            .alloc(forward_id, std::mem::size_of::<Value>())
            .unwrap();
        // SAFETY: initialized forwarding storage remains owned by the heap.
        unsafe {
            owner.data_ptr().cast::<Value>().write(chain);
        }
        chain = call(
            &mut vm,
            "forward",
            Value::from_ptr(owner.as_ptr().cast::<u8>()),
        )
        .unwrap();
        let result = call(&mut vm, "field", chain);
        if depth <= 8 {
            assert!(result.is_ok(), "depth={depth}: {result:?}");
        } else {
            let error =
                result.expect_err("resource limit cannot return a partially checked receiver");
            assert!(
                matches!(error,InterpreterError::InvalidOperand{ref message} if message.contains("depth limit")),
                "{error}"
            );
        }
    }
    let cycle_owner = vm
        .state
        .heap
        .alloc(forward_id, std::mem::size_of::<Value>())
        .unwrap();
    // SAFETY: initialize before taking a reference, then deliberately encode a
    // cyclic reference value to exercise malformed-bytecode refusal.
    unsafe {
        cycle_owner.data_ptr().cast::<Value>().write(Value::nil());
    }
    let cycle = call(
        &mut vm,
        "forward",
        Value::from_ptr(cycle_owner.as_ptr().cast::<u8>()),
    )
    .unwrap();
    unsafe {
        cycle_owner.data_ptr().cast::<Value>().write(cycle);
    }
    let error = call(&mut vm, "field", cycle)
        .expect_err("cyclic tagged references cannot grant a field place");
    assert!(
        matches!(error,InterpreterError::InvalidOperand{ref message} if message.contains("cyclic")),
        "{error}"
    );
    // SAFETY: failed resolution did not touch or retire the record.
    assert_eq!(
        unsafe { record.data_ptr().cast::<Value>().read() }.try_as_i64(),
        Some(73)
    );
}

#[test]
fn fat_record_snapshots_are_validated_before_projection_or_field_access() {
    use verum_vbc::value::{Capabilities, FatRef};
    let source = "type Record is { value: Int };\n\
        fn field(record:&mut Record)->&mut Int { &mut record.value }\n\
        fn read(record:&Record)->Int { record.value }\n\
        fn update(record:&mut Record) { record.value=99; }";
    for named in [false, true] {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut module = VbcCodegen::new().compile_module(&ast).unwrap();
        if named {
            name_field_projections(&mut module, &["value"]);
        }
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        validate_module(&module).unwrap();
        let owner = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Record"))
            .unwrap()
            .id;
        let mut vm = Interpreter::new(module.into());
        let mut object = vm
            .state
            .heap
            .alloc(owner, std::mem::size_of::<Value>())
            .unwrap();
        // SAFETY: the live exact owner contains one initialized Value slot.
        unsafe {
            object.data_ptr().cast::<Value>().write(Value::from_i64(73));
        }
        let valid = FatRef::new(
            object.as_ptr().cast(),
            object.generation(),
            object.epoch(),
            Capabilities::MUT_EXCLUSIVE,
            1,
        );
        for (case, generation, epoch) in [
            (
                "generation",
                valid.generation().wrapping_add(1),
                valid.epoch(),
            ),
            ("epoch", valid.generation(), valid.epoch().wrapping_sub(1)),
        ] {
            let stale = Value::from_fat_ref(FatRef::new(
                valid.ptr(),
                generation,
                epoch,
                valid.capabilities(),
                1,
            ));
            for operation in ["field", "read", "update"] {
                let error = call(&mut vm, operation, stale)
                    .expect_err("stale snapshots cannot acquire current allocation authority");
                assert!(
                    matches!(error, InterpreterError::CbgrViolation { .. }),
                    "named={named} {case} {operation}: {error}"
                );
            }
        }
        let reference = Value::from_fat_ref(valid);
        assert_eq!(
            call(&mut vm, "read", reference).unwrap().try_as_i64(),
            Some(73)
        );
        call(&mut vm, "update", reference).unwrap();
        assert_eq!(
            call(&mut vm, "read", reference).unwrap().try_as_i64(),
            Some(99)
        );
        object.header_mut().set_capabilities(Capabilities::READ_ONLY.0);
        let error = call(&mut vm, "update", reference)
            .expect_err("a live owner can revoke write authority from an older snapshot");
        assert!(matches!(error, InterpreterError::CapabilityDenied { .. }), "{error}");
        assert_eq!(call(&mut vm, "read", reference).unwrap().try_as_i64(), Some(99));
        object.header_mut().set_capabilities(0);
        for operation in ["field", "read", "update"] {
            let error = call(&mut vm, operation, reference)
                .expect_err("current owner capability revocation is not a fresh ticket");
            assert!(matches!(error, InterpreterError::CapabilityDenied { .. }), "{error}");
        }
        object.header_mut().set_capabilities(Capabilities::FULL.0);
        // SAFETY: retire the allocation; subsequent probes must use membership
        // authority and must not inspect this released header.
        unsafe {
            vm.state.heap.free(object);
        }
        for operation in ["field", "read", "update"] {
            let error = call(&mut vm, operation, reference)
                .expect_err("released owner cannot authorize a fat snapshot");
            assert!(
                matches!(error, InterpreterError::CbgrViolation { .. }),
                "named={named} {operation}: {error}"
            );
        }
    }
}
