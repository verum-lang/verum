//! Generic layout queries preserve their declared parameter through execution.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::VbcModule;

fn compile(source: &str) -> VbcModule {
    let module = Parser::new(source).parse_module().expect("source grammar");
    VbcCodegen::new()
        .compile_module(&module)
        .expect("source VBC")
}

fn run(module: VbcModule, name: &str) -> i64 {
    let id = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some(name))
        .unwrap_or_else(|| {
            panic!(
                "missing {name}: {:?}",
                module
                    .functions
                    .iter()
                    .map(|f| module.get_string(f.name))
                    .collect::<Vec<_>>()
            )
        })
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .expect("execute layout query")
        .as_i64()
}

#[test]
fn generic_byte_size_matches_direct_declaration() {
    let source = "fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

#[test]
fn generic_int_size_preserves_value() {
    let source = "fn size<T>() -> Int { T.size } fn probe() -> Int { size<Int>() }";
    assert_eq!(run(compile(source), "probe"), 8);
}

#[test]
fn nested_call_passes_caller_layout_witness() {
    let source = "fn size<T>() -> Int { T.size } fn outer<U>() -> Int { size<U>() } fn probe() -> Int { outer<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

#[test]
fn alignment_and_stride_preserve_narrow_instantiation() {
    let source = "fn stride<T>() -> Int { T.stride } fn alignment<T>() -> Int { T.alignment } fn probe() -> Int { stride<Byte>() + alignment<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 2);
}

#[test]
fn ordinary_record_queries_agree_with_direct_properties() {
    let source = "type Triple is { x: Int, y: Int, z: Int }; fn size<T>() -> Int { T.size } fn direct() -> Int { Triple.size } fn probe() -> Int { size<Triple>() }";
    let module = compile(source);
    assert_eq!(run(module.clone(), "probe"), run(module, "direct"));
}

#[test]
fn repr_c_queries_agree_with_direct_properties() {
    let source = "@repr(C) type Pair is { x: Byte, y: Int32 }; fn size<T>() -> Int { T.size } fn direct() -> Int { Pair.size } fn probe() -> Int { size<Pair>() }";
    let module = compile(source);
    assert_eq!(run(module.clone(), "probe"), run(module, "direct"));
}

#[test]
fn generic_parameter_shadows_same_spelled_nominal() {
    let source = "type T is { x: Int, y: Int, z: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

#[test]
fn method_parameter_shadows_owner_parameter() {
    let source = "type Owner<T> is { value: T }; implement<T> Owner<T> { fn measure<T>(self) -> Int { T.size } } fn probe() -> Int { let owner = Owner { value: 7 }; owner.measure<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

#[test]
fn nominal_byte_does_not_acquire_primitive_layout_by_spelling() {
    let module = compile(
        "type Byte is { first: Int, second: Int }; fn size<T>() -> Int { T.size } fn direct() -> Int { Byte.size } fn probe() -> Int { size<Byte>() }",
    );
    assert_eq!(run(module.clone(), "direct"), 16);
    assert_eq!(run(roundtrip(&module), "probe"), 16);
}

#[test]
fn record_and_c_layout_facts_survive_wire_without_changing_slot_extent() {
    let module = roundtrip(&compile(
        "type Triple is { x: Int, y: Int, z: Int }; @repr(C) type Pair is { x: Byte, y: Int32 };",
    ));
    for (name, declared, slots) in [("Triple", 24, 24), ("Pair", 8, 16)] {
        let descriptor = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap();
        assert_eq!(descriptor.declared_layout.unwrap().size, declared);
        assert_eq!(descriptor.size, slots);
    }
}

#[test]
fn missing_nominal_layout_is_unknown_even_when_object_extent_is_known() {
    let mut module = compile(
        "type Cell is { value: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Cell>() }",
    );
    let descriptor = module
        .types
        .iter_mut()
        .find(|ty| ty.declared_layout.is_some())
        .unwrap();
    let id = descriptor.id;
    descriptor.declared_layout = None;
    assert_eq!(
        verum_vbc::type_layout::query(
            &module,
            &verum_vbc::types::TypeRef::Concrete(id),
            verum_vbc::instruction::LayoutProperty::Size
        ),
        None
    );
    let id = module.find_function_by_name("probe").unwrap();
    assert!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .is_err()
    );
}

#[test]
fn concrete_array_and_reference_witnesses_use_shared_layout_constants() {
    for (ty, expected) in [
        ("[Byte; 5]", 5),
        ("&Byte", 16),
        ("&checked Byte", 16),
        ("&unsafe Byte", 8),
        ("&[Byte]", 32),
    ] {
        let source = format!(
            "type Shape is {ty}; fn size<T>() -> Int {{ T.size }} fn probe() -> Int {{ size<Shape>() }}"
        );
        assert_eq!(run(roundtrip(&compile(&source)), "probe"), expected, "{ty}");
    }
}

#[test]
fn imported_calls_keep_each_declaring_nominal_layout_in_both_orders() {
    use verum_common::Map;
    use verum_vbc::module::FunctionId;
    let alpha = roundtrip(&compile(
        "module alpha; type Payload is { a: Int, b: Int, c: Int }; fn alpha_size<T>() -> Int { T.size } fn alpha_probe() -> Int { alpha_size<Payload>() }",
    ));
    let beta = roundtrip(&compile(
        "module beta; type Payload is { a: Int }; fn beta_size<T>() -> Int { T.size } fn beta_probe() -> Int { beta_size<Payload>() }",
    ));
    for modules in [[&alpha, &beta], [&beta, &alpha]] {
        let mut codegen = VbcCodegen::new();
        for module in modules {
            codegen.import_archive_module_types(module);
        }
        for (index, module) in modules.into_iter().enumerate() {
            let mapping: Map<_, _> = module
                .functions
                .iter()
                .enumerate()
                .map(|(i, function)| {
                    (
                        function.id.0,
                        FunctionId(7000 + 100 * index as u32 + i as u32),
                    )
                })
                .collect();
            assert!(codegen.merge_archive_function_bodies(module, &mapping.into()) > 0);
        }
        let module = roundtrip(&codegen.finalize_module_from_state().unwrap());
        assert_eq!(run(module.clone(), "alpha.alpha_probe"), 24);
        assert_eq!(run(module, "beta.beta_probe"), 8);
    }
}

#[test]
fn typed_query_roundtrip_keeps_nested_identity_and_following_instructions() {
    use verum_vbc::{
        bytecode::{decode_instructions, encode_instruction},
        instruction::{Instruction, LayoutProperty, Reg},
        types::{TypeId, TypeParamId, TypeRef},
    };
    let query = Instruction::TypeLayout {
        dst: Reg(301),
        property: LayoutProperty::Stride,
        type_ref: TypeRef::Array {
            element: Box::new(TypeRef::Instantiated {
                base: TypeId(7001),
                args: vec![TypeRef::Generic(TypeParamId(0x8000))],
            }),
            length: 17,
        },
    };
    let mut bytes = Vec::new();
    encode_instruction(&query, &mut bytes);
    encode_instruction(&Instruction::Ret { value: Reg(301) }, &mut bytes);
    assert_eq!(
        decode_instructions(&bytes).unwrap(),
        [query, Instruction::Ret { value: Reg(301) }]
    );
    bytes[4] = 7; // wide destination follows two-byte extended header.
    assert!(decode_instructions(&bytes).is_err());
}

#[test]
fn queries_do_not_hide_later_calls_from_mono_or_reachability() {
    use verum_vbc::mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal};
    let mut module = roundtrip(&compile(
        "fn inner<T>() -> Int { T.size } fn outer<T>() -> Int { let n = T.size; n + inner<Byte>() } fn probe() -> Int { outer<Int>() } fn unreachable() -> Int { 99 }",
    ));
    let probe = module.find_function_by_name("probe").unwrap();
    // Reachability consumes the decoded cache supplied by the compilation pipeline.
    for function in &mut module.functions {
        function.instructions = Some(
            verum_vbc::bytecode::decode_instructions(
                &module.bytecode[function.bytecode_offset as usize
                    ..(function.bytecode_offset + function.bytecode_length) as usize],
            )
            .unwrap(),
        );
    }
    let roots = verum_vbc::reachability::analyze_with_roots(&module, &[probe.0]);
    for name in ["inner", "outer", "probe"] {
        assert!(
            roots
                .reachable_ids
                .contains(&module.find_function_by_name(name).unwrap().0),
            "{name}"
        );
    }
    assert!(
        !roots
            .reachable_ids
            .contains(&module.find_function_by_name("unreachable").unwrap().0)
    );
    let mut graph = InstantiationGraph::new();
    for function in &mut module.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    for function in &module.functions {
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        discover_call_instantiations(&module, &instructions, function.func_id_base, &mut graph)
            .unwrap();
    }
    let mono = monomorphize_minimal(module, &graph).unwrap().module;
    assert_eq!(run(mono, "probe"), 9);
}

#[test]
fn parenthesized_reference_queries_share_the_declaration_authority() {
    for (source, expected) in [
        ("fn probe() -> Int { (&checked Byte).size }", 16),
        ("fn probe() -> Int { (&[Byte]).size }", 32),
        (
            "fn size<T>() -> Int { (&T).size } fn probe() -> Int { size<Byte>() }",
            16,
        ),
        (
            "fn size<T>() -> Int { (&T).size } type Shape is [Byte]; fn probe() -> Int { size<Shape>() }",
            32,
        ),
    ] {
        assert_eq!(run(compile(source), "probe"), expected, "{source}");
    }
}

#[test]
fn ordinary_value_field_named_size_is_not_a_layout_query() {
    assert_eq!(
        run(
            compile(
                "type Cell is { size: Int }; fn probe() -> Int { let cell = Cell { size: 37 }; cell.size }"
            ),
            "probe"
        ),
        37
    );
}

#[test]
fn reference_alias_arguments_retain_their_layout_in_nested_frames() {
    for (ty, expected) in [
        ("&Byte", 16),
        ("&checked Byte", 16),
        ("&unsafe Byte", 8),
        ("&[Byte]", 32),
    ] {
        let source = format!(
            "type Shape is {ty}; fn size<T>() -> Int {{ T.size }} fn outer<U>() -> Int {{ size<U>() }} fn probe() -> Int {{ outer<Shape>() }}"
        );
        assert_eq!(run(roundtrip(&compile(&source)), "probe"), expected, "{ty}");
    }
}

#[test]
fn layout_queries_refuse_extent_overflow_and_invalid_alignment() {
    use verum_vbc::{
        instruction::LayoutProperty,
        types::{TypeId, TypeRef},
    };
    let mut module = compile("type Cell is { value: Int };");
    let array = TypeRef::Array {
        element: Box::new(TypeRef::Concrete(TypeId::BYTE)),
        length: u64::MAX,
    };
    assert_eq!(
        verum_vbc::type_layout::query(&module, &array, LayoutProperty::Size),
        None
    );
    let descriptor = module
        .types
        .iter_mut()
        .find(|ty| ty.declared_layout.is_some())
        .unwrap();
    descriptor.declared_layout.as_mut().unwrap().alignment = 3;
    let ty = TypeRef::Concrete(descriptor.id);
    assert_eq!(
        verum_vbc::type_layout::query(&module, &ty, LayoutProperty::Stride),
        None
    );
}
