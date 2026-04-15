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
//! CVC5 Backend Integration Tests
//!
//! Tests the CVC5 backend implementation with stub FFI layer.

#![cfg(feature = "cvc5")]

use verum_smt::cvc5_backend::{
    Cvc5Backend, Cvc5Config, SatResult, SmtLogic as Cvc5SmtLogic, create_cvc5_backend,
    create_cvc5_backend_for_logic,
};

#[test]
fn test_cvc5_backend_creation() {
    let result = Cvc5Backend::new(Cvc5Config::default());
    // Stub implementation returns null pointers, causing initialization failure
    assert!(
        result.is_err(),
        "Stub implementation should fail to initialize"
    );
}

#[test]
fn test_cvc5_config_default() {
    let config = Cvc5Config::default();
    assert_eq!(config.logic, Cvc5SmtLogic::ALL);
    assert!(config.incremental);
    assert!(config.produce_models);
    assert!(config.produce_proofs);
    assert!(config.produce_unsat_cores);
}

#[test]
fn test_cvc5_config_custom() {
    let config = Cvc5Config {
        logic: Cvc5SmtLogic::QF_LIA,
        timeout_ms: verum_common::Maybe::Some(5000),
        incremental: false,
        produce_models: false,
        produce_proofs: false,
        produce_unsat_cores: false,
        preprocessing: false,
        quantifier_mode: verum_smt::cvc5_backend::QuantifierMode::None,
        random_seed: verum_common::Maybe::Some(42),
        verbosity: 2,
    };

    assert_eq!(config.logic, Cvc5SmtLogic::QF_LIA);
    assert_eq!(config.timeout_ms, verum_common::Maybe::Some(5000));
    assert!(!config.incremental);
}

#[test]
fn test_smtlogic_conversion() {
    assert_eq!(Cvc5SmtLogic::QF_LIA.as_str(), "QF_LIA");
    assert_eq!(Cvc5SmtLogic::QF_LRA.as_str(), "QF_LRA");
    assert_eq!(Cvc5SmtLogic::QF_BV.as_str(), "QF_BV");
    assert_eq!(Cvc5SmtLogic::QF_NRA.as_str(), "QF_NRA");
    assert_eq!(Cvc5SmtLogic::ALL.as_str(), "ALL");
}

#[test]
fn test_create_cvc5_backend_helpers() {
    // These should fail with stub implementation
    assert!(create_cvc5_backend().is_err());
    assert!(create_cvc5_backend_for_logic(Cvc5SmtLogic::QF_LIA).is_err());
}

#[test]
fn test_sat_result_enum() {
    // Test that SatResult enum works correctly
    let sat = SatResult::Sat;
    let unsat = SatResult::Unsat;
    let unknown = SatResult::Unknown;

    assert!(matches!(sat, SatResult::Sat));
    assert!(matches!(unsat, SatResult::Unsat));
    assert!(matches!(unknown, SatResult::Unknown));
}

#[test]
fn test_quantifier_mode_enum() {
    use verum_smt::cvc5_backend::QuantifierMode;

    let modes = vec![
        QuantifierMode::Auto,
        QuantifierMode::None,
        QuantifierMode::EMatching,
        QuantifierMode::CEGQI,
        QuantifierMode::MBQI,
    ];

    for mode in modes {
        // Just verify the enum values exist
        let _ = format!("{:?}", mode);
    }
}

// NOTE: Full integration tests with actual CVC5 solving would go here
// when the cvc5-sys feature is enabled and libcvc5 is available.
//
// Example test structure:
//
// #[test]
// #[cfg(feature = "cvc5-sys")]
// fn test_cvc5_basic_solving() {
//     let mut solver = Cvc5Backend::new(Cvc5Config::default()).unwrap();
//     let int_sort = solver.int_sort();
//     let x = solver.mk_const(&"x".to_string(), int_sort.clone()).unwrap();
//     let zero = solver.mk_int_val(0).unwrap();
//     let gt = solver.mk_gt(&x, &zero).unwrap();
//     solver.assert(&gt).unwrap();
//
//     match solver.check_sat().unwrap() {
//         SatResult::Sat => {
//             let model = solver.get_model().unwrap();
//             let x_val = solver.eval(&x).unwrap();
//             assert!(matches!(x_val, Cvc5Value::Int(v) if v > 0));
//         }
//         _ => panic!("Expected SAT result"),
//     }
// }
