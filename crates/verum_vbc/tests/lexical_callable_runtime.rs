//! Lexical type evidence belongs to the environment, even without value captures.
use std::sync::Arc;
use verum_vbc::{
    Instruction, Value,
    bytecode::encode_instruction,
    deserialize::deserialize_module,
    instruction::{LayoutProperty, Reg, RegRange},
    interpreter::Interpreter,
    module::{FunctionDescriptor, FunctionId, VbcModule},
    serialize::serialize_module,
    types::{TypeId, TypeParamDescriptor, TypeParamId, TypeRef},
};

fn add(module: &mut VbcModule, name: &str, ids: &[u16], body: &[Instruction]) {
    let name = module.intern_string(name);
    let mut function = FunctionDescriptor::new(name);
    function.id = FunctionId(module.functions.len() as u32);
    function.register_count = 8;
    function.type_params.extend(ids.iter().map(|id| TypeParamDescriptor {
        id: TypeParamId(*id), ..Default::default()
    }));
    function.bytecode_offset = module.bytecode.len() as u32;
    for instruction in body { encode_instruction(instruction, &mut module.bytecode); }
    function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
    module.functions.push(function);
}

fn fixture(args: Vec<TypeRef>, catch: bool) -> VbcModule {
    let mut module = VbcModule::new("lexical_runtime".into());
    add(&mut module, "probe", &[], &[
        Instruction::NewClosure { dst: Reg(1), func_id: 1, captures: vec![],
            capture_types: Some(vec![]), lexical_types: args },
        if catch { Instruction::Call { dst: Reg(0), func_id: 2,
            args: RegRange::new(Reg(1), 1) } }
        else { Instruction::CallClosure { dst: Reg(0), closure: Reg(1),
            args: RegRange::new(Reg(0), 0) } },
        Instruction::Ret { value: Reg(0) },
    ]);
    // Sparse IDs deliberately differ from argument vector positions.
    add(&mut module, "body", &[7, 42], &[
        Instruction::TypeLayout { dst: Reg(0), type_ref: TypeRef::Generic(TypeParamId(42)),
            property: LayoutProperty::Size },
        Instruction::Ret { value: Reg(0) },
    ]);
    add(&mut module, "core.base.panic.catch_unwind", &[], &[]);
    deserialize_module(&serialize_module(&module).unwrap()).unwrap()
}

#[test]
fn zero_capture_sparse_witnesses_survive_sync_callbacks() {
    for catch in [false, true] {
        for (ty, size) in [(TypeId::INT, 8), (TypeId::BOOL, 1)] {
            let module = fixture(vec![TypeRef::Concrete(TypeId::TEXT), TypeRef::Concrete(ty)], catch);
            let mut interpreter = Interpreter::new(Arc::new(module));
            let value = interpreter.execute_function(FunctionId(0)).unwrap();
            let result = if catch {
                assert_eq!(unsafe { verum_vbc::interpreter::variant_tag(value.as_ptr()) }, 0);
                // The live interpreter owns the successful Result payload.
                unsafe { *value.as_ptr::<u8>().add(verum_common::layout::VARIANT_PAYLOAD_OFFSET as usize).cast::<Value>() }
            } else { value };
            assert_eq!(result.as_i64(), size);
        }
    }
}

#[test]
fn missing_extra_and_unresolved_lexical_arguments_refuse() {
    for (args, expected) in [
        (vec![TypeRef::Concrete(TypeId::INT), TypeRef::AssociatedProjection {
            base: Box::new(TypeRef::Concrete(TypeId::INT)), assoc: "Output".into(),
        }], "unresolved closure lexical witness"),
        (vec![], "lexical witness mismatch"),
        (vec![TypeRef::Concrete(TypeId::INT)], "lexical witness mismatch"),
        (vec![TypeRef::Concrete(TypeId::INT); 3], "lexical witness mismatch"),
        (vec![TypeRef::Concrete(TypeId::INT), TypeRef::Generic(TypeParamId(42))], "unresolved closure lexical witness"),
    ] {
        let mut interpreter = Interpreter::new(Arc::new(fixture(args, false)));
        let error = interpreter.execute_function(FunctionId(0)).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn malformed_lexical_wire_counts_and_markers_refuse() {
    let instruction = Instruction::NewClosure { dst: Reg(0), func_id: 1, captures: vec![],
        capture_types: None, lexical_types: vec![TypeRef::Concrete(TypeId::INT)] };
    let mut bytes = Vec::new();
    encode_instruction(&instruction, &mut bytes);
    assert_eq!(verum_vbc::bytecode::decode_instructions(&bytes).unwrap(), [instruction]);
    // Extended tag, sub-op, dst, function, captures, optional declarations.
    let mut invalid = bytes.clone();
    invalid[5] = 2;
    assert!(verum_vbc::bytecode::decode_instructions(&invalid).is_err());
    let mut invalid = bytes[..6].to_vec();
    invalid.extend_from_slice(&[0x80, 0x80, 0x04]); // 65536 witnesses, no payload.
    assert!(verum_vbc::bytecode::decode_instructions(&invalid).is_err());
    bytes.pop();
    assert!(verum_vbc::bytecode::decode_instructions(&bytes).is_err());
}
