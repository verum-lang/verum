# VCS: Verum Conformance Suite

## Quick Reference

### Test Types (from basic to full execution)

| Type | Phase | Pass Condition | Use Case |
|------|-------|----------------|----------|
| `parse` | Lexer+Parser | No syntax errors | Grammar correctness |
| `parse-fail` | Lexer+Parser | Expected syntax error | Invalid syntax detection |
| `typecheck-pass` | +Type Check | No type errors | Type system correctness |
| `typecheck-fail` | +Type Check | Expected type error | Type error detection |
| `verify-pass` | +SMT Verify | Contracts verified | Refinement types |
| `verify-fail` | +SMT Verify | Expected verification error | Contract violation |
| `compile-only` | Full Compile | Compilation succeeds | Codegen correctness |
| `run` | +Execution | Expected stdout/exit code | Runtime behavior |
| `run-panic` | +Execution | Expected panic message | Panic handling |
| `run-interpreter` | VBC Interpreter | Expected stdout/exit code | Tier 0 interpreter testing |
| `run-interpreter-panic` | VBC Interpreter | Expected panic message | Tier 0 panic testing |
| `differential` | Multi-tier | Same results across tiers | Tier consistency |
| `benchmark` | Performance | Meets timing targets | Performance regression |

### Test Levels

| Level | Purpose | Tests | Stability |
|-------|---------|-------|-----------|
| **L0-critical** | Core safety guarantees | ~500 | Must never fail |
| **L1-core** | Type system, inference | ~300 | Should not fail |
| **L2-standard** | Async, contexts, modules | ~200 | May have known issues |
| **L3-extended** | FFI, GPU, dependent types | ~150 | Experimental |
| **L4-performance** | Benchmarks | ~40 | Performance targets |

### Directory Structure

```
vcs/
├── specs/
│   ├── L0-critical/
│   │   ├── lexer/           # Token tests
│   │   ├── parser/          # Grammar tests
│   │   ├── memory-safety/   # CBGR, bounds, safety
│   │   ├── reference_system/# References, lifetimes
│   │   ├── modules/         # Import/export
│   │   └── builtin-syntax/  # Meta functions
│   ├── L1-core/
│   │   ├── types/           # Type system
│   │   ├── inference/       # Type inference
│   │   └── refinement/      # Refinement types
│   ├── L2-standard/
│   │   ├── async/           # Async/await
│   │   ├── contexts/        # Using/provide
│   │   └── protocols/       # Protocol impls
│   ├── L3-extended/
│   │   ├── ffi/             # Foreign function interface
│   │   ├── proofs/          # Formal verification
│   │   └── dependent/       # Dependent types
│   └── L4-performance/
│       └── micro/           # Micro benchmarks
├── runner/
│   └── vtest/               # Test runner source
└── scripts/
    └── run-tests.sh         # CI scripts
```

### Test File Format

```verum
// @test: typecheck-pass|typecheck-fail|run|run-panic|...
// @level: L0|L1|L2|L3|L4
// @tier: 0|1|2|3|all (execution tier)
// @tags: memory-safety, bounds-check, ...
// @timeout: 5000 (milliseconds)
// @description: Human-readable description

// For expected errors:
// @expected-error: E400
// @expected-error-count: 3

// For expected output:
// @expected-stdout: expected output
// @expected-exit: 0

// For panics:
// @expected-panic: Index out of bounds

// For skipping:
// @skip: reason for skip
// @requires: runtime, gpu, ffi

fn main() {
    // Test code
}
```

### Running Tests

```bash
# Run all L0 tests
cargo run -p vtest -- run --level L0

# Run with compile-time only (skip runtime tests)
cargo run -p vtest -- run --level L0 --compile-time-only

# Run specific test file
cargo run -p vtest -- run specs/L0-critical/parser/basic.vr

# Run with filter
cargo run -p vtest -- run --level L0 --filter "typecheck"

# Run verbose (show details)
cargo run -p vtest -- run --level L0 --verbose

# List tests without running
cargo run -p vtest -- list --level L0
```

### Error Codes

| Code | Category | Example |
|------|----------|---------|
| E0xx | Parse errors | E001: Unexpected token |
| E1xx | Name resolution | E100: Undefined variable |
| E2xx | Module errors | E200: Import not found |
| E3xx | Memory/lifetime | E310: Use after move, E312: Lifetime error |
| E4xx | Type system | E400: Type mismatch, E401: Invalid cast, E402: Send bound, E403: Sync bound |
| E5xx | Verification | E500: Contract violation |

### Common Test Patterns

**Typecheck Pass** - Code should compile without errors:
```verum
// @test: typecheck-pass
// @level: L0
fn main() {
    let x: Int = 42;
    assert(x == 42);
}
```

**Typecheck Fail** - Expect specific error:
```verum
// @test: typecheck-fail
// @level: L0
// @expected-error: E400
fn main() {
    let x: Int = "hello";  // ERROR: Type mismatch
}
```

**Run with Expected Output**:
```verum
// @test: run
// @level: L0
// @expected-stdout: Hello World
// @expected-exit: 0
fn main() {
    print("Hello World");
}
```

**Run Expecting Panic**:
```verum
// @test: run-panic
// @level: L0
// @expected-panic: Index out of bounds
fn main() {
    let arr = [1, 2, 3];
    let _ = arr[10];  // Panic
}
```

### Important Notes

1. **--compile-time-only**: Use during VBC migration when runtime isn't available. Converts `run`/`run-panic` to `typecheck-pass`.

2. **@requires: runtime**: Mark tests that need runtime. These skip when runtime unavailable.

3. **Nominal Typing**: Verum uses nominal typing for structs. `User` and `Admin` are different types even if structurally identical.

4. **Test Tiers**:
   - Tier 0: VBC Interpreter
   - Tier 1: JIT (LLVM)
   - Tier 2: AOT (LLVM)
   - Tier 3: Native (optimized)

5. **Execution Order**: Parse → Typecheck → Verify → Compile → Execute

### Makefile Targets

```bash
cd vcs
make test          # All tests
make test-l0       # L0 only
make test-l1       # L1 only
make bench         # Benchmarks
make fuzz          # Fuzzing
make differential  # Differential tests
```


<claude-mem-context>
# Recent Activity

<!-- This section is auto-generated by claude-mem. Edit content outside the tags. -->

*No recent activity*
</claude-mem-context>