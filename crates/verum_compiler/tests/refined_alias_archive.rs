//! Runtime alias representation must not erase imported refinement obligations.
use verum_common::{List, Shared, Text};
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session,
    archive_metadata::archive_to_core_metadata,
};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{archive::read_archive_from_file, types::TypeRef, validate::validate_module};

#[test]
fn archived_refinement_replays_predicate_and_declaring_owner() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("core");
    std::fs::create_dir(&source).unwrap();
    for (file, text) in [
        (
            "mod.vr",
            "module core; public module provider; public module decoy; public module api;",
        ),
        (
            "provider.vr",
            "module core.provider;
            public type Positive is Int { it > 0 };
            public fn good()->Positive {73}
            public fn accept(value:Positive)->Int {value}",
        ),
        (
            "decoy.vr",
            "module core.decoy;
            public type Positive is Int { it < 0 };
            public fn accept(value:Positive)->Int {value}",
        ),
        (
            "api.vr",
            "module core.api;
            mount core.provider.Positive as Selected;
            public fn inspect(value:Selected)->Int {value}
            public fn qualified(value:core.provider.Positive)->Int {value}",
        ),
    ] {
        std::fs::write(source.join(file), text).unwrap();
    }
    let output = root.path().join("refined.vbca");
    let mut session = Session::new(CompilerOptions {
        input: source.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&source).with_output(&output))
            .compile_core()
            .unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    validate_module(&module).unwrap();
    for owner in ["core.provider", "core.decoy"] {
        let positive = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name == "Positive" || name.ends_with(".Positive"))
                    && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
            })
            .unwrap();
        assert_eq!(
            positive.alias_target,
            Some(TypeRef::Concrete(verum_vbc::types::TypeId::INT))
        );
        assert!(
            positive.source_declaration.is_some(),
            "lost predicate from {owner}"
        );
    }
    for (entry_name, valid, invalid) in [
        ("core.provider.accept", 73, -73),
        ("core.decoy.accept", -73, 73),
        ("core.api.inspect", 73, -73),
        ("core.api.qualified", 73, -73),
    ] {
        let entry = module.find_function_by_name(entry_name).unwrap();
        for (value, accepted) in [(invalid, false), (valid, true), (0, false)] {
            let mut vm = verum_vbc::interpreter::Interpreter::new(module.clone().into());
            vm.run_global_ctors().unwrap();
            let result = vm.execute_function_with_args(entry, &[verum_vbc::Value::from_i64(value)]);
            if accepted {
                assert_eq!(result.unwrap().try_as_i64(), Some(value), "{entry_name}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(verum_vbc::interpreter::InterpreterError::AssertionFailed { .. })
                    ),
                    "entry={entry_name}, invalid={value}: {result:?}"
                );
            }
        }
    }
    let metadata = archive_to_core_metadata(&archive).unwrap();
    for eager in [false, true] {
        for (program, expected) in [
            (
                "mount core.api.inspect; fn probe()->Int {inspect(73)}",
                true,
            ),
            (
                "mount core.api.inspect; fn probe()->Int {inspect(0)}",
                false,
            ),
            (
                "mount core.api.inspect; mount core.decoy.Positive;
                fn probe()->Int {inspect(-73)}",
                false,
            ),
            (
                "mount core.api.inspect; mount core.decoy.Positive;
                fn probe()->Int {inspect(73)}",
                true,
            ),
        ] {
            let mut registry = verum_modules::ModuleRegistry::new();
            for (index, name) in ["core.provider", "core.decoy", "core.api"]
                .iter()
                .enumerate()
            {
                registry.register(verum_modules::ModuleInfo::new(
                    verum_modules::ModuleId::new(index as u32),
                    verum_modules::ModulePath::from_str(name),
                    Parser::new("").parse_module().unwrap(),
                    verum_ast::FileId::new(index as u32),
                    Text::new(),
                ));
            }
            let mut checker = TypeChecker::with_minimal_context();
            checker.set_module_registry_direct(registry);
            checker.set_current_module_path("consumer");
            let metadata = Shared::new(metadata.clone()).into_arc();
            if eager {
                checker.load_core_eager(metadata);
            } else {
                checker.register_primitives();
                checker.set_core_metadata(metadata);
            }
            let ast = Parser::new(program).parse_module().unwrap();
            checker.register_stdlib_types_for_module(&ast);
            let mut errors: List<_> = checker.prepare_module_declarations(&ast);
            errors.extend(
                ast.items
                    .iter()
                    .filter_map(|item| checker.check_item(item).err()),
            );
            errors.extend(checker.take_deferred_errors());
            let diagnostics = checker.diagnostic_sources();
            assert_eq!(
                errors.is_empty() && diagnostics.is_empty(),
                expected,
                "eager={eager}, source={program}: {errors:?} {diagnostics:?}"
            );
        }
    }
}
