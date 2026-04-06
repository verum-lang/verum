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
//! Integration tests for the Verum diagnostics system
//!
//! These tests demonstrate the full capabilities of the diagnostic system,
//! particularly for refinement type errors.

use verum_diagnostics::codes;
use verum_diagnostics::context::CompilerStage;
use verum_diagnostics::renderer::RenderConfig;
use verum_diagnostics::*;

/// Test the critical v1.0 refinement error format
#[test]
fn test_refinement_error_v1_format() {
    let error = refinement_error::common::positive_constraint_violation(
        "x",
        "-5",
        Span::new("main.vr", 3, 12, 13),
    );

    let diagnostic = error.to_diagnostic();

    // Verify the error structure
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert_eq!(diagnostic.code(), Some(codes::E0312));
    assert!(
        diagnostic
            .message()
            .contains("refinement constraint not satisfied")
    );

    // Verify the primary label
    let primary_labels = diagnostic.primary_labels();
    assert_eq!(primary_labels.len(), 1);
    assert!(primary_labels[0].message.contains("-5"));
    assert!(primary_labels[0].message.contains("x > 0"));

    // Verify suggestions exist
    assert!(
        diagnostic.helps().len() >= 3,
        "Should have at least 3 help suggestions"
    );

    // Render it
    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "main.vr",
        "fn divide(a: Int, b: Int{> 0}) -> Int {\n    a / b\n}\ndivide(10, x)\n",
    );

    let output = renderer.render(&diagnostic);
    println!("Rendered refinement error:\n{}", output);

    assert!(output.contains("error"));
    assert!(output.contains("E0312"));
}

/// Test division by zero error
#[test]
fn test_division_by_zero_error() {
    let error =
        refinement_error::common::division_by_zero("divisor", "0", Span::new("calc.vr", 5, 10, 17));

    let diagnostic = error.to_diagnostic();

    assert_eq!(diagnostic.severity(), Severity::Error);
    assert!(error.constraint().expression.contains("!= 0"));
    assert_eq!(error.actual_value(), "0");
}

/// Test array bounds violation
#[test]
fn test_bounds_check_error() {
    let error = refinement_error::common::bounds_check_violation(
        "arr",
        "idx",
        "10",
        "5",
        Span::new("array.vr", 12, 5, 8),
    );

    let diagnostic = error.to_diagnostic();

    assert_eq!(diagnostic.severity(), Severity::Error);
    assert!(error.constraint().expression.contains("< arr.len()"));
}

/// Test SMT trace integration
#[test]
fn test_smt_trace() {
    let trace = SMTTrace::new(false)
        .add_step(VerificationStep::new(
            1,
            "assumed",
            "x >= 0 (from parameter refinement)",
        ))
        .add_step(VerificationStep::new(2, "computed", "result = x / 2.0"))
        .add_step(VerificationStep::new(
            3,
            "required",
            "result >= 0 (from return type)",
        ))
        .add_step(VerificationStep::new(4, "checking", "x / 2.0 >= 0"))
        .add_step(VerificationStep::new(
            5,
            "failed",
            "Cannot prove (found counterexample x = -4.0)",
        ))
        .with_counterexample(
            CounterExample::new()
                .add_assignment("x", "-4.0")
                .add_assignment("result", "-2.0"),
        );

    assert!(!trace.succeeded);
    assert_eq!(trace.steps.len(), 5);
    assert!(trace.counterexample.is_some());

    let formatted = trace.format();
    println!("SMT Trace:\n{}", formatted);
    assert!(formatted.contains("[1] assumed"));
    assert!(formatted.contains("Counterexample"));
}

/// Test full refinement error with SMT trace
#[test]
fn test_complete_refinement_error() {
    let trace = SMTTrace::new(false)
        .add_step(VerificationStep::new(1, "assumed", "x: Float{>= 0}"))
        .add_step(VerificationStep::new(2, "checking", "sqrt(x) >= 0"))
        .add_step(VerificationStep::new(3, "failed", "Cannot prove"))
        .with_counterexample(CounterExample::new().add_assignment("x", "-4.0"));

    let error = RefinementErrorBuilder::new()
        .constraint("x >= 0")
        .actual_value("-4.0")
        .expected("non-negative value")
        .span(Span::new("math.vr", 10, 15, 16))
        .trace(trace)
        .suggestion_obj(suggestion::templates::add_refinement_constraint(
            "x", ">= 0",
        ))
        .suggestion_obj(suggestion::templates::use_option_type("Float"))
        .context("in function sqrt")
        .build();

    let diagnostic = error.to_diagnostic();

    // Verify all components are present
    assert!(diagnostic.code().is_some());
    assert!(diagnostic.helps().len() >= 2);
    assert!(diagnostic.notes().len() >= 5); // Trace steps + context
}

/// Test diagnostic builder
#[test]
fn test_diagnostic_builder() {
    let diag = DiagnosticBuilder::error()
        .code(codes::E0308)
        .message("verification failed for function 'sqrt'")
        .span_label(
            Span::new("math.vr", 2, 5, 12),
            "Cannot prove postcondition: result >= 0",
        )
        .add_note("SMT solver found counterexample:")
        .add_note("  x = -4.0 (violates precondition x >= 0)")
        .add_note("  result = -2.0 (violates postcondition)")
        .help("Strengthen precondition: x: Float{> 0}")
        .help("Add assertion: assert!(x > 0) before division")
        .build();

    assert_eq!(diag.severity(), Severity::Error);
    assert_eq!(diag.code(), Some(codes::E0308));
    assert_eq!(diag.notes().len(), 3);
    assert_eq!(diag.helps().len(), 2);
}

/// Test emitter with multiple diagnostics
#[test]
fn test_emitter_accumulation() {
    let mut emitter = Emitter::new(EmitterConfig::no_color());

    // Add multiple diagnostics
    emitter.add(DiagnosticBuilder::error().message("first error").build());

    emitter.add(
        DiagnosticBuilder::warning()
            .message("first warning")
            .build(),
    );

    emitter.add(DiagnosticBuilder::error().message("second error").build());

    assert_eq!(emitter.error_count(), 2);
    assert_eq!(emitter.warning_count(), 1);
    assert!(emitter.has_errors());

    // Emit all
    let mut output = Vec::new();
    emitter.emit_all(&mut output).unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("first error"));
    assert!(text.contains("first warning"));
    assert!(text.contains("second error"));
}

/// Test JSON output format
#[test]
fn test_json_output() {
    let mut emitter = Emitter::new(EmitterConfig::json());
    let mut output = Vec::new();

    let diag = DiagnosticBuilder::error()
        .code(codes::E0312)
        .message("refinement constraint not satisfied")
        .span_label(
            Span::new("main.vr", 3, 12, 13),
            "value `-5` fails constraint `i > 0`",
        )
        .help("use runtime check")
        .build();

    emitter.emit(&diag, &mut output).unwrap();
    let json_text = String::from_utf8(output).unwrap();

    println!("JSON output:\n{}", json_text);

    // Verify JSON structure
    assert!(json_text.contains("\"level\": \"error\""));
    assert!(json_text.contains("\"code\": \"E0312\""));
    assert!(json_text.contains("refinement constraint not satisfied"));
}

/// Test suggestion templates
#[test]
fn test_suggestion_templates() {
    let add_ref = suggestion::templates::add_refinement_constraint("x", "> 0");
    assert_eq!(add_ref.applicability(), Applicability::Recommended);
    assert!(add_ref.snippet().is_some());

    let runtime_check = suggestion::templates::runtime_check("x > 0", "return Err(...)");
    assert_eq!(runtime_check.applicability(), Applicability::Alternative);

    let use_option = suggestion::templates::use_option_type("Int");
    assert!(use_option.snippet().unwrap().code.contains("Option<Int>"));

    let assertion = suggestion::templates::add_assertion("x > 0", "value must be positive");
    assert_eq!(assertion.applicability(), Applicability::MaybeIncorrect);
}

/// Test error context and chaining
#[test]
fn test_error_context() {
    let context = DiagnosticContext::new(CompilerStage::RefinementChecking)
        .with_file("main.vr")
        .with_scope("calculate")
        .add_metadata("pass", "verification");

    assert_eq!(context.stage, CompilerStage::RefinementChecking);
    assert_eq!(context.file, Some("main.vr".into()));

    let formatted = context.format();
    assert!(formatted.contains("refinement checking"));
    assert!(formatted.contains("main.vr"));
}

/// Test error chain
#[test]
fn test_error_chain() {
    let root = DiagnosticBuilder::error()
        .code(codes::E0308)
        .message("type mismatch")
        .build();

    let context1 = DiagnosticContext::new(CompilerStage::TypeChecking)
        .with_file("main.vr")
        .with_scope("foo");

    let context2 = DiagnosticContext::new(CompilerStage::RefinementChecking)
        .with_file("main.vr")
        .with_scope("bar");

    let related = DiagnosticBuilder::note_diag()
        .message("expected type Int")
        .build();

    let chain = ErrorChain::new(root)
        .add_context(context1)
        .add_context(context2)
        .add_related(related);

    assert_eq!(chain.contexts().len(), 2);
    assert_eq!(chain.related().len(), 1);

    let formatted = chain.format();
    println!("Error chain:\n{}", formatted);
    assert!(formatted.contains("Error propagation"));
}

/// Test the complete v1.0 critical example from spec
#[test]
fn test_v1_critical_example() {
    // This is the exact format that v1.0 MUST support
    let error = RefinementErrorBuilder::new()
        .constraint("i > 0")
        .actual_value("-5")
        .span(Span::new("main.vr", 3, 12, 13))
        .suggestion("wrap in runtime check: `PositiveInt::try_from(x)?`")
        .suggestion("or use compile-time proof: `@verify x > 0`")
        .build();

    let diagnostic = error.to_diagnostic();

    // Render with source
    let mut renderer = Renderer::new(RenderConfig::no_color());
    renderer.add_test_content(
        "main.vr",
        "fn main() {\n  let x = -5;\n  divide(10, x)\n}\n",
    );

    let output = renderer.render(&diagnostic);

    println!("\n=== V1.0 CRITICAL EXAMPLE ===\n{}", output);

    // Verify required components
    assert!(output.contains("error"));
    assert!(output.contains("E0312"));
    assert!(output.contains("refinement constraint not satisfied"));
    assert!(output.contains("main.vr:3:12"));
    assert!(output.contains("value `-5` fails constraint `i > 0`"));
    assert!(output.contains("help:"));

    // Verify suggestions are present
    assert!(output.contains("runtime check") || output.contains("try_from"));
}

/// Test rendering with colors disabled
#[test]
fn test_no_color_rendering() {
    let diag = DiagnosticBuilder::error()
        .code("E0001")
        .message("test error")
        .build();

    let mut renderer = Renderer::new(RenderConfig::no_color());
    let output = renderer.render(&diag);

    // Should not contain ANSI color codes
    assert!(!output.contains("\x1b["));
}

/// Test multi-line spans (future enhancement)
#[test]
fn test_multiline_span() {
    let span = Span::new_multiline("test.vr", 10, 5, 15, 20);
    assert!(span.is_multiline());
    assert_eq!(span.line, 10);
    assert_eq!(span.end_line, Some(15));
}

/// Test suggestion builder
#[test]
fn test_suggestion_builder() {
    let suggestion = SuggestionBuilder::new("Fix the issue")
        .description("This explains how to fix it")
        .code("x: Int{> 0}")
        .recommended()
        .build();

    assert_eq!(suggestion.title(), "Fix the issue");
    assert!(suggestion.description().is_some());
    assert!(suggestion.snippet().is_some());
    assert_eq!(suggestion.applicability(), Applicability::Recommended);
    assert!(suggestion.applicability().is_safe_to_apply());
}
