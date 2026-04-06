# Integration Tests - Quick Start Guide

## Running Tests

### Run All Integration Tests
```bash
cargo test --package verum_integration_tests
```

### Run Specific Test Suite
```bash
# End-to-end pipeline tests
cargo test --package verum_integration_tests --test end_to_end_tests

# Cross-crate integration
cargo test --package verum_integration_tests --test module_integration_tests

# Error handling
cargo test --package verum_integration_tests --test error_propagation_tests

# Performance/stress tests (use --release for accurate results)
cargo test --package verum_integration_tests --test stress_tests --release

# Compatibility tests
cargo test --package verum_integration_tests --test interop_tests

# Regression tests
cargo test --package verum_integration_tests --test bug_fixes_tests
```

### Run Specific Test
```bash
cargo test --package verum_integration_tests test_e2e_simple_arithmetic
```

### Show Test Output
```bash
cargo test --package verum_integration_tests -- --nocapture
```

### Use Integration Test Runner
```bash
# Basic run
./tests/integration_runner.sh

# With verbose output
./tests/integration_runner.sh --verbose

# With coverage report
./tests/integration_runner.sh --coverage

# With performance baseline comparison
./tests/integration_runner.sh --baseline baseline_20251112
```

## Test Organization

### End-to-End Tests (50 tests)
- Basic arithmetic and logic
- Function definitions and recursion
- Pattern matching
- Collections (lists, tuples)
- Control flow (if, match)
- CBGR memory management
- Error detection

### Cross-Crate Tests (30 tests)
- Lexer → Parser integration
- Parser → Type Checker integration
- Type Checker → Interpreter integration
- Full pipeline verification
- Standard library integration

### Error Handling Tests (35 tests)
- Lexer error detection
- Parser error recovery
- Type error reporting
- Interpreter error handling
- Diagnostic quality

### Performance Tests (25 tests)
- Large file compilation (10K+ LOC)
- Deep nesting stress tests
- Memory allocation stress
- CBGR overhead measurement
- Concurrent operations

### Compatibility Tests (30 tests)
- JSON serialization/deserialization
- File I/O operations
- Path handling
- Unicode support
- Platform compatibility

### Regression Tests (40 tests)
- Parser bug fixes
- Lexer bug fixes
- Type checker bug fixes
- Interpreter bug fixes
- Specification edge cases
- Fuzzing corner cases

## Quick Examples

### Test the Full Pipeline
```rust
#[test]
fn test_full_pipeline() {
    let source = "2 + 3";

    // Lex → Parse → Type Check → Evaluate
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().unwrap();

    let mut checker = TypeChecker::new();
    let typed = checker.synth_expr(&expr).unwrap();

    let mut env = Environment::new();
    let mut eval = Evaluator::new();
    let result = eval.eval_expr(&expr, &mut env).unwrap();

    assert_eq!(result, Value::Int(5));
}
```

### Test Error Handling
```rust
#[test]
fn test_type_error() {
    let source = "true + 42";

    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().unwrap();

    let mut checker = TypeChecker::new();
    let result = checker.synth_expr(&expr);

    assert!(result.is_err()); // Should detect type error
}
```

### Test Performance
```rust
#[test]
fn test_performance() {
    let start = Instant::now();

    // Your performance-critical code here

    let duration = start.elapsed();
    assert!(duration < Duration::from_secs(1));
}
```

## Understanding Test Results

### Successful Test
```
test test_e2e_simple_arithmetic ... ok
```

### Failed Test
```
test test_e2e_simple_arithmetic ... FAILED

failures:

---- test_e2e_simple_arithmetic stdout ----
thread 'test_e2e_simple_arithmetic' panicked at 'assertion failed: ...'
```

### Performance Metrics
```
CBGR overhead: 250ns per check
Parsed 10K LOC in 3.5s
Compilation speed: 2857 LOC/sec
```

## Troubleshooting

### Test Won't Compile
1. Check that all dependencies are in Cargo.toml
2. Run `cargo clean` and rebuild
3. Check for version mismatches

### Test Fails in Debug but Passes in Release
- This is expected for performance tests
- Always run performance tests with `--release`

### Test Hangs
- Check for infinite loops
- Check for deadlocks in concurrent tests
- Add timeout with `--test-threads=1`

### Test is Flaky
- Check for race conditions
- Check for uninitialized state
- Add proper synchronization

## Adding New Tests

1. Choose the appropriate test file
2. Follow the existing test pattern
3. Document what the test verifies
4. Ensure test is deterministic
5. Run test in both debug and release modes

## CI/CD Integration

Tests run automatically on:
- Every commit to main
- Every pull request
- Nightly builds
- Pre-release validation

## Performance Baselines

| Metric | Debug | Release Target |
|--------|-------|----------------|
| CBGR overhead | ~250ns | < 15ns |
| Parse 10K LOC | < 5s | < 100ms |
| Compile speed | > 1K LOC/s | > 50K LOC/s |

## Getting Help

- See full documentation in `README.md`
- See detailed report in `/INTEGRATION_TEST_REPORT.md`
- Check existing tests for examples
- Ask in team chat or create an issue
