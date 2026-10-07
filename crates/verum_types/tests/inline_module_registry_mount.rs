//! The ordinary compiler preregisters inline declarations separately from its file registry.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn check(source: &str, parent: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str("control"),
        ast.clone(),
        verum_ast::FileId::new(0),
        source.into(),
    ));
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("control");
    checker.set_module_registry_direct(registry.clone());
    // Match phase_type_check: registry contains the file, while this pass
    // publishes each inline namespace. Do not inject synthetic registry modules.
    for item in &ast.items {
        if let ItemKind::Module(module) = &item.kind {
            checker.pre_register_module_public(module, parent);
        }
    }
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "control", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => {
                if let Err(error) = checker.register_type_declaration(decl) {
                    errors.push(format!("{error:?}").into());
                }
            }
            ItemKind::Function(decl) => {
                if let Err(error) = checker.register_function_signature(decl) {
                    errors.push(format!("{error:?}").into());
                }
            }
            _ => {}
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors
}

#[test]
fn root_inline_alias_is_known_before_the_checker_visits_module_bodies() {
    for path in ["wide", "cog.wide"] {
        let source = format!(
            "mount {path} as short; module wide {{ public type Item is {{left:Int,right:Int}}; }} fn probe()->Int {{short.Item.size}}"
        );
        let errors = check(&source, "cog");
        assert!(errors.is_empty(), "{path}: {errors:?}");
    }
}

#[test]
fn inline_aliases_keep_same_leaf_types_and_lexical_values_distinct() {
    for reverse in [false, true] {
        let wide = "module wide {public type Item is {left:Int,right:Int};}";
        let narrow = "module narrow {public type Item is {left:Bool};}";
        let declarations = if reverse {
            format!("{narrow} {wide}")
        } else {
            format!("{wide} {narrow}")
        };
        let source = format!(
            "{declarations} mount wide as w; mount narrow as n; fn probe(a:w.Item,b:n.Item)->Bool {{a.left==37 && b.left}} type Cell is {{size:Bool}}; type Root is {{Item:Cell}}; fn shadow(w:Root)->Bool {{w.Item.size}}"
        );
        let errors = check(&source, "cog");
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn nested_module_requires_its_declared_path() {
    let declarations = "module outer {public module wide {public type Item is {left:Int};}}";
    let valid =
        format!("{declarations} mount outer.wide as short; fn probe()->Int {{short.Item.size}}");
    let errors = check(&valid, "cog");
    assert!(errors.is_empty(), "{errors:?}");
    let invalid =
        format!("{declarations} mount wide as short; fn probe()->Int {{short.Item.size}}");
    assert!(
        check(&invalid, "cog")
            .iter()
            .any(|e| e.contains("ImportModuleNotFound"))
    );
}

#[test]
fn file_owned_inline_module_does_not_acquire_a_root_alias() {
    let declarations = "module wide {public type Item is {left:Int};}";
    let valid =
        format!("{declarations} mount owner.wide as short; fn probe()->Int {{short.Item.size}}");
    let errors = check(&valid, "owner");
    assert!(errors.is_empty(), "{errors:?}");
    let invalid =
        format!("{declarations} mount wide as short; fn probe()->Int {{short.Item.size}}");
    assert!(
        check(&invalid, "owner")
            .iter()
            .any(|e| e.contains("ImportModuleNotFound"))
    );
}

#[test]
fn whole_module_and_named_mounts_share_one_import_source() {
    let source = "module wide {public type Item is {left:Int};} mount wide as short; mount cog.wide.{Item}; fn probe(value:Item)->Int {value.left}";
    let errors = check(source, "cog");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_outline_module_declaration_does_not_supply_an_inline_body() {
    let source = "module outer {public module wide;} mount outer.wide as short; fn probe() {}";
    let errors = check(source, "cog");
    assert!(
        !errors.is_empty(),
        "an unloaded file module is not a known inline namespace"
    );
}
