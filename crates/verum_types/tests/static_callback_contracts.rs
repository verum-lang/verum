//! A synchronous callback can diverge; it cannot suspend its caller.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    errors_with_provider(source, None)
}

fn errors_with_provider(source: &str, provider: Option<&str>) -> List<Text> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    if let Some(provider) = provider {
        use verum_modules::{
            ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
        };
        let module = verum_fast_parser::FastParser::new()
            .parse_module_str(provider, verum_ast::FileId::new(1))
            .unwrap();
        let id = ModuleId::new(1);
        let path = ModulePath::from_str("platform");
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            module.clone(),
            module.file_id,
            provider.into(),
        );
        info.exports = extract_exports_from_module(&module, id, &path).unwrap();
        let mut registry = ModuleRegistry::new();
        registry.register(info);
        checker.set_module_registry_direct(registry);
    }
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
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
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| Text::from(diagnostic.message())),
    );
    errors
}

#[test]
fn a_diverging_source_handler_remains_synchronous() {
    for source in [
        "fn stop() -> ! { loop {} } fn handler(value: Int) { stop(); } static mut HANDLER: fn(Int) = handler;",
        "static mut HANDLER: fn(Int) = handler; fn handler(value: Int) { stop(); } fn stop() -> ! { loop {} }",
        "extern \"C\" { fn stop() -> !; } fn handler(value: Int) { unsafe { stop(); } } static mut HANDLER: fn(Int) = handler;",
        "static mut HANDLER: fn(Int) = handler; fn handler(value: Int) { unsafe { stop(); } } extern \"C\" { fn stop() -> !; }",
    ] {
        let errors = errors(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn an_async_handler_cannot_be_installed_as_a_synchronous_callback() {
    for source in [
        "async fn handler(value: Int) {} static mut HANDLER: fn(Int) = handler;",
        "static mut HANDLER: fn(Int) = handler; fn handler(value: Int) { pending(); } async fn pending() {}",
        "static mut HANDLER: fn(Int) = handler; fn handler(value: Int) { let stop = async || {}; stop(); } fn stop() {}",
        "fn opaque_handler(value: Int); static mut HANDLER: fn(Int) = opaque_handler;",
    ] {
        assert!(
            !errors(source).is_empty(),
            "unknown or suspending callback accepted: {source}"
        );
    }
}

#[test]
fn a_foreign_abi_does_not_establish_purity() {
    let source =
        "extern \"C\" { fn sample() -> Int; } pure fn result() -> Int { unsafe { sample() } }";
    let errors = errors(source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("ImpurePureFunction")),
        "foreign purity fabricated: {errors:?}"
    );
}

#[test]
fn imported_foreign_callbacks_retain_the_declared_abi() {
    let source = "mount platform.sample as read_value;
        fn apply(f: fn(Int) -> Int) -> Int { f(7) }
        fn result() -> Int { apply(read_value) }";
    let errors = errors_with_provider(
        source,
        Some("extern \"C\" { public fn sample(value: Int) -> Int; }"),
    );
    assert!(errors.is_empty(), "{errors:?}");
}
