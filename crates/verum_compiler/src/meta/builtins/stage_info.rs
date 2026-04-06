//! Stage Information Intrinsics (Tier 1 - Requires StageInfo)
//!
//! Provides compile-time information about N-level staged metaprogramming.
//! All functions in this module require the `StageInfo` context since they
//! access staged compilation state.
//!
//! ## Stage Query
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `stage_current()` | `() -> Int` | Current compilation stage |
//! | `stage_max()` | `() -> Int` | Maximum stage level |
//! | `stage_is_runtime()` | `() -> Bool` | True if stage 0 |
//! | `stage_is_compile_time()` | `() -> Bool` | True if stage > 0 |
//! | `stage_is_max_stage()` | `() -> Bool` | True if at max stage |
//!
//! ## Stage Validation
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `stage_is_valid(n)` | `(Int) -> Bool` | Check if stage level valid |
//! | `stage_is_valid_transition(from, to)` | `(Int, Int) -> Bool` | Check transition validity |
//! | `stage_quote_target()` | `() -> Int` | Target stage for quote |
//! | `stage_can_generate(from, to)` | `(Int, Int) -> Bool` | Check generation validity |
//!
//! ## Stage-Aware Generation
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `stage_unique_ident(prefix)` | `(Text) -> Text` | Generate unique identifier |
//! | `stage_quote_depth()` | `() -> Int` | Current quote nesting depth |
//!
//! ## Function Stage Information
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `stage_function_stage(name)` | `(Text) -> Int` | Get stage of named function |
//! | `stage_functions_at(level)` | `(Int) -> List<Text>` | All functions at stage N |
//!
//! ## Configuration
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `stage_is_enabled()` | `() -> Bool` | Staging enabled |
//! | `stage_iteration_limit()` | `() -> Int` | Iteration limit |
//! | `stage_recursion_limit()` | `() -> Int` | Recursion limit |
//! | `stage_memory_limit()` | `() -> Int` | Memory limit (bytes) |
//!
//! ## Tracing and Debugging
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `stage_generation_chain()` | `() -> List<StageRecord>` | Generation provenance chain |
//! | `stage_trace_marker(name, data)` | `(Text, Text) -> ()` | Add trace marker |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [StageInfo]` context.

use verum_common::{List, OrderedMap, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register stage info builtins with context requirements
///
/// All stage info functions require StageInfo context since they access
/// staged compilation state.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Stage Query (Tier 1 - StageInfo)
    // ========================================================================

    map.insert(
        Text::from("stage_current"),
        BuiltinInfo::stage_info(
            meta_stage_current,
            "Get the current compilation stage level",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("stage_max"),
        BuiltinInfo::stage_info(
            meta_stage_max,
            "Get the maximum stage level",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("stage_is_runtime"),
        BuiltinInfo::stage_info(
            meta_stage_is_runtime,
            "Check if currently at runtime stage (stage 0)",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("stage_is_compile_time"),
        BuiltinInfo::stage_info(
            meta_stage_is_compile_time,
            "Check if currently at compile-time (stage >= 1)",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("stage_is_max_stage"),
        BuiltinInfo::stage_info(
            meta_stage_is_max_stage,
            "Check if at the maximum allowed stage",
            "() -> Bool",
        ),
    );

    // ========================================================================
    // Stage Validation (Tier 1 - StageInfo)
    // ========================================================================

    map.insert(
        Text::from("stage_is_valid"),
        BuiltinInfo::stage_info(
            meta_stage_is_valid,
            "Validate that a stage level is within [0, max_stage]",
            "(Int) -> Bool",
        ),
    );
    map.insert(
        Text::from("stage_is_valid_transition"),
        BuiltinInfo::stage_info(
            meta_stage_is_valid_transition,
            "Check if a stage transition from->to is valid",
            "(Int, Int) -> Bool",
        ),
    );
    map.insert(
        Text::from("stage_quote_target"),
        BuiltinInfo::stage_info(
            meta_stage_quote_target,
            "Get the target stage for a quote expression",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("stage_can_generate"),
        BuiltinInfo::stage_info(
            meta_stage_can_generate,
            "Check if current stage can generate code for target stage",
            "(Int, Int) -> Bool",
        ),
    );

    // ========================================================================
    // Stage-Aware Generation (Tier 1 - StageInfo)
    // ========================================================================

    map.insert(
        Text::from("stage_unique_ident"),
        BuiltinInfo::stage_info(
            meta_stage_unique_ident,
            "Generate a unique identifier scoped to the current stage",
            "(Text) -> Text",
        ),
    );
    map.insert(
        Text::from("stage_quote_depth"),
        BuiltinInfo::stage_info(
            meta_stage_quote_depth,
            "Get the current quote nesting depth",
            "() -> Int",
        ),
    );

    // ========================================================================
    // Function Stage Information (Tier 1 - StageInfo)
    // ========================================================================

    map.insert(
        Text::from("stage_function_stage"),
        BuiltinInfo::stage_info(
            meta_stage_function_stage,
            "Get the declared stage of a function by name",
            "(Text) -> Int",
        ),
    );
    map.insert(
        Text::from("stage_functions_at"),
        BuiltinInfo::stage_info(
            meta_stage_functions_at,
            "Get all functions declared at a specific stage level",
            "(Int) -> List<Text>",
        ),
    );

    // ========================================================================
    // Configuration (Tier 1 - StageInfo)
    // ========================================================================

    map.insert(
        Text::from("stage_is_enabled"),
        BuiltinInfo::stage_info(
            meta_stage_is_enabled,
            "Check if staged compilation is enabled",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("stage_iteration_limit"),
        BuiltinInfo::stage_info(
            meta_stage_iteration_limit,
            "Get stage-specific iteration limit",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("stage_recursion_limit"),
        BuiltinInfo::stage_info(
            meta_stage_recursion_limit,
            "Get stage-specific recursion limit",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("stage_memory_limit"),
        BuiltinInfo::stage_info(
            meta_stage_memory_limit,
            "Get stage-specific memory limit in bytes",
            "() -> Int",
        ),
    );

    // ========================================================================
    // Tracing and Debugging (Tier 1 - StageInfo)
    // ========================================================================

    map.insert(
        Text::from("stage_generation_chain"),
        BuiltinInfo::stage_info(
            meta_stage_generation_chain,
            "Get the generation chain leading to current execution",
            "() -> List<StageRecord>",
        ),
    );
    map.insert(
        Text::from("stage_trace_marker"),
        BuiltinInfo::stage_info(
            meta_stage_trace_marker,
            "Add a trace marker for debugging staged compilation",
            "(Text, Text) -> ()",
        ),
    );
}

// ============================================================================
// Stage Query
// ============================================================================

/// Get the current compilation stage level
fn meta_stage_current(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.current_stage as i128))
}

/// Get the maximum stage level
fn meta_stage_max(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.max_stage as i128))
}

/// Check if currently at runtime stage (stage 0)
fn meta_stage_is_runtime(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Bool(ctx.current_stage == 0))
}

/// Check if currently at compile-time (stage >= 1)
fn meta_stage_is_compile_time(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Bool(ctx.current_stage >= 1))
}

/// Check if at the maximum allowed stage
fn meta_stage_is_max_stage(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Bool(ctx.current_stage == ctx.max_stage))
}

// ============================================================================
// Stage Validation
// ============================================================================

/// Validate that a stage level is within [0, max_stage]
fn meta_stage_is_valid(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Int(n) => {
            let valid = *n >= 0 && *n <= ctx.max_stage as i128;
            Ok(ConstValue::Bool(valid))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if a stage transition from->to is valid
///
/// A transition is valid when to == from - 1 (quote lowers by exactly 1)
/// and both stages are within valid range.
fn meta_stage_is_valid_transition(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(from), ConstValue::Int(to)) => {
            let max = ctx.max_stage as i128;
            let valid = *from >= 0
                && *from <= max
                && *to >= 0
                && *to <= max
                && *to == *from - 1;
            Ok(ConstValue::Bool(valid))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int"),
            found: args[0].type_name(),
        }),
    }
}

/// Get the target stage for a quote expression
///
/// Returns current_stage - 1, or 0 if already at stage 0.
fn meta_stage_quote_target(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    let target = if ctx.current_stage > 0 {
        ctx.current_stage - 1
    } else {
        0
    };
    Ok(ConstValue::Int(target as i128))
}

/// Check if current stage can generate code for target stage
///
/// Returns true if from_stage > to_stage and both are valid.
fn meta_stage_can_generate(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(from), ConstValue::Int(to)) => {
            let max = ctx.max_stage as i128;
            let valid = *from >= 0
                && *from <= max
                && *to >= 0
                && *to <= max
                && *from > *to;
            Ok(ConstValue::Bool(valid))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Stage-Aware Generation
// ============================================================================

/// Generate a unique identifier scoped to the current stage
fn meta_stage_unique_ident(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(prefix) => {
            let ident = ctx.gen_unique_ident(prefix.as_str());
            // Include stage in the identifier for cross-stage hygiene
            let staged_ident = Text::from(format!("{}_s{}", ident, ctx.current_stage));
            Ok(ConstValue::Text(staged_ident))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get the current quote nesting depth
fn meta_stage_quote_depth(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.quote_depth as i128))
}

// ============================================================================
// Function Stage Information
// ============================================================================

/// Get the declared stage of a function by name
///
/// Returns the stage level, or -1 if the function is not found.
fn meta_stage_function_stage(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(name) => {
            match ctx.function_stages.get(name) {
                Some(stage) => Ok(ConstValue::Int(*stage as i128)),
                None => {
                    // Function not found - return -1 as sentinel
                    Ok(ConstValue::Int(-1))
                }
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get all functions declared at a specific stage level
fn meta_stage_functions_at(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Int(level) => {
            let target_stage = *level as u32;
            let functions: Vec<ConstValue> = ctx
                .function_stages
                .iter()
                .filter(|(_, stage)| **stage == target_stage)
                .map(|(name, _)| ConstValue::Text(name.clone()))
                .collect();
            Ok(ConstValue::Array(List::from(functions)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Configuration
// ============================================================================

/// Check if staged compilation is enabled
fn meta_stage_is_enabled(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Bool(ctx.staged_enabled))
}

/// Get stage-specific iteration limit
fn meta_stage_iteration_limit(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.iteration_limit as i128))
}

/// Get stage-specific recursion limit
fn meta_stage_recursion_limit(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.recursion_limit as i128))
}

/// Get stage-specific memory limit in bytes
fn meta_stage_memory_limit(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.memory_limit as i128))
}

// ============================================================================
// Tracing and Debugging
// ============================================================================

/// Get the generation chain leading to current execution
///
/// Returns a list of maps, each with keys: "stage", "function", "span".
fn meta_stage_generation_chain(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    let chain: Vec<ConstValue> = ctx
        .generation_chain
        .iter()
        .map(|record| {
            // Represent each StageRecord as a map
            let mut fields = OrderedMap::new();
            fields.insert(
                Text::from("stage"),
                ConstValue::Int(record.stage as i128),
            );
            fields.insert(
                Text::from("function"),
                ConstValue::Text(record.function.clone()),
            );
            ConstValue::Map(fields)
        })
        .collect();
    Ok(ConstValue::Array(List::from(chain)))
}

/// Add a trace marker for debugging staged compilation
///
/// Trace markers are collected by the compiler and can be displayed
/// with the `--trace-stages` CLI flag.
fn meta_stage_trace_marker(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(name), ConstValue::Text(data)) => {
            // Store as a map with name, data, and current stage
            let mut marker = OrderedMap::new();
            marker.insert(Text::from("name"), ConstValue::Text(name.clone()));
            marker.insert(Text::from("data"), ConstValue::Text(data.clone()));
            marker.insert(
                Text::from("stage"),
                ConstValue::Int(ctx.current_stage as i128),
            );
            ctx.trace_markers.push(ConstValue::Map(marker));
            Ok(ConstValue::Unit)
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage_current() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 2;
        let result = meta_stage_current(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(2));
    }

    #[test]
    fn test_stage_max() {
        let mut ctx = MetaContext::new();
        ctx.max_stage = 3;
        let result = meta_stage_max(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(3));
    }

    #[test]
    fn test_stage_is_runtime() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 0;
        let result = meta_stage_is_runtime(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        ctx.current_stage = 1;
        let result = meta_stage_is_runtime(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_stage_is_compile_time() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 0;
        let result = meta_stage_is_compile_time(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(false));

        ctx.current_stage = 1;
        let result = meta_stage_is_compile_time(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_stage_is_max_stage() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 3;
        ctx.max_stage = 3;
        let result = meta_stage_is_max_stage(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        ctx.current_stage = 1;
        let result = meta_stage_is_max_stage(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_stage_is_valid() {
        let mut ctx = MetaContext::new();
        ctx.max_stage = 3;

        let args = List::from(vec![ConstValue::Int(2)]);
        let result = meta_stage_is_valid(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        let args = List::from(vec![ConstValue::Int(4)]);
        let result = meta_stage_is_valid(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));

        let args = List::from(vec![ConstValue::Int(-1)]);
        let result = meta_stage_is_valid(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_stage_is_valid_transition() {
        let mut ctx = MetaContext::new();
        ctx.max_stage = 3;

        // Valid: stage 2 -> stage 1
        let args = List::from(vec![ConstValue::Int(2), ConstValue::Int(1)]);
        let result = meta_stage_is_valid_transition(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        // Invalid: stage 2 -> stage 0 (skips stage 1)
        let args = List::from(vec![ConstValue::Int(2), ConstValue::Int(0)]);
        let result = meta_stage_is_valid_transition(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));

        // Invalid: stage 0 -> stage 1 (going up)
        let args = List::from(vec![ConstValue::Int(0), ConstValue::Int(1)]);
        let result = meta_stage_is_valid_transition(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_stage_quote_target() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 2;
        let result = meta_stage_quote_target(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(1));

        ctx.current_stage = 0;
        let result = meta_stage_quote_target(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(0));
    }

    #[test]
    fn test_stage_can_generate() {
        let mut ctx = MetaContext::new();
        ctx.max_stage = 3;

        // Can generate from 2 to 1
        let args = List::from(vec![ConstValue::Int(2), ConstValue::Int(1)]);
        let result = meta_stage_can_generate(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        // Cannot generate from 1 to 2
        let args = List::from(vec![ConstValue::Int(1), ConstValue::Int(2)]);
        let result = meta_stage_can_generate(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));

        // Cannot generate same stage
        let args = List::from(vec![ConstValue::Int(2), ConstValue::Int(2)]);
        let result = meta_stage_can_generate(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_stage_unique_ident() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 1;

        let args = List::from(vec![ConstValue::Text(Text::from("temp"))]);
        let result = meta_stage_unique_ident(&mut ctx, args).unwrap();
        if let ConstValue::Text(ident) = &result {
            assert!(ident.as_str().starts_with("temp_"));
            assert!(ident.as_str().ends_with("_s1"));
        } else {
            panic!("Expected Text");
        }

        // Second call should produce a different identifier
        let args = List::from(vec![ConstValue::Text(Text::from("temp"))]);
        let result2 = meta_stage_unique_ident(&mut ctx, args).unwrap();
        assert_ne!(result, result2);
    }

    #[test]
    fn test_stage_quote_depth() {
        let mut ctx = MetaContext::new();
        ctx.quote_depth = 2;
        let result = meta_stage_quote_depth(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(2));
    }

    #[test]
    fn test_stage_function_stage() {
        let mut ctx = MetaContext::new();
        ctx.function_stages
            .insert(Text::from("my_module.helper"), 2);

        let args = List::from(vec![ConstValue::Text(Text::from("my_module.helper"))]);
        let result = meta_stage_function_stage(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(2));

        // Unknown function returns -1
        let args = List::from(vec![ConstValue::Text(Text::from("unknown"))]);
        let result = meta_stage_function_stage(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(-1));
    }

    #[test]
    fn test_stage_functions_at() {
        let mut ctx = MetaContext::new();
        ctx.function_stages.insert(Text::from("fn_a"), 1);
        ctx.function_stages.insert(Text::from("fn_b"), 1);
        ctx.function_stages.insert(Text::from("fn_c"), 2);

        let args = List::from(vec![ConstValue::Int(1)]);
        let result = meta_stage_functions_at(&mut ctx, args).unwrap();
        if let ConstValue::Array(list) = result {
            assert_eq!(list.len(), 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_stage_is_enabled() {
        let mut ctx = MetaContext::new();
        ctx.staged_enabled = true;
        let result = meta_stage_is_enabled(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        ctx.staged_enabled = false;
        let result = meta_stage_is_enabled(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_stage_limits() {
        let mut ctx = MetaContext::new();
        ctx.iteration_limit = 500_000;
        ctx.recursion_limit = 100;
        ctx.memory_limit = 50 * 1024 * 1024;

        let result = meta_stage_iteration_limit(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(500_000));

        let result = meta_stage_recursion_limit(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(100));

        let result = meta_stage_memory_limit(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(50 * 1024 * 1024));
    }

    #[test]
    fn test_stage_generation_chain() {
        use verum_ast::Span;
        use crate::meta::StageRecord;

        let mut ctx = MetaContext::new();
        ctx.generation_chain.push(StageRecord::new(
            2,
            Text::from("outer_gen"),
            Span::dummy(),
        ));
        ctx.generation_chain.push(StageRecord::new(
            1,
            Text::from("inner_gen"),
            Span::dummy(),
        ));

        let result = meta_stage_generation_chain(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(chain) = result {
            assert_eq!(chain.len(), 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_stage_trace_marker() {
        let mut ctx = MetaContext::new();
        ctx.current_stage = 2;

        let args = List::from(vec![
            ConstValue::Text(Text::from("checkpoint")),
            ConstValue::Text(Text::from("reached phase 3")),
        ]);
        let result = meta_stage_trace_marker(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Unit);
        assert_eq!(ctx.trace_markers.len(), 1);
    }
}
