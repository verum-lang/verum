//! Declaration roles survive source/metadata registration and scope changes.
//! Property-result and privacy oracles live in record_field_shadowed_properties.

use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Maybe};
use verum_fast_parser::FastParser;
use verum_modules::{
    ExportKind, ExportedItem, ModuleId, ModuleInfo, ModulePath, ModuleRegistry,
    extract_exports_from_module,
};
use verum_types::{
    Type, TypeChecker,
    context::{TypeEnv, TypeScheme},
    core_metadata::{CoreMetadata, FunctionDescriptor, StaticDescriptor},
};

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("fixture follows the production grammar")
}

#[test]
fn declaration_roles_follow_the_nearest_binding_and_its_lifetime() {
    // Every occupant deliberately has the same type: equality proves no role.
    let scheme = TypeScheme::mono(Type::int());
    let mut env = TypeEnv::new();
    env.insert_value("Item", scheme.clone());
    assert!(env.is_bound_value("Item"));
    assert!(env.insert_binding_alias("Copy", "Item"));
    assert!(env.is_bound_value("Copy"));
    env.insert("Copy", scheme.clone());
    assert!(!env.is_bound_value("Copy"));
    env.insert("Item", scheme.clone());
    assert!(
        !env.is_bound_value("Item"),
        "plain root overwrite clears role"
    );
    env.insert_value("Item", scheme.clone());
    assert!(env.remove("Item"));
    assert!(!env.is_bound_value("Item"));
    env.insert_value("Item", scheme.clone());
    env.push_scope();
    assert!(
        env.is_bound_value("Item"),
        "visible root value survives push"
    );
    env.insert_type_parameter_mirror("Item", scheme.clone());
    assert!(!env.is_bound_value("Item"), "inner mirror hides root value");
    env.insert("Item", scheme.clone());
    assert!(env.is_bound_value("Item"), "lexical value replaces mirror");
    env.insert_type_parameter_mirror("Item", scheme.clone());
    assert!(
        !env.is_bound_value("Item"),
        "type mirror is not a value root"
    );
    assert!(env.insert_binding_alias("Copy", "Item"));
    assert!(!env.is_bound_value("Copy"));
    assert!(env.remove("Item"));
    assert!(env.is_bound_value("Item"), "removal reveals root value");
    env.insert_type_parameter_mirror("Item", scheme.clone());
    env.pop_scope();
    assert!(env.is_bound_value("Item"), "pop restores actual root role");
    let mut child = env.child();
    child.insert_root("Item", scheme.clone());
    assert!(
        !child.is_bound_value("Item"),
        "root overwrite clears inherited role"
    );
    child.pop_scope();
    assert!(!child.is_bound_value("Item"));
    assert!(
        env.is_bound_value("Item"),
        "child clone does not mutate parent"
    );
    env.insert_type_parameter_mirror("Item", scheme.clone());
    assert!(!env.is_bound_value("Item"));
    assert!(env.insert_binding_alias("Copy", "Item"));
    assert!(env.is_type_parameter_mirror("Copy"));
    assert!(!env.is_bound_value("Copy"));
    env.insert_value("Item", scheme);
    assert!(env.is_bound_value("Item"));
    assert!(!env.is_type_parameter_mirror("Item"));
}

#[test]
fn source_const_static_and_function_registration_publish_value_roles() {
    let ast = parse(
        "const i32: Bool = true; static u64: Bool = true; fn f32() -> Bool { true }",
        0,
    );
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    for item in &ast.items {
        match &item.kind {
            ItemKind::Const(decl) => {
                checker.pre_register_const(decl);
                assert!(checker.context_mut().env.is_bound_value("i32"));
                checker.context_mut().env.remove("i32");
                checker
                    .register_const_declaration(decl)
                    .expect("const registration");
            }
            ItemKind::Static(decl) => checker.pre_register_static(&decl.name.name, Type::bool()),
            ItemKind::Function(decl) => checker
                .register_function_signature(decl)
                .expect("fn registration"),
            _ => panic!("fixture declaration"),
        }
    }
    for name in ["i32", "u64", "f32"] {
        assert!(
            checker.context_mut().env.is_bound_value(name),
            "registered {name}"
        );
    }
    for item in &ast.items {
        checker.check_item(item).expect("declaration checks");
    }
    for name in ["i32", "u64", "f32"] {
        assert!(
            checker.context_mut().env.is_bound_value(name),
            "checked {name}"
        );
    }
    checker
        .register_const_from_metadata("u32", "Bool")
        .expect("const descriptor");
    assert!(checker.context_mut().env.is_bound_value("u32"));
}

fn register_module(registry: &mut ModuleRegistry, owner: &str, source: &str, file: u32) {
    let ast = parse(source, file);
    let id = ModuleId::new(file);
    let path = ModulePath::from_str(owner);
    let mut info = ModuleInfo::new(
        id,
        path.clone(),
        ast.clone(),
        FileId::new(file),
        source.into(),
    );
    info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
    registry.register(info);
}

fn mount(checker: &mut TypeChecker, registry: &ModuleRegistry, source: &str) {
    let ast = parse(source, 9);
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    for item in &ast.items {
        let ItemKind::Mount(decl) = &item.kind else {
            panic!("fixture mount");
        };
        checker
            .process_import(decl, "consumer", registry)
            .expect("mount must resolve independently");
    }
}

#[test]
fn source_mounts_and_reexports_carry_roles_without_classifying_function_types() {
    let mut registry = ModuleRegistry::new();
    register_module(
        &mut registry,
        "core.reexport.roles",
        "public const VALUE: Bool = true; public static SLOT: Bool = true; public fn callable() -> Bool { true } public type Choice is Build(Int);",
        0,
    );
    register_module(
        &mut registry,
        "core.reexport",
        "public mount core.reexport.roles.{VALUE, SLOT, callable, Build};",
        1,
    );
    for owner in ["core.reexport.roles", "core.reexport"] {
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        mount(
            &mut checker,
            &registry,
            &format!("mount {owner}.{{VALUE as i32, SLOT as u64, callable as f32, Build}};"),
        );
        for name in ["i32", "u64", "f32"] {
            assert!(
                checker.context_mut().env.is_bound_value(name),
                "{owner}.{name}"
            );
        }
        assert!(
            checker.ctx_env_lookup("Build").is_some(),
            "constructor was imported"
        );
        assert!(
            checker.context_mut().env.is_bound_value("Build"),
            "an exported variant is a declared value"
        );
    }
}

fn function(name: &str, is_const: bool) -> FunctionDescriptor {
    FunctionDescriptor {
        name: name.into(),
        module_path: "core.roles".into(),
        origin_module_path: Maybe::None,
        generic_params: List::new(),
        params: List::new(),
        return_type: "Bool".into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: Maybe::None,
        parent_type: Maybe::None,
        impl_generic_names: List::new(),
        is_const,
        decl_span: Maybe::None,
        is_public: true,
        explicit_type_param_ids: Maybe::None,
    }
}

#[test]
fn decoded_metadata_mounts_keep_const_static_and_function_roles() {
    let mut metadata = CoreMetadata::default();
    metadata
        .functions
        .insert("core.roles.VALUE".into(), function("VALUE", true));
    metadata
        .functions
        .insert("core.roles.callable".into(), function("callable", false));
    metadata.statics.insert(
        "core.roles.SLOT".into(),
        StaticDescriptor {
            name: "SLOT".into(),
            module_path: "core.roles".into(),
            ty: "Bool".into(),
            is_mut: false,
        },
    );
    let bytes = serde_json::to_vec(&metadata).expect("metadata encoding");
    let metadata: CoreMetadata =
        serde_json::from_slice(&bytes).expect("independent metadata decode");
    let metadata = Arc::new(metadata);
    let mut registry = ModuleRegistry::new();
    // Match the archive loader's static export surface, retaining an empty AST:
    // the type and binding must still come from the decoded static descriptor.
    let id = ModuleId::new(0);
    let mut info = ModuleInfo::new(
        id,
        ModulePath::from_str("core.roles"),
        parse("", 0),
        FileId::new(0),
        "".into(),
    );
    info.exports
        .add_export(ExportedItem::new(
            "SLOT",
            ExportKind::Static,
            verum_ast::Visibility::Public,
            id,
            verum_ast::Span::dummy(),
        ))
        .expect("static export");
    registry.register(info);
    for eager in [false, true] {
        let mut checker = if eager {
            TypeChecker::new_with_core_eager(metadata.clone())
        } else {
            TypeChecker::new_with_core(metadata.clone())
        };
        checker.register_primitives();
        mount(
            &mut checker,
            &registry,
            "mount core.roles.{VALUE as i32, SLOT as u64, callable as f32};",
        );
        for name in ["i32", "u64", "f32"] {
            assert!(
                checker.context_mut().env.is_bound_value(name),
                "eager={eager}: {name}"
            );
        }
        let probe = parse("fn probe() -> Bool { i32 }", 10);
        checker
            .check_item(&probe.items[0])
            .expect("metadata const remains a Bool value");
    }
}

#[test]
fn inline_value_alias_keeps_the_imported_declaration_role() {
    for mount in [
        "mount owner.values.VALUE as i32;",
        "mount owner.values.{VALUE as i32};",
    ] {
        let source = format!("module values {{ public const VALUE: Bool = true; }} {mount}");
        let ast = parse(&source, 0);
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        checker.set_current_module_path("owner");
        for item in &ast.items {
            if let ItemKind::Module(module) = &item.kind {
                checker.pre_register_module_public(module, "owner");
            }
        }
        for item in &ast.items {
            if matches!(item.kind, ItemKind::Mount(_)) {
                checker.check_item(item).expect("inline alias must resolve");
            }
        }
        assert!(checker.context_mut().env.is_bound_value("i32"), "{mount}");
        assert_eq!(
            checker.ctx_env_lookup("i32").expect("alias").ty,
            Type::bool()
        );
    }
}

#[test]
fn mounted_variant_values_do_not_acquire_primitive_type_properties() {
    let mut registry = ModuleRegistry::new();
    register_module(
        &mut registry,
        "core.variant_roles",
        "public type Choice is | Build(Int) | Empty;",
        0,
    );
    for (variant, value) in [("Build", "i32(7)"), ("Empty", "i32")] {
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        mount(
            &mut checker,
            &registry,
            &format!("mount core.variant_roles.{{Choice, {variant} as i32}};"),
        );
        let valid = parse(&format!("fn valid() -> Choice {{ {value} }}"), 10);
        checker
            .check_item(&valid.items[0])
            .expect("the mounted variant remains constructible");
        let invalid = parse("fn invalid() -> Int { i32.min }", 11);
        let result = checker.check_item(&invalid.items[0]);
        assert!(
            result.is_err(),
            "{variant} is a value, not the primitive i32 type"
        );
        assert!(checker.context_mut().env.is_bound_value("i32"));
    }
}
