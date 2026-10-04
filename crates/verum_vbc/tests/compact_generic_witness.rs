//! T1528: runtime witnesses follow declaration IDs without sparse wire padding.

use std::sync::Arc;
use verum_vbc::bytecode::encode_instruction;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::{TypeId, TypeParamDescriptor, TypeParamId, TypeRef};

const SHADOW: u16 = 0x8000;

fn generic(id: u16) -> TypeRef {
    TypeRef::Generic(TypeParamId(id))
}

fn call(id: u32, type_args: Vec<TypeRef>) -> Instruction {
    Instruction::CallG {
        dst: Reg(0),
        func_id: id,
        type_args,
        args: RegRange {
            start: Reg(0),
            count: 0,
        },
    }
}

fn add_function(module: &mut VbcModule, ids: &[u16], instructions: &[Instruction]) {
    let id = module.functions.len() as u32;
    let name = module.intern_string(&format!("witness_{id}"));
    let mut function = FunctionDescriptor::new(name);
    function.id = FunctionId(id);
    function.register_count = 4;
    function.bytecode_offset = module.bytecode.len() as u32;
    function
        .type_params
        .extend(ids.iter().map(|id| TypeParamDescriptor {
            id: TypeParamId(*id),
            ..Default::default()
        }));
    for instruction in instructions {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
    module.functions.push(function);
}

fn returning(type_ref: TypeRef) -> [Instruction; 2] {
    [
        Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        },
        Instruction::Ret { value: Reg(0) },
    ]
}

fn direct(ids: &[u16], args: Vec<TypeRef>, result: TypeRef) -> verum_vbc::value::Value {
    let mut module = VbcModule::new("compact_witness".into());
    add_function(
        &mut module,
        &[],
        &[call(1, args), Instruction::Ret { value: Reg(0) }],
    );
    add_function(&mut module, ids, &returning(result));
    Interpreter::new(Arc::new(module))
        .execute_function(FunctionId(0))
        .unwrap()
}

#[test]
fn compact_method_shadow_and_parent_are_distinct_runtime_types() {
    let args = vec![
        TypeRef::Concrete(TypeId::INT),
        TypeRef::Concrete(TypeId::BOOL),
    ];
    assert_eq!(
        direct(&[0, SHADOW], args.clone(), generic(0)).as_type_id(),
        TypeId::INT
    );
    assert_eq!(
        direct(&[0, SHADOW], args, generic(SHADOW)).as_type_id(),
        TypeId::BOOL
    );
}

#[test]
fn compact_order_comes_from_declaration_ids() {
    let args = vec![
        TypeRef::Concrete(TypeId::TEXT),
        TypeRef::Concrete(TypeId::BOOL),
    ];
    assert_eq!(
        direct(&[7, 2], args.clone(), generic(7)).as_type_id(),
        TypeId::TEXT
    );
    assert_eq!(direct(&[7, 2], args, generic(2)).as_type_id(), TypeId::BOOL);
}

#[test]
fn missing_declared_witness_does_not_read_another_slot() {
    assert!(direct(&[7, 2], vec![TypeRef::Concrete(TypeId::TEXT)], generic(2)).is_nil());
    assert!(direct(&[7, 2], vec![TypeRef::Concrete(TypeId::TEXT)], generic(0)).is_nil());
}

#[test]
fn legacy_missing_roster_and_indexed_vectors_still_resolve() {
    let args = vec![
        TypeRef::Concrete(TypeId::INT),
        TypeRef::Concrete(TypeId::BOOL),
    ];
    assert_eq!(direct(&[], args, generic(1)).as_type_id(), TypeId::BOOL);
    let mut sparse = vec![generic(0); SHADOW as usize + 1];
    sparse[SHADOW as usize] = TypeRef::Concrete(TypeId::TEXT);
    assert_eq!(
        direct(&[0, SHADOW], sparse, generic(SHADOW)).as_type_id(),
        TypeId::TEXT
    );
}

#[test]
fn legacy_indexed_const_slot_survives_an_incomplete_type_roster() {
    let result = direct(
        &[0],
        vec![TypeRef::Concrete(TypeId::INT), TypeRef::ConstValue(37)],
        generic(1),
    );
    assert_eq!(result.as_i64(), 37);
}

#[test]
fn compact_const_value_witness_loads_as_a_value() {
    assert_eq!(
        direct(
            &[0, SHADOW],
            vec![TypeRef::Concrete(TypeId::INT), TypeRef::ConstValue(37)],
            generic(SHADOW)
        )
        .as_i64(),
        37
    );
}

#[test]
fn nested_call_resolves_the_callers_shadow_before_entering_the_callee() {
    let mut module = VbcModule::new("nested_compact_witness".into());
    add_function(
        &mut module,
        &[],
        &[
            call(
                1,
                vec![
                    TypeRef::Concrete(TypeId::INT),
                    TypeRef::Concrete(TypeId::BOOL),
                ],
            ),
            Instruction::Ret { value: Reg(0) },
        ],
    );
    add_function(
        &mut module,
        &[0, SHADOW],
        &[
            call(2, vec![generic(SHADOW)]),
            Instruction::Ret { value: Reg(0) },
        ],
    );
    add_function(&mut module, &[3], &returning(generic(3)));
    let result = Interpreter::new(Arc::new(module))
        .execute_function(FunctionId(0))
        .unwrap();
    assert_eq!(result.as_type_id(), TypeId::BOOL);
}

#[test]
fn method_sidecar_resolves_shadow_witnesses_inside_nested_types() {
    let mut module = VbcModule::new("sidecar_compact_witness".into());
    add_function(
        &mut module,
        &[],
        &[
            call(
                1,
                vec![
                    TypeRef::Concrete(TypeId::INT),
                    TypeRef::Concrete(TypeId::BOOL),
                ],
            ),
            Instruction::Ret { value: Reg(0) },
        ],
    );
    add_function(
        &mut module,
        &[0, SHADOW],
        &[
            Instruction::SetCallWitness {
                type_args: vec![
                    generic(SHADOW),
                    TypeRef::Instantiated {
                        base: TypeId::LIST,
                        args: vec![generic(SHADOW)],
                    },
                ],
            },
            Instruction::LoadUnit { dst: Reg(0) },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.execute_function(FunctionId(0)).unwrap();
    assert_eq!(
        interpreter.state.pending_call_witness.as_deref(),
        Some(
            [
                TypeRef::Concrete(TypeId::BOOL),
                TypeRef::Instantiated {
                    base: TypeId::LIST,
                    args: vec![TypeRef::Concrete(TypeId::BOOL)]
                },
            ]
            .as_slice()
        )
    );
}

#[test]
fn direct_calls_discard_a_method_only_sidecar() {
    for invoke in [
        call(1, vec![]),
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange {
                start: Reg(0),
                count: 0,
            },
        },
    ] {
        let mut module = VbcModule::new("sidecar_lifetime".into());
        add_function(
            &mut module,
            &[],
            &[
                Instruction::SetCallWitness {
                    type_args: vec![TypeRef::Concrete(TypeId::BOOL)],
                },
                invoke,
                Instruction::Ret { value: Reg(0) },
            ],
        );
        add_function(
            &mut module,
            &[],
            &[
                Instruction::LoadUnit { dst: Reg(0) },
                Instruction::Ret { value: Reg(0) },
            ],
        );
        let mut interpreter = Interpreter::new(Arc::new(module));
        interpreter.execute_function(FunctionId(0)).unwrap();
        assert!(
            interpreter.state.pending_call_witness.is_none(),
            "a direct call must not leave an old witness for a later method call"
        );
    }
}
