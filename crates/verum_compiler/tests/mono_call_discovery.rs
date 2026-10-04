//! T1526: the public compiler entrypoint must scan exact CallG without seeds.
use verum_compiler::phases::VbcMonomorphizationPhase;
use verum_vbc::{
    bytecode::encode_instructions_with_fixup,
    instruction::{Instruction, Reg, RegRange},
    module::{FunctionDescriptor, FunctionId, VbcModule},
    types::{StringId, TypeId, TypeParamDescriptor, TypeParamId, TypeRef, Variance},
};

#[test]
fn public_monomorphize_discovers_a_concrete_call_with_an_empty_seed_table() {
    let mut module = VbcModule::new("empty_seed".into());
    let body = vec![Instruction::Ret { value: Reg(0) }];
    let length = encode_instructions_with_fixup(&body, &mut module.bytecode);
    module.functions.push(FunctionDescriptor {
        id: FunctionId(0),
        name: module.strings.intern("identity"),
        bytecode_length: length as u32,
        instructions: Some(body),
        register_count: 2,
        return_type: TypeRef::Generic(TypeParamId(0)),
        type_params: [TypeParamDescriptor {
            id: TypeParamId(0),
            name: StringId::EMPTY,
            bounds: Default::default(),
            default: None,
            variance: Variance::Invariant,
            type_bounds: Default::default(),
        }]
        .into_iter()
        .collect(),
        ..Default::default()
    });
    let body = vec![
        Instruction::CallG {
            dst: Reg(1),
            func_id: 0,
            type_args: vec![TypeRef::Concrete(TypeId::INT)],
            args: RegRange {
                start: Reg(0),
                count: 1,
            },
        },
        Instruction::Ret { value: Reg(1) },
    ];
    let offset = module.bytecode.len() as u32;
    let length = encode_instructions_with_fixup(&body, &mut module.bytecode);
    module.functions.push(FunctionDescriptor {
        id: FunctionId(1),
        name: module.strings.intern("entry"),
        bytecode_offset: offset,
        bytecode_length: length as u32,
        instructions: Some(body),
        register_count: 2,
        ..Default::default()
    });
    assert!(module.specializations.is_empty());
    let result = VbcMonomorphizationPhase::new()
        .without_cache()
        .without_parallel()
        .without_optimize()
        .monomorphize(&module)
        .unwrap();
    assert_eq!(result.functions.len(), 3);
    assert!(matches!(
        &result.functions[1].instructions.as_ref().unwrap()[0],
        Instruction::Call { func_id: 2, .. }
    ));
    assert_eq!(
        result.functions[2].return_type,
        TypeRef::Concrete(TypeId::INT)
    );
}
