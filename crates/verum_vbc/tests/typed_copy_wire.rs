//! Typed-copy declaration witnesses survive wire, linking and specialization.
use verum_common::List;
use verum_vbc::{
    Instruction, Reg, TypeId, TypeRef,
    bytecode::{decode_instructions, encode_instruction},
    bytecode_remap::{IdRemap, rewrite_instruction_ids},
    module::{FunctionDescriptor, FunctionId, VbcModule},
    mono::{BytecodeSpecializer, InstantiationGraph, TypeSubstitution},
    types::{CbgrTier, Mutability, TypeParamId},
};
fn declared() -> TypeRef {
    TypeRef::Instantiated {
        base: TypeId(7001),
        args: vec![TypeRef::Reference {
            inner: Box::new(TypeRef::Generic(TypeParamId(17))),
            mutability: Mutability::Immutable,
            tier: CbgrTier::Tier0,
        }],
    }
}
fn operation(ty: TypeRef) -> Instruction {
    Instruction::CloneTyped {
        dst: Reg(300),
        src: Reg(1),
        type_ref: ty,
    }
}
fn module(op: Instruction) -> VbcModule {
    let mut module = VbcModule::new("typed_copy_wire".into());
    let mut function = FunctionDescriptor::new(module.intern_string("probe"));
    function.id = FunctionId(0);
    function.register_count = 301;
    encode_instruction(&op, &mut module.bytecode);
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    module.header.function_table_count = 1;
    module
}
#[test]
fn typed_copy_wire_keeps_all_operands_and_refuses_every_truncation() {
    let op = operation(declared());
    let mut bytes = List::<u8>::new().into();
    encode_instruction(&op, &mut bytes);
    assert_eq!(&bytes[..2], &[0x1f, 0x0f]);
    for length in 1..bytes.len() {
        assert!(decode_instructions(&bytes[..length]).is_err(), "{length}");
    }
    encode_instruction(&Instruction::RetV, &mut bytes);
    assert_eq!(
        decode_instructions(&bytes).unwrap(),
        [op, Instruction::RetV]
    );
}
#[test]
fn typed_copy_merge_changes_only_nominal_identity() {
    struct Shift;
    impl IdRemap for Shift {
        fn map_type_id(&self, id: TypeId) -> TypeId {
            TypeId(id.0 + 100)
        }
    }
    let mut op = operation(declared());
    rewrite_instruction_ids(&mut op, &Shift);
    let mut expected = declared();
    if let TypeRef::Instantiated { base, .. } = &mut expected {
        *base = TypeId(7101);
    }
    assert_eq!(op, operation(expected));
}
#[test]
fn typed_copy_specialization_keeps_the_reference_witness() {
    let original = declared();
    let module = module(operation(original.clone()));
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(17), TypeRef::Concrete(TypeId::INT));
    let graph = InstantiationGraph::new();
    let result = BytecodeSpecializer::new(&module, &substitution, &graph)
        .specialize(&module.functions[0], &[])
        .unwrap();
    assert_eq!(
        decode_instructions(&result.bytecode).unwrap(),
        [
            operation(substitution.apply(&original).unwrap()),
            Instruction::RetV
        ]
    );
}
#[test]
fn typed_copy_validation_checks_each_register_owner_and_format_version() {
    let good = module(operation(TypeRef::Concrete(TypeId::INT)));
    verum_vbc::validate::validate_module(&good).unwrap();
    for position in 0..2 {
        let mut op = operation(TypeRef::Concrete(TypeId::INT));
        if let Instruction::CloneTyped { dst, src, .. } = &mut op {
            *[dst, src][position] = Reg(301);
        }
        assert!(verum_vbc::validate::validate_module(&module(op)).is_err());
    }
    assert!(
        verum_vbc::validate::validate_module(&module(operation(TypeRef::Concrete(TypeId(7001)))))
            .is_err()
    );
    let mut old = good.clone();
    old.header.version_minor = 56;
    let error = verum_vbc::validate::validate_module(&old).unwrap_err();
    assert!(
        error.to_string().contains("requires VBC minor 57"),
        "{error}"
    );
    let mut new = good;
    new.header.version_minor = verum_vbc::format::VERSION_MINOR + 1;
    assert!(matches!(
        verum_vbc::validate::validate_module(&new),
        Err(verum_vbc::VbcError::UnsupportedVersion { .. })
    ));
}
