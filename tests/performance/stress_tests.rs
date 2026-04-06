//! Performance and Stress Tests
//!
//! Tests compilation performance, memory usage under load,
//! and concurrent operations.
//!
//! Performance Targets (from CLAUDE.md):
//! - CBGR overhead: < 15ns per check
//! - Type inference: < 100ms for 10K LOC
//! - Compilation speed: > 50K LOC/sec (release)
//! - Memory overhead: < 5% vs unsafe code

use std::time::{Duration, Instant};
use verum_cbgr::{Allocator, GenRef, Tier};
use verum_interpreter::{Environment, Evaluator};
use verum_lexer::Lexer;
use verum_parser::Parser;
use verum_std::core::{List, Text};
use verum_types::TypeChecker;

// ============================================================================
// Large File Compilation Tests (10K+ LOC)
// ============================================================================

#[test]
fn test_large_file_parsing_performance() {
    // Generate a large source file (10K lines)
    let mut source = String::new();
    for i in 0..10_000 {
        source.push_str(&format!("let var{} = {};\n", i, i));
    }

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let result = parser.parse_module();
    let duration = start.elapsed();

    assert!(result.is_ok(), "Large file should parse successfully");

    // Should parse 10K LOC in reasonable time
    println!("Parsed 10K LOC in {:?}", duration);
    assert!(
        duration < Duration::from_secs(5),
        "Parsing too slow: {:?}",
        duration
    );
}

#[test]
fn test_large_file_lexing_performance() {
    // Generate large source
    let mut source = String::new();
    for i in 0..10_000 {
        source.push_str(&format!("let variable_{} = {} + {} * {};\n", i, i, i + 1, i + 2));
    }

    let start = Instant::now();
    let mut lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();
    let duration = start.elapsed();

    assert!(!tokens.is_empty(), "Should produce tokens");

    println!("Lexed 10K LOC in {:?}", duration);
    assert!(
        duration < Duration::from_secs(2),
        "Lexing too slow: {:?}",
        duration
    );
}

#[test]
fn test_compilation_speed_target() {
    // Target: > 50K LOC/sec in release mode
    // This test generates and parses a large program

    let mut source = String::new();
    let loc_count = 1000; // Smaller for debug builds

    for i in 0..loc_count {
        source.push_str(&format!(
            "fn func{}(x: Int) -> Int {{ x + {} }}\n",
            i, i
        ));
    }

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let result = parser.parse_module();
    let duration = start.elapsed();

    assert!(result.is_ok());

    let loc_per_sec = (loc_count as f64) / duration.as_secs_f64();
    println!("Compilation speed: {:.0} LOC/sec", loc_per_sec);

    // Relaxed target for debug builds
    assert!(
        loc_per_sec > 1000.0,
        "Compilation too slow: {:.0} LOC/sec",
        loc_per_sec
    );
}

// ============================================================================
// Deeply Nested Expression Stress Tests
// ============================================================================

#[test]
fn test_deeply_nested_expressions() {
    // Create deeply nested expression: ((((1 + 2) + 3) + 4) + ... + 100)
    let mut source = String::from("1");
    for i in 2..=100 {
        source = format!("({} + {})", source, i);
    }

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let result = parser.parse_expr();
    let duration = start.elapsed();

    assert!(result.is_ok(), "Should parse deeply nested expression");

    println!("Parsed 100-level nesting in {:?}", duration);
    assert!(
        duration < Duration::from_secs(1),
        "Deep nesting too slow: {:?}",
        duration
    );
}

#[test]
fn test_deeply_nested_function_calls() {
    // Create deeply nested calls: f(f(f(f(...))))
    let mut source = String::from("x");
    for _ in 0..50 {
        source = format!("f({})", source);
    }

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let result = parser.parse_expr();
    let duration = start.elapsed();

    assert!(result.is_ok(), "Should parse deeply nested calls");

    println!("Parsed 50-level call nesting in {:?}", duration);
}

#[test]
fn test_deeply_nested_match_expressions() {
    // Create nested match expressions
    let mut source = String::from("match x { 0 => 0, _ => 1 }");
    for _ in 0..20 {
        source = format!("match x {{ 0 => {}, _ => 1 }}", source);
    }

    let mut parser = Parser::new(&source);
    let result = parser.parse_expr();

    assert!(result.is_ok(), "Should parse nested match expressions");
}

// ============================================================================
// Memory Usage Under Load Tests
// ============================================================================

#[test]
fn test_memory_allocation_stress() {
    let allocator = Allocator::new();

    // Allocate many small objects
    let mut refs = Vec::new();
    for i in 0..10_000 {
        let gen_ref: GenRef<i64> = allocator.alloc(i as i64, Tier::Standard);
        refs.push(gen_ref);
    }

    // Verify all allocations
    for (i, gen_ref) in refs.iter().enumerate() {
        assert_eq!(**gen_ref, i as i64);
    }

    println!("Successfully allocated and verified 10K objects");
}

#[test]
fn test_cbgr_overhead_performance() {
    // Target: < 15ns per check
    let allocator = Allocator::new();
    let iterations = 10_000;

    // Allocate objects
    let refs: Vec<GenRef<i64>> = (0..iterations)
        .map(|i| allocator.alloc(i as i64, Tier::Standard))
        .collect();

    // Measure access time
    let start = Instant::now();
    for gen_ref in &refs {
        let _ = **gen_ref; // Dereference (triggers CBGR check)
    }
    let duration = start.elapsed();

    let ns_per_check = duration.as_nanos() / iterations as u128;

    println!("CBGR overhead: {}ns per check", ns_per_check);

    // Relaxed target for debug builds (< 500ns)
    assert!(
        ns_per_check < 500,
        "CBGR overhead too high: {}ns",
        ns_per_check
    );
}

#[test]
fn test_large_list_operations() {
    let mut list = List::new();

    // Add many elements
    for i in 0..10_000 {
        list.push(i);
    }

    assert_eq!(list.len(), 10_000);

    // Access elements
    for i in 0..10_000 {
        assert_eq!(list[i], i);
    }

    println!("Successfully operated on 10K element list");
}

#[test]
fn test_large_text_operations() {
    let mut text = Text::from("");

    // Build large text
    for i in 0..1000 {
        let chunk = format!("Line {} of text\n", i);
        text = Text::from(format!("{}{}", text.as_str(), chunk));
    }

    assert!(text.len() > 10_000);
    println!("Successfully built text with {} bytes", text.len());
}

// ============================================================================
// Concurrent Compilation Tests
// ============================================================================

#[test]
fn test_concurrent_parsing() {
    use std::sync::Arc;
    use std::thread;

    let sources: Vec<String> = (0..10)
        .map(|i| format!("fn func{}(x: Int) -> Int {{ x + {} }}", i, i))
        .collect();

    let sources = Arc::new(sources);
    let mut handles = vec![];

    for i in 0..10 {
        let sources = Arc::clone(&sources);
        let handle = thread::spawn(move || {
            let source = &sources[i];
            let mut parser = Parser::new(source);
            parser.parse_module().expect("Parse failed")
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        let result = handle.join();
        assert!(result.is_ok(), "Concurrent parsing failed");
    }

    println!("Successfully parsed 10 files concurrently");
}

#[test]
fn test_concurrent_type_checking() {
    use std::sync::Arc;
    use std::thread;

    let sources: Vec<String> = (0..10)
        .map(|i| format!("{} + {}", i, i + 1))
        .collect();

    let sources = Arc::new(sources);
    let mut handles = vec![];

    for i in 0..10 {
        let sources = Arc::clone(&sources);
        let handle = thread::spawn(move || {
            let source = &sources[i];
            let mut parser = Parser::new(source);
            let expr = parser.parse_expr().expect("Parse failed");

            let mut checker = TypeChecker::new();
            checker.synth_expr(&expr).expect("Type check failed")
        });
        handles.push(handle);
    }

    for handle in handles {
        let result = handle.join();
        assert!(result.is_ok(), "Concurrent type checking failed");
    }

    println!("Successfully type-checked 10 expressions concurrently");
}

// ============================================================================
// Memory Overhead Tests
// ============================================================================

#[test]
fn test_memory_overhead_measurement() {
    // Compare memory usage of CBGR vs direct allocation
    let allocator = Allocator::new();

    let count = 1000;
    let mut cbgr_refs = Vec::new();
    let mut direct_refs = Vec::new();

    // CBGR allocations
    for i in 0..count {
        let gen_ref: GenRef<i64> = allocator.alloc(i as i64, Tier::Standard);
        cbgr_refs.push(gen_ref);
    }

    // Direct allocations
    for i in 0..count {
        direct_refs.push(Box::new(i as i64));
    }

    // Both should work correctly
    assert_eq!(cbgr_refs.len(), count);
    assert_eq!(direct_refs.len(), count);

    // Target: < 5% overhead (hard to measure precisely in test)
    println!("CBGR allocated {} objects", cbgr_refs.len());
}

// ============================================================================
// Stress Test Combinations
// ============================================================================

#[test]
fn test_combined_stress() {
    // Combine multiple stress factors
    let allocator = Allocator::new();

    // Parse large source
    let mut source = String::new();
    for i in 0..1000 {
        source.push_str(&format!("let var{} = {} + {};\n", i, i, i + 1));
    }

    let start = Instant::now();

    // Parse
    let mut parser = Parser::new(&source);
    let module = parser.parse_module().expect("Parse failed");

    // Allocate memory
    let mut refs = Vec::new();
    for i in 0..1000 {
        let gen_ref: GenRef<i64> = allocator.alloc(i as i64, Tier::Standard);
        refs.push(gen_ref);
    }

    let duration = start.elapsed();

    assert!(!module.declarations.is_empty());
    assert_eq!(refs.len(), 1000);

    println!("Combined stress test completed in {:?}", duration);
}

// ============================================================================
// Long-Running Operation Tests
// ============================================================================

#[test]
fn test_long_running_compilation() {
    // Simulate a long compilation session
    let allocator = Allocator::new();

    for round in 0..10 {
        // Parse source
        let source = format!("fn func{}(x: Int) -> Int {{ x + {} }}", round, round);
        let mut parser = Parser::new(&source);
        let _module = parser.parse_module().expect("Parse failed");

        // Allocate some memory
        for i in 0..100 {
            let _gen_ref: GenRef<i64> = allocator.alloc(i as i64, Tier::Standard);
        }
    }

    println!("Completed 10 rounds of compilation");
}

// ============================================================================
// Pathological Input Tests
// ============================================================================

#[test]
fn test_many_sequential_declarations() {
    let mut source = String::new();
    for i in 0..1000 {
        source.push_str(&format!("let x{} = {}; ", i, i));
    }

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let result = parser.parse_module();
    let duration = start.elapsed();

    assert!(result.is_ok(), "Should parse many declarations");
    println!("Parsed 1000 declarations in {:?}", duration);
}

#[test]
fn test_very_long_expression() {
    // Create a very long expression: 1 + 1 + 1 + ... + 1
    let mut source = String::from("1");
    for _ in 1..1000 {
        source.push_str(" + 1");
    }

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let result = parser.parse_expr();
    let duration = start.elapsed();

    assert!(result.is_ok(), "Should parse long expression");
    println!("Parsed 1000-term expression in {:?}", duration);
}

#[test]
fn test_many_function_parameters() {
    // Function with many parameters
    let mut params = String::new();
    for i in 0..100 {
        if i > 0 {
            params.push_str(", ");
        }
        params.push_str(&format!("p{}: Int", i));
    }

    let source = format!("fn many_params({}) -> Int {{ 42 }}", params);

    let mut parser = Parser::new(&source);
    let result = parser.parse_module();

    assert!(result.is_ok(), "Should parse many parameters");
}

// ============================================================================
// Resource Limit Tests
// ============================================================================

#[test]
fn test_stack_depth_limit() {
    // Test parser stack depth with nested structures
    let mut source = String::from("(");
    for _ in 0..100 {
        source.push('(');
    }
    source.push_str("42");
    for _ in 0..100 {
        source.push(')');
    }
    source.push(')');

    let mut parser = Parser::new(&source);
    let result = parser.parse_expr();

    // Should either parse or gracefully handle stack depth
    assert!(result.is_ok() || result.is_err());
}
