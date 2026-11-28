//! A raw record destination receives all the record's owned fields.
#![cfg(feature = "codegen")]

#[path = "fixtures/owned_flat_record.rs"]
mod fixture;

use fixture::source_module;
use verum_vbc::{Value, VbcModule, interpreter::Interpreter};

#[test]
fn an_unrelated_function_named_ptr_write_keeps_its_declared_behavior() {
    use verum_fast_parser::Parser;
    let ast = Parser::new(r#"
        type Pair is { x: Int, y: Int };
        fn ptr_write(address: *mut Pair, pair: Pair) -> Int { pair.y }
        fn call_local(address: *mut Pair) -> Int { ptr_write(address, Pair { x: 17, y: 73 }) }
    "#).parse_module().unwrap();
    let module = verum_vbc::codegen::VbcCodegen::new().compile_module(&ast).unwrap();
    let call = module.find_function_by_name("call_local").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    assert_eq!(vm.call(call, &[Value::nil()]).unwrap().try_as_i64(), Some(73));
}

#[test]
fn a_named_owned_record_can_be_moved_read_and_replaced() {
    let module = source_module();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for image in [module, decoded] {
        let store = image.find_function_by_name("store").unwrap();
        let take = image.find_function_by_name("take").unwrap();
        let releases = image.find_function_by_name("releases").unwrap();
        let mut vm = Interpreter::try_new_validated(image.into()).unwrap();
        vm.run_global_ctors().unwrap();
        let baseline = vm.state.heap.allocated();
        let mut slots = [0_u64; 3];
        let address = Value::from_ptr(slots.as_mut_ptr().cast::<u8>());
        let mut released = 0;
        for number in [13, 29] {
            vm.call(store, &[address, Value::from_i64(number)]).unwrap();
            assert_eq!(vm.call(releases, &[]).unwrap().try_as_i64(), Some(released));
            assert_eq!(
                vm.call(take, &[address]).unwrap().try_as_i64(),
                Some(number * 10 + 7)
            );
            released += number;
            assert_eq!(vm.call(releases, &[]).unwrap().try_as_i64(), Some(released));
            assert_eq!(slots[2], 0);
            assert_eq!(
                vm.state.heap.allocated(),
                baseline,
                "the source and result shells retired"
            );
        }
    }
}

#[test]
fn a_refused_transfer_preserves_the_previous_bytes_and_owned_fields() {
    use verum_vbc::{Capabilities, ThinRef};
    let module = source_module();
    let make = module.find_function_by_name("make_owner").unwrap();
    let transfer = module.find_function_by_name("transfer").unwrap();
    let releases = module.find_function_by_name("releases").unwrap();
    let owner_id = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Envelope"))
        .unwrap()
        .id;
    for (fault, expected) in [
        ("source", "source differs"),
        ("extent", "destination is shorter"),
        ("access", "capability"),
        ("overlap", "overlaps"),
    ] {
        let mut vm = Interpreter::try_new_validated(module.clone().into()).unwrap();
        vm.run_global_ctors().unwrap();
        let mut buffer = [0x4343_u64; 2];
        let raw = buffer.as_mut_ptr().cast::<u8>();
        let owner = if fault == "source" {
            Value::from_ptr(
                vm.state
                    .heap
                    .alloc(owner_id, 8)
                    .unwrap()
                    .as_ptr()
                    .cast::<u8>(),
            )
        } else {
            vm.call(make, &[]).unwrap()
        };
        let address = match fault {
            "access" => Value::from_thin_ref(ThinRef::new(raw, 0, 0, Capabilities::READ_ONLY)),
            "extent" => {
                vm.state.cbgr_bridge_extents.insert(raw as usize, 8);
                Value::from_ptr(raw)
            }
            "overlap" => owner,
            _ => Value::from_ptr(raw),
        };
        let failure = vm
            .call(transfer, &[address, owner])
            .unwrap_err()
            .to_string();
        assert!(
            failure.to_lowercase().contains(expected),
            "{fault}: {failure}"
        );
        assert_eq!(buffer, [0x4343; 2]);
        assert!(vm.state.heap.contains(owner.as_ptr()));
        assert_eq!(vm.call(releases, &[]).unwrap().try_as_i64(), Some(0));
    }
}

#[test]
fn a_minor_67_flat_write_keeps_its_borrowed_source_alive() {
    use verum_vbc::{
        FunctionId, Instruction, Reg,
        bytecode::encode_instruction,
        instruction::MemSubOpcode,
        module::{FunctionDescriptor, ParamDescriptor},
        types::TypeId,
    };
    let mut image = VbcModule::new("legacy_record_write".into());
    image.header.version_minor = 67;
    let name = image.intern_string("legacy");
    let owner_type = TypeId(2100);
    let mut operands = vec![0, 1, 16];
    operands.extend_from_slice(&owner_type.0.to_le_bytes());
    for instruction in [
        Instruction::MemExtended {
            sub_op: MemSubOpcode::PtrWrite.to_byte(),
            operands,
        },
        Instruction::Ret { value: Reg(1) },
    ] {
        encode_instruction(&instruction, &mut image.bytecode);
    }
    let entry = image.add_function(FunctionDescriptor {
        id: FunctionId(0),
        name,
        params: vec![ParamDescriptor::default(); 2].into(),
        register_count: 2,
        bytecode_length: image.bytecode.len() as u32,
        ..Default::default()
    });
    image.header.function_table_count = image.functions.len() as u32;
    let bytes = verum_vbc::serialize::serialize_module(&image).unwrap();
    let archived = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    let mut vm = Interpreter::try_new_validated(archived.into()).unwrap();
    let allocation = vm.state.heap.alloc(owner_type, 16).unwrap();
    let source = allocation.data_ptr().cast::<Value>();
    // SAFETY: the newly allocated record holds exactly two aligned value slots.
    unsafe {
        source.write(Value::from_i64(11));
        source.add(1).write(Value::from_i64(37));
    }
    let mut destination = [0_u64; 2];
    let address = destination.as_mut_ptr().cast::<u8>();
    vm.state.cbgr_bridge_extents.insert(address as usize, 16);
    let owned = Value::from_ptr(allocation.as_ptr().cast::<u8>());
    let result = vm.call(entry, &[Value::from_ptr(address), owned]).unwrap();
    assert_eq!(result.to_bits(), owned.to_bits());
    assert!(vm.state.heap.contains(owned.as_ptr()));
    assert_eq!(
        destination,
        [Value::from_i64(11).to_bits(), Value::from_i64(37).to_bits()]
    );
}
