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
// Tests for module-aware type checking
//
// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
//
// These tests verify that the type system correctly:
// - Tracks current module context
// - Resolves qualified type names (Module.Type)
// - Maintains backward compatibility with unqualified names

use indexmap::IndexMap;
use verum_common::{Maybe, Text};
use verum_types::{ModuleId, Type, TypeChecker, TypeContext};

#[test]
fn test_module_context_tracking() {
    let mut ctx = TypeContext::new();

    // Initially no module context
    assert_eq!(ctx.current_module(), Maybe::None);

    // Set module context
    let module_id = ModuleId::new(1);
    ctx.set_current_module(module_id);

    assert_eq!(ctx.current_module(), Maybe::Some(module_id));
}

#[test]
fn test_qualified_type_definition() {
    let mut ctx = TypeContext::new();

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Define types in different modules
    let mut user_a_fields = IndexMap::new();
    user_a_fields.insert(Text::from("id"), Type::int());
    user_a_fields.insert(Text::from("name"), Type::text());

    let mut user_b_fields = IndexMap::new();
    user_b_fields.insert(Text::from("username"), Type::text());
    user_b_fields.insert(Text::from("email"), Type::text());

    ctx.define_module_type(module_a, Text::from("User"), Type::Record(user_a_fields));
    ctx.define_module_type(module_b, Text::from("User"), Type::Record(user_b_fields));

    // Lookup types in specific modules
    let user_a = ctx.lookup_module_type(module_a, "User");
    let user_b = ctx.lookup_module_type(module_b, "User");

    assert!(user_a.is_some());
    assert!(user_b.is_some());

    // Verify they are different types
    if let (Maybe::Some(a), Maybe::Some(b)) = (user_a, user_b) {
        assert_ne!(a, b);
    }
}

#[test]
fn test_type_checker_module_context() {
    let mut checker = TypeChecker::new();

    // Initially no module
    assert_eq!(checker.current_module(), Maybe::None);

    // Set module
    let module_id = ModuleId::new(42);
    checker.set_current_module(module_id);

    assert_eq!(checker.current_module(), Maybe::Some(module_id));

    // Define type in module
    let mut config_fields = IndexMap::new();
    config_fields.insert(Text::from("debug"), Type::bool());
    config_fields.insert(Text::from("port"), Type::int());
    checker.define_module_type(module_id, Text::from("Config"), Type::Record(config_fields));

    // Look it up
    let config_type = checker.lookup_module_type(module_id, "Config");
    assert!(config_type.is_some());
}

#[test]
fn test_backward_compatibility_unqualified_types() {
    let mut ctx = TypeContext::new();

    // Define type the old way (unqualified)
    let mut point_fields = IndexMap::new();
    point_fields.insert(Text::from("x"), Type::float());
    point_fields.insert(Text::from("y"), Type::float());
    ctx.define_type(Text::from("Point"), Type::Record(point_fields));

    // Should still be able to lookup
    let point_type = ctx.lookup_type("Point");
    assert!(point_type.is_some());
}

#[test]
fn test_current_module_auto_registers_unqualified() {
    let mut ctx = TypeContext::new();

    let module_id = ModuleId::new(1);
    ctx.set_current_module(module_id);

    // Define type in current module using qualified API
    ctx.define_module_type(module_id, "Data".to_string(), Type::text());

    // Should also be available via unqualified lookup (since it's current module)
    let data_type = ctx.lookup_type("Data");
    assert!(data_type.is_some());
}

#[test]
fn test_multiple_modules_isolation() {
    let mut ctx = TypeContext::new();

    let module_a = ModuleId::new(1);
    let module_b = ModuleId::new(2);

    // Define same-named type in both modules
    ctx.define_module_type(module_a, "Result".to_string(), Type::bool());
    ctx.define_module_type(module_b, "Result".to_string(), Type::int());

    // Each module should have its own version
    let result_a = ctx.lookup_module_type(module_a, "Result");
    let result_b = ctx.lookup_module_type(module_b, "Result");

    assert!(result_a.is_some());
    assert!(result_b.is_some());

    // Verify they're different
    if let (Maybe::Some(a), Maybe::Some(b)) = (result_a, result_b) {
        // Type::bool() gives Type::Bool, Type::int() gives Type::Int
        assert_ne!(a, b, "Module A and B should have different Result types");
    }
}

#[test]
fn test_qualified_type_lookup_placeholder() {
    let mut ctx = TypeContext::new();

    // Define a type normally
    ctx.define_type("MyType".to_string(), Type::text());

    // The placeholder should fall back to unqualified lookup
    let result = ctx.lookup_qualified_type("some.module.MyType");
    assert!(result.is_some());

    // Simple name should work
    let result2 = ctx.lookup_qualified_type("MyType");
    assert!(result2.is_some());
}

#[test]
fn test_module_context_inheritance_in_scopes() {
    let mut ctx = TypeContext::new();

    let module_id = ModuleId::new(5);
    ctx.set_current_module(module_id);

    // Enter a new scope
    ctx.enter_scope();

    // Module context should be inherited
    assert_eq!(ctx.current_module(), Maybe::Some(module_id));

    // Exit scope
    ctx.exit_scope();

    // Module context should still be there
    assert_eq!(ctx.current_module(), Maybe::Some(module_id));
}
