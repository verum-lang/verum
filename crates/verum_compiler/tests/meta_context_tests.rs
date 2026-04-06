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
// Tests for meta_context module
// Migrated from src/meta_context.rs per CLAUDE.md standards

use verum_ast::Span;
use verum_compiler::meta::*;
use verum_compiler::meta::value_ops::MetaValueOps;
use verum_common::Text;

fn default_span() -> Span {
    Span::default()
}

fn int_type() -> verum_ast::ty::Type {
    verum_ast::ty::Type::int(default_span())
}

fn text_type() -> verum_ast::ty::Type {
    verum_ast::ty::Type::text(default_span())
}

fn unit_type() -> verum_ast::ty::Type {
    verum_ast::ty::Type::unit(default_span())
}

#[test]
fn test_const_value_arithmetic() {
    let a = ConstValue::Int(10);
    let b = ConstValue::Int(5);

    assert_eq!(a.clone().add(b.clone()).unwrap(), ConstValue::Int(15));
    assert_eq!(a.clone().sub(b.clone()).unwrap(), ConstValue::Int(5));
    assert_eq!(a.clone().mul(b.clone()).unwrap(), ConstValue::Int(50));
    assert_eq!(a.clone().div(b.clone()).unwrap(), ConstValue::Int(2));
}

#[test]
fn test_const_value_comparison() {
    let a = ConstValue::Int(10);
    let b = ConstValue::Int(5);

    assert_eq!(a.clone().lt(b.clone()).unwrap(), ConstValue::Bool(false));
    assert_eq!(a.clone().gt(b.clone()).unwrap(), ConstValue::Bool(true));
}

#[test]
fn test_meta_context_bindings() {
    let mut ctx = MetaContext::new();
    ctx.bind(Text::from("x"), ConstValue::Int(42));

    assert!(ctx.has(&Text::from("x")));
    assert_eq!(ctx.get(&Text::from("x")), Some(ConstValue::Int(42)));

    ctx.unbind(&Text::from("x"));
    assert!(!ctx.has(&Text::from("x")));
}

// ============================================================================
// Reflection API Tests
// ============================================================================

#[test]
fn test_register_struct_and_get_fields() {
    let mut ctx = MetaContext::new();

    // Register a Point struct with x, y fields
    let fields = vec![(Text::from("x"), int_type()), (Text::from("y"), int_type())]
        .into_iter()
        .collect();

    ctx.register_struct(Text::from("Point"), fields);

    // Retrieve fields
    let retrieved = ctx.get_struct_fields(&Text::from("Point"));
    assert!(retrieved.is_some());

    let fields = retrieved.unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0, Text::from("x"));
    assert_eq!(fields[1].0, Text::from("y"));
}

#[test]
fn test_register_enum_and_get_variants() {
    let mut ctx = MetaContext::new();

    // Register a Color enum with Red, Green, Blue variants
    let variants = vec![
        (Text::from("Red"), unit_type()),
        (Text::from("Green"), unit_type()),
        (Text::from("Blue"), unit_type()),
    ]
    .into_iter()
    .collect();

    ctx.register_enum(Text::from("Color"), variants);

    // Retrieve variants
    let retrieved = ctx.get_enum_variants(&Text::from("Color"));
    assert!(retrieved.is_some());

    let variants = retrieved.unwrap();
    assert_eq!(variants.len(), 3);
    assert_eq!(variants[0].0, Text::from("Red"));
    assert_eq!(variants[1].0, Text::from("Green"));
    assert_eq!(variants[2].0, Text::from("Blue"));
}

#[test]
fn test_register_protocol_implementation() {
    let mut ctx = MetaContext::new();

    // Register that Point implements Debug
    let methods = vec![Text::from("fmt")].into_iter().collect();
    ctx.register_protocol_implementation(Text::from("Point"), Text::from("Debug"), methods);

    // Check protocol is registered
    let protocols = ctx.get_implemented_protocols(&Text::from("Point"));
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0], Text::from("Debug"));

    // Check implementors
    let implementors = ctx.get_implementors(&Text::from("Debug"));
    assert_eq!(implementors.len(), 1);
    assert_eq!(implementors[0], Text::from("Point"));
}

#[test]
fn test_multiple_protocol_implementations() {
    let mut ctx = MetaContext::new();

    // Point implements Debug
    ctx.register_protocol_implementation(
        Text::from("Point"),
        Text::from("Debug"),
        vec![Text::from("fmt")].into_iter().collect(),
    );

    // Point implements Clone
    ctx.register_protocol_implementation(
        Text::from("Point"),
        Text::from("Clone"),
        vec![Text::from("clone")].into_iter().collect(),
    );

    // Point implements PartialEq
    ctx.register_protocol_implementation(
        Text::from("Point"),
        Text::from("PartialEq"),
        vec![Text::from("eq"), Text::from("ne")]
            .into_iter()
            .collect(),
    );

    // Check all protocols
    let protocols = ctx.get_implemented_protocols(&Text::from("Point"));
    assert_eq!(protocols.len(), 3);
    assert!(protocols.iter().any(|p| p == &Text::from("Debug")));
    assert!(protocols.iter().any(|p| p == &Text::from("Clone")));
    assert!(protocols.iter().any(|p| p == &Text::from("PartialEq")));
}

#[test]
fn test_get_type_definition() {
    let mut ctx = MetaContext::new();

    // Register struct
    ctx.register_struct(
        Text::from("User"),
        vec![
            (Text::from("id"), int_type()),
            (Text::from("name"), text_type()),
        ]
        .into_iter()
        .collect(),
    );

    // Get type definition
    let typedef = ctx.get_type_definition(&Text::from("User"));
    assert!(typedef.is_some());

    match typedef.unwrap() {
        TypeDefinition::Struct { name, fields, .. } => {
            assert_eq!(*name, Text::from("User"));
            assert_eq!(fields.len(), 2);
        }
        _ => panic!("Expected Struct definition"),
    }
}

#[test]
fn test_field_info_creation() {
    let field = FieldInfo::new(Text::from("x"), int_type(), 0)
        .with_visibility(Visibility::Public)
        .with_doc(Text::from("X coordinate"))
        .with_attribute(Text::from("serialize"));

    assert_eq!(field.name, Text::from("x"));
    assert_eq!(field.index, 0);
    assert!(field.is_public());
    assert!(field.doc.is_some());
    assert_eq!(field.attributes.len(), 1);
}

#[test]
fn test_variant_info_creation() {
    // Unit variant
    let unit_variant = VariantInfo::unit(Text::from("None"), 0);
    assert_eq!(unit_variant.name, Text::from("None"));
    assert_eq!(unit_variant.index, 0);
    assert!(matches!(unit_variant.kind, VariantKind::Unit));

    // Tuple variant
    let tuple_fields: verum_common::List<FieldInfo> = vec![
        FieldInfo::new(Text::from("0"), int_type(), 0),
    ]
    .into_iter()
    .collect();
    let tuple_variant = VariantInfo::tuple(Text::from("Some"), tuple_fields, 1);
    assert_eq!(tuple_variant.name, Text::from("Some"));
    assert!(matches!(tuple_variant.kind, VariantKind::Tuple));

    // Struct variant
    let struct_fields: verum_common::List<FieldInfo> = vec![
        FieldInfo::new(Text::from("x"), int_type(), 0),
        FieldInfo::new(Text::from("y"), int_type(), 1),
    ]
    .into_iter()
    .collect();
    let struct_variant = VariantInfo::record(Text::from("Point"), struct_fields, 2);
    assert_eq!(struct_variant.name, Text::from("Point"));
    assert!(matches!(struct_variant.kind, VariantKind::Struct));
}

#[test]
fn test_function_info_creation() {
    let method = FunctionInfo::new(Text::from("fmt"), Text::from("()"))
        .async_fn();

    assert_eq!(method.name, Text::from("fmt"));
    assert!(method.is_async);
    // No self param by default, so not a method
    assert!(!method.is_method());
}

#[test]
fn test_field_info_to_const_value() {
    let field = FieldInfo::new(Text::from("x"), int_type(), 0);

    let const_value = field.to_const_value();

    // Should be a tuple with (name, index, type_name, type_kind, is_public)
    if let ConstValue::Tuple(items) = const_value {
        assert_eq!(items.len(), 5);
        assert!(matches!(&items[0], ConstValue::Text(t) if t == &Text::from("x")));
        assert!(matches!(&items[1], ConstValue::Int(0)));
        assert!(matches!(&items[4], ConstValue::Bool(true)));
    } else {
        panic!("Expected Tuple ConstValue");
    }
}

#[test]
fn test_variant_info_to_const_value() {
    let variant = VariantInfo::unit(Text::from("None"), 0);
    let const_value = variant.to_const_value();

    // Should be a tuple with (name, index, kind, fields_count)
    if let ConstValue::Tuple(items) = const_value {
        assert_eq!(items.len(), 4);
        assert!(matches!(&items[0], ConstValue::Text(t) if t == &Text::from("None")));
        assert!(matches!(&items[1], ConstValue::Int(0))); // index
        assert!(matches!(&items[2], ConstValue::Int(0))); // kind = Unit = 0
        assert!(matches!(&items[3], ConstValue::Int(0))); // 0 fields
    } else {
        panic!("Expected Tuple ConstValue");
    }
}

#[test]
fn test_nonexistent_type_returns_none() {
    let ctx = MetaContext::new();

    // Try to get fields of non-existent type
    assert!(ctx.get_struct_fields(&Text::from("NonExistent")).is_none());
    assert!(ctx.get_enum_variants(&Text::from("NonExistent")).is_none());
    assert!(
        ctx.get_type_definition(&Text::from("NonExistent"))
            .is_none()
    );
}

#[test]
fn test_struct_returns_none_for_variants() {
    let mut ctx = MetaContext::new();

    ctx.register_struct(
        Text::from("Point"),
        vec![(Text::from("x"), int_type())].into_iter().collect(),
    );

    // Struct should return None for variants_of
    assert!(ctx.get_enum_variants(&Text::from("Point")).is_none());
    // But should return Some for fields_of
    assert!(ctx.get_struct_fields(&Text::from("Point")).is_some());
}

#[test]
fn test_enum_returns_none_for_fields() {
    let mut ctx = MetaContext::new();

    ctx.register_enum(
        Text::from("Color"),
        vec![(Text::from("Red"), unit_type())].into_iter().collect(),
    );

    // Enum should return None for fields_of
    assert!(ctx.get_struct_fields(&Text::from("Color")).is_none());
    // But should return Some for variants_of
    assert!(ctx.get_enum_variants(&Text::from("Color")).is_some());
}

// ============================================================================
// Type Property Tests
// ============================================================================

#[test]
fn test_type_property_size() {
    let ctx = MetaContext::new();
    let ty = int_type();

    let result = ctx.eval_type_property(&ty, verum_ast::expr::TypeProperty::Size);
    assert!(result.is_ok());
    // Int is 64-bit = 8 bytes
    assert_eq!(result.unwrap(), ConstValue::Int(8));
}

#[test]
fn test_type_property_alignment() {
    let ctx = MetaContext::new();
    let ty = int_type();

    let result = ctx.eval_type_property(&ty, verum_ast::expr::TypeProperty::Alignment);
    assert!(result.is_ok());
    // Int alignment is 8 bytes
    assert_eq!(result.unwrap(), ConstValue::Int(8));
}

#[test]
fn test_type_property_bits() {
    let ctx = MetaContext::new();
    let ty = int_type();

    let result = ctx.eval_type_property(&ty, verum_ast::expr::TypeProperty::Bits);
    assert!(result.is_ok());
    // Int is 64 bits
    assert_eq!(result.unwrap(), ConstValue::Int(64));
}

#[test]
fn test_type_property_name() {
    let ctx = MetaContext::new();
    let ty = int_type();

    let result = ctx.eval_type_property(&ty, verum_ast::expr::TypeProperty::Name);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Text(Text::from("Int")));
}

#[test]
fn test_type_property_id() {
    let ctx = MetaContext::new();
    let ty = int_type();

    let result = ctx.eval_type_property(&ty, verum_ast::expr::TypeProperty::Id);
    assert!(result.is_ok());

    // T.id should return a u64 hash, verify it's a non-zero integer
    if let ConstValue::UInt(id) = result.unwrap() {
        assert!(id != 0, "Type ID should be non-zero");
    } else {
        panic!("Expected ConstValue::UInt for type ID");
    }
}

#[test]
fn test_type_property_id_consistency() {
    let ctx = MetaContext::new();

    // Same type should always produce same ID
    let ty1 = int_type();
    let ty2 = int_type();

    let id1 = ctx.eval_type_property(&ty1, verum_ast::expr::TypeProperty::Id).unwrap();
    let id2 = ctx.eval_type_property(&ty2, verum_ast::expr::TypeProperty::Id).unwrap();

    assert_eq!(id1, id2, "Same type should produce same ID");
}

#[test]
fn test_type_property_id_uniqueness() {
    let ctx = MetaContext::new();

    // Different types should produce different IDs
    let int_ty = int_type();
    let text_ty = text_type();

    let int_id = ctx.eval_type_property(&int_ty, verum_ast::expr::TypeProperty::Id).unwrap();
    let text_id = ctx.eval_type_property(&text_ty, verum_ast::expr::TypeProperty::Id).unwrap();

    assert_ne!(int_id, text_id, "Different types should produce different IDs");
}
