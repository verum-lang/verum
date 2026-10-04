//! T1526: exact call identity, concrete discovery, nested specialization.
use verum_vbc::bytecode::{decode_instructions, encode_instructions_with_fixup};
use verum_vbc::instruction::{Instruction as I, Reg, RegRange};
use verum_vbc::module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcModule};
use verum_vbc::mono::{
    InstantiationGraph, MonoPhaseConfig, MonoPhaseError, MonomorphizationPhase, SourceLocation,
    discover_call_instantiations, monomorphize_minimal,
};
use verum_vbc::types::{
    StringId, TypeDescriptor, TypeId, TypeParamDescriptor, TypeParamId, TypeRef as T, Variance,
};

fn generic(id: u16) -> T {
    T::Generic(TypeParamId(id))
}
fn concrete(id: TypeId) -> T {
    T::Concrete(id)
}
fn applied(id: u32, arg: T) -> T {
    T::Instantiated {
        base: TypeId(id),
        args: vec![arg],
    }
}
fn parameter(id: u16) -> TypeParamDescriptor {
    TypeParamDescriptor {
        id: TypeParamId(id),
        name: StringId::EMPTY,
        bounds: Default::default(),
        default: None,
        variance: Variance::Invariant,
        type_bounds: Default::default(),
    }
}
fn param(ty: T) -> ParamDescriptor {
    ParamDescriptor {
        name: StringId::EMPTY,
        type_ref: ty,
        is_mut: false,
        default: None,
        type_name: StringId::EMPTY,
    }
}
fn add_function(
    m: &mut VbcModule,
    name: &str,
    ids: &[u16],
    params: Vec<T>,
    result: T,
    body: Vec<I>,
) -> u32 {
    let id = m.functions.len() as u32;
    let start = m.bytecode.len() as u32;
    let length = encode_instructions_with_fixup(&body, &mut m.bytecode);
    m.functions.push(FunctionDescriptor {
        id: FunctionId(id),
        name: m.strings.intern(name),
        bytecode_offset: start,
        bytecode_length: length as u32,
        instructions: Some(body),
        register_count: 8,
        type_params: ids.iter().copied().map(parameter).collect(),
        params: params.into_iter().map(param).collect(),
        return_type: result,
        is_generic: !ids.is_empty(),
        ..Default::default()
    });
    id
}
fn call(id: u32, args: Vec<T>) -> I {
    I::CallG {
        dst: Reg(1),
        func_id: id,
        type_args: args,
        args: RegRange {
            start: Reg(0),
            count: 1,
        },
    }
}
fn body(m: &VbcModule, id: u32) -> Vec<I> {
    let f = &m.functions[id as usize];
    decode_instructions(
        &m.bytecode[f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize],
    )
    .unwrap()
}
fn routed(instruction: &I) -> u32 {
    if let I::Call { func_id, .. } = instruction {
        *func_id
    } else {
        panic!("not routed: {instruction:?}")
    }
}
fn replace_body(m: &mut VbcModule, id: u32, instructions: Vec<I>) {
    let offset = m.bytecode.len() as u32;
    let length = encode_instructions_with_fixup(&instructions, &mut m.bytecode);
    let f = &mut m.functions[id as usize];
    f.bytecode_offset = offset;
    f.bytecode_length = length as u32;
    f.instructions = Some(instructions);
}

#[test]
fn concrete_sites_seed_without_preseed_and_preserve_two_targets_and_unknowns() {
    let mut m = VbcModule::new("exact".into());
    let target = add_function(
        &mut m,
        "identity",
        &[0],
        vec![generic(0)],
        generic(0),
        vec![I::Ret { value: Reg(0) }],
    );
    let calls = vec![
        call(target, vec![concrete(TypeId::INT)]),
        call(target, vec![concrete(TypeId::TEXT)]),
        call(target, vec![generic(7)]),
        I::Call {
            dst: Reg(1),
            func_id: target,
            args: RegRange {
                start: Reg(0),
                count: 1,
            },
        },
        I::Ret { value: Reg(1) },
    ];
    let entry = add_function(
        &mut m,
        "entry",
        &[],
        vec![concrete(TypeId::INT)],
        concrete(TypeId::INT),
        calls.clone(),
    );
    assert!(m.specializations.is_empty());
    let mut graph = InstantiationGraph::new();
    discover_call_instantiations(&m, &calls, 0, &mut graph).unwrap();
    discover_call_instantiations(&m, &calls, 0, &mut graph).unwrap();
    assert_eq!(graph.len(), 2);
    let result = monomorphize_minimal(m, &graph).unwrap();
    let ops = body(&result.module, entry);
    let int = routed(&ops[0]);
    let text = routed(&ops[1]);
    assert_ne!(int, text);
    assert_eq!(
        result.module.functions[int as usize].return_type,
        concrete(TypeId::INT)
    );
    assert_eq!(
        result.module.functions[text as usize].return_type,
        concrete(TypeId::TEXT)
    );
    assert!(
        matches!(&ops[2], I::CallG { func_id, type_args, .. } if *func_id == target && type_args == &[generic(7)])
    );
    assert_eq!(routed(&ops[3]), target);
}

#[test]
fn nested_calls_and_recursive_cycles_reach_one_deduplicated_fixed_point() {
    let mut m = VbcModule::new("nested".into());
    let inner = add_function(
        &mut m,
        "inner",
        &[0],
        vec![generic(0)],
        generic(0),
        vec![call(0, vec![generic(0)]), I::Ret { value: Reg(0) }],
    );
    let outer = add_function(
        &mut m,
        "outer",
        &[0],
        vec![generic(0)],
        generic(0),
        vec![
            I::SetCallWitness {
                type_args: vec![applied(100, generic(0))],
            },
            call(inner, vec![applied(100, generic(0))]),
            I::Ret { value: Reg(1) },
        ],
    );
    let entry = add_function(
        &mut m,
        "entry",
        &[],
        vec![concrete(TypeId::INT)],
        concrete(TypeId::INT),
        vec![
            call(outer, vec![concrete(TypeId::INT)]),
            I::Ret { value: Reg(1) },
        ],
    );
    let mut graph = InstantiationGraph::new();
    discover_call_instantiations(&m, &body(&m, entry), 0, &mut graph).unwrap();
    let result = monomorphize_minimal(m, &graph).unwrap();
    assert_eq!(result.metrics.new_specializations, 2);
    let outer_spec = routed(&body(&result.module, entry)[0]);
    let outer_body = body(&result.module, outer_spec);
    assert!(
        matches!(&outer_body[0], I::SetCallWitness { type_args } if type_args == &[applied(100,concrete(TypeId::INT))])
    );
    let inner_spec = routed(&outer_body[1]);
    assert_eq!(routed(&body(&result.module, inner_spec)[0]), inner_spec);
    let mut second = InstantiationGraph::new();
    for f in &result.module.functions {
        discover_call_instantiations(
            &result.module,
            &body(&result.module, f.id.0),
            0,
            &mut second,
        )
        .unwrap();
    }
    assert!(second.is_empty());
}

fn type_method_module(argument: T, include_authority: bool) -> (VbcModule, u32, u32) {
    let mut m = VbcModule::new("protocol".into());
    let mut owner = TypeDescriptor::default();
    owner.id = TypeId(100);
    owner.name = m.strings.intern("alpha.Sink");
    owner.type_params.push(parameter(0));
    m.types.push(owner);
    let target = add_function(
        &mut m,
        "alpha.Sink.from_iter",
        &[0, 1],
        vec![generic(1)],
        applied(100, generic(0)),
        vec![I::Ret { value: Reg(0) }],
    );
    m.functions[target as usize].parent_type = Some(TypeId(100));
    let _sibling = add_function(
        &mut m,
        "beta.Sink.from_iter",
        &[0, 1],
        vec![generic(1)],
        concrete(TypeId::BOOL),
        vec![I::Ret { value: Reg(0) }],
    );
    m.functions[1].parent_type = Some(TypeId(101));
    let method = m.strings.intern("from_iter");
    if include_authority {
        m.resolved_protocol_dispatch
            .insert((100, method.0), FunctionId(target));
    }
    let collect = add_function(
        &mut m,
        "collect",
        &[0, 1],
        vec![argument],
        generic(1),
        vec![
            I::LoadT {
                dst: Reg(1),
                type_ref: generic(1),
            },
            I::Mov {
                dst: Reg(2),
                src: Reg(0),
            },
            I::CallM {
                dst: Reg(3),
                receiver: Reg(1),
                method_id: method.0,
                args: RegRange {
                    start: Reg(2),
                    count: 1,
                },
            },
            I::Ret { value: Reg(3) },
        ],
    );
    (m, collect, target)
}

#[test]
fn generic_type_method_uses_exact_owner_and_full_argument_type_without_phantom_receiver() {
    let (m, collect, target) = type_method_module(applied(200, generic(0)), true);
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(
        FunctionId(collect),
        vec![concrete(TypeId::INT), applied(100, concrete(TypeId::TEXT))],
        SourceLocation::default(),
    );
    let result = monomorphize_minimal(m, &graph).unwrap();
    assert_eq!(result.metrics.new_specializations, 2);
    let collect_spec = result
        .module
        .functions
        .iter()
        .find(|f| {
            f.id.0 > collect
                && f.params[0].type_ref == applied(200, concrete(TypeId::INT))
                && f.return_type == applied(100, concrete(TypeId::TEXT))
        })
        .unwrap();
    let ops = body(&result.module, collect_spec.id.0);
    let target_spec = routed(&ops[2]);
    assert_ne!(target_spec, target);
    assert_eq!(
        result.module.functions[target_spec as usize].params[0].type_ref,
        applied(200, concrete(TypeId::INT))
    );
    assert!(matches!(
        ops[2],
        I::Call {
            args: RegRange {
                start: Reg(2),
                count: 1
            },
            ..
        }
    ));
}

#[test]
fn missing_owner_or_unknown_argument_never_selects_sibling_or_unit_fallback() {
    for (argument, authority) in [
        (applied(200, generic(0)), false),
        (concrete(TypeId::UNIT), true),
        (concrete(TypeId::PTR), true),
    ] {
        let (m, collect, _) = type_method_module(argument, authority);
        let mut graph = InstantiationGraph::new();
        graph.record_instantiation(
            FunctionId(collect),
            vec![concrete(TypeId::INT), applied(100, concrete(TypeId::TEXT))],
            SourceLocation::default(),
        );
        let result = monomorphize_minimal(m, &graph).unwrap();
        assert_eq!(result.metrics.new_specializations, 1);
        assert!(matches!(
            body(&result.module, collect + 1)[2],
            I::CallM { .. }
        ));
    }
}

#[test]
fn expanding_nested_types_obey_existing_depth_budget() {
    let mut m = VbcModule::new("growth".into());
    let recursive = add_function(
        &mut m,
        "grow",
        &[0],
        vec![generic(0)],
        generic(0),
        vec![
            call(0, vec![T::Tuple(vec![generic(0)])]),
            I::Ret { value: Reg(0) },
        ],
    );
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(
        FunctionId(recursive),
        vec![concrete(TypeId::INT)],
        SourceLocation::default(),
    );
    let error = monomorphize_minimal(m, &graph).unwrap_err();
    assert!(matches!(
        error,
        MonoPhaseError::ResourceLimit {
            resource: "type depth",
            limit: 64
        }
    ));
}

#[test]
fn recursive_expansion_fails_loudly_at_count_and_round_budgets() {
    for (count, rounds, resource) in [
        (4, 100, "instantiation count"),
        (100, 3, "discovery rounds"),
    ] {
        let mut m = VbcModule::new("budget".into());
        add_function(
            &mut m,
            "grow",
            &[0],
            vec![generic(0)],
            generic(0),
            vec![
                call(0, vec![applied(100, generic(0))]),
                call(0, vec![T::Tuple(vec![generic(0)])]),
                I::Ret { value: Reg(0) },
            ],
        );
        let mut graph = InstantiationGraph::new();
        graph.record_instantiation(
            FunctionId(0),
            vec![concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        let config = MonoPhaseConfig {
            max_instantiations: count,
            max_rounds: rounds,
            ..MonoPhaseConfig::minimal()
        };
        let error = MonomorphizationPhase::new(config)
            .execute(m, &graph)
            .unwrap_err();
        assert!(
            matches!(error,MonoPhaseError::ResourceLimit { resource: found, .. } if found == resource)
        );
    }
}

#[test]
fn converted_type_call_consumes_sidecar_and_preserves_conditional_jump_target() {
    let (mut m, collect, _) = type_method_module(generic(0), true);
    m.functions[collect as usize]
        .params
        .push(param(concrete(TypeId::BOOL)));
    let method = m.strings.intern("from_iter").0;
    replace_body(
        &mut m,
        collect,
        vec![
            I::JmpIf {
                cond: Reg(1),
                offset: 5,
            },
            I::LoadT {
                dst: Reg(3),
                type_ref: generic(1),
            },
            I::LoadI {
                dst: Reg(2),
                value: 41,
            },
            I::SetCallWitness {
                type_args: vec![concrete(TypeId::TEXT), generic(0)],
            },
            I::CallM {
                dst: Reg(4),
                receiver: Reg(3),
                method_id: method,
                args: RegRange {
                    start: Reg(2),
                    count: 1,
                },
            },
            I::Ret { value: Reg(4) },
        ],
    );
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(
        FunctionId(collect),
        vec![concrete(TypeId::INT), applied(100, concrete(TypeId::TEXT))],
        SourceLocation::default(),
    );
    let result = monomorphize_minimal(m, &graph).unwrap();
    let mut ops = body(&result.module, collect + 1);
    verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut ops);
    assert!(matches!(ops[0], I::JmpIf { offset: 5, .. }));
    assert!(matches!(ops[3], I::Nop));
    assert!(matches!(
        ops[4],
        I::Call {
            args: RegRange {
                start: Reg(2),
                count: 1
            },
            ..
        }
    ));
    assert!(matches!(ops[5], I::Ret { .. }));
}

#[test]
fn distinct_type_tokens_at_control_flow_join_do_not_choose_the_last_arm() {
    let mut m = VbcModule::new("join".into());
    let method = m.strings.intern("make");
    for (name, id) in [("alpha.A", 100), ("beta.B", 101)] {
        let mut ty = TypeDescriptor::default();
        ty.id = TypeId(id);
        ty.name = m.strings.intern(name);
        m.types.push(ty);
        let f = add_function(
            &mut m,
            &format!("{name}.make"),
            &[],
            vec![],
            concrete(TypeId::INT),
            vec![
                I::LoadI {
                    dst: Reg(0),
                    value: id as i64,
                },
                I::Ret { value: Reg(0) },
            ],
        );
        m.functions[f as usize].parent_type = Some(TypeId(id));
        m.resolved_protocol_dispatch
            .insert((id, method.0), FunctionId(f));
    }
    let caller = add_function(
        &mut m,
        "choose",
        &[0],
        vec![concrete(TypeId::BOOL)],
        concrete(TypeId::INT),
        vec![
            I::JmpIf {
                cond: Reg(0),
                offset: 3,
            },
            I::LoadT {
                dst: Reg(1),
                type_ref: concrete(TypeId(100)),
            },
            I::Jmp { offset: 2 },
            I::LoadT {
                dst: Reg(1),
                type_ref: concrete(TypeId(101)),
            },
            I::CallM {
                dst: Reg(2),
                receiver: Reg(1),
                method_id: method.0,
                args: RegRange {
                    start: Reg(0),
                    count: 0,
                },
            },
            I::Ret { value: Reg(2) },
        ],
    );
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(
        FunctionId(caller),
        vec![concrete(TypeId::INT)],
        SourceLocation::default(),
    );
    let result = monomorphize_minimal(m, &graph).unwrap();
    assert!(matches!(
        body(&result.module, caller + 1)[4],
        I::CallM { .. }
    ));
}

#[cfg(feature = "codegen")]
#[test]
fn source_generic_chain_seeds_and_specializes_without_environment_flags() {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    let source = "module calls; type Sink is { value: Int }; implement Sink { fn make<U>(value: U) -> Int { 7 } } fn outer<C>(value: Int) -> Int { C.make(value) } fn probe() -> Int { outer<Sink>(7) }";
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let mut m = VbcCodegen::with_config(CodegenConfig::new("calls"))
        .compile_module(&ast)
        .unwrap();
    m.resolve_protocol_dispatch();
    let entry = m
        .functions
        .iter()
        .find(|f| m.get_string(f.name) == Some("calls.probe"))
        .unwrap()
        .id
        .0;
    let mut graph = InstantiationGraph::new();
    discover_call_instantiations(&m, &body(&m, entry), 0, &mut graph).unwrap();
    assert_eq!(graph.len(), 1);
    let result = monomorphize_minimal(m, &graph).unwrap();
    assert_eq!(result.metrics.new_specializations, 2);
}

#[test]
fn initial_oversized_call_has_the_same_loud_depth_failure_as_nested_discovery() {
    let mut m = VbcModule::new("initial_budget".into());
    let target = add_function(
        &mut m,
        "id",
        &[0],
        vec![generic(0)],
        generic(0),
        vec![I::Ret { value: Reg(0) }],
    );
    let mut ty = concrete(TypeId::INT);
    for _ in 0..64 {
        ty = T::Tuple(vec![ty]);
    }
    let mut graph = InstantiationGraph::new();
    let error =
        discover_call_instantiations(&m, &[call(target, vec![ty])], 0, &mut graph).unwrap_err();
    assert!(matches!(
        error,
        MonoPhaseError::ResourceLimit {
            resource: "type depth",
            limit: 64
        }
    ));
    assert!(graph.is_empty());
}

#[test]
fn legacy_const_surplus_keeps_identity_and_substitutes_undeclared_slots() {
    let mut m = VbcModule::new("const_surplus".into());
    let target = add_function(
        &mut m,
        "sized",
        &[0],
        vec![],
        concrete(TypeId::INT),
        vec![
            I::LoadT {
                dst: Reg(0),
                type_ref: generic(1),
            },
            I::Ret { value: Reg(0) },
        ],
    );
    let mut graph = InstantiationGraph::new();
    discover_call_instantiations(
        &m,
        &[
            call(target, vec![concrete(TypeId::INT), T::ConstValue(7)]),
            call(target, vec![concrete(TypeId::INT), T::ConstValue(9)]),
        ],
        0,
        &mut graph,
    )
    .unwrap();
    assert_eq!(graph.len(), 2);
    let result = monomorphize_minimal(m, &graph).unwrap();
    let mut constants = result
        .module
        .functions
        .iter()
        .skip(1)
        .map(|f| {
            if let I::LoadT {
                type_ref: T::ConstValue(value),
                ..
            } = body(&result.module, f.id.0)[0]
            {
                value
            } else {
                panic!("const slot was lost")
            }
        })
        .collect::<Vec<_>>();
    constants.sort();
    assert_eq!(constants, vec![7, 9]);
}
