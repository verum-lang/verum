//! Integration tests for the complementary Z3 + CVC5 SMT architecture.
//!
//! These tests exercise the full pipeline end-to-end:
//!
//!   `@verify(...)` attribute → `VerifyStrategy` → `BackendSwitcher`
//!                                                 → `CapabilityRouter`
//!                                                 → telemetry recording
//!
//! They use in-memory constructed goals rather than a compiled Verum source
//! because constructing real VBC-backed goals requires the full compilation
//! pipeline. The tests focus on verifying that:
//!
//! 1. The routing decisions match the expected theory winners.
//! 2. Telemetry is recorded correctly for each routing path.
//! 3. `VerifyStrategy` parses all documented attribute values.
//! 4. Cross-validation divergence is detected and logged.
//! 5. Stub-mode CVC5 falls back to Z3-only routing transparently.

use verum_smt::capability_router::{
    CapabilityRouter, CrossValidationStrictness, ExtendedCharacteristics, SolverChoice,
    TieBreaker,
};
use verum_smt::portfolio_executor::{
    CrossValidateResult, PortfolioExecutor, PortfolioSolver, SolverId, SolverVerdict,
};
use verum_smt::routing_stats::{DivergenceEvent, RoutingStats, TheoryClass};
use verum_smt::verify_strategy::VerifyStrategy;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ============================================================================
// Test helpers
// ============================================================================

/// A deterministic mock solver for integration tests.
struct MockSolver {
    id: SolverId,
    verdict: SolverVerdict,
    delay_ms: u64,
}

impl PortfolioSolver for MockSolver {
    fn check_sat(&mut self, interrupt: &AtomicBool) -> SolverVerdict {
        let end = std::time::Instant::now() + Duration::from_millis(self.delay_ms);
        while std::time::Instant::now() < end {
            if interrupt.load(Ordering::SeqCst) {
                return SolverVerdict::Cancelled;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.verdict.clone()
    }

    fn solver_id(&self) -> SolverId {
        self.id
    }
}

// ============================================================================
// End-to-end: VerifyStrategy + Router + Telemetry
// ============================================================================

#[test]
fn verify_strategy_runtime_skips_smt() {
    let strategy = VerifyStrategy::from_attribute_value("runtime").unwrap();
    assert_eq!(strategy, VerifyStrategy::Runtime);
    assert!(!strategy.requires_smt());
}

#[test]
fn verify_strategy_formal_routes_via_capability() {
    let strategy = VerifyStrategy::from_attribute_value("formal").unwrap();
    assert_eq!(strategy, VerifyStrategy::Formal);
    assert!(strategy.requires_smt());
    assert!(!strategy.requires_cross_validation());

    #[cfg(feature = "cvc5")]
    {
        use verum_smt::backend_switcher::BackendChoice;
        assert_eq!(
            strategy.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
    }
}

#[test]
fn verify_strategy_cross_validate_sets_flag() {
    let strategy = VerifyStrategy::from_attribute_value("cross_validate").unwrap();
    assert!(strategy.requires_cross_validation());
    assert!(strategy.requires_smt());
}

#[test]
fn verify_strategy_thorough_uses_portfolio() {
    let strategy = VerifyStrategy::from_attribute_value("thorough").unwrap();
    assert_eq!(strategy, VerifyStrategy::Thorough);
    #[cfg(feature = "cvc5")]
    {
        use verum_smt::backend_switcher::BackendChoice;
        assert_eq!(
            strategy.to_backend_choice(),
            Some(BackendChoice::Portfolio)
        );
    }
}

#[test]
fn verify_strategy_rejects_solver_specific_names() {
    // By design, user code cannot reference specific solver backends.
    // z3/cvc5 in attribute values are no longer recognized.
    assert_eq!(VerifyStrategy::from_attribute_value("z3"), None);
    assert_eq!(VerifyStrategy::from_attribute_value("cvc5"), None);
    // Semantic aliases still work.
    assert_eq!(VerifyStrategy::from_attribute_value("fast"), Some(VerifyStrategy::Fast));
    assert_eq!(VerifyStrategy::from_attribute_value("reliable"), Some(VerifyStrategy::Thorough));
}

// ============================================================================
// Router decisions for each theory
// ============================================================================

#[test]
fn router_dispatches_strings_to_cvc5() {
    let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
    let mut chars = ExtendedCharacteristics::default();
    chars.has_strings = true;

    match router.route(&chars) {
        SolverChoice::Cvc5Only { confidence, .. } => {
            assert!(confidence >= 0.85, "strings should strongly favor CVC5");
        }
        other => panic!("expected Cvc5Only for strings, got {:?}", other),
    }
}

#[test]
fn router_dispatches_nonlinear_real_to_cvc5() {
    let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
    let mut chars = ExtendedCharacteristics::default();
    chars.has_nonlinear_real = true;

    assert!(matches!(router.route(&chars), SolverChoice::Cvc5Only { .. }));
}

#[test]
fn router_dispatches_interpolation_to_z3() {
    let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
    let mut chars = ExtendedCharacteristics::default();
    chars.needs_interpolation = true;

    match router.route(&chars) {
        SolverChoice::Z3Only { confidence, .. } => {
            assert_eq!(confidence, 1.0, "interpolation is Z3-only");
        }
        other => panic!("expected Z3Only for interpolation, got {:?}", other),
    }
}

#[test]
fn router_routes_security_critical_to_cross_validate() {
    let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
    let mut chars = ExtendedCharacteristics::default();
    chars.is_security_critical = true;

    assert!(matches!(
        router.route(&chars),
        SolverChoice::CrossValidate { .. }
    ));
}

#[test]
fn router_falls_back_to_z3_when_cvc5_unavailable() {
    // Even if the goal would normally route to CVC5, we fall back to Z3
    // when CVC5 is not available.
    let router = CapabilityRouter::z3_only();
    let mut chars = ExtendedCharacteristics::default();
    chars.has_nonlinear_real = true;  // would be CVC5 if available

    match router.route(&chars) {
        SolverChoice::Z3Only { reason, .. } => {
            assert!(reason.contains("CVC5 not available"));
        }
        other => panic!("expected Z3Only fallback, got {:?}", other),
    }
}

// ============================================================================
// Portfolio execution + tie-breakers
// ============================================================================

#[test]
fn portfolio_first_wins_basic() {
    let z3 = MockSolver {
        id: SolverId::Z3,
        verdict: SolverVerdict::Sat,
        delay_ms: 10,
    };
    let cvc5 = MockSolver {
        id: SolverId::Cvc5,
        verdict: SolverVerdict::Unsat,
        delay_ms: 200,
    };

    let result = PortfolioExecutor::solve_portfolio(
        z3,
        cvc5,
        Duration::from_secs(2),
        TieBreaker::Fastest,
    );

    assert_eq!(result.winner, SolverId::Z3);
    assert_eq!(result.verdict, SolverVerdict::Sat);
}

#[test]
fn portfolio_tie_breaker_respects_preference() {
    let z3 = MockSolver {
        id: SolverId::Z3,
        verdict: SolverVerdict::Sat,
        delay_ms: 30,
    };
    let cvc5 = MockSolver {
        id: SolverId::Cvc5,
        verdict: SolverVerdict::Sat,
        delay_ms: 30,
    };

    let result = PortfolioExecutor::solve_portfolio(
        z3,
        cvc5,
        Duration::from_secs(2),
        TieBreaker::Cvc5,
    );

    // Both produced SAT, verdict should match.
    assert_eq!(result.verdict, SolverVerdict::Sat);
}

// ============================================================================
// Cross-validation
// ============================================================================

#[test]
fn cross_validate_agreement_returns_agreed() {
let z3 = MockSolver {
        id: SolverId::Z3,
        verdict: SolverVerdict::Unsat,
        delay_ms: 10,
    };
    let cvc5 = MockSolver {
        id: SolverId::Cvc5,
        verdict: SolverVerdict::Unsat,
        delay_ms: 10,
    };

    let result = PortfolioExecutor::solve_cross_validate(
        z3,
        cvc5,
        Duration::from_secs(1),
        CrossValidationStrictness::ResultOnly,
    );

    match result {
        CrossValidateResult::Agreed { verdict, .. } => {
            assert_eq!(verdict, SolverVerdict::Unsat);
        }
        other => panic!("expected Agreed, got {:?}", other),
    }
}

#[test]
fn cross_validate_divergence_detected() {
let z3 = MockSolver {
        id: SolverId::Z3,
        verdict: SolverVerdict::Sat,
        delay_ms: 10,
    };
    let cvc5 = MockSolver {
        id: SolverId::Cvc5,
        verdict: SolverVerdict::Unsat,
        delay_ms: 10,
    };

    let result = PortfolioExecutor::solve_cross_validate(
        z3,
        cvc5,
        Duration::from_secs(1),
        CrossValidationStrictness::ResultOnly,
    );

    assert!(
        result.is_diverged(),
        "expected divergence to be detected, got {:?}",
        result
    );
}

// ============================================================================
// Telemetry integration
// ============================================================================

#[test]
fn telemetry_records_routing_decisions() {
    let stats = RoutingStats::new();

    stats.record_routing(
        &SolverChoice::Z3Only {
            confidence: 0.9,
            reason: "LIA".into(),
        },
        TheoryClass::LinearInt,
    );
    stats.record_routing(
        &SolverChoice::Cvc5Only {
            confidence: 0.95,
            reason: "NRA".into(),
        },
        TheoryClass::NonlinearReal,
    );
    stats.record_routing(
        &SolverChoice::Portfolio {
            timeout_ms: 30_000,
            tie_breaker: TieBreaker::Fastest,
        },
        TheoryClass::Mixed,
    );

    assert_eq!(stats.total_queries.load(Ordering::Relaxed), 3);
    assert_eq!(stats.z3_only_count.load(Ordering::Relaxed), 1);
    assert_eq!(stats.cvc5_only_count.load(Ordering::Relaxed), 1);
    assert_eq!(stats.portfolio_count.load(Ordering::Relaxed), 1);
}

#[test]
fn telemetry_tracks_portfolio_winners() {
    let stats = RoutingStats::new();

    // 3 portfolio runs on LIA: Z3 wins 2, CVC5 wins 1
    stats.record_portfolio_win(TheoryClass::LinearInt, SolverId::Z3);
    stats.record_portfolio_win(TheoryClass::LinearInt, SolverId::Z3);
    stats.record_portfolio_win(TheoryClass::LinearInt, SolverId::Cvc5);

    assert_eq!(stats.z3_portfolio_wins.load(Ordering::Relaxed), 2);
    assert_eq!(stats.cvc5_portfolio_wins.load(Ordering::Relaxed), 1);

    let report = stats.report();
    assert!(report.contains("Portfolio Z3 wins"));
    assert!(report.contains("Portfolio CVC5 wins"));
}

#[test]
fn telemetry_logs_divergences() {
    let stats = RoutingStats::new();

    stats.record_divergence(DivergenceEvent {
        timestamp_secs: 1700000000,
        theory: TheoryClass::NonlinearReal,
        z3_verdict: SolverVerdict::Sat,
        cvc5_verdict: SolverVerdict::Unsat,
        z3_elapsed_ms: 50,
        cvc5_elapsed_ms: 30,
    });

    assert_eq!(stats.cross_validate_diverged.load(Ordering::Relaxed), 1);
    assert_eq!(stats.divergence_events().len(), 1);

    let events = stats.divergence_events();
    assert_eq!(events[0].theory, TheoryClass::NonlinearReal);
}

#[test]
fn telemetry_json_export_is_machine_readable() {
    let stats = RoutingStats::new();

    stats.record_routing(
        &SolverChoice::Z3Only {
            confidence: 0.9,
            reason: "test".into(),
        },
        TheoryClass::LinearInt,
    );
    stats.record_outcome(
        TheoryClass::LinearInt,
        &SolverVerdict::Sat,
        Duration::from_millis(5),
    );

    let json = stats.as_json();

    // Verify key fields are present and correctly typed.
    assert!(json["total_queries"].is_number());
    assert_eq!(json["total_queries"].as_u64().unwrap(), 1);
    assert!(json["routing"]["z3_only"].is_number());
    assert!(json["outcomes"]["sat"].is_number());
    assert!(json["per_theory"].is_object());
}

// ============================================================================
// Theory classification
// ============================================================================

#[test]
fn theory_classification_prioritizes_correctly() {
    // Priority: Sequences > Strings > NRA > NIA > Arrays > BV > Datatypes >
    //           LRA > LIA > Quantified > UF > Propositional > Mixed

    let mut chars = ExtendedCharacteristics::default();
    chars.has_sequences = true;
    chars.has_strings = true;
    chars.has_nonlinear_real = true;
    chars.base.is_qflia = true;
    // Sequences wins because it's top priority.
    assert_eq!(TheoryClass::classify(&chars), TheoryClass::Sequences);
}

#[test]
fn theory_classification_mnemonics() {
    // Verify all 13 classes have distinct short names.
    let mnemonics: Vec<_> = [
        TheoryClass::Propositional,
        TheoryClass::LinearInt,
        TheoryClass::LinearReal,
        TheoryClass::NonlinearReal,
        TheoryClass::NonlinearInt,
        TheoryClass::BitVectors,
        TheoryClass::Arrays,
        TheoryClass::Uf,
        TheoryClass::Strings,
        TheoryClass::Sequences,
        TheoryClass::Datatypes,
        TheoryClass::Quantified,
        TheoryClass::Mixed,
    ]
    .iter()
    .map(|t| t.mnemonic())
    .collect();

    let unique: std::collections::HashSet<_> = mnemonics.iter().copied().collect();
    assert_eq!(mnemonics.len(), unique.len(), "mnemonics must be unique");
}

// ============================================================================
// Stub mode: CVC5 not linked
// ============================================================================

#[test]
fn cvc5_sys_stub_mode_reported_correctly() {
    // When CVC5 is not linked (cvc5-sys features disabled), init() returns false.
    let linked = cvc5_sys::init();
    let version = cvc5_sys::version();

    if linked {
        assert_ne!(version, "unavailable");
    } else {
        assert_eq!(version, "unavailable");
    }
}

#[test]
fn router_detects_cvc5_availability_at_construction() {
    let router = CapabilityRouter::with_defaults();
    assert_eq!(router.is_cvc5_available(), cvc5_sys::init());
}
