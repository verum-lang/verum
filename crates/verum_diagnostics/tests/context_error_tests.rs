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
// Tests for context_error module
// Migrated from src/context_error.rs per CLAUDE.md standards

use verum_common::{List, Text};
use verum_diagnostics::Span;
use verum_diagnostics::codes;
use verum_diagnostics::context_error::*;

#[test]
fn test_levenshtein_distance() {
    assert_eq!(levenshtein_distance("", ""), 0);
    assert_eq!(levenshtein_distance("abc", "abc"), 0);
    assert_eq!(levenshtein_distance("abc", "ab"), 1);
    assert_eq!(levenshtein_distance("abc", "def"), 3);
    assert_eq!(levenshtein_distance("Logger", "Loger"), 1);
    assert_eq!(levenshtein_distance("Database", "DataBase"), 1);
}

#[test]
fn test_find_similar_contexts() {
    let available: List<Text> = vec![
        Text::from("Logger"),
        Text::from("Database"),
        Text::from("Auth"),
        Text::from("Metrics"),
    ].into();

    let similar = find_similar_contexts("Loger", &available);
    assert!(similar.contains(&Text::from("Logger")));

    let similar = find_similar_contexts("DB", &available);
    // DB is too far from Database (distance > 3), might be empty
    assert!(similar.len() <= 5);
}

#[test]
fn test_call_frame_format() {
    let frame = CallFrame::new("get_user", Span::new("main.vr", 42, 15, 25))
        .with_contexts(vec!["Database".into()].into());

    let formatted = frame.format(1);
    assert!(formatted.contains("get_user"));
    assert!(formatted.contains("main.vr:42"));
    assert!(formatted.contains("Database"));
}

#[test]
fn test_call_chain_format() {
    let chain = CallChain::new("Database")
        .add_frame(CallFrame::new("main", Span::new("main.vr", 10, 5, 10)))
        .add_frame(
            CallFrame::new("handle_request", Span::new("main.vr", 20, 8, 22))
                .with_contexts(vec!["Logger".into()].into()),
        )
        .add_frame(
            CallFrame::new("get_user", Span::new("main.vr", 42, 15, 25))
                .with_contexts(vec!["Database".into()].into())
                .origin(),
        );

    let formatted = chain.format();
    assert!(formatted.contains("main()"));
    assert!(formatted.contains("handle_request()"));
    assert!(formatted.contains("get_user()"));
    assert!(formatted.contains("Database"));
}

#[test]
fn test_context_not_declared_error() {
    let error = ContextNotDeclaredError::new("Database", Span::new("main.vr", 42, 15, 25)).build();

    assert_eq!(error.code(), Some(codes::E0301));
    assert!(error.message().contains("Database"));
    assert!(error.message().contains("not declared"));
}

#[test]
fn test_context_not_declared_with_chain() {
    let chain = CallChain::new("Database")
        .add_frame(CallFrame::new("main", Span::new("main.vr", 10, 5, 10)))
        .add_frame(
            CallFrame::new("get_user", Span::new("main.vr", 42, 15, 25))
                .with_contexts(vec!["Database".into()].into()),
        );

    let error = ContextNotDeclaredError::new("Database", Span::new("main.vr", 42, 15, 25))
        .with_call_chain(chain)
        .build();

    assert_eq!(error.code(), Some(codes::E0301));
    assert!(!error.notes().is_empty());
}

#[test]
fn test_context_suggestions() {
    let chain = CallChain::new("Database").add_frame(
        CallFrame::new("process_user", Span::new("main.vr", 35, 5, 17))
            .with_contexts(vec!["Logger".into()].into()),
    );

    let suggestions = chain.suggestions();
    assert!(!suggestions.is_empty());
    assert!(suggestions.iter().any(|s| s.title().contains("signature")));
}
