//! Integration tests for vtest
//!
//! These tests verify that the VCS test runner infrastructure works correctly:
//! - Directive parsing from .vr files
//! - Test discovery and filtering
//! - Report generation
//! - Test result handling

use tempfile::tempdir;

use verum_common::{Set, Text};
use vtest::{
    RunSummary, RunnerConfig, VTestToml,
    directive::{
        DirectiveError, ErrorCategory, ExpectedError, Level, TestDirectives, TestType, Tier,
        discover_tests,
    },
    executor::{ExecutorConfig, ParsedError, ProcessOutput, TestOutcome, TestResult},
    list_tests,
    report::{ReportFormat, Reporter},
};

// ============================================================================
// Directive Parsing Tests
// ============================================================================

/// Test parsing a simple parse-pass test
#[test]
fn test_parse_simple_parse_pass() {
    let content = r#"
// @test: parse-pass
// @tier: all
// @level: L0
// @tags: lexer, keywords

fn main() {
    let x = 42;
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::ParsePass);
    assert_eq!(directives.level, Level::L0);
    assert_eq!(directives.tiers.len(), 4); // all = Tier 0, 1, 2, 3
    assert!(directives.tags.contains(&Text::from("lexer")));
    assert!(directives.tags.contains(&Text::from("keywords")));
}

/// Test parsing a typecheck-fail test with expected errors
#[test]
fn test_parse_typecheck_fail_with_errors() {
    let content = r#"
// @test: typecheck-fail
// @tier: all
// @level: L0
// @tags: types, errors
// @expected-error: E201 "Type mismatch" at line 8

fn main() {
    let x: Int = "hello"; // Type error!
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::TypecheckFail);
    assert_eq!(directives.expected_errors.len(), 1);

    let err = &directives.expected_errors[0];
    assert_eq!(err.code, "E201");
    assert_eq!(err.message, Some("Type mismatch".to_string().into()));
    assert_eq!(err.line, Some(8));
}

/// Test parsing a run test with expected output
#[test]
fn test_parse_run_with_stdout() {
    let content = r#"
// @test: run
// @tier: 0, 3
// @level: L1
// @tags: runtime
// @expected-stdout: Hello, World!
// @expected-exit: 0

fn main() {
    print("Hello, World!");
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::Run);
    assert_eq!(directives.level, Level::L1);
    assert_eq!(directives.tiers.len(), 2);
    assert!(directives.tiers.contains(&Tier::Tier0));
    assert!(directives.tiers.contains(&Tier::Tier3));
    assert_eq!(
        directives.expected_stdout,
        Some("Hello, World!".to_string().into())
    );
    assert_eq!(directives.expected_exit, Some(0));
}

/// Test parsing a run-panic test
#[test]
fn test_parse_run_panic() {
    let content = r#"
// @test: run-panic
// @tier: all
// @level: L0
// @tags: safety, panic
// @expected-panic: "Use after free"

fn main() {
    let dangling = create_dangling();
    *dangling; // Panic!
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::RunPanic);
    assert_eq!(
        directives.expected_panic,
        Some("Use after free".to_string().into())
    );
}

/// Test parsing multiline expected stdout
#[test]
fn test_parse_multiline_stdout() {
    let content = r#"
// @test: run
// @tier: all
// @level: L1
// @expected-stdout-begin
// Line 1
// Line 2
// Line 3
// @expected-stdout-end

fn main() {
    println("Line 1");
    println("Line 2");
    println("Line 3");
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(
        directives.expected_stdout,
        Some("Line 1\nLine 2\nLine 3".to_string().into())
    );
}

/// Test parsing with skip directive
#[test]
fn test_parse_skip_directive() {
    let content = r#"
// @test: run
// @tier: all
// @level: L1
// @skip: GPU not available in CI

fn main() {
    gpu_compute();
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(directives.skip, Some("GPU not available in CI".to_string().into()));
}

/// Test parsing with requires directive
#[test]
fn test_parse_requires_directive() {
    let content = r#"
// @test: run
// @tier: all
// @level: L2
// @requires: ffi, network

fn main() {
    call_external();
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert!(directives.requires.contains(&Text::from("ffi")));
    assert!(directives.requires.contains(&Text::from("network")));
}

/// Test parsing benchmark test
#[test]
fn test_parse_benchmark() {
    let content = r#"
// @test: benchmark
// @tier: 3
// @level: L4
// @tags: performance, cbgr
// @expected-performance: < 15ns
// @timeout: 60000

fn cbgr_check() {
    // CBGR reference check
}
"#;

    let directives = TestDirectives::parse(content, "test.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::Benchmark);
    assert_eq!(directives.level, Level::L4);
    assert_eq!(directives.expected_performance, Some("< 15ns".to_string().into()));
    assert_eq!(directives.timeout_ms, Some(60000));
}

/// Test missing @test directive error
#[test]
fn test_missing_test_directive() {
    let content = r#"
// @tier: all
// @level: L0

fn main() {}
"#;

    let result = TestDirectives::parse(content, "test.vr".into());
    assert!(matches!(result, Err(DirectiveError::MissingTestDirective)));
}

/// Test unclosed block error
#[test]
fn test_unclosed_block_error() {
    let content = r#"
// @test: run
// @tier: all
// @level: L0
// @expected-stdout-begin
// Line 1

fn main() {}
"#;

    let result = TestDirectives::parse(content, "test.vr".into());
    assert!(matches!(result, Err(DirectiveError::UnclosedBlock { .. })));
}

/// Test conflicting directives error
#[test]
fn test_conflicting_stdout_directives() {
    let content = r#"
// @test: run
// @tier: all
// @level: L0
// @expected-stdout: hello
// @expected-stdout-file: expected.txt

fn main() {}
"#;

    let result = TestDirectives::parse(content, "test.vr".into());
    assert!(matches!(
        result,
        Err(DirectiveError::ConflictingDirectives(_))
    ));
}

/// Test run-panic requires expected-panic
#[test]
fn test_run_panic_requires_expected_panic() {
    let content = r#"
// @test: run-panic
// @tier: all
// @level: L0

fn main() { panic("oops"); }
"#;

    let result = TestDirectives::parse(content, "test.vr".into());
    assert!(matches!(result, Err(DirectiveError::ValidationError(_))));
}

// ============================================================================
// Expected Error Tests
// ============================================================================

/// Test parsing various error specification formats
#[test]
fn test_expected_error_formats() {
    // Simple format
    let err = ExpectedError::parse("E302").unwrap();
    assert_eq!(err.code, "E302");
    assert_eq!(err.message, None);
    assert_eq!(err.line, None);

    // With message
    let err = ExpectedError::parse(r#"E302 "Use after move""#).unwrap();
    assert_eq!(err.code, "E302");
    assert_eq!(err.message, Some("Use after move".to_string().into()));

    // With message and line
    let err = ExpectedError::parse(r#"E302 "Use after move" at line 8"#).unwrap();
    assert_eq!(err.code, "E302");
    assert_eq!(err.message, Some("Use after move".to_string().into()));
    assert_eq!(err.line, Some(8));

    // With line and column
    let err = ExpectedError::parse(r#"E302 at line 8, col 10"#).unwrap();
    assert_eq!(err.line, Some(8));
    assert_eq!(err.column, Some(10));

    // Compact format
    let err = ExpectedError::parse("E302 at 8:10").unwrap();
    assert_eq!(err.line, Some(8));
    assert_eq!(err.column, Some(10));

    // Column range
    let err = ExpectedError::parse("E302 at 8:10-15").unwrap();
    assert_eq!(err.column, Some(10));
    assert_eq!(err.end_column, Some(15));

    // With severity
    let err = ExpectedError::parse(r#"[error] E302 "message""#).unwrap();
    assert_eq!(err.severity, Some("error".to_string().into()));
}

/// Test error category detection
#[test]
fn test_error_categories() {
    assert_eq!(ErrorCategory::from_code("E001"), Some(ErrorCategory::Parse));
    assert_eq!(ErrorCategory::from_code("E123"), Some(ErrorCategory::Lexer));
    assert_eq!(ErrorCategory::from_code("E234"), Some(ErrorCategory::Type));
    assert_eq!(
        ErrorCategory::from_code("E345"),
        Some(ErrorCategory::Borrow)
    );
    assert_eq!(
        ErrorCategory::from_code("E456"),
        Some(ErrorCategory::Verification)
    );
    assert_eq!(
        ErrorCategory::from_code("E567"),
        Some(ErrorCategory::Context)
    );
    assert_eq!(
        ErrorCategory::from_code("E678"),
        Some(ErrorCategory::Module)
    );
    assert_eq!(ErrorCategory::from_code("E789"), Some(ErrorCategory::Async));
    assert_eq!(ErrorCategory::from_code("E890"), Some(ErrorCategory::Ffi));
    assert_eq!(
        ErrorCategory::from_code("E901"),
        Some(ErrorCategory::Internal)
    );

    // Warning codes also work
    assert_eq!(
        ErrorCategory::from_code("W302"),
        Some(ErrorCategory::Borrow)
    );
}

/// Test expected error matching
#[test]
fn test_expected_error_matching() {
    let expected = ExpectedError {
        code: "E302".to_string().into(),
        message: Some("Use after move".to_string().into()),
        line: Some(10),
        column: Some(5),
        end_column: None,
        severity: None,
        category: Some(ErrorCategory::Borrow),
    };

    // Exact match
    assert!(expected.matches("E302", Some("Use after move"), Some(10), Some(5)));

    // Message substring
    assert!(expected.matches(
        "E302",
        Some("Error: Use after move in x"),
        Some(10),
        Some(5)
    ));

    // Wrong code
    assert!(!expected.matches("E303", Some("Use after move"), Some(10), Some(5)));

    // Wrong line
    assert!(!expected.matches("E302", Some("Use after move"), Some(11), Some(5)));
}

/// Test expected error column range matching
#[test]
fn test_expected_error_column_range() {
    let expected = ExpectedError {
        code: "E302".to_string().into(),
        message: None,
        line: Some(10),
        column: Some(5),
        end_column: Some(10),
        severity: None,
        category: None,
    };

    // Within range
    assert!(expected.matches("E302", None, Some(10), Some(5)));
    assert!(expected.matches("E302", None, Some(10), Some(7)));
    assert!(expected.matches("E302", None, Some(10), Some(10)));

    // Outside range
    assert!(!expected.matches("E302", None, Some(10), Some(4)));
    assert!(!expected.matches("E302", None, Some(10), Some(11)));
}

/// Test expected error stderr matching
#[test]
fn test_expected_error_stderr_matching() {
    let expected = ExpectedError {
        code: "E302".to_string().into(),
        message: Some("Use after move".to_string().into()),
        line: Some(10),
        column: None,
        end_column: None,
        severity: None,
        category: Some(ErrorCategory::Borrow),
    };

    let stderr = r#"error[E302]: Use after move
  --> test.vr:10:5
   |
10 |     let x = y;
   |         ^ value moved here
"#;

    assert!(expected.matches_stderr(stderr));

    // Wrong code
    let wrong_code = stderr.replace("E302", "E303");
    assert!(!expected.matches_stderr(&wrong_code));
}

// ============================================================================
// Parsed Error Tests
// ============================================================================

/// Test parsing Rust-style errors
#[test]
fn test_parse_rust_style_errors() {
    let stderr = r#"error[E302]: Use after move
  --> test.vr:10:5
   |
10 |     let x = y;
   |         ^ value moved here

error[E201]: Type mismatch
  --> test.vr:15:10
   |
15 |     x + "hello"
   |         ^^^^^^^ expected Int, found Text
"#;

    let errors = ParsedError::parse_stderr(stderr);
    assert_eq!(errors.len(), 2);

    assert_eq!(errors[0].code, "E302");
    assert_eq!(errors[0].message, "Use after move");
    assert_eq!(errors[0].line, Some(10));
    assert_eq!(errors[0].column, Some(5));

    assert_eq!(errors[1].code, "E201");
    assert_eq!(errors[1].line, Some(15));
    assert_eq!(errors[1].column, Some(10));
}

/// Test parsing GCC-style errors
#[test]
fn test_parse_gcc_style_errors() {
    let stderr = r#"test.vr:10:5: error[E302]: Use after move
test.vr:15:10: warning[W101]: Unused variable
"#;

    let errors = ParsedError::parse_stderr(stderr);
    assert_eq!(errors.len(), 2);

    assert_eq!(errors[0].code, "E302");
    assert_eq!(errors[0].line, Some(10));
    assert_eq!(errors[0].column, Some(5));
    assert_eq!(errors[0].severity, "error");

    assert_eq!(errors[1].code, "W101");
    assert_eq!(errors[1].severity, "warning");
}

// ============================================================================
// Test Discovery Tests
// ============================================================================

/// Test discovering tests in a directory
#[test]
fn test_discover_tests() {
    let dir = tempdir().unwrap();

    // Create test files
    let test1 = r#"
// @test: parse-pass
// @tier: all
// @level: L0
fn main() {}
"#;

    let test2 = r#"
// @test: run
// @tier: all
// @level: L1
fn main() { 42 }
"#;

    // File without @test directive (should be skipped)
    let not_a_test = "fn helper() {}";

    std::fs::write(dir.path().join("test1.vr"), test1).unwrap();
    std::fs::write(dir.path().join("test2.vr"), test2).unwrap();
    std::fs::write(dir.path().join("helper.vr"), not_a_test).unwrap();

    let paths = discover_tests(dir.path(), "*.vr", &[]).unwrap();
    assert_eq!(paths.len(), 3); // All .vr files found

    // Filter to actual tests
    let tests = list_tests(
        &[dir.path().to_path_buf()],
        "*.vr",
        &[],
        &Set::new(),
        &Set::new(),
    )
    .unwrap();

    assert_eq!(tests.len(), 2); // Only files with @test
}

/// Test filtering by level
#[test]
fn test_filter_by_level() {
    let dir = tempdir().unwrap();

    let l0_test = r#"
// @test: parse-pass
// @tier: all
// @level: L0
fn main() {}
"#;

    let l1_test = r#"
// @test: run
// @tier: all
// @level: L1
fn main() { 42 }
"#;

    std::fs::write(dir.path().join("l0.vr"), l0_test).unwrap();
    std::fs::write(dir.path().join("l1.vr"), l1_test).unwrap();

    let mut levels = Set::new();
    levels.insert(Level::L0);

    let tests = list_tests(
        &[dir.path().to_path_buf()],
        "*.vr",
        &[],
        &levels,
        &Set::new(),
    )
    .unwrap();

    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].level, Level::L0);
}

/// Test filtering by tags
#[test]
fn test_filter_by_tags() {
    let dir = tempdir().unwrap();

    let parser_test = r#"
// @test: parse-pass
// @tier: all
// @level: L0
// @tags: parser, syntax
fn main() {}
"#;

    let lexer_test = r#"
// @test: parse-pass
// @tier: all
// @level: L0
// @tags: lexer, tokens
fn main() {}
"#;

    std::fs::write(dir.path().join("parser.vr"), parser_test).unwrap();
    std::fs::write(dir.path().join("lexer.vr"), lexer_test).unwrap();

    let mut include_tags: Set<Text> = Set::new();
    include_tags.insert("parser".to_string().into());

    let tests = list_tests(
        &[dir.path().to_path_buf()],
        "*.vr",
        &[],
        &Set::new(),
        &include_tags,
    )
    .unwrap();

    assert_eq!(tests.len(), 1);
    assert!(tests[0].tags.contains(&Text::from("parser")));
}

/// Test exclusion patterns
#[test]
fn test_exclusion_patterns() {
    let dir = tempdir().unwrap();

    // Create skip directory
    std::fs::create_dir_all(dir.path().join("skip")).unwrap();

    let test1 = r#"
// @test: parse-pass
// @tier: all
// @level: L0
fn main() {}
"#;

    std::fs::write(dir.path().join("good.vr"), test1).unwrap();
    std::fs::write(dir.path().join("skip/skipped.vr"), test1).unwrap();

    let paths = discover_tests(dir.path(), "**/*.vr", &["**/skip/**"]).unwrap();

    // Only good.vr should be found
    assert_eq!(paths.len(), 1);
    assert!(paths[0].contains("good.vr"));
}

// ============================================================================
// Test Result Tests
// ============================================================================

/// Test TestOutcome states
#[test]
fn test_outcome_states() {
    use std::time::Duration;

    let pass = TestOutcome::Pass {
        tier: Tier::Tier0,
        duration: Duration::from_millis(100),
    };
    assert!(pass.is_pass());
    assert!(!pass.is_fail());
    assert!(!pass.is_skip());

    let fail = TestOutcome::Fail {
        tier: Tier::Tier0,
        reason: "Test failed".to_string().into(),
        expected: Some("success".to_string().into()),
        actual: Some("failure".to_string().into()),
        duration: Duration::from_millis(100),
    };
    assert!(!fail.is_pass());
    assert!(fail.is_fail());

    let skip = TestOutcome::Skip {
        tier: Tier::Tier0,
        reason: "Feature not available".to_string().into(),
    };
    assert!(skip.is_skip());
}

/// Test TestResult aggregation
#[test]
fn test_result_aggregation() {
    use std::time::Duration;

    let directives = TestDirectives::default();

    let outcomes = vec![
        TestOutcome::Pass {
            tier: Tier::Tier0,
            duration: Duration::from_millis(100),
        },
        TestOutcome::Pass {
            tier: Tier::Tier1,
            duration: Duration::from_millis(50),
        },
        TestOutcome::Fail {
            tier: Tier::Tier3,
            reason: "Timeout".to_string().into(),
            expected: None,
            actual: None,
            duration: Duration::from_millis(5000),
        },
    ];

    let result = TestResult {
        directives,
        outcomes: outcomes.into(),
        total_duration: Duration::from_secs(5),
    };

    assert!(!result.all_pass()); // Has a failure
    assert_eq!(result.pass_count(), 2);
    assert_eq!(result.fail_count(), 1);
    assert_eq!(result.failure_reasons().len(), 1);
}

// ============================================================================
// Reporter Tests
// ============================================================================

/// Test console report generation
#[test]
fn test_console_report() {
    use std::time::Duration;

    let mut reporter = Reporter::new("v0.1.0".to_string().into()).with_colors(false);

    let directives = TestDirectives {
        test_type: TestType::ParsePass,
        source_path: "test.vr".to_string().into(),
        level: Level::L0,
        ..Default::default()
    };

    let result = TestResult {
        directives,
        outcomes: vec![TestOutcome::Pass {
            tier: Tier::Tier0,
            duration: Duration::from_millis(10),
        }].into(),
        total_duration: Duration::from_millis(10),
    };

    reporter.add_results(vec![result].into());

    let mut output = Vec::new();
    reporter
        .generate(&mut output, ReportFormat::Console)
        .unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.contains("VTEST"));
    assert!(output_str.contains("SUMMARY"));
}

/// Test JSON report generation
#[test]
fn test_json_report() {
    use std::time::Duration;

    let mut reporter = Reporter::new("v0.1.0".to_string().into());

    let directives = TestDirectives {
        test_type: TestType::Run,
        source_path: "test.vr".to_string().into(),
        level: Level::L1,
        ..Default::default()
    };

    let result = TestResult {
        directives,
        outcomes: vec![TestOutcome::Pass {
            tier: Tier::Tier0,
            duration: Duration::from_millis(10),
        }].into(),
        total_duration: Duration::from_millis(10),
    };

    reporter.add_results(vec![result].into());

    let mut output = Vec::new();
    reporter.generate(&mut output, ReportFormat::Json).unwrap();

    let output_str = String::from_utf8(output).unwrap();

    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&output_str).unwrap();
    assert!(parsed.get("summary").is_some());
    assert!(parsed.get("results").is_some());
}

// ============================================================================
// Configuration Tests
// ============================================================================

/// Test default configuration
#[test]
fn test_default_config() {
    let config = RunnerConfig::default();

    assert!(!config.test_paths.is_empty());
    assert_eq!(config.test_pattern, "**/*.vr");
    assert!(config.parallel > 0);
    assert_eq!(config.default_timeout_ms, 30_000);
    assert!(!config.fail_fast);
}

/// Test VTestToml parsing
#[test]
fn test_vtest_toml_parsing() {
    let dir = tempdir().unwrap();
    let toml_path = dir.path().join("vtest.toml");

    let toml_content = r#"
[discovery]
paths = ["specs/"]
pattern = "**/*.vr"
exclude = ["**/skip/**"]

[execution]
parallel = 4
timeout_default = 60000
tier_default = "all"

[reporting]
format = "console"
colors = true
verbose = false
"#;

    std::fs::write(&toml_path, toml_content).unwrap();

    let config = VTestToml::from_file(&toml_path).unwrap();

    let expected_paths: verum_common::List<Text> = vec!["specs/".to_string().into()].into();
    assert_eq!(config.discovery.paths, expected_paths);
    assert_eq!(config.execution.parallel, 4);
    assert_eq!(config.execution.timeout_default, 60000);
    assert!(config.reporting.colors);
}

/// Test VTestToml to RunnerConfig conversion
#[test]
fn test_vtest_toml_to_runner_config() {
    let vtest_config = VTestToml::default();
    let runner_config = vtest_config.to_runner_config();

    assert!(!runner_config.test_paths.is_empty());
    assert!(runner_config.parallel > 0);
}

// ============================================================================
// Tier and Level Tests
// ============================================================================

/// Test tier parsing
#[test]
fn test_tier_parsing() {
    assert_eq!(Tier::from_str("0").unwrap(), Tier::Tier0);
    assert_eq!(Tier::from_str("1").unwrap(), Tier::Tier1);
    assert_eq!(Tier::from_str("2").unwrap(), Tier::Tier2);
    assert_eq!(Tier::from_str("3").unwrap(), Tier::Tier3);

    assert!(Tier::from_str("4").is_err());
    assert!(Tier::from_str("invalid").is_err());
}

/// Test level parsing
#[test]
fn test_level_parsing() {
    assert_eq!(Level::from_str("L0").unwrap(), Level::L0);
    assert_eq!(Level::from_str("l1").unwrap(), Level::L1);
    assert_eq!(Level::from_str("L2").unwrap(), Level::L2);
    assert_eq!(Level::from_str("L3").unwrap(), Level::L3);
    assert_eq!(Level::from_str("L4").unwrap(), Level::L4);

    assert!(Level::from_str("L5").is_err());
}

/// Test level pass thresholds
#[test]
fn test_level_thresholds() {
    assert_eq!(Level::L0.pass_threshold(), 1.0);
    assert_eq!(Level::L1.pass_threshold(), 1.0);
    assert_eq!(Level::L2.pass_threshold(), 0.95);
    assert_eq!(Level::L3.pass_threshold(), 0.90);
    assert_eq!(Level::L4.pass_threshold(), 0.0); // Advisory
}

// ============================================================================
// Test Type Tests
// ============================================================================

/// Test all test types can be parsed
#[test]
fn test_all_test_types() {
    let test_types = [
        ("parse-pass", TestType::ParsePass),
        ("parse-fail", TestType::ParseFail),
        ("typecheck-pass", TestType::TypecheckPass),
        ("typecheck-fail", TestType::TypecheckFail),
        ("verify-pass", TestType::VerifyPass),
        ("verify-fail", TestType::VerifyFail),
        ("run", TestType::Run),
        ("run-panic", TestType::RunPanic),
        ("compile-only", TestType::CompileOnly),
        ("differential", TestType::Differential),
        ("benchmark", TestType::Benchmark),
    ];

    for (name, expected) in test_types {
        let parsed = TestType::from_str(name).unwrap();
        assert_eq!(parsed, expected, "Failed for {}", name);
    }
}

/// Test test type properties
#[test]
fn test_test_type_properties() {
    // Execution required
    assert!(!TestType::ParsePass.requires_execution());
    assert!(!TestType::ParseFail.requires_execution());
    assert!(TestType::Run.requires_execution());
    assert!(TestType::RunPanic.requires_execution());
    assert!(TestType::Benchmark.requires_execution());

    // Compile success expected
    assert!(!TestType::ParseFail.expects_compile_success());
    assert!(!TestType::TypecheckFail.expects_compile_success());
    assert!(TestType::Run.expects_compile_success());
    assert!(TestType::CompileOnly.expects_compile_success());
}

// ============================================================================
// Integration with Spec Files
// ============================================================================

/// Test parsing an actual spec file from the repository
#[test]
fn test_parse_actual_spec_file() {
    // This test uses the actual spec file format from the VCS
    let content = r#"
// @test: parse-pass
// @tier: all
// @level: L0
// @tags: lexer, keywords, primary

// Primary keywords for type definitions and constraints

type Point = {
    x: Float,
    y: Float,
}

type Color = Red | Green | Blue

fn print_all<T>(items: List<T>) where T: Display {
    for item in items {
        print(item.display())
    }
}
"#;

    let directives = TestDirectives::parse(content, "primary_keywords.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::ParsePass);
    assert_eq!(directives.level, Level::L0);
    assert!(directives.tags.contains(&Text::from("lexer")));
    assert!(directives.tags.contains(&Text::from("keywords")));
    assert_eq!(directives.tiers.len(), 4); // all = 4 tiers
}

/// Test parsing CBGR use-after-free spec
#[test]
fn test_parse_cbgr_spec() {
    let content = r#"
// @test: run-panic
// @expected-panic: "Use after free: generation mismatch"
// @tier: all
// @level: L0
// @tags: ownership, cbgr, use-after-free, runtime, safety

/// Test CBGR detection of use-after-free at runtime.

fn main() {
    let dangling_ref = create_dangling_reference();
    let _ = *dangling_ref;  // PANIC: Use after free
}
"#;

    let directives = TestDirectives::parse(content, "use_after_free.vr".into()).unwrap();

    assert_eq!(directives.test_type, TestType::RunPanic);
    assert_eq!(
        directives.expected_panic,
        Some("Use after free: generation mismatch".to_string().into())
    );
    assert!(directives.tags.contains(&Text::from("cbgr")));
    assert!(directives.tags.contains(&Text::from("safety")));
}

/// Test multiple expected errors
#[test]
fn test_multiple_expected_errors() {
    let content = r#"
// @test: typecheck-fail
// @tier: all
// @level: L0
// @expected-error: E201 "Type mismatch" at line 8
// @expected-error: E302 "Use after move" at line 10
// @expected-error-count: 2

fn main() {
    let x: Int = "hello";  // E201
    let y = x;
    println(x);  // E302
}
"#;

    let directives = TestDirectives::parse(content, "errors.vr".into()).unwrap();

    assert_eq!(directives.expected_errors.len(), 2);
    assert_eq!(directives.expected_error_count, Some(2));
    assert_eq!(directives.expected_errors[0].code, "E201");
    assert_eq!(directives.expected_errors[1].code, "E302");
}

// ============================================================================
// Executor Config Tests
// ============================================================================

/// Test default executor config
#[test]
fn test_executor_config_default() {
    let config = ExecutorConfig::default();

    assert_eq!(config.default_timeout_ms, 30_000);
    assert!(config.env.is_empty());
}

/// Test command for tier
#[test]
fn test_command_for_tier() {
    let config = ExecutorConfig::default();

    // Tier 0 uses the interpreter with no additional args (wrapper script handles 'run')
    let (_path, args) = config.command_for_tier(Tier::Tier0);
    assert!(args.is_empty());

    let (_path, args) = config.command_for_tier(Tier::Tier1);
    assert!(args.contains(&Text::from("--baseline")));

    let (_path, args) = config.command_for_tier(Tier::Tier2);
    assert!(args.contains(&Text::from("--optimize")));
}

// ============================================================================
// Process Output Tests
// ============================================================================

/// Test process output default
#[test]
fn test_process_output_default() {
    let output = ProcessOutput::default();

    assert_eq!(output.exit_code, None);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!output.timed_out);
}

// ============================================================================
// Summary Tests
// ============================================================================

/// Test run summary default
#[test]
fn test_run_summary_default() {
    let summary = RunSummary::default();

    assert_eq!(summary.total, 0);
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.skipped, 0);
}
