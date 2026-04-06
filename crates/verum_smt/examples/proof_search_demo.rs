//! Proof Search and Automation Demo
//!
//! This example demonstrates the proof search engine and hints database
//! for automated theorem proving in Verum.
//!
//! Run with: cargo run --example proof_search_demo

use std::time::Duration;
use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
};
use verum_smt::{
    Context,
    proof_search::{
        DecisionProcedure, HintsDatabase, LemmaHint, ProofDomain, ProofSearchEngine, ProofTactic,
        TacticHint,
    },
};
use verum_common::{Heap, Maybe, Text};

fn main() {
    println!("=== Verum Proof Search and Automation Demo ===\n");

    let ctx = Context::new();

    // ==================== Demo 1: Hints Database ====================
    println!("--- Demo 1: Hints Database ---");
    println!("Creating a hints database with standard library hints\n");

    let mut hints_db = HintsDatabase::with_core();
    println!("✓ Hints database initialized with stdlib hints");

    // Add custom hint
    let custom_lemma = LemmaHint {
        name: "custom_monotonicity".into(),
        priority: 150,
        lemma: Heap::new(create_placeholder_lemma()),
    };
    hints_db.register_lemma("_ < _".into(), custom_lemma);
    println!("✓ Registered custom lemma: custom_monotonicity");
    println!();

    // Query hints for a goal
    let goal_expr = create_binary_expr(BinOp::Lt, create_var("x"), create_var("y"));

    println!("Finding hints for goal: x < y");
    let hints = hints_db.find_hints(&goal_expr);
    println!("Found {} applicable hints:", hints.len());

    for (i, hint) in hints.iter().enumerate() {
        println!(
            "  {}. {} (priority: {})",
            i + 1,
            hint.name(),
            hint.priority()
        );
    }

    let stats = hints_db.stats();
    println!("\nHints Database Statistics:");
    println!("  Total queries: {}", stats.total_queries);
    println!("  Hit rate: {:.1}%", stats.hit_rate() * 100.0);
    println!("  Avg lookup time: {:.2}μs", stats.avg_time_us());
    println!();

    // ==================== Demo 2: Decision Procedures ====================
    println!("--- Demo 2: Decision Procedures ---");
    println!("Testing decision procedures for different proof domains\n");

    let decision_procs = vec![
        DecisionProcedure {
            name: "linear_arithmetic".into(),
            applicable_to: ProofDomain::LinearArithmetic,
            timeout: Duration::from_millis(100),
        },
        DecisionProcedure {
            name: "propositional".into(),
            applicable_to: ProofDomain::Propositional,
            timeout: Duration::from_millis(50),
        },
    ];

    // Test with linear arithmetic goal
    let lia_goal = create_binary_expr(BinOp::Add, create_var("x"), create_var("y"));

    println!("Goal: x + y (Linear Arithmetic)");
    for proc in &decision_procs {
        let applicable = proc.is_applicable(&lia_goal);
        println!(
            "  {}: {}",
            proc.name,
            if applicable {
                "✓ applicable"
            } else {
                "✗ not applicable"
            }
        );
    }
    println!();

    // Test with boolean goal
    let bool_goal = create_binary_expr(BinOp::And, create_var("p"), create_var("q"));

    println!("Goal: p && q (Propositional)");
    for proc in &decision_procs {
        let applicable = proc.is_applicable(&bool_goal);
        println!(
            "  {}: {}",
            proc.name,
            if applicable {
                "✓ applicable"
            } else {
                "✗ not applicable"
            }
        );
    }
    println!();

    // ==================== Demo 3: Proof Tactics ====================
    println!("--- Demo 3: Proof Tactics ---");
    println!("Demonstrating tactic composition\n");

    // Create tactics
    let simplify = ProofTactic::Simplify;
    let intro = ProofTactic::Intro;
    let split = ProofTactic::Split;

    // Compose tactics
    let auto_tactic = simplify.then(intro).or_else(split).repeat();

    println!("Auto tactic created: simplify; intro | split; repeat");
    println!("✓ Tactics can be composed sequentially and with alternatives");
    println!();

    // ==================== Demo 4: Proof Search Engine ====================
    println!("--- Demo 4: Proof Search Engine ---");
    println!("Attempting automated proof search\n");

    let mut search_engine = ProofSearchEngine::new();
    search_engine.set_max_depth(10);
    search_engine.set_timeout(Duration::from_secs(5));

    println!("Proof Search Engine Configuration:");
    println!("  Max depth: {}", 10);
    println!("  Timeout: {:?}", Duration::from_secs(5));
    println!();

    // Try to prove a simple goal
    let simple_goal = create_binary_expr(BinOp::Gt, create_var("x"), create_int_literal(0));

    println!("Attempting to prove: x > 0");
    match search_engine.auto_prove(&ctx, &simple_goal) {
        Ok(proof) => {
            println!("✓ Proof found!");
            println!("  Duration: {:?}", proof.cost.duration);
            println!("  Cached: {}", proof.cached);
        }
        Err(e) => {
            println!("✗ Proof search failed: {}", e);
        }
    }
    println!();

    // ==================== Demo 5: Proof Search Statistics ====================
    println!("--- Demo 5: Proof Search Statistics ---");

    let search_stats = search_engine.stats();
    println!("Proof Search Statistics:");
    println!("  Total attempts: {}", search_stats.total_attempts);
    println!("  Successes: {}", search_stats.successes);
    println!("  Failures: {}", search_stats.failures);
    println!("  No hints: {}", search_stats.no_hints);
    println!(
        "  Success rate: {:.1}%",
        search_stats.success_rate() * 100.0
    );
    println!();

    // ==================== Demo 6: Hints Management ====================
    println!("--- Demo 6: Hints Management ---");
    println!("Managing the hints database\n");

    let hints_ref = search_engine.hints();
    let hints_stats = hints_ref.stats();

    println!("Current Hints Database:");
    println!("  Total queries: {}", hints_stats.total_queries);
    println!("  Hits: {}", hints_stats.hits);
    println!("  Hit rate: {:.1}%", hints_stats.hit_rate() * 100.0);
    println!();

    // ==================== Demo 7: Custom Hints ====================
    println!("--- Demo 7: Adding Custom Hints ---");

    let hints_mut = search_engine.hints_mut();

    // Add a custom tactic hint
    let custom_tactic = TacticHint {
        name: "custom_induction".into(),
        priority: 200,
        tactic: ProofTactic::Induction { var: "n".into() },
    };

    hints_mut.register_tactic("induction(_)".into(), custom_tactic);
    println!("✓ Added custom tactic hint: custom_induction");

    // Add another decision procedure
    let custom_proc = DecisionProcedure {
        name: "equality_logic".into(),
        applicable_to: ProofDomain::Equality,
        timeout: Duration::from_millis(75),
    };

    hints_mut.register_decision_procedure(custom_proc);
    println!("✓ Added custom decision procedure: equality_logic");
    println!();

    // ==================== Summary ====================
    println!("=== Summary ===");
    println!("This demo showed:");
    println!("  1. Hints Database - storing and querying proof hints");
    println!("  2. Decision Procedures - automatic proof for decidable fragments");
    println!("  3. Proof Tactics - composable proof strategies");
    println!("  4. Proof Search Engine - automated theorem proving");
    println!("  5. Statistics - monitoring proof search performance");
    println!("  6. Hints Management - accessing and managing hints");
    println!("  7. Custom Hints - extending the proof automation");
    println!();
    println!("These features provide foundation for automated theorem proving");
    println!("and proof search in Verum's formal verification system.");
}

// ==================== Helper Functions ====================

fn create_int_literal(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

fn create_var(name: &str) -> Expr {
    use verum_ast::{Ident, Path, PathSegment};

    let ident = Ident::new(name, Span::dummy());
    let path = Path::from_ident(ident);
    Expr::new(ExprKind::Path(path), Span::dummy())
}

fn create_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

fn create_placeholder_lemma() -> Expr {
    // Create a trivial lemma (in real usage, would be loaded from theory files)
    use verum_ast::literal::{Literal, LiteralKind};

    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
        Span::dummy(),
    )
}
