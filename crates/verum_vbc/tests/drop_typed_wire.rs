//! Typed destruction retains declarations through the wire, merge and mono paths.
use verum_vbc::{
    Instruction,
    bytecode::{decode_instructions, encode_instruction},
    bytecode_remap::{IdRemap, rewrite_instruction_ids},
    cbgr_analysis::VbcEscapeAnalyzer,
    instruction::{BinaryIntOp, ExtendedSubOpcode, Reg},
    module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcFunction, VbcModule},
    mono::{
        BytecodeSpecializer, InstantiationGraph, SourceLocation, SpecializationOptimizer,
        TypeSubstitution, discover_call_instantiations, monomorphize_minimal,
    },
    types::{
        CbgrTier, Mutability, TypeDescriptor, TypeId, TypeKind, TypeParamDescriptor, TypeParamId,
        TypeRef,
    },
};

fn declared_type() -> TypeRef {
    TypeRef::Instantiated {
        base: TypeId(7001),
        args: vec![TypeRef::Reference {
            inner: Box::new(TypeRef::Tuple(vec![
                TypeRef::Concrete(TypeId(7002)),
                TypeRef::Generic(TypeParamId(0x8000)),
            ])),
            mutability: Mutability::Immutable,
            tier: CbgrTier::Tier1,
        }],
    }
}

#[test]
fn destruction_wire_preserves_nested_evidence_and_instruction_boundary() {
    let instruction = Instruction::DropTyped {
        src: Reg(301),
        type_ref: declared_type(),
    };
    let mut bytes = Vec::new();
    encode_instruction(&instruction, &mut bytes);
    assert_eq!(&bytes[..2], &[0x1f, 0x09]);
    assert_eq!(
        ExtendedSubOpcode::from_byte(0x09),
        Some(ExtendedSubOpcode::DropTyped)
    );
    for end in 1..bytes.len() {
        assert!(
            decode_instructions(&bytes[..end]).is_err(),
            "accepted truncated instruction at {end}"
        );
    }
    encode_instruction(&Instruction::RetV, &mut bytes);
    assert_eq!(
        decode_instructions(&bytes).unwrap(),
        [instruction, Instruction::RetV]
    );
}

#[test]
fn destruction_merge_remaps_nominal_ids_and_preserves_reference_and_binder_identity() {
    struct Shift;
    impl IdRemap for Shift {
        fn map_type_id(&self, id: TypeId) -> TypeId {
            TypeId(id.0 + 100)
        }
    }
    let mut instruction = Instruction::DropTyped {
        src: Reg(301),
        type_ref: declared_type(),
    };
    rewrite_instruction_ids(&mut instruction, &Shift);
    assert_eq!(
        instruction,
        Instruction::DropTyped {
            src: Reg(301),
            type_ref: TypeRef::Instantiated {
                base: TypeId(7101),
                args: vec![TypeRef::Reference {
                    inner: Box::new(TypeRef::Tuple(vec![
                        TypeRef::Concrete(TypeId(7102)),
                        TypeRef::Generic(TypeParamId(0x8000)),
                    ])),
                    mutability: Mutability::Immutable,
                    tier: CbgrTier::Tier1,
                }],
            },
        }
    );
}

fn module(instructions: &[Instruction], registers: u16) -> VbcModule {
    let mut module = VbcModule::new("drop_wire".into());
    let mut function = FunctionDescriptor::new(module.intern_string("probe"));
    function.id = FunctionId(0);
    function.register_count = registers;
    for instruction in instructions {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    module.header.function_table_count = module.functions.len() as u32;
    module
}

#[test]
fn destruction_specialization_substitutes_sparse_witnesses_without_erasing_borrows() {
    let original = declared_type();
    let module = module(
        &[
            Instruction::DropTyped {
                src: Reg(301),
                type_ref: original.clone(),
            },
            Instruction::RetV,
        ],
        302,
    );
    let mut substitution = TypeSubstitution::empty();
    substitution.bind(TypeParamId(0x8000), TypeRef::Concrete(TypeId::INT));
    let graph = InstantiationGraph::new();
    let specialized = BytecodeSpecializer::new(&module, &substitution, &graph)
        .specialize(&module.functions[0], &[])
        .unwrap();
    assert_eq!(
        decode_instructions(&specialized.bytecode).unwrap(),
        [
            Instruction::DropTyped {
                src: Reg(301),
                type_ref: substitution.apply(&original)
            },
            Instruction::RetV,
        ]
    );
}

#[test]
fn destruction_validates_source_register_and_declared_nominal_type() {
    let valid = module(
        &[
            Instruction::DropTyped {
                src: Reg(0),
                type_ref: TypeRef::Concrete(TypeId::INT),
            },
            Instruction::RetV,
        ],
        1,
    );
    verum_vbc::validate::validate_module(&valid).expect("valid typed destruction");
    let archive = verum_vbc::serialize::serialize_module(&valid).unwrap();
    let reloaded = verum_vbc::deserialize::deserialize_module_validated(&archive).unwrap();
    assert_eq!(reloaded.bytecode, valid.bytecode);
    let mut invalid_register = valid.clone();
    invalid_register.functions[0].register_count = 0;
    assert!(verum_vbc::validate::validate_module(&invalid_register).is_err());
    let invalid_type = module(
        &[
            Instruction::DropTyped {
                src: Reg(0),
                type_ref: TypeRef::Concrete(TypeId(7001)),
            },
            Instruction::RetV,
        ],
        1,
    );
    assert!(verum_vbc::validate::validate_module(&invalid_type).is_err());
}

fn owner_type(argument: TypeRef) -> TypeRef {
    TypeRef::Instantiated {
        base: TypeId(7001),
        args: vec![argument],
    }
}

fn generic_destructor_module() -> VbcModule {
    let mut module = module(
        &[
            Instruction::DropTyped {
                src: Reg(0),
                type_ref: owner_type(TypeRef::Generic(TypeParamId(7))),
            },
            Instruction::RetV,
        ],
        1,
    );
    module.functions[0].type_params.push(TypeParamDescriptor {
        id: TypeParamId(7),
        ..Default::default()
    });
    module.functions[0].is_generic = true;
    let mut owner = TypeDescriptor {
        id: TypeId(7001),
        kind: TypeKind::Record,
        drop_fn: Some(1),
        ..Default::default()
    };
    owner.type_params.push(TypeParamDescriptor {
        id: TypeParamId(0),
        ..Default::default()
    });
    module.types.push(owner);
    let mut destructor = FunctionDescriptor::new(module.intern_string("drop_owner"));
    destructor.id = FunctionId(1);
    destructor.register_count = 1;
    destructor.is_generic = true;
    destructor.type_params.push(TypeParamDescriptor {
        id: TypeParamId(0x8000),
        ..Default::default()
    });
    let receiver = TypeRef::Reference {
        inner: Box::new(owner_type(TypeRef::Generic(TypeParamId(0x8000)))),
        mutability: Mutability::Mutable,
        tier: CbgrTier::Tier0,
    };
    destructor.params.push(ParamDescriptor {
        type_ref: receiver.clone(),
        ..Default::default()
    });
    destructor.semantic_params = Some(vec![Some(receiver)].into());
    destructor.bytecode_offset = module.bytecode.len() as u32;
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    destructor.bytecode_length = module.bytecode.len() as u32 - destructor.bytecode_offset;
    module.functions.push(destructor);
    module
}

#[test]
fn destruction_discovers_only_complete_declaration_owned_generic_calls() {
    let module = generic_destructor_module();
    let int = TypeRef::Concrete(TypeId::INT);
    let mut graph = InstantiationGraph::new();
    let complete = Instruction::DropTyped {
        src: Reg(0),
        type_ref: owner_type(int.clone()),
    };
    discover_call_instantiations(&module, &[complete], 0, &mut graph).unwrap();
    assert!(graph.contains(FunctionId(1), &[int]));
    assert_eq!(graph.len(), 1);
    let mut unresolved = InstantiationGraph::new();
    let deferred = Instruction::DropTyped {
        src: Reg(0),
        type_ref: owner_type(TypeRef::Generic(TypeParamId(7))),
    };
    discover_call_instantiations(&module, &[deferred], 0, &mut unresolved).unwrap();
    assert_eq!(unresolved.len(), 0);
    let mut incomplete = module.clone();
    incomplete.functions[1].semantic_params = None;
    let absent = Instruction::DropTyped {
        src: Reg(0),
        type_ref: owner_type(TypeRef::Concrete(TypeId::INT)),
    };
    discover_call_instantiations(&incomplete, &[absent], 0, &mut unresolved).unwrap();
    assert_eq!(unresolved.len(), 0);
}

#[test]
fn destruction_discovers_generic_glue_after_enclosing_function_specialization() {
    let module = generic_destructor_module();
    let args = vec![TypeRef::Concrete(TypeId::INT)];
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(FunctionId(0), args.clone(), SourceLocation::default());
    let result = monomorphize_minimal(module, &graph).unwrap();
    assert_eq!(result.metrics.total_instantiations, 2);
    assert!(
        result
            .module
            .specializations
            .iter()
            .any(|entry| entry.generic_fn == FunctionId(1) && entry.type_args == args)
    );
}

#[test]
fn destruction_is_a_mutating_optimizer_boundary_and_call_like_escape() {
    let instructions = [
        Instruction::LoadI {
            dst: Reg(0),
            value: 7,
        },
        Instruction::DropTyped {
            src: Reg(0),
            type_ref: TypeRef::Concrete(TypeId::INT),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(0),
            b: Reg(0),
        },
        Instruction::RetV,
    ];
    let module = module(&instructions, 2);
    let optimized = SpecializationOptimizer::new().optimize(module.bytecode);
    assert_eq!(decode_instructions(&optimized).unwrap(), instructions);

    let mut descriptor = FunctionDescriptor::new(Default::default());
    descriptor.id = FunctionId(0);
    descriptor.register_count = 2;
    let function = VbcFunction::new(
        descriptor,
        vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 7,
            },
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::DropTyped {
                src: Reg(1),
                type_ref: TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(TypeId::INT)),
                    mutability: Mutability::Immutable,
                    tier: CbgrTier::Tier0,
                },
            },
            Instruction::RetV,
        ],
    );
    let analysis = VbcEscapeAnalyzer::new().analyze(&[function]);
    assert_eq!(
        analysis.decisions.get(&(FunctionId(0), 1)),
        Some(&CbgrTier::Tier0)
    );
}
