//! Debugging Builtins for @meta_trace
//!
//! This module provides builtin functions for macro debugging during
//! compile-time execution. These are Tier 0 functions (always available).
//!
//! ## Functions
//!
//! - `meta_trace_on()` - Enable tracing
//! - `meta_trace_off()` - Disable tracing
//! - `meta_trace_log(message)` - Log a trace message
//! - `meta_trace_dump()` - Get accumulated trace output
//! - `meta_trace_clear()` - Clear trace buffer
//! - `meta_trace_is_enabled()` - Check if tracing is enabled
//! - `meta_trace_depth()` - Get current call depth
//!
//! ## Usage in Verum
//!
//! ```verum
//! @meta fn debug_example() {
//!     meta_trace_on();
//!     meta_trace_log("Starting computation");
//!     let result = compute_something();
//!     meta_trace_log(f"Result: {result}");
//!     meta_trace_off();
//!
//!     // Get all trace output
//!     let trace = meta_trace_dump();
//!     compile_warning(trace);
//! }
//! ```
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta audit: validation of meta function safety, resource limits,
//! and sandbox compliance before execution.

use verum_common::{List, Text};

use crate::meta::context::{ConstValue, MetaContext};
use crate::meta::MetaError;

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};

/// Register all debugging builtins
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // meta_trace_on - enable tracing
    map.insert(
        Text::from("meta_trace_on"),
        BuiltinInfo::tier0(
            meta_trace_on,
            "Enable tracing for macro debugging",
            "() -> ()",
        ),
    );

    // meta_trace_off - disable tracing
    map.insert(
        Text::from("meta_trace_off"),
        BuiltinInfo::tier0(
            meta_trace_off,
            "Disable tracing for macro debugging",
            "() -> ()",
        ),
    );

    // meta_trace_log - log a trace message
    map.insert(
        Text::from("meta_trace_log"),
        BuiltinInfo::tier0(
            meta_trace_log,
            "Log a trace message (only when tracing enabled)",
            "(Text) -> ()",
        ),
    );

    // meta_trace_dump - get accumulated trace output
    map.insert(
        Text::from("meta_trace_dump"),
        BuiltinInfo::tier0(
            meta_trace_dump,
            "Get accumulated trace output as a single text string",
            "() -> Text",
        ),
    );

    // meta_trace_lines - get trace output as list of lines
    map.insert(
        Text::from("meta_trace_lines"),
        BuiltinInfo::tier0(
            meta_trace_lines,
            "Get accumulated trace output as a list of lines",
            "() -> List<Text>",
        ),
    );

    // meta_trace_clear - clear trace buffer
    map.insert(
        Text::from("meta_trace_clear"),
        BuiltinInfo::tier0(
            meta_trace_clear,
            "Clear the trace output buffer",
            "() -> ()",
        ),
    );

    // meta_trace_is_enabled - check if tracing is enabled
    map.insert(
        Text::from("meta_trace_is_enabled"),
        BuiltinInfo::tier0(
            meta_trace_is_enabled,
            "Check if tracing is currently enabled",
            "() -> Bool",
        ),
    );

    // meta_trace_depth - get current call depth
    map.insert(
        Text::from("meta_trace_depth"),
        BuiltinInfo::tier0(
            meta_trace_depth,
            "Get current trace indentation depth",
            "() -> Int",
        ),
    );

    // meta_trace_enter - manually log function entry
    map.insert(
        Text::from("meta_trace_enter"),
        BuiltinInfo::tier0(
            meta_trace_enter,
            "Log entry into a named scope (increases indent)",
            "(Text) -> ()",
        ),
    );

    // meta_trace_exit - manually log function exit
    map.insert(
        Text::from("meta_trace_exit"),
        BuiltinInfo::tier0(
            meta_trace_exit,
            "Log exit from a named scope (decreases indent)",
            "(Text) -> ()",
        ),
    );

    // meta_trace_value - log a value with label
    map.insert(
        Text::from("meta_trace_value"),
        BuiltinInfo::tier0(
            meta_trace_value,
            "Log a labeled value for inspection",
            "(Text, Any) -> ()",
        ),
    );

    // meta_trace_assert - trace assertion (logs and checks)
    map.insert(
        Text::from("meta_trace_assert"),
        BuiltinInfo::tier0(
            meta_trace_assert,
            "Log and check an assertion condition",
            "(Bool, Text) -> ()",
        ),
    );
}

/// Enable tracing
fn meta_trace_on(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    ctx.trace_on();
    Ok(ConstValue::Unit)
}

/// Disable tracing
fn meta_trace_off(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    ctx.trace_off();
    Ok(ConstValue::Unit)
}

/// Log a trace message
fn meta_trace_log(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let message = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        other => Text::from(format!("{:?}", other)),
    };

    ctx.trace_log(message);
    Ok(ConstValue::Unit)
}

/// Get accumulated trace output as text
fn meta_trace_dump(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    Ok(ConstValue::Text(ctx.dump_trace()))
}

/// Get accumulated trace output as list of lines
fn meta_trace_lines(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let lines = ctx.get_trace_output();
    let values: List<ConstValue> = lines.iter().map(|t| ConstValue::Text(t.clone())).collect();
    Ok(ConstValue::Array(values))
}

/// Clear trace buffer
fn meta_trace_clear(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    ctx.clear_trace_output();
    Ok(ConstValue::Unit)
}

/// Check if tracing is enabled
fn meta_trace_is_enabled(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    Ok(ConstValue::Bool(ctx.is_tracing()))
}

/// Get current trace depth
fn meta_trace_depth(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    Ok(ConstValue::Int(ctx.trace_indent as i128))
}

/// Log entry into a named scope
fn meta_trace_enter(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let name = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        other => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: Text::from(format!("{:?}", other)),
            });
        }
    };

    ctx.trace_enter(&name);
    Ok(ConstValue::Unit)
}

/// Log exit from a named scope
fn meta_trace_exit(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let name = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        other => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: Text::from(format!("{:?}", other)),
            });
        }
    };

    // Just decrease indent and log, no result to show
    if ctx.is_tracing() {
        if ctx.trace_indent > 0 {
            ctx.trace_indent -= 1;
        }
        ctx.trace_log(Text::from(format!("<- {}", name)));
    }
    Ok(ConstValue::Unit)
}

/// Log a labeled value for inspection
fn meta_trace_value(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let label = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        other => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: Text::from(format!("{:?}", other)),
            });
        }
    };

    let value = &args[1];
    let value_str = format_const_value(value);

    ctx.trace_log(Text::from(format!("{} = {}", label, value_str)));
    Ok(ConstValue::Unit)
}

/// Log and check an assertion
fn meta_trace_assert(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let condition = match &args[0] {
        ConstValue::Bool(b) => *b,
        other => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Bool"),
                found: Text::from(format!("{:?}", other)),
            });
        }
    };

    let message = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        other => Text::from(format!("{:?}", other)),
    };

    if condition {
        ctx.trace_log(Text::from(format!("ASSERT OK: {}", message)));
        Ok(ConstValue::Unit)
    } else {
        ctx.trace_log(Text::from(format!("ASSERT FAILED: {}", message)));
        Err(MetaError::AssertionFailed { message })
    }
}

/// Format a ConstValue for trace output
fn format_const_value(value: &ConstValue) -> String {
    match value {
        ConstValue::Unit => "()".to_string(),
        ConstValue::Bool(b) => b.to_string(),
        ConstValue::Int(i) => i.to_string(),
        ConstValue::Float(f) => f.to_string(),
        ConstValue::Text(t) => format!("\"{}\"", t),
        ConstValue::Char(c) => format!("'{}'", c),
        ConstValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_const_value).collect();
            format!("[{}]", items.join(", "))
        }
        ConstValue::Tuple(items) => {
            let formatted: Vec<String> = items.iter().map(format_const_value).collect();
            format!("({})", formatted.join(", "))
        }
        ConstValue::Maybe(opt) => match opt {
            Some(v) => format!("Some({})", format_const_value(v)),
            None => "None".to_string(),
        },
        ConstValue::Map(m) => {
            let items: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("\"{}\": {}", k, format_const_value(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        ConstValue::Set(s) => {
            let items: Vec<String> = s.iter().map(|k| format!("\"{}\"", k)).collect();
            format!("{{{}}}", items.join(", "))
        }
        _ => format!("{:?}", value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_on_off() {
        let mut ctx = MetaContext::new();

        // Initially off
        assert!(!ctx.is_tracing());

        // Turn on
        meta_trace_on(&mut ctx, List::new()).unwrap();
        assert!(ctx.is_tracing());

        // Turn off
        meta_trace_off(&mut ctx, List::new()).unwrap();
        assert!(!ctx.is_tracing());
    }

    #[test]
    fn test_trace_log() {
        let mut ctx = MetaContext::new();

        // Log when tracing is off - no output
        meta_trace_log(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("test1"))]),
        )
        .unwrap();
        assert!(ctx.get_trace_output().is_empty());

        // Turn on and log
        ctx.trace_on();
        meta_trace_log(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("test2"))]),
        )
        .unwrap();
        meta_trace_log(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("test3"))]),
        )
        .unwrap();

        let output = ctx.get_trace_output();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0], Text::from("test2"));
        assert_eq!(output[1], Text::from("test3"));
    }

    #[test]
    fn test_trace_enter_exit() {
        let mut ctx = MetaContext::new();
        ctx.trace_on();

        meta_trace_enter(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("foo"))]),
        )
        .unwrap();
        meta_trace_log(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("inside foo"))]),
        )
        .unwrap();
        meta_trace_exit(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("foo"))]),
        )
        .unwrap();

        let output = ctx.get_trace_output();
        assert_eq!(output.len(), 3);
        assert_eq!(output[0], Text::from("-> foo"));
        assert_eq!(output[1], Text::from("  inside foo"));
        assert_eq!(output[2], Text::from("<- foo"));
    }

    #[test]
    fn test_trace_dump() {
        let mut ctx = MetaContext::new();
        ctx.trace_on();
        ctx.trace_log(Text::from("line1"));
        ctx.trace_log(Text::from("line2"));

        let result = meta_trace_dump(&mut ctx, List::new()).unwrap();
        match result {
            ConstValue::Text(t) => assert_eq!(t, Text::from("line1\nline2")),
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn test_trace_clear() {
        let mut ctx = MetaContext::new();
        ctx.trace_on();
        ctx.trace_log(Text::from("line1"));

        assert!(!ctx.get_trace_output().is_empty());
        meta_trace_clear(&mut ctx, List::new()).unwrap();
        assert!(ctx.get_trace_output().is_empty());
    }

    #[test]
    fn test_trace_value() {
        let mut ctx = MetaContext::new();
        ctx.trace_on();

        meta_trace_value(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("x")), ConstValue::Int(42)]),
        )
        .unwrap();

        let output = ctx.get_trace_output();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0], Text::from("x = 42"));
    }

    #[test]
    fn test_trace_assert_pass() {
        let mut ctx = MetaContext::new();
        ctx.trace_on();

        let result = meta_trace_assert(
            &mut ctx,
            List::from(vec![
                ConstValue::Bool(true),
                ConstValue::Text(Text::from("value is positive")),
            ]),
        );

        assert!(result.is_ok());
        let output = ctx.get_trace_output();
        assert!(output[0].contains("ASSERT OK"));
    }

    #[test]
    fn test_trace_assert_fail() {
        let mut ctx = MetaContext::new();
        ctx.trace_on();

        let result = meta_trace_assert(
            &mut ctx,
            List::from(vec![
                ConstValue::Bool(false),
                ConstValue::Text(Text::from("value should be positive")),
            ]),
        );

        assert!(result.is_err());
        match result {
            Err(MetaError::AssertionFailed { message }) => {
                assert!(message.contains("value should be positive"));
            }
            _ => panic!("Expected AssertionFailed error"),
        }
    }

    #[test]
    fn test_arity_errors() {
        let mut ctx = MetaContext::new();

        // meta_trace_on with args should fail
        let result = meta_trace_on(&mut ctx, List::from(vec![ConstValue::Int(1)]));
        assert!(matches!(result, Err(MetaError::ArityMismatch { .. })));

        // meta_trace_log with no args should fail
        let result = meta_trace_log(&mut ctx, List::new());
        assert!(matches!(result, Err(MetaError::ArityMismatch { .. })));
    }

    #[test]
    fn test_format_const_value() {
        assert_eq!(format_const_value(&ConstValue::Unit), "()");
        assert_eq!(format_const_value(&ConstValue::Bool(true)), "true");
        assert_eq!(format_const_value(&ConstValue::Int(42)), "42");
        assert_eq!(
            format_const_value(&ConstValue::Text(Text::from("hello"))),
            "\"hello\""
        );
        assert_eq!(
            format_const_value(&ConstValue::Array(List::from(vec![
                ConstValue::Int(1),
                ConstValue::Int(2)
            ]))),
            "[1, 2]"
        );
    }
}
