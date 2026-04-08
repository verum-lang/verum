# VCS Codegen Verification Infrastructure

This directory contains comprehensive testing infrastructure for verifying the correctness,
quality, and performance of Verum's code generation across all tiers (interpreter, JIT, AOT).

## Architecture

```
codegen/
├── llvm-ir/                  # LLVM IR verification tests
│   ├── correctness/          # IR semantic correctness
│   ├── optimization/         # Optimization verification
│   ├── patterns/             # Code pattern tests
│   └── regression/           # Regression tests
├── jit/                      # JIT-specific verification
│   ├── compilation/          # JIT compilation tests
│   ├── hot-code/             # Hot code replacement tests
│   ├── deoptimization/       # Deopt/bailout tests
│   ├── memory/               # JIT memory management
│   └── benchmarks/           # JIT performance benchmarks
├── aot/                      # AOT-specific verification
│   ├── linking/              # Static linking tests
│   ├── cross-compile/        # Cross-compilation tests
│   ├── binary-format/        # ELF/Mach-O/PE tests
│   └── benchmarks/           # AOT performance benchmarks
├── cbgr/                     # CBGR codegen verification
│   ├── generation/           # Generation tracking codegen
│   ├── promotion/            # Tier promotion tests
│   └── elimination/          # Check elimination verification
├── runner/                   # Rust runner infrastructure
│   ├── ir_verifier.rs        # LLVM IR verification
│   ├── jit_tester.rs         # JIT test harness
│   ├── aot_tester.rs         # AOT test harness
│   ├── benchmark.rs          # Performance benchmarking
│   ├── golden_master.rs      # Golden master testing
│   └── mod.rs                # Module exports
└── scripts/                  # Automation scripts
```

## Components

### 1. LLVM IR Verification

Verifies that generated LLVM IR is correct, well-formed, and optimized:

```rust
use vcs_codegen_runner::{IrVerifier, IrVerificationConfig};

let verifier = IrVerifier::new(IrVerificationConfig {
    check_types: true,           // Type safety verification
    check_aliasing: true,        // TBAA correctness
    check_cbgr: true,            // CBGR intrinsic verification
    check_memory: true,          // Memory operation verification
    optimization_level: OptLevel::O2,
    ..Default::default()
});

let result = verifier.verify_module(&llvm_module)?;
assert!(result.is_valid());
```

### 2. IR Pattern Testing

Tests that specific code patterns produce expected IR:

```verum
// @test: ir-pattern
// @tier: 1, 2, 3
// @level: L0
// @pattern: tail-call-elimination
// @expect-ir: "musttail call"

fn factorial(n: Int, acc: Int) -> Int {
    if n <= 1 { acc }
    else { factorial(n - 1, acc * n) }  // Should be tail-call optimized
}
```

### 3. JIT Verification

Tests JIT-specific functionality:

#### 3.1 Compilation Testing
```rust
use vcs_codegen_runner::{JitTester, JitTestConfig};

let tester = JitTester::new(JitTestConfig {
    strategy: CompilationStrategy::Lazy,
    optimization: OptLevel::O1,
    timeout_ms: 5000,
    ..Default::default()
});

// Test lazy compilation triggers correctly
let result = tester.test_lazy_compilation(&module)?;
assert!(result.compiled_on_demand);

// Test hot code replacement
let result = tester.test_hot_code_replacement(&module, &updated_module)?;
assert!(result.seamless_transition);
```

#### 3.2 Deoptimization Testing
```verum
// @test: jit-deopt
// @tier: 1, 2
// @level: L1
// @tags: deoptimization, bailout
// @scenario: type-guard-failure

fn polymorphic(x: dyn Show) -> Text {
    x.show()  // JIT may specialize, must deopt if type changes
}
```

#### 3.3 Memory Management Testing
```rust
use vcs_codegen_runner::{JitMemoryTester, MemoryConfig};

let tester = JitMemoryTester::new(MemoryConfig {
    max_code_size: 100 * 1024 * 1024,  // 100 MB
    track_leaks: true,
    enforce_limits: true,
    ..Default::default()
});

// Verify no memory leaks after many compilations
let result = tester.stress_test(&modules, 10_000)?;
assert!(result.memory_stable);
assert!(result.leak_count == 0);
```

### 4. AOT Verification

Tests AOT-specific functionality:

#### 4.1 Binary Format Verification
```rust
use vcs_codegen_runner::{AotVerifier, BinaryFormat};

let verifier = AotVerifier::new();

// Verify ELF format
let elf_result = verifier.verify_binary(Path::new("output.o"), BinaryFormat::Elf)?;
assert!(elf_result.valid_headers);
assert!(elf_result.proper_relocations);
assert!(elf_result.debug_info_present);

// Verify symbol table
assert!(elf_result.symbols.contains("verum_main"));
assert!(elf_result.symbols.contains("verum_cbgr_check"));
```

#### 4.2 Cross-Compilation Verification
```verum
// @test: cross-compile
// @tier: 3
// @level: L2
// @targets: x86_64-linux, aarch64-linux, x86_64-windows
// @expect: identical-semantics

fn main() {
    let result = compute();
    assert_eq(result, 42);
}
```

### 5. CBGR Codegen Verification

Tests CBGR (Compact Bounded Generation References) code generation:

#### 5.1 Generation Tracking
```verum
// @test: cbgr-generation
// @tier: 1, 2, 3
// @level: L0
// @expect-ir: "verum_cbgr_check"

fn safe_access(data: &List<Int>) -> Int {
    data[0]  // Should generate CBGR check in IR
}
```

#### 5.2 Check Elimination
```verum
// @test: cbgr-elimination
// @tier: 2, 3
// @level: L1
// @expect-no-ir: "verum_cbgr_check"
// @reason: escape-analysis-proves-local

fn local_only() -> Int {
    let list = List::new();  // Local allocation
    list.push(42);
    list[0]  // CBGR check should be eliminated
}
```

### 6. Performance Benchmarks

#### JIT Benchmarks
```rust
use vcs_codegen_runner::{JitBenchmark, BenchmarkConfig};

let benchmark = JitBenchmark::new(BenchmarkConfig {
    warmup_iterations: 10,
    measure_iterations: 100,
    ..Default::default()
});

let results = benchmark.run(&[
    BenchmarkCase::new("fibonacci_recursive", fib_source),
    BenchmarkCase::new("fibonacci_iterative", fib_iter_source),
    BenchmarkCase::new("matrix_multiply", matrix_source),
])?;

// Assert performance targets
assert!(results["fibonacci_recursive"].mean_ns < 1_000_000);
assert!(results["matrix_multiply"].ops_per_sec > 10_000);
```

#### AOT vs JIT Comparison
```bash
# Run comparative benchmark
vbench --codegen-compare --tier 1,3 benchmarks/compute.vr

# Expected output:
# compute.vr - Tier Comparison
#   Tier 1 (JIT):        45.2 ns/op ± 2.3%
#   Tier 3 (AOT):        38.1 ns/op ± 1.1%
#   AOT speedup:         1.19x
```

## Test Annotations

Codegen tests use specific annotations:

```verum
// @test: ir-pattern|ir-verify|jit-deopt|aot-link|cbgr-check
// @tier: 1|2|3|all
// @level: L0|L1|L2
// @tags: comma, separated, tags
// @timeout: milliseconds
// @expect-ir: "pattern to find in IR"
// @expect-no-ir: "pattern that must not exist"
// @optimization: none|less|default|aggressive
// @target: native|x86_64-linux|aarch64-darwin|...
```

## Performance Targets

| Metric | Target | Description |
|--------|--------|-------------|
| IR Generation | < 10ms/KLOC | LLVM IR generation speed |
| JIT Compilation | < 50ms/function | First-call compilation latency |
| JIT Warmup | < 100ms | Time to reach peak performance |
| AOT Compilation | > 50K LOC/sec | Ahead-of-time compilation speed |
| Binary Size | < 2x Rust | Comparable to idiomatic Rust |
| Runtime Perf | 0.85-0.95x C | Near-native performance |
| CBGR Overhead | < 15ns/check | Generation check cost |

## Running Tests

### All Codegen Tests
```bash
vtest --codegen vcs/codegen
```

### Specific Categories
```bash
# IR verification
vtest --codegen --category ir vcs/codegen/llvm-ir

# JIT tests
vtest --codegen --category jit vcs/codegen/jit

# AOT tests
vtest --codegen --category aot vcs/codegen/aot

# CBGR tests
vtest --codegen --category cbgr vcs/codegen/cbgr
```

### Benchmarks
```bash
# Run all benchmarks
vbench --codegen vcs/codegen/*/benchmarks

# Compare tiers
vbench --codegen-compare --tier 0,1,2,3 vcs/codegen/jit/benchmarks
```

## Golden Master Testing

For regression testing of IR output:

```bash
# Generate golden master (first time)
vtest --codegen --generate-golden vcs/codegen/llvm-ir

# Compare against golden master
vtest --codegen --check-golden vcs/codegen/llvm-ir
```

## Integration with CI

The codegen tests integrate with the CI pipeline:

1. **PR Checks**: Fast IR verification (~30s)
2. **Nightly**: Full JIT/AOT test suite (~10min)
3. **Weekly**: Performance regression detection
4. **Release**: Full cross-platform verification

## Adding New Tests

### IR Pattern Test
```verum
// @test: ir-pattern
// @tier: 2, 3
// @level: L1
// @pattern: simd-vectorization
// @expect-ir: "llvm.vector"

fn sum_array(arr: [Int; 1024]) -> Int {
    arr.iter().sum()  // Should vectorize
}
```

### JIT Test
```verum
// @test: jit-compilation
// @tier: 1, 2
// @level: L1
// @scenario: lazy-compile

fn lazy_function() -> Int {
    // This function should only be compiled when called
    expensive_computation()
}

fn main() {
    // lazy_function not compiled here
    let should_call = get_condition();
    if should_call {
        lazy_function();  // Compiled on first call
    }
}
```

### AOT Test
```verum
// @test: aot-link
// @tier: 3
// @level: L2
// @link-with: external_lib.a
// @expect: successful-link

extern fn external_function() -> Int;

fn main() {
    let result = external_function();
    assert_eq(result, 42);
}
```

## Debugging Failures

### IR Verification Failure
```
=== IR Verification Failed ===
File: test.vr
Phase: Post-Optimization

Error: Invalid type in getelementptr
  %ptr = getelementptr inbounds i64, ptr %arr, i32 %idx
  Expected: i64 index for i64 element type
  Actual: i32 index

Suggestion: Check array indexing codegen in expressions/mod.rs
```

### JIT Test Failure
```
=== JIT Test Failed ===
Test: hot-code-replacement
Scenario: function-update-mid-execution

Expected: Seamless transition to new code
Actual: Segmentation fault during transition

JIT Stats:
  Functions compiled: 42
  Hot replacements: 5
  Failed replacements: 1

Stack trace:
  at verum_jit::hot_code::replace_function (jit/hot_code.rs:234)
  at verum_jit::engine::update_module (jit/engine.rs:567)
```

## Contributing

When adding codegen tests:

1. **Determinism**: Tests must produce identical results across runs
2. **Isolation**: Tests should not depend on external state
3. **Documentation**: Document what aspect of codegen is being tested
4. **Performance**: Keep individual tests fast (< 100ms)
5. **Coverage**: Cover both success and error paths
6. **Regression**: Add regression tests for any bugs found
