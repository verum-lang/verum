#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs
)]
//! Comprehensive tests for must_handle_errors module
//!
//! Tests the MustHandleTracker and E0317 error generation for @must_handle Results.

use verum_common::span::{FileId, Span};
use verum_diagnostics::diagnostic::Severity;
use verum_diagnostics::must_handle_errors::{
    BranchInfo, E0317, FlowContext, MustHandleTracker, ResultHandling, ViolationKind,
};

fn make_span(start: u32, end: u32) -> Span {
    Span::new(start, end, FileId::dummy())
}

// === MustHandleTracker Tests ===

#[test]
fn test_tracker_new() {
    let tracker = MustHandleTracker::new();
    assert!(tracker.get_unhandled().is_empty());
    assert!(tracker.errors.is_empty());
}

#[test]
fn test_tracker_register_result() {
    let mut tracker = MustHandleTracker::new();

    let id = tracker.register_result(
        make_span(0, 10),
        Some("result".into()),
        "CriticalError".into(),
        "risky_operation()".into(),
    );

    assert!(tracker.is_tracked(id));
    assert_eq!(tracker.get_unhandled().len(), 1);
}

#[test]
fn test_tracker_mark_handled_propagated() {
    let mut tracker = MustHandleTracker::new();

    let id = tracker.register_result(
        make_span(0, 10),
        Some("result".into()),
        "CriticalError".into(),
        "risky_operation()".into(),
    );

    tracker.mark_handled(id, ResultHandling::Propagated);

    // After marking handled, get_unhandled should be empty
    assert!(tracker.get_unhandled().is_empty());
}

#[test]
fn test_tracker_mark_handled_matched() {
    let mut tracker = MustHandleTracker::new();

    let id = tracker.register_result(
        make_span(0, 10),
        Some("result".into()),
        "CriticalError".into(),
        "risky()".into(),
    );

    tracker.mark_handled(id, ResultHandling::Matched);
    assert!(tracker.get_unhandled().is_empty());
}

#[test]
fn test_tracker_mark_handled_by_name() {
    let mut tracker = MustHandleTracker::new();

    let _id = tracker.register_result(
        make_span(0, 10),
        Some("my_result".into()),
        "DbError".into(),
        "db.query()".into(),
    );

    tracker.mark_handled_by_name("my_result", ResultHandling::IfLet);
    assert!(tracker.get_unhandled().is_empty());
}

#[test]
fn test_tracker_wildcard_ignored() {
    let mut tracker = MustHandleTracker::new();

    let scope = tracker.enter_scope();
    let id = tracker.register_result(
        make_span(0, 10),
        Some("_".into()),
        "CriticalError".into(),
        "risky()".into(),
    );

    // Wildcard ignored is a special case - it's explicitly ignored but should still error
    tracker.mark_handled(id, ResultHandling::WildcardIgnored);

    // After marking as WildcardIgnored, exit scope to generate error
    tracker.exit_scope();

    // Should generate an error for wildcard-ignored result
    assert_eq!(tracker.errors.len(), 1);
    assert_eq!(tracker.errors[0].kind, ViolationKind::WildcardIgnored);
}

#[test]
fn test_tracker_scope_management() {
    let mut tracker = MustHandleTracker::new();

    // Enter a scope
    let scope1 = tracker.enter_scope();

    // Register a result in this scope
    let id = tracker.register_result(
        make_span(0, 10),
        Some("result".into()),
        "Error".into(),
        "op()".into(),
    );

    // Exit the scope without handling - should generate error
    tracker.exit_scope();

    // Error should be generated for unhandled result
    assert!(!tracker.errors.is_empty());
}

#[test]
fn test_tracker_nested_scopes() {
    let mut tracker = MustHandleTracker::new();

    // Outer scope
    let outer = tracker.enter_scope();
    let outer_id = tracker.register_result(
        make_span(0, 10),
        Some("outer_result".into()),
        "Error".into(),
        "outer_op()".into(),
    );

    // Inner scope
    let inner = tracker.enter_scope();
    let inner_id = tracker.register_result(
        make_span(20, 30),
        Some("inner_result".into()),
        "Error".into(),
        "inner_op()".into(),
    );

    // Handle inner result
    tracker.mark_handled(inner_id, ResultHandling::Propagated);

    // Exit inner scope - should not generate error
    tracker.exit_scope();
    assert!(tracker.errors.is_empty());

    // Exit outer scope without handling - should generate error
    tracker.exit_scope();
    assert_eq!(tracker.errors.len(), 1);
}

#[test]
fn test_tracker_clear() {
    let mut tracker = MustHandleTracker::new();

    let _id = tracker.register_result(
        make_span(0, 10),
        Some("result".into()),
        "Error".into(),
        "op()".into(),
    );

    tracker.clear();

    assert!(tracker.get_unhandled().is_empty());
    assert!(tracker.errors.is_empty());
}

#[test]
fn test_tracker_to_diagnostics() {
    let mut tracker = MustHandleTracker::new();

    let _scope = tracker.enter_scope();
    let _id = tracker.register_result(
        make_span(0, 10),
        Some("result".into()),
        "CriticalError".into(),
        "risky()".into(),
    );
    tracker.exit_scope();

    let diagnostics = tracker.to_diagnostics();
    assert_eq!(diagnostics.len(), 1);
}

// === FlowContext Tests ===

#[test]
fn test_flow_context_new() {
    let ctx = FlowContext::new();
    assert!(ctx.handled_branches.is_empty());
    assert!(ctx.unhandled_branches.is_empty());
    assert!(ctx.is_fully_handled());
}

#[test]
fn test_flow_context_add_handled() {
    let mut ctx = FlowContext::new();
    ctx.add_handled(BranchInfo::handled(
        "then branch",
        make_span(0, 10),
        "unwrap()",
    ));

    assert_eq!(ctx.handled_branches.len(), 1);
    assert!(ctx.is_fully_handled());
    assert_eq!(ctx.handled_percentage(), 100);
}

#[test]
fn test_flow_context_add_unhandled() {
    let mut ctx = FlowContext::new();
    ctx.add_unhandled(BranchInfo::unhandled("else branch", make_span(10, 20)));

    assert_eq!(ctx.unhandled_branches.len(), 1);
    assert!(!ctx.is_fully_handled());
    assert_eq!(ctx.handled_percentage(), 0);
}

#[test]
fn test_flow_context_partial_handling() {
    let mut ctx = FlowContext::new();
    ctx.add_handled(BranchInfo::handled(
        "then branch",
        make_span(0, 10),
        "unwrap()",
    ));
    ctx.add_unhandled(BranchInfo::unhandled("else branch", make_span(10, 20)));

    assert!(!ctx.is_fully_handled());
    assert_eq!(ctx.handled_percentage(), 50);
}

#[test]
fn test_flow_context_summary_fully_handled() {
    let mut ctx = FlowContext::new();
    ctx.add_handled(BranchInfo::handled("branch1", make_span(0, 10), "match"));
    ctx.add_handled(BranchInfo::handled("branch2", make_span(10, 20), "match"));

    let summary = ctx.summary();
    assert!(summary.contains("handled in all branches"));
}

#[test]
fn test_flow_context_summary_partial() {
    let mut ctx = FlowContext::new();
    ctx.add_handled(BranchInfo::handled("branch1", make_span(0, 10), "match"));
    ctx.add_unhandled(BranchInfo::unhandled("branch2", make_span(10, 20)));
    ctx.add_unhandled(BranchInfo::unhandled("branch3", make_span(20, 30)));

    let summary = ctx.summary();
    // Should show "1/3 branches (33%)"
    assert!(summary.contains("1/3"));
}

// === BranchInfo Tests ===

#[test]
fn test_branch_info_handled() {
    let branch = BranchInfo::handled("then branch", make_span(0, 10), "unwrap()");

    assert!(branch.is_handled());
    assert_eq!(branch.description, "then branch");
    assert_eq!(branch.handling, Some("unwrap()".into()));
}

#[test]
fn test_branch_info_unhandled() {
    let branch = BranchInfo::unhandled("else branch", make_span(10, 20));

    assert!(!branch.is_handled());
    assert_eq!(branch.description, "else branch");
    assert!(branch.handling.is_none());
}

// === E0317 Tests ===

#[test]
fn test_e0317_unused_binding() {
    let error = E0317::unused_binding(
        make_span(0, 10),
        "result".into(),
        "CriticalError".into(),
        "risky()".into(),
    );

    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.severity(), Severity::Error);
    // Message contains "unused Result that must be used"
    assert!(diagnostic.message().contains("unused Result"));
}

#[test]
fn test_e0317_direct_drop() {
    let error = E0317::direct_drop(make_span(0, 10), "CriticalError".into(), "risky()".into());

    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.severity(), Severity::Error);
    // Message contains "dropped without handling"
    assert!(diagnostic.message().contains("dropped"));
}

#[test]
fn test_e0317_wildcard_ignored() {
    let error = E0317::wildcard_ignored(make_span(0, 10), "CriticalError".into(), "risky()".into());

    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert!(diagnostic.message().contains("intentionally ignored"));
}

#[test]
fn test_e0317_partial_handling() {
    let mut flow_context = FlowContext::new();
    flow_context.add_handled(BranchInfo::handled("then", make_span(5, 10), "unwrap()"));
    flow_context.add_unhandled(BranchInfo::unhandled("else", make_span(15, 20)));

    let error = E0317::partial_handling(
        make_span(0, 30),
        Some("result".into()),
        "CriticalError".into(),
        "risky()".into(),
        flow_context,
    );

    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert!(diagnostic.message().contains("some code paths"));
}

#[test]
fn test_e0317_fix_suggestions_unused() {
    let error = E0317::unused_binding(
        make_span(0, 10),
        "result".into(),
        "CriticalError".into(),
        "risky()".into(),
    );

    let suggestions = error.create_fix_suggestions();
    assert!(!suggestions.is_empty());

    // Should suggest using ? operator
    assert!(suggestions.iter().any(|s| s.code().contains("?")));
    // Should suggest match expression
    assert!(suggestions.iter().any(|s| s.code().contains("match")));
}

#[test]
fn test_e0317_violation_kind() {
    let unused = E0317::unused_binding(make_span(0, 10), "r".into(), "Error".into(), "op()".into());
    assert_eq!(unused.kind, ViolationKind::UnusedBinding);

    let dropped = E0317::direct_drop(make_span(0, 10), "Error".into(), "op()".into());
    assert_eq!(dropped.kind, ViolationKind::DirectDrop);

    let wildcard = E0317::wildcard_ignored(make_span(0, 10), "Error".into(), "op()".into());
    assert_eq!(wildcard.kind, ViolationKind::WildcardIgnored);
}

// === ResultHandling Tests ===

#[test]
fn test_result_handling_variants() {
    // Test all variants exist and can be compared
    assert_ne!(ResultHandling::NotHandled, ResultHandling::Propagated);
    assert_ne!(ResultHandling::Matched, ResultHandling::IfLet);
    assert_ne!(ResultHandling::Unwrapped, ResultHandling::ErrorChecked);
    assert_ne!(ResultHandling::WildcardIgnored, ResultHandling::NotHandled);
}

// === Integration Tests ===

#[test]
fn test_full_tracking_workflow() {
    let mut tracker = MustHandleTracker::new();

    // Simulate entering a function body
    let fn_scope = tracker.enter_scope();

    // Call a function that returns Result<T, @must_handle Error>
    let result1_id = tracker.register_result(
        make_span(100, 120),
        Some("db_result".into()),
        "DatabaseError".into(),
        "db.query(\"SELECT * FROM users\")".into(),
    );

    // Call another risky function
    let result2_id = tracker.register_result(
        make_span(150, 180),
        Some("file_result".into()),
        "IoError".into(),
        "std::fs::read(path)".into(),
    );

    // Handle first result via ?
    tracker.mark_handled(result1_id, ResultHandling::Propagated);

    // Enter an if block
    let if_scope = tracker.enter_scope();

    // Call another risky function in if block
    let result3_id = tracker.register_result(
        make_span(200, 220),
        Some("inner_result".into()),
        "NetworkError".into(),
        "http::get(url)".into(),
    );

    // Handle via match
    tracker.mark_handled(result3_id, ResultHandling::Matched);

    // Exit if block
    tracker.exit_scope();

    // Now exit function without handling result2
    tracker.exit_scope();

    // Should have one error for unhandled file_result
    assert_eq!(tracker.errors.len(), 1);

    let diagnostics = tracker.to_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    // The error is for the unhandled Result
    assert!(diagnostics[0].message().contains("unused Result"));
}

#[test]
fn test_multiple_unhandled_results() {
    let mut tracker = MustHandleTracker::new();

    let scope = tracker.enter_scope();

    // Multiple unhandled results
    let _id1 = tracker.register_result(
        make_span(0, 10),
        Some("r1".into()),
        "Error1".into(),
        "op1()".into(),
    );

    let _id2 = tracker.register_result(
        make_span(20, 30),
        Some("r2".into()),
        "Error2".into(),
        "op2()".into(),
    );

    let _id3 = tracker.register_result(make_span(40, 50), None, "Error3".into(), "op3()".into());

    tracker.exit_scope();

    // All three should generate errors
    assert_eq!(tracker.errors.len(), 3);
}
