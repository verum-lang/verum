//! Value-parameter domains must survive wire, archive publication and sidecar decoding.
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
};

const OWNER: &str = "core.value_telescope";
const SOURCE: &str =
    "module core.value_telescope; public type PointCell<N: meta Int{>=0}> is { public value: () };";

fn metadata(interface: bool) -> CoreMetadata {
    metadata_from(SOURCE, interface)
}

fn metadata_from(source: &str, interface: bool) -> CoreMetadata {
    let ast = Parser::new(source)
        .parse_module()
        .expect("source declaration grammar");
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(OWNER));
    let source = if interface {
        producer.collect_declaration_interface(&[&ast])
    } else {
        producer.compile_module(&ast)
    }
    .expect("ordinary declaration producer");
    let bytes = verum_vbc::serialize::serialize_module(&source).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let mut builder = ArchiveBuilder::new();
    builder.add_module(OWNER, &wire, &[]).unwrap();
    let mut archive_bytes = List::new();
    write_archive(&builder.finish(), &mut archive_bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(archive_bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).expect("archive publication");
    let sidecar = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
    verum_types::core_metadata::decode_metadata(&sidecar).unwrap()
}

fn errors(metadata: &CoreMetadata, consumer: &str, eager: bool) -> List<Text> {
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str(OWNER),
        Parser::new("").parse_module().unwrap(),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(&format!("mount {OWNER}.{{PointCell}}; {consumer}"))
        .parse_module()
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}").into()),
    );
    errors
}

fn cases(consumer: &str, expected_error: Option<&str>) {
    for interface in [false, true] {
        let metadata = metadata(interface);
        for eager in [false, true] {
            let errors = errors(&metadata, consumer, eager);
            if let Some(expected) = expected_error {
                assert!(
                    errors.iter().any(|e| e.contains(expected)),
                    "interface={interface} eager={eager} consumer={consumer}: {errors:?}"
                );
            } else {
                assert!(
                    errors.is_empty(),
                    "interface={interface} eager={eager} consumer={consumer}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn valid_constant_survives_archive() {
    cases("fn consume(x: &PointCell<0>) {}", None);
}
#[test]
fn invalid_constant_keeps_its_domain_obligation() {
    cases(
        "fn consume(x: &PointCell<-1>) {}",
        Some("MetaRefinementViolation"),
    );
}
#[test]
fn wrong_value_sort_is_not_an_ordinary_type_argument() {
    cases(
        "fn consume<M: meta Bool>(x: &PointCell<M>) {}",
        Some("wrong kind"),
    );
}
#[test]
fn stronger_symbolic_domain_survives_archive() {
    cases(
        "fn consume<M: meta Int{>=7}>(x: &PointCell<M>) -> Int { M }",
        None,
    );
}
#[test]
fn weaker_symbolic_domain_remains_unproved() {
    cases(
        "fn consume<M: meta Int>(x: &PointCell<M>) {}",
        Some("cannot establish the declared value domain"),
    );
}

#[test]
fn nominal_meta_and_legacy_const_headers_survive_exactly() {
    for source in [
        SOURCE,
        "module core.value_telescope; public type PointCell<const N: Int> is { public value: () };",
        "module core.value_telescope; public type PointCell<N: meta Int{>=0}> is Empty | Occupied;",
    ] {
        let ast = Parser::new(source)
            .parse_module()
            .expect("source header grammar");
        let declaration = ast
            .items
            .iter()
            .find_map(|item| match &item.kind {
                verum_ast::ItemKind::Type(declaration) => Some(declaration),
                _ => None,
            })
            .expect("source nominal declaration");
        for interface in [false, true] {
            let metadata = metadata_from(source, interface);
            let exported = metadata
                .types
                .get(&Text::from(format!("{OWNER}.PointCell")))
                .expect("exact exported declaration owner");
            let carrier = exported
                .source_declaration
                .as_ref()
                .expect("value telescope must not be reduced to unkinded runtime slots");
            assert_eq!(
                carrier.generics, declaration.generics,
                "interface={interface}: {source}"
            );
        }
    }
}
