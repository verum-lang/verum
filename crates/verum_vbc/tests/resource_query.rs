//! T1807: generic ownership decisions use declarations, never object carriers.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::ResourceDiscipline as D;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, VbcModule,
    bytecode::{decode_instructions, encode_instruction},
    bytecode_remap::{IdRemap, rewrite_instruction_ids},
    codegen::VbcCodegen,
    instruction::Reg,
    interpreter::Interpreter,
    types::{TypeId, TypeParamId, TypeRef},
};

// There is deliberately no public source property for this compiler query.
// Retain the source producer's actual generic-call declarations and witnesses,
// replacing only the single query in the branch-free generic function.
fn compile(source: &str) -> VbcModule {
    let mut module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let original = std::mem::take(&mut module.bytecode);
    for function in &mut module.functions {
        let mut body = decode_instructions(
            &original[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        for instruction in &mut body {
            if let Instruction::TypeLayout { dst, type_ref, .. } = instruction {
                *instruction = Instruction::TypeResource {
                    dst: *dst,
                    type_ref: type_ref.clone(),
                };
            }
        }
        function.bytecode_offset = module.bytecode.len() as u32;
        for instruction in &body {
            encode_instruction(instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        function.instructions = Some(body);
        function.value_uses = None;
        function.is_generic = !function.type_params.is_empty();
    }
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap()
}
fn execute(module: VbcModule, name: &str) -> i64 {
    let id = module.find_function_by_name(name).unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .unwrap()
        .as_i64()
}
fn probe(ty: &str) -> VbcModule {
    compile(&format!(
        r#"
type affine Socket is {{ fd: Int }};
type linear Ticket is {{ id: Int }};
type Holder<T> is {{ value: T }};
fn discipline<T>() -> Int {{ T.size }}
fn outer<U>() -> Int {{ discipline<U>() }}
fn probe() -> Int {{ outer<{ty}>() }}
"#
    ))
}

#[test]
fn generic_query_retains_exact_owned_and_borrowed_components() {
    for (ty, expected) in [
        ("Int", D::Unrestricted),
        ("Socket", D::Affine),
        ("Ticket", D::Linear),
        ("Holder<Socket>", D::Affine),
        ("Holder<Int>", D::Unrestricted),
        ("&Socket", D::Unrestricted),
        ("&checked Socket", D::Unrestricted),
        ("&unsafe Socket", D::Unrestricted),
    ] {
        assert_eq!(execute(probe(ty), "probe"), expected as u8 as i64, "{ty}");
    }
}

#[test]
fn unresolved_generic_frame_returns_unknown_not_copy_permission() {
    assert_eq!(
        execute(probe("Socket"), "discipline"),
        D::Unknown as u8 as i64
    );
}

#[test]
fn specialization_resolves_the_same_nested_resource_witness() {
    use verum_vbc::mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal};
    for (ty, expected) in [
        ("Holder<Socket>", D::Affine),
        ("Holder<Int>", D::Unrestricted),
    ] {
        let mut module = probe(ty);
        let mut graph = InstantiationGraph::new();
        for function in &mut module.functions {
            function.is_generic = !function.type_params.is_empty();
        }
        for function in &module.functions {
            let instructions = decode_instructions(
                &module.bytecode[function.bytecode_offset as usize
                    ..(function.bytecode_offset + function.bytecode_length) as usize],
            )
            .unwrap();
            discover_call_instantiations(&module, &instructions, function.func_id_base, &mut graph)
                .unwrap();
        }
        let module = monomorphize_minimal(module, &graph).unwrap().module;
        assert_eq!(execute(module, "probe"), expected as u8 as i64);
    }
}

#[test]
fn wire_query_preserves_nested_types_wide_registers_and_next_instruction() {
    let query = Instruction::TypeResource {
        dst: Reg(301),
        type_ref: TypeRef::Tuple(vec![
            TypeRef::Instantiated {
                base: TypeId(7001),
                args: vec![TypeRef::Generic(TypeParamId(0x8000))],
            },
            TypeRef::Concrete(TypeId::INT),
        ]),
    };
    let mut bytes = Vec::new();
    encode_instruction(&query, &mut bytes);
    for end in 1..bytes.len() {
        assert!(
            decode_instructions(&bytes[..end]).is_err(),
            "truncated at {end}"
        );
    }
    let ret = Instruction::Ret { value: Reg(301) };
    encode_instruction(&ret, &mut bytes);
    assert_eq!(decode_instructions(&bytes).unwrap(), [query, ret]);
}

#[test]
fn import_remaps_nested_nominal_ids_without_changing_generic_slots() {
    struct Remap;
    impl IdRemap for Remap {
        fn map_type_id(&self, id: TypeId) -> TypeId {
            TypeId(id.0 + 100)
        }
    }
    let mut query = Instruction::TypeResource {
        dst: Reg(0),
        type_ref: TypeRef::Instantiated {
            base: TypeId(7001),
            args: vec![
                TypeRef::Generic(TypeParamId(12)),
                TypeRef::Concrete(TypeId(7002)),
            ],
        },
    };
    rewrite_instruction_ids(&mut query, &Remap);
    assert_eq!(
        query,
        Instruction::TypeResource {
            dst: Reg(0),
            type_ref: TypeRef::Instantiated {
                base: TypeId(7101),
                args: vec![
                    TypeRef::Generic(TypeParamId(12)),
                    TypeRef::Concrete(TypeId(7102))
                ],
            }
        }
    );
}

#[test]
fn query_validates_destination_register_at_load_time() {
    let mut module = probe("Int");
    verum_vbc::validate::validate_module(&module).expect("valid source query module");
    let function = module
        .functions
        .iter_mut()
        .find(|f| module.strings.get(f.name) == Some("discipline"))
        .unwrap();
    // A real query destination is outside this deliberately invalid frame.
    function.register_count = 0;
    assert!(verum_vbc::validate::validate_module(&module).is_err());
}
