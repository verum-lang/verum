//! Source signatures retain actual language declarations without borrowing caller aliases.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::{Type, TypeChecker};

fn check(checker: &mut TypeChecker, owner: &str, sources: &[(&str, &str)]) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (index, (name, source)) in sources.iter().enumerate() {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(name);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut module = ModuleInfo::new(id, path, ast, FileId::new(index as u32), (*source).into());
        module.exports = exports;
        registry.register(module);
    }
    let ast = registry.get_by_path(owner).expect("selected owner").ast.clone();
    checker.set_module_registry_direct(registry);
    checker.set_current_module_path(owner);
    let mut errors: List<Text> = checker.prepare_module_declarations(&ast)
        .into_iter().map(|error| format!("{error:?}").into()).collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| Text::from(format!("{error:?}"))));
    errors.extend(checker.diagnostic_sources().iter().map(|error| Text::from(format!("{error:?}"))));
    errors
}

fn checker() -> TypeChecker {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker
}

fn declare_foreign_alias(checker: &mut TypeChecker, source: &str) {
    let ast = Parser::new(source).parse_module().expect("alias grammar");
    let ItemKind::Type(declaration) = &ast.items[0].kind else { panic!("type declaration") };
    checker.set_current_module_path("foreign");
    checker.register_type_declaration(declaration).expect("valid foreign declaration");
}

#[test]
fn registered_language_aliases_survive_direct_and_imported_source_scopes() {
    for name in ["RawPtr", "Epoch", "u32"] {
        let provider = format!("public type Slot is {{ public pointer: {name} }};");
        let sources = [
            ("realm.storage", provider.as_str()),
            ("realm.client", "mount realm.storage.Slot; fn inspect(slot: &Slot) {}"),
        ];
        for owner in ["realm.storage", "realm.client"] {
            let errors = check(&mut checker(), owner, &sources);
            assert!(errors.is_empty(), "{name} in {owner}: {errors:?}");
        }
    }
}

#[test]
fn a_scalar_shaped_foreign_alias_is_not_a_language_declaration() {
    for target in ["Int", "Bool", "RawPtr"] {
        let mut checker = checker();
        declare_foreign_alias(&mut checker, &format!("type ForeignValue is {target};"));
        checker.register_builtins();
        let errors = check(&mut checker, "realm.storage", &[
            ("realm.storage", "public type Slot is { pointer: ForeignValue };"),
        ]);
        assert!(errors.iter().any(|error| error.contains("TypeNotFound") && error.contains("ForeignValue")),
            "unmounted alias of {target} acquired builtin authority: {errors:?}");
    }
}

#[test]
fn source_declarations_can_lexically_shadow_language_aliases() {
    let mut checker = checker();
    let errors = check(&mut checker, "realm.storage", &[
        ("realm.storage", "public type RawPtr is {public value:Bool}; public type Slot is {public pointer:RawPtr}; fn inspect(slot:Slot)->Bool {slot.pointer.value}"),
    ]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_explicit_source_mount_selects_its_declared_alias_owner() {
    let mut checker = checker();
    let errors = check(&mut checker, "realm.storage", &[
        ("foreign", "public type RawPtr is {public value:Bool};"),
        ("realm.storage", "mount foreign.RawPtr; public type Slot is {public pointer:RawPtr}; fn inspect(slot:Slot)->Bool {slot.pointer.value}"),
    ]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_foreign_homonym_cannot_replace_a_registered_language_alias() {
    let mut checker = checker();
    declare_foreign_alias(&mut checker, "type RawPtr is Bool;");
    let errors = check(&mut checker, "realm.storage", &[
        ("realm.storage", "public type Slot is {public pointer:RawPtr};"),
    ]);
    assert!(errors.is_empty(), "{errors:?}");
    let Some(Type::Record(fields)) = checker.resolved_type("__struct_fields_realm.storage.Slot") else { panic!("declared fields") };
    assert!(matches!(fields.get(&Text::from("pointer")), Some(Type::Pointer {mutable:true, inner}) if **inner == Type::Int), "{fields:?}");
}

#[test]
fn registration_is_required_before_a_language_alias_is_available() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    let errors = check(&mut checker, "realm.storage", &[
        ("realm.storage", "public type Slot is {pointer:RawPtr};"),
    ]);
    assert!(errors.iter().any(|error| error.contains("TypeNotFound") && error.contains("RawPtr")), "{errors:?}");
}
