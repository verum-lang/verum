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
    unused_assignments,
    clippy::approx_constant
)]
//! Comprehensive tests for Error Context Protocol (Tier 2.3)
//!
//! Tests cover:
//! - Basic context addition
//! - Lazy context evaluation
//! - Context chain propagation
//! - Backtrace capture
//! - Display formats
//! - Metadata attachment
//! - Source location tracking
//! - Zero-cost abstraction guarantees

use std::error::Error;
use verum_common::Text;
use verum_diagnostics::context;
use verum_diagnostics::context_protocol::*;

#[derive(Debug, Clone)]
struct TestError {
    message: String,
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TestError {}

// Test 1: Basic context addition
#[test]
fn test_context_basic() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base error".into(),
    });

    let err = result.context("operation failed").unwrap_err();

    assert_eq!(err.context.message.as_str(), "operation failed");
    assert_eq!(err.error.message, "base error");
}

// Test 2: Lazy context with closure
#[test]
fn test_with_context_lazy() {
    let expensive_value = "expensive computation";
    let result: Result<(), TestError> = Err(TestError {
        message: "base error".into(),
    });

    let err = result
        .with_context(|| format!("computed: {}", expensive_value))
        .unwrap_err();

    assert!(err.context.message.as_str().contains("computed"));
    assert!(err.context.message.as_str().contains(expensive_value));
}

// Test 3: Context chain building
#[test]
fn test_context_chain() {
    fn inner() -> Result<(), ErrorWithContext<TestError>> {
        Err(TestError {
            message: "inner error".into(),
        })
        .context("inner operation")
    }

    fn middle() -> Result<ErrorWithContext<TestError>, ErrorWithContext<TestError>> {
        let err = inner().unwrap_err();
        Err(err.with_additional_context("middle operation"))
    }

    fn outer() -> Result<ErrorWithContext<TestError>, ErrorWithContext<TestError>> {
        let err = middle().unwrap_err();
        Err(err.with_additional_context("outer operation"))
    }

    let err = outer().unwrap_err();
    assert_eq!(err.context.message.as_str(), "inner operation");
}

// Test 4: Source location tracking
#[test]
fn test_source_location() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result.context("test context").unwrap_err();

    // Location should be captured
    assert!(!err.context.location.file.as_str().is_empty());
    assert!(err.context.location.line > 0);
}

// Test 5: Custom location
#[test]
fn test_custom_location() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result.at("custom.rs", 42, 15).unwrap_err();

    assert_eq!(err.context.location.file.as_str(), "custom.rs");
    assert_eq!(err.context.location.line, 42);
    assert_eq!(err.context.location.column, 15);
}

// Test 6: Operation context
#[test]
fn test_operation_context() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result.operation("database_query").unwrap_err();

    assert_eq!(err.context.message.as_str(), "database_query");
    assert_eq!(err.context.context_chain.len(), 1);
    assert_eq!(
        err.context.context_chain[0].operation.as_str(),
        "database_query"
    );
}

// Test 7: Metadata attachment
#[test]
fn test_metadata() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result
        .meta("user_id", "usr_123")
        .unwrap_err()
        .with_metadata(
            String::from("request_id"),
            ContextValue::Text("req_456".into()),
        );

    assert_eq!(err.context.metadata.len(), 2);
    assert!(err.context.metadata.contains_key(&Text::from("user_id")));
    assert!(
        err.context
            .metadata
            .contains_key(&Text::from("request_id"))
    );
}

// Test 8: Display formats - full
#[test]
fn test_display_full() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base error".into(),
    });

    let err = result.context("operation failed").unwrap_err();
    let full = err.display_full();

    assert!(full.as_str().contains("base error"));
    assert!(full.as_str().contains("operation failed"));
    assert!(full.as_str().contains("Error:"));
}

// Test 9: Display formats - user
#[test]
fn test_display_user() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base error".into(),
    });

    let err = result.context("operation failed").unwrap_err();
    let user = err.display_user();

    assert!(user.as_str().contains("operation failed"));
    assert!(user.as_str().contains("base error"));
}

// Test 10: Display formats - developer
#[test]
fn test_display_developer() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base error".into(),
    });

    let err = result.context("operation failed").unwrap_err();
    let dev = err.display_developer();

    // Developer format should be same as full
    assert!(dev.as_str().contains("base error"));
    assert!(dev.as_str().contains("operation failed"));
}

// Test 11: Display formats - log (structured JSON)
#[test]
fn test_display_log() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base error".into(),
    });

    let err = result.context("operation failed").unwrap_err();
    let log = err.display_log();

    // Should produce valid JSON structure
    assert!(log.as_str().starts_with("{"));
    assert!(log.as_str().ends_with("}"));
    assert!(log.as_str().contains("\"error\":"));
    assert!(log.as_str().contains("\"context\":"));
    assert!(log.as_str().contains("\"location\":"));
    assert!(log.as_str().contains("\"file\":"));
    assert!(log.as_str().contains("\"line\":"));
    assert!(log.as_str().contains("\"column\":"));
}

// Test 11b: Display log JSON escaping
#[test]
fn test_display_log_json_escaping() {
    let result: Result<(), TestError> = Err(TestError {
        message: "error with \"quotes\" and \\ backslash".into(),
    });

    let err = result.context("context with\nnewline").unwrap_err();
    let log = err.display_log();

    // Should properly escape special JSON characters
    assert!(log.as_str().contains("\\\"quotes\\\""));
    assert!(log.as_str().contains("\\\\"));
    assert!(log.as_str().contains("\\n"));
}

// Test 11c: Display log with metadata
#[test]
fn test_display_log_with_metadata() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result
        .meta("request_id", "req-123")
        .unwrap_err()
        .with_metadata(String::from("count"), ContextValue::Int(42));
    let log = err.display_log();

    assert!(log.as_str().contains("\"metadata\":"));
    assert!(log.as_str().contains("\"request_id\":"));
    assert!(log.as_str().contains("\"count\":"));
    assert!(log.as_str().contains("42"));
}

// Test 11d: Display log with nested List and Map metadata
#[test]
fn test_display_log_with_nested_metadata() {
    use std::collections::HashMap;

    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    // Create a nested List
    let list_value = ContextValue::List(vec![
        ContextValue::Text("item1".into()),
        ContextValue::Int(100),
        ContextValue::Bool(true),
    ].into());

    // Create a nested Map
    let mut inner_map = HashMap::new();
    inner_map.insert(Text::from("key1"), ContextValue::Text("value1".into()));
    inner_map.insert(Text::from("key2"), ContextValue::Int(200));
    let map_value = ContextValue::Map(inner_map);

    let err = result
        .meta("list_field", list_value)
        .unwrap_err()
        .with_metadata(String::from("map_field"), map_value);
    let log = err.display_log();

    // Verify List serialization
    assert!(log.as_str().contains("\"list_field\":"));
    assert!(log.as_str().contains("["));
    assert!(log.as_str().contains("\"item1\""));
    assert!(log.as_str().contains("100"));
    assert!(log.as_str().contains("true"));
    assert!(log.as_str().contains("]"));

    // Verify Map serialization
    assert!(log.as_str().contains("\"map_field\":"));
    assert!(log.as_str().contains("\"key1\":"));
    assert!(log.as_str().contains("\"value1\""));
    assert!(log.as_str().contains("\"key2\":"));
    assert!(log.as_str().contains("200"));
}

// Test 11e: Display log with deeply nested structures
#[test]
fn test_display_log_with_deeply_nested_structures() {
    use std::collections::HashMap;

    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    // Create a List containing a Map
    let mut inner_map = HashMap::new();
    inner_map.insert(
        Text::from("nested_key"),
        ContextValue::Text("nested_value".into()),
    );

    let nested_list = ContextValue::List(vec![
        ContextValue::Int(1),
        ContextValue::Map(inner_map),
        ContextValue::List(vec![ContextValue::Bool(false), ContextValue::Float(3.14)].into()),
    ].into());

    let err = result.meta("complex", nested_list).unwrap_err();
    let log = err.display_log();

    // Verify deep nesting is properly serialized
    assert!(log.as_str().contains("\"complex\":"));
    assert!(log.as_str().contains("\"nested_key\":"));
    assert!(log.as_str().contains("\"nested_value\""));
    assert!(log.as_str().contains("3.14"));
    assert!(log.as_str().contains("false"));
}

// Test 12: Backtrace capture (when enabled)
#[test]
fn test_backtrace_capture() {
    // Note: Backtrace capture depends on VERUM_BACKTRACE env var
    // This test just verifies the API works
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result.context("test context").unwrap_err();

    // Backtrace may be None (default) or Some (if VERUM_BACKTRACE=1)
    // We just verify it doesn't panic
    let _ = err.backtrace();
}

// Test 13: Context value conversions
#[test]
fn test_context_value_conversions() {
    let _cv1: ContextValue = "string".into();
    let _cv2: ContextValue = Text::from("owned").into();
    let _cv3: ContextValue = 42i32.into();
    let _cv4: ContextValue = 42i64.into();
    let _cv5: ContextValue = 3.14f64.into();
    let _cv6: ContextValue = true.into();

    // All conversions should compile
}

// Test 14: Multiple context layers
#[test]
fn test_multiple_context_layers() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base".into(),
    });

    let err = result
        .context("layer 1")
        .unwrap_err()
        .with_additional_context("layer 2")
        .with_additional_context("layer 3");

    assert_eq!(err.context.context_chain.len(), 2); // layer 2 and layer 3
}

// Test 15: Success path has no overhead
#[test]
fn test_success_path_no_overhead() {
    let result: Result<i32, TestError> = Ok(42);

    // This should compile to essentially a no-op on success
    let value = result.context("this context is never used").unwrap();

    assert_eq!(value, 42);
}

// Test 16: Error source chain
#[test]
fn test_error_source_chain() {
    let result: Result<(), TestError> = Err(TestError {
        message: "original".into(),
    });

    let err = result.context("wrapped").unwrap_err();

    // Should implement std::error::Error
    assert!(err.source().is_some());
}

// Test 17: Clone support
#[test]
fn test_clone_support() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err1 = result.context("test").unwrap_err();
    let err2 = err1.clone();

    assert_eq!(err1.context.message, err2.context.message);
    assert_eq!(err1.error.message, err2.error.message);
}

// Test 18: Source location with function name
#[test]
fn test_source_location_with_function() {
    let loc = SourceLocation::new("test.rs", 10, 5).with_function("my_function");

    assert_eq!(loc.file.as_str(), "test.rs");
    assert_eq!(loc.line, 10);
    assert_eq!(loc.column, 5);
    assert_eq!(loc.function.unwrap().as_str(), "my_function");
}

// Test 19: Context frame tracking
#[test]
fn test_context_frame_tracking() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result.operation("test_op").unwrap_err();

    assert!(!err.context.context_chain.is_empty());
    let frame = &err.context.context_chain[0];
    assert_eq!(frame.operation.as_str(), "test_op");
    assert!(frame.timestamp > 0);
}

// Test 20: Empty context handling
#[test]
fn test_empty_context() {
    let ctx = ErrorContext::default();

    assert_eq!(ctx.message.as_str(), "");
    assert_eq!(ctx.context_chain.len(), 0);
    assert_eq!(ctx.metadata.len(), 0);
}

// Test 21: Macro context! usage
#[test]
fn test_context_macro() {
    let result: Result<(), TestError> = Err(TestError {
        message: "base".into(),
    });

    let err = context!(result, "from macro").unwrap_err();
    assert_eq!(err.context.message.as_str(), "from macro");
}

// Test 22: Integration with standard errors
#[test]
fn test_std_error_integration() {
    use std::io;

    let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
    let result: Result<(), io::Error> = Err(io_err);

    let err = result.context("failed to read config").unwrap_err();

    assert_eq!(err.context.message.as_str(), "failed to read config");
    assert!(err.error.to_string().contains("file not found"));
}

// Test 23: Complex nested context
#[test]
fn test_complex_nested_context() {
    fn level3() -> Result<(), ErrorWithContext<TestError>> {
        Err(TestError {
            message: "level 3 error".into(),
        })
        .context("level 3 context")
    }

    fn level2() -> Result<ErrorWithContext<TestError>, ErrorWithContext<TestError>> {
        let err = level3().unwrap_err();
        Err(err.with_additional_context("level 2 context"))
    }

    fn level1() -> Result<ErrorWithContext<TestError>, ErrorWithContext<TestError>> {
        let err = level2().unwrap_err();
        Err(err.with_additional_context("level 1 context"))
    }

    let err = level1().unwrap_err();

    // Original context from level 3
    assert_eq!(err.context.message.as_str(), "level 3 context");
    // Should have context chain from propagation
    assert!(!err.context.context_chain.is_empty());
}

// Test 24: Metadata with different types
#[test]
fn test_metadata_mixed_types() {
    let result: Result<(), TestError> = Err(TestError {
        message: "test".into(),
    });

    let err = result
        .meta("string_key", "value")
        .unwrap_err()
        .with_metadata(String::from("int_key"), ContextValue::Int(42))
        .with_metadata(String::from("float_key"), ContextValue::Float(3.14))
        .with_metadata(String::from("bool_key"), ContextValue::Bool(true));

    assert_eq!(err.context.metadata.len(), 4);
}

// Test 25: Performance - many context additions
#[test]
fn test_performance_many_contexts() {
    // This is a stress test to ensure the implementation can handle many context additions
    let result: Result<(), TestError> = Err(TestError {
        message: "base".into(),
    });

    let mut err = result.context("initial").unwrap_err();

    for i in 0..100 {
        err = err.with_additional_context(format!("context {}", i));
    }

    assert_eq!(err.context.context_chain.len(), 100);
}

// Integration test: Full error handling pipeline
#[test]
fn test_full_pipeline() {
    fn read_config(path: &str) -> Result<String, ErrorWithContext<std::io::Error>> {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {}", path))
    }

    fn parse_config(content: &str) -> Result<(), ErrorWithContext<TestError>> {
        if content.is_empty() {
            Err(TestError {
                message: "empty config".into(),
            })
            .context("Config file is empty")
        } else {
            Ok(())
        }
    }

    fn load_and_parse(path: &str) -> Result<(), ErrorWithContext<TestError>> {
        let _content = read_config(path)
            .map_err(|e| TestError {
                message: format!("IO error: {}", e),
            })
            .context("Failed to load config")?;

        Ok(())
    }

    // This should fail with proper context chain
    let result = load_and_parse("/nonexistent/path.conf");
    assert!(result.is_err());
}
