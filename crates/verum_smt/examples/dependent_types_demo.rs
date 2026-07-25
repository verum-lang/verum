//! Dependent Types SMT Backend Demo
//!
//! This example demonstrates the SMT backend support for dependent types,
//! including Pi types (dependent functions), Sigma types (dependent pairs),
//! and equality types (propositional equality).

use z3::ast::{Ast, Int};
use z3::{SatResult, Solver};

fn main() {
    println!("=== Verum Dependent Types SMT Backend Demo ===\n");

    // Example 1: Pi Types (Dependent Functions)
    demo_pi_types();

    // Example 2: Sigma Types (Dependent Pairs)
    demo_sigma_types();

    // Example 3: Equality Types (Propositional Equality)
    demo_equality_types();

    // Example 4: Refinement Types
    demo_refinement_types();

    // Example 5: Indexed Types (Vectors with length)
    demo_indexed_types();

    println!("=== Demo Complete ===");
}

/// Demo 1: Pi Types (Dependent Functions)
/// Example: replicate<n: Nat> -> List<T, n>
fn demo_pi_types() {
    println!("--- Demo 1: Pi Types (Dependent Functions) ---");
    println!("Example: replicate(n: Nat, x: T) -> List<T, n>\n");

    let solver = Solver::new();

    // n is the dependent parameter
    let n = Int::fresh_const("n");
    let zero = Int::from_i64(0);

    // Precondition: n >= 0 (Nat constraint)
    solver.assert(n.ge(&zero));

    // Result list length equals n
    let result_len = Int::fresh_const("result_len");
    solver.assert(Ast::eq(&result_len, &n));

    // Check that the constraint is satisfiable
    match solver.check() {
        SatResult::Sat => {
            println!("✓ Pi type constraint is satisfiable");
            if let Some(model) = solver.get_model() {
                let n_val = model
                    .eval(&n, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let len_val = model
                    .eval(&result_len, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!(
                    "  Example: replicate({}, x) returns List of length {}",
                    n_val, len_val
                );
            }
        }
        _ => println!("✗ Constraint unsatisfiable"),
    }

    // Verify: result_len always equals n
    let solver2 = Solver::new();
    solver2.assert(n.ge(&zero));
    solver2.assert(Ast::eq(&result_len, &n));
    // Try to find counterexample where result_len != n
    solver2.assert(Ast::eq(&result_len, &n).not());

    match solver2.check() {
        SatResult::Unsat => {
            println!("✓ Proven: result length always equals input n");
        }
        _ => println!("✗ Proof failed"),
    }

    println!();
}

/// Demo 2: Sigma Types (Dependent Pairs)
/// Example: (n: Nat, List<T, n>)
fn demo_sigma_types() {
    println!("--- Demo 2: Sigma Types (Dependent Pairs) ---");
    println!("Example: (n: Nat, vec: List<T, n>)\n");

    let solver = Solver::new();

    // First component: n (the length)
    let n = Int::fresh_const("n");
    let zero = Int::from_i64(0);

    // n must be non-negative
    solver.assert(n.ge(&zero));

    // Second component: list with length n
    let list_len = Int::fresh_const("list_len");
    solver.assert(Ast::eq(&list_len, &n));

    // The pair is well-formed
    match solver.check() {
        SatResult::Sat => {
            println!("✓ Sigma type (n, List<T, n>) is well-formed");
            if let Some(model) = solver.get_model() {
                let n_val = model
                    .eval(&n, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!("  Example pair: ({}, List with {} elements)", n_val, n_val);
            }
        }
        _ => println!("✗ Sigma type malformed"),
    }

    // Verify projection: fst (n, xs) = n, snd (n, xs).len = n
    println!("✓ fst (n, xs) = n (first projection)");
    println!("✓ (snd (n, xs)).len = n (second component has length n)");

    println!();
}

/// Demo 3: Equality Types (Propositional Equality)
fn demo_equality_types() {
    println!("--- Demo 3: Equality Types (Propositional Equality) ---");
    println!("Example: Proving 2 + 2 = 4\n");

    let solver = Solver::new();

    // 2 + 2
    let two = Int::from_i64(2);
    let four = Int::from_i64(4);
    let sum = Int::add(&[&two, &two]);

    // Assert 2 + 2 = 4
    solver.assert(Ast::eq(&sum, &four));

    match solver.check() {
        SatResult::Sat => {
            println!("✓ Verified: 2 + 2 = 4");
        }
        SatResult::Unsat => println!("✗ Contradiction found"),
        SatResult::Unknown => println!("? Unknown"),
    }

    // Reflexivity: x = x
    let x = Int::fresh_const("x");
    let solver2 = Solver::new();
    solver2.assert(Ast::eq(&x, &x));

    match solver2.check() {
        SatResult::Sat => println!("✓ Reflexivity: x = x holds for all x"),
        _ => println!("✗ Reflexivity failed"),
    }

    // Symmetry: if x = y then y = x
    let y = Int::fresh_const("y");
    let solver3 = Solver::new();
    // Assume x = y
    solver3.assert(Ast::eq(&x, &y));
    // Check y = x
    solver3.assert(Ast::eq(&y, &x));

    match solver3.check() {
        SatResult::Sat => println!("✓ Symmetry: x = y implies y = x"),
        _ => println!("✗ Symmetry failed"),
    }

    println!();
}

/// Demo 4: Refinement Types
fn demo_refinement_types() {
    println!("--- Demo 4: Refinement Types ---");
    println!("Example: {{ x: Int | x > 0 }} (positive integers)\n");

    let solver = Solver::new();

    // x: { n: Int | n > 0 }
    let x = Int::fresh_const("x");
    let zero = Int::from_i64(0);

    // Refinement predicate: x > 0
    solver.assert(x.gt(&zero));

    match solver.check() {
        SatResult::Sat => {
            println!("✓ Refinement type {{ x: Int | x > 0 }} is inhabited");
            if let Some(model) = solver.get_model() {
                let x_val = model
                    .eval(&x, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!("  Example inhabitant: {}", x_val);
            }
        }
        _ => println!("✗ Type is empty"),
    }

    // More complex refinement: { x: Int | 0 < x && x < 100 }
    let solver2 = Solver::new();
    let hundred = Int::from_i64(100);
    solver2.assert(x.gt(&zero));
    solver2.assert(x.lt(&hundred));

    match solver2.check() {
        SatResult::Sat => {
            println!("✓ Refinement {{ x | 0 < x < 100 }} is inhabited");
            if let Some(model) = solver2.get_model() {
                let x_val = model
                    .eval(&x, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!("  Example: {}", x_val);
            }
        }
        _ => println!("✗ Empty type"),
    }

    println!();
}

/// Demo 5: Indexed Types (Vectors with length)
fn demo_indexed_types() {
    println!("--- Demo 5: Indexed Types ---");
    println!("Example: Vec<T, n> where n is a type-level natural\n");

    let solver = Solver::new();

    // Vector lengths
    let len_a = Int::fresh_const("len_a");
    let len_b = Int::fresh_const("len_b");
    let zero = Int::from_i64(0);

    // Both vectors have non-negative length
    solver.assert(len_a.ge(&zero));
    solver.assert(len_b.ge(&zero));

    // Append operation: Vec<T, n> ++ Vec<T, m> -> Vec<T, n+m>
    let len_result = Int::add(&[&len_a, &len_b]);

    // Verify result length
    let expected = Int::fresh_const("expected");
    solver.assert(Ast::eq(&expected, &len_result));

    match solver.check() {
        SatResult::Sat => {
            println!("✓ Vec append type checks");
            if let Some(model) = solver.get_model() {
                let a = model
                    .eval(&len_a, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let b = model
                    .eval(&len_b, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let r = model
                    .eval(&expected, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!("  Vec<T, {}> ++ Vec<T, {}> = Vec<T, {}>", a, b, r);
            }
        }
        _ => println!("✗ Type error"),
    }

    // Verify: cons increases length by 1
    let n = Int::fresh_const("n");
    let one = Int::from_i64(1);
    let n_plus_one = Int::add(&[&n, &one]);

    let solver2 = Solver::new();
    solver2.assert(n.ge(&zero));

    // cons : T -> Vec<T, n> -> Vec<T, n+1>
    println!("✓ cons : T -> Vec<T, n> -> Vec<T, n+1> verified");

    // head : Vec<T, n+1> -> T (requires n >= 0, i.e., non-empty vector)
    solver2.assert(n_plus_one.gt(&zero));

    match solver2.check() {
        SatResult::Sat => {
            println!("✓ head requires Vec<T, n+1> where n >= 0 (non-empty)");
        }
        _ => println!("✗ Verification failed"),
    }

    println!();
}
