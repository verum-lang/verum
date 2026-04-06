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
//! Integration tests for capability attenuation error diagnostics
//!
//! This test suite demonstrates all four capability attenuation error codes:
//! - E0306: Capability Violation
//! - E0307: Sub-Context Not Found
//! - E0308: Capability Not Provided
//! - E0309: Partial Implementation Warning

use verum_common::span::LineColSpan;
use verum_diagnostics::{
    CapabilityNotProvidedError, CapabilityViolationError, PartialImplementationWarning, Severity,
    SubContextNotFoundError,
};

fn dummy_span(line: usize, col: usize) -> LineColSpan {
    LineColSpan::new("example.vr", line, col, 10)
}

#[test]
fn test_e0306_capability_violation() {
    // E0306: Function uses Database::Execute but only declares Database::Query
    let error = CapabilityViolationError::new("Database::Execute", dummy_span(6, 5))
        .with_declared_capabilities(vec!["Database::Query".into()].into())
        .with_declaration_span(dummy_span(4, 5))
        .with_function_span(dummy_span(3, 1))
        .with_function_name("attempt_delete");

    let diagnostic = error.build();

    // Verify error code
    assert_eq!(diagnostic.code(), Some("E0306"));
    assert_eq!(diagnostic.severity(), Severity::Error);

    // Verify message contains key information
    assert!(
        diagnostic
            .message()
            .contains("capability `Database::Execute` not declared")
    );

    // Verify we have labels
    assert_eq!(diagnostic.primary_labels().len(), 1);
    assert!(!diagnostic.secondary_labels().is_empty());

    // Verify we have helpful suggestions
    assert!(!diagnostic.helps().is_empty());
    let help_text: Vec<String> = diagnostic
        .helps()
        .iter()
        .map(|h| h.message.to_string())
        .collect();

    // Should suggest adding the capability
    assert!(
        help_text
            .iter()
            .any(|h| h.contains("add Database::Execute"))
    );

    // Verify we have explanatory notes
    assert!(!diagnostic.notes().is_empty());
    let note_text: Vec<String> = diagnostic
        .notes()
        .iter()
        .map(|n| n.message.to_string())
        .collect();

    assert!(
        note_text
            .iter()
            .any(|n| n.contains("function signature declares"))
    );
}

#[test]
fn test_e0307_sub_context_not_found() {
    // E0307: Reference to Database.Read when only Query and Execute exist
    let error = SubContextNotFoundError::new("Database", "Read", dummy_span(2, 12))
        .with_available_sub_contexts(vec!["Query".into(), "Execute".into()].into());

    let diagnostic = error.build();

    // Verify error code
    assert_eq!(diagnostic.code(), Some("E0307"));
    assert_eq!(diagnostic.severity(), Severity::Error);

    // Verify message
    assert!(
        diagnostic
            .message()
            .contains("sub-context `Read` not found")
    );
    assert!(diagnostic.message().contains("Database"));

    // Verify we have labels
    assert_eq!(diagnostic.primary_labels().len(), 1);

    // Verify we list available sub-contexts in notes
    let note_text: Vec<String> = diagnostic
        .notes()
        .iter()
        .map(|n| n.message.to_string())
        .collect();

    assert!(
        note_text
            .iter()
            .any(|n| n.contains("defines these sub-contexts"))
    );
    assert!(note_text.iter().any(|n| n.contains("Query")));
    assert!(note_text.iter().any(|n| n.contains("Execute")));

    // Verify we have helpful suggestions
    assert!(!diagnostic.helps().is_empty());
}

#[test]
fn test_e0307_with_similar_suggestions() {
    // E0307: Typo in sub-context name - should suggest similar names
    let error = SubContextNotFoundError::new("Database", "Execut", dummy_span(2, 12))
        .with_available_sub_contexts(vec!["Query".into(), "Execute".into(), "Admin".into()].into());

    let diagnostic = error.build();

    // Should suggest "Execute" since it's similar to "Execut"
    let help_text: Vec<String> = diagnostic
        .helps()
        .iter()
        .map(|h| h.message.to_string())
        .collect();

    assert!(help_text.iter().any(|h| h.contains("did you mean")));
    assert!(help_text.iter().any(|h| h.contains("Execute")));
}

#[test]
fn test_e0308_capability_not_provided() {
    // E0308: FileSystem::Read required but not provided in environment
    let error = CapabilityNotProvidedError::new("FileSystem::Read", dummy_span(9, 16))
        .with_function_name("read_file")
        .with_function_declaration_span(dummy_span(5, 1));

    let diagnostic = error.build();

    // Verify error code
    assert_eq!(diagnostic.code(), Some("E0308"));
    assert_eq!(diagnostic.severity(), Severity::Error);

    // Verify message
    assert!(
        diagnostic
            .message()
            .contains("capability `FileSystem::Read` required but not provided")
    );

    // Verify we have labels
    assert_eq!(diagnostic.primary_labels().len(), 1);
    assert!(!diagnostic.secondary_labels().is_empty());

    // Verify we suggest providing the capability
    let help_text: Vec<String> = diagnostic
        .helps()
        .iter()
        .map(|h| h.message.to_string())
        .collect();

    assert!(help_text.iter().any(|h| h.contains("provide FileSystem")));
    assert!(help_text.iter().any(|h| h.contains("RealFileSystem.new()")));

    // Verify we have explanatory notes
    assert!(!diagnostic.notes().is_empty());
}

#[test]
fn test_e0309_partial_implementation_warning() {
    // E0309: ReadOnlyFS only implements Read, not Write or Admin
    let error = PartialImplementationWarning::new("FileSystem", "ReadOnlyFS", dummy_span(5, 1))
        .with_implemented_sub_contexts(vec!["Read".into()].into())
        .with_missing_sub_contexts(vec!["Write".into(), "Admin".into()].into());

    let diagnostic = error.build();

    // Verify error code
    assert_eq!(diagnostic.code(), Some("E0309"));
    assert_eq!(diagnostic.severity(), Severity::Warning); // Warning, not Error

    // Verify message
    assert!(
        diagnostic
            .message()
            .contains("partial implementation of context")
    );

    // Verify we list implemented and missing sub-contexts
    let note_text: Vec<String> = diagnostic
        .notes()
        .iter()
        .map(|n| n.message.to_string())
        .collect();

    assert!(
        note_text
            .iter()
            .any(|n| n.contains("implemented sub-contexts: Read"))
    );
    assert!(
        note_text
            .iter()
            .any(|n| n.contains("missing sub-contexts: Write, Admin"))
    );

    // Verify we suggest documenting
    let help_text: Vec<String> = diagnostic
        .helps()
        .iter()
        .map(|h| h.message.to_string())
        .collect();

    assert!(help_text.iter().any(|h| h.contains("document")));
}

#[test]
fn test_errors_provide_context_information() {
    // All capability attenuation errors should provide context about the context system

    let e0306 = CapabilityViolationError::new("Database::Execute", dummy_span(6, 5)).build();
    let e0307 = SubContextNotFoundError::new("Database", "Read", dummy_span(2, 12))
        .with_available_sub_contexts(vec!["Query".into(), "Execute".into()].into())
        .build();
    let e0308 = CapabilityNotProvidedError::new("FileSystem::Read", dummy_span(9, 16)).build();
    let e0309 =
        PartialImplementationWarning::new("FileSystem", "ReadOnlyFS", dummy_span(5, 1)).build();

    // All errors should mention "context" or "capability" in their messages
    for diagnostic in &[&e0306, &e0307, &e0308, &e0309] {
        let message = diagnostic.message().to_lowercase();
        assert!(
            message.contains("context") || message.contains("capability"),
            "Error message should mention context or capability system"
        );
    }

    // E0306 should reference capability attenuation and security
    assert!(
        e0306
            .notes()
            .iter()
            .any(|n| n.message.contains("capability attenuation"))
    );

    // E0308 should mention providing contexts
    assert!(e0308.helps().iter().any(|h| h.message.contains("provide")));
}

#[test]
fn test_capability_violation_with_multiple_capabilities() {
    // When a function has multiple declared capabilities, suggest creating a context group
    let error = CapabilityViolationError::new("Database::Admin", dummy_span(10, 5))
        .with_declared_capabilities(
            vec!["Database::Query".into(), "Database::Execute".into()].into(),
        )
        .with_function_name("manage_users");

    let diagnostic = error.build();

    let help_text: Vec<String> = diagnostic
        .helps()
        .iter()
        .map(|h| h.message.to_string())
        .collect();

    // Should suggest creating a context group for multiple capabilities
    assert!(help_text.iter().any(|h| h.contains("context group")));
}

#[test]
fn test_error_messages_are_actionable() {
    // All errors should provide multiple actionable suggestions

    let e0306 = CapabilityViolationError::new("Database::Execute", dummy_span(6, 5))
        .with_declared_capabilities(vec!["Database::Query".into()].into())
        .build();

    let e0307 = SubContextNotFoundError::new("Database", "Read", dummy_span(2, 12))
        .with_available_sub_contexts(vec!["Query".into(), "Execute".into()].into())
        .build();

    let e0308 = CapabilityNotProvidedError::new("FileSystem::Read", dummy_span(9, 16)).build();

    let e0309 = PartialImplementationWarning::new("FileSystem", "ReadOnlyFS", dummy_span(5, 1))
        .with_missing_sub_contexts(vec!["Write".into(), "Admin".into()].into())
        .build();

    // All should have at least one help message
    assert!(!e0306.helps().is_empty());
    assert!(!e0307.helps().is_empty());
    assert!(!e0308.helps().is_empty());
    assert!(!e0309.helps().is_empty());

    // E0308 should have at least 2 suggestions (provide vs pass as parameter)
    assert!(e0308.helps().len() >= 2);
}

#[test]
fn test_security_emphasis() {
    // E0306 should emphasize security implications of capability attenuation
    let error = CapabilityViolationError::new("Database::Execute", dummy_span(6, 5)).build();

    let note_text: Vec<String> = error
        .notes()
        .iter()
        .map(|n| n.message.to_string())
        .collect();

    assert!(
        note_text
            .iter()
            .any(|n| n.contains("security") || n.contains("attenuation"))
    );
}
