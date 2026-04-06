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
// Comprehensive tests for field access on imported record types
//
// Record types: "type T is { field: Type, ... }" with named fields, structural matching
// Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — .1 - Cross-Module Type Resolution
//
// This test suite validates:
// 1. Simple field access on imported record types
// 2. Nested field access on imported records
// 3. Field access with imported types in record definitions
// 4. Method calls on imported record types
// 5. Error cases for missing or unknown fields
// 6. Field access with multi-segment module paths

use indexmap::IndexMap;
use verum_ast::{
    expr::*,
    literal::Literal,
    pattern::{Pattern, PatternKind},
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::{Heap, List, Text};
use verum_types::context::TypeScheme;
use verum_types::infer::*;
use verum_types::ty::Type;

// ============================================================================
// Test 1: Simple Field Access on Imported Records
// ============================================================================

#[test]
fn test_field_access_imported_record_simple() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define User = { id: Int, name: Text } as if imported from std
    let mut user_fields = IndexMap::new();
    user_fields.insert(Text::from("id"), Type::int());
    user_fields.insert(Text::from("name"), Type::text());
    let user_type = Type::Record(user_fields);

    // Add user variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("user"), TypeScheme::mono(user_type.clone()));

    // Expression: user.id
    let user_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("user".to_string(), span))),
        span,
    );
    let field_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(user_expr),
            field: Ident::new("id".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&field_expr);
    assert!(
        result.is_ok(),
        "Field access on imported record should work"
    );
    assert_eq!(result.unwrap().ty, Type::int(), "user.id should be Int");
}

#[test]
fn test_field_access_imported_record_text_field() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define User = { id: Int, name: Text }
    let mut user_fields = IndexMap::new();
    user_fields.insert(Text::from("id"), Type::int());
    user_fields.insert(Text::from("name"), Type::text());
    let user_type = Type::Record(user_fields);

    // Add user variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("user"), TypeScheme::mono(user_type.clone()));

    // Expression: user.name
    let user_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("user".to_string(), span))),
        span,
    );
    let field_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(user_expr),
            field: Ident::new("name".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&field_expr);
    assert!(
        result.is_ok(),
        "Field access on imported record should work"
    );
    assert_eq!(result.unwrap().ty, Type::text(), "user.name should be Text");
}

#[test]
fn test_field_access_imported_record_in_expression() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Point = { x: Int, y: Int }
    let mut point_fields = IndexMap::new();
    point_fields.insert(Text::from("x"), Type::int());
    point_fields.insert(Text::from("y"), Type::int());
    let point_type = Type::Record(point_fields);

    // Add point variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("p"), TypeScheme::mono(point_type.clone()));

    // Expression: p.x + p.y
    let p_x = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("p".to_string(), span))),
                span,
            )),
            field: Ident::new("x".to_string(), span),
        },
        span,
    );

    let p_y = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("p".to_string(), span))),
                span,
            )),
            field: Ident::new("y".to_string(), span),
        },
        span,
    );

    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(p_x),
            right: Box::new(p_y),
        },
        span,
    );

    let result = checker.synth_expr(&add_expr);
    assert!(
        result.is_ok(),
        "Expression with imported record fields should work"
    );
    assert_eq!(result.unwrap().ty, Type::int(), "p.x + p.y should be Int");
}

// ============================================================================
// Test 2: Nested Field Access on Imported Records
// ============================================================================

#[test]
fn test_field_access_nested_imported_record() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Address = { street: Text, city: Text }
    let mut address_fields = IndexMap::new();
    address_fields.insert(Text::from("street"), Type::text());
    address_fields.insert(Text::from("city"), Type::text());
    let address_type = Type::Record(address_fields);

    // Define Person = { name: Text, address: Address }
    let mut person_fields = IndexMap::new();
    person_fields.insert(Text::from("name"), Type::text());
    person_fields.insert(Text::from("address"), address_type.clone());
    let person_type = Type::Record(person_fields);

    // Add person variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("person"), TypeScheme::mono(person_type.clone()));

    // Expression: person.address.city
    let person_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("person".to_string(), span))),
        span,
    );

    let address_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(person_expr),
            field: Ident::new("address".to_string(), span),
        },
        span,
    );

    let city_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(address_expr),
            field: Ident::new("city".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&city_expr);
    assert!(
        result.is_ok(),
        "Nested field access on imported records should work"
    );
    assert_eq!(
        result.unwrap().ty,
        Type::text(),
        "person.address.city should be Text"
    );
}

#[test]
fn test_field_access_deeply_nested_imported_records() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Coord = { lat: Int, lon: Int }
    let mut coord_fields = IndexMap::new();
    coord_fields.insert(Text::from("lat"), Type::int());
    coord_fields.insert(Text::from("lon"), Type::int());
    let coord_type = Type::Record(coord_fields);

    // Define Location = { name: Text, coord: Coord }
    let mut location_fields = IndexMap::new();
    location_fields.insert(Text::from("name"), Type::text());
    location_fields.insert(Text::from("coord"), coord_type.clone());
    let location_type = Type::Record(location_fields);

    // Define Event = { title: Text, location: Location }
    let mut event_fields = IndexMap::new();
    event_fields.insert(Text::from("title"), Type::text());
    event_fields.insert(Text::from("location"), location_type.clone());
    let event_type = Type::Record(event_fields);

    // Add event variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("event"), TypeScheme::mono(event_type.clone()));

    // Expression: event.location.coord.lat
    let event_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("event".to_string(), span))),
        span,
    );

    let location_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(event_expr),
            field: Ident::new("location".to_string(), span),
        },
        span,
    );

    let coord_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(location_expr),
            field: Ident::new("coord".to_string(), span),
        },
        span,
    );

    let lat_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(coord_expr),
            field: Ident::new("lat".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&lat_expr);
    assert!(
        result.is_ok(),
        "Deeply nested field access on imported records should work"
    );
    assert_eq!(
        result.unwrap().ty,
        Type::int(),
        "event.location.coord.lat should be Int"
    );
}

#[test]
fn test_field_access_nested_with_operations() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Vector = { x: Int, y: Int }
    let mut vector_fields = IndexMap::new();
    vector_fields.insert(Text::from("x"), Type::int());
    vector_fields.insert(Text::from("y"), Type::int());
    let vector_type = Type::Record(vector_fields);

    // Define Line = { start: Vector, end: Vector }
    let mut line_fields = IndexMap::new();
    line_fields.insert(Text::from("start"), vector_type.clone());
    line_fields.insert(Text::from("end"), vector_type.clone());
    let line_type = Type::Record(line_fields);

    // Add line variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("line"), TypeScheme::mono(line_type.clone()));

    // Expression: line.start.x + line.end.x
    let line_start_x = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Field {
                    expr: Box::new(Expr::new(
                        ExprKind::Path(Path::single(Ident::new("line".to_string(), span))),
                        span,
                    )),
                    field: Ident::new("start".to_string(), span),
                },
                span,
            )),
            field: Ident::new("x".to_string(), span),
        },
        span,
    );

    let line_end_x = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Field {
                    expr: Box::new(Expr::new(
                        ExprKind::Path(Path::single(Ident::new("line".to_string(), span))),
                        span,
                    )),
                    field: Ident::new("end".to_string(), span),
                },
                span,
            )),
            field: Ident::new("x".to_string(), span),
        },
        span,
    );

    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(line_start_x),
            right: Box::new(line_end_x),
        },
        span,
    );

    let result = checker.synth_expr(&add_expr);
    assert!(
        result.is_ok(),
        "Nested field access in operations should work"
    );
    assert_eq!(
        result.unwrap().ty,
        Type::int(),
        "line.start.x + line.end.x should be Int"
    );
}

// ============================================================================
// Test 3: Field Access with Cross-Module Types
// ============================================================================

#[test]
fn test_field_access_record_with_imported_field_types() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Simulate imported types from different modules
    // Define UserId = Int (from user module)
    // Define Email = Text (from email module)

    // Define User = { id: UserId, email: Email }
    let mut user_fields = IndexMap::new();
    user_fields.insert(Text::from("id"), Type::int()); // UserId is alias for Int
    user_fields.insert(Text::from("email"), Type::text()); // Email is alias for Text
    let user_type = Type::Record(user_fields);

    // Add user variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("user"), TypeScheme::mono(user_type.clone()));

    // Expression: user.id
    let user_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("user".to_string(), span))),
        span,
    );
    let id_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(user_expr),
            field: Ident::new("id".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&id_expr);
    assert!(
        result.is_ok(),
        "Field access with imported field types should work"
    );
    assert_eq!(result.unwrap().ty, Type::int());
}

#[test]
fn test_field_access_mixed_imported_and_builtin_types() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Config = { name: Text, count: Int, enabled: Bool }
    // Mix of builtin and imported types
    let mut config_fields = IndexMap::new();
    config_fields.insert(Text::from("name"), Type::text());
    config_fields.insert(Text::from("count"), Type::int());
    config_fields.insert(Text::from("enabled"), Type::bool());
    let config_type = Type::Record(config_fields);

    // Add config variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("config"), TypeScheme::mono(config_type.clone()));

    // Test all three fields
    // Expression: config.name
    let name_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("config".to_string(), span))),
                span,
            )),
            field: Ident::new("name".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&name_expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().ty, Type::text());

    // Expression: config.count
    let count_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("config".to_string(), span))),
                span,
            )),
            field: Ident::new("count".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&count_expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().ty, Type::int());

    // Expression: config.enabled
    let enabled_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("config".to_string(), span))),
                span,
            )),
            field: Ident::new("enabled".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&enabled_expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().ty, Type::bool());
}

// ============================================================================
// Test 4: Field Access in Pattern Matching
// ============================================================================

#[test]
fn test_field_access_in_record_pattern() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Person = { name: Text, age: Int }
    let mut person_fields = IndexMap::new();
    person_fields.insert(Text::from("name"), Type::text());
    person_fields.insert(Text::from("age"), Type::int());
    let person_type = Type::Record(person_fields);

    // Pattern: { name, age } - destructure imported record
    use verum_ast::pattern::FieldPattern;

    let pattern = Pattern::new(
        PatternKind::Record {
            path: Path::single(Ident::new("Person".to_string(), span)),
            fields: vec![
                FieldPattern::shorthand(Ident::new("name".to_string(), span)),
                FieldPattern::shorthand(Ident::new("age".to_string(), span)),
            ].into(),
            rest: false,
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &person_type);
    assert!(
        result.is_ok(),
        "Record pattern on imported type should work"
    );

    // Verify bindings
    let name = checker.context_mut().env.lookup("name");
    assert!(name.is_some());
    assert_eq!(name.unwrap().ty, Type::text());

    let age = checker.context_mut().env.lookup("age");
    assert!(age.is_some());
    assert_eq!(age.unwrap().ty, Type::int());
}

#[test]
fn test_field_access_record_pattern_with_rest() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Data = { x: Int, y: Int, z: Int, metadata: Text }
    let mut data_fields = IndexMap::new();
    data_fields.insert(Text::from("x"), Type::int());
    data_fields.insert(Text::from("y"), Type::int());
    data_fields.insert(Text::from("z"), Type::int());
    data_fields.insert(Text::from("metadata"), Type::text());
    let data_type = Type::Record(data_fields);

    // Pattern: { x, y, .. } - only extract x and y
    use verum_ast::pattern::FieldPattern;

    let pattern = Pattern::new(
        PatternKind::Record {
            path: Path::single(Ident::new("Data".to_string(), span)),
            fields: vec![
                FieldPattern::shorthand(Ident::new("x".to_string(), span)),
                FieldPattern::shorthand(Ident::new("y".to_string(), span)),
            ].into(),
            rest: true, // Allow unmatched fields
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &data_type);
    assert!(
        result.is_ok(),
        "Record pattern with rest on imported type should work"
    );

    // Verify only x and y are bound
    assert!(checker.context_mut().env.lookup("x").is_some());
    assert!(checker.context_mut().env.lookup("y").is_some());
}

// ============================================================================
// Test 5: Error Cases
// ============================================================================

#[test]
fn test_field_access_unknown_field() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define User = { id: Int, name: Text }
    let mut user_fields = IndexMap::new();
    user_fields.insert(Text::from("id"), Type::int());
    user_fields.insert(Text::from("name"), Type::text());
    let user_type = Type::Record(user_fields);

    // Add user variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("user"), TypeScheme::mono(user_type.clone()));

    // Expression: user.email - unknown field
    let user_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("user".to_string(), span))),
        span,
    );
    let field_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(user_expr),
            field: Ident::new("email".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&field_expr);
    assert!(result.is_err(), "Unknown field should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("email") || err_msg.contains("field"),
        "Error should mention the unknown field: {}",
        err_msg
    );
}

#[test]
fn test_field_access_on_non_record() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Add a non-record variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("value"), TypeScheme::mono(Type::int()));

    // Expression: value.field - trying to access field on Int
    let value_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("value".to_string(), span))),
        span,
    );
    let field_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(value_expr),
            field: Ident::new("field".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&field_expr);
    assert!(
        result.is_err(),
        "Field access on non-record type should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("record") || err_msg.contains("field"),
        "Error should mention record type expected: {}",
        err_msg
    );
}

#[test]
fn test_field_access_nested_unknown_field() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Inner = { value: Int }
    let mut inner_fields = IndexMap::new();
    inner_fields.insert(Text::from("value"), Type::int());
    let inner_type = Type::Record(inner_fields);

    // Define Outer = { inner: Inner }
    let mut outer_fields = IndexMap::new();
    outer_fields.insert(Text::from("inner"), inner_type.clone());
    let outer_type = Type::Record(outer_fields);

    // Add outer variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("obj"), TypeScheme::mono(outer_type.clone()));

    // Expression: obj.inner.unknown - unknown field in nested record
    let obj_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("obj".to_string(), span))),
        span,
    );

    let inner_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(obj_expr),
            field: Ident::new("inner".to_string(), span),
        },
        span,
    );

    let unknown_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(inner_expr),
            field: Ident::new("unknown".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&unknown_expr);
    assert!(
        result.is_err(),
        "Unknown field in nested record should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("unknown") || err_msg.contains("field"),
        "Error should mention the unknown field: {}",
        err_msg
    );
}

// ============================================================================
// Test 6: Complex Scenarios
// ============================================================================

#[test]
fn test_field_access_with_optional_fields() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> variant
    let mut maybe_variants = IndexMap::new();
    maybe_variants.insert(Text::from("None"), Type::Unit);
    maybe_variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(maybe_variants);

    // Define User = { id: Int, age: Maybe<Int> }
    let mut user_fields = IndexMap::new();
    user_fields.insert(Text::from("id"), Type::int());
    user_fields.insert(Text::from("age"), maybe_int.clone());
    let user_type = Type::Record(user_fields);

    // Add user variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("user"), TypeScheme::mono(user_type.clone()));

    // Expression: user.age
    let user_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("user".to_string(), span))),
        span,
    );
    let age_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(user_expr),
            field: Ident::new("age".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&age_expr);
    assert!(result.is_ok(), "Field access on optional field should work");
    // The type should be Maybe<Int> (variant type)
    match &result.unwrap().ty {
        Type::Variant(_) => {} // Expected
        other => panic!("Expected Variant type, got {:?}", other),
    }
}

#[test]
fn test_field_access_generic_record() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Container<T> = { value: T, count: Int }
    // For this test, instantiate with T = Text
    let mut container_fields = IndexMap::new();
    container_fields.insert(Text::from("value"), Type::text());
    container_fields.insert(Text::from("count"), Type::int());
    let container_text = Type::Record(container_fields);

    // Add container variable to context
    checker.context_mut().env.insert(
        Text::from("container"),
        TypeScheme::mono(container_text.clone()),
    );

    // Expression: container.value
    let container_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("container".to_string(), span))),
        span,
    );
    let value_expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(container_expr),
            field: Ident::new("value".to_string(), span),
        },
        span,
    );

    let result = checker.synth_expr(&value_expr);
    assert!(
        result.is_ok(),
        "Field access on generic imported record should work"
    );
    assert_eq!(result.unwrap().ty, Type::text());
}

#[test]
fn test_field_access_in_function_call() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Point = { x: Int, y: Int }
    let mut point_fields = IndexMap::new();
    point_fields.insert(Text::from("x"), Type::int());
    point_fields.insert(Text::from("y"), Type::int());
    let point_type = Type::Record(point_fields);

    // Add point variable to context
    checker
        .context_mut()
        .env
        .insert(Text::from("p"), TypeScheme::mono(point_type.clone()));

    // Add a function abs: Int -> Int to context
    let abs_type = Type::Function {
        params: vec![Type::int()].into(),
        return_type: Box::new(Type::int()),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };
    checker
        .context_mut()
        .env
        .insert(Text::from("abs"), TypeScheme::mono(abs_type));

    // Expression: abs(p.x)
    let p_x = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("p".to_string(), span))),
                span,
            )),
            field: Ident::new("x".to_string(), span),
        },
        span,
    );

    let call_expr = Expr::new(
        ExprKind::Call { type_args: vec![].into(),
            func: Box::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("abs".to_string(), span))),
                span,
            )),
            args: vec![p_x].into(),
        },
        span,
    );

    let result = checker.synth_expr(&call_expr);
    assert!(result.is_ok(), "Field access in function call should work");
    assert_eq!(result.unwrap().ty, Type::int());
}
