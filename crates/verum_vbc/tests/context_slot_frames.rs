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
fn numeric_slots_execute_on_2_71_and_require_their_original_minor() {
    let mut bytes = verum_vbc::serialize::serialize_module(&fixture::numeric_module()).unwrap();
    bytes[6..8].copy_from_slice(&71u16.to_le_bytes());
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(wire.into()).unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    bytes[6..8].copy_from_slice(&70u16.to_le_bytes());
    let error = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap_err();
    assert!(error.to_string().contains("context slots require VBC minor 71"), "{error}");
}

#[test]
fn pointer_operations_each_require_2_72_on_the_wire() {
    use verum_vbc::instruction::Instruction;
    for (old, new) in [(0x36, 0x3b), (0x37, 0x3c)] {
        let mut module = fixture::numeric_module();
        let instructions = module.functions[0].instructions.as_mut().unwrap();
        for instruction in instructions.iter_mut() {
            if let Instruction::SysExtended { sub_op, .. } = instruction
                && *sub_op == old { *sub_op = new; }
        }
        module.bytecode.clear();
        verum_vbc::bytecode::encode_instructions(instructions, &mut module.bytecode);
        let mut bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        bytes[6..8].copy_from_slice(&71u16.to_le_bytes());
        let error = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap_err();
        assert!(error.to_string().contains("context slots require VBC minor 72"), "{error}");
    }
}

#[test]
fn pointer_contexts_preserve_access_rights_and_check_before_rebinding() {
    use verum_vbc::{interpreter::Interpreter, value::Value};
    let source = fixture::module();
    let wire = verum_vbc::deserialize::deserialize_module_validated(
        &verum_vbc::serialize::serialize_module(&source).unwrap(),
    ).unwrap();
    for module in [source, wire] {
        let id = |name: &str| module.find_function_by_name(&format!("core.sys.common.{name}")).unwrap();
        let (get, set, readonly) = (id("__ctx_slot_get_pointer"), id("__ctx_slot_set_pointer"), id("attempt_readonly_context_write"));
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        let slot = Value::from_i64(18);
        let mut word = 17u64;
        let borrowed = Value::from_thin_ref(verum_vbc::value::ThinRef::new(
            std::ptr::addr_of_mut!(word).cast(), 731, 6, verum_vbc::value::Capabilities::READ_ONLY,
        ));
        vm.execute_function_with_args(set, &[slot, borrowed]).unwrap();
        assert_eq!(vm.execute_function_with_args(get, &[slot]).unwrap().bits(), borrowed.bits());
        vm.execute_function_with_args(set, &[slot, Value::from_i64(73)]).unwrap();
        let error = vm.execute_function_with_args(set, &[slot, Value::from_bool(true)]).unwrap_err();
        assert!(error.to_string().contains("raw pointer"), "{error}");
        assert_eq!(vm.execute_function_with_args(get, &[slot]).unwrap().try_as_i64(), Some(73));
        vm.execute_function_with_args(set, &[slot, Value::nil()]).unwrap();
        assert!(vm.state.context_stack.get(18).is_none());
        let error = vm.execute_function(readonly).unwrap_err();
        assert!(error.to_string().contains("write"), "{error}");
    }
}

#[test]
fn pointer_slot_declarations_require_the_selected_unsafe_byte_pointer() {
    use verum_fast_parser::Parser;
    use verum_vbc::codegen::VbcCodegen;
    for declaration in [
        r#"@intrinsic("ctx_slot_get_pointer") fn get(s: Bool) -> &unsafe Byte;"#,
        r#"@intrinsic("ctx_slot_get_pointer") fn get(s: Int) -> Int;"#,
        r#"@intrinsic("ctx_slot_get_pointer") fn get(s: Int) -> &mut Byte;"#,
        r#"@intrinsic("ctx_slot_set_pointer") fn set(s: Int, p: &unsafe Int) -> Int;"#,
        r#"@intrinsic("ctx_slot_set_pointer") fn set(s: Int, p: &unsafe mut Byte) -> Int;"#,
        r#"@intrinsic("ctx_slot_set_pointer") fn set(s: Int, p: &Byte) -> Int;"#,
    ] {
        let ast = Parser::new(declaration).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
        assert!(error.to_string().contains("requires (Int"), "{error}");
    }
    let mut module = fixture::module();
    let id = module.find_function_by_name("core.sys.common.__ctx_slot_set_pointer").unwrap();
    module.get_function_mut(id).unwrap().params[1].type_ref =
        verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::BOOL);
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let error = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap_err();
    assert!(error.to_string().contains("(Int, &unsafe Byte) -> Int"), "{error}");
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
