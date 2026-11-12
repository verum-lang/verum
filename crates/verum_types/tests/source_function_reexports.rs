//! Function imports use the selected export owner, including namespace aliases.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn errors(provider: &str, reexport: &str, client: &str) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    registry.register_path_alias("formats", "workspace.config.format");
    for (index, (owner, source)) in [
        ("workspace.config.format", provider),
        ("workspace.config", reexport),
    ].iter().enumerate() {
        let ast = FastParser::new().parse_module_str(source, FileId::new(index as u32 + 1)).unwrap();
        let id = registry.allocate_id();
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, (*source).into());
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
    }
    if let Err(error) = verum_modules::resolve_specific_reexport_kinds(&mut registry) {
        return List::from_single(error.to_string().into());
    }
    if let Err(error) = verum_modules::resolve_glob_reexports(&mut registry) {
        return List::from_single(error.to_string().into());
    }
    let ast = FastParser::new().parse_module_str(client, FileId::new(0)).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_module_registry_direct(registry);
    let mut errors: List<Text> = checker.prepare_module_declarations(&ast).into_iter()
        .map(|error| error.to_string().into()).collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(error.to_string().into()); }
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| error.to_string().into()));
    errors.extend(checker.diagnostic_sources().iter().map(|error| error.to_string().into()));
    errors
}

#[test]
fn namespace_alias_reexports_preserve_the_actual_function_signature() {
    for reexport in ["public mount formats.create;", "public mount formats.{create};", "public mount formats.*;",
                     "public mount formats.original as create;"] {
        let provider = if reexport.contains("original") {
            "public fn original() -> Int { 73 }"
        } else { "public fn create() -> Int { 73 }" };
        let result = errors(provider, reexport,
            "mount workspace.config.create; fn answer() -> Int { create() }");
        assert!(result.is_empty(), "{reexport}: {result:?}");
        let rejected = errors(provider, reexport,
            "mount workspace.config.create; fn answer() -> Text { create() }");
        assert!(!rejected.is_empty(), "the re-export discarded its return type");
    }
}

#[test]
fn reexported_functions_keep_privacy_and_argument_obligations() {
    for provider in ["fn create() -> Int { 73 }", "public fn create(value: Int) -> Int { value }"] {
        let result = errors(provider, "public mount formats.{create};",
            "mount workspace.config.create; fn answer() -> Int { create() }");
        assert!(!result.is_empty(), "{provider}");
    }
}
