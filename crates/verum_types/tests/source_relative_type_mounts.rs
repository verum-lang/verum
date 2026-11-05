//! Source signature lookup uses the same mount path rules as module loading.
use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn errors(owner: &str, mount: &str, field: &str, origin: &str) -> Vec<String> {
    let api = format!("{mount} public type Model is {{ handle:{field} }}; public type Driver is protocol {{ fn accept(value:Model); }};");
    let user = format!("mount {owner}.Driver;");
    let sources = [("pkg.raw", origin), (owner, api.as_str()), ("consumer", user.as_str()),
        ("unrelated", "public type Token is (Bool);")];
    let mut registry = ModuleRegistry::new();
    for (index, (name, source)) in sources.iter().enumerate() {
        let ast = FastParser::new().parse_module_str(source, FileId::new(index as u32)).unwrap();
        let id = registry.allocate_id();
        let path = ModulePath::from_str(name);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(module);
    }
    verum_modules::resolve_specific_reexport_kinds(&mut registry).unwrap();
    verum_modules::resolve_glob_reexports(&mut registry).unwrap();
    let ast = registry.get_by_path("consumer").unwrap().ast.clone();
    let mut checker = TypeChecker::new();
    checker.register_builtins();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let mut errors: Vec<_> = checker.prepare_module_declarations(&ast).iter().map(|e| format!("{e:?}")).collect();
    errors.extend(checker.take_deferred_errors().iter().map(|e| format!("{e:?}")));
    errors.extend(checker.diagnostic_sources().iter().map(|e| format!("{e:?}")));
    errors
}
const PUBLIC: &str = "public type Token is (Int);";
#[test]
fn relative_named_nested_and_glob_mounts_share_source_authority() {
    for (owner, mount) in [
        ("pkg.api", "mount super.raw.Token;"),
        ("pkg.api", "mount super.raw.{Token};"),
        ("pkg.api", "mount super.raw.*;"),
        ("pkg.deep.api", "mount super.super.raw.{Token};"),
        ("pkg", "mount .raw.{Token};"),
        ("pkg", "mount self.raw.Token;"),
        ("pkg.api", "mount pkg.{raw.{Token}};"),
    ] {
        let e = errors(owner, mount, "Token", PUBLIC);
        assert!(e.is_empty(), "{owner}: {mount}: {e:#?}");
    }
}
#[test]
fn renamed_relative_mount_keeps_original_carrier() {
    let e = errors("pkg.api", "mount super.raw.{Token as Handle};", "Handle", PUBLIC);
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn renaming_does_not_retain_the_old_local_spelling() {
    let e = errors("pkg.api", "mount super.raw.{Token as Handle};", "Token", PUBLIC);
    assert!(e.iter().any(|e| e.contains("TypeNotFound") && e.contains("Token")), "{e:#?}");
}
#[test]
fn relative_mount_cannot_import_private_type() {
    let e = errors("pkg.api", "mount super.raw.{Token};", "Token", "type Token is (Int);");
    assert!(e.iter().any(|e| e.contains("TypeNotFound") && e.contains("Token")), "{e:#?}");
}
#[test]
fn wrong_relative_owner_does_not_borrow_a_flat_homonym() {
    let e = errors("pkg.api", "mount super.missing.{Token};", "Token", PUBLIC);
    assert!(e.iter().any(|e| e.contains("TypeNotFound") && e.contains("Token")), "{e:#?}");
}
