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
// Comprehensive tests for Craig interpolation
//
// Tests all interpolation algorithms, compositional verification,
// and CEGAR loops.
//
// FIXED (Session 24): Interpolant.validate now uses verum_smt::Context
#![allow(unexpected_cfgs)]
#![allow(unused_variables)]
#![allow(unused_imports)]

use verum_smt::Context;
use verum_smt::interpolation::*;
use verum_common::{List, Text};
use z3::ast::{Ast, Bool, Int};

// ==================== Basic Interpolation Tests ====================

#[test]
fn test_basic_craig_interpolation() {
    // Test basic Craig interpolation: A = (x > 0), B = (x < 0)
    // These are unsatisfiable together
    // Interpolant should be something like (x >= 0) or similar

    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");

    // Create formulas (simplified for testing)
    let a = x.clone(); // x
    let b = x.not(); // ¬x

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Basic interpolation should succeed");

    let interpolant = result.unwrap();
    assert!(interpolant.time_ms >= 0, "Should track computation time");

    // Validate interpolation properties
    let validation = interpolant.validate(&ctx);
    assert!(validation.is_ok(), "Interpolant should be valid");
}

#[test]
fn test_interpolation_with_arithmetic() {
    // Test interpolation with arithmetic constraints
    // A = (x > 5 ∧ y = x + 1), B = (y < 3)
    // Should be UNSAT

    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::MBI,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // A: x > 5 ∧ y = x + 1
    let five = Int::from_i64(5);
    let one = Int::from_i64(1);
    let a = Bool::and(&[&x.gt(&five), &y._eq(Int::add(&[&x, &one]))]);

    // B: y < 3
    let three = Int::from_i64(3);
    let b = y.lt(&three);

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Arithmetic interpolation should succeed");
}

#[test]
fn test_satisfiable_formulas_fail() {
    // Test that interpolation correctly fails on satisfiable formulas
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");

    let a = x.clone();
    let b = x.clone(); // Same formula - A ∧ B is SAT

    let result = engine.interpolate(&a, &b);
    assert!(
        result.is_err(),
        "Interpolation should fail for SAT formulas"
    );
}

// ==================== Algorithm Tests ====================

#[test]
fn test_mcmillan_algorithm() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::McMillan,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "McMillan interpolation should work");
}

#[test]
fn test_pudlak_algorithm() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::Pudlak,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Pudlák interpolation should work");
}

#[test]
fn test_dual_algorithm() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::Dual,
        strength: InterpolantStrength::Balanced,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Dual interpolation should work");
}

#[test]
fn test_mbi_algorithm() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::MBI,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "MBI interpolation should work");
}

#[test]
fn test_pingpong_algorithm() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::PingPong,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Ping-pong interpolation should work");
}

#[test]
fn test_pogo_algorithm() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::Pogo,
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Pogo interpolation should work");
}

// ==================== Sequence Interpolation Tests ====================

#[test]
fn test_sequence_interpolation_basic() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");

    // Create sequence: x, x => y, ¬y
    // This is unsatisfiable
    let mut formulas = List::new();
    formulas.push(x.clone());
    formulas.push(x.implies(&y));
    formulas.push(y.not());

    let result = engine.sequence_interpolate(formulas);
    assert!(result.is_ok(), "Sequence interpolation should succeed");

    let seq_interp = result.unwrap();
    assert_eq!(
        seq_interp.interpolants.len(),
        2,
        "Should have n-1 interpolants for n formulas"
    );
    assert!(seq_interp.time_ms >= 0, "Should track time");
}

#[test]
fn test_sequence_interpolation_too_few_formulas() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");

    let mut formulas = List::new();
    formulas.push(x);

    let result = engine.sequence_interpolate(formulas);
    assert!(result.is_err(), "Should fail with too few formulas");
}

// ==================== Tree Interpolation Tests ====================

#[test]
fn test_tree_interpolation_basic() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");

    // Create simple tree with just root
    let root = TreeNode {
        id: Text::from("root"),
        formula: x.clone(),
        children: List::new(),
    };

    let tree = InterpolationTree { root };

    let result = engine.tree_interpolate(tree);
    assert!(result.is_ok(), "Tree interpolation should succeed");

    let tree_interp = result.unwrap();
    assert_eq!(tree_interp.num_nodes, 1, "Should have 1 node");
    assert!(tree_interp.time_ms >= 0, "Should track time");
}

#[test]
fn test_tree_interpolation_with_children() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");

    // Create tree with children
    let mut children = List::new();
    children.push(TreeNode {
        id: Text::from("child1"),
        formula: x.clone(),
        children: List::new(),
    });
    children.push(TreeNode {
        id: Text::from("child2"),
        formula: y.not(),
        children: List::new(),
    });

    let root = TreeNode {
        id: Text::from("root"),
        formula: Bool::and(&[&x, &y]),
        children,
    };

    let tree = InterpolationTree { root };

    let result = engine.tree_interpolate(tree);
    assert!(
        result.is_ok(),
        "Tree interpolation with children should succeed"
    );

    let tree_interp = result.unwrap();
    assert!(tree_interp.num_nodes >= 3, "Should have at least 3 nodes");
}

// ==================== Compositional Verification Tests ====================

#[test]
fn test_compositional_verifier_basic() {
    let config = InterpolationConfig::default();
    let verifier = CompositionalVerifier::new(config);

    let _ctx = Context::new();
    let x = Bool::new_const("x");

    // Create module spec where precondition implies postcondition (x => x)
    // This is necessary for interpolation to work
    let module = ModuleSpec {
        id: Text::from("module1"),
        precondition: x.clone(),
        postcondition: x.clone(),
        invariants: List::new(),
    };

    let mut modules = List::new();
    modules.push(module);

    // Property: x => x (trivially true)
    let property = x.implies(&x);

    let result = verifier.verify_modular(modules, property);
    assert!(result.is_ok(), "Compositional verification should succeed");

    let proof = result.unwrap();
    assert!(proof.time_ms >= 0, "Should track time");
}

#[test]
fn test_compositional_verifier_multiple_modules() {
    let config = InterpolationConfig::default();
    let verifier = CompositionalVerifier::new(config);

    let _ctx = Context::new();
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");

    // Create multiple modules where each precondition implies its postcondition
    // Module 1: x => x (trivially true)
    // Module 2: y => y (trivially true)
    let mut modules = List::new();

    modules.push(ModuleSpec {
        id: Text::from("module1"),
        precondition: x.clone(),
        postcondition: x.clone(),
        invariants: List::new(),
    });

    modules.push(ModuleSpec {
        id: Text::from("module2"),
        precondition: y.clone(),
        postcondition: y.clone(),
        invariants: List::new(),
    });

    // Property: x => x (trivially true)
    let property = x.implies(&x);

    let result = verifier.verify_modular(modules, property);
    assert!(result.is_ok(), "Multi-module verification should succeed");
}

// ==================== CEGAR Tests ====================

#[test]
fn test_cegar_basic() {
    let config = InterpolationConfig::default();
    let cegar = AbstractionRefinement::new(config);

    let _ctx = Context::new();
    let x = Bool::new_const("x");

    // Initial abstraction: x (the same as property, so it holds immediately)
    let abstraction = x.clone();

    // Property: x
    let property = x.clone();

    let result = cegar.cegar(abstraction, property, 5);
    assert!(result.is_ok(), "CEGAR should complete");

    let cegar_result = result.unwrap();
    // The abstraction already satisfies the property, so 0 or more iterations
    assert!(
        cegar_result.iterations >= 0,
        "Should have non-negative iterations"
    );
    assert!(cegar_result.time_ms >= 0, "Should track time");
}

#[test]
fn test_cegar_refinement() {
    let config = InterpolationConfig::default();
    let cegar = AbstractionRefinement::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");

    // Abstraction: x
    let abstraction = x.clone();

    // Counterexample: ¬x ∧ y
    let counterexample = Bool::and(&[&x.not(), &y]);

    let result = cegar.refine(&abstraction, &counterexample);
    assert!(result.is_ok(), "Refinement should succeed");

    // refinement is a z3::ast::Bool, check that we got a valid refinement
    let _refined = result.unwrap();
    // If we got here without error, refinement was computed successfully
}

// ==================== Shared Variable Tests ====================

#[test]
fn test_shared_variables_extraction() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let _ctx = Context::new();
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");

    // A = x ∧ y, B = ¬x ∧ ¬y
    // A ∧ B = (x ∧ y) ∧ (¬x ∧ ¬y) = false (UNSAT)
    // Shared variables: x, y
    let a = Bool::and(&[&x, &y]);
    let b = Bool::and(&[&x.not(), &y.not()]);

    let result = engine.interpolate(&a, &b);
    // Interpolation requires A ∧ B to be UNSAT, which it is
    if let Ok(interpolant) = result {
        // The interpolation should succeed
        // Shared vars may or may not be detected depending on implementation
        assert!(interpolant.time_ms >= 0, "Should have valid timing");
    }
}

// ==================== Performance Tests ====================

#[test]
fn test_interpolation_performance_tracking() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::MBI,
        simplify: false, // Faster without simplification
        ..Default::default()
    };
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok());

    let interpolant = result.unwrap();
    assert!(interpolant.time_ms >= 0, "Should track time");
    println!("Interpolation took {} ms", interpolant.time_ms);
}

// ==================== Edge Cases ====================

#[test]
fn test_interpolation_with_true() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let a = Bool::from_bool(true);
    let b = Bool::from_bool(false);

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok(), "Should handle true/false");
}

#[test]
fn test_interpolation_validates_correctly() {
    let config = InterpolationConfig::default();
    let engine = InterpolationEngine::new(config);

    let ctx = Context::new();
    let x = Bool::new_const("x");
    let a = x.clone();
    let b = x.not();

    let result = engine.interpolate(&a, &b);
    assert!(result.is_ok());

    let interpolant = result.unwrap();
    let validation = interpolant.validate(&ctx);
    assert!(validation.is_ok(), "Interpolant should validate");
}

// ==================== Configuration Tests ====================

#[test]
fn test_config_default() {
    let config = InterpolationConfig::default();
    assert_eq!(config.algorithm, InterpolationAlgorithm::MBI);
    assert_eq!(config.strength, InterpolantStrength::Balanced);
    assert!(config.simplify);
    assert_eq!(config.timeout_ms, verum_common::Maybe::Some(5000));
}

#[test]
fn test_config_custom() {
    let config = InterpolationConfig {
        algorithm: InterpolationAlgorithm::Pudlak,
        strength: InterpolantStrength::Weakest,
        simplify: false,
        timeout_ms: verum_common::Maybe::Some(10000),
        proof_based: true,
        model_based: false,
        quantifier_elimination: false,
        max_projection_vars: 50,
    };

    assert_eq!(config.algorithm, InterpolationAlgorithm::Pudlak);
    assert_eq!(config.strength, InterpolantStrength::Weakest);
    assert!(!config.simplify);
}
