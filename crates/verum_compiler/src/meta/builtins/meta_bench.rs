//! Meta Benchmarking Intrinsics (Tier 1 - Requires MetaBench)
//!
//! Provides compile-time benchmarking and profiling for meta functions.
//! All functions require the `MetaBench` context.
//!
//! ## Timing
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `bench_start(name)` | `(Text) -> Int` | Start timer, return nanos |
//! | `bench_now_ns()` | `() -> Int` | Current monotonic nanos |
//! | `bench_report(name, ns)` | `(Text, Int) -> ()` | Store benchmark result |
//!
//! ## Memory
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `bench_memory_usage()` | `() -> Int` | Current memory usage (bytes) |
//! | `bench_peak_memory()` | `() -> Int` | Peak memory usage (bytes) |
//!
//! ## Counters
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `bench_count(name)` | `(Text) -> ()` | Increment counter by 1 |
//! | `bench_count_by(name, n)` | `(Text, Int) -> ()` | Increment counter by N |
//! | `bench_get_count(name)` | `(Text) -> Int` | Read counter value |
//!
//! ## Results
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `bench_all_results()` | `() -> List<(Text, List<BenchResult>)>` | All results |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [MetaBench]` context.

use std::time::Instant;

use verum_common::{Heap, List, Maybe, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};
use crate::meta::BenchResult;

/// Monotonic reference point for bench_now_ns / bench_start
///
/// Uses `std::time::Instant::now()` at first call to establish a baseline.
/// All subsequent calls compute elapsed nanos from this point.
fn monotonic_nanos() -> i128 {
    use std::sync::OnceLock;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_nanos() as i128
}

/// Register meta bench builtins with context requirements
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Timing (Tier 1 - MetaBench)
    // ========================================================================

    map.insert(
        Text::from("bench_start"),
        BuiltinInfo::meta_bench(
            meta_bench_start,
            "Start a named benchmark timer; returns current monotonic nanos",
            "(Text) -> Int",
        ),
    );
    map.insert(
        Text::from("bench_now_ns"),
        BuiltinInfo::meta_bench(
            meta_bench_now_ns,
            "Get current monotonic time in nanoseconds",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("bench_report"),
        BuiltinInfo::meta_bench(
            meta_bench_report,
            "Store a benchmark result with name and duration in nanoseconds",
            "(Text, Int) -> ()",
        ),
    );

    // ========================================================================
    // Memory (Tier 1 - MetaBench)
    // ========================================================================

    map.insert(
        Text::from("bench_memory_usage"),
        BuiltinInfo::meta_bench(
            meta_bench_memory_usage,
            "Get current estimated memory usage in bytes",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("bench_peak_memory"),
        BuiltinInfo::meta_bench(
            meta_bench_peak_memory,
            "Get peak memory usage in bytes",
            "() -> Int",
        ),
    );

    // ========================================================================
    // Counters (Tier 1 - MetaBench)
    // ========================================================================

    map.insert(
        Text::from("bench_count"),
        BuiltinInfo::meta_bench(
            meta_bench_count,
            "Increment a named counter by 1",
            "(Text) -> ()",
        ),
    );
    map.insert(
        Text::from("bench_count_by"),
        BuiltinInfo::meta_bench(
            meta_bench_count_by,
            "Increment a named counter by a specified amount",
            "(Text, Int) -> ()",
        ),
    );
    map.insert(
        Text::from("bench_get_count"),
        BuiltinInfo::meta_bench(
            meta_bench_get_count,
            "Read the current value of a named counter",
            "(Text) -> Int",
        ),
    );

    // ========================================================================
    // Results (Tier 1 - MetaBench)
    // ========================================================================

    map.insert(
        Text::from("bench_all_results"),
        BuiltinInfo::meta_bench(
            meta_bench_all_results,
            "Get all stored benchmark results grouped by name",
            "() -> List<(Text, List<BenchResult>)>",
        ),
    );
}

// ============================================================================
// Timing
// ============================================================================

/// Start a named benchmark timer; returns current monotonic nanos
fn meta_bench_start(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(_name) => {
            // Return the current monotonic timestamp so the caller can compute
            // elapsed time later with `bench_now_ns() - start`.
            let now = monotonic_nanos();
            Ok(ConstValue::Int(now))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get current monotonic time in nanoseconds
fn meta_bench_now_ns(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }
    Ok(ConstValue::Int(monotonic_nanos()))
}

/// Store a benchmark result with name and duration in nanoseconds
fn meta_bench_report(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    let name = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            });
        }
    };

    let duration_ns: u64 = match &args[1] {
        ConstValue::Int(n) => (*n).max(0) as u64,
        ConstValue::UInt(n) => *n as u64,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[1].type_name(),
            });
        }
    };

    let result = BenchResult::new(duration_ns);
    ctx.bench_results
        .entry(name)
        .or_insert_with(List::new)
        .push(result);

    Ok(ConstValue::Unit)
}

// ============================================================================
// Memory
// ============================================================================

/// Get current estimated memory usage in bytes
///
/// This returns the tracked `memory_used` field from the meta context.
/// For a more accurate measurement, the compiler pipeline should update
/// this field periodically.
fn meta_bench_memory_usage(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    // Try to get a rough estimate from the allocator if the tracked value is 0
    let usage = if ctx.memory_used == 0 {
        // Fallback: estimate from context size (bindings + type definitions)
        let binding_estimate = ctx.bindings.len() as u64 * 64;
        let type_estimate = ctx.type_definitions.len() as u64 * 256;
        let bench_estimate = ctx.bench_results.len() as u64 * 32;
        binding_estimate + type_estimate + bench_estimate
    } else {
        ctx.memory_used
    };

    Ok(ConstValue::Int(usage as i128))
}

/// Get peak memory usage in bytes
fn meta_bench_peak_memory(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    // Update peak if current exceeds it
    if ctx.memory_used > ctx.peak_memory {
        ctx.peak_memory = ctx.memory_used;
    }

    Ok(ConstValue::Int(ctx.peak_memory as i128))
}

// ============================================================================
// Counters
// ============================================================================

/// Increment a named counter by 1
fn meta_bench_count(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(name) => {
            let counter = ctx.counters.entry(name.clone()).or_insert(0);
            *counter += 1;
            Ok(ConstValue::Unit)
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Increment a named counter by a specified amount
fn meta_bench_count_by(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    let name = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            });
        }
    };

    let amount: u64 = match &args[1] {
        ConstValue::Int(n) => (*n).max(0) as u64,
        ConstValue::UInt(n) => *n as u64,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[1].type_name(),
            });
        }
    };

    let counter = ctx.counters.entry(name).or_insert(0);
    *counter += amount;

    Ok(ConstValue::Unit)
}

/// Read the current value of a named counter
fn meta_bench_get_count(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(name) => {
            let count = ctx.counters.get(name).copied().unwrap_or(0);
            Ok(ConstValue::Int(count as i128))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Results
// ============================================================================

/// Get all stored benchmark results grouped by name
fn meta_bench_all_results(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    let results: List<ConstValue> = ctx
        .bench_results
        .iter()
        .map(|(name, bench_list)| {
            let entries: List<ConstValue> = bench_list
                .iter()
                .map(|br| {
                    let context_val = match &br.context {
                        Some(c) => ConstValue::Maybe(Maybe::Some(Heap::new(
                            ConstValue::Text(c.clone()),
                        ))),
                        None => ConstValue::Maybe(Maybe::None),
                    };
                    ConstValue::Tuple(List::from(vec![
                        ConstValue::Int(br.duration_ns as i128),
                        context_val,
                    ]))
                })
                .collect();
            ConstValue::Tuple(List::from(vec![
                ConstValue::Text(name.clone()),
                ConstValue::Array(entries),
            ]))
        })
        .collect();

    Ok(ConstValue::Array(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context() -> MetaContext {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts
            .enable(super::super::context_requirements::RequiredContext::MetaBench);
        ctx
    }

    #[test]
    fn test_bench_start_returns_nanos() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("my_bench"))]);
        let result = meta_bench_start(&mut ctx, args).unwrap();
        if let ConstValue::Int(ns) = result {
            assert!(ns >= 0);
        } else {
            panic!("Expected Int");
        }
    }

    #[test]
    fn test_bench_now_ns_monotonic() {
        let mut ctx = create_test_context();
        let r1 = meta_bench_now_ns(&mut ctx, List::new()).unwrap();
        let r2 = meta_bench_now_ns(&mut ctx, List::new()).unwrap();
        if let (ConstValue::Int(t1), ConstValue::Int(t2)) = (r1, r2) {
            assert!(t2 >= t1, "monotonic time should not go backwards");
        } else {
            panic!("Expected Int values");
        }
    }

    #[test]
    fn test_bench_report_and_all_results() {
        let mut ctx = create_test_context();

        // Report a benchmark
        let args = List::from(vec![
            ConstValue::Text(Text::from("parse")),
            ConstValue::Int(42_000i128),
        ]);
        meta_bench_report(&mut ctx, args).unwrap();

        // Report another
        let args = List::from(vec![
            ConstValue::Text(Text::from("parse")),
            ConstValue::Int(43_000i128),
        ]);
        meta_bench_report(&mut ctx, args).unwrap();

        // Get all results
        let result = meta_bench_all_results(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(groups) = result {
            assert_eq!(groups.len(), 1); // One group: "parse"
            if let ConstValue::Tuple(fields) = &groups[0] {
                assert_eq!(fields[0], ConstValue::Text(Text::from("parse")));
                if let ConstValue::Array(entries) = &fields[1] {
                    assert_eq!(entries.len(), 2);
                } else {
                    panic!("Expected Array for entries");
                }
            } else {
                panic!("Expected Tuple");
            }
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_bench_count() {
        let mut ctx = create_test_context();

        let name = Text::from("expansions");

        // Count once
        let args = List::from(vec![ConstValue::Text(name.clone())]);
        meta_bench_count(&mut ctx, args).unwrap();

        // Count by 5
        let args = List::from(vec![
            ConstValue::Text(name.clone()),
            ConstValue::Int(5i128),
        ]);
        meta_bench_count_by(&mut ctx, args).unwrap();

        // Get count (should be 6)
        let args = List::from(vec![ConstValue::Text(name)]);
        let result = meta_bench_get_count(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(6i128));
    }

    #[test]
    fn test_bench_get_count_missing() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent"))]);
        let result = meta_bench_get_count(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(0i128));
    }

    #[test]
    fn test_bench_memory_usage() {
        let mut ctx = create_test_context();

        // With memory_used = 0, should return an estimate
        let result = meta_bench_memory_usage(&mut ctx, List::new()).unwrap();
        if let ConstValue::Int(usage) = result {
            assert!(usage >= 0);
        } else {
            panic!("Expected Int");
        }

        // Set explicit memory usage
        ctx.memory_used = 1024;
        let result = meta_bench_memory_usage(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(1024i128));
    }

    #[test]
    fn test_bench_peak_memory() {
        let mut ctx = create_test_context();
        ctx.peak_memory = 2048;

        let result = meta_bench_peak_memory(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Int(2048i128));
    }

    #[test]
    fn test_bench_start_wrong_arity() {
        let mut ctx = create_test_context();
        let result = meta_bench_start(&mut ctx, List::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_bench_report_wrong_types() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Int(42i128), ConstValue::Int(100i128)]);
        let result = meta_bench_report(&mut ctx, args);
        assert!(result.is_err());
    }

    #[test]
    fn test_bench_count_wrong_type() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Int(42i128)]);
        let result = meta_bench_count(&mut ctx, args);
        assert!(result.is_err());
    }
}
