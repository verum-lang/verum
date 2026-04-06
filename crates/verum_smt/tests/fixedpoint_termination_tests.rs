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
//! Termination Verification Tests using Z3 Fixed-Point Engine
//!
//! These tests verify that recursive functions terminate using ranking functions.
//! Tests cover various patterns: simple recursion, mutual recursion, and complex cases.

use verum_smt::fixedpoint::{
    RankingFunction, RecursiveCall, RecursiveFunction, RecursiveProgramVerifier,
};
use verum_common::{List, Text};
use z3::{
    Context, Sort,
    ast::{Bool, Int},
};

#[test]
fn test_termination_factorial() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define factorial function: fact(n) = if n <= 0 then 1 else n * fact(n-1)
    // Ranking function: n
    let n = Int::new_const("n");

    let func = RecursiveFunction {
        name: Text::from("fact"),
        param_sorts: List::from(vec![Sort::int()]),
        precondition: n.ge(Int::from_i64(0)), // n >= 0
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("fact"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    // Ranking function: n (strictly decreases from n to n-1)
    let ranking = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.ge(Int::from_i64(0)), // n >= 0
    };

    let result = verifier.verify_termination(func, ranking);
    assert!(result.is_ok(), "Termination verification should succeed");
    assert!(
        result.unwrap(),
        "Factorial should terminate with ranking function n"
    );
}

#[test]
fn test_termination_countdown() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define countdown function: countdown(n) = if n > 0 then countdown(n-1) else done
    let n = Int::new_const("n");

    let func = RecursiveFunction {
        name: Text::from("countdown"),
        param_sorts: List::from(vec![Sort::int()]),
        precondition: n.ge(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("countdown"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "Countdown should terminate");
}

#[test]
fn test_termination_fibonacci() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define fibonacci: fib(n) = if n <= 1 then n else fib(n-1) + fib(n-2)
    let n = Int::new_const("n");

    let func = RecursiveFunction {
        name: Text::from("fib"),
        param_sorts: List::from(vec![Sort::int()]),
        precondition: n.ge(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![
            RecursiveCall {
                predicate: Text::from("fib"),
                args: List::new(),
            },
            RecursiveCall {
                predicate: Text::from("fib"),
                args: List::new(),
            },
        ]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "Fibonacci should terminate");
}

#[test]
fn test_termination_gcd() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define GCD: gcd(a, b) = if b == 0 then a else gcd(b, a mod b)
    // Ranking function: b (strictly decreases)
    let a = Int::new_const("a");
    let b = Int::new_const("b");

    let func = RecursiveFunction {
        name: Text::from("gcd"),
        param_sorts: List::from(vec![Sort::int(), Sort::int()]),
        precondition: Bool::and(&[&a.ge(Int::from_i64(0)), &b.ge(Int::from_i64(0))]),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("gcd"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: b.clone(),
        well_founded_constraint: b.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "GCD should terminate with ranking function b");
}

#[test]
fn test_termination_list_length() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define length: length(list) = if empty then 0 else 1 + length(tail)
    // Ranking function: size of list (strictly decreases)
    let list_size = Int::new_const("list_size");

    let func = RecursiveFunction {
        name: Text::from("length"),
        param_sorts: List::from(vec![Sort::int()]), // Simplified: size as Int
        precondition: list_size.ge(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("length"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: list_size.clone(),
        well_founded_constraint: list_size.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "List length should terminate");
}

#[test]
fn test_termination_tree_height() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define height: height(tree) = if leaf then 0 else 1 + max(height(left), height(right))
    let tree_size = Int::new_const("tree_size");

    let func = RecursiveFunction {
        name: Text::from("height"),
        param_sorts: List::from(vec![Sort::int()]),
        precondition: tree_size.ge(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![
            RecursiveCall {
                predicate: Text::from("height"),
                args: List::new(),
            },
            RecursiveCall {
                predicate: Text::from("height"),
                args: List::new(),
            },
        ]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: tree_size.clone(),
        well_founded_constraint: tree_size.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "Tree height should terminate");
}

#[test]
fn test_termination_power() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define power: pow(x, n) = if n == 0 then 1 else x * pow(x, n-1)
    let n = Int::new_const("n");

    let func = RecursiveFunction {
        name: Text::from("pow"),
        param_sorts: List::from(vec![Sort::int(), Sort::int()]),
        precondition: n.ge(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("pow"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "Power function should terminate");
}

#[test]
fn test_termination_binary_search() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define binary_search: search(low, high) = ...
    // Ranking function: high - low (strictly decreases)
    let low = Int::new_const("low");
    let high = Int::new_const("high");
    let range = Int::sub(&[&high, &low]);

    let func = RecursiveFunction {
        name: Text::from("binary_search"),
        param_sorts: List::from(vec![Sort::int(), Sort::int()]),
        precondition: low.le(&high),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("binary_search"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: range.clone(),
        well_founded_constraint: range.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "Binary search should terminate");
}

#[test]
fn test_termination_merge_sort_split() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Define merge_sort: sort(list) = if len <= 1 then list else merge(sort(left), sort(right))
    // Ranking function: length of list
    let len = Int::new_const("len");

    let func = RecursiveFunction {
        name: Text::from("merge_sort"),
        param_sorts: List::from(vec![Sort::int()]),
        precondition: len.ge(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![
            RecursiveCall {
                predicate: Text::from("merge_sort"),
                args: List::new(),
            },
            RecursiveCall {
                predicate: Text::from("merge_sort"),
                args: List::new(),
            },
        ]),
        verification_conditions: List::new(),
    };

    let ranking = RankingFunction {
        expression: len.clone(),
        well_founded_constraint: len.ge(Int::from_i64(0)),
    };

    let result = verifier.verify_termination(func, ranking).unwrap();
    assert!(result, "Merge sort should terminate");
}

#[test]
fn test_termination_collatz_conjecture() {
    let ctx = Context::thread_local();
    let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();

    // Collatz: collatz(n) = if n == 1 then 1 else if even(n) then collatz(n/2) else collatz(3n+1)
    // Note: Termination is unproven in general! This test demonstrates the API.
    let n = Int::new_const("n");

    let func = RecursiveFunction {
        name: Text::from("collatz"),
        param_sorts: List::from(vec![Sort::int()]),
        precondition: n.gt(Int::from_i64(0)),
        postcondition: Bool::from_bool(true),
        recursive_calls: List::from(vec![RecursiveCall {
            predicate: Text::from("collatz"),
            args: List::new(),
        }]),
        verification_conditions: List::new(),
    };

    // Try with n as ranking function (won't work for Collatz in general)
    let ranking = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.gt(Int::from_i64(0)),
    };

    // This may or may not verify depending on Z3's analysis
    let result = verifier.verify_termination(func, ranking);
    // We don't assert success because Collatz is unproven
    assert!(
        result.is_ok(),
        "Verification should complete (even if it returns false)"
    );
}
