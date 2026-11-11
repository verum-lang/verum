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
