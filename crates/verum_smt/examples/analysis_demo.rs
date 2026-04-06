//! Analysis Module Demonstration
//!
//! Demonstrates the formal verification capabilities of the Analysis module.
//!
//! This example shows:
//! 1. Complete ordered field verification (real numbers)
//! 2. Function continuity verification
//! 3. Limit computation and verification
//! 4. Sequence convergence analysis
//! 5. Key analysis theorems (IVT, EVT)

use verum_smt::Context;
use verum_smt::analysis::{
    AnalysisVerifier, CompleteOrderedField, Limit, RealFunction, RealSequence,
    standard_functions,
};

fn main() {
    println!("=== Verum Analysis Module Demonstration ===\n");

    // Create Z3 context
    let ctx = Context::new();

    demo_complete_ordered_field(&ctx);
    demo_limits(&ctx);
    demo_continuity(&ctx);
    demo_sequences(&ctx);
    demo_intermediate_value_theorem(&ctx);
    demo_extreme_value_theorem(&ctx);

    println!("\n=== All demonstrations completed successfully! ===");
}

fn demo_complete_ordered_field(ctx: &Context) {
    println!("1. Complete Ordered Field (Real Numbers)");
    println!("   Completeness axiom: Every bounded set has a supremum\n");

    let mut field = CompleteOrderedField::reals();

    // Verify completeness for a bounded set
    let set = vec![1.0, 1.5, 1.9, 1.99, 1.999];
    println!("   Set: {:?}", set);

    match field.verify_completeness(ctx, &set) {
        Ok(proof) => {
            println!("   ✓ Completeness verified: supremum exists");
            println!("   Proof: {:?}\n", proof);
        }
        Err(e) => println!("   ✗ Verification failed: {}\n", e),
    }
}

fn demo_limits(ctx: &Context) {
    println!("2. Limits (Epsilon-Delta Definition)");
    println!("   lim_{{x → a}} f(x) = L\n");

    // Example: lim_{x -> 2} (3x + 1) = 7
    let f = RealFunction::linear(3.0, 1.0);
    let mut limit = Limit::new(f, 2.0, 7.0);

    println!("   Function: f(x) = 3x + 1");
    println!("   Point: x = 2");
    println!("   Expected limit: 7");

    match limit.verify(ctx) {
        Ok(proof) => {
            println!("   ✓ Limit verified using ε-δ definition");
            println!("   Proof: {:?}\n", proof);
        }
        Err(e) => println!("   ✗ Verification failed: {}\n", e),
    }
}

fn demo_continuity(ctx: &Context) {
    println!("3. Continuity at a Point");
    println!("   f is continuous at a if lim_{{x → a}} f(x) = f(a)\n");

    let mut verifier = AnalysisVerifier::new();

    // Example: f(x) = x^2 is continuous at x = 3
    let f = standard_functions::square();
    println!("   Function: f(x) = x²");
    println!("   Point: x = 3");

    match verifier.verify_continuity_at(ctx, &f, 3.0) {
        Ok(proof) => {
            println!("   ✓ Continuity verified at x = 3");
            println!("   Proof: {:?}\n", proof);
        }
        Err(e) => println!("   ✗ Verification failed: {}\n", e),
    }
}

fn demo_sequences(ctx: &Context) {
    println!("4. Sequence Convergence");
    println!("   Verify that a sequence converges to a limit\n");

    // Sequence converging to 2: 1, 1.5, 1.75, 1.875, ...
    let terms = vec![1.0, 1.5, 1.75, 1.875, 1.9375, 1.96875, 1.984375, 1.9921875];
    println!("   Sequence: {:?}...", &terms[..5]);
    println!("   Expected limit: 2.0");

    let mut seq = RealSequence::new("converging_to_2", terms.into_iter().collect());

    // Check properties
    println!("   Bounded: {}", seq.is_bounded());
    println!("   Cauchy (ε=0.1): {}", seq.is_cauchy(0.1));

    match seq.verify_convergence(ctx, 2.0) {
        Ok(proof) => {
            println!("   ✓ Convergence verified");
            println!("   Proof: {:?}\n", proof);
        }
        Err(e) => println!("   ✗ Verification failed: {}\n", e),
    }
}

fn demo_intermediate_value_theorem(ctx: &Context) {
    println!("5. Intermediate Value Theorem (IVT)");
    println!("   If f continuous on [a,b] with f(a)<0<f(b),");
    println!("   then ∃c ∈ (a,b) such that f(c) = 0\n");

    let mut verifier = AnalysisVerifier::new();

    // f(x) = x² - 4
    // f(1) = -3 < 0, f(3) = 5 > 0
    // Root at x = 2
    let f = RealFunction::quadratic(1.0, 0.0, -4.0);

    println!("   Function: f(x) = x² - 4");
    println!("   Interval: [1, 3]");
    println!("   f(1) = -3 < 0");
    println!("   f(3) = 5 > 0");

    match verifier.verify_intermediate_value_theorem(ctx, &f, 1.0, 3.0) {
        Ok(proof) => {
            println!("   ✓ IVT verified: root found in interval");
            println!("   Proof: {:?}\n", proof);
        }
        Err(e) => println!("   ✗ Verification failed: {}\n", e),
    }
}

fn demo_extreme_value_theorem(ctx: &Context) {
    println!("6. Extreme Value Theorem (EVT)");
    println!("   If f continuous on [a,b], then f attains");
    println!("   its maximum and minimum on [a,b]\n");

    let mut verifier = AnalysisVerifier::new();

    // f(x) = -x² + 4 on [0, 3]
    let f = RealFunction::quadratic(-1.0, 0.0, 4.0);

    println!("   Function: f(x) = -x² + 4");
    println!("   Interval: [0, 3]");
    println!("   Maximum at x=0: f(0) = 4");
    println!("   Minimum at x=3: f(3) = -5");

    match verifier.verify_extreme_value_theorem(ctx, &f, 0.0, 3.0) {
        Ok(proof) => {
            println!("   ✓ EVT verified: extrema found");
            println!("   Proof: {:?}\n", proof);
        }
        Err(e) => println!("   ✗ Verification failed: {}\n", e),
    }
}
