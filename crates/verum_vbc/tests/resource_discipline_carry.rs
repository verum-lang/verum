//! T1594: source constraints survive archive boundaries without inventing ownership.
#![cfg(feature = "codegen")]
use verum_common::ResourceDiscipline as D;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
    types::{TypeDescriptor, TypeId, TypeRef},
};

fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}
fn roundtrip(module: &VbcModule) -> VbcModule {
    deserialize_module(&serialize_module(module).unwrap()).unwrap()
}
fn declared<'a>(module: &'a VbcModule, name: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some(name))
        .unwrap_or_else(|| {
            panic!(
                "missing {name}: {:?}",
                module
                    .types
                    .iter()
                    .map(|ty| module.get_string(ty.name))
                    .collect::<Vec<_>>()
            )
        })
}
fn effective(module: &VbcModule, name: &str) -> D {
    module.resource_discipline(&TypeRef::Concrete(declared(module, name).id))
}

#[test]
fn source_declarations_and_attribute_have_one_wire_meaning() {
    let module = compile(
        "type Plain is { id: Int }; type affine Owned is { id: Int }; type linear Once is { id: Int }; @must_consume type affine Ack is { id: Int };",
    );
    for module in [module.clone(), roundtrip(&module)] {
        for (name, expected) in [
            ("Plain", D::Unrestricted),
            ("Owned", D::Affine),
            ("Once", D::Linear),
            ("Ack", D::Linear),
        ] {
            assert_eq!(
                declared(&module, name).resource_discipline,
                expected,
                "{name}"
            );
            assert_eq!(effective(&module, name), expected, "effective {name}");
        }
    }
}

#[test]
fn serialized_sibling_declarations_keep_exact_owner_in_either_order() {
    let alpha = "module alpha { public type affine Token is { id: Int }; }";
    let beta = "module beta { public type Token is { id: Int }; }";
    for source in [format!("{alpha} {beta}"), format!("{beta} {alpha}")] {
        let module = roundtrip(&compile(&source));
        let mut facts: Vec<_> = module
            .types
            .iter()
            .filter(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name.ends_with("Token"))
            })
            .map(|ty| {
                (
                    ty.origin_module
                        .and_then(|id| module.get_string(id))
                        .unwrap_or(""),
                    ty.resource_discipline,
                )
            })
            .collect();
        facts.sort_by_key(|(owner, _)| *owner);
        assert_eq!(
            facts,
            [("alpha", D::Affine), ("beta", D::Unrestricted)],
            "all descriptors: {:?}",
            module.types
        );
    }
}

#[test]
fn generic_components_aliases_and_borrowed_fields_are_distinct() {
    let module = roundtrip(&compile(
        "type affine Token is { id: Int }; type Holder<T> is { value: T }; type Alias is Token; type Borrowed is { value: &Token }; type Choice is Empty | Value(Token);",
    ));
    let holder = declared(&module, "Holder");
    assert_eq!(holder.resource_discipline, D::Unrestricted);
    assert_eq!(effective(&module, "Holder"), D::Unknown);
    for (argument, expected) in [
        (TypeId::INT, D::Unrestricted),
        (declared(&module, "Token").id, D::Affine),
        (TypeId(99999), D::Unknown),
    ] {
        assert_eq!(
            module.resource_discipline(&TypeRef::Instantiated {
                base: holder.id,
                args: vec![TypeRef::Concrete(argument)]
            }),
            expected
        );
    }
    assert_eq!(effective(&module, "Alias"), D::Affine);
    assert_eq!(effective(&module, "Borrowed"), D::Unrestricted);
    assert_eq!(effective(&module, "Choice"), D::Affine);
}

#[test]
fn archive_import_reinterns_identity_without_changing_the_constraint() {
    let source = roundtrip(&compile(
        "module library; public type affine Token is { id: Int }; public type Envelope<T> is { value: T };",
    ));
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..100 {
        codegen
            .ctx_mut()
            .intern_string_raw(&format!("unrelated_{i}"));
    }
    codegen.import_archive_module_types(&source);
    let consumer = codegen.finalize_module().unwrap();
    let tokens: Vec<_> = consumer
        .types
        .iter()
        .filter(|ty| {
            consumer
                .get_string(ty.name)
                .is_some_and(|name| name.ends_with("Token"))
        })
        .collect();
    assert!(!tokens.is_empty());
    assert!(tokens.iter().all(|ty| ty.resource_discipline == D::Affine));
}

#[test]
fn missing_or_recursive_provenance_does_not_become_copy_permission() {
    let mut module =
        compile("type Holder is { missing: Unresolved }; type Cycle is { next: Cycle };");
    assert_eq!(effective(&module, "Holder"), D::Unknown);
    assert_eq!(effective(&module, "Cycle"), D::Unknown);
    let id = declared(&module, "Holder").id;
    module
        .types
        .iter_mut()
        .find(|ty| ty.id == id)
        .unwrap()
        .resource_discipline = D::Unknown;
    assert_eq!(
        module.resource_discipline(&TypeRef::Concrete(id)),
        D::Unknown
    );
    assert_eq!(
        module.resource_discipline(&TypeRef::Concrete(TypeId::PTR)),
        D::Unknown
    );
}

#[test]
fn legacy_descriptor_without_resource_byte_remains_unknown() {
    let mut module = compile("type affine Owned is ();");
    // Isolate the pre-resource type descriptor wire. Source lowering also
    // generates a constructor; unrelated newer function tails are not v2.17.
    module.functions.clear();
    module.bytecode.clear();
    assert_eq!(module.types.len(), 1);
    let mut bytes = serialize_module(&module).unwrap();
    let read_u32 = |bytes: &[u8], at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let end = read_u32(&bytes, 24) as usize;
    assert_eq!(bytes[end - 1], D::Affine as u8);
    bytes.remove(end - 1);
    bytes[6..8].copy_from_slice(&17u16.to_le_bytes());
    for at in [24, 32, 48, 56, 64, 88] {
        let offset = read_u32(&bytes, at);
        if offset >= end as u32 {
            bytes[at..at + 4].copy_from_slice(&(offset - 1).to_le_bytes());
        }
    }
    let hash = blake3::hash(&bytes[verum_vbc::format::HEADER_SIZE..]);
    bytes[72..80].copy_from_slice(&hash.as_bytes()[..8]);
    let legacy = deserialize_module(&bytes).unwrap();
    assert_eq!(declared(&legacy, "Owned").resource_discipline, D::Unknown);
    assert_eq!(effective(&legacy, "Owned"), D::Unknown);
}

#[test]
fn invalid_resource_wire_value_is_rejected() {
    let mut bytes = serialize_module(&compile("type Owned is { id: Int };")).unwrap();
    let end = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    bytes[end - 1] = 255;
    let error = deserialize_module(&bytes).unwrap_err();
    assert!(format!("{error:?}").contains("resource_discipline"));
}

#[test]
fn nested_instantiations_keep_each_exact_argument() {
    let module = roundtrip(&compile(
        "type affine Token is { id: Int }; type Holder<T> is { value: T };",
    ));
    let holder = declared(&module, "Holder").id;
    let nested = |id| TypeRef::Instantiated {
        base: holder,
        args: vec![TypeRef::Instantiated {
            base: holder,
            args: vec![TypeRef::Concrete(id)],
        }],
    };
    assert_eq!(
        module.resource_discipline(&nested(TypeId::INT)),
        D::Unrestricted
    );
    assert_eq!(
        module.resource_discipline(&nested(declared(&module, "Token").id)),
        D::Affine
    );
}

#[test]
fn shared_component_graph_is_resolved_without_exponential_work() {
    let mut source = String::from("type affine Leaf is { id: Int }; ");
    let mut previous = String::from("Leaf");
    for i in 0..40 {
        let name = format!("Node{i}");
        source.push_str(&format!(
            "type {name} is {{ left: {previous}, right: {previous} }}; "
        ));
        previous = name;
    }
    let module = roundtrip(&compile(&source));
    assert_eq!(effective(&module, "Node39"), D::Affine);
}

#[test]
fn exhausted_component_budget_cannot_authorize_unrestricted_use() {
    let module = compile("");
    let oversized = TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::INT); 5000]);
    assert_eq!(module.resource_discipline(&oversized), D::Unknown);
}

#[test]
fn reused_codegen_reclaims_source_owner_after_reset() {
    let mut codegen = VbcCodegen::new();
    for source in [
        "type affine Token is { id: Int };",
        "type Token is { id: Int, pad: Int };",
    ] {
        let module = codegen
            .compile_module(&Parser::new(source).parse_module().unwrap())
            .unwrap();
        let descriptor = declared(&module, "Token");
        let expected = if source.contains("affine") {
            D::Affine
        } else {
            D::Unrestricted
        };
        assert_eq!(descriptor.resource_discipline, expected);
        assert_eq!(effective(&module, "Token"), expected);
    }
}

#[test]
fn actual_function_specialization_preserves_instantiated_resource_arguments() {
    use verum_vbc::{
        bytecode::decode_instructions,
        instruction::Instruction,
        mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
    };
    let mut module = roundtrip(&compile(
        "type affine Token is { id: Int }; type Borrow is &Token; type Holder<T> is { value: T }; fn relay<T>(value: T) -> T { value } fn owned_probe(value: Holder<Token>) -> Holder<Token> { relay<Holder<Token>>(value) } fn ordinary_probe(value: Holder<Int>) -> Holder<Int> { relay<Holder<Int>>(value) } fn borrowed_probe(value: Holder<Borrow>) -> Holder<Borrow> { relay<Holder<Borrow>>(value) }",
    ));
    for function in &mut module.functions {
        function.is_generic = !function.type_params.is_empty();
        // The compiler supplies decoded bodies to the mono merger. The
        // archive roundtrip above deliberately discarded this derived cache.
        let body = &module.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        function.instructions = Some(decode_instructions(body).unwrap());
    }
    let mut graph = InstantiationGraph::new();
    for function in &module.functions {
        let body = &module.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        discover_call_instantiations(
            &module,
            &decode_instructions(body).unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .unwrap();
    }
    assert!(!graph.is_empty(), "the source must exercise specialization");
    let specialized = monomorphize_minimal(module, &graph).unwrap();
    assert_eq!(specialized.metrics.new_specializations, 3);
    let module = roundtrip(&specialized.module);
    for (name, expected) in [
        ("owned_probe", D::Affine),
        ("ordinary_probe", D::Unrestricted),
        ("borrowed_probe", D::Unrestricted),
    ] {
        let caller = module
            .get_function(module.find_function_by_name(name).unwrap())
            .unwrap();
        let body = &module.bytecode[caller.bytecode_offset as usize
            ..(caller.bytecode_offset + caller.bytecode_length) as usize];
        let calls: Vec<_> = decode_instructions(body)
            .unwrap()
            .into_iter()
            .filter_map(|instruction| match instruction {
                Instruction::Call { func_id, .. }
                | Instruction::TailCall { func_id, .. }
                | Instruction::CallG { func_id, .. } => Some(func_id),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls.len(),
            1,
            "{name} must select one specialized callee: {:?}",
            decode_instructions(body).unwrap()
        );
        let callee = module
            .get_function(verum_vbc::FunctionId(calls[0]))
            .unwrap();
        assert!(callee.type_params.is_empty(), "{name}: {callee:?}");
        assert_eq!(
            module.resource_discipline(&callee.return_type),
            expected,
            "{name} return"
        );
        assert_eq!(
            module.resource_discipline(&callee.params[0].type_ref),
            expected,
            "{name} parameter"
        );
    }
}
