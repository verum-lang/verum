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
fn associated_storage_follows_its_receiver_and_requires_an_applicable_equation() {
    use verum_vbc::{copy_plan, drop_plan};
    let module = compile(r#"
        type Producer is protocol { type Output; };
        type Other is protocol { type Output; };
        type Required is protocol {};
        type affine Ticket is { id: Int };
        type Plain is ();
        type Restricted is ();
        type Ambiguous is ();
        type Conditional<T> is { value: T };
        implement Producer for Plain { type Output = Text; }
        implement Producer for Restricted { type Output = Ticket; }
        implement Producer for Ambiguous { type Output = Int; }
        implement Other for Ambiguous { type Output = Ticket; }
        implement<T: Required> Producer for Conditional<T> { type Output = Int; }
        type Cell<P: Producer> is { value: P.Output };
    "#);
    let mut module = roundtrip(&module);
    let cell = declared(&module, "Cell").id;
    let conditional = declared(&module, "Conditional").id;
    let named = |name| TypeRef::Concrete(declared(&module, name).id);
    let plain = TypeRef::instantiated(cell, [named("Plain")]);
    let restricted = TypeRef::instantiated(cell, [named("Restricted")]);
    let ambiguous_owner = declared(&module, "Ambiguous").id;
    let other = declared(&module, "Other").id;
    let project = |qualifier| TypeRef::AssociatedProjection {
        base: Box::new(TypeRef::Concrete(ambiguous_owner)),
        qualifier,
        assoc: "Output".into(),
        args: vec![],
    };
    let ambiguous = project(None);
    let qualified = project(Some(verum_vbc::types::ProtocolApplication {
        protocol: verum_vbc::types::ProtocolId(declared(&module, "Producer").id.0),
        args: vec![],
    }));
    let blocked = TypeRef::instantiated(cell, [TypeRef::instantiated(conditional, [TypeRef::Concrete(TypeId::INT)])]);
    assert_eq!(module.resource_discipline(&plain), D::Unrestricted);
    assert!(copy_plan::build(&module, &plain).is_ok());
    let drop_plan::DropPlan::Record { fields, .. } = drop_plan::build(&module, &plain).unwrap()
        else { panic!("expected structural cleanup"); };
    assert!(matches!(fields.as_slice(), [drop_plan::DropPlan::Text]));
    assert_eq!(module.resource_discipline(&restricted), D::Affine);
    assert!(copy_plan::build(&module, &restricted).is_err());

    // A carried result cannot prove the source implementation's requirement.
    let name = module.intern_string("Output");
    let owner = module.types.iter_mut().find(|ty| ty.id == conditional).unwrap();
    owner.protocols[0].associated_types = vec![(name, TypeRef::Concrete(TypeId::INT))];
    let plain_id = declared(&module, "Plain").id;
    let plain_owner = module.types.iter_mut().find(|ty| ty.id == plain_id).unwrap();
    plain_owner.protocols[0].associated_types.clear();
    let owner = module.types.iter_mut().find(|ty| ty.id == ambiguous_owner).unwrap();
    owner.protocols.iter_mut().find(|row| row.protocol.0 == other.0).unwrap().associated_types.clear();
    for module in [&module, &roundtrip(&module)] {
        assert_eq!(module.resource_discipline(&qualified), D::Unrestricted);
        assert!(copy_plan::build(module, &qualified).is_ok());
        assert!(matches!(drop_plan::build(module, &qualified), Ok(drop_plan::DropPlan::Ignore)));
        for ty in [&blocked, &plain, &ambiguous] {
            assert_eq!(module.resource_discipline(ty), D::Unknown);
            assert!(copy_plan::build(module, ty).is_err());
            assert!(drop_plan::build(module, ty).is_err());
        }
    }
}

#[test]
fn missing_or_recursive_provenance_does_not_become_copy_permission() {
    let syntax = Parser::new("type Holder is { missing: Unresolved };").parse_module().unwrap();
    assert!(VbcCodegen::new().compile_module(&syntax).is_err());
    let mut module = compile("type Element is (); type Holder is { missing: Element }; type Cycle is { next: Cycle };");
    let element = declared(&module, "Element").id;
    module.types.retain(|declaration| declaration.id != element);
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

// Frozen type-descriptor wire from v2.17 (204121137) and v2.18 (803158916).
// Both archives contain one unit type. Only v2.18 carries resource discipline.
fn historical_owner(discipline: Option<u8>) -> verum_common::List<u8> {
    const HEADER: usize = 96;
    let mut bytes = verum_common::List::from(vec![0; HEADER]);
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved empty string
    bytes.extend_from_slice(&5u32.to_le_bytes());
    bytes.extend_from_slice(b"Owned");
    let start = bytes.len() as u32;
    bytes.extend_from_slice(&3000u32.to_le_bytes()); // declaration id
    bytes.extend_from_slice(&4u32.to_le_bytes()); // Owned
    bytes.extend_from_slice(&[6, 1]); // unit, public
    bytes.extend_from_slice(&0u32.to_le_bytes()); // size
    bytes.extend_from_slice(&1u32.to_le_bytes()); // alignment
    // No parameters, fields, variants, drop, clone, protocols, alias, wrapper,
    // alias name, declaring submodule or explicit target layout.
    bytes.extend_from_slice(&[0; 11]);
    bytes.extend(discipline);
    let code = bytes.len() as u32;
    bytes.push(0); // uncompressed empty bytecode
    let end = bytes.len() as u32;
    let wire = bytes.as_mut_slice();
    wire[..4].copy_from_slice(b"VBC1");
    wire[4..6].copy_from_slice(&2u16.to_le_bytes());
    wire[6..8].copy_from_slice(&(if discipline.is_some() { 18u16 } else { 17u16 }).to_le_bytes());
    for (offset, value) in [
        (12, 4), (16, start), (20, 1), (24, code), (32, code), (40, HEADER as u32),
        (44, start - HEADER as u32), (48, code), (52, 1), (56, end),
    ] {
        wire[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let hash = blake3::hash(&wire[HEADER..]);
    wire[72..80].copy_from_slice(&hash.as_bytes()[..8]);
    wire[80..88].copy_from_slice(&blake3::hash(&[]).as_bytes()[..8]);
    bytes
}

#[test]
fn legacy_descriptor_without_resource_byte_remains_unknown() {
    let legacy = deserialize_module(&historical_owner(None)).unwrap();
    assert_eq!(declared(&legacy, "Owned").resource_discipline, D::Unknown);
    assert_eq!(effective(&legacy, "Owned"), D::Unknown);
    let explicit = deserialize_module(&historical_owner(Some(D::Affine as u8))).unwrap();
    assert_eq!(effective(&explicit, "Owned"), D::Affine);
}

#[test]
fn invalid_resource_wire_value_is_rejected() {
    let error = deserialize_module(&historical_owner(Some(255))).unwrap_err();
    assert!(matches!(error, verum_vbc::VbcError::InvalidHeader { field: "resource_discipline", .. }), "{error}");
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

#[test]
fn builtin_method_descriptors_do_not_erase_intrinsic_primitive_discipline() {
    let mut module = compile("type affine Socket is { fd: Int, enabled: Bool };");
    for raw in 0..TypeId::FIRST_USER {
        let id = TypeId(raw);
        if id == TypeId::PTR || id == TypeId::RESERVED { continue; }
        module.types.push(TypeDescriptor {
            id,
            kind: verum_vbc::types::TypeKind::Primitive,
            // Inherent/protocol method registration may create a descriptor
            // without a source resource modifier for this intrinsic type.
            resource_discipline: D::Unknown,
            ..TypeDescriptor::default()
        });
    }
    for module in [module.clone(), roundtrip(&module)] {
        assert_eq!(effective(&module, "Socket"), D::Affine);
        for raw in 0..TypeId::FIRST_USER {
            let id = TypeId(raw);
            let expected = if id == TypeId::PTR || id == TypeId::RESERVED { D::Unknown } else { D::Unrestricted };
            assert_eq!(module.resource_discipline(&TypeRef::Concrete(id)), expected, "{id:?}");
        }
        assert_eq!(module.resource_discipline(&TypeRef::Instantiated {
            base: TypeId::INT, args: vec![TypeRef::Concrete(TypeId::BOOL)],
        }), D::Unknown, "a primitive cannot acquire generic arguments");
    }
}
