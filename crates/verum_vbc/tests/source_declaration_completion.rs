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
        .find(|descriptor| descriptor.source_declaration.is_some())
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
