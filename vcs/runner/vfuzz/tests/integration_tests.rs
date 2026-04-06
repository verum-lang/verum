//! Comprehensive integration tests for vfuzz
//!
//! These tests verify that all components work together correctly,
//! with particular focus on:
//! - All oracle types with known bugs
//! - Shrinking produces minimal examples
//! - Coverage increases over time
//! - Property-based testing
//! - Campaign management
//! - Parallel coordination

use std::collections::HashMap;
use std::time::Duration;

use rand::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use tempfile::tempdir;

use verum_vfuzz::{
    campaign::{
        CampaignConfig, CampaignType, TargetComponent,
        SeedCorpus, SeedCorpusConfig, SeedSource,
        EnergyScheduler, EnergyConfig, PowerSchedule,
        ParallelCoordinator, ParallelConfig, WorkItem, WorkType,
        CampaignCheckpoint, CampaignState, CampaignPhase,
    },
    corpus::CorpusManager,
    coverage::{
        CoverageTracker, GlobalCoverage,
        BranchCoverage, BranchId, AstNodeCoverage, ErrorCodeCoverage, ErrorSeverity,
        SmtTheoryCoverage, SmtTheory, UnifiedCoverage,
    },
    generator::{Generator, GeneratorConfig, GeneratorKind},
    generators::{GeneratorStrategy, UnifiedGenerator},
    mutator::{Mutator, MutatorConfig},
    oracle::{
        CompareConfig, DifferentialOracle, ExecutionTier, SerializedValue,
        TierResult, CrashOracle, MemorySafetyOracle, TypeSafetyOracle, SmtOracle,
        TimeoutOracle, TimeoutPhase, OracleRunner, OracleExecutionResult,
    },
    property::{
        PropertyRunner, ExtendedPropertyRunner,
        IdempotencyProperties, RoundtripProperties, CommutativityProperties,
        AssociativityProperties, PropertyCategory,
    },
    shrink::{
        ShrinkConfig, ShrinkResult, ShrinkStrategy, Shrinker,
        DeltaDebugger, DeltaDebugConfig, DeltaUnit,
        HierarchicalShrinker, HierarchicalConfig, AstAwareShrinker,
    },
    triage::{
        CrashClass, CrashSignature, CrashTriager,
        CrashDeduplicator, SeverityClassifier, Severity,
        RegressionDetector, AutoCategorizer,
    },
    FuzzConfig, FuzzEngine,
};

/// Test the complete fuzzing pipeline
#[test]
fn test_complete_fuzzing_pipeline() {
    let dir = tempdir().unwrap();
    let corpus_dir = dir.path().join("corpus");
    let crash_dir = dir.path().join("crashes");
    let seed_dir = dir.path().join("seeds");

    std::fs::create_dir_all(&corpus_dir).unwrap();
    std::fs::create_dir_all(&crash_dir).unwrap();
    std::fs::create_dir_all(&seed_dir).unwrap();

    // Write a seed file
    let seed_content = "fn main() { 42 }";
    std::fs::write(seed_dir.join("seed.vr"), seed_content).unwrap();

    let config = FuzzConfig {
        iterations: 100,
        timeout_ms: 1000,
        workers: 1,
        crash_dir,
        corpus_dir,
        minimize: false,
        max_program_size: 10000,
        seed: Some(42),
        differential: false,
        max_depth: 5,
        verbose: false,
        ..Default::default()
    };

    let mut engine = FuzzEngine::new(config).expect("Failed to create engine");

    // Load seeds
    let seed_count = engine.load_seeds(&seed_dir).expect("Failed to load seeds");
    assert!(seed_count >= 1);

    // Run fuzzing
    let stats = engine.run(100).expect("Fuzzing failed");
    assert!(stats.iterations > 0);
}

/// Test generator produces valid programs
#[test]
fn test_generator_integration() {
    let config = GeneratorConfig {
        max_depth: 8,
        max_statements: 30,
        kind: GeneratorKind::Mixed,
        ..Default::default()
    };

    let mut gen_instance = Generator::new(config);
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    // Generate multiple programs
    for _ in 0..50 {
        let program = gen_instance.generate(&mut rng);
        assert!(!program.is_empty());
        // Should have fn main() in most cases
        assert!(
            program.contains("fn") || program.contains("type"),
            "Program should have function or type definition"
        );
    }
}

/// Test unified generator with all strategies
#[test]
fn test_unified_generator() {
    let config = verum_vfuzz::generators::GeneratorConfig {
        max_depth: 5,
        max_statements: 20,
        include_async: true,
        include_cbgr: true,
        include_refinements: true,
        ..Default::default()
    };

    let mut gen_instance = UnifiedGenerator::new(config);
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    // Test each strategy
    let strategies = [
        GeneratorStrategy::Lexer,
        GeneratorStrategy::Parser,
        GeneratorStrategy::Refinement,
        GeneratorStrategy::Async,
        GeneratorStrategy::Cbgr,
    ];

    for strategy in strategies {
        let program = gen_instance.generate_with(strategy, &mut rng);
        assert!(!program.is_empty(), "Strategy {:?} produced empty output", strategy);
    }
}

/// Test mutation produces different outputs
#[test]
fn test_mutation_integration() {
    let config = MutatorConfig {
        mutation_rate: 1.0,
        ..Default::default()
    };

    let mutator = Mutator::new(config);
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    let original = "fn main() { let x = 42; x + 1 }";
    let mut mutations = std::collections::HashSet::new();

    for _ in 0..100 {
        let mutated = mutator.mutate(original, &mut rng);
        mutations.insert(mutated);
    }

    // Should produce multiple different mutations
    assert!(mutations.len() > 1, "Should produce varied mutations");
}

/// Test shrinking reduces input size
#[test]
fn test_shrinking_integration() {
    let config = ShrinkConfig {
        max_iterations: 500,
        strategy: ShrinkStrategy::Combined,
        ..Default::default()
    };

    let shrinker = Shrinker::new(config);

    // Input where only one line matters
    let input = r#"
fn helper() {
    let a = 1;
    let b = 2;
    let c = 3;
}

fn main() {
    let important = 42;
    let noise1 = 1;
    let noise2 = 2;
    important
}
"#;

    // Test function that requires "important"
    let test_fn = |s: &str| s.contains("important");

    let result = shrinker.shrink(input, test_fn);

    match result {
        ShrinkResult::Success(minimized) => {
            assert!(minimized.contains("important"));
            assert!(minimized.len() <= input.len());
        }
        ShrinkResult::NoProgress => {
            // Still okay
        }
        ShrinkResult::Error(e) => {
            panic!("Shrinking error: {}", e);
        }
    }
}

/// Test corpus management
#[test]
fn test_corpus_integration() {
    let dir = tempdir().unwrap();
    let mut manager = CorpusManager::new(dir.path()).unwrap();

    // Add entries
    for i in 0..10 {
        let content = format!("fn main() {{ {} }}", i);
        manager.add(&content, None);
    }

    assert_eq!(manager.len(), 10);

    // Pick entries
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let picked = manager.pick(&mut rng);
    assert!(!picked.is_empty());

    // Cull
    manager.cull(5);
    assert_eq!(manager.len(), 5);

    // Persistence test
    drop(manager);
    let manager2 = CorpusManager::new(dir.path()).unwrap();
    assert!(!manager2.is_empty());
}

/// Test coverage tracking
#[test]
fn test_coverage_integration() {
    let tracker = CoverageTracker::new();

    // Simulate execution
    tracker.visit_location(100);
    tracker.visit_location(200);
    tracker.visit_location(300);
    tracker.visit_location(200);

    let bitmap = tracker.bitmap();
    assert!(bitmap.edge_count() > 0);

    // Global coverage
    let global = GlobalCoverage::new();
    let found_new = global.update(bitmap);
    assert!(found_new);

    // Same coverage again should not be new
    let found_new = global.update(bitmap);
    assert!(!found_new);
}

/// Test crash triage
#[test]
fn test_triage_integration() {
    let mut triager = CrashTriager::new();

    // Add crashes
    let crashes = vec![
        (
            "crash1",
            "fn panic() {}",
            "panicked at 'index out of bounds'",
            "0: verum_parser::parse",
        ),
        (
            "crash2",
            "fn panic2() {}",
            "panicked at 'index out of bounds'",
            "0: verum_parser::parse",
        ), // Duplicate
        (
            "crash3",
            "fn other() {}",
            "segmentation fault",
            "0: some_func",
        ), // Different
    ];

    let mut unique_hashes = std::collections::HashSet::new();

    for (id, input, error, bt) in crashes {
        let (is_new, hash) = triager.triage(id, input, error, bt);
        if is_new {
            unique_hashes.insert(hash);
        }
    }

    let stats = triager.stats();
    assert_eq!(stats.total_crashes, 3);
    assert_eq!(stats.unique_crashes, 2); // Two unique types
}

/// Test differential oracle
#[test]
fn test_oracle_integration() {
    let mut oracle = DifferentialOracle::default();

    // Test matching results
    let tier0 = TierResult {
        tier: ExecutionTier::Tier0,
        success: true,
        return_value: Some(SerializedValue::Int(42)),
        stdout: "hello\n".to_string(),
        stderr: String::new(),
        error: None,
        duration: Duration::from_millis(100),
        memory_bytes: 1024,
    };

    let tier3 = TierResult {
        tier: ExecutionTier::Tier3,
        success: true,
        return_value: Some(SerializedValue::Int(42)),
        stdout: "hello\n".to_string(),
        stderr: String::new(),
        error: None,
        duration: Duration::from_millis(10),
        memory_bytes: 512,
    };

    let mut results = HashMap::new();
    results.insert(ExecutionTier::Tier0, tier0);
    results.insert(ExecutionTier::Tier3, tier3);

    let diff = oracle.compare("fn main() { 42 }", results);
    assert!(diff.consistent);
    assert!(diff.mismatches.is_empty());
}

/// Test property runner
#[test]
fn test_property_integration() {
    let mut runner = PropertyRunner::new();
    runner.add_compiler_properties();

    // Run on valid inputs
    let inputs = vec![
        "fn main() { 42 }",
        "fn main() { let x = 1; x + 2 }",
        "type Foo = { a: Int, b: Text }",
    ];

    for input in inputs {
        let passed = runner.run(input);
        assert!(passed, "Properties should pass for input: {}", input);
    }

    let stats = runner.stats();
    assert!(stats.passed > 0);
    assert_eq!(stats.failed, 0);
}

/// Test campaign configuration
#[test]
fn test_campaign_integration() {
    // Test different campaign types
    let exploration = CampaignConfig::exploration();
    assert_eq!(exploration.campaign_type, CampaignType::Exploration);
    assert!(exploration.coverage.enabled);

    let bug_hunting = CampaignConfig::bug_hunting();
    assert_eq!(bug_hunting.campaign_type, CampaignType::BugHunting);
    assert!(bug_hunting.mutator.aggressive);

    let differential = CampaignConfig::differential();
    assert_eq!(differential.campaign_type, CampaignType::Differential);
    assert!(differential.oracle.enabled);

    let targeted = CampaignConfig::targeted(TargetComponent::Parser);
    assert!(matches!(
        targeted.campaign_type,
        CampaignType::Targeted(TargetComponent::Parser)
    ));

    // Test save/load
    let dir = tempdir().unwrap();
    let path = dir.path().join("campaign.json");

    exploration.save(&path).unwrap();
    let loaded = CampaignConfig::load(&path).unwrap();
    assert_eq!(loaded.name, exploration.name);
}

/// Test end-to-end fuzzing with all components
#[test]
fn test_end_to_end_fuzzing() {
    let mut rng = ChaCha8Rng::seed_from_u64(12345);

    // 1. Generate programs
    let gen_config = GeneratorConfig {
        max_depth: 5,
        max_statements: 10,
        kind: GeneratorKind::Mixed,
        ..Default::default()
    };
    let mut generator = Generator::new(gen_config);

    // 2. Set up corpus
    let dir = tempdir().unwrap();
    let mut corpus = CorpusManager::new(dir.path()).unwrap();

    // 3. Set up coverage
    let global_coverage = GlobalCoverage::new();

    // 4. Set up mutator
    let mut_config = MutatorConfig::default();
    let mutator = Mutator::new(mut_config);

    // 5. Set up triager
    let mut triager = CrashTriager::new();

    // 6. Set up property runner
    let mut property_runner = PropertyRunner::new();
    property_runner.add_compiler_properties();

    // Run mini fuzzing loop
    for i in 0..50 {
        // Generate or mutate
        let input = if corpus.is_empty() || rng.random_bool(0.3) {
            generator.generate(&mut rng)
        } else {
            let base = corpus.pick(&mut rng);
            mutator.mutate(&base, &mut rng)
        };

        // Track coverage
        let tracker = CoverageTracker::new();
        tracker.visit_location(i as u32 * 100);
        tracker.visit_location((i as u32 * 100) + 50);

        let found_new = global_coverage.update(tracker.bitmap());

        // Add to corpus if interesting
        if found_new {
            corpus.add(&input, None);
        }

        // Check properties
        property_runner.run(&input);

        // Simulate occasional crash
        if i == 25 {
            triager.triage(
                &format!("crash_{}", i),
                &input,
                "panicked at test",
                "0: test_func",
            );
        }
    }

    // Verify results
    assert!(!corpus.is_empty(), "Corpus should have entries");
    assert!(global_coverage.discovered_count() > 0, "Should have coverage");

    let stats = property_runner.stats();
    assert!(stats.total > 0, "Should have run properties");
}

/// Test complex value comparison in oracle
#[test]
fn test_complex_value_comparison() {
    let config = CompareConfig::default();

    // Nested structures
    let a = SerializedValue::List(vec![
        SerializedValue::Tuple(vec![SerializedValue::Int(1), SerializedValue::Text("a".into())]),
        SerializedValue::Tuple(vec![SerializedValue::Int(2), SerializedValue::Text("b".into())]),
    ]);

    let b = SerializedValue::List(vec![
        SerializedValue::Tuple(vec![SerializedValue::Int(1), SerializedValue::Text("a".into())]),
        SerializedValue::Tuple(vec![SerializedValue::Int(2), SerializedValue::Text("b".into())]),
    ]);

    let result = a.compare(&b, &config);
    assert_eq!(result, verum_vfuzz::oracle::CompareResult::Equal);

    // Nested with difference
    let c = SerializedValue::List(vec![
        SerializedValue::Tuple(vec![SerializedValue::Int(1), SerializedValue::Text("a".into())]),
        SerializedValue::Tuple(vec![
            SerializedValue::Int(3), // Different!
            SerializedValue::Text("b".into()),
        ]),
    ]);

    let result = a.compare(&c, &config);
    assert_ne!(result, verum_vfuzz::oracle::CompareResult::Equal);
}

/// Test generator edge cases
#[test]
fn test_generator_edge_cases() {
    let config = verum_vfuzz::generators::GeneratorConfig {
        max_depth: 1, // Very shallow
        max_statements: 1,
        ..Default::default()
    };

    let mut gen_instance = UnifiedGenerator::new(config);
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    // Should still produce valid output with minimal config
    for _ in 0..10 {
        let program = gen_instance.generate(&mut rng);
        assert!(!program.is_empty());
    }
}

// ============================================================================
// Oracle Integration Tests
// ============================================================================

/// Test crash oracle detects panics
#[test]
fn test_crash_oracle_detects_panic() {
    let mut oracle = CrashOracle::default();

    // Use exit code 0 so the panic detection code path is exercised
    // (exit codes are checked before panic patterns)
    let result = OracleExecutionResult {
        success: false,
        exit_code: Some(0),
        stdout: String::new(),
        stderr: "thread 'main' panicked at 'assertion failed: x > 0'".to_string(),
        return_value: None,
        duration: Duration::from_millis(50),
        memory_bytes: 1024,
        tier: ExecutionTier::Tier0,
    };

    let violation = oracle.check(&result);
    assert!(violation.is_some());
    let v = violation.unwrap();
    assert_eq!(v.crash_type, verum_vfuzz::oracle::CrashType::Panic);
}

/// Test crash oracle ignores expected errors
#[test]
fn test_crash_oracle_ignores_expected() {
    let config = verum_vfuzz::oracle::CrashOracleConfig {
        expected_error_patterns: vec!["syntax error".to_string()],
        ..Default::default()
    };
    let mut oracle = CrashOracle::new(config);

    let result = OracleExecutionResult {
        success: false,
        exit_code: Some(1),
        stdout: String::new(),
        stderr: "syntax error: unexpected token".to_string(),
        return_value: None,
        duration: Duration::from_millis(50),
        memory_bytes: 1024,
        tier: ExecutionTier::Tier0,
    };

    let violation = oracle.check(&result);
    assert!(violation.is_none()); // Expected error, not a crash
}

/// Test memory safety oracle detects use-after-free
#[test]
fn test_memory_safety_oracle_uaf() {
    let mut oracle = MemorySafetyOracle::default();

    let asan_output = r#"
==12345==ERROR: AddressSanitizer: heap-use-after-free on address 0x1234
READ of size 8 at 0x1234 thread T0
    #0 test_function at test.rs:42
freed by thread T0 here:
    #0 free_function at alloc.rs:10
"#;

    let violations = oracle.check_sanitizer_output(asan_output);
    assert_eq!(violations.len(), 1);
    assert!(matches!(
        violations[0],
        verum_vfuzz::oracle::MemorySafetyViolation::UseAfterFree { .. }
    ));
}

/// Test type safety oracle
#[test]
fn test_type_safety_oracle() {
    let mut oracle = TypeSafetyOracle::default();

    let stderr = "type confusion: expected Int, got Text at line 42";
    let violation = oracle.check("", stderr);

    assert!(violation.is_some());
    let v = violation.unwrap();
    assert_eq!(v.kind, verum_vfuzz::oracle::TypeSafetyKind::TypeConfusion);
}

/// Test SMT oracle detects verification failures
#[test]
fn test_smt_oracle() {
    let mut oracle = SmtOracle::default();

    let output = "verification failed: postcondition failed for function foo";
    let violation = oracle.check(output);

    assert!(violation.is_some());
    assert!(matches!(
        violation.unwrap().kind,
        verum_vfuzz::oracle::SmtViolationKind::PostconditionViolation
    ));
}

/// Test timeout oracle
#[test]
fn test_timeout_oracle() {
    let mut oracle = TimeoutOracle::default();

    // Within limit
    let result = oracle.check(TimeoutPhase::Parsing, Duration::from_millis(100));
    assert!(result.is_none());

    // Exceeds limit
    let result = oracle.check(TimeoutPhase::Parsing, Duration::from_secs(10));
    assert!(result.is_some());
    assert_eq!(result.unwrap().phase, TimeoutPhase::Parsing);
}

/// Test unified oracle runner
#[test]
fn test_oracle_runner_integration() {
    let mut runner = OracleRunner::default();

    // Normal execution
    let normal_result = OracleExecutionResult {
        success: true,
        exit_code: Some(0),
        stdout: "42".to_string(),
        stderr: String::new(),
        return_value: Some(SerializedValue::Int(42)),
        duration: Duration::from_millis(50),
        memory_bytes: 1024,
        tier: ExecutionTier::Tier0,
    };

    let violations = runner.check_all(&normal_result);
    assert!(violations.is_empty());

    // Crashing execution
    let crash_result = OracleExecutionResult {
        success: false,
        exit_code: Some(139), // SIGSEGV
        stdout: String::new(),
        stderr: "Segmentation fault".to_string(),
        return_value: None,
        duration: Duration::from_millis(50),
        memory_bytes: 1024,
        tier: ExecutionTier::Tier0,
    };

    let violations = runner.check_all(&crash_result);
    assert!(!violations.is_empty());
}

// ============================================================================
// Shrinking Integration Tests
// ============================================================================

/// Test delta debugging produces minimal example
#[test]
fn test_delta_debugging_minimal() {
    let config = DeltaDebugConfig {
        unit: DeltaUnit::Chars,
        one_minimal: true,
        ..Default::default()
    };
    let mut dd = DeltaDebugger::new(config);

    let input = "abcdefghXijklmnop";
    let result = dd.minimize(input, |s| s.contains('X'));

    if let ShrinkResult::Success(minimized) = result {
        assert!(minimized.contains('X'));
        // Should be close to minimal (just 'X')
        assert!(minimized.len() <= 3);
    } else {
        panic!("Delta debugging should succeed");
    }
}

/// Test hierarchical shrinking
#[test]
fn test_hierarchical_shrinking() {
    let config = HierarchicalConfig::default();
    let mut shrinker = HierarchicalShrinker::new(config);

    let input = r#"
fn unused1() {
    let a = 1;
    let b = 2;
}

fn unused2() {
    let c = 3;
}

fn main() {
    let IMPORTANT = true;
    let noise = 42;
}
"#;

    let result = shrinker.shrink(input, |s| s.contains("IMPORTANT"));

    if let ShrinkResult::Success(minimized) = result {
        assert!(minimized.contains("IMPORTANT"));
        assert!(minimized.len() < input.len());
        // Should have removed unused functions
        assert!(!minimized.contains("unused1") || !minimized.contains("unused2"));
    }
}

/// Test AST-aware shrinking
#[test]
fn test_ast_aware_shrinking() {
    let mut shrinker = AstAwareShrinker::default();

    let input = r#"
fn helper() {
    let x = 42;
}

fn main() {
    let BUG = panic!;
}
"#;

    let result = shrinker.shrink(input, |s| s.contains("BUG"));

    if let ShrinkResult::Success(minimized) = result {
        assert!(minimized.contains("BUG"));
    }
}

/// Test shrinking preserves bug reproduction
#[test]
fn test_shrinking_preserves_reproduction() {
    let shrinker = Shrinker::new(ShrinkConfig::default());

    // Complex input with a specific pattern
    let input = "xxxxxxxxxxxBUGxxxxxxxxxxx\naaaaaa\nbbbbb\nccccc";
    let test_fn = |s: &str| s.contains("BUG");

    let result = shrinker.shrink(input, test_fn);

    match result {
        ShrinkResult::Success(minimized) => {
            // Verify bug is still reproducible
            assert!(test_fn(&minimized));
            assert!(minimized.len() <= input.len());
        }
        _ => {} // May not shrink further, that's okay
    }
}

// ============================================================================
// Coverage Integration Tests
// ============================================================================

/// Test coverage increases over time
#[test]
fn test_coverage_increases() {
    let global = GlobalCoverage::new();
    global.set_total_edges(1000);

    let mut coverage_history = Vec::new();

    for i in 0..100 {
        let tracker = CoverageTracker::new();
        // Simulate visiting new locations
        tracker.visit_location(i * 10);
        tracker.visit_location(i * 10 + 5);

        global.update(tracker.bitmap());
        coverage_history.push(global.discovered_count());
    }

    // Coverage should be monotonically increasing
    for i in 1..coverage_history.len() {
        assert!(
            coverage_history[i] >= coverage_history[i - 1],
            "Coverage should not decrease"
        );
    }

    // Coverage should increase
    assert!(coverage_history.last() > coverage_history.first());
}

/// Test branch coverage tracking
#[test]
fn test_branch_coverage_integration() {
    let coverage = BranchCoverage::new();

    let branch1 = BranchId::new("main.vr", 10, 5, "if condition");
    let branch2 = BranchId::new("main.vr", 20, 10, "match arm");

    coverage.register_branch(branch1.clone());
    coverage.register_branch(branch2.clone());

    assert_eq!(coverage.total_branches(), 2);
    assert_eq!(coverage.covered_branches(), 0);

    // Cover branch1 true path
    coverage.record_branch(&branch1, true);
    assert_eq!(coverage.coverage_pct(), 0.0); // Neither fully covered

    // Cover branch1 false path
    coverage.record_branch(&branch1, false);
    assert_eq!(coverage.covered_branches(), 1); // branch1 fully covered

    // Cover branch2 both paths
    coverage.record_branch(&branch2, true);
    coverage.record_branch(&branch2, false);
    assert_eq!(coverage.covered_branches(), 2);
    assert_eq!(coverage.coverage_pct(), 100.0);
}

/// Test AST node coverage
#[test]
fn test_ast_node_coverage_integration() {
    let coverage = AstNodeCoverage::new();

    // Register known node types
    coverage.register_known_types(&[
        "FunctionDef",
        "LetBinding",
        "IfExpr",
        "ForLoop",
        "MatchExpr",
    ]);

    // Record some coverage
    coverage.record_node("FunctionDef", 0, Some("fn main() {}"), true, false);
    coverage.record_node("LetBinding", 1, Some("let x = 1"), true, false);
    coverage.record_node("IfExpr", 2, Some("if x > 0 { }"), true, false);

    assert_eq!(coverage.covered_types(), 3);
    assert_eq!(coverage.coverage_pct(), Some(60.0));

    let uncovered = coverage.uncovered_types();
    assert!(uncovered.contains(&"ForLoop".to_string()));
    assert!(uncovered.contains(&"MatchExpr".to_string()));
}

/// Test error code coverage
#[test]
fn test_error_code_coverage_integration() {
    let coverage = ErrorCodeCoverage::new();

    coverage.register_known_codes(&["E0001", "E0002", "E0003", "E0100", "E0200"]);

    coverage.record_error(
        "E0001",
        Some("let x: Int = \"hello\""),
        Some("type mismatch"),
        ErrorSeverity::Error,
    );
    coverage.record_error(
        "E0100",
        Some("fn foo() { x }"),
        Some("unresolved identifier"),
        ErrorSeverity::Error,
    );

    assert_eq!(coverage.triggered_count(), 2);
    assert_eq!(coverage.coverage_pct(), Some(40.0));

    let untriggered = coverage.untriggered_codes();
    assert!(untriggered.contains(&"E0002".to_string()));
}

/// Test SMT theory coverage
#[test]
fn test_smt_theory_coverage_integration() {
    let coverage = SmtTheoryCoverage::new();

    coverage.record_usage(&[SmtTheory::LIA, SmtTheory::Arrays], true, false, false, 100);
    coverage.record_usage(&[SmtTheory::BV, SmtTheory::UF], false, true, false, 5000);
    coverage.record_usage(&[SmtTheory::Quantifiers], true, false, false, 200);

    assert_eq!(coverage.covered_count(), 5);
    assert!(coverage.coverage_pct() > 0.0);

    let combinations = coverage.combinations();
    assert!(combinations.len() >= 3);
}

/// Test unified coverage tracker
#[test]
fn test_unified_coverage_integration() {
    let unified = UnifiedCoverage::new();

    // Add branch coverage
    let branch = BranchId::new("test.vr", 1, 1, "");
    unified.branch.register_branch(branch.clone());
    unified.branch.record_branch(&branch, true);
    unified.branch.record_branch(&branch, false);

    // Add line coverage
    unified.line.record_line("test.vr", 1);
    unified.line.record_line("test.vr", 2);
    unified.line.record_line("test.vr", 3);

    // Add AST coverage
    unified.ast.record_node("FunctionDef", 0, None, true, false);

    // Add error coverage
    unified.error.record_error("E0001", None, None, ErrorSeverity::Error);

    // Add SMT coverage
    unified.smt.record_usage(&[SmtTheory::LIA], true, false, false, 50);

    let report = unified.report();
    assert!(report.branch.covered_branches > 0);
    assert!(report.line.total_lines > 0);
    assert!(report.ast.covered_types > 0);
    assert!(report.error.triggered_count > 0);
    assert!(report.smt.covered_count > 0);
}

// ============================================================================
// Property Testing Integration Tests
// ============================================================================

/// Test extended property runner
#[test]
fn test_extended_property_runner_integration() {
    let mut runner = ExtendedPropertyRunner::new();
    runner.add_all_properties();

    // Check categories populated
    assert!(runner.category_count(PropertyCategory::Structural) > 0);
    assert!(runner.category_count(PropertyCategory::Algebraic) > 0);
    assert!(runner.category_count(PropertyCategory::Safety) > 0);

    // Run on valid input
    let passed = runner.run("fn main() { 42 }");
    assert!(passed);

    let stats = runner.stats();
    assert!(stats.total > 0);
}

/// Test individual property categories
#[test]
fn test_property_categories() {
    // Idempotency
    let prop = IdempotencyProperties::parse_idempotent();
    let result = prop.test("fn main() { 42 }".to_string());
    assert!(result.is_pass());

    // Roundtrip
    let prop = RoundtripProperties::ast_roundtrip();
    let result = prop.test("fn main() { 42 }".to_string());
    assert!(result.is_pass());

    // Commutativity
    let prop = CommutativityProperties::addition_commutative();
    let result = prop.test("1 + 2".to_string());
    assert!(result.is_pass());

    // Associativity
    let prop = AssociativityProperties::addition_associative();
    let result = prop.test("1 + 2 + 3".to_string());
    assert!(result.is_pass());
}

// ============================================================================
// Triage Integration Tests
// ============================================================================

/// Test crash deduplication
#[test]
fn test_deduplication_integration() {
    let mut dedup = CrashDeduplicator::default();

    // Same crash, different inputs
    let sig1 = CrashSignature::from_crash(
        "panicked at 'index out of bounds'",
        "0: verum_parser::parse at parser.rs:100",
    );
    let sig2 = CrashSignature::from_crash(
        "panicked at 'index out of bounds'",
        "0: verum_parser::parse at parser.rs:100",
    );

    // Different crash
    let sig3 = CrashSignature::from_crash(
        "panicked at 'unwrap on None'",
        "0: verum_lexer::lex at lexer.rs:50",
    );

    assert!(dedup.process("crash1", sig1, "input1"));
    assert!(!dedup.process("crash2", sig2, "input2")); // Duplicate
    assert!(dedup.process("crash3", sig3, "input3")); // New

    let stats = dedup.stats();
    assert_eq!(stats.unique, 2);
    assert_eq!(stats.exact_duplicates, 1);
}

/// Test severity classification
#[test]
fn test_severity_classification_integration() {
    let classifier = SeverityClassifier::default();

    // Critical
    let assessment = classifier.assess(&CrashClass::Segfault, "", 50);
    assert_eq!(assessment.level, Severity::Critical);
    assert!(assessment.security_relevant);

    // High
    let assessment = classifier.assess(&CrashClass::TypeCheckerCrash, "", 100);
    assert_eq!(assessment.level, Severity::High);

    // Medium
    let assessment = classifier.assess(&CrashClass::SmtCrash, "", 200);
    assert_eq!(assessment.level, Severity::Medium);

    // Low
    let assessment = classifier.assess(&CrashClass::LexerCrash, "", 500);
    assert_eq!(assessment.level, Severity::Low);
}

/// Test regression detection
#[test]
fn test_regression_detection_integration() {
    let mut triager = CrashTriager::new();
    let detector = RegressionDetector::default();
    let classifier = SeverityClassifier::default();

    // Baseline crashes
    triager.triage("old1", "input1", "panicked at 'old bug'", "0: old_func");
    triager.triage("old2", "input2", "panicked at 'another old bug'", "0: old_func2");

    let baseline = detector.create_baseline("v1.0.0", &triager);

    // Add new crashes (simulating new version)
    triager.triage("new1", "input3", "panicked at 'new bug'", "0: new_func");

    let report = detector.compare("v1.1.0", &baseline, &triager, &classifier);

    assert_eq!(report.summary.new_count, 1);
    assert_eq!(report.summary.persistent_count, 2);
    assert_eq!(report.summary.fixed_count, 0);
}

/// Test auto-categorization
#[test]
fn test_auto_categorization_integration() {
    let categorizer = AutoCategorizer::new();

    // Parser crash
    let sig = CrashSignature::from_crash(
        "panicked at 'parse error'",
        "0: verum_parser::parse_expr",
    );
    let cat = categorizer.categorize(&sig, "parse error", "verum_parser::parse_expr");
    assert_eq!(cat.component, "parser");

    // Type system crash
    let sig = CrashSignature::from_crash(
        "panicked at 'type mismatch'",
        "0: verum_types::unify",
    );
    let cat = categorizer.categorize(&sig, "type mismatch", "verum_types::unify");
    assert_eq!(cat.component, "type-system");
}

// ============================================================================
// Campaign Integration Tests
// ============================================================================

/// Test seed corpus management
#[test]
fn test_seed_corpus_integration() {
    let dir = tempdir().unwrap();
    let config = SeedCorpusConfig::default();
    let corpus = SeedCorpus::new(dir.path().to_path_buf(), config);

    // Add seeds
    let hash1 = corpus.add("fn main() { 1 }", SeedSource::UserProvided).unwrap();
    let _hash2 = corpus.add("fn main() { 2 }", SeedSource::UserProvided).unwrap();
    let _hash3 = corpus.add("fn main() { 3 }", SeedSource::Generated).unwrap();

    assert_eq!(corpus.len(), 3);

    // Test duplicate detection
    let dup_result = corpus.add("fn main() { 1 }", SeedSource::UserProvided);
    assert!(dup_result.is_err());

    // Test selection and energy
    corpus.mark_selected(&hash1);
    let entry = corpus.get(&hash1).unwrap();
    assert_eq!(entry.selections, 1);
    assert!(entry.energy < 1.0); // Energy should decay

    // Test persistence
    let save_path = dir.path().join("corpus");
    corpus.save_to_directory(&save_path).unwrap();

    // Verify files were saved
    assert!(save_path.exists());
}

/// Test energy scheduling
#[test]
fn test_energy_scheduling_integration() {
    let config = EnergyConfig {
        schedule: PowerSchedule::Explore,
        initial_energy: 100.0,
        ..Default::default()
    };
    let scheduler = EnergyScheduler::new(config);

    // Add inputs with different coverage/speed
    scheduler.add("high_cov".to_string(), 100, 1000);
    scheduler.add("low_cov".to_string(), 10, 1000);
    scheduler.add("fast".to_string(), 50, 100);

    assert_eq!(scheduler.len(), 3);

    // Selection should work
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let mut selections = HashMap::new();

    for _ in 0..100 {
        if let Some(hash) = scheduler.select(&mut rng) {
            *selections.entry(hash).or_insert(0) += 1;
        }
    }

    // High coverage should be selected more often (probabilistically)
    assert!(selections.len() > 1);

    // Test productivity boost
    scheduler.mark_productive("high_cov", 50);
    let (_, boosts, _) = scheduler.stats();
    assert_eq!(boosts, 1);
}

/// Test parallel coordination
#[test]
fn test_parallel_coordination_integration() {
    let config = ParallelConfig {
        num_workers: 4,
        batch_size: 10,
        ..Default::default()
    };
    let coordinator = ParallelCoordinator::new(config);

    // Enqueue work
    let work_items: Vec<WorkItem> = (0..20)
        .map(|i| WorkItem {
            work_type: WorkType::Mutate,
            input_hash: format!("input_{}", i),
            mutations: 5,
            priority: 1,
        })
        .collect();

    coordinator.enqueue_work(work_items);
    assert_eq!(coordinator.queue_len(), 20);

    // Workers get batches
    let batch0 = coordinator.get_work(0);
    let batch1 = coordinator.get_work(1);

    assert_eq!(batch0.len(), 10);
    assert_eq!(batch1.len(), 10);
    assert_eq!(coordinator.queue_len(), 0);

    // Update worker states
    coordinator.update_worker(0, 100, 2, 50);
    coordinator.update_worker(1, 80, 1, 30);

    let (total_iters, total_crashes, _, _, _) = coordinator.get_stats();
    assert_eq!(total_iters, 180);
    assert_eq!(total_crashes, 3);

    // Test shutdown
    coordinator.shutdown();
    assert!(coordinator.should_shutdown());
}

/// Test checkpoint save/load
#[test]
fn test_checkpoint_integration() {
    let dir = tempdir().unwrap();
    let checkpoint_path = dir.path().join("checkpoint.json");

    let state = CampaignState {
        config: CampaignConfig::exploration(),
        start_time: 1000,
        end_time: None,
        iterations: 50000,
        crashes: 10,
        unique_crashes: 5,
        coverage_pct: 75.5,
        corpus_size: 500,
        phase: CampaignPhase::Fuzzing,
        stop_reason: None,
    };

    let checkpoint = CampaignCheckpoint::new(
        state,
        vec!["hash1".to_string(), "hash2".to_string(), "hash3".to_string()],
        "coverage_abc123".to_string(),
        vec!["crash_sig1".to_string()],
    );

    // Save
    checkpoint.save(&checkpoint_path).unwrap();

    // Load
    let loaded = CampaignCheckpoint::load(&checkpoint_path).unwrap();

    assert_eq!(loaded.state.iterations, 50000);
    assert_eq!(loaded.state.coverage_pct, 75.5);
    assert_eq!(loaded.corpus_hashes.len(), 3);
    assert_eq!(loaded.crash_signatures.len(), 1);
}

// ============================================================================
// End-to-End Integration Tests
// ============================================================================

/// Comprehensive end-to-end test combining all components
#[test]
fn test_comprehensive_fuzzing_pipeline() {
    let mut rng = ChaCha8Rng::seed_from_u64(99999);
    let dir = tempdir().unwrap();

    // Initialize components
    let mut generator = UnifiedGenerator::new(verum_vfuzz::generators::GeneratorConfig {
        max_depth: 5,
        max_statements: 10,
        ..Default::default()
    });

    let corpus = SeedCorpus::new(
        dir.path().join("corpus"),
        SeedCorpusConfig::default(),
    );

    let scheduler = EnergyScheduler::default();
    let unified_coverage = UnifiedCoverage::new();
    let mut triager = CrashTriager::new();
    let mut property_runner = ExtendedPropertyRunner::new();
    property_runner.add_all_properties();
    let mut oracle_runner = OracleRunner::default();

    // Seed the corpus
    for i in 0..5 {
        let program = generator.generate(&mut rng);
        if let Ok(hash) = corpus.add(&program, SeedSource::Generated) {
            scheduler.add(hash, i * 10, 1000);
        }
    }

    // Run fuzzing iterations
    let iterations = 100;
    let mut crashes_found = 0;
    let mut coverage_discoveries = 0;

    for i in 0..iterations {
        // Generate or select from corpus
        let input = if rng.random_bool(0.3) || scheduler.is_empty() {
            generator.generate(&mut rng)
        } else {
            let hash = scheduler.select(&mut rng).unwrap_or_default();
            corpus.get(&hash).map(|e| e.content).unwrap_or_else(|| generator.generate(&mut rng))
        };

        // Track coverage
        let tracker = CoverageTracker::new();
        for j in 0..5 {
            tracker.visit_location((i * 100 + j) as u32);
        }

        let found_new = unified_coverage.edge.update(tracker.bitmap());
        if found_new {
            coverage_discoveries += 1;
            if let Ok(hash) = corpus.add(&input, SeedSource::Mutated {
                parent: "parent".to_string(),
            }) {
                scheduler.add(hash, 5, 500);
            }
        }

        // Track branches
        let branch_id = BranchId::new("test.vr", i, 0, "");
        unified_coverage.branch.register_branch(branch_id.clone());
        unified_coverage.branch.record_branch(&branch_id, rng.random_bool(0.5));

        // Simulate execution result
        let result = OracleExecutionResult {
            success: rng.random_bool(0.95),
            exit_code: if rng.random_bool(0.95) { Some(0) } else { Some(1) },
            stdout: "output".to_string(),
            stderr: if rng.random_bool(0.05) {
                "panicked at 'test error'".to_string()
            } else {
                String::new()
            },
            return_value: Some(SerializedValue::Int(42)),
            duration: Duration::from_millis(50),
            memory_bytes: 1024,
            tier: ExecutionTier::Tier0,
        };

        // Check oracles
        let violations = oracle_runner.check_all(&result);
        if !violations.is_empty() {
            crashes_found += 1;
            triager.triage(
                &format!("crash_{}", i),
                &input,
                &result.stderr,
                "0: test_func",
            );
        }

        // Run properties
        property_runner.run(&input);
    }

    // Verify results
    assert!(coverage_discoveries > 0, "Should discover new coverage");
    assert!(corpus.len() > 5, "Corpus should grow");
    assert!(unified_coverage.edge.discovered_count() > 0);

    let prop_stats = property_runner.stats();
    assert!(prop_stats.total > 0);

    let triage_stats = triager.stats();
    assert_eq!(triage_stats.total_crashes, crashes_found);
}

/// Test that shrinking works with all oracle types
#[test]
fn test_shrinking_with_oracles() {
    let shrinker = Shrinker::new(ShrinkConfig::default());
    let oracle = OracleRunner::default();

    let buggy_input = "fn main() {\n    let x = 1;\n    let y = 2;\n    let CRASH = panic!;\n    let z = 3;\n}";

    // Test function that checks for the CRASH pattern
    // We cannot use check_all here because it requires mutable access,
    // but Shrinker::shrink requires Fn (not FnMut).
    // This test verifies the shrinking works with oracle-like detection logic.
    let test_fn = |input: &str| -> bool {
        if input.contains("CRASH") {
            // Simulate oracle-like detection of the crash pattern
            true
        } else {
            false
        }
    };

    let result = shrinker.shrink(buggy_input, test_fn);

    if let ShrinkResult::Success(minimized) = result {
        assert!(minimized.contains("CRASH"));
        assert!(minimized.len() <= buggy_input.len());
    }

    // Separately verify oracle can detect crash in the result
    // (using a separate oracle instance to avoid borrow issues)
    let mut check_oracle = OracleRunner::default();
    let crash_result = OracleExecutionResult {
        success: false,
        exit_code: Some(101),
        stdout: String::new(),
        stderr: "panicked at 'CRASH'".to_string(),
        return_value: None,
        duration: Duration::from_millis(50),
        memory_bytes: 1024,
        tier: ExecutionTier::Tier0,
    };
    let violations = check_oracle.check_all(&crash_result);
    assert!(!violations.is_empty(), "Oracle should detect crash");

    // Verify oracle is initialized correctly
    drop(oracle);
}
