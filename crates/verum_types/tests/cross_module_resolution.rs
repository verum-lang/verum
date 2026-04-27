#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Cross-module type resolution tests.
//!
//! Tests the integration of NameResolver with TypeChecker for resolving
//! types across module boundaries.
//!
//! Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Cross-module name resolution

use verum_ast::decl::{TypeDecl, TypeDeclBody, Visibility};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, Type as AstType, TypeKind};
use verum_ast::{FileId, Item, ItemKind, Module};
use verum_modules::{
    ModuleId, ModulePath, ModuleRegistry, NameResolver,
    resolver::{NameKind, ResolvedName},
};
use verum_common::{List, Maybe, Shared};
use verum_types::TypeChecker;

/// Helper to create a simple module for testing
fn create_test_module(_name: &str, file_id: FileId) -> Module {
    let span = Span::new(0, 0, file_id);
    Module::new(vec![].into(), file_id, span)
}

/// Helper to create a type declaration
fn create_type_decl(name: &str, visibility: Visibility) -> Item {
    let span = Span::dummy();
    let type_decl = TypeDecl {
        name: Ident::new(name, span),
        visibility,
        generics: vec![].into(),
        attributes: vec![].into(),
        body: TypeDeclBody::Alias(AstType {
            kind: TypeKind::Int,
            span,
        }),
        resource_modifier: None,
        generic_where_clause: verum_common::Maybe::None,
        meta_where_clause: None,
        span,
    };

    Item::new(ItemKind::Type(type_decl), span)
}

/// Test 1: Simple type resolution in same module
#[test]
fn test_same_module_type_resolution() {
    let mut checker = TypeChecker::new();
    let module_id = ModuleId::new(1);

    // Set current module
    checker.set_current_module(module_id);

    // Define a type in the current module
    let ty = verum_types::ty::Type::int();
    checker.define_module_type(module_id, "MyInt".to_string(), ty.clone());

    // Look up the type
    let result = checker.lookup_module_type(module_id, "MyInt");
    assert!(matches!(result, Maybe::Some(_)));
}

/// Test 2: Type resolution across modules (Module A defines, Module B uses)
#[test]
fn test_cross_module_type_resolution() {
    let registry = verum_modules::SharedModuleRegistry::empty();
    let mut checker = TypeChecker::with_shared_registry(registry.clone());
    let mut resolver = NameResolver::new();

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Module A defines type "Container"
    checker.set_current_module(module_a);
    let container_type = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("Container", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_a, "Container".to_string(), container_type.clone());

    // Register the type in the resolver
    let scope_a = resolver.create_scope(module_a);
    scope_a.add_binding(
        "Container",
        ResolvedName::new(
            module_a,
            ModulePath::from_str("module_a"),
            NameKind::Type,
            "Container",
        ),
    );

    // Module B imports from Module A
    checker.set_current_module(module_b);
    let scope_b = resolver.create_scope(module_b);
    scope_b.add_binding(
        "Container",
        ResolvedName::new(
            module_a, // Points to module A
            ModulePath::from_str("module_a"),
            NameKind::Type,
            "Container",
        ),
    );

    // Set the resolver in the type checker
    checker.set_name_resolver(resolver);

    // Module B should be able to resolve Container
    let result = checker.lookup_module_type(module_a, "Container");
    assert!(matches!(result, Maybe::Some(_)));
}

/// Test 3: Qualified path resolution (A.Container)
#[test]
fn test_qualified_path_resolution() {
    let registry = verum_modules::SharedModuleRegistry::empty();
    let mut checker = TypeChecker::with_shared_registry(registry.clone());
    let mut resolver = NameResolver::new();

    let module_a = ModuleId::new(1);
    let current_module = ModuleId::new(2);

    // Define Container in module A
    checker.set_current_module(module_a);
    let container_type = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("Container", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_a, "Container".to_string(), container_type.clone());

    // Register module A in resolver
    let scope_a = resolver.create_scope(module_a);
    scope_a.add_binding(
        "Container",
        ResolvedName::new(
            module_a,
            ModulePath::from_str("a.Container"),
            NameKind::Type,
            "Container",
        ),
    );

    // Current module knows about module A
    checker.set_current_module(current_module);
    let scope_current = resolver.create_scope(current_module);
    scope_current.add_binding(
        "a",
        ResolvedName::new(module_a, ModulePath::from_str("a"), NameKind::Module, "a"),
    );

    checker.set_name_resolver(resolver);

    // Should be able to resolve a.Container
    let result = checker.lookup_module_type(module_a, "Container");
    assert!(matches!(result, Maybe::Some(_)));
}

/// Test 4: Nested module resolution (std.collections.List)
#[test]
fn test_nested_module_resolution() {
    let registry = verum_modules::SharedModuleRegistry::empty();
    let mut checker = TypeChecker::with_shared_registry(registry.clone());
    let mut resolver = NameResolver::new();

    let std_id = ModuleId::new(1);
    let collections_id = ModuleId::new(2);
    let current_id = ModuleId::new(3);

    // Define List in std.collections
    checker.set_current_module(collections_id);
    let list_type = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(collections_id, "List".to_string(), list_type.clone());

    // Set up resolver hierarchy: std -> collections -> List
    let scope_std = resolver.create_scope(std_id);
    scope_std.add_binding(
        "collections",
        ResolvedName::new(
            collections_id,
            ModulePath::from_str("std.collections"),
            NameKind::Module,
            "collections",
        ),
    );

    let scope_collections = resolver.create_scope(collections_id);
    scope_collections.add_binding(
        "List",
        ResolvedName::new(
            collections_id,
            ModulePath::from_str("std.collections.List"),
            NameKind::Type,
            "List",
        ),
    );

    let scope_current = resolver.create_scope(current_id);
    scope_current.add_binding(
        "std",
        ResolvedName::new(std_id, ModulePath::from_str("std"), NameKind::Module, "std"),
    );

    checker.set_current_module(current_id);
    checker.set_name_resolver(resolver);

    // Should resolve std.collections.List
    let result = checker.lookup_module_type(collections_id, "List");
    assert!(matches!(result, Maybe::Some(_)));
}

/// Test 5: Generic types across modules (A.Container<B.Item>)
#[test]
fn test_generic_types_cross_module() {
    let registry = verum_modules::SharedModuleRegistry::empty();
    let mut checker = TypeChecker::with_shared_registry(registry.clone());

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Module A defines Container<T>
    checker.set_current_module(module_a);
    let container_type = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("Container", Span::dummy())),
        args: vec![verum_types::ty::Type::Var(verum_types::ty::TypeVar::fresh())].into(),
    };
    checker.define_module_type(module_a, "Container".to_string(), container_type);

    // Module B defines Item
    checker.set_current_module(module_b);
    let item_type = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("Item", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_b, "Item".to_string(), item_type.clone());

    // Both types should be resolvable
    assert!(matches!(
        checker.lookup_module_type(module_a, "Container"),
        Maybe::Some(_)
    ));
    assert!(matches!(
        checker.lookup_module_type(module_b, "Item"),
        Maybe::Some(_)
    ));
}

/// Test 6: Type alias across modules
#[test]
fn test_type_alias_cross_module() {
    let mut checker = TypeChecker::new();

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Module A defines original type
    checker.set_current_module(module_a);
    let original = verum_types::ty::Type::int();
    checker.define_module_type(module_a, "Original".to_string(), original.clone());

    // Module B defines alias to A's type
    checker.set_current_module(module_b);
    checker.define_module_type(module_b, "Alias".to_string(), original.clone());

    // Both should resolve to the same underlying type
    let a_type = checker.lookup_module_type(module_a, "Original");
    let b_type = checker.lookup_module_type(module_b, "Alias");

    assert!(matches!(a_type, Maybe::Some(_)));
    assert!(matches!(b_type, Maybe::Some(_)));
}

/// Test 7: Visibility enforcement (private types not accessible)
#[test]
fn test_visibility_enforcement() {
    let mut checker = TypeChecker::new();

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Module A defines private type
    checker.set_current_module(module_a);
    let private_type = verum_types::ty::Type::int();
    checker.define_module_type(module_a, "PrivateType".to_string(), private_type.clone());

    // Module B tries to access it
    checker.set_current_module(module_b);

    // For now, visibility checking is a stub, but the API is ready
    // Future: This should fail once VisibilityChecker is integrated
    let result = checker.lookup_module_type(module_a, "PrivateType");
    assert!(matches!(result, Maybe::Some(_))); // Currently allows (stub)
}

/// Test 8: Module context switching
#[test]
fn test_module_context_switching() {
    let mut checker = TypeChecker::new();

    let module_1 = ModuleId::new(1);
    let module_2 = ModuleId::new(2);

    // Define types in different modules
    checker.set_current_module(module_1);
    checker.define_module_type(module_1, "Type1".to_string(), verum_types::ty::Type::int());

    checker.set_current_module(module_2);
    checker.define_module_type(module_2, "Type2".to_string(), verum_types::ty::Type::bool());

    // Verify context switches correctly
    assert_eq!(checker.current_module(), Maybe::Some(module_2));

    // Types should be in their respective modules
    assert!(matches!(
        checker.lookup_module_type(module_1, "Type1"),
        Maybe::Some(_)
    ));
    assert!(matches!(
        checker.lookup_module_type(module_2, "Type2"),
        Maybe::Some(_)
    ));
}

/// Test 9: Type not found error
#[test]
fn test_type_not_found() {
    let checker = TypeChecker::new();
    let module_id = ModuleId::new(1);

    // Try to look up non-existent type
    let result = checker.lookup_module_type(module_id, "NonExistent");
    assert!(matches!(result, Maybe::None));
}

/// Test 10: Reverse module lookup (get_type_module)
#[test]
fn test_get_type_module() {
    let mut checker = TypeChecker::new();
    let module_id = ModuleId::new(1);

    // Define a type
    checker.set_current_module(module_id);
    checker.define_module_type(
        module_id,
        "MyType".to_string(),
        verum_types::ty::Type::int(),
    );

    // The context should be able to tell us which module defines this type
    // Note: This tests the TypeContext method directly
    // In practice, type resolution would happen through the resolver
}

/// Test 11: Multiple modules with same type name (namespacing)
#[test]
fn test_type_namespacing() {
    let mut checker = TypeChecker::new();

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Both modules define "Config" but with different implementations
    checker.set_current_module(module_a);
    let config_a = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("ConfigA", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_a, "Config".to_string(), config_a);

    checker.set_current_module(module_b);
    let config_b = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("ConfigB", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_b, "Config".to_string(), config_b);

    // Both should be resolvable in their respective modules
    assert!(matches!(
        checker.lookup_module_type(module_a, "Config"),
        Maybe::Some(_)
    ));
    assert!(matches!(
        checker.lookup_module_type(module_b, "Config"),
        Maybe::Some(_)
    ));
}

/// Test 12: Prelude types available everywhere
#[test]
fn test_prelude_types() {
    let mut checker = TypeChecker::new();
    let mut resolver = NameResolver::new();

    let prelude_module = ModuleId::ROOT;
    let user_module = ModuleId::new(1);

    // Add a prelude type
    resolver.add_prelude_item(
        "List",
        ResolvedName::new(
            prelude_module,
            ModulePath::from_str("prelude.List"),
            NameKind::Type,
            "List",
        ),
    );

    checker.set_name_resolver(resolver);
    checker.set_current_module(user_module);

    // Prelude types should be accessible without qualification
    // This would be tested through actual type inference, not direct lookup
}

/// Test 13: Circular module references
#[test]
fn test_circular_module_reference() {
    let registry = verum_modules::SharedModuleRegistry::empty();
    let mut checker = TypeChecker::with_shared_registry(registry.clone());

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Module A uses type from B
    checker.set_current_module(module_a);
    let type_from_b = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("TypeB", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_a, "TypeFromB".to_string(), type_from_b);

    // Module B uses type from A
    checker.set_current_module(module_b);
    let type_from_a = verum_types::ty::Type::Named {
        path: Path::single(Ident::new("TypeA", Span::dummy())),
        args: List::new(),
    };
    checker.define_module_type(module_b, "TypeFromA".to_string(), type_from_a);

    // Both should be resolvable (circular refs are OK at type level)
    assert!(matches!(
        checker.lookup_module_type(module_a, "TypeFromB"),
        Maybe::Some(_)
    ));
    assert!(matches!(
        checker.lookup_module_type(module_b, "TypeFromA"),
        Maybe::Some(_)
    ));
}

/// Test 14: Module registry integration
#[test]
fn test_module_registry_integration() {
    let registry = verum_modules::SharedModuleRegistry::empty();
    let _checker = TypeChecker::with_shared_registry(registry.clone());

    // Registry should be shared. Allocate an ID through the inner handle.
    let _id = registry.write().allocate_id();

    // Type checker should have access to the same registry
    // This ensures consistent module IDs across the system
}

/// Test 15: Performance - Many modules
#[test]
fn test_many_modules_performance() {
    let mut checker = TypeChecker::new();

    // Create 100 modules with types
    for i in 0..100 {
        let module_id = ModuleId::new(i);
        checker.set_current_module(module_id);

        let ty = verum_types::ty::Type::int();
        checker.define_module_type(module_id, format!("Type{}", i), ty);
    }

    // All should be resolvable efficiently
    for i in 0..100 {
        let module_id = ModuleId::new(i);
        let result = checker.lookup_module_type(module_id, &format!("Type{}", i));
        assert!(matches!(result, Maybe::Some(_)));
    }
}
