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
// Unit tests for emitter.rs
//
// Migrated from src/emitter.rs to comply with CLAUDE.md test organization.

use verum_diagnostics::{
    DiagnosticBuilder,
    emitter::{Emitter, EmitterConfig, OutputFormat},
};

#[test]
fn test_emitter_config() {
    let config = EmitterConfig::default();
    assert_eq!(config.format, OutputFormat::Human);
    assert!(config.show_source);
    assert!(config.colors);
}

#[test]
fn test_emitter_json_config() {
    let config = EmitterConfig::json();
    assert_eq!(config.format, OutputFormat::Json);
}

#[test]
fn test_emitter_creation() {
    let emitter = Emitter::new(EmitterConfig::default());
    assert_eq!(emitter.diagnostics().len(), 0);
    assert!(!emitter.has_errors());
}

#[test]
fn test_add_diagnostic() {
    let mut emitter = Emitter::default();

    let diag = DiagnosticBuilder::error().message("test error").build();

    emitter.add(diag);
    assert_eq!(emitter.diagnostics().len(), 1);
    assert!(emitter.has_errors());
    assert_eq!(emitter.error_count(), 1);
}

#[test]
fn test_clear_diagnostics() {
    let mut emitter = Emitter::default();

    emitter.add(DiagnosticBuilder::error().message("error").build());
    emitter.add(DiagnosticBuilder::warning().message("warning").build());

    assert_eq!(emitter.diagnostics().len(), 2);

    emitter.clear();
    assert_eq!(emitter.diagnostics().len(), 0);
}

#[test]
fn test_error_warning_counts() {
    let mut emitter = Emitter::default();

    emitter.add(DiagnosticBuilder::error().message("error 1").build());
    emitter.add(DiagnosticBuilder::error().message("error 2").build());
    emitter.add(DiagnosticBuilder::warning().message("warning 1").build());

    assert_eq!(emitter.error_count(), 2);
    assert_eq!(emitter.warning_count(), 1);
    assert!(emitter.has_errors());
}

#[test]
fn test_emit_human() {
    let mut emitter = Emitter::new(EmitterConfig::no_color());
    let mut output = Vec::new();

    let diag = DiagnosticBuilder::error()
        .code("E0308")
        .message("test error")
        .add_note("test note")
        .build();

    emitter.emit(&diag, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("error<E0308>"));
    assert!(text.contains("test error"));
    assert!(text.contains("test note"));
}

#[test]
fn test_emit_json() {
    let mut emitter = Emitter::new(EmitterConfig::json());
    let mut output = Vec::new();

    let diag = DiagnosticBuilder::error()
        .code("E0308")
        .message("test error")
        .build();

    emitter.emit(&diag, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("\"level\": \"error\""));
    assert!(text.contains("\"code\": \"E0308\""));
    assert!(text.contains("\"message\": \"test error\""));
}

#[test]
fn test_emit_all() {
    let mut emitter = Emitter::new(EmitterConfig::no_color());

    emitter.add(DiagnosticBuilder::error().message("error 1").build());
    emitter.add(DiagnosticBuilder::warning().message("warning 1").build());

    let mut output = Vec::new();
    emitter.emit_all(&mut output).unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("error 1"));
    assert!(text.contains("warning 1"));
}

#[test]
fn test_emit_summary() {
    let mut emitter = Emitter::default();

    emitter.add(DiagnosticBuilder::error().message("error").build());
    emitter.add(DiagnosticBuilder::warning().message("warning").build());

    let mut output = Vec::new();
    emitter.emit_summary(&mut output).unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("1 error"));
    assert!(text.contains("1 warning"));
}
