//! Inline namespaces keep their declared source owner through core archives.
use verum_common::Text;
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session, archive_ctx_loader::ArchiveCtxCache,
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn contract(consumer: &str) -> Result<(), Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    for (file, source) in [
        ("mod.vr", "module core; public module consumer; public module decoy; public module facade;".to_owned()),
        ("facade.vr", "module core.facade; public mount core.decoy.generic as renamed;".to_owned()),
        ("consumer.vr", format!("module core.consumer;\n{consumer}")),
        ("decoy.vr", "module core.decoy; public pure fn selected(value: Int) -> Int { value } public pure fn generic<T>(value: T) -> T { value } public pure fn configured<Shape: meta [Int; 3], T>(value: T) -> T { value } pure fn secret(value: Int) -> Int { value } public module provider { public pure fn identity(value: Int) -> Int { 5 } }".to_owned()),
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
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let app = Parser::new("module app; mount core.consumer.probe; fn main() -> Int { probe() }")
        .parse_module()
        .unwrap();
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
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("app.main").unwrap();
        let result = Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(73), "{consumer}");
    }
    Ok(())
}

const PROVIDER: &str =
    "public module provider { public pure fn identity(value: Int) -> Int { value } }";

#[test]
fn lexical_calls_retain_owner_and_body_through_the_archive() {
    for forward in [false, true] {
        let probe = "public pure fn probe() -> Int { provider.identity(73) }";
        let source = if forward {
            format!("{probe} {PROVIDER}")
        } else {
            format!("{PROVIDER} {probe}")
        };
        contract(&source).unwrap();
    }
}

#[test]
fn explicit_inline_alias_uses_the_same_selected_declaration() {
    contract(&format!("{PROVIDER} mount core.consumer.provider as chosen; public pure fn probe() -> Int {{ chosen.identity(73) }}")).unwrap();
}

#[test]
fn nested_inline_namespace_calls_keep_their_source_owner() {
    contract("public module provider { public module ops { public pure fn identity(value: Int) -> Int { value } } } public pure fn probe() -> Int { provider.ops.identity(73) }").unwrap();
}

#[test]
fn absent_private_and_impure_inline_selections_are_refused() {
    for (source, expected) in [
        (
            "public pure fn probe() -> Int { provider.identity(73) }",
            "provider",
        ),
        (
            "public module provider { public pure fn other(value: Int) -> Int { value } } public pure fn probe() -> Int { provider.identity(73) }",
            "identity",
        ),
        (
            "public module provider { pure fn identity(value: Int) -> Int { value } } public pure fn probe() -> Int { provider.identity(73) }",
            "identity",
        ),
        (
            "public module provider { extern \"C\" { fn touch(); } public fn identity(value: Int) -> Int { unsafe { touch(); } value } } public pure fn probe() -> Int { provider.identity(73) }",
            "E503",
        ),
    ] {
        let error = contract(source).expect_err("invalid namespace selection was admitted");
        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn source_namespace_alias_keeps_generic_call_obligations_and_visibility() {
    contract("mount core.decoy.{self as service}; public pure fn probe() -> Int { service.generic<Int>(service.selected(73)) }").unwrap();
    let source = "mount core.decoy.{self as service}; public pure fn secret(value: Int) -> Int { value } public pure fn probe() -> Int { service.secret(73) }";
    let error =
        contract(source).expect_err("a caller homonym cannot authorize a private source member");
    assert!(error.contains("secret"), "{error}");
}

#[test]
fn namespace_alias_retains_the_canonical_callable_generic_roster() {
    let mut errors = verum_common::List::new();
    for call in ["core.decoy.generic<Int>(73)", "service.generic<Int>(73)"] {
        let source = format!(
            "mount core.decoy.{{self as service}}; public pure fn probe() -> Int {{ {call} }}"
        );
        if let Err(error) = contract(&source) {
            errors.push(format!("{call}: {error}"));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn explicit_source_arguments_bind_the_selected_declaration() {
    let mut admitted = verum_common::List::new();
    for source in [
        "public pure fn generic<T>(value: T) -> T { value } public pure fn probe() -> Int { generic<Bool>(73) }",
        "mount core.decoy.generic; public pure fn probe() -> Int { generic<Bool>(73) }",
        "public pure fn probe() -> Int { core.decoy.generic<Bool>(73) }",
        "mount core.decoy.{self as service}; public pure fn probe() -> Int { service.generic<Bool>(73) }",
        "mount core.facade.renamed as selected; public pure fn probe() -> Int { selected<Bool>(73) }",
        "mount core.facade as service; public pure fn renamed<T>(value: Int) -> Int { value } public pure fn probe() -> Int { service.renamed<Bool>(73) }",
    ] {
        match contract(source) {
            Ok(()) => admitted.push(source),
            Err(error) => assert!(
                error.contains("Bool") && error.contains("Int"),
                "{source}: {error}"
            ),
        }
    }
    assert!(
        admitted.is_empty(),
        "explicit Bool argument was discarded: {admitted:?}"
    );
}

#[test]
fn renamed_source_reexports_keep_their_generic_owner() {
    contract("mount core.facade as service; public pure fn renamed<T>(value: Bool) -> Bool { value } public pure fn probe() -> Int { service.renamed<Int>(73) }").unwrap();
}

#[test]
fn explicit_source_argument_arity_and_kind_are_preserved() {
    for (arguments, expected) in [("Int, Bool", "E408"), ("3", "requires a type argument")] {
        for (mount, callee) in [
            ("mount core.decoy.generic;", "generic"),
            ("", "core.decoy.generic"),
            ("mount core.facade as service;", "service.renamed"),
        ] {
            let source =
                format!("{mount} public pure fn probe() -> Int {{ {callee}<{arguments}>(73) }}");
            let error = contract(&source).expect_err("generic argument contract was discarded");
            assert!(error.contains(expected), "{source}: {error}");
        }
    }
}

#[test]
fn declared_value_binder_survives_fresh_archive_publication() {
    contract("mount core.decoy as service; public pure fn probe() -> Int {service.configured<[2; 3], Int>(73)}").unwrap();
    for value in ["[true; 3]", "[2; 4]"] {
        let source = format!(
            "mount core.decoy as service; public pure fn probe() -> Int {{service.configured<{value}, Int>(73)}}"
        );
        assert!(
            contract(&source).is_err(),
            "declared meta array domain must hold: {source}"
        );
    }
}
