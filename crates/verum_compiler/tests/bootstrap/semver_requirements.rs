//! T1663: complete current SemVer source → bootstrap unit → archive → metadata.
//!
//! This is a producer gate, not a runtime or full-stdlib-bake gate. It uses the
//! normal bootstrap builtin/intrinsic seed, real Result/Maybe/Ordering files,
//! and the normal canonical-method interface prepass over complete Text/List/
//! memory sources. Later dependency bodies are not emitted or executed here.
//! No embedded SemVer bodies, fabricated declarations or edited field policies
//! are used. Every SemVer body must emit rather than become a lenient panic stub.
use super::*;
use crate::Session;
use verum_ast::{
    ItemKind, TypeKind, Visibility,
    decl::{ImplItemKind, ImplKind},
};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::core_metadata::{CoreMetadata, TypeDescriptorKind, VariantPayload};
use verum_vbc::{
    Instruction, VbcModule,
    archive::ArchiveBuilder,
    types::{TypeId, TypeRef},
};

const VERSION_OWNER: &str = "core.base.semver";
const REQUIREMENT_OWNER: &str = "core.base.semver_constraint";
const SOURCES: &[(&str, &str)] = &[
    (
        "core.base.result",
        include_str!("../../../../core/base/result.vr"),
    ),
    (
        "core.base.maybe",
        include_str!("../../../../core/base/maybe.vr"),
    ),
    (
        "core.base.ordering",
        include_str!("../../../../core/base/ordering.vr"),
    ),
    (
        VERSION_OWNER,
        include_str!("../../../../core/base/semver.vr"),
    ),
    (
        REQUIREMENT_OWNER,
        include_str!("../../../../core/base/semver_constraint.vr"),
    ),
];

// These complete files supply the same declaration interfaces that compile_core
// registers before compiling core.base. They are not a linked runtime closure.
const BOOTSTRAP_INTERFACES: &[(&str, &str)] = &[
    (
        "core.text.text",
        include_str!("../../../../core/text/text.vr"),
    ),
    (
        "core.collections.list",
        include_str!("../../../../core/collections/list.vr"),
    ),
    (
        "core.base.memory",
        include_str!("../../../../core/base/memory.vr"),
    ),
];

fn parse_sources(sources: &[(&'static str, &str)]) -> List<(&'static str, verum_ast::Module)> {
    sources
        .iter()
        .map(|(owner, source)| {
            (
                *owner,
                Parser::new(source).parse_module().unwrap_or_else(|error| {
                    panic!("complete current source {owner} does not parse: {error:?}")
                }),
            )
        })
        .collect()
}

fn declared_bodies(owner: &str, ast: &verum_ast::Module) -> List<(Text, Text)> {
    let mut expected = List::new();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Function(function) if function.body.is_some() => {
                expected.push((owner.into(), function.name.name.clone()));
            }
            ItemKind::Impl(implementation) => {
                let target = match &implementation.kind {
                    ImplKind::Inherent(target) => target,
                    ImplKind::Protocol { for_type, .. } => for_type,
                };
                let TypeKind::Path(target) = &target.kind else {
                    panic!("the complete source fixture gained an unhandled impl target");
                };
                for item in &implementation.items {
                    if let ImplItemKind::Function(function) = &item.kind
                        && function.body.is_some()
                    {
                        expected.push((
                            owner.into(),
                            format!("{target}.{}", function.name.name).into(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    assert!(
        !expected.is_empty(),
        "source body inventory cannot be empty"
    );
    expected
}

fn declaration_id(module: &VbcModule, owner: &str, name: &str) -> TypeId {
    module
        .types
        .iter()
        .find(|ty| {
            ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                && module
                    .get_string(ty.name)
                    .is_some_and(|written| written == name || written == format!("{owner}.{name}"))
        })
        .unwrap_or_else(|| panic!("missing exact declared type {owner}.{name}"))
        .id
}

fn assert_bodies(module: &VbcModule, expected: &[(Text, Text)], phase: &str) {
    let mut failures: List<Text> = List::new();
    for (owner, name) in expected {
        let candidates: List<_> = module
            .functions
            .iter()
            .filter(|function| {
                function.origin_module.and_then(|id| module.get_string(id)) == Some(owner.as_str())
                    && module.get_string(function.name).is_some_and(|written| {
                        written == name.as_str() || written == format!("{owner}.{name}")
                    })
            })
            .collect();
        if candidates.len() != 1 {
            failures.push(format!("{owner}.{name}: {} declared bodies", candidates.len()).into());
            continue;
        }
        let function = candidates[0];
        if !function.has_source_body || function.bytecode_length == 0 {
            failures.push(format!("{owner}.{name}: missing source-body provenance/bytes").into());
        }
        // A lenient panic stub clears provenance. Still decode its bytes so a
        // failed gate retains the actual unresolved dependency/body diagnostic.
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let instructions = verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end])
            .expect("selected body must decode independently of cached instructions");
        for instruction in &instructions {
            if let Instruction::Panic { message_id } = instruction
                && module
                    .get_string(verum_vbc::StringId(*message_id))
                    .is_some_and(|message| {
                        message.starts_with("[lenient] ")
                            && message.contains("compiled to panic-stub:")
                    })
            {
                failures.push(
                    format!(
                        "{owner}.{name}: {}",
                        module.get_string(verum_vbc::StringId(*message_id)).unwrap()
                    )
                    .into(),
                );
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{phase} source body failures:\n{}",
        failures.join("\n")
    );
    eprintln!(
        "semver-source-archive: {phase} {} actual bodies",
        expected.len()
    );
}

fn assert_signatures(module: &VbcModule) {
    let constraint = declaration_id(module, REQUIREMENT_OWNER, "SemVerConstraint");
    let error = declaration_id(module, REQUIREMENT_OWNER, "SemVerConstraintError");
    let comparator = declaration_id(module, REQUIREMENT_OWNER, "SemVerComparator");
    let result = declaration_id(module, "core.base.result", "Result");
    let descriptor = module.get_type(constraint).unwrap();
    let payload = descriptor
        .variants
        .iter()
        .find(|variant| module.get_string(variant.name) == Some("Comparator"))
        .expect("parsed comparator is an actual declared variant");
    assert_eq!(payload.fields.len(), 1);
    assert_eq!(
        payload.fields[0].declaration_type,
        Some(TypeRef::Concrete(comparator))
    );
    for name in ["parse", "parse_cargo", "parse_extended"] {
        let function = module
            .functions
            .iter()
            .find(|function| {
                function.origin_module.and_then(|id| module.get_string(id))
                    == Some(REQUIREMENT_OWNER)
                    && module.get_string(function.name)
                        == Some(format!("{REQUIREMENT_OWNER}.{name}").as_str())
            })
            .unwrap_or_else(|| panic!("missing parser entry {name}"));
        assert!(matches!(
            function.visibility,
            verum_vbc::types::Visibility::Public
        ));
        assert_eq!(function.params.len(), 1);
        assert_eq!(
            function.return_type,
            TypeRef::Instantiated {
                base: result,
                args: List::from_iter([TypeRef::Concrete(constraint), TypeRef::Concrete(error)])
                    .into(),
            },
            "{name}: source return authority"
        );
    }
}

fn assert_metadata(metadata: &CoreMetadata) {
    for (owner, name, expected_fields) in [
        (
            VERSION_OWNER,
            "SemVer",
            &["major", "minor", "patch", "pre_release", "build_meta"][..],
        ),
        (
            REQUIREMENT_OWNER,
            "SemVerComparator",
            &["operator", "major", "minor", "patch", "pre_release"][..],
        ),
        (REQUIREMENT_OWNER, "MatchOptions", &["allow_prerelease"][..]),
    ] {
        let descriptor = metadata
            .types
            .get(&Text::from(format!("{owner}.{name}")))
            .unwrap_or_else(|| panic!("missing declared metadata {owner}.{name}"));
        assert_eq!(
            descriptor
                .origin_module_path
                .as_deref()
                .unwrap_or(descriptor.module_path.as_str()),
            owner
        );
        let TypeDescriptorKind::Record { fields } = &descriptor.kind else {
            panic!("declared {owner}.{name} must stay a record");
        };
        assert_eq!(
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<List<_>>()
                .as_slice(),
            expected_fields
        );
        for field in fields {
            assert!(
                !field.is_public,
                "{owner}.{name}.{} was publicized",
                field.name
            );
            assert_eq!(
                field.declared_visibility,
                Some(Visibility::Private),
                "{}.{} declaration policy",
                name,
                field.name
            );
        }
    }
    let descriptor = metadata
        .types
        .get(&Text::from(format!("{REQUIREMENT_OWNER}.SemVerConstraint")))
        .unwrap();
    let TypeDescriptorKind::Variant { cases } = &descriptor.kind else {
        panic!("constraint declaration must remain a variant type");
    };
    let payload = &cases
        .iter()
        .find(|case| case.name == "Comparator")
        .unwrap()
        .payload;
    assert!(
        matches!(payload, Some(VariantPayload::Tuple(types)) if types.len() == 1 && types[0] == "SemVerComparator"),
        "{payload:?}"
    );
}

#[test]
fn current_semver_sources_publish_real_bodies_signatures_and_private_fields() {
    let parsed = parse_sources(SOURCES);
    let interfaces = parse_sources(BOOTSTRAP_INTERFACES);
    // Support files supply real declarations, but only the two changed APIs'
    // body completeness is accepted by this narrowly scoped producer gate.
    let expected: List<_> = parsed
        .iter()
        .filter(|(owner, _)| *owner == VERSION_OWNER || *owner == REQUIREMENT_OWNER)
        .flat_map(|(owner, ast)| declared_bodies(owner, ast))
        .collect();
    let mut session = Session::new(Default::default());
    {
        let registry = session.module_registry();
        let mut registry = registry.write();
        for (owner, ast) in parsed.iter().chain(interfaces.iter()) {
            let id = registry.allocate_id();
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, Text::new());
            info.exports = extract_exports_from_module(ast, id, &path).unwrap();
            registry.register(info);
        }
        resolve_specific_reexport_kinds(&mut registry).unwrap();
        resolve_glob_reexports(&mut registry).unwrap();
    }
    let config = CoreConfig::new(".");
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    // Match compile_core's declaration prepass before the unit producer. The
    // prepass carries actual arity/return declarations for later modules; it
    // does not supply their runtime bodies or excuse a missing SemVer body.
    let interface_modules: List<_> = ["core.base", "core.text", "core.collections"]
        .into_iter()
        .map(|module| {
            let asts: List<_> = parsed
                .iter()
                .chain(interfaces.iter())
                .filter(|(owner, _)| owner.rsplit_once('.').unwrap().0 == module)
                .map(|(owner, ast)| {
                    (
                        PathBuf::from(owner.replace('.', "/")).with_extension("vr"),
                        ast.clone(),
                    )
                })
                .collect();
            (module.into(), asts.into())
        })
        .collect();
    pipeline
        .pre_register_canonical_static_methods(interface_modules.as_slice())
        .expect("normal source-derived canonical method interfaces");
    for (name, arity) in [
        ("Text.new", 0),
        ("Text.from_bytes", 1),
        ("Text.from_utf8_unchecked", 1),
        ("List.new", 0),
        ("Heap.new", 1),
    ] {
        assert_eq!(
            pipeline
                .global_function_registry
                .get(name)
                .unwrap()
                .param_count,
            arity,
            "actual dependency interface {name}"
        );
    }
    let unit = StdlibModule {
        name: "core.base".into(),
        source_files: List::new().into(),
        dependencies: List::new().into(),
    };
    let asts: List<_> = parsed.iter().map(|(_, ast)| ast).collect();
    let (module, count) = pipeline
        .compile_core_module_from_ast(
            &unit,
            asts.as_slice(),
            &config,
            &verum_ast::cfg::TargetConfig::host(),
            &Default::default(),
            &[],
        )
        .expect("actual source bootstrap producer");
    assert!(
        count >= expected.len(),
        "source declarations cannot silently disappear"
    );
    assert_signatures(&module);
    assert_bodies(&module, expected.as_slice(), "source");
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.base", &module, &[]).unwrap();
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let decoded = archive.load_module("core.base").unwrap();
    assert_signatures(&decoded);
    assert_bodies(&decoded, expected.as_slice(), "decoded archive");
    let metadata = crate::archive_metadata::archive_to_core_metadata(&archive);
    let metadata: CoreMetadata =
        bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap();
    assert_metadata(&metadata);
    eprintln!("semver-source-archive: complete");
}
