# Tier Oracle: Differential Testing Between Execution Tiers

This directory contains differential tests that verify semantic equivalence between the Verum interpreter (Tier 0) and the AOT-compiled code (Tier 3).

## Overview

The Verum language supports multiple execution tiers:

| Tier | Name | Description | Use Case |
|------|------|-------------|----------|
| 0 | Interpreter | Direct AST interpretation | Development, debugging, REPL |
| 1 | Bytecode | Compiled bytecode execution | Faster development iteration |
| 2 | JIT | Just-in-time compilation | Production runtime optimization |
| 3 | AOT | Ahead-of-time LLVM compilation | Maximum performance |

**The fundamental invariant**: For the same input, all tiers MUST produce identical observable behavior.

## Purpose

These tests serve as an "oracle" - the interpreter defines the canonical semantics, and the AOT compiler must match them exactly. Discrepancies indicate bugs in either:
- The AOT codegen (most common)
- The interpreter (if found, update tests after fixing)
- The specification itself (requires committee review)

## Test Categories

### basic_arithmetic.vr
Tests fundamental arithmetic operations:
- Integer operations (add, sub, mul, div, mod, pow)
- Floating-point operations with precision validation
- Bitwise operations (and, or, xor, not, shifts)
- Overflow and underflow behavior
- Mixed-type arithmetic promotion

### control_flow.vr
Tests control flow semantics:
- If/else branching
- Match expressions with guards
- Loop constructs (while, for, loop)
- Break/continue with labels
- Short-circuit evaluation

### function_calls.vr
Tests function call semantics:
- Direct calls and tail calls
- Recursion (including mutual recursion)
- Closures and captured variables
- Higher-order functions
- Variadic arguments
- Default parameters

### memory_operations.vr
Tests memory handling:
- Stack allocation and deallocation
- Heap allocation via Box/Heap
- Reference semantics (&T, &mut T)
- CBGR generation tracking
- Move vs copy semantics
- Drop ordering

### async_behavior.vr
Tests async execution:
- Async function execution order
- Await suspension points
- Concurrent join! semantics
- Race condition handling via select!
- Timeout behavior
- Channel communication

### error_handling.vr
Tests error handling:
- Result<T, E> propagation
- Maybe<T> (Option) semantics
- ? operator behavior
- try { } blocks
- Panic handling

### refinement_checks.vr
Tests refinement type verification:
- Compile-time refinement checks
- Runtime refinement validation
- SMT-verified invariants
- Assertion behavior

## Test Annotations

Each test file uses these annotations:

```verum
// @test: differential       // Test type (required)
// @tier: 0, 3               // Tiers to compare (required)
// @level: L1                // Verification level (L0-L3)
// @tags: arithmetic, math   // Tags for filtering
// @timeout: 5000            // Per-execution timeout in ms
// @expected-stdout: ...     // Optional: expected output pattern
```

## Running Tests

### Using vtest

```bash
# Run all tier-oracle tests
verum test vcs/differential/tier-oracle/

# Run specific test
verum test vcs/differential/tier-oracle/basic_arithmetic.vr

# Run with verbose diff output
verum test --verbose vcs/differential/tier-oracle/

# Filter by tags
verum test --tags arithmetic vcs/differential/tier-oracle/
```

### Programmatic Usage

```rust
use verum_vtest::{DifferentialRunner, TierConfig};

let runner = DifferentialRunner::new()
    .with_tier(0, TierConfig::interpreter())
    .with_tier(3, TierConfig::aot())
    .with_timeout(Duration::from_secs(30));

let result = runner.run("tier-oracle/basic_arithmetic.vr")?;
assert!(result.outputs_match());
```

## Debugging Failures

When a test fails, the runner provides:

1. **Output Diff**: Line-by-line comparison of stdout
2. **Stderr Comparison**: Any error messages
3. **Exit Code**: Process exit codes
4. **Timing**: Execution time per tier

Example failure output:

```
FAIL: tier-oracle/basic_arithmetic.vr

Tier 0 (interpreter):
  42 17 59 2
  3.3333333333333335

Tier 3 (aot):
  42 17 59 2
  3.333333333333333

Diff:
  Line 2: floating-point precision difference
    Expected: 3.3333333333333335
    Actual:   3.333333333333333
```

## Adding New Tests

1. Create a new `.vr` file with appropriate annotations
2. Use `println()` to output values for comparison
3. Ensure deterministic output (no random, no timing-dependent values)
4. Include edge cases and boundary conditions
5. Document any tier-specific behavior differences (should be rare)

## Known Tier Differences

Some behaviors intentionally differ between tiers:

| Behavior | Tier 0 | Tier 3 | Reason |
|----------|--------|--------|--------|
| Float formatting | Full precision | LLVM precision | IEEE 754 allows variation |
| Stack overflow | Rust stack | Native stack | Different stack sizes |
| Debug info | Full | Stripped | Release mode optimization |

These are documented and tests account for acceptable variance.

## Performance Comparison

While not the primary goal, differential tests also reveal performance characteristics:

```bash
# Run with timing comparison
verum test --differential --timing vcs/differential/tier-oracle/

# Output includes:
#   basic_arithmetic.vr: T0=5ms, T3=0.1ms (50x speedup)
#   async_behavior.vr:   T0=100ms, T3=15ms (6.7x speedup)
```

Target speedups:
- Simple compute: 10-100x
- Complex compute: 5-20x
- I/O bound: 1-5x

## Integration with CI

These tests run on every PR and merge to main:

```yaml
# .github/workflows/differential.yml
differential-tests:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Run differential tests
      run: verum test --differential vcs/differential/tier-oracle/
```
