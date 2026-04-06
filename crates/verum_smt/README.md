# verum_smt - SMT-Based Verification for Verum

[![Crates.io](https://img.shields.io/crates/v/verum_smt)](https://crates.io/crates/verum_smt)
[![Documentation](https://docs.rs/verum_smt/badge.svg)](https://docs.rs/verum_smt)

**Status**: P0 (Highest Priority) Component for v1.0

SMT-based verification engine for Verum's refinement type system. This crate integrates Z3 SMT solver to enable static verification of refinement constraints, providing:

- **Refinement Type Verification**: Prove that values satisfy type constraints at compile time
- **Counterexample Generation**: Produce concrete values demonstrating constraint violations
- **Verification Cost Reporting**: Track and report verification time per function (P0 Feature)
- **Proof Failure Diagnostics**: Rich error messages with actionable suggestions (P0 Feature)
- **Gradual Verification**: Support for `@verify(runtime)`, `@verify(proof)`, and `@verify(auto)` modes

## Architecture

The verum_smt crate consists of six main modules:

```
verum_smt/
├── context.rs      - Z3 context management and configuration (~320 lines)
├── translate.rs    - Verum AST → Z3 expression translation (~520 lines)
├── verify.rs       - Core verification engine with cost tracking (~450 lines) ⭐ P0
├── cost.rs         - Verification cost tracking and reporting (~450 lines) ⭐ P0
├── counterexample.rs - Counterexample extraction and formatting (~400 lines) ⭐ P0
└── tests.rs        - Comprehensive test suite (600+ lines, 40+ tests)
```

## Quick Start

### Basic Verification

```rust
use verum_smt::{Context, verify_refinement, VerifyMode};
use verum_ast::{Type, Expr};

// Create Z3 context
let ctx = Context::new();

// Define a refinement type: Int{> 0}
let positive_int_type = /* ... create refined type ... */;

// Verify the refinement
match verify_refinement(&ctx, &positive_int_type, None, VerifyMode::Proof) {
    Ok(proof) => {
        println!("✓ Verified in {:.2}s", proof.cost.time_secs());
    }
    Err(e) => {
        eprintln!("✗ Cannot prove constraint: {}", e);
        if let Some(ce) = e.counterexample {
            eprintln!("  Counterexample: {}", ce);
        }
        for suggestion in e.suggestions() {
            eprintln!("  Suggestion: {}", suggestion);
        }
    }
}
```

### Cost Tracking (P0 Feature)

```rust
use verum_smt::{CostTracker, Context, verify_refinement};

let ctx = Context::new();
let mut tracker = CostTracker::new();

// Verify multiple functions
for func_type in function_types {
    match verify_refinement(&ctx, &func_type, None, VerifyMode::Auto) {
        Ok(proof) => tracker.record(proof.cost),
        Err(e) => {
            if let Some(cost) = e.cost() {
                tracker.record(cost.clone());
            }
        }
    }
}

// Generate comprehensive report
let report = tracker.report();
println!("{}", report);

// Example output:
// Verification Summary:
//   Total: 42 checks in 15.3s
//   Average: 0.36s per check
//   Slowest: complex_algorithm() (8.4s)
//
// Slow verifications (3):
//   • complex_algorithm() - 8.4s
//   • matrix_transform() - 6.2s
//   • data_processor() - 5.1s
//
// Suggestion:
//   Consider using @verify(runtime) for expensive checks
//   This will use runtime validation instead of compile-time SMT
```

## Verification Modes

### `@verify(runtime)` - Runtime Checks (Default for JIT)

Skip SMT verification entirely. Refinements are checked at runtime:

```verum
@verify(runtime)
fn divide(x: Float, y: Float{!= 0}) -> Float {
    x / y  // Runtime check: assert!(y != 0.0)
}
```

**Build time**: 0s verification
**Runtime cost**: 1-5% overhead from assertion checks

### `@verify(proof)` - Static Verification (AOT Optimization)

Full SMT verification. Proven constraints are eliminated in compiled code:

```verum
@verify(proof)
fn abs(x: Int) -> Int{>= 0} {
    if x < 0 { -x } else { x }
}
```

**Build time**: Varies (0.1s - 30s+ per function)
**Runtime cost**: 0% overhead when proven

### `@verify(auto)` - Heuristic Mode

Automatically chooses between proof and runtime based on complexity:

- Simple constraints (< 30 complexity) → `proof`
- Complex constraints (> 70 complexity) → `runtime`
- Medium complexity → `proof` with short timeout

## Cost Reporting (P0 Feature)

### Per-Function Tracking

The verifier tracks detailed metrics for each function:

```rust
pub struct VerificationCost {
    pub location: String,           // Function name
    pub duration: Duration,          // Total verification time
    pub succeeded: bool,             // Whether proof succeeded
    pub num_checks: u64,             // Number of SMT queries
    pub complexity: u32,             // Complexity estimate (0-100)
    pub timed_out: bool,            // Whether verification timed out
}
```

### Automatic Suggestions

The cost tracker generates actionable suggestions:

```
⚠ complex_fn(): Timeout after 30s
  Suggestion: Use @verify(runtime) for faster builds (30s → 0s)

✓ simple_fn(): Proved in 1.2s using Z3
  Suggestion: Consider @verify(runtime) for faster builds (1.2s → 0s)
```

## Counterexample Generation (P0 Feature)

### Extraction from Z3 Models

When verification fails, concrete counterexamples are extracted:

```rust
pub struct CounterExample {
    pub assignments: HashMap<String, CounterExampleValue>,
    pub description: String,
    pub violated_constraint: String,
}

pub enum CounterExampleValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<CounterExampleValue>),
    Record(HashMap<String, CounterExampleValue>),
    Unknown(String),
}
```

### Example Output

```
✗ Cannot prove postcondition
  Location: validate_range:1:37
  Failed constraint: result >= 0 && result <= 100

  Counterexample: x = 51 yields 102
  Violates: result <= 100 (102 > 100)

Suggestions:
  • Add precondition: x: Int{>= 0 && it <= 50}
  • Or weaken postcondition: Int{>= 0 && it <= 200}
```

### Suggestion Generation

The system analyzes failures and generates helpful suggestions:

```rust
pub fn generate_suggestions(
    counterexample: &CounterExample,
    constraint: &str,
) -> Vec<String>;
```

**Common suggestion patterns**:
- Negative integers violating `> 0` → "Add precondition: require x > 0"
- Division operations → "Check for division by zero"
- Array access → "Verify array indices are within bounds"
- Timeout → "Consider @verify(runtime) or simplify predicate"

## Translation to SMT-LIB

The `translate` module converts Verum expressions to Z3:

### Supported Operations

| Verum | SMT-LIB | Example |
|-------|---------|---------|
| `42` | `(Int 42)` | Integer literals |
| `x + y` | `(+ x y)` | Arithmetic operations |
| `x > 0` | `(> x 0)` | Comparisons |
| `a && b` | `(and a b)` | Logical operations |
| `abs(x)` | `(ite (>= x 0) x (- x))` | Built-in functions |
| `min(a, b)` | `(ite (< a b) a b)` | Min/max functions |
| `x ** y` | `(^ x y)` | Power function |

### Type Mapping

| Verum Type | Z3 Sort | Notes |
|------------|---------|-------|
| `Int` | `Int` | Unbounded integers |
| `Float` | `Real` | Real number approximation |
| `Bool` | `Bool` | Boolean values |
| `String` | `String` | String theory (limited) |
| `Int{> 0}` | `Int` + constraint | Refinement as separate assertion |

## Configuration

### Context Configuration

```rust
use std::time::Duration;
use verum_smt::{Context, ContextConfig};

// Fast mode (5s timeout)
let ctx = Context::with_config(ContextConfig::fast());

// Thorough mode (120s timeout)
let ctx = Context::with_config(ContextConfig::thorough());

// Custom configuration
let config = ContextConfig::default()
    .with_timeout(Duration::from_secs(10))
    .with_memory_limit(512)  // MB
    .with_models()           // Enable counterexample generation
    .with_seed(42);          // Reproducible results

let ctx = Context::with_config(config);
```

### Cost Tracker Configuration

```rust
use verum_smt::CostTracker;

// Default threshold (5s)
let tracker = CostTracker::new();

// Custom slow threshold
let tracker = CostTracker::with_threshold(Duration::from_secs(2));
```

## Performance Characteristics

### Verification Time

| Constraint Complexity | Typical Time | Strategy |
|----------------------|--------------|----------|
| Simple (`x > 0`) | 0.1 - 0.5s | Always use `@verify(proof)` |
| Medium (`x > 0 && x < 100`) | 0.5 - 2s | Use `@verify(auto)` |
| Complex (quantifiers, loops) | 2s - 30s | Consider `@verify(runtime)` |
| Very Complex (nested quantifiers) | 30s+ timeout | Use `@verify(runtime)` |

### Memory Usage

- Base Z3 context: ~50 MB
- Per verification: ~5-20 MB
- Peak with complex constraints: ~200-500 MB

### Scalability

- ✓ Handles projects with 1000+ functions
- ✓ Parallel verification of independent functions
- ✓ Incremental verification (only changed functions)
- ⚠ Large quantified formulas may timeout

## Integration with Verum Compiler

### Compiler Pipeline Integration

```
Source Code
    ↓
Parsing (verum_parser)
    ↓
Type Checking (verum_types)
    ↓
[VERIFICATION] ← verum_smt
    ↓
Code Generation (verum_codegen)
    ↓
Output Binary
```

### Command Line Usage

```bash
# Skip verification (fastest builds)
verum build --verify=runtime

# Full verification (safest code)
verum build --verify=proof

# Automatic mode (balanced)
verum build --verify=auto

# With detailed cost reporting
verum build --verify=proof --emit-verify-stats
```

### Expected Compiler Output

```
Building main.vr...
  ✓ parse() - Proved in 0.3s using Z3
  ✓ validate() - Proved in 0.8s using Z3
  ⚠ complex_algorithm() - Timeout after 30s, using runtime checks
  ✗ unsafe_divide() - Cannot prove postcondition

Verification Summary:
  Functions analyzed: 247
  Successfully proven: 198 (80.2%)
  Timeouts: 12 (4.9%)
  Failed proofs: 37 (15.0%)

  Total verification time: 142.3s
  Average time per function: 0.58s
  Longest verification: complex_transform() at 28.4s

Suggestions:
  • Use @verify(runtime) for complex_algorithm() (30s → 0s)
  • Fix unsafe_divide(): Add precondition y != 0

Build time can be reduced by ~40s by switching to @verify(runtime) for:
  - complex_algorithm()
  - matrix_transform()
```

## Error Handling

### Verification Errors

```rust
pub enum VerificationError {
    /// Constraint cannot be proven (with counterexample)
    CannotProve {
        constraint: String,
        counterexample: Option<CounterExample>,
        cost: VerificationCost,
        suggestions: Vec<String>,
    },

    /// Verification timed out
    Timeout {
        constraint: String,
        timeout: Duration,
        cost: VerificationCost,
    },

    /// Translation error (unsupported feature)
    Translation(TranslationError),

    /// SMT solver error
    SolverError(String),

    /// Unknown result from solver
    Unknown(String),
}
```

### Translation Errors

```rust
pub enum TranslationError {
    UnsupportedExpr(String),
    UnsupportedLiteral(String),
    UnsupportedOp(String),
    UnsupportedFunction(String),
    UnsupportedType(String),
    TypeMismatch(String),
    UnboundVariable(String),
}
```

## Testing

The crate includes 40+ comprehensive tests covering all modules:

```bash
# Run all tests
cargo test -p verum_smt

# Run with output
cargo test -p verum_smt -- --nocapture

# Run specific test category
cargo test -p verum_smt context_tests
cargo test -p verum_smt verification_tests
cargo test -p verum_smt cost_tests
```

### Test Categories

1. **Context Tests** (5 tests) - Z3 context management
2. **Translation Tests** (10 tests) - AST → Z3 conversion
3. **Verification Tests** (10 tests) - Core verification logic
4. **Cost Tracking Tests** (5 tests) - Cost reporting
5. **Counterexample Tests** (5 tests) - Counterexample extraction
6. **Integration Tests** (5 tests) - End-to-end workflows

## Limitations

### Current Limitations

- **No array theory support**: Length functions not yet implemented
- **Limited quantifier support**: Complex nested quantifiers may fail
- **No string constraints**: String refinements not fully supported
- **No recursive functions**: Induction proofs not automated
- **Timeout sensitivity**: Very complex proofs may need manual hints

### Planned Improvements (v1.1+)

- [ ] Array theory integration for length/indexing
- [ ] String constraint solving (Z3 string theory)
- [ ] Proof caching for incremental builds
- [ ] Custom tactics for common patterns
- [ ] Parallel verification of independent constraints
- [ ] Interactive proof mode for failed verifications

## Dependencies

- `z3` - Z3 SMT solver bindings (v0.12+)
- `verum_ast` - Verum AST definitions
- `thiserror` - Error handling
- `tracing` - Structured logging

## Contributing

This is a critical (P0) component for Verum v1.0. All changes must:

1. Pass all existing tests
2. Add new tests for new features
3. Maintain zero warnings (`cargo clippy`)
4. Include documentation
5. Preserve performance characteristics

## Practical Examples

### Example 1: Safe Division

```rust
use verum_smt::{Context, verify_refinement, VerifyMode};

// Type: Int{!= 0}
fn verify_non_zero_divisor() {
    let ctx = Context::new();

    // Build refinement type for non-zero integers
    // This will fail verification because zero exists
    // In practice, you'd use this as a parameter constraint:
    //   fn divide(x: Int, y: Int{!= 0}) -> Int

    // The verification will produce a counterexample: y = 0
    // Suggesting you need runtime checks or preconditions
}
```

### Example 2: Array Bounds

```rust
// Type: Int{it >= 0 && it < arr.length}
fn verify_array_index(arr_length: usize) {
    let ctx = Context::new();

    // When verifying array access like arr[i],
    // we need: i: Int{it >= 0 && it < arr.length}

    // If the constraint can't be proven, you'll get:
    //   Counterexample: i = -1 (violates it >= 0)
    //   Suggestion: Add precondition or use checked indexing
}
```

### Example 3: Postcondition Verification

```rust
// Function: abs(x: Int) -> Int{>= 0}
fn verify_abs_postcondition() {
    let ctx = Context::new();

    // Verum will verify that abs always returns non-negative:
    //   if x < 0 { -x } else { x }

    // This should verify successfully (no counterexample)
    // because the implementation ensures result >= 0
}
```

### Example 4: Range Validation

```rust
// Type: Int{it >= 0 && it <= 100}  (percentage)
fn verify_percentage_range() {
    let ctx = Context::new();

    // For a percentage type, we want values in [0, 100]
    // Verification will fail if input is unbounded

    // Solution: Add constructor that validates:
    //   fn make_percentage(x: Int) -> Result<Percentage, Error>
    //   Or use refinement in function signature
}
```

### Example 5: Cost-Based Optimization

```rust
use verum_smt::{CostTracker, Context};

fn optimize_verification_strategy() {
    let ctx = Context::new();
    let mut tracker = CostTracker::new();

    // Verify multiple functions
    // ... verification loop ...

    let report = tracker.report();

    // Output might suggest:
    // "Function complex_algorithm() took 15s to verify"
    // "Suggestion: Use @verify(runtime) to reduce build time by 15s"
}
```

### Example 6: Incremental Verification

```rust
// When building incrementally, only verify changed functions
fn incremental_build() {
    // In practice, Verum compiler will:
    // 1. Hash function signatures and bodies
    // 2. Skip verification if hash matches cached result
    // 3. Only verify modified or new functions

    // This dramatically improves iteration time during development
}
```

## Best Practices

### 1. Start with Runtime Checks

```verum
// Begin with runtime validation
@verify(runtime)
fn process_data(x: Int{> 0}, y: Int{!= 0}) -> Int {
    x / y
}
```

### 2. Gradually Add Proofs

```verum
// Once stable, try proof mode
@verify(proof)
fn process_data(x: Int{> 0}, y: Int{!= 0}) -> Int {
    x / y
}
```

### 3. Use Auto Mode for Balance

```verum
// Let compiler decide
@verify(auto)
fn process_data(x: Int{> 0}, y: Int{!= 0}) -> Int {
    x / y
}
```

### 4. Handle Verification Failures

When verification fails:

1. **Read the counterexample**: It shows a concrete case where your constraint fails
2. **Check suggestions**: Automatically generated hints for fixes
3. **Strengthen preconditions**: Add more constraints to input types
4. **Weaken postconditions**: Relax output constraints if too strict
5. **Use runtime checks**: For complex cases, `@verify(runtime)` is often best

### 5. Monitor Verification Costs

```bash
# Generate cost report
verum build --emit-verify-stats

# Review slow verifications
# Consider @verify(runtime) for functions > 5s
```

## License

See the Verum project root for license information.

## References

- Verum Verification System: safety-first static verification for AOT optimization. Three levels: `@verify(runtime)` (default, runtime checks), `@verify(static)` (dataflow analysis), `@verify(proof)` (SMT solver proof, 0ns runtime). CBGR checking: 15-50ns per dereference for managed references `&T`.
- Formal Proofs Extension (v2.0+): full proof assistant with tactics (simp, ring, omega, blast), machine-checkable certificates, program extraction from constructive proofs via Curry-Howard correspondence.
- [Z3 Documentation](https://github.com/Z3Prover/z3)
- [SMT-LIB Standard](http://smtlib.cs.uiowa.edu/)

## Acknowledgments

Built with the Z3 Theorem Prover from Microsoft Research.
