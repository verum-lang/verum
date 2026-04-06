# Verum Integration Tests

Comprehensive integration testing suite for the Verum language platform.

## Overview

This crate contains end-to-end integration tests that verify the entire Verum compilation and execution pipeline works correctly. These tests complement the unit tests in individual crates by testing how all components work together.

## Test Suites

### 1. End-to-End Compilation Tests (`tests/end_to_end_tests.rs`)

Tests the complete compilation pipeline from source code to execution:

- **Basic Arithmetic and Logic**: Integer/float operations, boolean logic, comparisons
- **Function Definitions**: Simple functions, recursive functions (factorial, fibonacci)
- **Pattern Matching**: Literal patterns, tuple patterns, list patterns
- **Variable Bindings**: Let bindings, shadowing
- **Collections**: List and tuple creation/manipulation
- **Control Flow**: If expressions, nested conditionals
- **CBGR Memory Management**: All three tiers (Standard, Checked, Unsafe)
- **Standard Library Usage**: List, Text, Map, Set operations
- **Complex Programs**: Multiple functions, type annotations
- **Error Cases**: Parse errors, type errors
- **Edge Cases**: Empty modules, whitespace-only, comments-only
- **Large Programs**: 100+ functions, deeply nested expressions
- **Deep Nesting**: 100+ levels of parentheses

**Coverage**: ~50 tests covering the full pipeline

### 2. Cross-Crate Integration Tests (`tests/module_integration_tests.rs`)

Verifies all crates work together correctly:

- **Lexer → Parser**: Token generation and consumption
- **Parser → AST**: AST structure verification
- **Parser → Type Checker**: Type checking integration
- **Type Checker → Interpreter**: Execution after type checking
- **AST → Resolver**: Symbol resolution
- **CBGR → Runtime**: Memory management integration
- **Context System**: Dependency injection across crates
- **Diagnostics**: Error reporting from all pipeline stages
- **Standard Library**: Library integration with interpreter
- **Runtime Thread Pool**: Concurrent execution
- **Full Pipeline**: Source → Lexer → Parser → TypeChecker → Interpreter
- **Re-exports**: Verify all public re-exports work
- **Version Compatibility**: Cross-crate version consistency
- **Data Structure Compatibility**: Span and Type consistency

**Coverage**: ~30 tests for cross-crate interactions

### 3. Error Handling Tests (`tests/error_propagation_tests.rs`)

Tests error detection, propagation, and recovery:

- **Lexer Errors**: Invalid characters, unterminated strings, malformed numbers
- **Parser Errors**: Missing semicolons, unmatched braces/parens, invalid syntax
- **Parser Recovery**: Error recovery and continuation
- **Type Errors**: Type mismatches, undefined variables, incompatible operations
- **Interpreter Errors**: Division by zero, undefined functions, pattern match failures
- **Diagnostic Quality**: Span accuracy, error context, multiple errors
- **Error Recovery**: Continuing after parse/type errors
- **Error Messages**: Message clarity and helpfulness
- **Cascading Errors**: Multiple related errors
- **Error Boundaries**: Errors in nested expressions
- **Suggestions**: Typo corrections and helpful hints
- **Error Limits**: Error count tracking
- **Warnings vs Errors**: Proper severity distinction
- **Stack Traces**: Call stack tracking in errors

**Coverage**: ~35 tests for error handling

### 4. Performance and Stress Tests (`tests/stress_tests.rs`)

Tests performance under load and extreme conditions:

**Performance Targets** (from CLAUDE.md):
- CBGR overhead: < 15ns per check (measured)
- Type inference: < 100ms for 10K LOC
- Compilation speed: > 50K LOC/sec (release)
- Memory overhead: < 5% vs unsafe code

**Test Categories**:
- **Large File Compilation**: 10K+ lines of code
- **Lexing Performance**: Large source files
- **Compilation Speed**: LOC/second measurements
- **Deep Nesting**: 100+ levels of expressions
- **Memory Stress**: 10K+ object allocations
- **CBGR Overhead**: Per-check timing measurements
- **Large Collections**: 10K+ element lists
- **Large Text**: Multi-megabyte strings
- **Concurrent Operations**: Parallel parsing and type checking
- **Memory Overhead**: CBGR vs direct allocation comparison
- **Long-Running Sessions**: Multiple compilation rounds
- **Pathological Input**: Very long expressions, many parameters
- **Resource Limits**: Stack depth testing

**Coverage**: ~25 performance tests

### 5. Compatibility Tests (`tests/interop_tests.rs`)

Tests compatibility with external systems and platforms:

- **JSON Serialization**: Struct serialization/deserialization
- **JSON Roundtrip**: Data preservation through serialization
- **JSON AST**: AST serialization to/from JSON
- **JSON Complex**: Nested structures and collections
- **File I/O**: File creation, reading, writing, deletion
- **Directory Operations**: Directory creation and listing
- **Source File Handling**: Reading/writing Verum source files
- **Binary I/O**: Binary file operations
- **Path Handling**: Path construction and manipulation
- **Platform-Specific**: Line endings, path separators
- **Standard Library Compatibility**: Conversion to/from Rust std types
- **Unicode Support**: Unicode strings and identifiers
- **UTF-8 Encoding**: Multi-language text handling
- **Error Handling**: I/O and JSON error handling
- **Environment Variables**: Env var access
- **Process Information**: Current directory, temp directory
- **Concurrent I/O**: Parallel file operations
- **Large Files**: Multi-megabyte file handling
- **Buffered I/O**: Line-by-line reading
- **Metadata**: File metadata access
- **Symbolic Links** (Unix): Symlink creation and following

**Coverage**: ~30 compatibility tests

### 6. Regression Tests (`tests/bug_fixes_tests.rs`)

Tests for previously found bugs and edge cases:

- **Parser Regressions**: Empty bodies, trailing commas, nested parens, operator precedence, EOF comments
- **Lexer Regressions**: CRLF endings, float parsing, escaped quotes
- **Type Checker Regressions**: Recursive types, nested calls, tuple unification
- **Interpreter Regressions**: Short-circuit evaluation, empty pattern matching, variable shadowing
- **CBGR Regressions**: Deallocation, reference counting
- **Spec Edge Cases**: Empty programs, whitespace-only, max/min integers, empty literals, single-element tuples
- **Fuzzing Corner Cases**: Parser crashes, lexer loops, type checker panics
- **Boundary Conditions**: Zero values, negative zero, long identifiers, large lists
- **Consistency Tests**: Deterministic parsing, idempotent type checking
- **GitHub Issues**: Documented bug fixes

**Coverage**: ~40 regression tests

## Running Tests

### Run all integration tests:
```bash
cargo test --package verum_integration_tests
```

### Run specific test suite:
```bash
cargo test --package verum_integration_tests --test end_to_end_tests
cargo test --package verum_integration_tests --test module_integration_tests
cargo test --package verum_integration_tests --test error_propagation_tests
cargo test --package verum_integration_tests --test stress_tests
cargo test --package verum_integration_tests --test interop_tests
cargo test --package verum_integration_tests --test bug_fixes_tests
```

### Run with output:
```bash
cargo test --package verum_integration_tests -- --nocapture
```

### Run performance tests in release mode:
```bash
cargo test --package verum_integration_tests --test stress_tests --release
```

### Use the integration test runner script:
```bash
./tests/integration_runner.sh --verbose --coverage
```

## Test Organization

All tests follow the CLAUDE.md testing requirements:

- **NO** `#[cfg(test)]` modules in `src/`
- All tests in `tests/` directory
- Each test file focuses on a specific area
- Tests document their purpose and expected behavior
- Regression tests reference the bug they prevent

## Performance Baselines

Current performance baselines (debug build):

- Parsing 10K LOC: < 5s
- Lexing 10K LOC: < 2s
- Compilation speed: > 1000 LOC/sec (debug), > 50K LOC/sec (release target)
- CBGR overhead: < 500ns per check (debug), < 15ns target (release)
- Deep nesting: 100 levels in < 1s

## Test Statistics

- **Total Integration Tests**: ~210
- **End-to-End**: ~50
- **Cross-Crate**: ~30
- **Error Handling**: ~35
- **Performance**: ~25
- **Compatibility**: ~30
- **Regression**: ~40

## Coverage Goals

- Unit tests: ≥95% line coverage
- Integration tests: 100% of public API
- Property tests: All invariants
- Fuzzing: 24hr minimum before release

## CI/CD Integration

These tests are run in CI on:
- Every commit to main
- Every pull request
- Nightly builds
- Before every release

## Reporting Issues

If a test fails:
1. Check the test output for the specific failure
2. Look for related unit tests in the component crates
3. Add a regression test if it's a new bug
4. Document the issue in the test comments
5. Create a GitHub issue with the test name and failure details

## Performance Regression

If performance tests fail:
1. Check if it's a known issue
2. Compare against baseline (use `--baseline` flag in runner script)
3. Profile the affected code path
4. Optimize if >5% regression
5. Update baselines if intentional change

## Adding New Tests

When adding integration tests:
1. Choose the appropriate test file
2. Follow existing test patterns
3. Document what the test verifies
4. Add test to the count in this README
5. If regression test, document the bug it prevents
6. Ensure test passes in both debug and release modes
