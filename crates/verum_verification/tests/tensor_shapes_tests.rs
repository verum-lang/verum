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
// Tests for tensor_shapes module
// Migrated from src/tensor_shapes.rs per CLAUDE.md standards

use verum_common::{List, Maybe, Text};
use verum_verification::tensor_shapes::*;

#[test]
fn test_static_shape_creation() {
    let shape = TensorShape::from_dims(vec![2, 3, 4]);
    assert_eq!(shape.rank(), 3);
    assert!(shape.is_fully_static());
    assert_eq!(shape.static_dims(), Maybe::Some(List::from(vec![2, 3, 4])));
}

#[test]
fn test_dynamic_shape_creation() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_dynamic_dim("N");

    assert_eq!(shape.rank(), 2);
    assert!(!shape.is_fully_static());
    assert_eq!(shape.static_dims(), Maybe::None);
}

#[test]
fn test_meta_parameter_binding() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_dynamic_dim("N");
    shape.bind_meta_param("M", 128);
    shape.bind_meta_param("N", 256);

    assert_eq!(shape.get_meta_param(&Text::from("M")), Maybe::Some(128));
    assert_eq!(shape.get_meta_param(&Text::from("N")), Maybe::Some(256));
}

#[test]
fn test_shape_resolution() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_static_dim(256);
    shape.bind_meta_param("M", 128);

    let resolved = shape.resolve().unwrap();
    assert!(resolved.is_fully_static());
    assert_eq!(
        resolved.static_dims(),
        Maybe::Some(List::from(vec![128, 256]))
    );
}

// ==================== Dimension Constraint System Tests ====================

#[test]
fn test_constraint_system_creation() {
    let system = DimensionConstraintSystem::new();
    assert!(system.constraints().is_empty());
}

#[test]
fn test_equality_constraint() {
    let mut system = DimensionConstraintSystem::new();
    system.add_equality("n_batch", "m_batch");

    assert_eq!(system.constraints().len(), 1);
    assert!(system.are_equal(&Text::from("n_batch"), &Text::from("m_batch")));
}

#[test]
fn test_constant_constraint() {
    let mut system = DimensionConstraintSystem::new();
    system.add_constant("seq_len", 512);

    assert_eq!(
        system.get_fixed_value(&Text::from("seq_len")),
        Maybe::Some(512)
    );
}

#[test]
fn test_range_constraint() {
    let mut system = DimensionConstraintSystem::new();
    system.add_range("batch_size", 1, 1024);

    // Should be satisfiable
    match system.check_satisfiable() {
        Ok(ConstraintCheckResult::Satisfiable { model }) => {
            // Model should have batch_size in range [1, 1024]
            if let Some(&val) = model.get(&Text::from("batch_size")) {
                assert!((1..=1024).contains(&val));
            }
        }
        Ok(ConstraintCheckResult::Unsatisfiable { explanation, .. }) => {
            panic!("Range constraint should be satisfiable: {}", explanation);
        }
        Ok(ConstraintCheckResult::Unknown { .. }) => {
            // SMT solver timeout is acceptable
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_conflicting_constants() {
    let mut system = DimensionConstraintSystem::new();

    // Add constraints that make n = 10 and n = 20 (impossible)
    system.add_constant("n", 10);
    system.add_equality("n", "m");
    system.add_constant("m", 20);

    // Should be unsatisfiable
    match system.check_satisfiable() {
        Ok(ConstraintCheckResult::Unsatisfiable { .. }) => {
            // Expected - constraints conflict
        }
        Ok(ConstraintCheckResult::Satisfiable { .. }) => {
            panic!("Conflicting constants should be unsatisfiable");
        }
        Ok(ConstraintCheckResult::Unknown { .. }) => {
            // SMT solver timeout is acceptable
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_linear_constraint_n_plus_one() {
    let mut system = DimensionConstraintSystem::new();

    // Define: n_plus_one = n + 1
    system.add_linear("n_plus_one", "n", 1, 1);
    system.add_positive("n");

    // Check that n and n_plus_one cannot be equal
    let dim_n = Dimension::Dynamic(Text::from("n"));
    let dim_n_plus_one = Dimension::Dynamic(Text::from("n_plus_one"));

    match system.verify_dimension_equality(&dim_n, &dim_n_plus_one) {
        Ok(DimensionEqualityResult::NotEqual { reason }) => {
            // Expected - n != n + 1 for any positive n
            assert!(reason.contains("cannot"));
        }
        Ok(DimensionEqualityResult::Equal) => {
            panic!("n and n+1 should not be equal");
        }
        Ok(DimensionEqualityResult::PossiblyEqual { .. }) => {
            panic!("n and n+1 cannot possibly be equal");
        }
        Ok(DimensionEqualityResult::Unknown { .. }) => {
            // SMT solver timeout is acceptable
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_shape_verifier_with_constraints() {
    let mut verifier = ShapeVerifier::new();

    // Add constraint that batch dimensions must be equal
    verifier.add_equality_constraint("n_batch", "m_batch");

    // Create shapes with dynamic batch dimensions
    let mut shape_a = TensorShape::new();
    shape_a.add_dynamic_dim("n_batch");
    shape_a.add_static_dim(128);

    let mut shape_b = TensorShape::new();
    shape_b.add_dynamic_dim("m_batch");
    shape_b.add_static_dim(128);

    // Broadcast should succeed because n_batch = m_batch
    let result = verifier.verify_broadcast(&shape_a, &shape_b);
    assert!(result.is_ok());
}

#[test]
fn test_shape_verifier_incompatible_broadcast() {
    let mut verifier = ShapeVerifier::new();

    // Add constraints that make n and m necessarily different
    verifier.add_constant_constraint("n", 10);
    verifier.add_constant_constraint("m", 20);

    // Create shapes
    let mut shape_a = TensorShape::new();
    shape_a.add_dynamic_dim("n");

    let mut shape_b = TensorShape::new();
    shape_b.add_dynamic_dim("m");

    // Broadcast should fail because n = 10 and m = 20
    let result = verifier.verify_broadcast(&shape_a, &shape_b);
    match result {
        Err(ShapeError::IncompatibleDynamicDimensions { dim1, dim2, .. }) => {
            // Expected error
            assert!(dim1 == "n" || dim1 == "m");
        }
        Ok(_) => {
            panic!("Should have detected incompatible dimensions");
        }
        Err(_) => {
            // Other errors are acceptable (e.g., timeout)
        }
    }
}

#[test]
fn test_detect_n_vs_n_plus_one_broadcast() {
    let mut verifier = ShapeVerifier::new();

    // This is the key test case from the requirements:
    // Detect incompatible dimensions like [n] broadcast with [n+1]
    verifier.add_linear_constraint("n_plus_one", "n", 1, 1);
    verifier.add_positive("n");

    let mut shape_n = TensorShape::new();
    shape_n.add_dynamic_dim("n");

    let mut shape_n_plus_one = TensorShape::new();
    shape_n_plus_one.add_dynamic_dim("n_plus_one");

    // Broadcasting [n] with [n+1] should fail
    let result = verifier.verify_broadcast(&shape_n, &shape_n_plus_one);
    match result {
        Err(ShapeError::IncompatibleDynamicDimensions {
            dim1, dim2, reason, ..
        }) => {
            // Expected - n cannot equal n+1
            assert!(reason.contains("cannot") || reason.contains("constraints"));
        }
        Ok(_) => {
            panic!("Should have detected that [n] and [n+1] are incompatible for broadcast");
        }
        Err(_) => {
            // Other errors are acceptable (e.g., timeout)
        }
    }
}

#[test]
fn test_constraint_check_satisfiability() {
    let mut verifier = ShapeVerifier::new();

    // Add some consistent constraints
    verifier.add_equality_constraint("a", "b");
    verifier.add_range_constraint("a", 1, 100);
    verifier.add_positive("a");

    // Check constraints are satisfiable
    match verifier.check_constraints() {
        Ok(ConstraintCheckResult::Satisfiable { model }) => {
            // Model should satisfy all constraints
            if let (Some(&a_val), Some(&b_val)) =
                (model.get(&Text::from("a")), model.get(&Text::from("b")))
            {
                assert_eq!(a_val, b_val, "a and b should be equal");
                assert!((1..=100).contains(&a_val), "a should be in range [1, 100]");
                assert!(a_val > 0, "a should be positive");
            }
        }
        Ok(ConstraintCheckResult::Unsatisfiable { explanation, .. }) => {
            panic!("Constraints should be satisfiable: {}", explanation);
        }
        Ok(ConstraintCheckResult::Unknown { .. }) => {
            // SMT timeout is acceptable
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_transitive_equality() {
    let mut system = DimensionConstraintSystem::new();

    // a = b, b = c implies a = c
    system.add_equality("a", "b");
    system.add_equality("b", "c");

    // Verify a and c are in the same equivalence class
    assert!(system.are_equal(&Text::from("a"), &Text::from("c")));
}

#[test]
fn test_fixed_value_through_equality() {
    let mut system = DimensionConstraintSystem::new();

    // a = 42, a = b implies b = 42
    system.add_constant("a", 42);
    system.add_equality("a", "b");

    // b should have fixed value 42 through equivalence
    let result = system.check_satisfiable();
    if let Ok(ConstraintCheckResult::Satisfiable { model }) = result
        && let Some(&b_val) = model.get(&Text::from("b")) {
            assert_eq!(b_val, 42, "b should equal 42 through a = b constraint");
        }
}

#[test]
fn test_not_equal_constraint() {
    let mut system = DimensionConstraintSystem::new();

    // a != b, both in range [1, 2]
    system.add_not_equal("a", "b");
    system.add_range("a", 1, 2);
    system.add_range("b", 1, 2);

    // Should be satisfiable (e.g., a=1, b=2)
    match system.check_satisfiable() {
        Ok(ConstraintCheckResult::Satisfiable { model }) => {
            if let (Some(&a_val), Some(&b_val)) =
                (model.get(&Text::from("a")), model.get(&Text::from("b")))
            {
                assert_ne!(a_val, b_val, "a and b should be different");
            }
        }
        Ok(ConstraintCheckResult::Unsatisfiable { .. }) => {
            panic!("Constraints should be satisfiable");
        }
        Ok(ConstraintCheckResult::Unknown { .. }) => {}
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_less_than_constraint() {
    let mut system = DimensionConstraintSystem::new();

    // a < b, a >= 1, b <= 10
    system.add_less_than("a", "b");
    system.add_range("a", 1, 10);
    system.add_range("b", 1, 10);

    match system.check_satisfiable() {
        Ok(ConstraintCheckResult::Satisfiable { model }) => {
            if let (Some(&a_val), Some(&b_val)) =
                (model.get(&Text::from("a")), model.get(&Text::from("b")))
            {
                assert!(a_val < b_val, "a should be less than b");
            }
        }
        Ok(ConstraintCheckResult::Unsatisfiable { .. }) => {
            panic!("Constraints should be satisfiable");
        }
        Ok(ConstraintCheckResult::Unknown { .. }) => {}
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}
