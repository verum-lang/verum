#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Tests for Analysis Module
//!
//! Tests formal verification of real analysis theorems and properties.
//!
//! Real analysis verification: limits, continuity, intermediate value theorem.
//! Uses epsilon-delta definitions: `limit(f, a, L) = forall eps > 0. exists delta > 0.
//! forall x. 0 < |x - a| < delta => |f(x) - L| < eps`. Continuous at a means
//! `limit(f, a, f(a))`. Requires CompeteOrderedField axioms (completeness/supremum).

use verum_smt::Context;
use verum_smt::analysis::{
    AnalysisVerifier, CompleteOrderedField, Continuity, Limit, RealFunction, RealSequence,
    UniformContinuity, standard_functions,
};

// ==================== Complete Ordered Field Tests ====================

#[test]
fn test_completeness_axiom() {
    let ctx = Context::new();
    let mut field = CompleteOrderedField::reals();

    // Verify completeness for a bounded set
    let set = vec![1.0, 1.5, 1.9, 1.99, 1.999];
    let result = field.verify_completeness(&ctx, &set);

    assert!(
        result.is_ok(),
        "Completeness axiom should hold for bounded set"
    );
}

#[test]
fn test_completeness_empty_set() {
    let ctx = Context::new();
    let mut field = CompleteOrderedField::reals();

    // Empty set should fail
    let set = vec![];
    let result = field.verify_completeness(&ctx, &set);

    assert!(result.is_err(), "Empty set should not have supremum");
}

// ==================== Real Function Tests ====================

#[test]
fn test_polynomial_evaluation() {
    // f(x) = 3x^2 + 2x + 1
    let f = RealFunction::polynomial(vec![1.0, 2.0, 3.0].into_iter().collect());

    assert_eq!(f.evaluate(0.0).unwrap(), 1.0); // f(0) = 1
    assert_eq!(f.evaluate(1.0).unwrap(), 6.0); // f(1) = 3 + 2 + 1 = 6
    assert_eq!(f.evaluate(2.0).unwrap(), 17.0); // f(2) = 12 + 4 + 1 = 17
}

#[test]
fn test_rational_function() {
    // f(x) = x / (x + 1)
    let f = RealFunction::Rational {
        numerator: vec![0.0, 1.0].into_iter().collect(), // x
        denominator: vec![1.0, 1.0].into_iter().collect(), // x + 1
    };

    assert_eq!(f.evaluate(0.0).unwrap(), 0.0); // 0 / 1 = 0
    assert_eq!(f.evaluate(1.0).unwrap(), 0.5); // 1 / 2 = 0.5
    assert_eq!(f.evaluate(3.0).unwrap(), 0.75); // 3 / 4 = 0.75
}

#[test]
fn test_rational_function_undefined() {
    // f(x) = 1 / x (undefined at x = 0)
    let f = RealFunction::Rational {
        numerator: vec![1.0].into_iter().collect(),        // 1
        denominator: vec![0.0, 1.0].into_iter().collect(), // x
    };

    assert!(f.evaluate(0.0).is_err(), "Should be undefined at x = 0");
    assert!(f.evaluate(1.0).is_ok());
}

#[test]
fn test_standard_functions() {
    let id = standard_functions::identity();
    assert_eq!(id.evaluate(5.0).unwrap(), 5.0);

    let sq = standard_functions::square();
    assert_eq!(sq.evaluate(3.0).unwrap(), 9.0);

    let cube = standard_functions::cube();
    assert_eq!(cube.evaluate(2.0).unwrap(), 8.0);
}

// ==================== Limit Tests ====================

#[test]
fn test_limit_constant_function() {
    let ctx = Context::new();

    // lim_{x -> 5} 7 = 7
    let f = RealFunction::constant(7.0);
    let mut limit = Limit::new(f, 5.0, 7.0);

    let result = limit.verify(&ctx);
    assert!(result.is_ok(), "Constant function should have limit");
}

#[test]
fn test_limit_linear_function() {
    let ctx = Context::new();

    // lim_{x -> 2} (3x + 1) = 7
    let f = RealFunction::linear(3.0, 1.0);
    let mut limit = Limit::new(f, 2.0, 7.0);

    let result = limit.verify(&ctx);
    assert!(result.is_ok(), "Linear function should have limit");
}

#[test]
fn test_limit_quadratic_function() {
    let ctx = Context::new();

    // lim_{x -> 3} x^2 = 9
    let f = standard_functions::square();
    let mut limit = Limit::new(f, 3.0, 9.0);

    let result = limit.verify(&ctx);
    assert!(result.is_ok(), "Quadratic function should have limit");
}

#[test]
fn test_limit_wrong_value() {
    let ctx = Context::new();

    // lim_{x -> 2} x^2 should be 4, not 5
    let f = standard_functions::square();
    let mut limit = Limit::new(f, 2.0, 5.0);

    let result = limit.verify(&ctx);
    assert!(result.is_err(), "Wrong limit value should fail");
}

// ==================== Continuity Tests ====================

#[test]
fn test_continuity_constant() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    let f = RealFunction::constant(42.0);
    let result = verifier.verify_continuity_at(&ctx, &f, 0.0);

    assert!(result.is_ok(), "Constant function is continuous");
}

#[test]
fn test_continuity_linear() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    let f = RealFunction::linear(2.0, 3.0); // f(x) = 2x + 3
    let result = verifier.verify_continuity_at(&ctx, &f, 5.0);

    assert!(result.is_ok(), "Linear function is continuous");
}

#[test]
fn test_continuity_polynomial() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // f(x) = x^3 - 2x^2 + x - 1
    let f = RealFunction::polynomial(vec![-1.0, 1.0, -2.0, 1.0].into_iter().collect());
    let result = verifier.verify_continuity_at(&ctx, &f, 1.0);

    assert!(result.is_ok(), "Polynomial is continuous");
}

#[test]
fn test_continuity_object() {
    let ctx = Context::new();

    // Direct continuity verification
    let f = standard_functions::square();
    let mut cont = Continuity::new(f, 2.0);

    let result = cont.verify(&ctx);
    assert!(result.is_ok(), "x^2 is continuous at x=2");
}

// ==================== Sequence Tests ====================

#[test]
fn test_sequence_bounded() {
    let seq = RealSequence::new(
        "test_bounded",
        vec![1.0, 2.0, 1.5, 1.8, 1.6, 1.7].into_iter().collect(),
    );

    assert!(seq.is_bounded(), "Finite sequence should be bounded");
}

#[test]
fn test_sequence_cauchy() {
    // Sequence converging to 2: 1, 1.5, 1.75, 1.875, 1.9375, ...
    let seq = RealSequence::new(
        "test_cauchy",
        vec![1.0, 1.5, 1.75, 1.875, 1.9375, 1.96875]
            .into_iter()
            .collect(),
    );

    assert!(seq.is_cauchy(0.5), "Convergent sequence should be Cauchy");
}

#[test]
fn test_sequence_convergence() {
    let ctx = Context::new();

    let seq = RealSequence::new(
        "converging_to_2",
        vec![1.0, 1.5, 1.75, 1.875, 1.9375, 1.96875, 1.984375, 1.9921875]
            .into_iter()
            .collect(),
    );

    let mut seq_mut = seq.clone();
    let result = seq_mut.verify_convergence(&ctx, 2.0);

    assert!(result.is_ok(), "Sequence should converge to 2");
}

#[test]
fn test_sequence_not_convergent() {
    let ctx = Context::new();

    // Oscillating sequence: 1, -1, 1, -1, ...
    let seq = RealSequence::new(
        "oscillating",
        vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0].into_iter().collect(),
    );

    let mut seq_mut = seq.clone();
    let result = seq_mut.verify_convergence(&ctx, 0.0);

    assert!(result.is_err(), "Oscillating sequence should not converge");
}

// ==================== Theorem Verification Tests ====================

#[test]
fn test_intermediate_value_theorem() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // f(x) = x^2 - 4
    // f(1) = -3 < 0, f(3) = 5 > 0
    // So there exists c in (1, 3) with f(c) = 0 (namely c = 2)
    let f = RealFunction::quadratic(1.0, 0.0, -4.0);

    let result = verifier.verify_intermediate_value_theorem(&ctx, &f, 1.0, 3.0);

    assert!(
        result.is_ok(),
        "IVT should find zero between roots of x^2 - 4"
    );
}

#[test]
fn test_ivt_no_sign_change() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // f(x) = x^2 + 1 (always positive)
    let f = RealFunction::quadratic(1.0, 0.0, 1.0);

    let result = verifier.verify_intermediate_value_theorem(&ctx, &f, 0.0, 2.0);

    assert!(result.is_err(), "IVT should fail when no sign change");
}

#[test]
fn test_extreme_value_theorem() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // f(x) = -x^2 + 4 on [0, 3]
    // Minimum at x=3: f(3) = -5
    // Maximum at x=0: f(0) = 4
    let f = RealFunction::quadratic(-1.0, 0.0, 4.0);

    let result = verifier.verify_extreme_value_theorem(&ctx, &f, 0.0, 3.0);

    assert!(result.is_ok(), "EVT should find extrema on closed interval");
}

#[test]
fn test_bolzano_weierstrass() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // Bounded sequence should have convergent subsequence
    let seq = RealSequence::new(
        "bounded",
        vec![1.0, 2.0, 1.5, 1.8, 1.6, 1.7, 1.65, 1.68]
            .into_iter()
            .collect(),
    );

    let result = verifier.verify_bolzano_weierstrass(&ctx, &seq);

    assert!(
        result.is_ok(),
        "Bolzano-Weierstrass should hold for bounded sequence"
    );
}

#[test]
fn test_cauchy_completeness() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // Cauchy sequence should converge
    let seq = RealSequence::new(
        "cauchy",
        vec![1.0, 1.5, 1.75, 1.875, 1.9375, 1.96875]
            .into_iter()
            .collect(),
    );

    let result = verifier.verify_cauchy_completeness(&ctx, &seq);

    assert!(
        result.is_ok(),
        "Completeness: Cauchy sequence should converge"
    );
}

// ==================== Uniform Continuity Tests ====================

#[test]
fn test_uniform_continuity_linear() {
    let ctx = Context::new();

    // f(x) = 2x is uniformly continuous on [0, 1]
    let f = RealFunction::linear(2.0, 0.0);
    let mut unif = UniformContinuity::new(f, 0.0, 1.0);

    let result = unif.verify(&ctx);
    assert!(
        result.is_ok(),
        "Linear function is uniformly continuous on compact interval"
    );
}

#[test]
fn test_uniform_continuity_quadratic() {
    let ctx = Context::new();

    // f(x) = x^2 is uniformly continuous on [0, 2]
    let f = standard_functions::square();
    let mut unif = UniformContinuity::new(f, 0.0, 2.0);

    let result = unif.verify(&ctx);
    assert!(
        result.is_ok(),
        "x^2 is uniformly continuous on compact interval"
    );
}

// ==================== Integration Tests ====================

#[test]
fn test_complete_analysis_workflow() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    // Define function f(x) = x^2
    let f = standard_functions::square();

    // 1. Verify continuity at several points
    for &x in &[0.0, 1.0, 2.0, 3.0] {
        let result = verifier.verify_continuity_at(&ctx, &f, x);
        assert!(result.is_ok(), "x^2 continuous at x={}", x);
    }

    // 2. Verify IVT: f(x) = x^2 - 2 has root in (1, 2)
    let f_shifted = RealFunction::quadratic(1.0, 0.0, -2.0);
    let ivt_result = verifier.verify_intermediate_value_theorem(&ctx, &f_shifted, 1.0, 2.0);
    assert!(ivt_result.is_ok(), "IVT finds root of x^2 - 2");

    // 3. Verify EVT: f(x) = x^2 attains extrema on [0, 2]
    let evt_result = verifier.verify_extreme_value_theorem(&ctx, &f, 0.0, 2.0);
    assert!(evt_result.is_ok(), "EVT finds extrema of x^2");

    // 4. Verify sequence convergence
    let seq = RealSequence::new(
        "squares",
        vec![1.0, 1.414, 1.4142, 1.41421, 1.414213] // approximations of sqrt(2)
            .into_iter()
            .collect(),
    );
    let mut seq_mut = seq;
    let conv_result = seq_mut.verify_convergence(&ctx, 1.414213);
    assert!(conv_result.is_ok(), "Sequence converges to sqrt(2)");
}

#[test]
fn test_field_completeness_axiom() {
    let ctx = Context::new();
    let mut field = CompleteOrderedField::reals();

    // Multiple bounded sets
    let sets = vec![
        vec![1.0, 2.0, 3.0],
        vec![0.5, 0.75, 0.875, 0.9375],
        vec![-1.0, -0.5, 0.0, 0.5, 1.0],
    ];

    for set in sets {
        let result = field.verify_completeness(&ctx, &set);
        assert!(
            result.is_ok(),
            "Completeness should hold for bounded set {:?}",
            set
        );
    }
}

// ==================== Edge Cases ====================

#[test]
fn test_invalid_interval() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    let f = standard_functions::identity();

    // a > b is invalid
    let result = verifier.verify_intermediate_value_theorem(&ctx, &f, 3.0, 1.0);
    assert!(result.is_err(), "Invalid interval should fail");
}

#[test]
fn test_singleton_sequence() {
    let seq = RealSequence::new("singleton", vec![42.0].into_iter().collect());

    assert!(seq.is_bounded(), "Singleton sequence is bounded");
    assert!(
        seq.is_cauchy(0.01),
        "Singleton sequence is trivially Cauchy"
    );
}

#[test]
fn test_empty_sequence() {
    let ctx = Context::new();
    let mut seq = RealSequence::new("empty", vec![].into_iter().collect());

    let result = seq.verify_convergence(&ctx, 0.0);
    assert!(result.is_err(), "Empty sequence cannot converge");
}

// ==================== Performance Tests ====================

#[test]
fn test_large_sequence() {
    // Generate sequence converging to pi
    let n = 100;
    let mut terms = Vec::new();
    let mut approx = 3.0;
    for i in 1..=n {
        approx += ((-1.0_f64).powi(i + 1)) / (2.0 * (i as f64) + 1.0);
        terms.push(approx);
    }

    let seq = RealSequence::new("pi_approx", terms.into_iter().collect());

    assert!(seq.is_bounded(), "Pi approximation is bounded");
    // Note: This is a slow convergence, so Cauchy test might fail with small epsilon
}

#[test]
fn test_many_continuity_checks() {
    let ctx = Context::new();
    let mut verifier = AnalysisVerifier::new();

    let f = RealFunction::polynomial(vec![1.0, 2.0, 3.0, 4.0].into_iter().collect());

    // Verify continuity at many points
    for i in 0..20 {
        let x = (i as f64) / 10.0;
        let result = verifier.verify_continuity_at(&ctx, &f, x);
        assert!(result.is_ok(), "Polynomial continuous at x={}", x);
    }
}
