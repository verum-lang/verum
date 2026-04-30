//! Performance benchmarks for automatic tactic selection
//!
//! These benchmarks verify that automatic tactic selection improves
//! solver performance compared to the default SMT tactic.
//!
//! Performance targets:
//! - Analysis overhead: <100us per formula
//! - Tactic selection speedup: 2-5x for specialized problems

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_smt::tactics::{
    FormulaGoalAnalyzer, TacticCache, TacticExecutor, auto_select_tactic,
    auto_select_tactic_cached,
};
use z3::{
    Goal, Solver,
    ast::{BV, Bool, Int},
};

// ==================== Analysis Overhead Benchmarks ====================

fn bench_analysis_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("analysis_overhead");
    group.sample_size(100);

    let mut analyzer = FormulaGoalAnalyzer::new();

    // Simple propositional formula
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");
    let simple_prop = Bool::and(&[&x, &y]);

    group.bench_function("analyze_simple_prop", |b| {
        b.iter(|| analyzer.analyze(black_box(&simple_prop)))
    });

    // Linear arithmetic formula
    let a = Int::new_const("a");
    let b_var = Int::new_const("b");
    let five = Int::from_i64(5);
    let sum = Int::add(&[&a, &b_var]);
    let lia_formula = sum.gt(&five);

    group.bench_function("analyze_linear_arith", |b| {
        b.iter(|| analyzer.analyze(black_box(&lia_formula)))
    });

    // Bit-vector formula
    let bv_x = BV::new_const("bv_x", 32);
    let bv_y = BV::new_const("bv_y", 32);
    let bv_sum = bv_x.bvadd(&bv_y);
    let bv_mask = BV::from_i64(0xFF, 32);
    let bv_formula = bv_sum.bvand(&bv_mask).eq(&bv_mask);

    group.bench_function("analyze_bitvector", |b| {
        b.iter(|| analyzer.analyze(black_box(&bv_formula)))
    });

    // Complex formula with multiple theories
    let complex_and = Bool::and(&[&simple_prop, &lia_formula]);

    group.bench_function("analyze_mixed_theory", |b| {
        b.iter(|| analyzer.analyze(black_box(&complex_and)))
    });

    group.finish();
}

// ==================== Tactic Selection Benchmarks ====================

fn bench_tactic_selection(c: &mut Criterion) {
    let mut group = c.benchmark_group("tactic_selection");
    group.sample_size(100);

    let mut analyzer = FormulaGoalAnalyzer::new();

    // Test auto_select_tactic function
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");
    let formula = Bool::and(&[&x, &y]);

    group.bench_function("auto_select_prop", |b| {
        b.iter(|| auto_select_tactic(black_box(&mut analyzer), black_box(&formula)))
    });

    // Linear arithmetic
    let a = Int::new_const("a");
    let b_var = Int::new_const("b");
    let five = Int::from_i64(5);
    let sum = Int::add(&[&a, &b_var]);
    let lia_formula = sum.gt(&five);

    group.bench_function("auto_select_lia", |b| {
        b.iter(|| auto_select_tactic(black_box(&mut analyzer), black_box(&lia_formula)))
    });

    // Bit-vector
    let bv_x = BV::new_const("bv_x", 32);
    let bv_y = BV::new_const("bv_y", 32);
    let bv_formula = bv_x.bvult(&bv_y);

    group.bench_function("auto_select_bv", |b| {
        b.iter(|| auto_select_tactic(black_box(&mut analyzer), black_box(&bv_formula)))
    });

    group.finish();
}

// ==================== Speedup Comparison Benchmarks ====================

fn bench_speedup_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("speedup_comparison");
    group.sample_size(50);

    // Test 1: Propositional formulas (should benefit from SAT tactics)
    {
        let mut clauses = Vec::new();
        for i in 0..20 {
            let x = Bool::new_const(format!("x{}", i));
            let y = Bool::new_const(format!("y{}", i));
            clauses.push(Bool::or(&[&x, &y]));
        }
        let clause_refs: Vec<&Bool> = clauses.iter().collect();
        let prop_formula = Bool::and(&clause_refs);

        // Benchmark with default SMT tactic
        group.bench_function("prop_default_smt", |b| {
            b.iter(|| {
                let solver = Solver::new();
                solver.assert(&prop_formula);
                solver.check()
            })
        });

        // Benchmark with auto-selected tactic
        let mut analyzer = FormulaGoalAnalyzer::new();
        let selected_tactic = auto_select_tactic(&mut analyzer, &prop_formula);
        let mut executor = TacticExecutor::new();

        group.bench_function("prop_auto_selected", |b| {
            b.iter(|| {
                let goal = Goal::new(false, false, false);
                goal.assert(&prop_formula);
                executor.execute(&goal, &selected_tactic)
            })
        });
    }

    // Test 2: Linear integer arithmetic (should benefit from QF_LIA tactics)
    {
        let vars: Vec<Int> = (0..10).map(|i| Int::new_const(format!("v{}", i))).collect();

        let mut constraints = Vec::new();
        for i in 0..vars.len() {
            let bound = Int::from_i64((i * 10) as i64);
            constraints.push(vars[i].gt(&bound));
        }

        // Add a sum constraint
        let var_refs: Vec<&Int> = vars.iter().collect();
        let sum = Int::add(&var_refs);
        let hundred = Int::from_i64(100);
        constraints.push(sum.lt(&hundred));

        let constraint_refs: Vec<&Bool> = constraints.iter().collect();
        let lia_formula = Bool::and(&constraint_refs);

        // Benchmark with default SMT tactic
        group.bench_function("lia_default_smt", |b| {
            b.iter(|| {
                let solver = Solver::new();
                solver.assert(&lia_formula);
                solver.check()
            })
        });

        // Benchmark with auto-selected tactic
        let mut analyzer = FormulaGoalAnalyzer::new();
        let selected_tactic = auto_select_tactic(&mut analyzer, &lia_formula);
        let mut executor = TacticExecutor::new();

        group.bench_function("lia_auto_selected", |b| {
            b.iter(|| {
                let goal = Goal::new(false, false, false);
                goal.assert(&lia_formula);
                executor.execute(&goal, &selected_tactic)
            })
        });
    }

    // Test 3: Bit-vector formulas (should benefit from bit-blasting)
    {
        let bv_x = BV::new_const("x", 32);
        let bv_y = BV::new_const("y", 32);
        let bv_z = BV::new_const("z", 32);

        let sum = bv_x.bvadd(&bv_y);
        let product = sum.bvmul(&bv_z);
        let mask = BV::from_i64(0xFFFF, 32);
        let masked = product.bvand(&mask);
        let bv_formula = masked.bvugt(BV::from_i64(0x1000, 32));

        // Benchmark with default SMT tactic
        group.bench_function("bv_default_smt", |b| {
            b.iter(|| {
                let solver = Solver::new();
                solver.assert(&bv_formula);
                solver.check()
            })
        });

        // Benchmark with auto-selected tactic
        let mut analyzer = FormulaGoalAnalyzer::new();
        let selected_tactic = auto_select_tactic(&mut analyzer, &bv_formula);
        let mut executor = TacticExecutor::new();

        group.bench_function("bv_auto_selected", |b| {
            b.iter(|| {
                let goal = Goal::new(false, false, false);
                goal.assert(&bv_formula);
                executor.execute(&goal, &selected_tactic)
            })
        });
    }

    group.finish();
}

// ==================== Formula Size Scaling Benchmarks ====================

fn bench_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");
    group.sample_size(20);

    for num_vars in [5, 10, 20, 50].iter() {
        // Create a formula with increasing number of variables
        let vars: Vec<Int> = (0..*num_vars)
            .map(|i| Int::new_const(format!("x{}", i)))
            .collect();

        let mut constraints = Vec::new();
        for (i, v) in vars.iter().enumerate() {
            let bound = Int::from_i64(i as i64);
            constraints.push(v.gt(&bound));
        }
        let constraint_refs: Vec<&Bool> = constraints.iter().collect();
        let formula = Bool::and(&constraint_refs);

        // Benchmark analysis time scaling
        let mut analyzer = FormulaGoalAnalyzer::new();
        group.bench_with_input(
            BenchmarkId::new("analysis_scaling", num_vars),
            num_vars,
            |b, _| b.iter(|| analyzer.analyze(black_box(&formula))),
        );

        // Benchmark tactic selection scaling
        group.bench_with_input(
            BenchmarkId::new("selection_scaling", num_vars),
            num_vars,
            |b, _| b.iter(|| auto_select_tactic(black_box(&mut analyzer), black_box(&formula))),
        );
    }

    group.finish();
}

// ==================== Characteristic Detection Benchmarks ====================

fn bench_characteristic_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("characteristic_detection");
    group.sample_size(100);

    let mut analyzer = FormulaGoalAnalyzer::new();

    // Test individual characteristic detection methods
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");
    let prop_formula = Bool::and(&[&x, &y]);

    group.bench_function("is_propositional", |b| {
        b.iter(|| analyzer.is_propositional(black_box(&prop_formula)))
    });

    let a = Int::new_const("a");
    let b_var = Int::new_const("b");
    let sum = Int::add(&[&a, &b_var]);
    let five = Int::from_i64(5);
    let lia_formula = sum.gt(&five);

    group.bench_function("is_linear_arithmetic", |b| {
        b.iter(|| analyzer.is_linear_arithmetic(black_box(&lia_formula)))
    });

    let bv_x = BV::new_const("bv_x", 32);
    let bv_y = BV::new_const("bv_y", 32);
    let bv_formula = bv_x.bvult(&bv_y);

    group.bench_function("has_bitvectors", |b| {
        b.iter(|| analyzer.has_bitvectors(black_box(&bv_formula)))
    });

    group.bench_function("num_variables", |b| {
        b.iter(|| analyzer.num_variables(black_box(&lia_formula)))
    });

    group.finish();
}

// ==================== Tactic Cache Speedup Benchmarks (#103) ====================
//
// Compares the cost of repeated tactic selection on the same
// formula with and without [`TacticCache`]. The cache hit path
// computes a blake3 digest over the formula's S-expression and
// rebuilds the combinator tree from cached characteristics —
// skipping the nine Z3 probes that dominate the uncached path.
//
// Expected order-of-magnitude: cached lookup is dominated by
// `Bool::to_string` + blake3, both single-microsecond on the
// formulas used here. Uncached `auto_select_tactic` runs the full
// nine-probe sweep (~50–200us depending on formula size).
fn bench_cache_speedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_speedup");
    group.sample_size(100);

    // ===== Linear-arithmetic formula (representative of refinement
    //       VCs that dominate Verum verification load) =====
    let a = Int::new_const("a");
    let b_var = Int::new_const("b");
    let five = Int::from_i64(5);
    let sum = Int::add(&[&a, &b_var]);
    let lia_formula = sum.gt(&five);

    // Uncached baseline.
    {
        let mut analyzer = FormulaGoalAnalyzer::new();
        group.bench_function("lia_uncached", |bencher| {
            bencher.iter(|| auto_select_tactic(black_box(&mut analyzer), black_box(&lia_formula)))
        });
    }

    // Cached, hot path (cache populated on first iter, every
    // subsequent iter is a cache hit).
    {
        let cache = TacticCache::new();
        let mut analyzer = FormulaGoalAnalyzer::new();
        // Prime the cache so the bench measures hit-path cost.
        let _ = auto_select_tactic_cached(&cache, &mut analyzer, &lia_formula);
        group.bench_function("lia_cached_hot", |bencher| {
            bencher.iter(|| {
                auto_select_tactic_cached(
                    black_box(&cache),
                    black_box(&mut analyzer),
                    black_box(&lia_formula),
                )
            })
        });
    }

    // ===== Bit-vector formula =====
    let bv_x = BV::new_const("bvx", 32);
    let bv_y = BV::new_const("bvy", 32);
    let bv_formula = bv_x.bvult(&bv_y);

    {
        let mut analyzer = FormulaGoalAnalyzer::new();
        group.bench_function("bv_uncached", |bencher| {
            bencher.iter(|| auto_select_tactic(black_box(&mut analyzer), black_box(&bv_formula)))
        });
    }

    {
        let cache = TacticCache::new();
        let mut analyzer = FormulaGoalAnalyzer::new();
        let _ = auto_select_tactic_cached(&cache, &mut analyzer, &bv_formula);
        group.bench_function("bv_cached_hot", |bencher| {
            bencher.iter(|| {
                auto_select_tactic_cached(
                    black_box(&cache),
                    black_box(&mut analyzer),
                    black_box(&bv_formula),
                )
            })
        });
    }

    // ===== Realistic mixed workload: 16 distinct formula shapes
    //       cycled in round-robin. Approximates the steady-state
    //       behaviour of a verification pass that revisits a small
    //       set of formula templates many times. =====
    let workload: Vec<Bool> = (0..16)
        .map(|i| {
            let v = Int::new_const(format!("v{}", i));
            v.gt(Int::from_i64((i * 7) as i64))
        })
        .collect();

    {
        let mut analyzer = FormulaGoalAnalyzer::new();
        group.bench_function("mixed_uncached", |bencher| {
            bencher.iter(|| {
                for f in &workload {
                    auto_select_tactic(black_box(&mut analyzer), black_box(f));
                }
            })
        });
    }

    {
        let cache = TacticCache::new();
        let mut analyzer = FormulaGoalAnalyzer::new();
        // Prime each shape so the bench measures pure hit-path
        // cost (n cache hits per iter).
        for f in &workload {
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, f);
        }
        group.bench_function("mixed_cached_hot", |bencher| {
            bencher.iter(|| {
                for f in &workload {
                    auto_select_tactic_cached(
                        black_box(&cache),
                        black_box(&mut analyzer),
                        black_box(f),
                    );
                }
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_analysis_overhead,
    bench_tactic_selection,
    bench_speedup_comparison,
    bench_scaling,
    bench_characteristic_detection,
    bench_cache_speedup,
);

criterion_main!(benches);
