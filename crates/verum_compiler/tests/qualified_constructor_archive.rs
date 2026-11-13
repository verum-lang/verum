//! Qualified data construction retains its owner through source and archives.
use verum_common::{List, Shared, Text};
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session, archive_ctx_loader::ArchiveCtxCache,
};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    archive::VbcArchive,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const PROVIDER: &str = "module core.provider; public type Wrapped is (Int); public type Marker is (); public type Pair is (Int, Bool); public type Carrier<T> is (T); type Hidden is (Int);";
const CONSUMER: &str = r#"
mount core.provider as api;
mount core.provider.{Wrapped, Marker, Pair, Carrier};
public pure fn probe() -> Int {
    let unit = api.Marker();
    let pair = api.Pair(73, true);
    let wrapped = api.Wrapped(pair.0);
    let typed: Carrier<Int> = api.Carrier<Int>(wrapped.0);
    typed.0
}
"#;

fn compile(consumer: &str) -> Result<VbcArchive, Text> {
    compile_provider(PROVIDER, consumer)
}

fn compile_provider(provider: &str, consumer: &str) -> Result<VbcArchive, Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    for (file, source) in [
        (
            "mod.vr",
            "module core; public module provider; public module consumer;".to_owned(),
        ),
        ("provider.vr", provider.to_owned()),
        ("consumer.vr", format!("module core.consumer; {consumer}")),
    ] {
        std::fs::write(core.join(file), source).unwrap();
    }
    let output = root.path().join("stdlib.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
        .map_err(|error| Text::from(format!("{error}\n{}", session.format_diagnostics())))?;
    Ok(verum_vbc::archive::read_archive_from_file(&output).unwrap())
}

fn archive_errors(archive: &VbcArchive, source: &str, eager: bool) -> List<Text> {
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(archive).unwrap();
    let encoded = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
    let metadata =
        Shared::new(verum_types::core_metadata::decode_metadata(&encoded).unwrap()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("app");
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str("core.provider"),
        Parser::new("").parse_module().unwrap(),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|error| error.is_error())
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

#[test]
fn a_private_sum_crosses_modules_through_its_public_operations() {
    let provider = "module core.provider;
        type Receipt is Issued(Int) | Blank;
        public fn issue(n:Int)->Receipt { Issued(n) }
        public fn number(x:Receipt)->Int { match x { Issued(n)=>n, Blank=>0 } }";
    let consumer = "mount core.provider.{issue, number};
        public fn probe()->Int { number(issue(73)) }";
    let archive = compile_provider(provider, consumer).unwrap();
    execute(&archive, "module app; mount core.consumer.probe; fn main()->Int {probe()}");

    let forged = "mount core.provider.number; public fn probe()->Int { number(Issued(73)) }";
    let error = compile_provider(provider, forged).err().expect("a foreign module cannot supply the private case");
    assert!(error.contains("Issued"), "{error}");
}

#[test]
fn qualified_constructors_execute_after_source_archive_and_wire() {
    for consumer in [
        CONSUMER.to_owned(),
        CONSUMER
            .replace("mount core.provider as api;", "mount core.provider;")
            .replace("api.", "core.provider."),
        r#"mount core.provider as api;
            type Wrapped is (Bool);
            type Carrier<Other> is (Bool);
            public pure fn probe()->Int {
                let local = Wrapped(true);
                let value: api.Carrier<Int> = api.Carrier<Int>(api.Wrapped(73).0);
                value.0
            }"#
        .to_owned(),
    ] {
        let archive = compile(&consumer).unwrap();
        execute(
            &archive,
            "module app; mount core.consumer.probe; fn main()->Int {probe()}",
        );
    }
}

fn execute(archive: &VbcArchive, source: &str) {
    let app = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(&archive)
        .apply_lazy_with_types(&mut codegen, &app, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("app.main").unwrap();
        assert_eq!(
            Interpreter::new(module.into())
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn archive_loaded_constructors_execute_without_source_definitions() {
    let archive = compile("public fn probe()->Int {73}").unwrap();
    let source = format!(
        "module app; {}",
        CONSUMER.replace("public pure fn probe", "fn main")
    );
    execute(&archive, &source);
}

#[test]
fn fresh_archive_constructor_schemes_preserve_domains_and_owners() {
    let archive = compile("public fn probe()->Int {73}").unwrap();
    for eager in [false, true] {
        let errors = archive_errors(&archive, CONSUMER, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        for (source, expected) in [
            (
                "mount core.provider as api; fn wrong()->Int {let value=api.Carrier<Bool>(73); value.0}",
                "mismatch",
            ),
            (
                "mount core.provider as api; type Hidden is (Int); fn wrong()->Int {let value=api.Hidden(73); value.0}",
                "Hidden",
            ),
            (
                "mount core.provider as api; type Wrapped is (Int); fn wrong()->Wrapped {api.Wrapped(73)}",
                "mismatch",
            ),
        ] {
            let errors = archive_errors(&archive, source, eager);
            assert!(
                errors
                    .iter()
                    .any(|error| error.to_lowercase().contains(&expected.to_lowercase())),
                "eager={eager}, {source}: {errors:?}"
            );
        }
    }
}
