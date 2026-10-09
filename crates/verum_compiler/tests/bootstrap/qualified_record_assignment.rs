//! T1692: real record sources cross archive and metadata wires before field use.
use super::*;
use crate::Session;
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Set, Text};
use verum_fast_parser::FastParser;
use verum_types::{
    Type, TypeChecker,
    core_metadata::{CoreMetadata, TypeDescriptorKind},
};
use verum_vbc::{
    archive::{ArchiveBuilder, VbcArchive},
    types::{TypeId, TypeRef},
};

const SOURCES: [(&str, &str); 2] = [
    (
        "core.field_alpha",
        "public type Packet is { public body: Int, public alpha_only: Int }; implement Packet { public fn new(body: Int) -> Packet { Packet { body, alpha_only: 3 } } }",
    ),
    (
        "core.field_beta",
        "public type Packet is { public body: Bool, public beta_only: Int }; implement Packet { public fn new(body: Bool) -> Packet { Packet { body, beta_only: 5 } } }",
    ),
];

fn parse(source: &str, file: u32) -> verum_ast::Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("fixture grammar")
}

fn archive(reverse: bool) -> VbcArchive {
    let mut sources = SOURCES;
    if reverse {
        sources.reverse();
    }
    let sources: List<_> = sources
        .into_iter()
        .enumerate()
        .map(|(index, (owner, source))| {
            (
                owner,
                parse(&format!("module {owner}; {source}"), index as u32),
            )
        })
        .collect();
    let mut session = Session::new(Default::default());
    let config = CoreConfig::new(".");
    {
        let registry = session.module_registry();
        let mut registry = registry.write();
        for (owner, ast) in &sources {
            let id = registry.allocate_id();
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, Text::new());
            info.exports = extract_exports_from_module(ast, id, &path).expect("source exports");
            registry.register(info);
        }
        resolve_specific_reexport_kinds(&mut registry).unwrap();
        resolve_glob_reexports(&mut registry).unwrap();
    }
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let mut builder = ArchiveBuilder::new();
    for (owner, ast) in &sources {
        let unit = StdlibModule {
            name: (*owner).into(),
            source_files: List::new().into(),
            dependencies: List::new().into(),
        };
        let (module, _) = pipeline
            .compile_core_module_from_ast(
                &unit,
                &[ast],
                &config,
                &verum_ast::cfg::TargetConfig::host(),
                &Default::default(),
            )
            .expect("actual bootstrap accepts fixture source");
        builder
            .add_module(owner, &module, &[])
            .expect("archive publication");
        pipeline
            .compiled_stdlib_modules
            .insert((*owner).into(), module);
    }
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes)
        .expect("archive serialization");
    verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice()))
        .expect("archive decoding")
}

fn metadata(archive: &VbcArchive) -> CoreMetadata {
    let metadata = crate::archive_metadata::archive_to_core_metadata(archive);
    bincode::deserialize(&bincode::serialize(&metadata).expect("metadata serialization"))
        .expect("metadata decoding")
}

fn assert_producer_fields(archive: &VbcArchive, metadata: &CoreMetadata) {
    for (owner, expected, own_field) in [
        ("core.field_alpha", TypeId::INT, "alpha_only"),
        ("core.field_beta", TypeId::BOOL, "beta_only"),
    ] {
        let module = archive
            .load_module(owner)
            .expect("source owner archive unit");
        let descriptor = module
            .types
            .iter()
            .find(|descriptor| {
                descriptor
                    .origin_module
                    .and_then(|id| module.get_string(id))
                    == Some(owner)
                    && module
                        .get_string(descriptor.name)
                        .is_some_and(|name| name.rsplit('.').next() == Some("Packet"))
            })
            .expect("declaring record in archive");
        let body = descriptor
            .fields
            .iter()
            .find(|field| module.get_string(field.name) == Some("body"))
            .expect("archive body field");
        assert_eq!(
            body.type_ref,
            TypeRef::Concrete(expected),
            "{owner}: archive body type"
        );
        let descriptor = metadata
            .types
            .get(&Text::from(format!("{owner}.Packet")))
            .expect("exact metadata owner");
        let TypeDescriptorKind::Record { fields } = &descriptor.kind else {
            panic!("not a record: {descriptor:?}")
        };
        let body = fields
            .iter()
            .find(|field| field.name == "body")
            .expect("metadata body field");
        assert_eq!(
            body.ty.as_str(),
            expected.well_known_name().unwrap(),
            "{owner}: metadata body type"
        );
        assert!(body.is_public, "fixture explicitly declares public fields");
        assert!(
            fields
                .iter()
                .any(|field| field.name == own_field && field.is_public)
        );
    }
}

fn check(metadata: CoreMetadata, source: &str, eager: bool) -> (TypeChecker, List<Text>) {
    let mut registry = verum_modules::ModuleRegistry::new();
    let mut owners = Set::new();
    for descriptor in metadata.types.values() {
        owners.insert(descriptor.module_path.clone());
        if let Some(owner) = &descriptor.origin_module_path {
            owners.insert(owner.clone());
        }
    }
    // No source type AST survives on this path. Public metadata is the only
    // authority available for the consumed declarations and field maps.
    for owner in &owners {
        let id = registry.allocate_id();
        registry.register(ModuleInfo::new(
            id,
            ModulePath::from_str(owner),
            parse("", id.as_u32()),
            FileId::new(id.as_u32()),
            Text::new(),
        ));
    }
    let metadata = Arc::new(metadata);
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    let ast = parse(source, owners.len() as u32);
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    (checker, errors)
}

fn field_maps(checker: &TypeChecker) -> Text {
    [
        "__struct_fields_Packet",
        "__struct_fields_core.field_alpha.Packet",
        "core.field_alpha.__struct_fields_Packet",
        "__struct_fields_core.field_beta.Packet",
        "core.field_beta.__struct_fields_Packet",
    ]
    .into_iter()
    .map(|key| Text::from(format!("{key}={:?}", checker.lookup_type_for_testing(key))))
    .collect::<List<_>>()
    .join("; ")
    .into()
}

#[derive(Clone, Copy, Debug)]
enum Verdict {
    Accept,
    Mismatch,
    MissingField,
}

fn cases(bodies: &[&str], expected: Verdict) {
    let mut failures: List<Text> = List::new();
    for reverse in [false, true] {
        let archive = archive(reverse);
        let metadata = metadata(&archive);
        assert_producer_fields(&archive, &metadata);
        let mounts = if reverse {
            "mount core.field_beta.{Packet as B}; mount core.field_alpha.{Packet as A};"
        } else {
            "mount core.field_alpha.{Packet as A}; mount core.field_beta.{Packet as B};"
        };
        for eager in [false, true] {
            for body in bodies {
                let (checker, errors) = check(metadata.clone(), &format!("{mounts} {body}"), eager);
                let missing = errors
                    .iter()
                    .any(|error| error.contains("not found") || error.contains("UnknownField"));
                let matches = match expected {
                    Verdict::Accept => errors.is_empty(),
                    Verdict::Mismatch => {
                        !missing && errors.iter().any(|error| error.contains("Mismatch"))
                    }
                    Verdict::MissingField => missing,
                };
                if !matches {
                    failures.push(format!("reverse={reverse}, eager={eager}, expected={expected:?}, source={body}: {errors:?}; {}", field_maps(&checker)).into());
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn parsed_record_fields_survive_archive_and_metadata_wires_in_both_orders() {
    for reverse in [false, true] {
        let archive = archive(reverse);
        assert_producer_fields(&archive, &metadata(&archive));
    }
}

#[test]
fn archived_record_reads_preserve_the_declared_field_owner() {
    cases(
        &["fn first(value: &A) -> Int { value.body } fn second(value: &B) -> Bool { value.body }"],
        Verdict::Accept,
    );
}

#[test]
fn archived_record_writes_preserve_the_declared_field_owner() {
    cases(
        &[
            "fn first(value: &mut A) { value.body = 7; } fn second(value: &mut B) { value.body = true; }",
        ],
        Verdict::Accept,
    );
}

#[test]
fn archived_constructor_result_allows_field_read() {
    cases(
        &["fn first() -> Int { A.new(1).body } fn second() -> Bool { B.new(false).body }"],
        Verdict::Accept,
    );
}

#[test]
fn archived_constructor_result_allows_field_write_and_read() {
    cases(
        &[
            "fn first() -> Int { let mut value = A.new(1); value.body = 7; value.body } fn second() -> Bool { let mut value = B.new(false); value.body = true; value.body }",
        ],
        Verdict::Accept,
    );
}

#[test]
fn archived_record_writes_refuse_the_wrong_declared_field_type() {
    cases(
        &[
            "fn wrong(value: &mut A) { value.body = true; }",
            "fn wrong(value: &mut B) { value.body = 7; }",
        ],
        Verdict::Mismatch,
    );
}

#[test]
fn archived_record_read_and_write_refuse_sibling_only_fields() {
    cases(
        &[
            "fn wrong(value: &A) -> Int { value.beta_only }",
            "fn wrong(value: &mut A) { value.beta_only = 7; }",
            "fn wrong(value: &B) -> Int { value.alpha_only }",
            "fn wrong(value: &mut B) { value.alpha_only = 7; }",
        ],
        Verdict::MissingField,
    );
}

#[test]
fn archived_record_owner_cannot_be_replaced_by_a_sibling() {
    cases(&["fn wrong(value: A) -> B { value }"], Verdict::Mismatch);
}
