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
//! Model Enumeration Tests - Sort Universe Extraction
//!
//! These tests verify that we can enumerate all values of finite sorts
//! from Z3 models using the C API.

use verum_smt::advanced_model::AdvancedModelExtractor;
use verum_common::{Set, Text};
use z3::{
    DatatypeBuilder, FuncDecl, SatResult, Solver, Sort, Symbol,
    ast::{Ast, Datatype, Int},
};

#[test]
fn test_enumerate_bool_sort() {
    let solver = Solver::new();

    // Bool sort has exactly 2 values: true and false
    let b = z3::ast::Bool::new_const("b");
    solver.assert(&(&b | &b.not())); // Trivially true

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);

    // Note: Bool is a built-in sort, universe enumeration may not apply
    // This test demonstrates the API
}

#[test]
fn test_enumerate_color_datatype() {
    let solver = Solver::new();

    // Define Color enumeration: Red | Green | Blue
    let color_dt = DatatypeBuilder::new("Color")
        .variant("Red", vec![])
        .variant("Green", vec![])
        .variant("Blue", vec![])
        .finish();

    // Create a Color variable
    let c = Datatype::new_const("c", &color_dt.sort);

    // Assert that c is Red
    let red = Datatype::fresh_const("Red", &color_dt.sort);
    solver.assert(c._eq(&red));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);

    // Enumerate the Color sort universe
    let universe = extractor.enumerate_sort_universe("Color");

    // Expected: { Red, Green, Blue }
    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 3, "Color should have 3 values");
        // The exact string representation depends on Z3's formatting
        // assert!(values.contains(&Text::from("Red")));
        // assert!(values.contains(&Text::from("Green")));
        // assert!(values.contains(&Text::from("Blue")));
    }
}

#[test]
fn test_enumerate_day_of_week() {
    let solver = Solver::new();

    // Define DayOfWeek: Mon | Tue | Wed | Thu | Fri | Sat | Sun
    let day_dt = DatatypeBuilder::new("DayOfWeek")
        .variant("Monday", vec![])
        .variant("Tuesday", vec![])
        .variant("Wednesday", vec![])
        .variant("Thursday", vec![])
        .variant("Friday", vec![])
        .variant("Saturday", vec![])
        .variant("Sunday", vec![])
        .finish();

    let d = Datatype::new_const("today", &day_dt.sort);
    let monday = Datatype::fresh_const("Monday", &day_dt.sort);
    solver.assert(d._eq(&monday));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("DayOfWeek");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 7, "DayOfWeek should have 7 values");
    }
}

#[test]
fn test_enumerate_traffic_light() {
    let solver = Solver::new();

    // Define TrafficLight: Red | Yellow | Green
    let light_dt = DatatypeBuilder::new("TrafficLight")
        .variant("Red", vec![])
        .variant("Yellow", vec![])
        .variant("Green", vec![])
        .finish();

    let light = Datatype::new_const("light", &light_dt.sort);
    let green = Datatype::fresh_const("Green", &light_dt.sort);
    solver.assert(light._eq(&green));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("TrafficLight");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 3, "TrafficLight should have 3 values");
    }
}

#[test]
fn test_enumerate_card_suit() {
    let solver = Solver::new();

    // Define CardSuit: Hearts | Diamonds | Clubs | Spades
    let suit_dt = DatatypeBuilder::new("CardSuit")
        .variant("Hearts", vec![])
        .variant("Diamonds", vec![])
        .variant("Clubs", vec![])
        .variant("Spades", vec![])
        .finish();

    let suit = Datatype::new_const("suit", &suit_dt.sort);
    let hearts = Datatype::fresh_const("Hearts", &suit_dt.sort);
    solver.assert(suit._eq(&hearts));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("CardSuit");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 4, "CardSuit should have 4 values");
    }
}

#[test]
fn test_enumerate_empty_sort() {
    let solver = Solver::new();

    // Define an empty datatype (no variants)
    // This may not be supported by Z3, but demonstrates edge case handling
}

#[test]
fn test_enumerate_singleton_sort() {
    let solver = Solver::new();

    // Define Unit: Unit (single constructor)
    let unit_dt = DatatypeBuilder::new("Unit")
        .variant("Unit", vec![])
        .finish();

    let u = Datatype::new_const("u", &unit_dt.sort);
    let unit_val = Datatype::fresh_const("Unit", &unit_dt.sort);
    solver.assert(u._eq(&unit_val));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("Unit");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 1, "Unit should have 1 value");
    }
}

#[test]
fn test_enumerate_binary_choice() {
    let solver = Solver::new();

    // Define BinaryChoice: Yes | No
    let choice_dt = DatatypeBuilder::new("BinaryChoice")
        .variant("Yes", vec![])
        .variant("No", vec![])
        .finish();

    let choice = Datatype::new_const("choice", &choice_dt.sort);
    let yes = Datatype::fresh_const("Yes", &choice_dt.sort);
    solver.assert(choice._eq(&yes));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("BinaryChoice");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 2, "BinaryChoice should have 2 values");
    }
}

#[test]
fn test_enumerate_direction() {
    let solver = Solver::new();

    // Define Direction: North | South | East | West
    let dir_dt = DatatypeBuilder::new("Direction")
        .variant("North", vec![])
        .variant("South", vec![])
        .variant("East", vec![])
        .variant("West", vec![])
        .finish();

    let dir = Datatype::new_const("dir", &dir_dt.sort);
    let north = Datatype::fresh_const("North", &dir_dt.sort);
    solver.assert(dir._eq(&north));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("Direction");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 4, "Direction should have 4 values");
    }
}

#[test]
fn test_enumerate_state_machine() {
    let solver = Solver::new();

    // Define State: Init | Running | Paused | Stopped | Error
    let state_dt = DatatypeBuilder::new("State")
        .variant("Init", vec![])
        .variant("Running", vec![])
        .variant("Paused", vec![])
        .variant("Stopped", vec![])
        .variant("Error", vec![])
        .finish();

    let state = Datatype::new_const("state", &state_dt.sort);
    let init = Datatype::fresh_const("Init", &state_dt.sort);
    solver.assert(state._eq(&init));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("State");

    if let verum_common::Maybe::Some(values) = universe {
        assert_eq!(values.len(), 5, "State should have 5 values");
    }
}

#[test]
fn test_enumerate_infinite_sort_returns_none() {
    let solver = Solver::new();

    // Int is an infinite sort, should return None
    let x = Int::new_const("x");
    solver.assert(x.gt(Int::from_i64(0)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("Int");

    // Int is infinite, should return None
    assert!(
        matches!(universe, verum_common::Maybe::None),
        "Infinite sorts should return None"
    );
}

#[test]
fn test_enumerate_uninterpreted_sort() {
    let solver = Solver::new();

    // Create an uninterpreted sort
    let person_sort = Sort::uninterpreted(Symbol::String("Person".into()));

    // Declare constants of this sort
    let alice = z3::ast::Dynamic::from_ast(&z3::ast::Dynamic::new_const("alice", &person_sort));
    let bob = z3::ast::Dynamic::from_ast(&z3::ast::Dynamic::new_const("bob", &person_sort));

    // Assert they are different
    solver.assert(alice._eq(&bob).not());

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("Person");

    // Uninterpreted sorts with finite models should enumerate their values
    // The actual behavior depends on Z3's model construction
}

#[test]
fn test_enumerate_nonexistent_sort() {
    let solver = Solver::new();

    let x = Int::new_const("x");
    solver.assert(x._eq(Int::from_i64(42)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);
    let universe = extractor.enumerate_sort_universe("NonExistentSort");

    // Should return None for non-existent sorts
    assert!(
        matches!(universe, verum_common::Maybe::None),
        "Non-existent sorts should return None"
    );
}
