# Verum Compiler - Integration Test Suite

## Quick Start

This directory contains end-to-end integration tests for the complete Verum compilation pipeline.

## Test Files

### e2e_pipeline.rs
Complete pipeline tests from parsing through execution.

```bash
cargo test --test integration::e2e_pipeline
```

**Coverage:**
- Basic compilation (parse → typecheck → codegen)
- JIT and AOT modes
- Error handling
- Complex features (tuples, lists, pattern matching)
- Performance validation

### cbgr_integration.rs
CBGR memory safety and performance tests.

```bash
cargo test --test integration::cbgr_integration
```

**Coverage:**
- CBGR allocation and deallocation
- Generation tracking
- Three-tier reference system
- Memory safety guarantees
- Performance overhead (target: < 15ns per check)

### refinements.rs
Refinement type system integration.

```bash
cargo test --test integration::refinements
```

**Coverage:**
- Runtime constraint validation
- Compile-time verification
- SMT solver integration
- Type composition
- Real-world examples (division by zero, buffer overflow)

### references.rs
Three-tier reference system tests.

```bash
cargo test --test integration::references
```

**Coverage:**
- &T (managed references, ~15ns overhead)
- &checked T (runtime bounds checking)
- &unsafe T (zero-cost)
- Performance comparison
- Gradual typing

### performance.rs
Execution tier performance comparison.

```bash
cargo test --test integration::performance
```

**Coverage:**
- Interpreter baseline
- JIT compilation and execution
- AOT compilation
- Performance targets validation
- Graceful fallback

## Running Tests

### All Integration Tests
```bash
cargo test --package verum_compiler --tests
```

### Specific Test
```bash
cargo test --package verum_compiler --test cbgr_integration::test_cbgr_basic_allocation
```

### With Output
```bash
cargo test --package verum_compiler --tests -- --nocapture
```

### Ignore Failures (Continue Testing)
```bash
cargo test --package verum_compiler --tests --no-fail-fast
```

## Writing New Tests

### Structure
```rust
#[test]
fn test_my_feature() {
    // Arrange: Set up test data
    let source = "fn test() -> Int { 42 }";

    // Act: Execute the code under test
    let mut parser = Parser::new(source);
    let module = parser.parse_module();

    // Assert: Verify the result
    assert!(module.is_ok());
}
```

### Helper Functions
Use the helper functions defined in each file:
- `parse_source()` - Parse a string into a Module
- `typecheck_module()` - Type check a Module
- `interpret_expr()` - Evaluate an expression
- `create_test_session()` - Create a compiler session

### Best Practices
1. **One assertion per test** (when possible)
2. **Clear test names** describing what is tested
3. **Document expected behavior** in comments
4. **Use `expect()` with descriptive messages** instead of `unwrap()`
5. **Clean up resources** (use TempDir for file operations)

## Performance Testing

Integration tests should complete quickly:
- Simple tests: < 100ms
- Complex tests: < 1s
- Stress tests: < 10s

For longer-running performance tests, use benchmarks instead.

## Debugging Failed Tests

### Print Diagnostics
```rust
if let Err(e) = result {
    eprintln!("Error: {}", e);
    session.display_diagnostics();
}
```

### Enable Logging
```bash
RUST_LOG=debug cargo test --test cbgr_integration
```

### Run Single Test with Backtrace
```bash
RUST_BACKTRACE=1 cargo test --test references::test_tier0_basic_reference
```

## Test Categories

### Correctness Tests
Verify that the implementation matches the specification.

### Safety Tests
Ensure memory safety and prevent undefined behavior.

### Performance Tests
Validate that performance targets are met.

### Error Handling Tests
Verify graceful error reporting.

### Edge Case Tests
Test boundary conditions and unusual inputs.

## Dependencies

Tests depend on:
- `verum_lexer` - Lexical analysis
- `verum_parser` - Parsing
- `verum_types` - Type checking
- `verum_interpreter` - Interpretation
- `verum_cbgr` - Memory management
- `verum_codegen` - Code generation
- `tempfile` - Temporary file handling

## Coverage Goals

Target: **95%+ line coverage** for integration tests

Use cargo-tarpaulin for coverage reports:
```bash
cargo tarpaulin --package verum_compiler --tests
```

## Continuous Integration

These tests are run in CI on:
- Every commit
- Every pull request
- Before release

CI should:
1. Run all integration tests
2. Fail on any test failure
3. Report coverage metrics
4. Generate performance reports

## Related Documentation

- [INTEGRATION_TEST_SUITE_REPORT.md](../INTEGRATION_TEST_SUITE_REPORT.md) - Full test suite report
- [CLAUDE.md](../../../CLAUDE.md) - Development standards
- [docs/detailed/06-compilation-pipeline.md](../../../docs/detailed/06-compilation-pipeline.md) - Pipeline specification

## Contact

For questions or issues with the test suite, see the main project README or open an issue.
