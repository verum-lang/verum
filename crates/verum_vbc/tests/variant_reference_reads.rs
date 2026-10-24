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
