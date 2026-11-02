//! Release signatures preserve declaration owners through renamed imports.
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::ArchiveBuilder,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
};

#[test]
fn nominal_universe_declarations_survive_archive_and_sidecar() {
    for declaration_only in [false, true] {
        let mut archive = ArchiveBuilder::new();
        for (owner, level) in [("core.alpha", 1), ("core.beta", 2)] {
            let ast = Parser::new(&format!("module {owner}; public type Token<A> is {{ public value: Type({level}), public extra: A }};"))
                .parse_module().unwrap();
            let mut producer = VbcCodegen::with_config(CodegenConfig::new(owner));
            let module = if declaration_only {
                producer.collect_declaration_interface(&[&ast]).unwrap()
            } else { producer.compile_module(&ast).unwrap() };
            archive.add_module(owner, &module, &[]).unwrap();
        }
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&archive.finish(), &mut bytes).unwrap();
        let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let metadata = archive_to_core_metadata(&archive).unwrap();
        let bytes = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
        let metadata = verum_types::core_metadata::decode_metadata(&bytes).unwrap();
        for eager in [false, true] {
            let result = errors(&metadata, "mount core.alpha.{Token as A}; mount core.beta.{Token as B}; fn a(x: A<Int>) -> Type(1) { x.value } fn b(x: B<Bool>) -> Type(2) { x.value } fn extra(x: A<Int>) -> Int { x.extra }", eager);
            assert!(result.is_empty(), "interface={declaration_only}, eager={eager}: {result:?}");
            let result = errors(&metadata, "mount core.beta.{Token}; fn bad(x: Token<Int>) -> Type(1) { x.value }", eager);
            assert!(!result.is_empty(), "archive permitted a downward universe cast");
        }
    }
}

fn metadata(declaration_only: bool, reverse: bool) -> CoreMetadata {
    let mut owners = List::new();
    for owner in ["core.alpha", "core.beta"] {
        let source = format!("module {owner}; public type Token is {{ value: Int }};");
        let ast = Parser::new(&source).parse_module().unwrap();
        owners.push(
            VbcCodegen::with_config(CodegenConfig::new(owner))
                .compile_module(&ast)
                .unwrap(),
        );
    }
    if reverse {
        owners.reverse();
    }
    let ast = Parser::new(
        r#"
        module core.api;
        mount core.alpha.{Token as Instant};
        mount core.beta.{Token as Other};
        public type Envelope<T> is { value: T };
        public fn boxed<T>(value: Envelope<(&Instant, &T)>) -> Envelope<(&Instant, &T)> { value }
        public fn echo(value: &Instant) -> &Instant { value }
        public fn nested<T>(value: (&Instant, &T)) -> (&Instant, &T) { value }
        public fn other(value: &Other) -> &Other { value }
        public fn checked_echo(value: &checked mut Instant) -> &checked mut Instant { value }
        public fn callable<F: fn(Int) -> Int>(value: F) -> F { value }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.api"));
    producer
        .import_bootstrap_nominal_dependencies_with_resolver(
            &[&ast],
            &owners.iter().collect::<List<_>>(),
            |name| Some(name.into()),
        )
        .unwrap();
    let module = if declaration_only {
        producer.collect_declaration_interface(&[&ast]).unwrap()
    } else {
        // The bootstrap compilation path retains imported declarations;
        // compile_module starts a new independent module and resets them.
        producer.collect_unit_declarations(&[&ast]).unwrap();
        producer
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        producer.finalize_module_from_state().unwrap()
    };
    let mut archive = ArchiveBuilder::new();
    for module in owners.iter().chain(std::iter::once(&module)) {
        archive.add_module(&module.name, module, &[]).unwrap();
    }
    // Exercise both archive serialization and the release metadata format.
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&archive.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).expect("valid archive metadata");
    bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap()
}

fn errors(metadata: &CoreMetadata, source: &str, eager: bool) -> List<Text> {
    let mut registry = verum_modules::ModuleRegistry::new();
    for (index, owner) in ["core.alpha", "core.beta", "core.api"].iter().enumerate() {
        registry.register(verum_modules::ModuleInfo::new(
            verum_modules::ModuleId::new(index as u32),
            verum_modules::ModulePath::from_str(owner),
            Parser::new("").parse_module().unwrap(),
            verum_ast::FileId::new(index as u32),
            Text::new(),
        ));
    }
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| format!("{error:?}").into())
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
fn source_signatures_keep_owners_reference_tiers_and_generic_slots() {
    for declaration_only in [false, true] {
        for reverse in [false, true] {
            let metadata = metadata(declaration_only, reverse);
            for (name, expected) in [
                ("echo", "&core.alpha.Token"),
                ("other", "&core.beta.Token"),
                ("nested", "(&core.alpha.Token, &__generic_0)"),
                (
                    "boxed",
                    "core.api.Envelope<(&core.alpha.Token, &__generic_0)>",
                ),
                ("checked_echo", "&checked mut core.alpha.Token"),
            ] {
                let function = metadata
                    .functions
                    .get(&Text::from(format!("core.api.{name}")))
                    .unwrap();
                assert_eq!(
                    function.params[0].ty.as_str(),
                    expected,
                    "{name}, interface={declaration_only}"
                );
                assert_eq!(
                    function.return_type.as_str(),
                    expected,
                    "{name}, interface={declaration_only}"
                );
            }
            assert_eq!(
                metadata
                    .functions
                    .get(&Text::from("core.api.callable"))
                    .unwrap()
                    .params[0]
                    .declared_ty
                    .as_str(),
                "F"
            );
        }
    }
}

#[test]
fn release_consumers_accept_only_the_declared_owner_and_callable_bound() {
    for declaration_only in [false, true] {
        for reverse in [false, true] {
            let metadata = metadata(declaration_only, reverse);
            for eager in [false, true] {
                for source in [
                    "fn probe(value: &core.alpha.Token) { let result: &core.alpha.Token = core.api.echo(value); }",
                    "fn probe(value: &core.beta.Token) { let result: &core.beta.Token = core.api.other(value); }",
                    "fn probe(value: (&core.alpha.Token, &Bool)) { let result: (&core.alpha.Token, &Bool) = core.api.nested(value); }",
                    "fn probe(value: core.api.Envelope<(&core.alpha.Token, &Bool)>) { let result: core.api.Envelope<(&core.alpha.Token, &Bool)> = core.api.boxed(value); }",
                    "fn probe(value: fn(Int) -> Int) -> fn(Int) -> Int { core.api.callable(value) }",
                ] {
                    assert!(
                        errors(&metadata, source, eager).is_empty(),
                        "{source}: {:?}",
                        errors(&metadata, source, eager)
                    );
                }
                for source in [
                    "fn probe(value: &core.beta.Token) { let result: &core.alpha.Token = core.api.echo(value); }",
                    "fn probe(value: &core.alpha.Token) { let result: &core.beta.Token = core.api.echo(value); }",
                    "fn probe(value: (&core.beta.Token, &Bool)) { let result: (&core.alpha.Token, &Bool) = core.api.nested(value); }",
                    "fn probe(value: core.api.Envelope<(&core.beta.Token, &Bool)>) { let result: core.api.Envelope<(&core.alpha.Token, &Bool)> = core.api.boxed(value); }",
                    "fn probe(value: fn(Bool) -> Bool) -> fn(Bool) -> Bool { core.api.callable(value) }",
                ] {
                    let errors = errors(&metadata, source, eager);
                    assert!(
                        !errors.is_empty() && errors.iter().all(|e| e.contains("Mismatch")),
                        "{source}: expected type mismatches, got {errors:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn actual_stdlib_datetime_signature_retains_the_imported_declaration() {
    let archive = verum_compiler::embedded_stdlib_vbc::get_runtime_archive()
        .expect("release stdlib archive required");
    let metadata = archive_to_core_metadata(archive).expect("valid archive metadata");
    let function = metadata
        .functions
        .get(&Text::from(
            "core.database.sqlite.native.datetime.iso.render",
        ))
        .expect("public datetime renderer");
    assert_eq!(
        function.params[0].ty.as_str(),
        "&core.database.sqlite.native.datetime.instant.DatetimeInstant"
    );
    assert_eq!(function.return_type.as_str(), "Text");
}

#[test]
#[ignore = "requires the matching freshly baked stdlib embedded in this test binary"]
fn actual_stdlib_nominal_universes_reach_the_compiler_pipeline() {
    use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};
    let archive = verum_compiler::embedded_stdlib_vbc::get_runtime_archive().expect("fresh stdlib");
    let metadata = archive_to_core_metadata(archive).unwrap();
    let descriptor = metadata.types.get(&Text::from("core.math.hott.NType")).expect("declared NType");
    assert!(descriptor.source_declaration.is_some(), "fresh nominal declaration carrier");
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("nominal.vr");
    std::fs::write(&input, "mount core.math.hott.{NType}; fn inspect(x: &NType<0>) {} fn main() {} ").unwrap();
    let mut session = Session::new(CompilerOptions {
        input, verify_mode: VerifyMode::Runtime, output_format: OutputFormat::Human,
        check_only: true, ..Default::default()
    });
    let result = CompilationPipeline::new(&mut session).run_check_only();
    assert!(result.is_ok(), "{result:?}\n{}", session.format_diagnostics());
}
