//! A public type and its same-named factory module share an import, not an owner.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn errors(owner: &str, consumer: &str) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (index, (path, source)) in [("app.factory", owner), ("app.main", consumer)].into_iter().enumerate() {
        let id = ModuleId::new(index as u32);
        let file = FileId::new(index as u32);
        let ast = FastParser::new().parse_module_str(source, file).expect("grammar");
        let path = ModulePath::from_str(path);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("exports");
        registry.register(info);
    }
    let ast = FastParser::new().parse_module_str(consumer, FileId::new(1)).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("app");
    checker.set_current_module_path("app.main");
    checker.set_module_registry_direct(registry.clone());
    let mut errors = List::new();
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Mount(import) => checker.process_import(import, "app.main", &registry),
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result { errors.push(format!("{error:?}").into()); }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| format!("{error:?}").into()));
    errors.extend(checker.diagnostic_sources().iter().map(|error| format!("{error:?}").into()));
    errors
}

const FACTORY: &str = r#"
public type Factory is { value: Int };
public module Factory {
    public fn make(value: Int) -> Int { value }
    fn secret() -> Int { 9 }
}
"#;

#[test]
fn imported_type_retains_its_public_companion_factories() {
    for mount in ["mount app.factory.Factory;", "mount app.factory.{Factory};", "mount app.factory.{Factory as Build};"] {
        let name = if mount.contains("as Build") { "Build" } else { "Factory" };
        let errors = errors(FACTORY, &format!("{mount} fn probe() -> Int {{ {name}.make(7) }}"));
        assert!(errors.is_empty(), "{mount}: {errors:?}");
    }
}

#[test]
fn companion_factory_checks_arguments_and_visibility() {
    for call in ["Factory.make(true)", "Factory.make()", "Factory.secret()"] {
        let errors = errors(FACTORY, &format!("mount app.factory.Factory; fn probe() -> Int {{ {call} }}"));
        assert!(!errors.is_empty(), "must reject {call}");
    }
}

#[test]
fn private_companion_module_is_not_exported_with_a_public_type() {
    let owner = FACTORY.replace("public module Factory", "module Factory");
    let errors = errors(&owner, "mount app.factory.Factory; fn probe() -> Int { Factory.make(7) }");
    assert!(!errors.is_empty(), "private module must remain private");
}

fn metadata_errors(declaring_module: &str, public: bool, call: &str, eager: bool) -> List<Text> {
    use std::sync::Arc;
    use verum_common::{Maybe, ResourceDiscipline};
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata::default();
    metadata.types.insert("core.factory.Factory".into(), TypeDescriptor {
        name: "Factory".into(), module_path: "core.factory".into(), origin_module_path: Maybe::None,
        generic_params: List::new(), kind: TypeDescriptorKind::Record { fields: List::new() },
        size: Maybe::None, alignment: Maybe::None, methods: List::new(), implements: List::new(),
        decl_span: Maybe::None, is_public: true, is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    });
    let descriptor: FunctionDescriptor = serde_json::from_value(serde_json::json!({
        "name": "Factory.make", "module_path": "bundle", "origin_module_path": declaring_module,
        "generic_params": [], "params": [{"name": "value", "ty": "Int", "has_default": false}],
        "return_type": "Int", "contexts": [], "is_async": false, "is_unsafe": false,
        "intrinsic_id": null, "parent_type": null, "is_public": public
    })).unwrap();
    // Exercise the legacy compatibility index as well as exact-owner validation.
    metadata.functions.insert("Factory.make".into(), descriptor);
    let mut checker = if eager { TypeChecker::new_with_core_eager(Arc::new(metadata)) }
        else { TypeChecker::new_with_core(Arc::new(metadata)) };
    checker.set_current_cog("app");
    checker.set_current_module_path("app.main");
    let source = format!("mount core.factory.Factory; fn probe() -> Int {{ {call} }}");
    let ast = FastParser::new().parse_module_str(&source, FileId::new(0)).unwrap();
    checker.register_stdlib_types_for_module(&ast);
    for item in &ast.items {
        if let ItemKind::Function(decl) = &item.kind { checker.register_function_signature(decl).unwrap(); }
    }
    let mut errors: List<Text> = ast.items.iter().filter_map(|item| checker.check_item(item).err().map(|e| format!("{e:?}").into())).collect();
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| format!("{error:?}").into()));
    errors.extend(checker.diagnostic_sources().iter().map(|error| format!("{error:?}").into()));
    errors
}

#[test]
fn metadata_companion_factory_uses_its_declaring_owner() {
    for eager in [false, true] {
        let errors = metadata_errors("core.factory", true, "Factory.make(7)", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        for (owner, public, call) in [
            ("core.foreign", true, "Factory.make(7)"),
            ("core.factory", false, "Factory.make(7)"),
            ("core.factory", true, "Factory.make(true)"),
        ] {
            let errors = metadata_errors(owner, public, call, eager);
            assert!(!errors.is_empty(), "must reject {owner} public={public} {call} eager={eager}");
        }
    }
}
