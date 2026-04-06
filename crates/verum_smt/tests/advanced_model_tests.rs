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
// Tests for advanced_model module
// Migrated from src/advanced_model.rs per CLAUDE.md standards

use verum_smt::advanced_model::*;
use verum_common::{List, Text};

use z3::ast::Int;
use z3::{Config, Solver};

#[test]
fn test_extract_constant() {
    z3::with_z3_config(&Config::new(), || {
        let solver = Solver::new();
        let x = Int::new_const("x");

        solver.assert(x._eq(Int::from_i64(42)));

        if let z3::SatResult::Sat = solver.check() {
            let model = solver.get_model().unwrap();
            let extractor = AdvancedModelExtractor::new(model);

            let x_val = extractor.extract_constant_model("x");
            assert!(x_val.is_some());
        }
    });
}

#[test]
fn test_extract_all_constants() {
    z3::with_z3_config(&Config::new(), || {
        let solver = Solver::new();
        let x = Int::new_const("x");
        let y = Int::new_const("y");

        solver.assert(x._eq(Int::from_i64(10)));
        solver.assert(y._eq(Int::from_i64(20)));

        if let z3::SatResult::Sat = solver.check() {
            let model = solver.get_model().unwrap();
            let mut extractor = AdvancedModelExtractor::new(model);
            extractor.extract_complete_model();

            assert_eq!(extractor.get_constants().len(), 2);
            assert!(extractor.get_constant("x").is_some());
            assert!(extractor.get_constant("y").is_some());
        }
    });
}

#[test]
fn test_function_model_display() {
    let mut func_model = CompleteFunctionModel::new("test_func".into(), 2);
    func_model.add_entry(
        List::from_iter(vec![Text::from("1"), Text::from("2")]),
        Text::from("10"),
    );
    func_model.add_entry(
        List::from_iter(vec![Text::from("3"), Text::from("4")]),
        Text::from("20"),
    );
    func_model.set_default(Text::from("0"));

    let display = format!("{}", func_model);
    assert!(display.contains("test_func"));
    assert!(display.contains("1"));
    assert!(display.contains("10"));
    assert!(display.contains("else -> 0"));
}

#[test]
fn test_quick_extract_constants() {
    z3::with_z3_config(&Config::new(), || {
        let solver = Solver::new();
        let a = Int::new_const("a");
        let b = Int::new_const("b");

        solver.assert(a._eq(Int::from_i64(100)));
        solver.assert(b._eq(Int::from_i64(200)));

        if let z3::SatResult::Sat = solver.check() {
            let model = solver.get_model().unwrap();
            let constants = quick_extract_constants(&model);

            assert_eq!(constants.len(), 2);
            assert!(constants.contains_key(&"a".into()));
            assert!(constants.contains_key(&"b".into()));
        }
    });
}

#[test]
fn test_model_summary() {
    z3::with_z3_config(&Config::new(), || {
        let solver = Solver::new();
        let x = Int::new_const("x");
        let y = Int::new_const("y");

        solver.assert(x._eq(Int::from_i64(1)));
        solver.assert(y._eq(Int::from_i64(2)));

        if let z3::SatResult::Sat = solver.check() {
            let model = solver.get_model().unwrap();
            let mut extractor = AdvancedModelExtractor::new(model);
            extractor.extract_complete_model();

            let summary = extractor.summary();
            assert_eq!(summary.num_constants, 2);
            assert!(summary.constant_names.contains(&"x".into()));
            assert!(summary.constant_names.contains(&"y".into()));
        }
    });
}
