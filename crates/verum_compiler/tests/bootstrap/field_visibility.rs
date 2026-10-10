//! T1714: real parsed bootstrap output, archive wire and metadata projection.
use super::*;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::core_metadata::{CoreMetadata, TypeDescriptorKind};
use verum_vbc::archive::{ArchiveBuilder, VbcArchive};
#[path = "../../../verum_vbc/tests/fixtures/field_visibility.rs"]
mod fixture;

fn source_module(owner: &str, source: &str) -> verum_vbc::module::VbcModule {
    let ast = Parser::new(source).parse_module().expect("fixture grammar");
    let mut session = crate::Session::new(Default::default());
    let config = CoreConfig::new(".");
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let unit = StdlibModule {
        name: owner.into(),
        source_files: List::new().into(),
        dependencies: List::new().into(),
    };
    let (module, _) = pipeline
        .compile_core_module_from_ast(
            &unit,
            &[&ast],
            &config,
            &verum_ast::cfg::TargetConfig::host(),
            &Default::default(),
        )
        .expect("actual bootstrap source producer");
    module
}

fn archive_modules(modules: &[(&str, &verum_vbc::module::VbcModule)]) -> VbcArchive {
    let mut builder = ArchiveBuilder::new();
    for (entry, module) in modules {
        builder
            .add_module(entry, module, &[])
            .expect("archive module");
    }
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).expect("archive encode");
    verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice()))
        .expect("archive decode")
}

fn archive() -> VbcArchive {
    let module = source_module(fixture::OWNER, fixture::SOURCE);
    archive_modules(&[(fixture::OWNER, &module)])
}

fn metadata(archive: &VbcArchive) -> CoreMetadata {
    let metadata = crate::archive_metadata::archive_to_core_metadata(archive);
    bincode::deserialize(&bincode::serialize(&metadata).expect("metadata encode"))
        .expect("metadata decode")
}

#[test]
fn parsed_private_and_restricted_fields_are_not_public_after_archive_metadata() {
    let archive = archive();
    let metadata = crate::archive_metadata::archive_to_core_metadata(&archive);
    let metadata: CoreMetadata =
        bincode::deserialize(&bincode::serialize(&metadata).expect("metadata encode"))
            .expect("metadata decode");
    let descriptor = metadata
        .types
        .get(&Text::from(format!("{}.Vault", fixture::OWNER)))
        .expect("exact metadata owner");
    assert_eq!(
        descriptor
            .origin_module_path
            .as_deref()
            .unwrap_or(descriptor.module_path.as_str()),
        fixture::OWNER
    );
    let TypeDescriptorKind::Record { fields } = &descriptor.kind else {
        panic!("expected record")
    };
    assert_eq!(fields.len(), 12, "every declared field is retained");
    let mut failures: List<Text> = List::new();
    for field in fields {
        let expected = field.name == "visible";
        if field.is_public != expected {
            failures.push(
                format!(
                    "{}: expected is_public={expected}, found {}",
                    field.name, field.is_public
                )
                .into(),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn complete_record_and_variant_policy_reaches_span_free_metadata() {
    use verum_ast::{
        ItemKind,
        decl::{TypeDeclBody, VariantData},
    };
    use verum_types::core_metadata::VariantPayload;
    let metadata = metadata(&archive());
    let ast = Parser::new(fixture::SOURCE).parse_module().unwrap();
    let mut count = 0;
    for item in &ast.items {
        let ItemKind::Type(decl) = &item.kind else {
            continue;
        };
        let descriptor = metadata
            .types
            .get(&Text::from(format!("{}.{}", fixture::OWNER, decl.name)))
            .expect("exact declared metadata owner");
        let pairs: List<(
            &verum_ast::decl::RecordField,
            &verum_types::core_metadata::FieldDescriptor,
        )> = match (&decl.body, &descriptor.kind) {
            (TypeDeclBody::Record(source), TypeDescriptorKind::Record { fields }) => {
                source.iter().zip(fields.iter()).collect()
            }
            (TypeDeclBody::Variant(source), TypeDescriptorKind::Variant { cases }) => source
                .iter()
                .zip(cases.iter())
                .flat_map(|(source, case)| match (&source.data, &case.payload) {
                    (Some(VariantData::Record(source)), Some(VariantPayload::Record(fields))) => {
                        source.iter().zip(fields.iter()).collect()
                    }
                    (None, None) => List::new(),
                    _ => panic!("variant payload shape lost"),
                })
                .collect(),
            _ => panic!("declared type shape lost"),
        };
        for (source, field) in pairs {
            assert_eq!(field.name, source.name.name);
            assert_eq!(
                field.declared_visibility,
                Some(
                    source
                        .visibility
                        .resolve_declared_scope(fixture::OWNER)
                        .unwrap()
                ),
                "{}",
                field.name
            );
            count += 1;
        }
    }
    assert_eq!(count, 16);
}

#[test]
fn same_leaf_fields_keep_declaring_scope_in_both_archive_orders() {
    let left = source_module(
        "fixture.left",
        "module fixture.left; public type Vault is { public(in self.scope) value: Int };",
    );
    let right = source_module(
        "fixture.right",
        "module fixture.right; public type Vault is { private value: Int };",
    );
    for modules in [
        [("bundle.left", &left), ("bundle.right", &right)],
        [("bundle.right", &right), ("bundle.left", &left)],
    ] {
        let metadata = metadata(&archive_modules(&modules));
        for (owner, expected) in [
            ("fixture.left", Some("fixture::left::scope")),
            ("fixture.right", None),
        ] {
            let descriptor = metadata
                .types
                .get(&Text::from(format!("{owner}.Vault")))
                .expect("both exact owners exist");
            assert_eq!(descriptor.origin_module_path.as_deref(), Some(owner));
            let TypeDescriptorKind::Record { fields } = &descriptor.kind else {
                panic!("record");
            };
            assert_eq!(fields.len(), 1);
            match (&fields[0].declared_visibility, expected) {
                (Some(verum_ast::Visibility::PublicIn(path)), Some(expected)) => {
                    assert_eq!(path.to_string(), expected)
                }
                (Some(verum_ast::Visibility::Private), None) => {}
                pair => panic!("declaration policy changed: {pair:?}"),
            }
        }
    }
}

#[test]
fn missing_field_policy_stays_unknown_even_with_a_public_compatibility_flag() {
    let mut module = source_module(fixture::OWNER, fixture::SOURCE);
    let mut count = 0;
    for ty in &mut module.types {
        for field in ty.fields.iter_mut().chain(
            ty.variants
                .iter_mut()
                .flat_map(|variant| variant.fields.iter_mut()),
        ) {
            field.declared_visibility = None;
            field.visibility = verum_vbc::types::Visibility::Public;
            count += 1;
        }
    }
    assert_eq!(count, 16);
    let metadata = metadata(&archive_modules(&[(fixture::OWNER, &module)]));
    let record = metadata
        .types
        .get(&Text::from(format!("{}.Vault", fixture::OWNER)))
        .unwrap();
    let TypeDescriptorKind::Record { fields } = &record.kind else {
        panic!("record");
    };
    assert!(
        fields
            .iter()
            .all(|field| field.is_public && field.declared_visibility.is_none())
    );
}
