//! Dynamic signatures retain selected source owners when imported from an archive.
use verum_common::{List, Shared, Text};
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session,
    archive_metadata::archive_to_core_metadata,
};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{archive::read_archive_from_file, types::TypeRef, validate::validate_module};

#[test]
fn archived_dynamic_equalities_replay_the_selected_protocol_owner() {
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
            "module core.provider; public type Policy is protocol {type Item; fn read(&self)->Self.Item;};",
        ),
        (
            "decoy.vr",
            "module core.decoy; public type Policy is protocol {type Item; fn read(&self)->Self.Item;};",
        ),
        (
            "api.vr",
            "module core.api; mount core.provider.Policy as Selected;
            public fn inspect(value:&dyn Selected<Item=Int>)->Int {73}",
        ),
    ] {
        std::fs::write(source.join(file), text).unwrap();
    }
    let output = root.path().join("dynamic.vbca");
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
    let function = module.find_function_by_name("core.api.inspect").unwrap();
    let declared = module
        .get_function(function)
        .unwrap()
        .semantic_parameter_types()
        .unwrap()[0]
        .as_ref()
        .unwrap();
    let TypeRef::Reference { inner, .. } = declared else {
        panic!("{declared:?}")
    };
    let TypeRef::Dynamic(object) = inner.as_ref() else {
        panic!("{inner:?}")
    };
    assert_eq!(object.bindings.len(), 1);
    assert_eq!(object.bindings[0].owner, object.bounds[0]);
    let owner = module
        .get_type(verum_vbc::types::TypeId(object.bounds[0].protocol.0))
        .unwrap();
    assert_eq!(
        module.get_string(owner.origin_module.unwrap()),
        Some("core.provider")
    );
    let metadata = archive_to_core_metadata(&archive).unwrap();
    for eager in [false, true] {
        for (owner, expected) in [("provider", true), ("decoy", false)] {
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
            let ast = Parser::new(&format!(
                "mount core.api.inspect; mount core.{owner}.Policy;
                fn probe(value:&dyn Policy<Item=Int>)->Int {{inspect(value)}}"
            ))
            .parse_module()
            .unwrap();
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
                "owner={owner} eager={eager}: {errors:?} {diagnostics:?}"
            );
        }
    }
    let mut incomplete = module.clone();
    let incomplete_signature = incomplete.get_function_mut(function).unwrap();
    incomplete_signature.source_signature = None;
    incomplete_signature.declaration_identity = None;
    let mut builder = verum_vbc::archive::ArchiveBuilder::stdlib();
    builder.add_module("core", &incomplete, &[]).unwrap();
    let error = archive_to_core_metadata(&builder.finish()).unwrap_err();
    assert!(error.contains("applied dynamic contract lacks"), "{error}");
}

#[test]
fn checked_test_units_execute_the_default_from_the_selected_stdlib_source() {
    let consumer = r#"
        module app.probe;
        mount core.base.protocols.Selected;
        mount core.model.Badge;
        type Receiver is {};
        implement Selected for Receiver {}
        fn observe(badge: Badge) -> Int { badge.value }
        fn main() -> Int {
            let receiver = Receiver {};
            observe(Badge { value: receiver.probe() })
        }
    "#;
    for answer in [73, 19] {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("core");
        std::fs::create_dir_all(core.join("base")).unwrap();
        std::fs::write(core.join("mod.vr"), "module core; public module base; public module model; public module prelude { public mount super.base.protocols.*; }").unwrap();
        std::fs::write(core.join("base/mod.vr"), "module core.base; public module protocols; public module tag;").unwrap();
        std::fs::write(core.join("model.vr"), "module core.model; public type Badge is { public value: Int };").unwrap();
        std::fs::write(core.join("base/tag.vr"), "module core.base.tag; public type Tag is { public seal: Stamp };").unwrap();
        std::fs::write(core.join("base/protocols.vr"), format!(
            "module core.base.protocols; mount core.base.tag.Tag; public type Stamp is {{ public value: Int }};
             public fn take_tag(value: Tag) -> Int {{ value.seal.value }}
             public type Selected is protocol {{ fn probe(&self) -> Int {{ {answer} }} }};"
        )).unwrap();
        let input = root.path().join("probe.vr");
        let mut session = Session::new(CompilerOptions { input: input.clone(), ..Default::default() });
        let file = session.load_source_string(consumer, input).unwrap();
        let ast = verum_fast_parser::FastParser::new().parse_module_str(consumer, file).unwrap();
        let module = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core))
            .compile_test_unit_to_vbc(ast).unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        let entry = module.find_function_by_name("app.probe.main").unwrap();
        let value = verum_vbc::interpreter::Interpreter::new(module.into()).execute_function(entry).unwrap();
        assert_eq!(value.try_as_i64(), Some(answer));
    }
}
