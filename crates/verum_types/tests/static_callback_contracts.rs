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
fn assigning_a_diverging_handler_preserves_the_storage_contract() {
    for source in [
        "static mut HANDLER: fn(Int) = stop; fn stop(value: Int) -> ! { loop {} }
         fn reset() { unsafe { HANDLER = stop; } }
         fn replace(handler: fn(Int)) { unsafe { HANDLER = handler; } }",
        "fn stop(value: Int) -> ! { loop {} } static mut HANDLER: fn(Int) = stop;
         fn replace(handler: fn(Int)) { unsafe { HANDLER = handler; } }",
    ] {
        let errors = errors(source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn callable_assignment_uses_the_declared_type_of_every_place() {
    for (declarations, body) in [
        ("", "let mut value: fn(Int) = normal; value = stop; value = normal;"),
        ("type Holder is { handler: fn(Int) };",
         "let mut value = Holder { handler: normal }; value.handler = stop; value.handler = normal;"),
        ("type Holder is (fn(Int));",
         "let mut value = Holder(normal); value.0 = stop; value.0 = normal;"),
        ("", "let mut value: (fn(Int), Int) = (normal, 7); value.0 = stop; value.0 = normal;"),
        ("", "let mut value: [fn(Int); 1] = [normal]; value[0] = stop; value[0] = normal;"),
        ("", "let mut value: fn(Int) = normal; let reference = &mut value; *reference = stop; *reference = normal;"),
    ] {
        let source = format!("{declarations} fn normal(value: Int) {{}}
            fn stop(value: Int) -> ! {{ loop {{}} }} fn probe() {{ {body} }}");
        let errors = errors(&source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn assignment_preserves_callables_reference_permissions_and_literal_bounds() {
    for source in [
        "fn stop(value: Int) -> ! { loop {} } fn normal(value: Int) {}
         fn bad() { let mut callback: fn(Int) -> ! = stop; callback = normal; }",
        "fn normal(value: Int) {} async fn pending(value: Int) {}
         fn bad() { let mut callback: fn(Int) = normal; callback = pending; }",
        "fn normal(value: Int) {} fn wrong(value: Bool) {}
         fn bad() { let mut callback: fn(Int) = normal; callback = wrong; }",
        "fn normal(value: Int) {} fn stop(value: Int) -> ! { loop {} }
         fn bad() { let mut callback: fn(Int) = normal; let reference = &callback; *reference = stop; }",
        "fn bad() { let mut byte: Byte = 0; byte = 300; }",
        "type Holder is { byte: Byte }; fn bad() { let mut value = Holder { byte: 0 }; value.byte = 300; }",
    ] {
        assert!(!errors(source).is_empty(), "invalid assignment accepted: {source}");
    }
}

#[test]
fn imported_factories_and_qualified_helpers_preserve_synchronous_bounds() {
    let source = "mount platform.inspect;
        static mut HANDLER: fn() = handler;
        fn handler() { inspect(); }";
    for provider in [
        "public type Outcome<T> is Value(T); public fn inspect() -> Outcome<Int> { Outcome.Value(7) }",
        "public type Error is { public code: Int }; implement Error { public fn create(code: Int) -> Error { Error { code: code } } } public fn inspect() -> Error { Error.create(7) }",
        "public fn message(code: Int) -> Int { code + 1 } public fn inspect() -> Int { platform.message(7) }",
    ] {
        let accepted = errors_with_provider(source, Some(provider));
        assert!(accepted.is_empty(), "{provider}: {accepted:?}");
    }
    let provider = "public type Outcome<T> is Value(T) | Failed(Int);
        public type Error is { public code: Int };
        implement Error { public fn create(code: Int) -> Error { Error { code: code } } }
        extern \"C\" { fn read_value() -> Int; }
        public fn message(code: Int) -> Int { code + 1 }
        public fn inspect() -> Outcome<Error> {
            let code = unsafe { read_value() };
            if code < 0 { Outcome.Failed(platform.message(code)) }
            else { Outcome.Value(Error.create(code)) }
        }";
    let accepted = errors_with_provider(source, Some(provider));
    assert!(accepted.is_empty(), "{accepted:?}");
    for changed in [provider.replace("public fn message", "public async fn message"),
                    provider.replace("public fn create", "public async fn create")] {
        let rejected = errors_with_provider(source, Some(&changed));
        assert!(!rejected.is_empty(), "suspending factory/helper supplied false evidence");
    }
}

#[test]
fn source_contracts_cannot_bypass_shadowing_or_opaque_factories() {
    let source = "mount platform.inspect; static mut HANDLER: fn() = handler;
                  fn handler() { inspect(); }";
    for provider in [
        "public type Error is {}; implement Error { public fn create() -> Error; }
         public fn inspect() -> Error { Error.create() }",
        "public type Error is {}; implement Error { public async fn create() -> Error { Error {} } }
         public fn inspect() -> Error { Error.create() }",
        "type Receiver is {}; implement Receiver { async fn message(&self) -> Int { 7 } }
         static platform: Receiver = Receiver {}; public fn message() -> Int { 7 }
         public fn inspect() -> Int { platform.message() }",
        "type Receiver is {}; implement Receiver { async fn message(&self) -> Int { 7 } }
         public fn message() -> Int { 7 }
         public fn inspect() -> Int { let platform = Receiver {}; platform.message() }",
        "public type Error is {}; implement Error { public fn create() -> Error { Error {} } }
         public fn inspect() -> Error { let Error = async || Error {}; Error() }",
        "public type Event is Value(Int);
         public fn inspect() -> Event { Event.Value(pending()) } async fn pending() -> Int { 7 }",
    ] {
        assert!(!errors_with_provider(source, Some(provider)).is_empty(), "{provider}");
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
fn pure_observation_uses_the_imported_text_length_operation() {
    let caller =
        "mount platform.inspect as measure; pure fn probe(value: &Text) -> Int { measure(value) }";
    let declaration = "@intrinsic(\"text_byte_len\") public fn inspect(value: &Text) -> Int;";
    let accepted = errors_with_provider(caller, Some(declaration));
    assert!(accepted.is_empty(), "{accepted:?}");
    for provider in [
        "public fn inspect(value: &Text) -> Int;",
        "@intrinsic(\"missing_operation\") public pure fn inspect(value: &Text) -> Int;",
        "@intrinsic(\"text_byte_len\") public fn inspect(value: &Text) -> Int { opaque(value) }
         fn opaque(value: &Text) -> Int;",
        "@intrinsic(\"text_byte_len\") public fn inspect(value: &Text = other()) -> Int;
         fn other() -> &Text;",
    ] {
        let rejected = errors_with_provider(caller, Some(provider));
        assert!(
            rejected.iter().any(|error| error.contains("ImpurePureFunction")),
            "{provider}: {rejected:?}"
        );
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
    // This view has a proved finite footprint. Pure functions allow
    // deterministic allocation and failure during value construction.
    let accepted = errors(source);
    assert!(accepted.is_empty(), "{accepted:?}");
    let mutation = r#"
        @intrinsic("atomic_store_u8") fn write_address(ptr:*mut UInt8,value:UInt8,order:UInt8);
        pure fn inspect(ptr:*mut UInt8,value:UInt8,order:UInt8) { write_address(ptr,value,order); }
    "#;
    let rejected = errors(mutation);
    assert!(rejected.iter().any(|error| error.contains("ImpurePureFunction") || error.contains("pure function")),
        "a synchronous store cannot satisfy a pure contract: {rejected:?}");
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
        let first = errors(&source.replace("Second.package(value)", "First.package(value)"));
        assert!(first.is_empty(), "the selected synchronous sibling: {first:?}");
        let rejected = errors(&source);
        assert!(rejected.iter().any(|error| {
            let message = error.to_lowercase();
            message.contains("mismatch") && message.contains("async")
        }), "the unproved sibling must fail its callback contract: {source}: {rejected:?}");
    }
}

#[test]
fn typed_atomic_store_can_run_in_a_synchronous_callback() {
    let declarations = r#"
        @intrinsic("atomic_store_u8") fn publish(ptr:*mut UInt8,value:UInt8,order:UInt8);
        fn accept(callback:fn(*mut UInt8,UInt8,UInt8)) {}
    "#;
    let sync = format!("{declarations} fn install() {{ accept(|p,v,o| publish(p,v,o)); }}");
    let found = errors(&sync);
    assert!(found.is_empty(), "{found:?}");
    let suspending = format!("{declarations} async fn publish_later(p:*mut UInt8,v:UInt8,o:UInt8) {{ publish(p,v,o); }} fn install() {{ accept(publish_later); }}");
    let found = errors(&suspending);
    assert!(found.iter().any(|error| error.contains("Async") || error.contains("async")), "{found:?}");
}

#[test]
fn a_source_factory_alias_preserves_the_selected_declarations_contract() {
    let consumer = "mount platform.inspect; static mut HANDLER: fn() = handler;
                    fn handler() { inspect(); }";
    let provider = "public type Outcome is { code: Int };
        type First is Outcome; type Alias is First;
        implement Outcome { public fn create() -> Outcome { Outcome { code: 7 } } }
        public fn inspect() -> Outcome { Alias.create() }";
    let accepted = errors_with_provider(consumer, Some(provider));
    assert!(accepted.is_empty(), "synchronous alias factory: {accepted:?}");
    for rejected in [
        provider.replace("public fn create", "public async fn create"),
        provider.replace("public fn inspect", "implement Alias { public async fn create() -> Outcome { loop {} } }
            public fn inspect"),
        provider.replace("public fn create() -> Outcome { Outcome { code: 7 } }",
                         "public fn create() -> Outcome;"),
        provider.replace("type First is Outcome", "type First is Alias"),
        provider.replace("type First is Outcome", "type First is UnknownOwner"),
    ] {
        assert!(!errors_with_provider(consumer, Some(&rejected)).is_empty(),
            "unproved source contract accepted: {rejected}");
    }
}
