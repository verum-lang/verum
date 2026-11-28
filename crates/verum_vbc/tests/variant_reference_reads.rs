//! Variant consumers must resolve iterator ThinRefs before reading object bytes.
use std::sync::Arc;
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::{Capabilities, ThinRef, Value};

#[derive(Clone, Copy, Debug)]
enum Read {
    Payload,
    Extract,
    Tag,
    Match,
    Borrow,
}

fn interpreter(read: Read, layers: u16) -> Interpreter {
    let mut module = VbcModule::new("variant_references".into());
    let mut instructions = Vec::new();
    let mut source = Reg(0);
    for layer in 0..layers {
        let next = Reg(3 + layer);
        instructions.push(Instruction::Ref {
            dst: next,
            src: source,
        });
        source = next;
    }
    instructions.push(match read {
        Read::Payload => Instruction::AsVar {
            dst: Reg(1),
            value: source,
            tag: 0,
        },
        Read::Extract => Instruction::GetVariantData {
            dst: Reg(1),
            variant: source,
            field: 0,
        },
        Read::Tag => Instruction::GetTag {
            dst: Reg(1),
            variant: source,
        },
        Read::Match => Instruction::IsVar {
            dst: Reg(1),
            value: source,
            tag: 7,
        },
        Read::Borrow => Instruction::GetVariantDataRef {
            dst: Reg(1),
            variant: source,
            field: 0,
        },
    });
    if matches!(read, Read::Borrow) {
        instructions.push(Instruction::Deref {
            dst: Reg(1),
            ref_reg: Reg(1),
        });
    }
    instructions.push(Instruction::Ret { value: Reg(1) });
    for instruction in instructions {
        verum_vbc::bytecode::encode_instruction(&instruction, &mut module.bytecode);
    }
    let mut function = FunctionDescriptor::new(StringId::EMPTY);
    function.id = FunctionId(0);
    function.register_count = 3 + layers;
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    Interpreter::new(Arc::new(
        verum_vbc::deserialize::deserialize_module(&bytes).unwrap(),
    ))
}

fn check(read: Read, expected: Value) {
    for layers in [0, 2] {
        for borrowed in [false, true] {
            let mut vm = interpreter(read, layers);
            let mut slot = vm.alloc_variant(7, &[Value::from_i64(41)]).unwrap();
            let argument = if borrowed {
                Value::from_thin_ref(ThinRef::new(
                    (&mut slot as *mut Value).cast(),
                    0,
                    0,
                    Capabilities::READ_ONLY,
                ))
            } else {
                slot
            };
            let result = vm
                .execute_function_with_args(FunctionId(0), &[argument])
                .unwrap();
            assert_eq!(
                result, expected,
                "{read:?}, layers={layers}, borrowed={borrowed}"
            );
        }
    }
}

#[test]
fn thin_reference_payload_is_read_from_the_referent() {
    check(Read::Payload, Value::from_i64(41));
}

#[test]
fn thin_reference_extraction_is_read_from_the_referent() {
    check(Read::Extract, Value::from_i64(41));
}

#[test]
fn thin_reference_tag_is_read_from_the_referent() {
    check(Read::Tag, Value::from_i64(7));
}

#[test]
fn thin_reference_variant_matching_uses_the_referent() {
    check(Read::Match, Value::from_bool(true));
}

#[test]
fn thin_reference_payload_borrow_addresses_the_referent() {
    check(Read::Borrow, Value::from_i64(41));
}

#[test]
fn null_reference_markers_are_never_read_as_heap_addresses() {
    for (read, expected) in [
        (Read::Payload, Value::nil()),
        (Read::Extract, Value::nil()),
        (Read::Tag, Value::from_i64(0)),
        (Read::Match, Value::from_bool(false)),
    ] {
        for layers in [0, 2] {
            let mut vm = interpreter(read, layers);
            let result = vm
                .execute_function_with_args(FunctionId(0), &[Value::from_thin_ref(ThinRef::null())])
                .unwrap();
            assert_eq!(result, expected, "{read:?}, layers={layers}");
        }
    }
}


#[test]
fn typed_float_conversion_rejects_foreign_values_and_unknown_rounding_modes() {
    use verum_vbc::{instruction::FloatToIntMode, interpreter::InterpreterError};
    for (mode, expected) in [
        (FloatToIntMode::Trunc, -2),
        (FloatToIntMode::Floor, -3),
        (FloatToIntMode::Ceil, -2),
        (FloatToIntMode::Round, -3),
    ] {
        let mut module = VbcModule::new("typed_conversion".into());
        for instruction in [
            Instruction::CvtFI { mode, dst: Reg(1), src: Reg(0) },
            Instruction::Ret { value: Reg(1) },
        ] {
            verum_vbc::bytecode::encode_instruction(&instruction, &mut module.bytecode);
        }
        let mut function = FunctionDescriptor::new(StringId::EMPTY);
        function.id = FunctionId(0);
        function.params.push(verum_vbc::module::ParamDescriptor {
            type_ref: verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::F64),
            ..Default::default()
        });
        function.register_count = 2;
        function.bytecode_length = module.bytecode.len() as u32;
        module.functions.push(function);
        let mut vm = Interpreter::new(Arc::new(module.clone()));
        let actual = vm.execute_function_with_args(FunctionId(0), &[Value::from_f64(-2.7)]).unwrap();
        assert_eq!(actual.as_i64(), expected);
        for argument in [Value::from_i64(3), Value::nil(), Value::from_bool(false)] {
            let mut vm = Interpreter::new(Arc::new(module.clone()));
            assert!(matches!(
                vm.execute_function_with_args(FunctionId(0), &[argument]),
                Err(InterpreterError::TypeMismatch { operation: "CvtFI", .. })
            ));
        }
        // CvtFI's first operand is the encoded rounding-mode byte.
        module.bytecode[1] = u8::MAX;
        let mut vm = Interpreter::new(Arc::new(module));
        assert!(matches!(
            vm.execute_function_with_args(FunctionId(0), &[Value::from_f64(-2.7)]),
            Err(InterpreterError::InvalidBytecode { .. })
        ));
    }
}

#[test]
fn projected_payload_respects_owner_lifetime_field_bounds_and_shared_access() {
    use verum_vbc::interpreter::InterpreterError;
    use verum_vbc::types::TypeId;
    let mut module = VbcModule::new("variant_places".into());
    let payload = |variant, field| Instruction::GetVariantDataRef {
        dst: Reg(2),
        variant,
        field,
    };
    for (name, body) in [
        (
            "borrow",
            vec![payload(Reg(0), 0), Instruction::Ret { value: Reg(2) }],
        ),
        (
            "shared",
            vec![
                Instruction::Ref {
                    dst: Reg(3),
                    src: Reg(0),
                },
                payload(Reg(3), 0),
                Instruction::Ret { value: Reg(2) },
            ],
        ),
        (
            "out_of_bounds",
            vec![payload(Reg(0), 1), Instruction::Ret { value: Reg(2) }],
        ),
        (
            "read",
            vec![
                Instruction::Deref {
                    dst: Reg(2),
                    ref_reg: Reg(0),
                },
                Instruction::Ret { value: Reg(2) },
            ],
        ),
        (
            "write",
            vec![
                Instruction::DerefMut {
                    ref_reg: Reg(0),
                    value: Reg(1),
                },
                Instruction::RetV,
            ],
        ),
    ] {
        let mut function = FunctionDescriptor::new(module.intern_string(name));
        function.id = FunctionId(module.functions.len() as u32);
        function.register_count = 4;
        function.bytecode_offset = module.bytecode.len() as u32;
        verum_vbc::bytecode::encode_instructions(&body, &mut module.bytecode);
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        module.add_function(function);
    }
    module.header.function_table_count = module.functions.len() as u32;
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap(),
    ] {
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        let call = |vm: &mut Interpreter, name: &str, args: &[Value]| {
            let function = vm.state.module.find_function_by_name(name).unwrap();
            vm.execute_function_with_args(function, args)
        };
        let owner = vm.state.heap.alloc_variant(TypeId(100), 0, 1).unwrap();
        let source = Value::from_ptr(owner.as_ptr());
        let mutable = call(&mut vm, "borrow", &[source]).unwrap();
        let shared = call(&mut vm, "shared", &[source]).unwrap();
        call(&mut vm, "write", &[mutable, Value::from_i64(73)]).unwrap();
        assert!(matches!(
            call(&mut vm, "write", &[shared, Value::from_i64(9)]),
            Err(InterpreterError::CapabilityDenied { .. })
        ));
        assert_eq!(
            call(&mut vm, "read", &[shared]).unwrap(),
            Value::from_i64(73)
        );
        assert!(matches!(
            call(&mut vm, "out_of_bounds", &[source]),
            Err(InterpreterError::IndexOutOfBounds {
                index: 1,
                length: 1
            })
        ));
        let record = vm.state.heap.alloc(TypeId(100), 16).unwrap();
        assert!(matches!(
            call(&mut vm, "borrow", &[Value::from_ptr(record.as_ptr())]),
            Err(InterpreterError::InvalidOperand { .. })
        ));
        // SAFETY: the owner belongs to this VM. Subsequent accesses go through checked tickets.
        unsafe { vm.state.heap.free(owner) };
        assert!(matches!(
            call(&mut vm, "read", &[shared]),
            Err(InterpreterError::CbgrViolation { .. })
        ));
        assert!(matches!(
            call(&mut vm, "write", &[mutable, Value::from_i64(1)]),
            Err(InterpreterError::CbgrViolation { .. })
        ));
    }
}
