//! Scalar patterns use the same declared value across source owners and archives.
use std::sync::Arc;
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session, archive_ctx_loader::ArchiveCtxCache,
};
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::{VbcArchive, read_archive_from_file},
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn bootstrap(files: &[(&str, &str)]) -> VbcArchive {
    try_bootstrap(files).unwrap_or_else(|error| panic!("{error}"))
}

fn try_bootstrap(files: &[(&str, &str)]) -> Result<VbcArchive, verum_common::Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (name, source) in files {
        let path = core.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let output = root.path().join("stdlib.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
        .map_err(|error| {
            verum_common::Text::from(format!("{error}\n{}", session.format_diagnostics()))
        })?;
    Ok(read_archive_from_file(&output).unwrap())
}

fn execute(archive: &VbcArchive, source: &str, expected: i64) {
    let app = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(archive)
        .apply_lazy_with_types(&mut codegen, &app, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("app.main").unwrap();
        let value = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(expected), "{source}");
    }
}

#[test]
fn module_alias_patterns_keep_each_declaration_owner() {
    let archive = bootstrap(&[
        (
            "mod.vr",
            "module core; public module red; public module blue; public module readers; public module api;",
        ),
        (
            "readers/mod.vr",
            "module core.readers; public module first; public module second; public module exported;",
        ),
        ("api/mod.vr", "module core.api; public mount core.red.CODE; public mount core.blue.ALIAS as ALIASED;"),
        (
            "readers/exported.vr",
            "module core.readers.exported; mount core.api as surface; public fn code() -> Int { if surface.ALIASED != 20 { return -1; } surface.CODE }",
        ),
        (
            "red/mod.vr",
            "module core.red; public const CODE: Int = 7; public const ALIAS: Int = CODE + 1;",
        ),
        (
            "blue/mod.vr",
            "module core.blue; public const CODE: Int = 19; public const ALIAS: Int = CODE + 1;",
        ),
        (
            "readers/first.vr",
            "module core.readers.first; mount core.red as native; const LOCAL: Int = native.CODE + 1; public fn inspect(value: Int) -> Int { match value { native.CODE => 1, LOCAL => 2, _ => 0 } }",
        ),
        (
            "readers/second.vr",
            "module core.readers.second; mount core.blue as native; const LOCAL: Int = native.CODE + 1; public fn inspect(value: Int) -> Int { match value { native.CODE => 4, LOCAL => 8, _ => 0 } }",
        ),
    ]);
    execute(
        &archive,
        "module app; mount core.readers.{first, second, exported}; fn main() -> Int {
        assert(exported.code() == 7); assert(first.inspect(19) == 0); assert(second.inspect(7) == 0);
        first.inspect(7) + first.inspect(8) + second.inspect(19) + second.inspect(20)
    }",
        15,
    );
    execute(
        &archive,
        "module app; mount core.red as native;
        fn classify(value: Int) -> Int { match value { native.CODE | native.ALIAS => 1, _ => 0 } }
        fn main() -> Int { classify(7) + classify(8) + classify(19) }",
        2,
    );
}

#[test]
fn local_computed_scalar_patterns_keep_their_checked_value() {
    let archive = bootstrap(&[(
        "mod.vr",
        "module core;
        const LOW: Int = -9223372036854775807 - 1;
        const HIGH: Int = 9223372036854775807;
        const ENABLED: Bool = true;
        public fn inspect(value: Int) -> Int { match value { LOW => 1, HIGH => 2, _ => 0 } }
        public fn flag(value: Bool) -> Int { match value { ENABLED => 4, _ => 0 } }
    ",
    )]);
    execute(&archive, "module app; mount core.{inspect, flag};
        fn main() -> Int { inspect(-9223372036854775807 - 1) + inspect(9223372036854775807) + inspect(0) + flag(true) + flag(false) }", 7);
}

#[test]
fn unresolved_scalar_patterns_refuse_instead_of_matching_zero() {
    for (prefix, name) in [
        ("", "ABSENT"),
        ("", "missing.ABSENT"),
        ("fn compute() -> Int { 7 } const VALUE: Int = compute();", "VALUE"),
    ] {
        let source = format!(
            "{prefix} fn classify(value: Int) -> Int {{ match value {{ {name} => 73, _ => 0 }} }} fn main() -> Int {{ classify(0) }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let error = VbcCodegen::new()
            .compile_module(&ast)
            .err()
            .expect("an unresolved pattern must not acquire the value zero");
        assert!(
            error.to_string().contains("unresolved scalar constant"),
            "{source}: {error}"
        );
    }
}

#[test]
fn scalar_patterns_cannot_borrow_private_or_missing_owner_values() {
    for (path, member) in [("core.red", "SECRET"), ("core.blue", "CODE")] {
        let source = format!(
            "module core.reader; mount {path} as native; public fn inspect(value: Int) -> Int {{ match value {{ native.{member} => 73, _ => 0 }} }}"
        );
        let error = try_bootstrap(&[
            (
                "mod.vr",
                "module core; public module red; public module blue; public module reader;",
            ),
            (
                "red/mod.vr",
                "module core.red; const SECRET: Int = 7; public const CODE: Int = 11;",
            ),
            ("blue/mod.vr", "module core.blue;"),
            ("reader/mod.vr", &source),
        ])
        .err()
        .expect("a different owner supplied a private or absent constant");
        assert!(error.contains(member), "{error}");
    }
}

#[test]
fn unavailable_scalar_folding_preserves_the_declared_body() {
    let archive = bootstrap(&[("mod.vr", "module core;
        pure fn compute() -> Int { 9 }
        public const VALUE: Int = compute();
        public const ALIAS: Int = VALUE;
        public fn probe() -> Int { ALIAS }
    ")]);
    execute(&archive, "module app; mount core.probe; fn main() -> Int { probe() }", 9);
}

#[test]
fn archived_scalar_values_keep_their_value_type_and_visibility() {
    // Use the same complete metadata producer as a packaged stdlib. The
    // archive-only declaration converter does not own re-export/visibility
    // sidecars, so it cannot establish this consumer contract by itself.
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (name, source) in [
        ("mod.vr", "module core; public module provider; public module facade;"),
        ("provider/mod.vr", "module core.provider; public const CODE: Int = 7; const SECRET: Int = 11;"),
        ("facade/mod.vr", "module core.facade; public mount core.provider.CODE as EXPORTED;"),
    ] {
        let path = core.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let config = verum_compiler::precompile::PrecompileConfig::for_source(core, root.path());
    verum_compiler::precompile::precompile_stdlib(&config).unwrap();
    let bytes = std::fs::read(config.output_path.with_extension("core_metadata")).unwrap();
    let metadata = Arc::new(verum_compiler::embedded_stdlib_metadata::decode_metadata(&bytes).unwrap());
    for (source, accepted) in [
        ("fn probe() -> Int { core.provider.CODE }", true),
        ("mount core.provider as provider; fn probe() -> Int { provider.CODE }", true),
        ("mount core.facade as facade; fn probe() -> Int { facade.EXPORTED }", true),
        ("mount core.provider as provider; fn probe() -> Bool { provider.CODE }", false),
        ("fn probe() -> Int { core.provider.SECRET }", false),
        ("fn probe() -> Int { core.facade.CODE }", false),
    ] {
        let app = Parser::new(source).parse_module().unwrap();
        let mut checker = verum_types::TypeChecker::new_with_core(metadata.clone());
        checker.set_current_cog("app");
        checker.set_current_module_path("app");
        checker.register_stdlib_types_for_module(&app);
        for item in &app.items {
            if let verum_ast::ItemKind::Function(function) = &item.kind {
                checker.register_function_signature(function).unwrap();
            }
        }
        let mut errors: verum_common::List<_> = app.items.iter()
            .filter_map(|item| checker.check_item(item).err()).collect();
        errors.extend(checker.take_deferred_errors());
        assert_eq!(errors.is_empty() && checker.diagnostic_sources().is_empty(), accepted, "{source}: {errors:?}");
    }
}
