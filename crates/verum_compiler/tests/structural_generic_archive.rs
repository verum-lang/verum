//! Declaration-selected bracket arguments survive the real archive metadata producer.
use verum_common::{List, Map, Set, Shared, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::{ArchiveBuilder, VbcArchive},
    codegen::{CodegenConfig, VbcCodegen},
};

fn archive(sources: &[(&str, &str)], reverse: bool) -> VbcArchive {
    let mut builder = ArchiveBuilder::new();
    let mut sources: List<_> = sources.iter().collect();
    if reverse {
        sources.reverse();
    }
    for (owner, source) in sources {
        let ast = Parser::new(source)
            .parse_module()
            .expect("declaration grammar");
        let module = VbcCodegen::with_config(CodegenConfig::new(*owner))
            .compile_module(&ast)
            .expect("source archive producer");
        builder.add_module(owner, &module, &[]).unwrap();
    }
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap()
}

// Use the public full archive loader on the supplied archive. The lazy stdlib
// loader also consults its process-embedded metadata and symbol graph, which
// are unrelated to this independently produced fixture.
fn import(archive: &VbcArchive) -> VbcCodegen {
    let mut codegen = VbcCodegen::new();
    codegen.populate_types_from_archive(archive);
    let cache = verum_compiler::archive_ctx_loader::ArchiveCtxCache::new();
    codegen.import_functions(cache.get_or_build(archive));
    for entry in &archive.index {
        let module = archive.load_module(&entry.name).unwrap();
        let remap: Map<_, _> = module
            .functions
            .iter()
            .map(|function| {
                let name = module.get_string(function.name).unwrap();
                let qualified = verum_vbc::module::qualify_module_name(&entry.name, name);
                let info = codegen
                    .ctx_mut()
                    .lookup_function(&qualified)
                    .unwrap_or_else(|| panic!("imported declaration {qualified}"));
                (function.id.0, info.id)
            })
            .collect();
        assert!(codegen.merge_archive_function_bodies(&module, &remap.into()) > 0);
    }
    codegen
}

fn errors(archive: &VbcArchive, source: &str, eager: bool) -> List<Text> {
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(archive);
    let metadata: CoreMetadata =
        bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap();
    let mut registry = verum_modules::ModuleRegistry::new();
    let owners: Set<Text> = metadata
        .functions
        .values()
        .map(|fd| fd.module_path.clone())
        .collect();
    for (index, owner) in owners.iter().enumerate() {
        registry.register(verum_modules::ModuleInfo::new(
            verum_modules::ModuleId::new(index as u32),
            verum_modules::ModulePath::from_str(owner),
            Parser::new("").parse_module().unwrap(),
            verum_ast::FileId::new(index as u32),
            Text::new(),
        ));
    }
    let metadata = Shared::new(metadata).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source)
        .parse_module()
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| format!("{e:?}").into())
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors
}

#[test]
fn source_archives_keep_type_slots_distinct_from_const_and_meta_values() {
    for declaration in [
        "const Shape: [Int], T",
        "Shape: meta [Int], T",
        "'a, T",
        "{Unused}, T",
    ] {
        for reverse in [false, true] {
            let alpha = format!("public fn choose<{declaration}>(value: T)->T {{ value }}");
            let archive = archive(
                &[
                    ("core.alpha", &alpha),
                    ("core.beta", "public fn choose<T>(value: T)->T { value }"),
                ],
                reverse,
            );
            let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&archive);
            let alpha = metadata
                .functions
                .get(&Text::from("core.alpha.choose"))
                .unwrap();
            let slots = alpha
                .explicit_type_param_ids
                .as_ref()
                .expect("producer carries slots");
            let expected_len = if declaration.starts_with('{') { 1 } else { 2 };
            assert_eq!(slots.len(), expected_len, "{declaration}");
            assert!(slots.last().unwrap().is_some());
            if expected_len == 2 {
                assert!(slots[0].is_none());
            }
            let prefix = if declaration.starts_with("'a") {
                "'a, "
            } else if declaration.starts_with('{') {
                ""
            } else {
                "[2; 3], "
            };
            for eager in [false, true] {
                for (mount, callee) in [
                    ("", "core.alpha.choose"),
                    ("mount core.alpha.{choose};", "choose"),
                    ("mount core.alpha.{choose as selected};", "selected"),
                ] {
                    let good =
                        format!("{mount} fn probe()->Bool {{ {callee}<{prefix}Bool>(true) }}");
                    assert!(
                        errors(&archive, &good, eager).is_empty(),
                        "{declaration} / {good}: {:?}",
                        errors(&archive, &good, eager)
                    );
                    let bad = good.replace("Bool>(true)", "[Byte; 3]>(true)");
                    assert!(
                        !errors(&archive, &bad, eager).is_empty(),
                        "{declaration}: wrong array value accepted: {bad}"
                    );
                }
            }
        }
    }
}

#[test]
fn source_archive_method_shadow_uses_the_method_slot() {
    let archive = archive(
        &[(
            "core.factory",
            r#"
        public type Factory<T> is { value: T };
        implement<T> Factory<T> {
            public fn choose<T>(self, value:T)->T { value }
        }
    "#,
        )],
        false,
    );
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&archive);
    let method = metadata
        .functions
        .get(&Text::from("core.factory.Factory.choose"))
        .unwrap();
    assert_eq!(
        method.explicit_type_param_ids.as_ref().unwrap().as_slice(),
        &[Some(0x8000)]
    );
    for eager in [false, true] {
        let good = "mount core.factory.{Factory}; fn probe(factory: Factory<Int>)->Bool {factory.choose<Bool>(true)}";
        assert!(
            errors(&archive, good, eager).is_empty(),
            "{:?}",
            errors(&archive, good, eager)
        );
        let bad = good.replace("choose<Bool>", "choose<[Byte; 3]>");
        assert!(
            !errors(&archive, &bad, eager).is_empty(),
            "method shadow lost its Type slot"
        );
    }
}

#[test]
fn imported_array_layout_call_preserves_exact_witness_and_executes() {
    for reverse in [false, true] {
        let archive = archive(
            &[
                (
                    "core.alpha",
                    "public fn size<const Shape: [Int], T>()->Int {T.size}",
                ),
                ("core.beta", "public fn size<T>()->Int {99}"),
            ],
            reverse,
        );
        let source = "fn probe()->Int {core.alpha.size<[2; 3], [[Byte; 2]; 3]>()}";
        assert!(
            errors(&archive, source, false).is_empty(),
            "{:?}",
            errors(&archive, source, false)
        );
        let ast = Parser::new(source).parse_module().unwrap();
        let mut codegen = import(&archive);
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        let module = codegen
            .compile_function_bodies(&ast)
            .expect("imported source lowering");
        let module = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        let probe = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            verum_vbc::interpreter::Interpreter::new(Shared::new(module).into_arc())
                .execute_function(probe)
                .expect("imported witness execution")
                .as_i64(),
            6
        );
    }
}

#[test]
fn legacy_archive_without_slot_roster_does_not_guess_an_array_role() {
    let source = archive(
        &[("core.alpha", "public fn size<T>()->Int {T.size}")],
        false,
    );
    let mut module = source.load_module("core.alpha").unwrap();
    for function in &mut module.functions {
        function.explicit_type_param_ids.clear();
    }
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.alpha", &module, &[]).unwrap();
    let archive = builder.finish();
    let source = "fn probe()->Int {core.alpha.size<[Byte; 3]>()}";
    let errors = errors(&archive, source, false);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("declaration-owned slot metadata")),
        "{errors:?}"
    );
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = import(&archive);
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let error = codegen
        .compile_function_bodies(&ast)
        .err()
        .expect("missing slot proof must fail");
    assert!(
        format!("{error}").contains("declaration-owned slot metadata"),
        "{error}"
    );
}
