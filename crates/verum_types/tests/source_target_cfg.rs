use verum_ast::cfg::TargetConfig;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn source_import_errors(
    target: &str,
    host: &str,
    check_functions: bool,
) -> verum_common::List<verum_common::Text> {
    use verum_ast::{FileId, ItemKind};
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in [
        ("demo.windows", "public type Native is { public tag:Int };"),
        ("demo.macos", "public type Native is { public tag:Bool };"),
        ("demo.host", host),
    ]
    .into_iter()
    .enumerate()
    {
        let ast = Parser::new(source).parse_module().unwrap();
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            ast.clone(),
            FileId::new(index as u32),
            source.into(),
        );
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
    }
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_cfg_target(TargetConfig::for_target(target));
    checker.set_module_registry_direct(registry);
    checker.set_current_module_path("demo.host");
    let ast = Parser::new(host).parse_module().unwrap();
    let mut errors = verum_common::List::new();
    // Source declarations can be loaded by an earlier consumer before the
    // ordinary two-pass module preparation reaches their defining file.
    checker.with_source_signature_scope("demo.host", |checker| {
        for item in &ast.items {
            if let ItemKind::Type(declaration) = &item.kind {
                if let Err(error) = checker.register_type_declaration(declaration) {
                    errors.push(error);
                }
            }
        }
    });
    if check_functions {
        checker.set_user_code_phase();
        checker.register_all_type_names(&ast.items);
        for result in checker.resolve_all_type_definitions(&ast.items) {
            if let Err(error) = result {
                errors.push(error);
            }
        }
        for item in &ast.items {
            if matches!(item.kind, ItemKind::Function(_)) {
                if let Err(error) = checker.check_item(item) {
                    errors.push(error);
                }
            }
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources().iter())
        .map(|error| format!("{error:?}").into())
        .collect()
}

const HOST: &str = r#"module demo.host;
    @cfg(target_os = "windows") mount demo.windows.Native as Selected;
    @cfg(target_os = "macos") mount demo.macos.Native as Selected;
    public type Handle<T> is { public native:Selected, public value:T };
"#;

#[test]
fn repeated_type_registration_resolves_imports_in_its_declaring_target() {
    for (target, result) in [
        ("aarch64-apple-darwin", "Bool"),
        ("x86_64-pc-windows-msvc", "Int"),
    ] {
        let source = format!("{HOST} fn probe(value:Handle<Int>)->{result} {{value.native.tag}}");
        let errors = source_import_errors(target, &source, true);
        assert!(errors.is_empty(), "{target}: {errors:#?}");
    }
}

#[test]
fn inactive_import_cannot_supply_a_missing_selected_declaration() {
    let source = HOST.replace("demo.macos.Native", "demo.absent.Native");
    let errors = source_import_errors("aarch64-apple-darwin", &source, false);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("TypeNotFound") && error.contains("Selected")),
        "{errors:#?}"
    );
}

#[test]
fn statement_selection_uses_the_installed_compilation_target() {
    let module = Parser::new(
        "fn probe() -> Int { @cfg(target_os = \"windows\") let incompatible: Bool = 42; 42 }",
    )
    .parse_module()
    .unwrap();
    for (target, allowed) in [
        ("aarch64-apple-darwin", true),
        ("x86_64-pc-windows-msvc", false),
    ] {
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        checker.set_cfg_target(TargetConfig::for_target(target));
        let mut errors = checker.prepare_module_declarations(&module);
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
        errors.extend(checker.take_deferred_errors());
        let errors: verum_common::List<_> = errors
            .iter()
            .chain(checker.diagnostic_sources().iter())
            .map(|error| format!("{error:?}"))
            .collect();
        if allowed {
            assert!(errors.is_empty(), "{target}: {errors:?}");
        } else {
            assert!(
                errors
                    .iter()
                    .any(|error| format!("{error:?}").contains("Bool")),
                "target condition ignored: {errors:?}"
            );
        }
    }
}

fn imported_implementation_errors(
    target: &str,
    provider: &str,
    consumer: &str,
    preload: bool,
) -> verum_common::List<verum_common::Text> {
    use verum_ast::FileId;
    use verum_fast_parser::FastParser;
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };

    let parser = FastParser::new();
    let source = parser.parse_module_str(provider, FileId::new(1)).unwrap();
    let owner = ModulePath::from_str("demo.backend");
    let id = ModuleId::new(0);
    let exports = extract_exports_from_module(&source, id, &owner).unwrap();
    let mut info = ModuleInfo::new(id, owner, source.clone(), FileId::new(1), provider.into());
    info.exports = exports;
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    let consumer = parser.parse_module_str(consumer, FileId::new(2)).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_cfg_target(TargetConfig::for_target(target));
    checker.set_current_module_path("demo.app");
    checker.set_module_registry_direct(registry);
    if preload {
        let library = [("demo.backend".into(), std::sync::Arc::new(source))];
        checker.prepare_source_library_types(&library);
        checker.prepare_source_library_implementations(&library);
    }
    let mut errors = checker.prepare_module_declarations(&consumer);
    for item in &consumer.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources().iter())
        .map(|error| format!("{error:?}").into())
        .collect()
}

#[test]
fn imported_implementations_participate_only_on_their_declared_targets() {
    let provider = r#"
        public type Read is protocol { fn read(&self) -> Int; };
        public type Device is { public value: Int };
        @cfg(target_os = "linux")
        implement Read for Device { fn read(&self) -> Int { 1 } }
        @cfg(target_os = "macos")
        implement Read for Device { fn read(&self) -> Int { 2 } }
        @cfg(target_os = "linux")
        implement Device { public fn linux_only(&self) -> Int { 3 } }
    "#;
    let consumer =
        "mount demo.backend.{Read, Device}; fn use_device(d: Device) -> Int { d.read() }";
    for target in ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"] {
        for preload in [false, true] {
            let errors = imported_implementation_errors(target, provider, consumer, preload);
            assert!(errors.is_empty(), "{target}, preload={preload}: {errors:?}");
        }
    }
    let missing = consumer.replace("d.read()", "d.linux_only()");
    let duplicate = provider.replace(r#"target_os = "linux""#, r#"target_os = "macos""#);
    for preload in [false, true] {
        let errors =
            imported_implementation_errors("aarch64-apple-darwin", provider, &missing, preload);
        assert!(
            errors.iter().any(|e| e.contains("linux_only")),
            "{errors:?}"
        );
        let errors =
            imported_implementation_errors("aarch64-apple-darwin", &duplicate, consumer, preload);
        assert!(
            errors.iter().any(|e| e.contains("Overlapping")),
            "{errors:?}"
        );
    }
}
