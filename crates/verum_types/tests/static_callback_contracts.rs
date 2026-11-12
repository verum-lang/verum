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
fn intrinsic_declarations_propagate_the_selected_lowering_bound() {
    for provider in [
        "@intrinsic(\"text_byte_len\") public fn inspect(value: &Text) -> Int;",
        "public fn inspect(value: &Text) -> Int { @intrinsic(\"text_byte_len\", value) }",
        "@intrinsic(\"text_as_bytes\") fn bytes(value: &Text) -> &[Byte];
         public fn inspect(value: &Text) -> Int { let data = bytes(value); 7 }",
    ] {
        for source in [
            "mount platform.inspect; fn handler(value: &Text) { inspect(value); }
             static mut HANDLER: fn(&Text) = handler;",
            "mount platform.inspect as measure; static mut HANDLER: fn(&Text) = handler;
             fn handler(value: &Text) { measure(value); }",
        ] {
            let accepted = errors_with_provider(source, Some(provider));
            assert!(accepted.is_empty(), "{provider}: {source}: {accepted:?}");
        }
    }
}

#[test]
fn forward_import_contracts_obey_source_cfg_selection() {
    let provider = "@intrinsic(\"text_byte_len\") public fn inspect(value: &Text) -> Int;
                    public async fn pending(value: &Text) -> Int { 7 }";
    let accepted = errors_with_provider(
        "mount platform.inspect as measure;
         @cfg(target_os = \"unselected\") mount platform.pending as measure;
         static mut HANDLER: fn(&Text) = handler;
         fn handler(value: &Text) { measure(value); }", Some(provider));
    assert!(accepted.is_empty(), "{accepted:?}");
    let rejected = errors_with_provider(
        "@cfg(target_os = \"unselected\") mount platform.inspect as measure;
         mount platform.pending as measure;
         static mut HANDLER: fn(&Text) = handler;
         fn handler(value: &Text) { measure(value); }", Some(provider));
    assert!(!rejected.is_empty(), "excluded synchronous alias supplied evidence");
}

#[test]
fn intrinsic_hints_and_annotations_cannot_fabricate_callable_evidence() {
    for provider in [
        // Registration and a Pure optimizer hint alone do not certify a bound.
        "@intrinsic(\"text_parse_int\") public fn inspect(value: &Text) -> Int;",
        "@intrinsic(\"missing_operation\") public fn inspect(value: &Text) -> Int;",
        "public fn inspect(value: &Text) -> Int;",
        "fn text_byte_len(value: &Text) -> Int;
         public fn inspect(value: &Text) -> Int { text_byte_len(value) }",
        "@intrinsic(\"text_byte_len\") public fn inspect(value: &Text) -> Int { pending() }
         async fn pending() -> Int { 7 }",
    ] {
        let source = "mount platform.inspect; fn handler(value: &Text) { inspect(value); }
                      static mut HANDLER: fn(&Text) = handler;";
        assert!(!errors_with_provider(source, Some(provider)).is_empty(), "{provider}");
    }
    let source = "@intrinsic(\"text_as_bytes\") fn bytes(value: &Text) -> &[Byte];
                  pure fn inspect(value: &Text) -> Int { let data = bytes(value); 7 }";
    assert!(!errors(source).is_empty(), "synchronous must not imply pure");
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

#[test]
fn checked_method_contracts_reach_static_and_instance_callers() {
    for source in [
        "type Event is Value(Int); type Device is ();
         fn apply(f: fn(Int) -> Event) -> Event { f(7) }
         implement Device { fn package(value: Int) -> Event { Event.Value(value) } }
         fn result() -> Event { apply(|value| Device.package(value)) }",
        "type Event is Value(Int); type Device is ();
         implement Device { fn package(value: Int) -> Event { Event.Value(value) } }
         static mut CALLBACK: fn(Int) -> Event = Device.package;",
        "type Event is Value(Int); type Device is ();
         fn apply(f: fn(Int) -> Event) -> Event { f(7) }
         implement Device { fn package(&self, value: Int) -> Event { Event.Value(value) } }
         fn result(device: &Device) -> Event { apply(|value| device.package(value)) }",
        "type Event is Value(Int); type Device is ();
         type Encode is protocol { fn package(&self, value: Int) -> Event; };
         fn apply(f: fn(Int) -> Event) -> Event { f(7) }
         implement Encode for Device { fn package(&self, value: Int) -> Event { Event.Value(value) } }
         fn result(device: &Device) -> Event { apply(|value| device.package(value)) }",
        "type Device<T> is (); type Event<T> is Value(T);
         fn apply(f: fn(Int) -> Event<Int>) -> Event<Int> { f(7) }
         implement<T> Device<T> { fn package(value: T) -> Event<T> { Event.Value(value) } }
         fn result() -> Event<Int> { apply(|value| Device.package(value)) }",
    ] {
        let accepted = errors(source);
        assert!(accepted.is_empty(), "{source}: {accepted:?}");
    }
}

#[test]
fn checked_method_contracts_do_not_leak_to_sibling_owners() {
    for marker in ["async ", ""] {
        let body = if marker.is_empty() {
            "@intrinsic(\"syscall0\", value); Event.Value(value)"
        } else { "Event.Value(value)" };
        let source = format!(
            "type Event is Value(Int); type First is (); type Second is ();
             fn apply(f: fn(Int) -> Event) -> Event {{ f(7) }}
             implement Second {{ {marker}fn package(value: Int) -> Event {{ {body} }} }}
             implement First {{ fn package(value: Int) -> Event {{ Event.Value(value) }} }}
             fn result() -> Event {{ apply(|value| Second.package(value)) }}"
        );
        let rejected = errors(&source);
        assert!(rejected.iter().any(|e| e.contains("Mismatch") && e.contains("async")),
            "a sibling method supplied false evidence: {source}: {rejected:?}");
    }
}
