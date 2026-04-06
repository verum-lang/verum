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
//! Comprehensive Test Suite for Structured Error Contexts
//!
//! Tests all aspects of the structured context system:
//! - ContextValue types and conversions
//! - ToContextValue trait implementations
//! - Structured context API
//! - Result extension traits
//! - Output formatters (JSON, YAML, Logfmt)
//! - Zero-cost properties
//! - Edge cases and error conditions

use verum_common::{List, Map, Maybe, Text};
use verum_error::prelude::*;
use verum_error::structured_context::{ContextValue, ToContextValue};
use verum_error::{ErrorKind, VerumError};

// ============================================================================
// ContextValue Tests
// ============================================================================

#[test]
fn test_context_value_text() {
    let value = ContextValue::Text("hello world".to_string().into());
    let expected: Text = "hello world".to_string().into();
    assert_eq!(value.as_text(), Some(&expected));
    assert_eq!(value.as_int(), None);
    assert_eq!(value.as_bool(), None);
    assert!(!value.is_null());
}

#[test]
fn test_context_value_int() {
    let value = ContextValue::Int(42);
    assert_eq!(value.as_int(), Some(42));
    assert_eq!(value.as_text(), None);
    assert_eq!(value.as_uint(), None);
}

#[test]
fn test_context_value_uint() {
    let value = ContextValue::UInt(12345);
    assert_eq!(value.as_uint(), Some(12345));
    assert_eq!(value.as_int(), None);
}

#[test]
fn test_context_value_float() {
    let value = ContextValue::Float(std::f64::consts::PI);
    assert_eq!(value.as_float(), Some(std::f64::consts::PI));
    assert_eq!(value.as_int(), None);
}

#[test]
fn test_context_value_bool() {
    let value_true = ContextValue::Bool(true);
    let value_false = ContextValue::Bool(false);
    assert_eq!(value_true.as_bool(), Some(true));
    assert_eq!(value_false.as_bool(), Some(false));
}

#[test]
fn test_context_value_null() {
    let value = ContextValue::Null;
    assert!(value.is_null());
    assert_eq!(value.as_text(), None);
    assert_eq!(value.as_int(), None);
}

#[test]
fn test_context_value_list() {
    let list: List<ContextValue> = vec![
        ContextValue::Int(1),
        ContextValue::Int(2),
        ContextValue::Int(3),
    ].into();
    let value = ContextValue::List(list.clone());
    assert_eq!(value.as_list(), Some(&list));
    assert_eq!(value.as_map(), None);
}

#[test]
fn test_context_value_map() {
    let mut map: Map<Text, ContextValue> = Map::new();
    map.insert("key1".to_string().into(), ContextValue::Int(42));
    map.insert("key2".to_string().into(), ContextValue::Text("value".to_string().into()));

    let value = ContextValue::Map(map.clone());
    assert_eq!(value.as_map(), Some(&map));
    assert_eq!(value.as_list(), None);
}

#[test]
fn test_context_value_nested_structures() {
    let mut inner_map: Map<Text, ContextValue> = Map::new();
    inner_map.insert("inner_key".to_string().into(), ContextValue::Int(100));

    let mut outer_map: Map<Text, ContextValue> = Map::new();
    outer_map.insert("nested".to_string().into(), ContextValue::Map(inner_map));
    outer_map.insert(
        "list".to_string().into(),
        ContextValue::List(vec![ContextValue::Int(1), ContextValue::Int(2)].into()),
    );

    let value = ContextValue::Map(outer_map);
    let map = value.as_map().unwrap();
    let nested_key: Text = "nested".to_string().into();
    let list_key: Text = "list".to_string().into();
    assert!(map.contains_key(&nested_key));
    assert!(map.contains_key(&list_key));
}

// ============================================================================
// ToContextValue Trait Tests
// ============================================================================

#[test]
fn test_to_context_value_string_types() {
    let text: Text = "hello".to_string().into();
    let str_ref: &str = "world";
    let string_as_str: &str = "rust";

    let expected_hello: Text = "hello".to_string().into();
    let expected_world: Text = "world".to_string().into();
    let expected_rust: Text = "rust".to_string().into();

    assert_eq!(
        text.to_context_value().as_text(),
        Some(&expected_hello)
    );
    assert_eq!(
        str_ref.to_context_value().as_text(),
        Some(&expected_world)
    );
    // Test &str instead of String since ToContextValue is only implemented for &str
    assert_eq!(
        string_as_str.to_context_value().as_text(),
        Some(&expected_rust)
    );
}

#[test]
fn test_to_context_value_integers() {
    assert_eq!(42i8.to_context_value().as_int(), Some(42));
    assert_eq!(1000i16.to_context_value().as_int(), Some(1000));
    assert_eq!(100000i32.to_context_value().as_int(), Some(100000));
    assert_eq!(9999999i64.to_context_value().as_int(), Some(9999999));
    assert_eq!(42isize.to_context_value().as_int(), Some(42));
}

#[test]
fn test_to_context_value_unsigned_integers() {
    assert_eq!(42u8.to_context_value().as_uint(), Some(42));
    assert_eq!(1000u16.to_context_value().as_uint(), Some(1000));
    assert_eq!(100000u32.to_context_value().as_uint(), Some(100000));
    assert_eq!(9999999u64.to_context_value().as_uint(), Some(9999999));
    assert_eq!(42usize.to_context_value().as_uint(), Some(42));
}

#[test]
fn test_to_context_value_floats() {
    // f32 to f64 conversion has precision loss, so use approximate comparison
    let f32_val = std::f32::consts::PI.to_context_value().as_float().unwrap();
    assert!(
        (f32_val - std::f64::consts::PI).abs() < 0.0001,
        "f32 value should be approximately PI"
    );
    assert_eq!(
        std::f64::consts::E.to_context_value().as_float(),
        Some(std::f64::consts::E)
    );
}

#[test]
fn test_to_context_value_bool() {
    assert_eq!(true.to_context_value().as_bool(), Some(true));
    assert_eq!(false.to_context_value().as_bool(), Some(false));
}

#[test]
fn test_to_context_value_list() {
    let list: List<i32> = vec![1, 2, 3, 4, 5].into();
    let value = list.to_context_value();
    let result_list = value.as_list().unwrap();
    assert_eq!(result_list.len(), 5);
    assert_eq!(result_list[0].as_int(), Some(1));
    assert_eq!(result_list[4].as_int(), Some(5));
}

#[test]
fn test_to_context_value_maybe_some() {
    let maybe: Maybe<i32> = Some(42);
    let value = maybe.to_context_value();
    assert_eq!(value.as_int(), Some(42));
}

#[test]
fn test_to_context_value_maybe_none() {
    let maybe: Maybe<i32> = None;
    let value = maybe.to_context_value();
    assert!(value.is_null());
}

#[test]
fn test_to_context_value_nested_list() {
    let inner1: List<i32> = vec![1, 2].into();
    let inner2: List<i32> = vec![3, 4].into();
    let list: List<List<i32>> = vec![inner1, inner2].into();
    let value = list.to_context_value();
    let outer = value.as_list().unwrap();
    assert_eq!(outer.len(), 2);
    assert_eq!(outer[0].as_list().unwrap().len(), 2);
}

// ============================================================================
// JSON Formatting Tests
// ============================================================================

#[test]
fn test_json_primitives() {
    assert_eq!(
        ContextValue::Text("hello".to_string().into()).to_json(false),
        "\"hello\""
    );
    assert_eq!(ContextValue::Int(42).to_json(false), "42");
    assert_eq!(ContextValue::Int(-10).to_json(false), "-10");
    assert_eq!(ContextValue::UInt(100).to_json(false), "100");
    assert_eq!(ContextValue::Float(1.5).to_json(false), "1.5");
    assert_eq!(ContextValue::Bool(true).to_json(false), "true");
    assert_eq!(ContextValue::Bool(false).to_json(false), "false");
    assert_eq!(ContextValue::Null.to_json(false), "null");
}

#[test]
fn test_json_escape_special_characters() {
    let value = ContextValue::Text("hello \"world\"\nnew line\ttab".to_string().into());
    let json = value.to_json(false);
    assert!(json.contains("\\\""));
    assert!(json.contains("\\n"));
    assert!(json.contains("\\t"));
}

#[test]
fn test_json_list_compact() {
    let list: List<ContextValue> = vec![
        ContextValue::Int(1),
        ContextValue::Int(2),
        ContextValue::Int(3),
    ].into();
    let value = ContextValue::List(list);
    assert_eq!(value.to_json(false), "[1,2,3]");
}

#[test]
fn test_json_list_pretty() {
    let list: List<ContextValue> = vec![
        ContextValue::Int(1),
        ContextValue::Int(2),
        ContextValue::Int(3),
    ].into();
    let value = ContextValue::List(list);
    let json = value.to_json(true);
    assert!(json.contains("[\n"));
    assert!(json.contains("\n]"));
}

#[test]
fn test_json_map_compact() {
    let mut map: Map<Text, ContextValue> = Map::new();
    map.insert("name".to_string().into(), ContextValue::Text("Alice".to_string().into()));
    map.insert("age".to_string().into(), ContextValue::Int(30));

    let value = ContextValue::Map(map);
    let json = value.to_json(false);

    // Keys should be sorted
    assert!(json.contains("\"age\":30"));
    assert!(json.contains("\"name\":\"Alice\""));
    // Should be compact (no newlines)
    assert!(!json.contains("\n"));
}

#[test]
fn test_json_map_pretty() {
    let mut map: Map<Text, ContextValue> = Map::new();
    map.insert("name".to_string().into(), ContextValue::Text("Alice".to_string().into()));
    map.insert("age".to_string().into(), ContextValue::Int(30));

    let value = ContextValue::Map(map);
    let json = value.to_json(true);

    assert!(json.contains("{\n"));
    assert!(json.contains("\n}"));
    assert!(json.contains("  ")); // Should have indentation
}

#[test]
fn test_json_special_floats() {
    // NaN should be null
    let nan = ContextValue::Float(f64::NAN);
    assert_eq!(nan.to_json(false), "null");

    // Infinity
    let inf = ContextValue::Float(f64::INFINITY);
    assert_eq!(inf.to_json(false), "\"Infinity\"");

    let neg_inf = ContextValue::Float(f64::NEG_INFINITY);
    assert_eq!(neg_inf.to_json(false), "\"-Infinity\"");
}

// ============================================================================
// YAML Formatting Tests
// ============================================================================

#[test]
fn test_yaml_primitives() {
    assert_eq!(ContextValue::Text("hello".to_string().into()).to_yaml(0), "hello");
    assert_eq!(ContextValue::Int(42).to_yaml(0), "42");
    assert_eq!(ContextValue::UInt(100).to_yaml(0), "100");
    assert_eq!(ContextValue::Float(1.5).to_yaml(0), "1.5");
    assert_eq!(ContextValue::Bool(true).to_yaml(0), "true");
    assert_eq!(ContextValue::Null.to_yaml(0), "null");
}

#[test]
fn test_yaml_quote_special_strings() {
    // Strings with colons should be quoted
    let value = ContextValue::Text("key: value".to_string().into());
    let yaml = value.to_yaml(0);
    assert!(yaml.starts_with("\""));

    // Strings with newlines should be quoted
    let value = ContextValue::Text("line1\nline2".to_string().into());
    let yaml = value.to_yaml(0);
    assert!(yaml.starts_with("\""));
}

#[test]
fn test_yaml_special_floats() {
    assert_eq!(ContextValue::Float(f64::NAN).to_yaml(0), ".nan");
    assert_eq!(ContextValue::Float(f64::INFINITY).to_yaml(0), ".inf");
    assert_eq!(ContextValue::Float(f64::NEG_INFINITY).to_yaml(0), "-.inf");
}

#[test]
fn test_yaml_list() {
    let list: List<ContextValue> = vec![
        ContextValue::Int(1),
        ContextValue::Int(2),
        ContextValue::Int(3),
    ].into();
    let value = ContextValue::List(list);
    let yaml = value.to_yaml(0);
    assert!(yaml.contains("- 1"));
    assert!(yaml.contains("- 2"));
    assert!(yaml.contains("- 3"));
}

#[test]
fn test_yaml_map() {
    let mut map: Map<Text, ContextValue> = Map::new();
    map.insert("name".to_string().into(), ContextValue::Text("Alice".to_string().into()));
    map.insert("age".to_string().into(), ContextValue::Int(30));

    let value = ContextValue::Map(map);
    let yaml = value.to_yaml(0);
    assert!(yaml.contains("age: 30"));
    assert!(yaml.contains("name: Alice"));
}

// ============================================================================
// Logfmt Formatting Tests
// ============================================================================

#[test]
fn test_logfmt_primitives() {
    assert_eq!(ContextValue::Text("hello".to_string().into()).to_logfmt(), "hello");
    assert_eq!(ContextValue::Int(42).to_logfmt(), "42");
    assert_eq!(ContextValue::Bool(true).to_logfmt(), "true");
    assert_eq!(ContextValue::Null.to_logfmt(), "null");
}

#[test]
fn test_logfmt_quote_spaces() {
    let value = ContextValue::Text("hello world".to_string().into());
    assert_eq!(value.to_logfmt(), "\"hello world\"");
}

#[test]
fn test_logfmt_quote_special_chars() {
    let value = ContextValue::Text("key=value".to_string().into());
    let logfmt = value.to_logfmt();
    assert!(logfmt.starts_with("\""));
}

// ============================================================================
// ContextError Structured Context Tests
// ============================================================================

#[test]
fn test_add_single_structured_context() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context").add_structured("user_id", 12345);

    assert_eq!(
        ctx_error
            .get_structured_context("user_id")
            .unwrap()
            .as_int(),
        Some(12345)
    );
}

#[test]
fn test_add_multiple_structured_contexts() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context")
        .add_structured("user_id", 12345)
        .add_structured("operation", "fetch_user")
        .add_structured("retry_count", 3);

    assert_eq!(
        ctx_error
            .get_structured_context("user_id")
            .unwrap()
            .as_int(),
        Some(12345)
    );
    let expected_fetch_user: Text = "fetch_user".to_string().into();
    assert_eq!(
        ctx_error
            .get_structured_context("operation")
            .unwrap()
            .as_text(),
        Some(&expected_fetch_user)
    );
    assert_eq!(
        ctx_error
            .get_structured_context("retry_count")
            .unwrap()
            .as_int(),
        Some(3)
    );
}

#[test]
fn test_add_structured_map() {
    let mut map: Map<Text, ContextValue> = Map::new();
    map.insert("key1".to_string().into(), ContextValue::Int(42));
    map.insert("key2".to_string().into(), ContextValue::Text("value".to_string().into()));

    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context").add_structured_map(map);

    assert_eq!(
        ctx_error.get_structured_context("key1").unwrap().as_int(),
        Some(42)
    );
    let expected_value: Text = "value".to_string().into();
    assert_eq!(
        ctx_error.get_structured_context("key2").unwrap().as_text(),
        Some(&expected_value)
    );
}

#[test]
fn test_structured_contexts_accessor() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context")
        .add_structured("key1", 42)
        .add_structured("key2", "value");

    let structured = ctx_error.structured_contexts();
    assert_eq!(structured.len(), 2);
    let key1: Text = "key1".to_string().into();
    let key2: Text = "key2".to_string().into();
    assert!(structured.contains_key(&key1));
    assert!(structured.contains_key(&key2));
}

// ============================================================================
// Result Extension Trait Tests
// ============================================================================

#[test]
fn test_result_with_structured() {
    let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
    let ctx_result = result.with_structured("user_id", 12345);

    assert!(ctx_result.is_err());
    let err = ctx_result.unwrap_err();
    assert_eq!(
        err.get_structured_context("user_id").unwrap().as_int(),
        Some(12345)
    );
}

#[test]
fn test_result_with_structured_map() {
    let mut map: Map<Text, ContextValue> = Map::new();
    map.insert("key1".to_string().into(), ContextValue::Int(42));
    map.insert("key2".to_string().into(), ContextValue::Text("value".to_string().into()));

    let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
    let ctx_result = result.with_structured_map(map);

    assert!(ctx_result.is_err());
    let err = ctx_result.unwrap_err();
    assert_eq!(
        err.get_structured_context("key1").unwrap().as_int(),
        Some(42)
    );
}

#[test]
fn test_result_with_structured_fn_zero_cost() {
    let mut call_count = 0;

    // Success path - closure should NOT be called
    let result: Result<i32, VerumError> = Ok(42);
    let ctx_result = result.with_structured_fn(|| {
        call_count += 1;
        ("key", 100)
    });

    assert!(ctx_result.is_ok());
    assert_eq!(call_count, 0, "Closure should not be called on success");

    // Error path - closure SHOULD be called
    let result: Result<i32, VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
    let ctx_result = result.with_structured_fn(|| {
        call_count += 1;
        ("key", 100)
    });

    assert!(ctx_result.is_err());
    assert_eq!(call_count, 1, "Closure should be called on error");
}

#[test]
fn test_result_with_structured_map_fn_zero_cost() {
    let mut call_count = 0;

    // Success path
    let result: Result<i32, VerumError> = Ok(42);
    let ctx_result = result.with_structured_map_fn(|| {
        call_count += 1;
        Map::new()
    });

    assert!(ctx_result.is_ok());
    assert_eq!(call_count, 0);

    // Error path
    let result: Result<i32, VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
    let ctx_result = result.with_structured_map_fn(|| {
        call_count += 1;
        Map::new()
    });

    assert!(ctx_result.is_err());
    assert_eq!(call_count, 1);
}

#[test]
fn test_chaining_structured_contexts() {
    let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
    let ctx_result = result
        .with_structured("key1", 42)
        .add_structured("key2", "value")
        .add_structured("key3", true);

    assert!(ctx_result.is_err());
    let err = ctx_result.unwrap_err();
    assert_eq!(
        err.get_structured_context("key1").unwrap().as_int(),
        Some(42)
    );
    let expected_value: Text = "value".to_string().into();
    assert_eq!(
        err.get_structured_context("key2").unwrap().as_text(),
        Some(&expected_value)
    );
    assert_eq!(
        err.get_structured_context("key3").unwrap().as_bool(),
        Some(true)
    );
}

// ============================================================================
// Output Format Tests
// ============================================================================

#[test]
fn test_output_format_names() {
    assert_eq!(OutputFormat::PlainText.name(), "plain");
    assert_eq!(OutputFormat::Json.name(), "json");
    assert_eq!(OutputFormat::JsonPretty.name(), "json-pretty");
    assert_eq!(OutputFormat::Yaml.name(), "yaml");
    assert_eq!(OutputFormat::Logfmt.name(), "logfmt");
}

#[test]
fn test_output_format_from_str() {
    assert_eq!(
        OutputFormat::from_str("plain"),
        Some(OutputFormat::PlainText)
    );
    assert_eq!(OutputFormat::from_str("json"), Some(OutputFormat::Json));
    assert_eq!(OutputFormat::from_str("yaml"), Some(OutputFormat::Yaml));
    assert_eq!(OutputFormat::from_str("logfmt"), Some(OutputFormat::Logfmt));
    assert_eq!(OutputFormat::from_str("invalid"), None);
}

#[test]
fn test_format_plain_text() {
    let error = VerumError::new("Connection timeout", ErrorKind::Network);
    let ctx_error = ContextError::new(error, "Failed to connect")
        .add_structured("user_id", 12345)
        .add_structured("retry_count", 3);

    let output = ctx_error.format_plain_text();

    // VerumError Display format is "[{kind}] {message}"
    assert!(output.contains("Error: [Network] Connection timeout"));
    assert!(output.contains("Context:"));
    assert!(output.contains("Failed to connect"));
    assert!(output.contains("Structured Data:"));
    assert!(output.contains("user_id: 12345"));
    assert!(output.contains("retry_count: 3"));
}

#[test]
fn test_format_json_compact() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context").add_structured("key", 42);

    let output = ctx_error.format_json(false);

    assert!(output.contains("\"error\":"));
    assert!(output.contains("\"context\":"));
    assert!(output.contains("\"structured\":"));
    assert!(!output.contains("\n")); // Compact format
}

#[test]
fn test_format_json_pretty() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context").add_structured("key", 42);

    let output = ctx_error.format_json(true);

    assert!(output.contains("\"error\":"));
    assert!(output.contains("\n")); // Pretty format has newlines
    assert!(output.contains("  ")); // Pretty format has indentation
}

#[test]
fn test_format_yaml() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context").add_structured("key", 42);

    let output = ctx_error.format_yaml();

    assert!(output.contains("error:"));
    assert!(output.contains("context:"));
    assert!(output.contains("structured:"));
    assert!(output.contains("key: 42"));
}

#[test]
fn test_format_logfmt() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context")
        .add_structured("user_id", 12345)
        .add_structured("operation", "fetch");

    let output = ctx_error.format_logfmt();

    assert!(output.contains("error="));
    assert!(output.contains("user_id=12345"));
    assert!(output.contains("operation=fetch"));
}

#[test]
fn test_format_with_dispatcher() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context").add_structured("key", 42);

    for format in OutputFormat::all() {
        let output = ctx_error.format_with(*format);
        assert!(
            !output.is_empty(),
            "Format {:?} produced empty output",
            format
        );
    }
}

// ============================================================================
// Edge Cases and Error Conditions
// ============================================================================

#[test]
fn test_empty_structured_contexts() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context");

    assert!(ctx_error.structured_contexts().is_empty());
    assert_eq!(ctx_error.get_structured_context("nonexistent"), None);
}

#[test]
fn test_overwrite_structured_context_key() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context")
        .add_structured("key", 42)
        .add_structured("key", 100); // Overwrite

    assert_eq!(
        ctx_error.get_structured_context("key").unwrap().as_int(),
        Some(100)
    );
}

#[test]
fn test_deeply_nested_structures() {
    let mut inner: Map<Text, ContextValue> = Map::new();
    inner.insert("level3".to_string().into(), ContextValue::Int(42));

    let mut middle: Map<Text, ContextValue> = Map::new();
    middle.insert("level2".to_string().into(), ContextValue::Map(inner));

    let mut outer: Map<Text, ContextValue> = Map::new();
    outer.insert("level1".to_string().into(), ContextValue::Map(middle));

    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error =
        ContextError::new(error, "Context").add_structured("nested", ContextValue::Map(outer));

    let json = ctx_error.format_json(true);
    assert!(json.contains("level1"));
    assert!(json.contains("level2"));
    assert!(json.contains("level3"));
}

#[test]
fn test_special_characters_in_keys() {
    let error = VerumError::new("Error", ErrorKind::Other);
    let ctx_error = ContextError::new(error, "Context")
        .add_structured("key with spaces", 42)
        .add_structured("key:with:colons", 100);

    assert_eq!(
        ctx_error
            .get_structured_context("key with spaces")
            .unwrap()
            .as_int(),
        Some(42)
    );
    assert_eq!(
        ctx_error
            .get_structured_context("key:with:colons")
            .unwrap()
            .as_int(),
        Some(100)
    );
}

#[test]
fn test_display_string_conversion() {
    let value = ContextValue::Text("hello".to_string().into());
    assert_eq!(value.to_display_string(), "hello");

    let value = ContextValue::Int(42);
    assert_eq!(value.to_display_string(), "42");

    let list: List<ContextValue> = vec![ContextValue::Int(1), ContextValue::Int(2)].into();
    let value = ContextValue::List(list);
    assert_eq!(value.to_display_string(), "[1, 2]");
}
