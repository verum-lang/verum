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
// Tests for automatic accessor function generation for record types
// Cross-field refinements on structs: "type T is { f1: A, f2: B } where constraint(f1, f2)" — .2.1 lines 1839-1857

use indexmap::IndexMap;
use verum_common::Text;
use verum_types::context::TypeContext;
use verum_types::ty::Type;

#[test]
fn test_accessor_generation_simple_record() {
    // Create a simple User record type
    let mut fields = IndexMap::new();
    fields.insert(Text::from("name"), Type::text());
    fields.insert(Text::from("age"), Type::int());
    fields.insert(Text::from("email"), Type::text());

    let user_type = Type::Record(fields);

    // Create context and register type with accessors
    let mut ctx = TypeContext::new();
    ctx.define_type_with_accessors("User".to_string(), user_type)
        .unwrap();

    // Verify accessor functions were generated
    assert!(ctx.env.lookup("User.name").is_some());
    assert!(ctx.env.lookup("User.age").is_some());
    assert!(ctx.env.lookup("User.email").is_some());

    // Verify accessor function types
    let name_accessor = ctx.env.lookup("User.name").unwrap();
    let name_ty = name_accessor.instantiate();

    // Should be: fn(User) -> Text
    match name_ty {
        Type::Function {
            params,
            return_type,
            ..
        } => {
            assert_eq!(params.len(), 1);
            assert!(matches!(params[0], Type::Named { .. }));
            assert!(matches!(*return_type, Type::Text));
        }
        _ => panic!("Expected function type for accessor"),
    }
}

#[test]
fn test_accessor_generation_nested_records() {
    // Create Address record
    let mut address_fields = IndexMap::new();
    address_fields.insert(Text::from("street"), Type::text());
    address_fields.insert(Text::from("city"), Type::text());
    address_fields.insert(Text::from("zip"), Type::text());
    let address_type = Type::Record(address_fields);

    // Create Person record with nested Address
    let mut person_fields = IndexMap::new();
    person_fields.insert(Text::from("name"), Type::text());
    person_fields.insert(Text::from("address"), address_type.clone());
    let person_type = Type::Record(person_fields);

    // Register both types
    let mut ctx = TypeContext::new();
    ctx.define_type_with_accessors("Address".to_string(), address_type)
        .unwrap();
    ctx.define_type_with_accessors("Person".to_string(), person_type)
        .unwrap();

    // Verify accessors for Address
    assert!(ctx.env.lookup("Address.street").is_some());
    assert!(ctx.env.lookup("Address.city").is_some());
    assert!(ctx.env.lookup("Address.zip").is_some());

    // Verify accessors for Person
    assert!(ctx.env.lookup("Person.name").is_some());
    assert!(ctx.env.lookup("Person.address").is_some());
}

#[test]
fn test_accessor_generation_empty_record() {
    let empty_record = Type::Record(IndexMap::new());

    let mut ctx = TypeContext::new();
    ctx.define_type_with_accessors("Empty".to_string(), empty_record)
        .unwrap();

    // Should not generate any accessors for empty record
    assert!(ctx.env.lookup("Empty.anything").is_none());
}

#[test]
fn test_accessor_generation_non_record_types() {
    let mut ctx = TypeContext::new();

    // Primitive types shouldn't generate accessors
    ctx.define_type_with_accessors("MyInt".to_string(), Type::int())
        .unwrap();
    assert!(ctx.env.lookup("MyInt.anything").is_none());

    // Tuple types shouldn't generate accessors (they use numeric indexing)
    ctx.define_type_with_accessors(
        "Pair".to_string(),
        Type::tuple(vec![Type::int(), Type::text()].into()),
    )
    .unwrap();
    assert!(ctx.env.lookup("Pair.0").is_none());
}

#[test]
fn test_accessor_usage_in_refinements() {
    // This test demonstrates how accessors are used in inline refinement syntax
    // type ValidUser is User{age(it) >= 18 && email(it).contains("@")}

    let mut fields = IndexMap::new();
    fields.insert(Text::from("name"), Type::text());
    fields.insert(Text::from("age"), Type::int());
    fields.insert(Text::from("email"), Type::text());
    let user_type = Type::Record(fields);

    let mut ctx = TypeContext::new();
    ctx.define_type_with_accessors("User".to_string(), user_type)
        .unwrap();

    // Verify accessors are available for use in refinement predicates
    assert!(ctx.env.lookup("User.age").is_some());
    assert!(ctx.env.lookup("User.email").is_some());

    // The actual refinement predicate would be parsed and type-checked
    // using these accessor functions during refinement validation
}
