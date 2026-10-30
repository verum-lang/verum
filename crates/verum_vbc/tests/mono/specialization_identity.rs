use super::*;
use crate::types::{CbgrTier, ContextRef, Mutability};

fn assert_distinct_specializations(args: &[Vec<TypeRef>]) {
    let names: std::collections::HashSet<_> = args
        .iter()
        .map(|args| specialization_name("dispose", FunctionId(7), args))
        .collect();
    assert_eq!(
        names.len(),
        args.len(),
        "distinct declarations shared a native symbol"
    );
}

#[test]
fn specialization_symbols_preserve_nested_reference_mutability_and_tiers() {
    let mut arguments = Vec::new();
    for tier in [CbgrTier::Tier0, CbgrTier::Tier1, CbgrTier::Tier2] {
        for mutability in [Mutability::Immutable, Mutability::Mutable] {
            arguments.push(vec![TypeRef::Instantiated {
                base: TypeId(7001),
                args: vec![TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(TypeId::INT)),
                    mutability,
                    tier,
                }],
            }]);
        }
    }
    assert_distinct_specializations(&arguments);
}

#[test]
fn specialization_symbols_preserve_callable_contexts_and_const_values() {
    let function = |contexts: Vec<ContextRef>| TypeRef::Function {
        params: vec![TypeRef::Concrete(TypeId::INT)],
        return_type: Box::new(TypeRef::Concrete(TypeId::BOOL)),
        contexts: contexts.into(),
    };
    assert_distinct_specializations(&[
        vec![function(vec![])],
        vec![function(vec![ContextRef(1)])],
        vec![function(vec![ContextRef(2)])],
        vec![function(vec![ContextRef(1), ContextRef(2)])],
        vec![function(vec![ContextRef(2), ContextRef(1)])],
        vec![TypeRef::ConstValue(-1)],
        vec![TypeRef::ConstValue(0)],
        vec![TypeRef::ConstValue(i64::MIN)],
        vec![TypeRef::Concrete(TypeId(0))],
    ]);
}

#[test]
fn specialization_symbols_frame_argument_lists_and_nominal_structure() {
    let int = TypeRef::Concrete(TypeId::INT);
    let boolean = TypeRef::Concrete(TypeId::BOOL);
    assert_distinct_specializations(&[
        vec![],
        vec![TypeRef::Tuple(vec![])],
        vec![int.clone(), boolean.clone()],
        vec![TypeRef::Tuple(vec![int.clone(), boolean])],
        vec![int.clone()],
        vec![TypeRef::Instantiated {
            base: TypeId::INT,
            args: vec![],
        }],
        vec![TypeRef::Array {
            element: Box::new(int.clone()),
            length: 2,
        }],
        vec![TypeRef::Array {
            element: Box::new(int.clone()),
            length: 3,
        }],
        vec![TypeRef::Slice(Box::new(int))],
    ]);
}

#[test]
fn specialization_symbols_distinguish_same_named_function_declarations() {
    let args = [TypeRef::Concrete(TypeId::INT)];
    let first = specialization_name("same", FunctionId(7), &args);
    assert!(first.starts_with("same$mono$"));
    assert_eq!(first, specialization_name("same", FunctionId(7), &args));
    assert_ne!(first, specialization_name("same", FunctionId(8), &args));
}

#[test]
fn specialization_receipts_follow_call_reencoding_extent_changes() {
    use crate::instruction::{Instruction, Reg, RegRange};
    use crate::mono::{InstantiationGraph, SourceLocation, monomorphize_minimal};
    use crate::types::{TypeParamDescriptor, TypeParamId};
    let int = TypeRef::Concrete(TypeId::INT);
    let call = |callee, argument| Instruction::CallG {
        dst: Reg(0),
        func_id: callee,
        type_args: vec![argument],
        args: RegRange {
            start: Reg(0),
            count: 0,
        },
    };
    let mut module = VbcModule::new("receipt_rebase".into());
    for (id, instructions) in [
        vec![call(1, int.clone()), Instruction::RetV],
        vec![call(2, TypeRef::Generic(TypeParamId(0))), Instruction::RetV],
        vec![Instruction::RetV],
    ]
    .into_iter()
    .enumerate()
    {
        let mut descriptor =
            FunctionDescriptor::new(module.intern_string(&format!("function_{id}")));
        descriptor.id = FunctionId(id as u32);
        descriptor.register_count = 1;
        descriptor.bytecode_offset = module.bytecode.len() as u32;
        for instruction in &instructions {
            crate::bytecode::encode_instruction(instruction, &mut module.bytecode);
        }
        descriptor.bytecode_length = module.bytecode.len() as u32 - descriptor.bytecode_offset;
        descriptor.instructions = Some(instructions);
        if id != 0 {
            descriptor.is_generic = true;
            descriptor.type_params.push(TypeParamDescriptor::default());
        }
        module.functions.push(descriptor);
    }
    let old_caller_length = module.functions[0].bytecode_length;
    let old_generic_length = module.functions[1].bytecode_length;
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(FunctionId(1), vec![int.clone()], SourceLocation::default());
    let mut result = monomorphize_minimal(module, &graph).unwrap().module;
    assert!(
        result.functions[0].bytecode_length < old_caller_length,
        "control must shrink the preceding caller"
    );
    let selected = crate::drop_plan::native_glue(&result, FunctionId(1), &[int.clone()])
        .expect("exact receipt must still identify its specialized body");
    let body = result.get_function(selected).unwrap();
    assert!(body.type_params.is_empty());
    assert!(
        body.bytecode_length < old_generic_length,
        "control must shrink the specialized body itself"
    );
    let nested = crate::drop_plan::native_glue(&result, FunctionId(2), &[int.clone()]).unwrap();
    assert!(matches!(body.instructions.as_ref().unwrap()[0],
        Instruction::Call { func_id, .. } if func_id == nested.0));
    for receipt in &result.specializations {
        let id =
            crate::drop_plan::native_glue(&result, receipt.generic_fn, &receipt.type_args).unwrap();
        let descriptor = result.get_function(id).unwrap();
        assert_eq!(
            (receipt.bytecode_offset, receipt.bytecode_length),
            (descriptor.bytecode_offset, descriptor.bytecode_length)
        );
    }
    result
        .specializations
        .retain(|entry| entry.generic_fn != FunctionId(1));
    assert_eq!(
        crate::drop_plan::native_glue(&result, FunctionId(1), &[int]),
        None,
        "a body or name alone must not replace the absent receipt"
    );
}

#[test]
fn unmatched_specialization_receipt_does_not_survive_reencoding() {
    let mut module = VbcModule::new("invalid_receipt".into());
    module.specializations.push(SpecializationEntry {
        generic_fn: FunctionId(7),
        type_args: vec![TypeRef::Concrete(TypeId::INT)],
        hash: 123,
        bytecode_offset: 40,
        bytecode_length: 3,
        register_count: 1,
    });
    ModuleMerger::reencode_bytecode(&mut module);
    assert!(module.specializations.is_empty());
}
