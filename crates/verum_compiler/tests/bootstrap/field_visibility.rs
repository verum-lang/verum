//! T1714: real parsed bootstrap output, archive wire and metadata projection.
use super::*;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::core_metadata::{CoreMetadata, TypeDescriptorKind};
use verum_vbc::archive::{ArchiveBuilder, VbcArchive};
#[path = "../../../verum_vbc/tests/fixtures/field_visibility.rs"]
mod fixture;

fn archive() -> VbcArchive {
    let ast = Parser::new(fixture::SOURCE)
        .parse_module()
        .expect("fixture grammar");
    let mut session = crate::Session::new(Default::default());
    let config = CoreConfig::new(".");
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let unit = StdlibModule {
        name: fixture::OWNER.into(),
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
    let mut builder = ArchiveBuilder::new();
    builder
        .add_module(fixture::OWNER, &module, &[])
        .expect("archive module");
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).expect("archive encode");
    verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice()))
        .expect("archive decode")
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
        descriptor.origin_module_path.as_deref(),
        Some(fixture::OWNER)
    );
    let TypeDescriptorKind::Record { fields } = &descriptor.kind else {
        panic!("expected record")
    };
    assert_eq!(fields.len(), 9, "every declared field is retained");
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
