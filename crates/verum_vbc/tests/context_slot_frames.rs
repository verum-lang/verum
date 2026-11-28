//! Dynamic and lexical access through the actual core.sys.common facade.
#![cfg(feature = "codegen")]
#[path = "fixtures/context_slots.rs"]
mod fixture;

#[test]
fn slots_and_lexical_providers_share_inherited_context_frames() {
    let module = fixture::module();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for module in [module, wire] {
        assert_eq!(
            module
                .ctx_dense_slot_map()
                .values()
                .copied()
                .collect::<Vec<_>>(),
            [32]
        );
        let entry = module
            .find_function_by_name("core.sys.common.main")
            .unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn context_operation_is_rejected_in_an_older_image() {
    let mut bytes = verum_vbc::serialize::serialize_module(&fixture::module()).unwrap();
    bytes[6..8].copy_from_slice(&70u16.to_le_bytes());
    let error = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("context slots require VBC minor 71"),
        "{error}"
    );
}

#[test]
fn older_named_contexts_respect_slot_identity_and_scope_order() {
    let module = fixture::legacy_module();
    let mut bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    bytes[6..8].copy_from_slice(&70u16.to_le_bytes());
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for module in [module, wire] {
        let slots = module.ctx_dense_slot_map();
        for (key, slot) in slots {
            let name = module.get_string(verum_vbc::types::StringId(key)).unwrap();
            assert_eq!(slot, if name == "Counter" { 32 } else { 33 });
        }
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn register_bounds_and_operand_count_are_checked_before_execution() {
    use verum_vbc::instruction::{Instruction, SysSubOpcode};
    for (operands, expected) in [(vec![0, 2], "register"), (vec![0], "end of"), (vec![0, 1, 1], "invalid context store operands")] {
        let mut module = fixture::legacy_module();
        module.bytecode.clear();
        let body = vec![
            Instruction::SysExtended {
                sub_op: SysSubOpcode::ContextSlotGet.to_byte(),
                operands,
            },
            Instruction::Ret {
                value: verum_vbc::instruction::Reg(0),
            },
        ];
        verum_vbc::bytecode::encode_instructions(&body, &mut module.bytecode);
        module.functions[0].instructions = Some(body);
        module.functions[0].bytecode_length = module.bytecode.len() as u32;
        let error = verum_vbc::validate::validate_module(&module).unwrap_err().to_string();
        assert!(error.to_lowercase().contains(expected), "{error}");
    }
}
