# Verum Gradual Verification System

Comprehensive implementation of Verum's gradual verification system as specified in `docs/detailed/09-verification-system.md`.

## Overview

The gradual verification system provides a smooth transition from runtime checking to compile-time formal verification through three verification levels:

1. **Runtime (dynamic)**: Quick runtime checks with ~5-15ns overhead
2. **Static (compile-time)**: SMT verification at compile time, 0ns runtime
3. **Proof (formal)**: Full formal proofs with proof certificates

## Architecture

### Core Components

- **`level`**: Verification level types (Runtime, Static, Proof)
- **`context`**: Verification context and scope tracking
- **`transition`**: Gradual transition analysis and recommendation
- **`cost`**: Cost tracking, reporting, and budget enforcement
- **`boundary`**: Trusted/untrusted code boundary management
- **`integration`**: Integration with type system, SMT, and codegen
- **`passes`**: Compiler passes for verification

### Three-Level System

```rust
use verum_verification::*;

// Level 1: Runtime verification (default)
#[verify(runtime)]
fn withdraw(balance: Float, amount: Positive) -> Float {
    balance - amount  // Refinement validation at runtime
}

// Level 2: Static verification (AOT optimization)
#[verify(static)]
fn fast_withdraw(balance: Float, amount: Float) -> Float {
    balance - amount  // Checks eliminated when proven safe
}

// Level 3: Proof verification (formal)
#[verify(proof)]
fn verified_withdraw(
    balance: Float{>= 0},
    amount: Float{> 0 && it <= balance}
) -> Float{it == balance - amount && it >= 0} {
    balance - amount  // Proven safe by SMT solver
}
```

## Gradual Transition

The system supports seamless migration between verification levels:

```rust
use verum_verification::{TransitionAnalyzer, TransitionStrategy, CodeMetrics};

let analyzer = TransitionAnalyzer::new(TransitionStrategy::Balanced);

// Analyze code for transition opportunity
let mut metrics = CodeMetrics::default();
metrics.test_coverage = 0.95;
metrics.change_frequency_per_week = 0.1;
metrics.execution_frequency = 1000.0;

let decision = analyzer.analyze_function(
    &"hot_function",
    VerificationLevel::Runtime,
    &metrics,
);

if decision.recommend {
    println!("Recommend transition: {} -> {}",
        decision.from, decision.to);
    println!("Confidence: {:.0}%", decision.confidence * 100.0);
    println!("Expected benefit: {:.1}%", decision.expected_benefit_percent);
}
```

## Verification Context

Track verification levels across scopes:

```rust
use verum_verification::{VerificationContext, VerificationMode};

let mut ctx = VerificationContext::new();

// Push a static verification scope
let scope_id = ctx.push_scope(
    VerificationMode::static_mode(),
    "optimized_function".into(),
);

// Current verification level
assert_eq!(ctx.current_level(), VerificationLevel::Static);

// Pop back to parent scope
ctx.pop_scope().unwrap();
```

## Cost Tracking

Monitor and report verification costs:

```rust
use verum_verification::{VerificationCost, CostReport, CostThreshold};
use std::time::Duration;

// Record costs
let mut costs = List::new();
costs.push(VerificationCost::new(
    "function1".into(),
    VerificationLevel::Static,
    Duration::from_millis(100),
    10,   // SMT queries
    true, // success
    50,   // problem size
));

// Generate report
let report = CostReport::from_costs(
    costs,
    Some(CostThreshold::static_default()),
);

println!("{}", report.format());
```

## Verification Boundaries

Handle boundaries between code at different verification levels:

```rust
use verum_verification::{VerificationBoundary, BoundaryKind};

let boundary = VerificationBoundary {
    id: BoundaryId::new(0),
    from_level: VerificationLevel::Runtime,
    to_level: VerificationLevel::Proof,
    kind: BoundaryKind::FunctionCall,
    obligations: List::new(),
};

// Check if proof obligations required
if boundary.requires_obligations() {
    // Generate and verify proof obligations
}
```

## Integration

### Type System Integration

```rust
use verum_verification::TypeSystemIntegration;

// Check if type requires verification
if TypeSystemIntegration::requires_verification(&ty) {
    let level = TypeSystemIntegration::recommend_level(&ty);
    // Apply verification
}
```

### SMT Integration

```rust
use verum_verification::SmtIntegration;

let smt_mode = SmtIntegration::to_smt_mode(VerificationLevel::Proof);
// Use with verum_smt solver
```

### Codegen Integration

```rust
use verum_verification::CodegenIntegration;

// Determine if runtime checks should be emitted
let emit_checks = CodegenIntegration::emit_runtime_checks(
    VerificationLevel::Static,
    proven_safe,
);

// Get optimization level
let opt_level = CodegenIntegration::optimization_level(
    VerificationLevel::Proof,
);
```

## Verification Passes

Run verification passes in the compiler pipeline:

```rust
use verum_verification::{VerificationPipeline, VerificationContext};

let mut pipeline = VerificationPipeline::default_pipeline();
let mut ctx = VerificationContext::new();

let results = pipeline.run_all(&module, &mut ctx)?;

for result in results {
    println!("Pass completed: {} functions verified in {:.2}s",
        result.functions_verified,
        result.duration.as_secs_f64());
}
```

## Performance Characteristics

### Runtime Overhead

| Level | CBGR Overhead | Refinement Checks | Total Overhead |
|-------|---------------|-------------------|----------------|
| Runtime | ~15ns (Tier 1-3) | ~5ns per check | ~20ns |
| Static | 0ns (proven safe) | 0ns (proven safe) | 0ns |
| Proof | 0ns (proven safe) | 0ns (proven safe) | 0ns |

### Compile-Time Overhead

| Level | Overhead | Use Case |
|-------|----------|----------|
| Runtime | +0% | Development, prototyping |
| Static | +10-20% | Production hot paths |
| Proof | +200-1000% | Critical code, formal verification |

## Transition Strategies

### Conservative
- High confidence threshold (95%)
- Low cost increase tolerance (10%)
- Best for critical systems

### Balanced (Default)
- Moderate confidence threshold (80%)
- Moderate cost tolerance (20%)
- Best for general development

### Aggressive
- Low confidence threshold (60%)
- High cost tolerance (50%)
- Best for maximizing static verification

### Manual
- Requires explicit user annotation
- No automatic transitions
- Full user control

## Examples

### Complete Workflow

```rust
use verum_verification::*;

// 1. Create verification context
let mut ctx = VerificationContext::new();

// 2. Start with runtime verification
let func_id = ctx.push_scope(
    VerificationMode::runtime(),
    "user_function".into(),
);

// 3. Collect metrics over time
let mut metrics = CodeMetrics::default();
metrics.test_coverage = 0.95;
metrics.change_frequency_per_week = 0.1;

// 4. Analyze for transition
let analyzer = TransitionAnalyzer::new(TransitionStrategy::Balanced);
let decision = analyzer.analyze_function(
    &"user_function".into(),
    VerificationLevel::Runtime,
    &metrics,
);

// 5. Apply recommendation
if decision.recommend && decision.passes_threshold(&TransitionStrategy::Balanced) {
    ctx.pop_scope().unwrap();
    ctx.push_scope(
        VerificationMode::new(decision.to),
        "user_function".into(),
    );
}

// 6. After more maturity, transition to proof
metrics.criticality_score = 9;
metrics.change_frequency_per_week = 0.05;

let proof_decision = analyzer.analyze_function(
    &"user_function".into(),
    VerificationLevel::Static,
    &metrics,
);

if proof_decision.recommend {
    ctx.pop_scope().unwrap();
    ctx.push_scope(
        VerificationMode::proof(),
        "user_function".into(),
    );
}
```

## Testing

Run tests with:

```bash
cargo test -p verum_verification
```

Comprehensive integration tests cover:
- Verification level transitions
- Boundary detection and proof obligations
- Cost tracking and reporting
- Transition recommendations
- Integration with type system and SMT

## Specification Compliance

This implementation follows:
- **docs/detailed/09-verification-system.md** - Complete verification system spec
- **docs/detailed/03-type-system.md Section 1.3** - Refinement types integration

All features are compliant with the Verum v6.0-BALANCED specification.

## Future Enhancements

- [ ] Interactive proof mode for complex properties
- [ ] Proof certificate export (Coq/Lean format)
- [ ] Machine learning-based transition recommendations
- [ ] Automatic loop invariant inference
- [ ] Parallel SMT solving for large codebases
- [ ] Integration with full dependent type system

## License

Apache-2.0 - See repository root for details.
