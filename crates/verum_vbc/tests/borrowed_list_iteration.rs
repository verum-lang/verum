//! The declared cursor borrows live List storage through independent positions.
#![cfg(feature = "codegen")]
#[path = "fixtures/borrowed_list_iteration.rs"]
mod fixture;

#[test]
fn borrowed_list_positions_survive_the_next_cursor_call() {
    let original = fixture::module();
    let bytes = verum_vbc::serialize::serialize_module(&original).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for module in [original, wire] {
        let entry = module
            .find_function_by_name("core.collections.list.main")
            .unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn wire_keeps_legacy_element_access_and_requires_the_mutable_version() {
    use verum_vbc::{
        Instruction, Reg, VbcModule, bytecode::encode_instructions, instruction::CbgrSubOpcode,
        module::FunctionDescriptor,
    };
    fn projection(access: &[u8], minor: u16) -> VbcModule {
        let mut module = VbcModule::new("projection".into());
        module.header.version_minor = minor;
        let mut descriptor = FunctionDescriptor::new(module.intern_string("probe"));
        descriptor.register_count = 3;
        let mut operands = vec![0, 1, 2];
        operands.extend_from_slice(access);
        let instructions = [
            Instruction::CbgrExtended { sub_op: CbgrSubOpcode::RefListElement.to_byte(), operands },
            Instruction::Ret { value: Reg(0) },
        ];
        descriptor.bytecode_length = encode_instructions(&instructions, &mut module.bytecode) as u32;
        module.add_function(descriptor);
        module
    }
    fn load(access: &[u8], minor: u16) -> verum_vbc::error::VbcResult<VbcModule> {
        let mut bytes = verum_vbc::serialize::serialize_module(&projection(access, minor)).unwrap();
        // The serializer always emits its own current minor. Change the
        // encoded header to exercise the same reader as an old image.
        bytes[6..8].copy_from_slice(&minor.to_le_bytes());
        verum_vbc::deserialize::deserialize_module_validated(&bytes)
    }
    let introduced = verum_vbc::format::MUTABLE_ELEMENT_PROJECTION_VERSION;
    for access in [&[][..], &[1][..]] {
        assert_eq!(load(access, introduced - 1).unwrap().header.version_minor, introduced - 1);
    }
    let error = load(&[2], introduced - 1).unwrap_err();
    assert!(error.to_string().contains("writable element projection"), "{error}");
    load(&[2], introduced).unwrap();
    for malformed in [&[0][..], &[3][..], &[2, 0][..]] {
        assert!(load(malformed, introduced).is_err());
    }
}

#[test]
fn both_legacy_element_encodings_execute_from_vbc_2_69() {
    for annotated in [false, true] {
        let module = fixture::legacy_read(annotated);
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn a_shared_loop_cannot_write_its_element_or_tuple_field_through_an_unchecked_caller() {
    let module = fixture::module_with_client(r#"
        fn write(slot: &mut Int) { *slot = 17; }
        fn scalar() {
            let values: List<Int> = [42];
            for slot in &values { write(slot); }
        }
        fn pair() {
            let values: List<(Int, Int)> = [(42, 9)];
            for (left, _) in &values { write(left); }
        }
    "#);
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for image in [module, restored] {
        for name in ["scalar", "pair"] {
            let entry = image.find_function_by_name(&format!("core.collections.list.{name}")).unwrap();
            let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(image.clone().into()).unwrap();
            let error = vm.execute_function(entry).unwrap_err();
            assert!(matches!(error, verum_vbc::interpreter::InterpreterError::CapabilityDenied { .. }), "{error:?}");
        }
    }
}
