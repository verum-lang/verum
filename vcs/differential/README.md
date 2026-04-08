# VCS Differential Testing Infrastructure

This directory contains the comprehensive differential testing infrastructure for the Verum language.
Differential testing ensures that the interpreter (Tier 0) and AOT compiler (Tier 3)
produce identical results for the same source code, and verifies cross-implementation conformance.

## Architecture

```
differential/
├── tier-oracle/           # Tier 0 == Tier 3 tests
│   ├── arithmetic.vr      # Basic arithmetic operations
│   ├── arithmetic_edge_cases.vr # Overflow, underflow, edge cases
│   ├── control_flow.vr    # If/match/loops
│   ├── recursion.vr       # Recursive functions
│   ├── closures.vr        # Closures and captures
│   ├── generics.vr        # Generic functions and types
│   ├── collections.vr     # List, Map, Set operations
│   ├── collection_ordering.vr # Ordering and hashing
│   ├── pattern_matching.vr # Pattern matching
│   ├── error_handling.vr  # Result, Maybe handling
│   ├── string_operations.vr # Text operations
│   ├── async_basic.vr     # Basic async operations
│   ├── async_ordering.vr  # Async ordering semantics
│   └── memory_operations.vr # Memory and CBGR
├── cross-impl/            # Cross-implementation tests
│   ├── spec_conformance.vr # Language spec conformance
│   ├── edge_cases.vr      # Edge cases
│   ├── numeric_precision.vr # Float precision
│   ├── unicode_handling.vr # Unicode support
│   └── memory_model.vr    # Memory model conformance
├── runner/                # Rust runner infrastructure
│   ├── differential.rs    # Core tier oracle
│   ├── normalizer.rs      # Output normalization
│   ├── semantic_equiv.rs  # Semantic equivalence checking
│   ├── divergence.rs      # Divergence reporting
│   ├── test_generator.rs  # Automatic test generation
│   ├── cross_impl.rs      # Cross-implementation framework
│   ├── vtest_integration.rs # VTest runner integration
│   ├── mod.rs             # Module exports
│   └── Cargo.toml         # Dependencies
└── README.md              # This file
```

## Components

### 1. Tier Oracle (Tier 0 vs Tier 3)

The tier oracle compares outputs from the interpreter (Tier 0) and AOT compiler (Tier 3)
to ensure semantic equivalence.

```rust
use vcs_differential_runner::DifferentialRunner;

let runner = DifferentialRunner::new()
    .with_interpreter("verum-interpret")
    .with_aot("verum-run")
    .with_timeout(30_000);

let result = runner.run_differential(Path::new("test.vr"))?;
assert!(result.is_success());
```

### 2. Output Normalization

The normalizer handles platform-specific differences to enable reliable comparison:

```rust
use vcs_differential_runner::{Normalizer, NormalizationConfig};

let normalizer = Normalizer::new(NormalizationConfig {
    strip_addresses: true,      // Remove 0x7fff1234abcd
    normalize_floats: true,     // Round to consistent precision
    strip_timestamps: true,     // Remove timestamps
    normalize_line_endings: true, // CRLF -> LF
    strip_ansi_codes: true,     // Remove color codes
    ..Default::default()
});

let normalized = normalizer.normalize(output);
```

### 3. Semantic Equivalence Checking

Goes beyond string matching to determine semantic equivalence:

```rust
use vcs_differential_runner::{SemanticEquivalenceChecker, EquivalenceConfig};

let checker = SemanticEquivalenceChecker::new(EquivalenceConfig {
    float_epsilon: 1e-10,           // Float tolerance
    allow_unordered_collections: true, // Set/Map order doesn't matter
    allow_async_reordering: false,  // Async output must be ordered
    ..Default::default()
});

match checker.check(expected, actual) {
    EquivalenceResult::Equivalent => println!("Match!"),
    EquivalenceResult::Different(diffs) => {
        for diff in diffs {
            println!("Difference at {}: {:?}", diff.location, diff.kind);
        }
    }
}
```

### 4. Divergence Reporting

Generates detailed reports in multiple formats:

```rust
use vcs_differential_runner::{DivergenceReporter, ReportFormat};

let reporter = DivergenceReporter::new(PathBuf::from("reports"))
    .with_format(ReportFormat::Markdown)
    .with_context_lines(5);

let report_path = reporter.report(&divergence)?;
```

Supported formats:
- **Text**: Human-readable plain text
- **JSON**: Machine-readable JSON
- **SARIF**: IDE integration (VS Code, etc.)
- **Markdown**: Documentation-ready
- **HTML**: Web viewing

### 5. Automatic Test Generation

Generate regression tests from discovered divergences:

```rust
use vcs_differential_runner::{TestGenerator, GeneratorConfig};

let generator = TestGenerator::new(GeneratorConfig {
    output_dir: PathBuf::from("generated_tests"),
    generate_mutations: true,
    mutation_count: 5,
    ..Default::default()
});

// Generate minimized test case
let tests = generator.generate_variants(&divergence)?;

for test in tests {
    generator.write_test(&test)?;
}
```

### 6. Cross-Implementation Testing

Test across multiple Verum implementations:

```rust
use vcs_differential_runner::{CrossImplRunner, CrossImplConfig, Implementation};

let config = CrossImplConfig::default()
    .with_reference("interpreter", "verum-interpret")
    .with_alternative("aot", "verum-run")
    .with_alternative("jit", "verum-jit");

let runner = CrossImplRunner::new(config);
let results = runner.run_directory(Path::new("specs/"))?;

runner.generate_report(&results)?;
```

### 7. VTest Integration

Seamless integration with the VTest runner:

```rust
use vcs_differential_runner::{DifferentialExecutor, DifferentialTestConfig};

let executor = DifferentialExecutor::new(DifferentialTestConfig::default());
let result = executor.execute(Path::new("test.vr"))?;

if result.passed {
    println!("All tiers agree!");
} else {
    for div in &result.divergences {
        println!("Divergence: {} vs {}: {}", div.tier1, div.tier2, div.summary);
    }
}
```

## Test Annotations

Each test file uses annotations to specify testing parameters:

```verum
// @test: differential     # Test type
// @tier: 0, 3             # Tiers to compare
// @level: L1              # Verification level
// @tags: arithmetic, math # Tags for filtering
// @timeout: 5000          # Timeout in milliseconds
// @impl: all              # Run on all implementations
// @require: async         # Required features
// @platform: any          # Platform constraints
```

## Test Categories

### tier-oracle/

| File | Description |
|------|-------------|
| arithmetic.vr | Basic integer and float arithmetic |
| arithmetic_edge_cases.vr | Overflow, underflow, edge cases |
| control_flow.vr | If/else, match, loops |
| recursion.vr | Recursive functions, tail calls |
| closures.vr | Closures, captures |
| generics.vr | Generic functions and types |
| collections.vr | List, Map, Set operations |
| collection_ordering.vr | Ordering, hashing, iteration |
| pattern_matching.vr | Patterns, guards, destructuring |
| error_handling.vr | Result, Maybe, error propagation |
| string_operations.vr | Text operations, formatting |
| async_basic.vr | Basic async/await |
| async_ordering.vr | Async ordering semantics |
| memory_operations.vr | Memory, references, CBGR |

### cross-impl/

| File | Description |
|------|-------------|
| spec_conformance.vr | Language spec conformance |
| edge_cases.vr | Numeric limits, corner cases |
| numeric_precision.vr | IEEE 754 conformance |
| unicode_handling.vr | Unicode support, normalization |
| memory_model.vr | Memory model behavior |

## Running Tests

### Using VTest

```bash
# Run all differential tests
vtest --differential

# Run specific test
vtest --differential tier-oracle/arithmetic.vr

# Run with specific tiers
vtest --differential --tier 0,3

# Run with verbose output
vtest --differential --verbose

# Generate reports
vtest --differential --report markdown
```

### Using the Runner Directly

```rust
use vcs_differential_runner::{DifferentialRunner, DifferentialTestConfig};

let runner = DifferentialRunner::new();
let report = runner.run_directory(Path::new("tier-oracle/"))?;

report.print_summary();
assert!(report.success());
```

### Fuzzing

```rust
use vcs_differential_runner::DifferentialFuzzer;

let runner = DifferentialRunner::new();
let mut fuzzer = DifferentialFuzzer::new(runner, 12345);

let failures = fuzzer.fuzz(10_000)?;
for (program, result) in failures {
    println!("Found divergence:\n{}\n{:?}", program, result);
}
```

## Debugging Failures

When a test fails, detailed diff output is provided:

```
=== Differential Test Report ===
ID:            abc123def456
Classification: Float Precision
Source File:   tier-oracle/arithmetic.vr

Tier 1 (Interpreter):
  Exit code: 0
  Duration:  45ms

Tier 2 (AOT):
  Exit code: 0
  Duration:  12ms

DIFFERENCES
-----------
Difference #1:
  Location:  line 24
  Kind:      FloatMismatch { expected: 3.3333333333333335, actual: 3.333333333333333 }
  Severity:  Warning

UNIFIED DIFF
------------
--- Tier 0 (Interpreter)
+++ Tier 3 (AOT)
 59 42 42 42 2
-6.0 3.3333333333333335
+6.0 3.333333333333333
```

## Performance Targets

| Operation | Target |
|-----------|--------|
| Simple test execution | < 100ms |
| Complex async test | < 1s |
| Fuzz iterations | > 1000/sec |
| Normalization | < 1ms per KB |
| Report generation | < 100ms |

## Adding New Tests

1. Create a `.vr` file in the appropriate directory
2. Add test annotations at the top
3. Write deterministic test cases
4. Use `println` to output results for comparison

Example:

```verum
// @test: differential
// @tier: 0, 3
// @level: L1
// @tags: my_feature

fn main() {
    let result = my_function(42);
    println(f"result = {result}");
}
```

## Contributing

When adding differential tests:

1. Ensure tests are deterministic (no random values, timestamps, addresses)
2. Use specific assertions rather than complex outputs
3. Document expected behavior in comments
4. Add appropriate tags for filtering
5. Consider edge cases (empty, null, max values)
6. Test both success and failure paths
