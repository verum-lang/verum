//! A source interface may import its own earlier interface through dependencies.
#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Module};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenError, CodegenErrorKind, SkipClass, VbcCodegen},
    module::VbcModule,
    types::{TypeId, TypeRef},
};

fn parsed(owner: &str, declaration: &str) -> Module {
    Parser::new(&format!("module {owner}; {declaration}"))
        .parse_module()
        .unwrap()
}

fn interface(owner: &str, ast: &Module) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .collect_declaration_interface(&[ast])
        .unwrap()
}

fn complete(owner: &str, imported: &VbcModule, ast: &Module) -> Result<VbcModule, CodegenError> {
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(owner));
    producer.import_archive_module_types(imported);
    producer.collect_declaration_interface(&[ast])
}

fn assert_fatal(error: CodegenError) {
    assert_eq!(error.skip_class(), SkipClass::Fatal);
    assert!(matches!(
        error.kind,
        CodegenErrorKind::InvalidSourceSignature(_)
    ));
}

#[test]
fn actual_glue_declaration_completes_its_imported_interface_once() {
    let mut ast = Parser::new(include_str!("../../../core/math/cubical.vr"))
        .parse_module()
        .unwrap();
    // Keep the real declaring module and exact Glue AST, including value
    // parameters and meta-type body, without unrelated function dependencies.
    ast.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || matches!(&item.kind, ItemKind::Type(declaration) if declaration.name.name.as_str() == "Glue")
    });
    let first = interface("core.math", &ast);
    let completed = complete("core.math", &first, &ast).unwrap();
    let declarations: Vec<_> = completed
        .types
        .iter()
        .filter_map(|descriptor| descriptor.source_declaration.as_ref())
        .collect();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name.name.as_str(), "Glue");
    assert_eq!(declarations[0].value_params.len(), 3);
    assert_eq!(
        completed
            .types
            .iter()
            .filter(|descriptor| {
                completed
                    .get_string(descriptor.name)
                    .is_some_and(|name| name.rsplit('.').next() == Some("Glue"))
            })
            .count(),
        1
    );
}

#[test]
fn source_completion_preserves_records_aliases_and_protocols() {
    for declaration in [
        "public type Carrier<A> is { value: Type(1), extra: A };",
        "public type Carrier<A> is fn(A) -> Type(1);",
        "public type Carrier<A> is protocol { fn value(&self, input: A) -> Type(1); };",
    ] {
        let ast = parsed("owner", declaration);
        let imported = interface("owner", &ast);
        let completed = complete("owner", &imported, &ast).unwrap();
        let carriers: Vec<_> = completed
            .types
            .iter()
            .filter(|descriptor| descriptor.source_declaration.is_some())
            .collect();
        assert_eq!(carriers.len(), 1, "{declaration}");
        assert_eq!(
            carriers[0].source_declaration,
            imported
                .types
                .iter()
                .find(|descriptor| descriptor.source_declaration.is_some())
                .unwrap()
                .source_declaration
        );
    }
}

#[test]
fn a_different_source_owner_keeps_a_distinct_identity() {
    let left = parsed("left", "public type Carrier is { value: Type(2) };");
    let right = parsed("right", "public type Carrier is { value: Type(1) };");
    let completed = complete("right", &interface("left", &left), &right).unwrap();
    let carriers: Vec<_> = completed
        .types
        .iter()
        .filter(|descriptor| descriptor.source_declaration.is_some())
        .collect();
    assert_eq!(carriers.len(), 2);
    assert_ne!(carriers[0].id, carriers[1].id);
}

#[test]
fn source_completion_rejects_changed_semantics_or_missing_evidence() {
    let first = parsed("owner", "public type Carrier is { value: Type(1) };");
    let changed = parsed("owner", "public type Carrier is { value: Type(2) };");
    let imported = interface("owner", &first);
    assert_fatal(complete("owner", &imported, &changed).unwrap_err());

    let mut abi_only = imported.clone();
    for descriptor in &mut abi_only.types {
        descriptor.source_declaration = None;
    }
    assert_fatal(complete("owner", &abi_only, &first).unwrap_err());

    let ordinary = parsed("owner", "public type Carrier is { value: Int };");
    assert_fatal(complete("owner", &imported, &ordinary).unwrap_err());
}

#[test]
fn equal_source_ast_does_not_authorize_a_different_imported_abi() {
    let ast = parsed("owner", "public type Carrier is { value: Type(1) };");
    let mut imported = interface("owner", &ast);
    let descriptor = imported
        .types
        .iter_mut()
        .find(|descriptor| descriptor.source_declaration.is_some())
        .unwrap();
    descriptor.fields[0].type_ref = TypeRef::Concrete(TypeId::INT);
    descriptor.fields[0].declaration_type = Some(TypeRef::Concrete(TypeId::INT));
    assert_fatal(complete("owner", &imported, &ast).unwrap_err());
}

fn second_bootstrap_interface(owner: &str, ast: &Module) -> VbcModule {
    let first = interface(owner, ast);
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(owner));
    producer
        .import_bootstrap_nominal_dependencies(&[ast], &[&first])
        .unwrap();
    producer.collect_declaration_interface(&[ast]).unwrap()
}

#[test]
fn bootstrap_reserves_local_protocol_ids_before_remapping_semantic_record_fields() {
    let ast = parsed(
        "owner",
        "
        public type Ordinary<A, N: meta Int> is protocol { fn witness(&self) -> Bool; };
        public type Carrier<N: meta Int> is { carrier: Type, witness: Ordinary<carrier, N> };
        public type Alias is Carrier<0>;
    ",
    );
    let completed = second_bootstrap_interface("owner", &ast);
    for name in ["Ordinary", "Carrier", "Alias"] {
        let descriptors: Vec<_> = completed
            .types
            .iter()
            .filter(|descriptor| {
                completed
                    .get_string(descriptor.name)
                    .is_some_and(|actual| actual.rsplit('.').next() == Some(name))
            })
            .collect();
        assert_eq!(descriptors.len(), 1, "one final identity for {name}");
    }
    let ordinary = completed
        .types
        .iter()
        .find(|descriptor| {
            completed
                .get_string(descriptor.name)
                .is_some_and(|name| name.rsplit('.').next() == Some("Ordinary"))
        })
        .unwrap();
    let record = completed
        .types
        .iter()
        .find(|descriptor| {
            descriptor.origin_module.and_then(|owner| completed.get_string(owner)) == Some("owner")
                && descriptor.source_declaration.as_ref()
                    .is_some_and(|source| source.name.name.as_str() == "Carrier")
        })
        .unwrap();
    let TypeRef::Instantiated { base, .. } = &record.fields[1].type_ref else {
        panic!(
            "ordinary protocol application must retain its nominal head: {:?}",
            record.fields[1].type_ref
        )
    };
    assert_eq!(*base, ordinary.id);
}

#[test]
fn actual_ntype_references_the_completed_local_istrunc_identity() {
    let mut ast = Parser::new(include_str!("../../../core/math/hott.vr"))
        .parse_module()
        .unwrap();
    ast.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || matches!(&item.kind, ItemKind::Type(declaration)
                if ["IsTrunc", "NType", "HProp", "HSet"].contains(&declaration.name.name.as_str()))
    });
    let completed = second_bootstrap_interface("core.math", &ast);
    let mut declarations: Vec<_> = completed
        .types
        .iter()
        .filter_map(|ty| Some((ty.source_declaration.as_ref()?.name.name.as_str(), ty)))
        .collect();
    declarations.sort_by_key(|(name, _)| *name);
    let [("IsTrunc", protocol), ("NType", record)] = declarations.as_slice() else {
        panic!("expected one source declaration each for IsTrunc and NType");
    };
    let witness = record.fields.iter()
        .find(|field| completed.get_string(field.name) == Some("truncation"))
        .expect("NType declares a truncation witness");
    let TypeRef::Instantiated { base, .. } = &witness.type_ref else {
        panic!("the witness must retain its nominal protocol application");
    };
    assert_eq!(*base, protocol.id);
}

#[test]
fn repeated_source_completion_keeps_declared_type_visibility() {
    for (visibility, expected) in [
        ("", verum_vbc::types::Visibility::Private),
        ("public(cog)", verum_vbc::types::Visibility::Cog),
        ("public", verum_vbc::types::Visibility::Public),
    ] {
        for body in [
            "{ value: &Int }",
            "fn(&Int) -> &Int",
            "protocol { fn value(&self) -> &Int; }",
        ] {
            let ast = parsed("owner", &format!("{visibility} type Carrier is {body};"));
            let first = interface("owner", &ast);
            let second = complete("owner", &first, &ast).unwrap();
            let third = complete("owner", &second, &ast).unwrap();
            for module in [&first, &second, &third] {
                let descriptor = module.types.iter()
                    .find(|descriptor| module.get_string(descriptor.name)
                        .is_some_and(|name| name == "Carrier" || name == "owner.Carrier"))
                    .expect("the declared nominal owner exists after every completion");
                assert_eq!(descriptor.visibility, expected, "{visibility}: {body}");
            }
        }
    }
}

#[test]
fn source_completion_rejects_forged_visibility() {
    let ast = parsed("owner", "type Carrier is { value: &Int };");
    let mut altered = interface("owner", &ast);
    altered.types.iter_mut()
        .find(|descriptor| descriptor.source_declaration.is_some()).unwrap()
        .visibility = verum_vbc::types::Visibility::Public;
    assert_fatal(complete("owner", &altered, &ast).unwrap_err());
}

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    ).unwrap()
}

#[test]
fn homonymous_source_interfaces_complete_in_both_orders() {
    // These forms retain exact source declarations. ABI-only sum completion
    // has a separate duplicate-identity defect and cannot authorize this path.
    for body in [
        "{ public value: Int }",
        "(Int)",
        "()",
        "Int { it > 0 }",
        "protocol { fn value(&self) -> Int; }",
    ] {
        let declaration = format!("public type Carrier is {body};");
        let first = parsed("bundle.first", &declaration);
        let second = parsed("bundle.second", &declaration);
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("bundle"));
        let archive = wire(&producer.collect_declaration_interface(&[&first, &second]).unwrap());
        for units in [[&first, &second], [&second, &first]] {
            let mut consumer = VbcCodegen::with_config(CodegenConfig::new("bundle"));
            consumer.import_archive_module_types(&archive);
            let completed = wire(&consumer.collect_declaration_interface(&units)
                .unwrap_or_else(|error| panic!("{body}: {error}")));
            let mut identities = verum_common::List::new();
            for owner in ["bundle.first", "bundle.second"] {
                let carriers: verum_common::List<_> = completed.types.iter().filter(|descriptor| {
                    descriptor.origin_module.and_then(|id| completed.get_string(id)) == Some(owner)
                        && completed.get_string(descriptor.name).is_some_and(|name|
                            verum_vbc::module::qualify_module_name(owner, name) == format!("{owner}.Carrier"))
                }).collect();
                assert_eq!(carriers.len(), 1, "{owner}: {body}");
                let original = archive.types.iter().find(|descriptor| {
                    descriptor.origin_module.and_then(|id| archive.get_string(id)) == Some(owner)
                        && archive.get_string(descriptor.name).is_some_and(|name|
                            verum_vbc::module::qualify_module_name(owner, name) == format!("{owner}.Carrier"))
                }).expect("each source owner was present before completion");
                assert_eq!(carriers[0].source_declaration, original.source_declaration,
                    "completion preserves the same source evidence: {owner}: {body}");
                identities.push(carriers[0].id);
            }
            assert_ne!(identities[0], identities[1], "same spelling cannot merge owners: {body}");
        }
    }
}

#[test]
fn qualified_spelling_never_authorizes_a_changed_runtime_descriptor() {
    let ast = parsed("owner", "public type Carrier is { public value: Int };");
    let original = interface("owner", &ast);
    for changed in ["size", "alignment", "layout", "visibility", "resource", "offset", "payload"] {
        let mut imported = original.clone();
        let qualified = imported.intern_string("owner.Carrier");
        let descriptor = imported.types.iter_mut()
            .find(|descriptor| descriptor.source_declaration.is_some()).unwrap();
        descriptor.name = qualified;
        match changed {
            "size" => descriptor.size += 8,
            "alignment" => descriptor.alignment *= 2,
            "layout" => descriptor.declared_layout.as_mut().unwrap().size += 8,
            "visibility" => descriptor.visibility = verum_vbc::types::Visibility::Private,
            "resource" => descriptor.resource_discipline = verum_common::ResourceDiscipline::Affine,
            "offset" => descriptor.fields[0].offset += 8,
            "payload" => descriptor.fields[0].declaration_type = Some(TypeRef::Concrete(TypeId::BOOL)),
            _ => unreachable!(),
        }
        let error = complete("owner", &imported, &ast)
            .expect_err("a qualified name cannot establish ABI correspondence");
        // Resource policy can reject at source integrity before ABI comparison.
        // Both paths must remain fatal rather than completing the forged header.
        assert_fatal(error);
    }
}
