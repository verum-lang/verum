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
// Unit tests for context.rs
//
// Migrated from src/context.rs to comply with CLAUDE.md test organization.

use verum_diagnostics::{
    DiagnosticBuilder, Span,
    context::{Backtrace, CompilerStage, DiagnosticContext, ErrorChain, StackFrame, WithContext},
};

#[test]
fn test_diagnostic_context() {
    let context = DiagnosticContext::new(CompilerStage::TypeChecking)
        .with_file("main.vr")
        .with_scope("calculate")
        .add_metadata("pass", "refinement");

    assert_eq!(context.stage, CompilerStage::TypeChecking);
    assert_eq!(context.file, Some("main.vr".into()));
    assert_eq!(context.scope, Some("calculate".into()));
    assert_eq!(context.metadata.len(), 1);
}

#[test]
fn test_compiler_stage_display() {
    assert_eq!(CompilerStage::Lexing.to_string(), "lexing");
    assert_eq!(CompilerStage::TypeChecking.to_string(), "type checking");
    assert_eq!(CompilerStage::Verification.to_string(), "verification");
}

#[test]
fn test_error_chain() {
    let root = DiagnosticBuilder::error().message("type mismatch").build();

    let context = DiagnosticContext::new(CompilerStage::TypeChecking).with_file("main.vr");

    let related = DiagnosticBuilder::note_diag()
        .message("expected type Int")
        .build();

    let chain = ErrorChain::new(root)
        .add_context(context)
        .add_related(related);

    assert_eq!(chain.contexts().len(), 1);
    assert_eq!(chain.related().len(), 1);
}

#[test]
fn test_with_context_trait() {
    let diag = DiagnosticBuilder::error()
        .message("error")
        .build()
        .in_scope("my_function")
        .in_file("test.vr");

    // Each context adds one child
    assert!(!diag.children().is_empty());
}

#[test]
fn test_backtrace() {
    let frame1 = StackFrame::new("calculate")
        .with_module("math")
        .with_span(Span::new("math.vr", 10, 5, 15));

    let frame2 = StackFrame::new("main");

    let backtrace = Backtrace::capture().add_frame(frame1).add_frame(frame2);

    // Note: backtrace.captured depends on RUST_BACKTRACE=1 environment variable.
    // We only check that frames are added correctly regardless of capture status.
    assert_eq!(backtrace.frames.len(), 2);
}

#[test]
fn test_stack_frame_format() {
    let frame = StackFrame::new("calculate")
        .with_module("math")
        .with_span(Span::new("math.vr", 10, 5, 15));

    let formatted = frame.format();
    assert!(formatted.contains("math::"));
    assert!(formatted.contains("calculate"));
    assert!(formatted.contains("math.vr"));
}
