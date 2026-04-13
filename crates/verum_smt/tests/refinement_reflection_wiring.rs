//! Integration test: `RefinementReflectionRegistry` round-trips
//! through `ProofSearchEngine` setter/getter so downstream proof
//! search invocations have access to user-function axioms.

use verum_common::{List, Text};

use verum_smt::proof_search::ProofSearchEngine;
use verum_smt::refinement_reflection::{
    ReflectedFunction, RefinementReflectionRegistry,
};

fn double_def() -> ReflectedFunction {
    ReflectedFunction {
        name: Text::from("double"),
        parameters: List::from_iter([Text::from("n")]),
        body_smtlib: Text::from("(* 2 n)"),
        return_sort: Text::from("Int"),
        parameter_sorts: List::from_iter([Text::from("Int")]),
    }
}

fn add_def() -> ReflectedFunction {
    ReflectedFunction {
        name: Text::from("add"),
        parameters: List::from_iter([Text::from("a"), Text::from("b")]),
        body_smtlib: Text::from("(+ a b)"),
        return_sort: Text::from("Int"),
        parameter_sorts: List::from_iter([Text::from("Int"), Text::from("Int")]),
    }
}

#[test]
fn engine_starts_with_empty_registry() {
    let engine = ProofSearchEngine::new();
    assert!(engine.reflection_registry().is_empty());
    assert_eq!(engine.reflection_registry().len(), 0);
}

#[test]
fn set_registry_makes_axioms_available() {
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(double_def()).unwrap();
    reg.register(add_def()).unwrap();

    let mut engine = ProofSearchEngine::new();
    engine.set_reflection_registry(reg);

    assert_eq!(engine.reflection_registry().len(), 2);
    assert!(engine.reflection_registry().lookup(&Text::from("double")).is_some());
    assert!(engine.reflection_registry().lookup(&Text::from("add")).is_some());
}

#[test]
fn replacing_registry_supersedes_previous() {
    let mut reg1 = RefinementReflectionRegistry::new();
    reg1.register(double_def()).unwrap();

    let mut engine = ProofSearchEngine::new();
    engine.set_reflection_registry(reg1);
    assert_eq!(engine.reflection_registry().len(), 1);

    let mut reg2 = RefinementReflectionRegistry::new();
    reg2.register(add_def()).unwrap();
    engine.set_reflection_registry(reg2);

    // Old `double` should be gone; only `add` remains.
    assert_eq!(engine.reflection_registry().len(), 1);
    assert!(engine.reflection_registry().lookup(&Text::from("double")).is_none());
    assert!(engine.reflection_registry().lookup(&Text::from("add")).is_some());
}

#[test]
fn registry_block_renders_through_engine() {
    let mut reg = RefinementReflectionRegistry::new();
    reg.register(double_def()).unwrap();

    let mut engine = ProofSearchEngine::new();
    engine.set_reflection_registry(reg);

    let block = engine.reflection_registry().to_smtlib_block();
    let s = block.as_str();

    assert!(s.contains("(declare-fun double (Int) Int)"));
    assert!(s.contains("(assert (forall ((n Int)) (= (double n) (* 2 n))))"));
}
