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
        let wide = "module wide {public type Item is {public left:Int,public right:Int};}";
        let narrow = "module narrow {public type Item is {public left:Bool};}";
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
    let source = "module wide {public type Item is {public left:Int};} mount wide as short; mount cog.wide.{Item}; fn probe(value:Item)->Int {value.left}";
    let errors = check(source, "cog");
    assert!(errors.is_empty(), "{errors:?}");
}

fn assert_private_left(errors: &List<Text>, owner: &str) {
    assert_eq!(
        errors.len(),
        1,
        "one declared private-field refusal: {errors:?}"
    );
    let error = &errors[0];
    assert!(
        error.contains("VisibilityError")
            && error.contains("name: Text { inner: \"left\" }")
            && error.contains("visibility: Text { inner: \"Private\" }")
            && error.contains(&format!("module_path: Text {{ inner: \"{owner}\" }}")),
        "the field must keep its exact declaring owner: {errors:?}"
    );
}

#[test]
fn inline_alias_private_field_keeps_its_owner_beside_a_public_homonym() {
    for reverse in [false, true] {
        let wide = "module wide {public type Item is {left:Int};}";
        let narrow = "module narrow {public type Item is {public left:Bool};}";
        let declarations = if reverse {
            format!("{narrow} {wide}")
        } else {
            format!("{wide} {narrow}")
        };
        let source = format!(
            "{declarations} mount wide as w; mount narrow as n; fn public_value(value:n.Item)->Bool {{value.left}} fn private_value(value:w.Item)->Int {{value.left}}"
        );
        assert_private_left(&check(&source, "cog"), "cog.wide");
    }
}

#[test]
fn named_mount_private_field_keeps_the_whole_module_owner() {
    let source = "module wide {public type Item is {left:Int};} mount wide as short; mount cog.wide.{Item}; fn probe(value:Item)->Int {value.left}";
    assert_private_left(&check(source, "cog"), "cog.wide");
}

#[test]
fn missing_alias_member_cannot_acquire_an_unrelated_mounted_record() {
    let declarations = "module wide {public type Item is {public left:Int};} module empty {public type Other is {public left:Bool};} mount empty as w; mount cog.wide.{Item};";
    let valid = format!("{declarations} fn probe(value:w.Other)->Bool {{value.left}}");
    let errors = check(&valid, "cog");
    assert!(errors.is_empty(), "{errors:?}");
    let missing = format!("{declarations} fn probe(value:w.Item)->Int {{value.left}}");
    let errors = check(&missing, "cog");
    assert!(
        errors.iter().any(|error| {
            (error.contains("VisibilityError") && error.contains("unknown declaration visibility"))
                || error.contains("TypeNotFound")
        }),
        "the absent owner must not resolve through mounted Item: {errors:?}"
    );
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

#[test]
fn renamed_inline_types_keep_fields_and_owners_before_and_after_module_checks() {
    for mount in [
        "mount cog.wide.Item as W; mount cog.narrow.Item as N;",
        "mount cog.{wide.Item as W, narrow.Item as N};",
        "mount cog.{wide.{Item as W}, narrow.{Item as N}};",
    ] {
        for reverse in [false, true] {
            let wide = "module wide { public type Item is { public value: Int }; }";
            let narrow = "module narrow { public type Item is { public value: Bool }; }";
            let declarations = if reverse { format!("{narrow} {wide}") } else { format!("{wide} {narrow}") };
            let functions = "fn first(item:W)->Int {item.value} fn second(item:N)->Bool {item.value}";
            for source in [format!("{mount} {functions} {declarations}"), format!("{declarations} {mount} {functions}")] {
                let errors = check(&source, "cog");
                assert!(errors.is_empty(), "{source}: {errors:?}");
            }
        }
    }
}

#[test]
fn renamed_inline_type_retains_private_field_visibility() {
    for mount in [
        "mount cog.wide.Item as W; mount cog.narrow.Item as N;",
        "mount cog.{wide.{Item as W}, narrow.{Item as N}};",
    ] {
        let source = format!("{mount} module wide {{ public type Item is {{ left:Int }}; }} module narrow {{ public type Item is {{ public left:Int }}; }} fn public_value(item:N)->Int {{item.left}} fn private_value(item:W)->Int {{item.left}}");
        assert_private_left(&check(&source, "cog"), "cog.wide");
    }
}

#[test]
fn same_shaped_inline_aliases_do_not_identify_distinct_owners() {
    let source = "mount cog.wide.Item as W; mount cog.narrow.Item as N; module wide {public type Item is {public value:Int};} module narrow {public type Item is {public value:Int};} fn wrong(item:W)->N {item}";
    let errors = check(source, "cog");
    assert!(errors.iter().any(|error| error.contains("Mismatch")), "{errors:?}");
}

#[test]
fn renamed_inline_generic_types_keep_their_parameter_slots() {
    let source = "mount cog.wide.Item as W; mount cog.narrow.Item as N; module wide {public type Item<A,B> is {public first:A,public second:B};} module narrow {public type Item<A> is {public first:A};} fn first(item:W<Int,Bool>)->Bool {item.second} fn second(item:N<Int>)->Int {item.first}";
    let errors = check(source, "cog");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn renamed_inline_type_does_not_replace_another_mounts_local_binding() {
    for mounts in [
        "mount cog.wide.*; mount cog.narrow.Item as N;",
        "mount cog.narrow.Item as N; mount cog.wide.*;",
        "mount cog.{wide.*, narrow.Item as N};",
        "mount cog.{narrow.Item as N, wide.*};",
    ] {
        let source = format!("{mounts} fn wide(item:Item)->Int {{item.value}} fn narrow(item:N)->Bool {{item.value}} module wide {{public type Item is {{public value:Int}};}} module narrow {{public type Item is {{public value:Bool}};}}");
        let errors = check(&source, "cog");
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn renamed_inline_type_does_not_authorize_the_original_spelling() {
    let source = "mount cog.other.Item as Selected; fn wrong(item:Item)->Int {item.value} module other {public type Item is {public value:Int};}";
    let errors = check(source, "cog");
    assert!(errors.iter().any(|error| error.contains("TypeNotFound") && error.contains("Item")), "{errors:?}");
}

fn imported_factory_errors(file: &str, separate_child: Option<&str>, body: &str) -> List<Text> {
    use verum_ast::{FileId, Module};
    use verum_fast_parser::FastParser;
    use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};

    let parser = FastParser::new();
    let parent = parser.parse_module_str(file, FileId::new(1)).unwrap();
    let child = match separate_child {
        Some(source) => parser.parse_module_str(source, FileId::new(2)).unwrap(),
        None => parent.items.iter().find_map(|item| match &item.kind {
            ItemKind::Module(decl) => Some(Module::new(decl.items.clone()?, parent.file_id, item.span)),
            _ => None,
        }).expect("inline body"),
    };
    let mut registry = ModuleRegistry::new();
    for (index, (owner, ast)) in [("demo.family", parent), ("demo.family.Parcel", child)].into_iter().enumerate() {
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let file_id = ast.file_id;
        let mut info = ModuleInfo::new(id, path, ast, file_id, Text::new());
        info.exports = exports;
        registry.register(info);
    }

    let source = format!("mount demo.family.Parcel.pack; type Batch<T> is {{ public foreign: T }}; fn consume() -> Int {{ {body} }}");
    let ast = parser.parse_module_str(&source, FileId::new(3)).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("demo.client");
    checker.set_module_registry_direct(registry);
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    errors.iter().chain(checker.diagnostic_sources().iter()).map(|e| format!("{e:?}").into()).collect()
}

#[test]
fn imported_factory_uses_its_enclosing_declarations_and_preserves_its_signature() {
    let parent = "public type Parcel<T> is { public item: T }; public module Parcel {
        public fn pack<T>(item: T) -> Batch<T> { Batch { item } }
    } public type Batch<T> is { public item: T };";
    for source in [
        parent.to_owned(),
        parent.replace("public fn pack", "mount super.Batch as Bundle; public fn pack")
            .replace("-> Batch<T> { Batch", "-> Bundle<T> { Bundle"),
    ] {
        let valid = imported_factory_errors(&source, None, "pack(47).item");
        assert!(valid.is_empty(), "{valid:?}");
        let wrong = imported_factory_errors(&source, None, "pack(true).item");
        assert!(wrong.iter().any(|e| e.contains("Mismatch") && e.contains("Bool")), "{wrong:?}");
    }
}

#[test]
fn enclosing_types_require_visibility_and_an_actual_inline_body() {
    let function = "public fn pack<T>(item: T) -> Batch<T> { Batch { item } }";
    let record = "public type Batch<T> is { public item: T };";
    let private = format!("public module Parcel {{ {function} }} {}", record.replace("public type", "private type"));
    let missing = format!("public module Parcel {{ mount demo.absent.Batch; {function} }} {record}");
    let separate = format!("public module Parcel; {record}");
    for (source, child) in [(&private, None), (&missing, None), (&separate, Some(function))] {
        let errors = imported_factory_errors(source, child, "pack(47).item");
        assert!(errors.iter().any(|e| (e.contains("TypeNotFound") || e.contains("VisibilityError")) && e.contains("Batch")), "{source}: {errors:?}");
    }
}
