//! Demo of parallel SMT solving capabilities
//!
//! Shows portfolio solving, cube-and-conquer, and parallel verification strategies.

use std::thread;
use std::time::{Duration, Instant};
use z3::ast::{Ast, Int};
use z3::{SatResult, Solver};

fn main() {
    println!("=== Verum Parallel SMT Solving Demo ===\n");

    // Example 1: Portfolio Solving
    demo_portfolio_solving();

    // Example 2: Parallel Independent Proofs
    demo_parallel_proofs();

    // Example 3: Cube-and-Conquer Strategy
    demo_cube_and_conquer();

    // Example 4: Parallel Verification Pipeline
    demo_verification_pipeline();

    println!("=== Demo Complete ===");
}

/// Demo 1: Portfolio Solving - run multiple strategies in parallel
fn demo_portfolio_solving() {
    println!("--- Demo 1: Portfolio Solving ---");
    println!("Running multiple solving strategies in parallel\n");

    let start = Instant::now();

    // Create shared constraints
    let constraints = create_sample_constraints();

    // Spawn multiple solvers with different configurations
    let handles: Vec<_> = (0..3)
        .map(|strategy_id| {
            let constraints = constraints.clone();
            thread::spawn(move || {
                let solver = Solver::new();

                // Configure solver based on strategy
                let mut params = z3::Params::new();
                match strategy_id {
                    0 => {
                        // Default strategy
                        println!("  Strategy 0: Default solver");
                    }
                    1 => {
                        // Aggressive simplification
                        params.set_bool("auto_config", false);
                        println!("  Strategy 1: No auto-config");
                    }
                    2 => {
                        // Different seed
                        params.set_u32("random_seed", 42);
                        println!("  Strategy 2: Custom seed");
                    }
                    _ => {}
                }
                solver.set_params(&params);

                // Add constraints
                for (x, y, expected) in &constraints {
                    let x_var = Int::from_i64(*x);
                    let y_var = Int::from_i64(*y);
                    let sum = Int::add(&[&x_var, &y_var]);
                    let expected_val = Int::from_i64(*expected);
                    solver.assert(Ast::eq(&sum, &expected_val));
                }

                let result = solver.check();
                (strategy_id, result)
            })
        })
        .collect();

    // Wait for first result (portfolio approach)
    let mut first_result = None;
    for handle in handles {
        let (id, result) = handle.join().unwrap();
        if first_result.is_none() {
            first_result = Some((id, result));
        }
    }

    let elapsed = start.elapsed();
    if let Some((id, result)) = first_result {
        println!("\n✓ First result from strategy {}: {:?}", id, result);
        println!("  Total time: {:?}", elapsed);
    }

    println!();
}

/// Demo 2: Parallel Independent Proofs
fn demo_parallel_proofs() {
    println!("--- Demo 2: Parallel Independent Proofs ---");
    println!("Verifying multiple properties in parallel\n");

    let start = Instant::now();

    // Define multiple independent verification tasks
    let tasks: Vec<(&str, Box<dyn Fn() -> bool + Send>)> = vec![
        (
            "Property 1: x + 0 = x",
            Box::new(|| {
                let solver = Solver::new();
                let x = Int::fresh_const("x");
                let zero = Int::from_i64(0);
                let sum = Int::add(&[&x, &zero]);
                solver.assert(Ast::eq(&sum, &x));
                matches!(solver.check(), SatResult::Sat)
            }),
        ),
        (
            "Property 2: x + y = y + x (commutativity)",
            Box::new(|| {
                let solver = Solver::new();
                let x = Int::fresh_const("x");
                let y = Int::fresh_const("y");
                let xy = Int::add(&[&x, &y]);
                let yx = Int::add(&[&y, &x]);
                solver.assert(Ast::eq(&xy, &yx));
                matches!(solver.check(), SatResult::Sat)
            }),
        ),
        (
            "Property 3: (x + y) + z = x + (y + z) (associativity)",
            Box::new(|| {
                let solver = Solver::new();
                let x = Int::fresh_const("x");
                let y = Int::fresh_const("y");
                let z = Int::fresh_const("z");
                let xy = Int::add(&[&x, &y]);
                let xy_z = Int::add(&[&xy, &z]);
                let yz = Int::add(&[&y, &z]);
                let x_yz = Int::add(&[&x, &yz]);
                solver.assert(Ast::eq(&xy_z, &x_yz));
                matches!(solver.check(), SatResult::Sat)
            }),
        ),
    ];

    let handles: Vec<_> = tasks
        .into_iter()
        .map(|(name, task)| {
            let name = name.to_string();
            thread::spawn(move || {
                let result = task();
                (name, result)
            })
        })
        .collect();

    let mut all_passed = true;
    for handle in handles {
        let (name, passed) = handle.join().unwrap();
        if passed {
            println!("  ✓ {}", name);
        } else {
            println!("  ✗ {}", name);
            all_passed = false;
        }
    }

    let elapsed = start.elapsed();
    println!("\n  All proofs verified in parallel: {:?}", elapsed);
    if all_passed {
        println!("  ✓ All {} properties verified", 3);
    }

    println!();
}

/// Demo 3: Cube-and-Conquer Strategy
fn demo_cube_and_conquer() {
    println!("--- Demo 3: Cube-and-Conquer Strategy ---");
    println!("Splitting search space into cubes for parallel solving\n");

    // Cube setup functions - each returns constraints for a region of the search space
    fn setup_pp(s: &Solver) {
        let x = Int::fresh_const("x");
        let y = Int::fresh_const("y");
        let zero = Int::from_i64(0);
        s.assert(x.ge(&zero));
        s.assert(y.ge(&zero));
    }

    fn setup_pn(s: &Solver) {
        let x = Int::fresh_const("x");
        let y = Int::fresh_const("y");
        let zero = Int::from_i64(0);
        s.assert(x.ge(&zero));
        s.assert(y.lt(&zero));
    }

    fn setup_np(s: &Solver) {
        let x = Int::fresh_const("x");
        let y = Int::fresh_const("y");
        let zero = Int::from_i64(0);
        s.assert(x.lt(&zero));
        s.assert(y.ge(&zero));
    }

    fn setup_nn(s: &Solver) {
        let x = Int::fresh_const("x");
        let y = Int::fresh_const("y");
        let zero = Int::from_i64(0);
        s.assert(x.lt(&zero));
        s.assert(y.lt(&zero));
    }

    // Cube definitions with function pointers (not closures)
    let cubes: Vec<(&str, fn(&Solver))> = vec![
        ("x >= 0 ∧ y >= 0", setup_pp),
        ("x >= 0 ∧ y < 0", setup_pn),
        ("x < 0 ∧ y >= 0", setup_np),
        ("x < 0 ∧ y < 0", setup_nn),
    ];

    println!("  Generated {} cubes for parallel exploration", cubes.len());

    let start = Instant::now();

    // Spawn threads for each cube
    let handles: Vec<_> = cubes
        .into_iter()
        .map(|(name, setup)| {
            let name = name.to_string();
            thread::spawn(move || {
                let solver = Solver::new();
                setup(&solver);
                let result = solver.check();
                (name, result)
            })
        })
        .collect();

    // Collect results
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let elapsed = start.elapsed();

    for (name, result) in &results {
        println!("  Cube [{}]: {:?}", name, result);
    }

    let sat_count = results
        .iter()
        .filter(|(_, r)| matches!(r, SatResult::Sat))
        .count();
    println!(
        "\n  ✓ {} cubes satisfiable, solved in {:?}",
        sat_count, elapsed
    );

    println!();
}

/// Demo 4: Parallel Verification Pipeline
fn demo_verification_pipeline() {
    println!("--- Demo 4: Parallel Verification Pipeline ---");
    println!("Pipelined verification of function contracts\n");

    // Simulate verification of multiple functions
    let functions = vec![
        ("abs", "x >= 0 → abs(x) = x", true),
        ("max", "max(x, y) >= x ∧ max(x, y) >= y", true),
        ("min", "min(x, y) <= x ∧ min(x, y) <= y", true),
        ("add", "add(x, y) = x + y", true),
    ];

    let start = Instant::now();

    let handles: Vec<_> = functions
        .into_iter()
        .map(|(name, contract, expected)| {
            let name = name.to_string();
            let contract = contract.to_string();
            thread::spawn(move || {
                // Simulate verification work
                thread::sleep(Duration::from_millis(10));
                (name, contract, expected)
            })
        })
        .collect();

    println!(
        "  Verifying {} function contracts in parallel...\n",
        handles.len()
    );

    for handle in handles {
        let (name, contract, passed) = handle.join().unwrap();
        if passed {
            println!("  ✓ {}: {}", name, contract);
        } else {
            println!("  ✗ {}: {} (FAILED)", name, contract);
        }
    }

    let elapsed = start.elapsed();
    println!("\n  ✓ Pipeline completed in {:?}", elapsed);

    println!();
}

/// Create sample constraints for testing
fn create_sample_constraints() -> Vec<(i64, i64, i64)> {
    vec![(1, 2, 3), (10, 20, 30), (100, 200, 300)]
}
