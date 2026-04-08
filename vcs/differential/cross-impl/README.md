# Cross-Implementation: Differential Testing Across Implementations

This directory contains differential tests for future cross-implementation testing of the Verum language. These tests verify that different implementations of the Verum language produce semantically equivalent results.

## Overview

As Verum matures, multiple implementations may exist:

| Implementation | Status | Description |
|----------------|--------|-------------|
| **verum-rs** | Reference | Rust-based reference implementation |
| **verum-llvm** | Planned | Direct LLVM frontend |
| **verum-js** | Planned | JavaScript/WebAssembly target |
| **verum-jvm** | Planned | JVM bytecode target |
| **verum-native** | Planned | Native machine code generator |

These tests ensure all implementations agree on language semantics.

## Purpose

Cross-implementation testing serves several goals:

1. **Specification Validation**: Tests serve as executable specifications
2. **Portability Assurance**: Code behaves identically across platforms
3. **Standard Compliance**: All implementations follow the same standard
4. **Edge Case Discovery**: Find implementation-specific quirks

## Test Categories

### portable_semantics.vr
Tests core language semantics that must be portable:
- Expression evaluation order
- Scope and binding rules
- Type system behavior
- Control flow semantics
- Function call conventions

### ieee754_conformance.vr
Tests IEEE 754 floating-point conformance:
- Special values (NaN, Infinity, -0.0)
- Rounding modes
- Precision requirements
- Edge cases and corner cases
- Comparison behavior

### unicode_handling.vr
Tests Unicode text processing:
- UTF-8 encoding/decoding
- Normalization forms (NFC, NFD, NFKC, NFKD)
- Grapheme cluster handling
- Case conversion
- Collation and comparison

### memory_model.vr
Tests memory model semantics:
- Allocation behavior
- Reference semantics
- Concurrent access (when applicable)
- Memory ordering guarantees
- Drop and cleanup ordering

## Test Annotations

Each test file uses these annotations:

```verum
// @test: differential       // Test type
// @tier: all                // Run on all execution tiers
// @impl: all                // Run on all implementations
// @level: L2                // Verification level
// @tags: portable, ieee754  // Tags for filtering
// @platform: any            // Platform requirements
```

## Running Tests

### Single Implementation

```bash
# Run with reference implementation
verum test --impl verum-rs vcs/differential/cross-impl/

# Run with alternative implementation
verum test --impl verum-llvm vcs/differential/cross-impl/
```

### Cross-Implementation

```bash
# Compare two implementations
verum test --diff-impl verum-rs,verum-llvm vcs/differential/cross-impl/

# Compare all implementations
verum test --diff-impl all vcs/differential/cross-impl/
```

### Output Format

```
=== Cross-Implementation Differential Report ===

Test: portable_semantics.vr

verum-rs:
  stdout: "42\nhello\ntrue\n"
  exit: 0
  time: 5ms

verum-llvm:
  stdout: "42\nhello\ntrue\n"
  exit: 0
  time: 0.5ms

Status: MATCH

---

Test: ieee754_conformance.vr

verum-rs:
  stdout: "0.30000000000000004\n"

verum-llvm:
  stdout: "0.3\n"

Status: MISMATCH
Diff: Line 1: precision difference in 0.1 + 0.2
```

## Platform Considerations

Some tests are platform-specific:

| Test | Linux | macOS | Windows | WASM |
|------|-------|-------|---------|------|
| portable_semantics.vr | Yes | Yes | Yes | Yes |
| ieee754_conformance.vr | Yes | Yes | Yes | Yes |
| unicode_handling.vr | Yes | Yes | Yes | Yes |
| memory_model.vr | Yes | Yes | Yes | Partial |

Platform-specific behavior is documented in each test file.

## Known Implementation Differences

Some behaviors intentionally or acceptably differ:

| Behavior | Allowed Variance | Reason |
|----------|------------------|--------|
| Float formatting | Minor precision | IEEE 754 allows variation |
| Error messages | Text differs | Implementation detail |
| Stack traces | Format differs | Debug info variation |
| Timing values | Varies | Hardware dependent |

Tests account for these acceptable variations.

## Adding New Tests

1. Identify portable behavior that needs testing
2. Create test file with appropriate annotations
3. Use `println()` for deterministic, comparable output
4. Document any platform-specific considerations
5. Add expected behavior comments

Example structure:

```verum
// @test: differential
// @impl: all
// @platform: any

/// Test description
/// Expected: consistent across all implementations

fn main() {
    // Test case
    let result = operation();
    println(f"result: {result}");

    // Edge case
    let edge = edge_operation();
    println(f"edge: {edge}");
}
```

## Integration with Specification

These tests tie directly to the Verum Language Specification:

| Test File | Spec Sections |
|-----------|---------------|
| portable_semantics.vr | Ch 3 (Expressions), Ch 4 (Control Flow) |
| ieee754_conformance.vr | Ch 5.2 (Float Types) |
| unicode_handling.vr | Ch 5.3 (Text Type) |
| memory_model.vr | Ch 8 (Memory Model) |

When specification changes, update corresponding tests.

## CI Integration

Cross-implementation tests run on merge to main:

```yaml
# .github/workflows/cross-impl.yml
cross-impl-tests:
  strategy:
    matrix:
      impl: [verum-rs, verum-llvm]
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Run cross-impl tests
      run: verum test --impl ${{ matrix.impl }} vcs/differential/cross-impl/

compare-implementations:
  needs: cross-impl-tests
  runs-on: ubuntu-latest
  steps:
    - name: Compare outputs
      run: verum test --diff-impl all --report vcs/differential/cross-impl/
```

## Future Work

- [ ] Add more edge case tests
- [ ] Implement verum-llvm comparison
- [ ] Add WASM implementation tests
- [ ] Create fuzz-based cross-impl testing
- [ ] Add performance comparison mode
